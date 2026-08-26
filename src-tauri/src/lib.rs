mod config;

use config::Config;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Accessory policy: no Dock icon, no app menu, and the app never becomes the
            // active application. Epic B rests on this — a regular-policy app steals focus
            // the moment its window shows, which is the one thing redpen must never do to
            // the app you are typing in.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let cfg = Config::load();
            let _ = Config::ensure_exists();

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
                        match Config::ensure_exists() {
                            Ok(path) => {
                                if let Err(e) = app.opener().open_path(path.to_string_lossy(), None::<&str>) {
                                    eprintln!("[redpen] could not open config: {e}");
                                }
                            }
                            Err(e) => eprintln!("[redpen] could not create config: {e}"),
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // ---- global hotkey -------------------------------------------------------
            // A1.2 only logs. A1.3 replaces the body with capture::selection().
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|_app, shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            println!("[redpen] hotkey fired: {shortcut:?} — capture is A1.3");
                        }
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

            // The window is created hidden (tauri.conf.json `visible: false`) so it can be
            // converted to an NSPanel *before* its first show. Converting after the first
            // show costs one frame of focus theft — see step B1.1.
            let window = app
                .get_webview_window("main")
                .expect("window `main` is declared in tauri.conf.json");
            println!(
                "[redpen] main window ready · visible={:?} · config={}",
                window.is_visible(),
                Config::path().display()
            );

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
