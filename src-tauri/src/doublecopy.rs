//! Spot a quick double ⌘C, the translate trigger (Epic F).
//!
//! Pure on purpose: no keyboard, no clock, no AppKit. The key monitor (F1.2) feeds it
//! timestamps and the raw event fields, the same way `capture_with` gets its keystroke and
//! timings injected, so every rule here is unit-tested without pressing anything.

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
    pub fn new(window: Duration) -> Self {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
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
