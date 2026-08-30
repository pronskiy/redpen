//! "Check for Updates…" behind the tray menu.
//!
//! Rust drives the whole flow and the panel only renders it, which is the same split
//! critique already uses: `llm::run` emits `critique-*`, this emits `update-*`, and
//! `main.ts` is the one place that knows what either looks like.
//!
//! The endpoint is read from `config.json` at check time rather than baked into
//! `tauri.conf.json`, so it hot-reloads with everything else and an empty string switches
//! checks off. The `pubkey` cannot move with it — it is paired with the signing key at
//! build time — so the two halves of the updater's configuration deliberately live apart.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::config::ConfigStore;
use crate::panel;

pub const EVENT_CHECKING: &str = "update-checking";
pub const EVENT_AVAILABLE: &str = "update-available";
pub const EVENT_NONE: &str = "update-none";
pub const EVENT_ERROR: &str = "update-error";
pub const EVENT_PROGRESS: &str = "update-progress";

/// Holds the `Update` between the check and the user pressing Install. The plugin's handle
/// carries the resolved download URL and its signature, so re-checking on click would be a
/// second round trip that could disagree with what the panel is showing.
#[derive(Default)]
pub struct Pending(std::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[derive(Clone, Serialize)]
pub struct Available {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    /// Set when installing this update would cost the Accessibility grant — see
    /// `is_developer_id_signed`. The panel turns it into a warning above the button.
    pub unsigned: bool,
}

/// macOS keys the Accessibility grant to the app's *designated requirement*. On a Developer
/// ID signed bundle that requirement is the signing identity, so the grant survives a
/// replaced binary; on an unsigned or ad-hoc signed one it is the binary hash itself, and
/// any update revokes it. Capture then fails silently until it is granted again — the exact
/// failure the SPEC already records for rebuilds ("macOS ties it to the exact binary").
///
/// So this is asked *before* offering Install, and the answer is shown, rather than letting
/// the user find out by pressing ⌥⌘E and getting nothing.
#[cfg(target_os = "macos")]
fn is_developer_id_signed() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    // `redpen.app/Contents/MacOS/redpen` -> `redpen.app`. Under `cargo run` there is no
    // bundle above the binary, the filter fails, and codesign reports it unsigned — which
    // is the truth in dev.
    let target = exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .map(|p| p.to_path_buf())
        .unwrap_or(exe);

    std::process::Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(&target)
        .output()
        // codesign writes its report to stderr, including on success.
        .map(|o| String::from_utf8_lossy(&o.stderr).contains("Authority=Developer ID Application"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn is_developer_id_signed() -> bool {
    true
}

async fn check(app: &AppHandle) -> Result<Option<Available>, String> {
    let endpoint = app.state::<ConfigStore>().current().config.update_endpoint;
    if endpoint.trim().is_empty() {
        return Err("update_endpoint is empty in config.json, so update checks are off".into());
    }
    let url = endpoint
        .parse()
        .map_err(|e| format!("update_endpoint {endpoint:?} is not a URL ({e})"))?;

    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(Some(update)) => {
            let info = Available {
                version: update.version.clone(),
                current: update.current_version.clone(),
                notes: update.body.clone(),
                unsigned: !is_developer_id_signed(),
            };
            *app.state::<Pending>().0.lock().expect("pending lock poisoned") = Some(update);
            Ok(Some(info))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Tray entry point. The check is a network round trip, so the panel goes up first and says
/// what it is doing — the same reason C1.3 shows the card before capture finishes.
pub fn check_from_tray(app: &AppHandle) {
    let app = app.clone();
    panel::position_at_mouse(&app);
    panel::show(&app);
    let _ = app.emit(EVENT_CHECKING, ());

    tauri::async_runtime::spawn(async move {
        match check(&app).await {
            Ok(Some(info)) => {
                println!("[redpen] update available: {} -> {}", info.current, info.version);
                let _ = app.emit(EVENT_AVAILABLE, info);
            }
            Ok(None) => {
                println!("[redpen] no update available");
                // Carries the running version so the panel can name it without reaching
                // for a second Tauri API just to say "you have 0.1.0".
                let _ = app.emit(EVENT_NONE, app.package_info().version.to_string());
            }
            Err(e) => {
                eprintln!("[redpen] update check failed: {e}");
                let _ = app.emit(EVENT_ERROR, e);
            }
        }
    });
}

/// The panel's Install button. Downloads, swaps the bundle, and restarts.
#[tauri::command]
pub fn install_update(app: AppHandle) {
    let Some(update) = app.state::<Pending>().0.lock().expect("pending lock poisoned").take()
    else {
        let _ = app.emit(EVENT_ERROR, "nothing to install — check for updates first");
        return;
    };

    tauri::async_runtime::spawn(async move {
        let progress = app.clone();
        let mut got: usize = 0;
        let result = update
            .download_and_install(
                move |chunk, total| {
                    got += chunk;
                    // Percent only when the server sent a length; otherwise the panel just
                    // says "downloading" rather than inventing a denominator.
                    if let Some(total) = total.filter(|t| *t > 0) {
                        let pct = (got as f64 / total as f64 * 100.0).min(100.0) as u8;
                        let _ = progress.emit(EVENT_PROGRESS, pct);
                    }
                },
                || {},
            )
            .await;

        match result {
            Ok(()) => {
                println!("[redpen] update installed — restarting");
                app.restart();
            }
            Err(e) => {
                eprintln!("[redpen] update install failed: {e}");
                let _ = app.emit(EVENT_ERROR, e.to_string());
            }
        }
    });
}
