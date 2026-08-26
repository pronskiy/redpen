//! Turns the hidden window into a non-activating floating NSPanel.
//!
//! Every use of `tauri-nspanel` lives in this file. It is a commit-pinned git dependency
//! and the risk register rates it Med/High, so the blast radius of swapping it — for a
//! Swift shim, or for hand-rolled objc2 — is one module.
//!
//! **Two independent focus paths have to be closed, and closing one is not enough:**
//!
//! 1. `canBecomeKeyWindow = false` stops the panel taking key focus once it is on screen.
//! 2. `NSWindowStyleMaskNonactivatingPanel` stops showing the panel *activating the app*
//!    in the first place.
//!
//! Miss (1) and the panel steals the caret. Miss (2) and the app comes forward, the menu
//! bar changes, and the user's typing goes somewhere else — even though the panel itself
//! never became key.

use tauri::{Manager, WebviewWindow, Wry};
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};

tauri_panel! {
    panel!(RedpenPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

/// Convert **before the first show**. Converting a window that has already been shown costs
/// a frame of focus theft — the thing this whole epic exists to prevent (step B1.1).
pub fn convert(window: &WebviewWindow<Wry>) -> tauri::Result<()> {
    let panel = window.to_panel::<RedpenPanel>()?;

    // Above ordinary windows, below menus and status items.
    panel.set_level(PanelLevel::Floating.value());

    // Focus path 2. Keeping the titled/closable bits so the window stays visible and
    // draggable while Epic B is being tested; C1.1 strips the chrome for the HUD card.
    panel.set_style_mask(StyleMask::new().nonactivating_panel().value());

    // Follow the user across Spaces, and show over full-screen apps — redpen is summoned
    // wherever you happen to be typing, which is often a full-screen editor.
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .value(),
    );

    // Do not vanish when another app becomes active: the whole point is that the source
    // app keeps focus while the panel stays readable.
    panel.set_hides_on_deactivate(false);

    Ok(())
}
