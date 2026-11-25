//! Spot a quick double ⌘C, the translate trigger (Epic F).
//!
//! Pure on purpose: no keyboard, no clock, no AppKit. The key monitor (F1.2) feeds it
//! timestamps and the raw event fields, the same way `capture_with` gets its keystroke and
//! timings injected, so every rule here is unit-tested without pressing anything.
//!
//! The AppKit half, at the bottom, is the passive key monitor (F1.2). It observes ⌘C and
//! never consumes it, the same mechanism as the Esc monitor in `panel.rs` (decision #38).

use crate::capture::Clipboard;
use std::time::{Duration, Instant};

/// `kVK_ANSI_C`, the physical C key. Never match the character: with a Russian layout
/// frontmost the character is "с", and the keycode is what stays put (decision #27).
pub const KEYCODE_C: u16 = 0x08;

// NSEventModifierFlags bits.
const SHIFT: u64 = 1 << 17;
const CONTROL: u64 = 1 << 18;
const OPTION: u64 = 1 << 19;
const COMMAND: u64 = 1 << 20;

/// A plain ⌘C: the C key with Command and no other chord modifier, and not auto-repeat.
///
/// Caps Lock, Fn and the numeric-pad bit are ignored, so caps lock being on doesn't turn the
/// trigger off. ⇧⌘C, ⌥⌘C and ⌃⌘C are different shortcuts in plenty of apps, and holding ⌘C
/// down is not two presses.
pub fn is_plain_cmd_c(keycode: u16, modifier_flags: u64, is_repeat: bool) -> bool {
    let chord = modifier_flags & (SHIFT | CONTROL | OPTION | COMMAND);
    keycode == KEYCODE_C && chord == COMMAND && !is_repeat
}

/// How close two ⌘C presses have to be to count as one double press.
pub const WINDOW: Duration = Duration::from_millis(400);

pub struct DoubleCopy {
    window: Duration,
    last: Option<Instant>,
    suppressed: bool,
}

impl DoubleCopy {
    pub const fn new(window: Duration) -> Self {
        Self { window, last: None, suppressed: false }
    }

    /// Feed one plain ⌘C. True means this press completes a double press.
    ///
    /// A fire forgets the pair, so a triple press is one translation, not two: the third
    /// press only starts a new pair.
    pub fn press(&mut self, at: Instant) -> bool {
        if self.suppressed {
            // Forget, too: a synthetic ⌘C from the capture must not pair with a real one
            // that comes after the suppression ends.
            self.last = None;
            return false;
        }
        match self.last.take() {
            Some(prev) if at.saturating_duration_since(prev) <= self.window => true,
            _ => {
                self.last = Some(at);
                false
            }
        }
    }

    /// Set while a ⌥⌘E capture runs. Capture sends synthetic ⌘C on the same keycode, up to
    /// twice, and a global monitor cannot tell those from the user's.
    pub fn set_suppressed(&mut self, suppressed: bool) {
        self.suppressed = suppressed;
        self.last = None;
    }

    pub fn is_suppressed(&self) -> bool {
        self.suppressed
    }
}

/// After a double press: wait for the target app's copy to land, then read it.
///
/// `baseline` is the pasteboard's change count at the first press. The global monitor
/// sees the key event asynchronously, so the first copy may or may not have landed by then;
/// either way the second ⌘C moves the count past it whenever something is selected. Nothing
/// selected means it never moves, and the answer is `None`: a double ⌘C on nothing shows no
/// card at all.
///
/// The pasteboard is only read. The copy was the user's own, so there is nothing to restore.
pub fn copied_text<C: Clipboard>(
    clip: &C,
    baseline: i64,
    budget: Duration,
    poll: Duration,
    settle: Duration,
) -> Option<String> {
    let deadline = Instant::now() + budget;
    while clip.change_count() == baseline {
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(poll);
    }
    // The count moves on `clearContents`, a beat before the text is written.
    std::thread::sleep(settle);
    clip.read_text().filter(|t| !t.trim().is_empty())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{is_plain_cmd_c, DoubleCopy, WINDOW};
    use crate::capture::{Clipboard, MacClipboard};
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    struct State {
        detector: DoubleCopy,
        /// Change count at the press that may turn out to be the first of a pair.
        baseline: i64,
    }

    static STATE: Mutex<State> = Mutex::new(State { detector: DoubleCopy::new(WINDOW), baseline: 0 });

    /// ⌥⌘E captures in progress. A count, not a flag: a second capture can start before the
    /// first one's release delay runs out.
    static CAPTURES: AtomicUsize = AtomicUsize::new(0);

    /// How long suppression outlasts a capture. The monitor gets key events asynchronously,
    /// so capture's synthetic ⌘C can arrive after capture has already returned.
    const RELEASE_DELAY: Duration = Duration::from_millis(250);

    /// Call right before a ⌥⌘E capture sends its first synthetic ⌘C.
    pub fn capture_started() {
        CAPTURES.fetch_add(1, Ordering::SeqCst);
    }

    /// Call once the capture is over, whatever its outcome.
    pub fn capture_finished() {
        std::thread::spawn(|| {
            std::thread::sleep(RELEASE_DELAY);
            CAPTURES.fetch_sub(1, Ordering::SeqCst);
        });
    }

    /// Install the monitor. `on_fire` gets the change count from the first press of the pair
    /// and runs on the main thread, so it must hand anything slow to another thread.
    pub fn install(on_fire: impl Fn(i64) + 'static) {
        let block = RcBlock::new(move |event: NonNull<NSEvent>| {
            let ev = unsafe { event.as_ref() };
            if !is_plain_cmd_c(ev.keyCode(), ev.modifierFlags().0 as u64, ev.isARepeat()) {
                return;
            }
            let count = MacClipboard.change_count();
            let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let busy = CAPTURES.load(Ordering::SeqCst) > 0;
            if busy != st.detector.is_suppressed() {
                st.detector.set_suppressed(busy);
            }
            if st.detector.press(Instant::now()) {
                let baseline = st.baseline;
                drop(st);
                on_fire(baseline);
            } else {
                st.baseline = count;
            }
        });
        let monitor =
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block);
        if monitor.is_none() {
            eprintln!("[redpen] could not install the double ⌘C monitor");
        }
        // Lives for the whole process, like the dismissal monitors.
        std::mem::forget((monitor, block));
    }
}

#[cfg(target_os = "macos")]
pub use mac::{capture_finished, capture_started, install};

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::{Cell, RefCell};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[derive(Default)]
    struct Mock {
        count: Cell<i64>,
        text: RefCell<Option<String>>,
        restored: Cell<usize>,
    }

    impl Clipboard for Mock {
        fn change_count(&self) -> i64 { self.count.get() }
        fn snapshot(&self) -> crate::capture::Snapshot { Vec::new() }
        fn restore(&self, _snap: &crate::capture::Snapshot) { self.restored.set(self.restored.get() + 1); }
        fn read_text(&self) -> Option<String> { self.text.borrow().clone() }
    }

    fn read(m: &Mock, baseline: i64) -> Option<String> {
        copied_text(m, baseline, ms(30), ms(1), Duration::ZERO)
    }

    #[test]
    fn reads_what_was_copied_once_the_count_moves() {
        let m = Mock::default();
        m.count.set(8);
        *m.text.borrow_mut() = Some("Ik snap het niet".into());
        assert_eq!(read(&m, 7).as_deref(), Some("Ik snap het niet"));
    }

    #[test]
    fn nothing_copied_means_no_card() {
        // Double ⌘C with nothing selected: the count never moves.
        let m = Mock::default();
        m.count.set(7);
        *m.text.borrow_mut() = Some("what was on the clipboard before".into());
        assert_eq!(read(&m, 7), None, "must not translate an old clipboard");
    }

    #[test]
    fn a_non_text_copy_means_no_card() {
        let m = Mock::default();
        m.count.set(8);
        assert_eq!(read(&m, 7), None);
    }

    #[test]
    fn a_whitespace_copy_means_no_card() {
        let m = Mock::default();
        m.count.set(8);
        *m.text.borrow_mut() = Some(" \n\t".into());
        assert_eq!(read(&m, 7), None);
    }

    #[test]
    fn the_clipboard_is_never_restored() {
        // The copy was the user's own.
        let m = Mock::default();
        m.count.set(8);
        *m.text.borrow_mut() = Some("text".into());
        read(&m, 7);
        assert_eq!(m.restored.get(), 0);
    }

    #[test]
    fn two_presses_inside_the_window_fire() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        assert!(!d.press(t));
        assert!(d.press(t + ms(250)));
    }

    #[test]
    fn the_window_edge_still_counts() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.press(t);
        assert!(d.press(t + WINDOW));
    }

    #[test]
    fn two_presses_outside_the_window_do_not_fire() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        assert!(!d.press(t));
        assert!(!d.press(t + ms(401)));
    }

    #[test]
    fn a_late_second_press_starts_a_new_pair() {
        // Copy, wait, then a quick double: the double must still fire.
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.press(t);
        assert!(!d.press(t + ms(1000)));
        assert!(d.press(t + ms(1200)));
    }

    #[test]
    fn a_triple_press_fires_once() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.press(t);
        assert!(d.press(t + ms(100)));
        assert!(!d.press(t + ms(200)), "the third press must not translate again");
    }

    #[test]
    fn presses_while_suppressed_are_ignored() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.set_suppressed(true);
        assert!(!d.press(t));
        assert!(!d.press(t + ms(50)), "capture's retry ⌘C must not look like a double press");
    }

    #[test]
    fn a_suppressed_press_does_not_pair_with_a_later_real_one() {
        // Capture's synthetic ⌘C lands just before suppression lifts; the user's next ⌘C
        // right after is a single copy, not a double.
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.set_suppressed(true);
        d.press(t);
        d.set_suppressed(false);
        assert!(!d.press(t + ms(100)));
    }

    #[test]
    fn a_press_before_suppression_does_not_pair_with_one_after() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(WINDOW);
        d.press(t);
        d.set_suppressed(true);
        d.set_suppressed(false);
        assert!(!d.press(t + ms(100)));
    }

    #[test]
    fn plain_cmd_c_qualifies() {
        assert!(is_plain_cmd_c(KEYCODE_C, COMMAND, false));
    }

    #[test]
    fn caps_lock_and_fn_bits_do_not_matter() {
        let caps_lock = 1 << 16;
        let function = 1 << 23;
        assert!(is_plain_cmd_c(KEYCODE_C, COMMAND | caps_lock | function, false));
    }

    #[test]
    fn other_chords_do_not_qualify() {
        for extra in [SHIFT, CONTROL, OPTION] {
            assert!(!is_plain_cmd_c(KEYCODE_C, COMMAND | extra, false), "flag {extra:#x}");
        }
    }

    #[test]
    fn c_without_command_does_not_qualify() {
        assert!(!is_plain_cmd_c(KEYCODE_C, 0, false));
    }

    #[test]
    fn auto_repeat_does_not_qualify() {
        assert!(!is_plain_cmd_c(KEYCODE_C, COMMAND, true));
    }

    #[test]
    fn another_key_does_not_qualify() {
        let kvk_ansi_v = 0x09;
        assert!(!is_plain_cmd_c(kvk_ansi_v, COMMAND, false));
    }
}
