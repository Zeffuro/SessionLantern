#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod commands;
mod cookies;
mod export;
#[cfg(all(feature = "native-smoke", debug_assertions))]
mod popup_smoke;
mod popups;
mod readiness;
#[cfg(all(feature = "native-smoke", debug_assertions))]
mod readiness_smoke;
#[cfg(all(feature = "native-smoke", debug_assertions))]
mod smoke;
mod updates;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(commands::Operations::default())
        .manage(readiness::Monitor::default())
        .invoke_handler(tauri::generate_handler![
            commands::open_login,
            commands::check_session,
            commands::export_session,
            commands::clear_session,
            updates::check_updates,
            updates::open_releases
        ])
        .setup(|app| {
            #[cfg(all(feature = "native-smoke", debug_assertions))]
            if std::env::args().any(|argument| argument == "--native-smoke") {
                browser::create_control(app.handle(), false)?;
                smoke::run(app.handle())?;
                return Ok(());
            }
            browser::create_control(app.handle(), true)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Session Lantern could not start");
}
