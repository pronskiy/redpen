//! Accessibility permission (SPEC step C1.4).
//!
//! Capture synthesises ⌘C, which macOS gates behind Accessibility. Without it `capture.rs`
//! fails on every press with a message nobody reads until they are already confused — so
//! check up front and say so.
//!
//! The permission is tied to the *signed binary*, so a rebuild or an app update revokes it
//! and everything silently stops working. That is a documented dev annoyance (README) and
//! the reason the onboarding text mentions re-granting.

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// Non-prompting check. Deliberately not `AXIsProcessTrustedWithOptions` with the prompt
/// flag: that system alert is a single easily-dismissed dialog with no explanation, and
/// once dismissed it never appears again. Our own panel can say *why* redpen needs it and
/// offer the button.
#[cfg(target_os = "macos")]
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Deep link straight to the Accessibility pane.
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
