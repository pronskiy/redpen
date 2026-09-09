//! Accessibility permission (SPEC step C1.4).
//!
//! Capture synthesises ⌘C, which macOS gates behind Accessibility. Without it `capture.rs`
//! fails on every press with a message nobody reads until they are already confused — so
//! check up front and say so.
//!
//! macOS keys the grant to the bundle's designated requirement, and what lands in that
//! requirement depends on the signature. Ad-hoc — every release, and any build that skips
//! `scripts/dev-cert.sh` — puts the binary hash there, so a rebuild or an update revokes
//! the grant and everything silently stops working. Signing with a certificate moves the
//! requirement onto the certificate, which does not change between builds. See README,
//! "The permission annoyance". The onboarding text mentions re-granting either way.

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
