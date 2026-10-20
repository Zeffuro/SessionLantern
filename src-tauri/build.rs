fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "open_login",
        "check_session",
        "export_session",
        "clear_session",
        "check_updates",
        "open_releases",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("Application configuration could not be built");
}
