mod capture;
mod config;
mod llm;
mod panel;

use config::{Config, ConfigStore};
use llm::InFlight;
use tauri_nspanel::ManagerExt;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

/// Debug builds show a snippet so you can see capture working; release builds never print
/// the user's writing to a log.
fn preview(text: &str) -> String {
    #[cfg(debug_assertions)]
    {
        let one_line: String = text.chars().take(60).collect::<String>().replace('\n', " ");
        let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
        format!(": {one_line:?}{ellipsis}")
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = text;
        String::new()
    }
}

/// Dismissing must *abort*, not just hide. A hidden window with a live request keeps
/// generating tokens you are no longer reading, and billing for them.
#[tauri::command]
fn dismiss(app: tauri::AppHandle) {
    let aborted = app.state::<InFlight>().abort();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    println!("[redpen] dismissed{}", if aborted { " — request aborted" } else { "" });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_nspanel::init())
        .invoke_handler(tauri::generate_handler![dismiss])
        .setup(|app| {
            // Accessory policy: no Dock icon, no app menu, and the app never becomes the
            // active application. Epic B rests on this — a regular-policy app steals focus
            // the moment its window shows, which is the one thing redpen must never do to
            // the app you are typing in.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let cfg_path = Config::path();
            let _ = Config::ensure_exists(&cfg_path);
            let store = ConfigStore::new(cfg_path.clone());
            let cfg = store.current().config;
            app.manage(store.clone());
            app.manage(InFlight::default());

            // ---- tray ----------------------------------------------------------------
            let icon = app
                .default_window_icon()
                .cloned()
                .expect("bundle config must supply a default window icon");

            let open_config = MenuItem::with_id(app, "open_config", "Open Config…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit redpen", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&open_config, &sep, &quit])?;

            // Placeholder art — the menu bar wants a template image, which is a C1 job.
            TrayIconBuilder::with_id("redpen")
                .icon(icon)
                .tooltip("redpen")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "open_config" => {
                        let path = Config::path();
                        if let Err(e) = Config::ensure_exists(&path) {
                            eprintln!("[redpen] could not create config: {e}");
                            return;
                        }
                        if let Err(e) = app.opener().open_path(path.to_string_lossy(), None::<&str>) {
                            eprintln!("[redpen] could not open config: {e}");
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // ---- global hotkey -------------------------------------------------------
            // A1.2 only logs. A1.3 replaces the body with capture::selection().
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|_app, _shortcut, event| {
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        // Off the hotkey thread: capture sleeps ~50ms and can poll for a
                        // full 2s under secure input. Doing that inline would stall the UI
                        // and swallow the next press.
                        let app = _app.clone();
                        std::thread::spawn(move || match capture::selection() {
                            Ok(text) => {
                                println!("[redpen] captured {} chars{}", text.chars().count(), preview(&text));
                                // Show only *after* the copy has landed. Showing first
                                // would move focus here and the synthetic ⌘C would target
                                // our own empty window instead of the user's selection.
                                //
                                // `order_front_regardless`, never `set_focus`: the panel
                                // must appear without the app activating. set_focus would
                                // ask for exactly the thing B1.2 exists to prevent.
                                match app.get_webview_panel("main") {
                                    Ok(panel) => panel.order_front_regardless(),
                                    Err(_) => {
                                        if let Some(window) = app.get_webview_window("main") {
                                            let _ = window.show();
                                        }
                                    }
                                }
                                let loaded = app.state::<ConfigStore>().current();
                                let handle = tauri::async_runtime::spawn(llm::run(
                                    app.clone(),
                                    loaded,
                                    text,
                                ));
                                app.state::<InFlight>().set(handle);
                            }
                            Err(e) => eprintln!("[redpen] capture failed: {e}"),
                        });
                    })
                    .build(),
            )?;

            match cfg.hotkey.parse::<Shortcut>() {
                Ok(shortcut) => match app.global_shortcut().register(shortcut) {
                    Ok(()) => println!("[redpen] hotkey registered: {}", cfg.hotkey),
                    // Almost always means another app already owns the combination.
                    Err(e) => eprintln!("[redpen] could not register {}: {e}", cfg.hotkey),
                },
                Err(e) => eprintln!(
                    "[redpen] hotkey {:?} is not parseable ({e}); expected e.g. \"Alt+Cmd+E\"",
                    cfg.hotkey
                ),
            }

            // ---- hot reload ---------------------------------------------------------
            // Watches config.json *and* the prompt file it points at (decision #23). The
            // hotkey re-registers live, which is also the visible proof reload works.
            let handle = app.handle().clone();
            if let Err(e) = config::watch(store, move |old, new| {
                if old.config.hotkey != new.config.hotkey {
                    if let Ok(previous) = old.config.hotkey.parse::<Shortcut>() {
                        let _ = handle.global_shortcut().unregister(previous);
                    }
                    match new.config.hotkey.parse::<Shortcut>() {
                        Ok(next) => match handle.global_shortcut().register(next) {
                            Ok(()) => println!("[redpen] hotkey rebound: {} -> {}", old.config.hotkey, new.config.hotkey),
                            Err(err) => eprintln!("[redpen] could not bind {}: {err}", new.config.hotkey),
                        },
                        Err(err) => eprintln!("[redpen] hotkey {:?} is not parseable ({err})", new.config.hotkey),
                    }
                }
                if old.config.model != new.config.model {
                    println!("[redpen] model: {} -> {}", old.config.model, new.config.model);
                }
                if old.config.effort != new.config.effort {
                    println!("[redpen] effort: {} -> {}", old.config.effort, new.config.effort);
                }
                if old.system_prompt != new.system_prompt {
                    println!("[redpen] prompt reloaded ({} chars)", new.system_prompt.chars().count());
                }
                if old.config.api_key != new.config.api_key {
                    println!("[redpen] api key updated");   // never print the key itself
                }
            }) {
                eprintln!("[redpen] hot reload unavailable: {e}");
            }

            // The window is created hidden (tauri.conf.json `visible: false`) so it can be
            // converted to an NSPanel *before* its first show. Converting after the first
            // show costs one frame of focus theft — see step B1.1.
            let window = app
                .get_webview_window("main")
                .expect("window `main` is declared in tauri.conf.json");

            // B1.1: convert while still hidden. After the first show is one frame too late.
            match panel::convert(&window) {
                Ok(()) => println!("[redpen] window converted to a non-activating NSPanel"),
                Err(e) => eprintln!("[redpen] panel conversion failed: {e}"),
            }
            println!(
                "[redpen] main window ready · visible={:?} · config={}",
                window.is_visible(),
                Config::path().display()
            );

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                window.app_handle().state::<InFlight>().abort();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
