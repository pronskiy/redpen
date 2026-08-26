use tauri::tray::TrayIconBuilder;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Accessory policy: no Dock icon, no app menu, and the app is never the
            // active application. Epic B rests on this — a regular-policy app steals
            // focus the moment its window shows, which is the one thing redpen must
            // never do to the app you are typing in.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let icon = app
                .default_window_icon()
                .cloned()
                .expect("bundle config must supply a default window icon");

            // Placeholder art — the menu bar wants a template image, which is a C1
            // design task, not a scaffold one.
            TrayIconBuilder::with_id("redpen")
                .icon(icon)
                .tooltip("redpen")
                .build(app)?;

            // The window is created hidden (tauri.conf.json `visible: false`) so it can
            // be converted to an NSPanel *before* its first show. Converting after the
            // first show costs one frame of focus theft — see step B1.1.
            let window = app
                .get_webview_window("main")
                .expect("window `main` is declared in tauri.conf.json");
            println!(
                "[redpen] main window ready · visible={:?} · A1.1 scaffold",
                window.is_visible()
            );

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
