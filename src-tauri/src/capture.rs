//! Grab the current selection by simulating ⌘C and reading the pasteboard.
//!
//! AX-based capture was rejected (decision #4): it is patchy in exactly the apps that
//! matter — Slack, browsers, anything Electron. Simulated ⌘C works wherever ⌘C works.
//!
//! The three battle-scars this module exists to preserve, all of them in `capture_with`
//! and all of them unit-tested against a mock:
//!
//! 1. A settle delay before polling. Firing the keystroke and reading immediately races
//!    the target app.
//! 2. Compare `changeCount`, never content. If you copy the same text twice, content
//!    comparison sees no change and hangs until timeout.
//! 3. Restore only if `changeCount == before + 1`. Anything else means a clipboard manager
//!    (Raycast, Maccy) wrote after us, and restoring would stomp what the user just did.
//!
//! Nothing here ever writes to the user's text. Decision #1.

use std::fmt;
use std::time::{Duration, Instant};

/// Every representation on the pasteboard, not just the text one — an image or RTF payload
/// has to survive a capture untouched.
pub type Snapshot = Vec<(String, Vec<u8>)>;

pub trait Clipboard {
    fn change_count(&self) -> i64;
    fn snapshot(&self) -> Snapshot;
    fn restore(&self, snap: &Snapshot);
    fn read_text(&self) -> Option<String>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// The pasteboard never changed. Usually secure input (a password field), where macOS
    /// swallows synthetic keystrokes — this is expected, not a bug. Sometimes it just means
    /// nothing was selected.
    Timeout,
    /// Something was copied, but it is not text — an image, a file list.
    NotText,
    /// Text, but only whitespace.
    EmptySelection,
    /// Could not synthesise the keystroke at all. Nearly always missing Accessibility
    /// permission, which macOS revokes on rebuild.
    Keystroke(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "nothing was copied (secure input, or no selection)"),
            Self::NotText => write!(f, "the selection is not text"),
            Self::EmptySelection => write!(f, "the selection is empty"),
            Self::Keystroke(e) => write!(f, "could not send ⌘C: {e} (check Accessibility permission)"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// The whole algorithm, with every dependency injected so it can be tested without a
/// real pasteboard, a real keyboard, or a two-second wall-clock wait.
pub fn capture_with<C: Clipboard>(
    clip: &C,
    send_copy: impl FnOnce() -> Result<(), CaptureError>,
    settle: Duration,
    poll_every: Duration,
    timeout: Duration,
) -> Result<String, CaptureError> {
    let snapshot = clip.snapshot();
    let before = clip.change_count();

    send_copy()?;
    std::thread::sleep(settle);

    let deadline = Instant::now() + timeout;
    let mut changed = false;
    let mut text = None;
    loop {
        if clip.change_count() != before {
            changed = true;
            text = clip.read_text();
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(poll_every);
    }

    // Exactly one write since the snapshot — ours. Anything else and somebody is holding
    // the pasteboard; leave it alone rather than fight a clipboard manager.
    if clip.change_count() == before + 1 {
        clip.restore(&snapshot);
    }

    if !changed {
        return Err(CaptureError::Timeout);
    }
    match text {
        None => Err(CaptureError::NotText),
        Some(t) if t.trim().is_empty() => Err(CaptureError::EmptySelection),
        Some(t) => Ok(t),
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{Clipboard, Snapshot};
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::{NSArray, NSData, NSString};

    pub struct MacClipboard;

    impl MacClipboard {
        fn general() -> objc2::rc::Retained<NSPasteboard> {
            NSPasteboard::generalPasteboard()
        }
    }

    impl Clipboard for MacClipboard {
        fn change_count(&self) -> i64 {
            Self::general().changeCount() as i64
        }

        fn snapshot(&self) -> Snapshot {
            let pb = Self::general();
            let mut out = Vec::new();
            let Some(types) = pb.types() else { return out };
            for ty in types.iter() {
                if let Some(data) = pb.dataForType(&ty) {
                    out.push((ty.to_string(), data.to_vec()));
                }
            }
            out
        }

        fn restore(&self, snap: &Snapshot) {
            if snap.is_empty() {
                return;
            }
            unsafe {
                let pb = Self::general();
                pb.clearContents();
                let types: Vec<_> = snap.iter().map(|(t, _)| NSString::from_str(t)).collect();
                let refs: Vec<&NSString> = types.iter().map(|t| t.as_ref()).collect();
                pb.declareTypes_owner(&NSArray::from_slice(&refs), None);
                for (ty, bytes) in snap {
                    let ns_ty = NSString::from_str(ty);
                    let data = NSData::with_bytes(bytes);
                    pb.setData_forType(Some(&data), &ns_ty);
                }
            }
        }

        fn read_text(&self) -> Option<String> {
            unsafe {
                Self::general()
                    .stringForType(NSPasteboardTypeString)
                    .map(|s| s.to_string())
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::MacClipboard;

/// `kVK_ANSI_C` — the *physical* C key, from Carbon's `Events.h`.
///
/// It must be the raw keycode, never `Key::Unicode('c')`. That form asks enigo to resolve
/// the character through the active keyboard layout; when the lookup fails — which it does
/// whenever a non-Latin layout is frontmost, and this app is built for a Russian speaker —
/// it falls back to keycode 0. `kVK_ANSI_A` *is* 0, so the app quietly sends ⌘A: the target
/// app selects all, nothing lands on the pasteboard, and capture times out looking like a
/// secure-input failure. Keycodes are layout-independent; characters are not.
#[cfg(target_os = "macos")]
const KEYCODE_C: u32 = 0x08;

#[cfg(target_os = "macos")]
fn send_copy() -> Result<(), CaptureError> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| CaptureError::Keystroke(e.to_string()))?;
    let press = |e: &mut Enigo, k: Key, d: Direction| {
        e.key(k, d).map_err(|err| CaptureError::Keystroke(err.to_string()))
    };
    press(&mut enigo, Key::Meta, Direction::Press)?;
    let hit = press(&mut enigo, Key::Other(KEYCODE_C), Direction::Click);
    // Release Cmd even if the 'c' failed, or the modifier stays stuck down system-wide.
    let _ = enigo.key(Key::Meta, Direction::Release);
    hit
}

/// Capture the current selection. Blocks for up to ~2s in the worst case.
#[cfg(target_os = "macos")]
pub fn selection() -> Result<String, CaptureError> {
    capture_with(
        &MacClipboard,
        send_copy,
        Duration::from_millis(50),
        Duration::from_millis(5),
        Duration::from_secs(2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Mock {
        count: Cell<i64>,
        text: RefCell<Option<String>>,
        restored: Cell<usize>,
        snapshot_data: RefCell<Snapshot>,
    }

    impl Clipboard for Mock {
        fn change_count(&self) -> i64 { self.count.get() }
        fn snapshot(&self) -> Snapshot { self.snapshot_data.borrow().clone() }
        fn restore(&self, _snap: &Snapshot) { self.restored.set(self.restored.get() + 1); }
        fn read_text(&self) -> Option<String> { self.text.borrow().clone() }
    }

    fn fast<C: Clipboard>(clip: &C, copy: impl FnOnce() -> Result<(), CaptureError>) -> Result<String, CaptureError> {
        capture_with(clip, copy, Duration::ZERO, Duration::from_millis(1), Duration::from_millis(40))
    }

    #[test]
    fn captures_text_and_restores_the_previous_clipboard() {
        let m = Mock::default();
        *m.snapshot_data.borrow_mut() = vec![("public.png".into(), vec![1, 2, 3])];
        let r = fast(&m, || {
            m.count.set(m.count.get() + 1);
            *m.text.borrow_mut() = Some("selected words".into());
            Ok(())
        });
        assert_eq!(r.unwrap(), "selected words");
        assert_eq!(m.restored.get(), 1, "prior clipboard must be put back");
    }

    #[test]
    fn leaves_the_clipboard_alone_when_a_third_party_wrote_after_us() {
        // Raycast/Maccy grabbing the board mid-flight: +2 rather than +1.
        let m = Mock::default();
        let r = fast(&m, || {
            m.count.set(m.count.get() + 2);
            *m.text.borrow_mut() = Some("selected words".into());
            Ok(())
        });
        assert_eq!(r.unwrap(), "selected words");
        assert_eq!(m.restored.get(), 0, "must not stomp a clipboard manager");
    }

    #[test]
    fn times_out_cleanly_under_secure_input() {
        // Password field: macOS swallows the synthetic ⌘C, changeCount never moves.
        let m = Mock::default();
        let r = fast(&m, || Ok(()));
        assert_eq!(r, Err(CaptureError::Timeout));
        assert_eq!(m.restored.get(), 0);
    }

    #[test]
    fn reports_non_text_selections() {
        let m = Mock::default();
        let r = fast(&m, || { m.count.set(1); Ok(()) });
        assert_eq!(r, Err(CaptureError::NotText));
    }

    #[test]
    fn rejects_a_whitespace_only_selection() {
        let m = Mock::default();
        let r = fast(&m, || {
            m.count.set(1);
            *m.text.borrow_mut() = Some("   \n ".into());
            Ok(())
        });
        assert_eq!(r, Err(CaptureError::EmptySelection));
    }

    #[test]
    fn a_failed_keystroke_does_not_touch_the_clipboard() {
        let m = Mock::default();
        let r = fast(&m, || Err(CaptureError::Keystroke("no accessibility permission".into())));
        assert!(matches!(r, Err(CaptureError::Keystroke(_))));
        assert_eq!(m.restored.get(), 0);
    }

    #[test]
    fn identical_content_copied_twice_still_registers() {
        // Why changeCount and not content comparison: the text is the same both times.
        let m = Mock::default();
        *m.text.borrow_mut() = Some("same text".into());
        let r = fast(&m, || {
            m.count.set(m.count.get() + 1);
            *m.text.borrow_mut() = Some("same text".into());
            Ok(())
        });
        assert_eq!(r.unwrap(), "same text");
    }
}
