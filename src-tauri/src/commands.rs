use crate::{browser, cookies, export};
use serde::Serialize;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct Operations(AtomicBool);
pub(crate) struct Operation<'a>(&'a AtomicBool, Option<AppHandle>);
impl Operations {
    pub(crate) fn begin(&self) -> Result<Operation<'_>, String> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Operation(&self.0, None))
            .map_err(|_| "Finish the current action first.".into())
    }
}
impl Operations {
    pub(crate) fn begin_for(&self, app: &AppHandle) -> Result<Operation<'_>, String> {
        let mut operation = self.begin()?;
        operation.1 = Some(app.clone());
        Ok(operation)
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
        if let Some(app) = &self.1 {
            crate::readiness::resume(app);
        }
    }
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[tauri::command]
pub async fn open_login(
    window: WebviewWindow,
    app: AppHandle,
    operations: State<'_, Operations>,
) -> Result<(), String> {
    browser::authorize(&window)?;
    let _operation = operations.begin_for(&app)?;
    browser::open(&app)
}

#[tauri::command]
pub async fn check_session(
    window: WebviewWindow,
    app: AppHandle,
    operations: State<'_, Operations>,
) -> Result<crate::readiness::Update, String> {
    browser::authorize(&window)?;
    let _operation = operations.begin_for(&app)?;
    let ticket = crate::readiness::ticket(&app)?;
    let document = crate::readiness::login_document(&app).await;
    let snapshot = browser::snapshot(&app).await?;
    crate::readiness::manual(&app, &ticket, cookies::inspect(&snapshot, now()), document)
}

#[derive(Serialize)]
pub struct ExportResult {
    outcome: &'static str,
    message: Option<&'static str>,
}

fn save_export(path: &std::path::Path, bytes: &[u8]) -> ExportResult {
    let mut output = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(output) => output,
        Err(error) => {
            return ExportResult {
                outcome: "failed",
                message: Some(if error.kind() == std::io::ErrorKind::AlreadyExists {
                    "Export not saved: a file already exists at that location. It was not replaced. Choose a different file name and save again."
                } else {
                    "Export not saved: the file could not be created. Choose a new file name in a writable folder."
                }),
            }
        }
    };
    if output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .is_err()
    {
        return ExportResult {
            outcome: "failed",
            message: Some(
                "Export not saved completely. Choose a different file name and save again.",
            ),
        };
    }
    ExportResult {
        outcome: "saved",
        message: None,
    }
}

#[tauri::command]
pub async fn export_session(
    window: WebviewWindow,
    app: AppHandle,
    operations: State<'_, Operations>,
    password: String,
) -> Result<ExportResult, String> {
    let password = Zeroizing::new(password);
    browser::authorize(&window)?;
    let _operation = operations.begin_for(&app)?;
    let snapshot = browser::snapshot(&app).await?;
    let values = cookies::collect(&snapshot, now())?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = export::encrypt_export(&values, &password)?;
        let Some(file) = app
            .dialog()
            .file()
            .set_title("Save encrypted bot session")
            .add_filter("Encrypted session", &["sessionpack"])
            .set_file_name("bot-session.sessionpack")
            .blocking_save_file()
        else {
            return Ok(ExportResult {
                outcome: "cancelled",
                message: None,
            });
        };
        let path = file
            .into_path()
            .map_err(|_| "Choose a local file destination.")?;
        Ok(save_export(&path, &bytes))
    })
    .await
    .map_err(|_| "The export could not finish.")?
}

#[tauri::command]
pub async fn clear_session(
    window: WebviewWindow,
    app: AppHandle,
    operations: State<'_, Operations>,
) -> Result<(), String> {
    browser::authorize(&window)?;
    let _operation = operations.begin_for(&app)?;
    crate::readiness::invalidate(&app);
    crate::popups::close_auth_popups(&app)?;
    let Some(login) = app.get_webview_window(browser::LOGIN) else {
        return Ok(());
    };
    tauri::async_runtime::spawn_blocking(move || {
        login
            .clear_all_browsing_data()
            .map_err(|_| "The browser session could not be cleared.")?;
        login
            .close()
            .map_err(|_| "The login window could not close. Close it manually.")
    })
    .await
    .map_err(|_| "The session cleanup could not finish.")?
    .map_err(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_exclude_concurrent_export_and_cleanup() {
        let operations = Operations::default();
        let guard = operations.begin().unwrap();
        assert!(operations.begin().is_err());
        drop(guard);
        assert!(operations.begin().is_ok());
    }

    #[test]
    fn duplicate_export_never_replaces_original_and_can_retry_with_new_name() {
        let directory = std::env::temp_dir().join(format!(
            "lantern-export-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("synthetic.sessionpack");
        assert_eq!(
            save_export(&path, b"original synthetic export").outcome,
            "saved"
        );
        let duplicate = save_export(&path, b"replacement synthetic export");
        assert_eq!(duplicate.outcome, "failed");
        assert!(duplicate.message.unwrap().contains("It was not replaced"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original synthetic export");
        let retry = directory.join("synthetic-retry.sessionpack");
        assert_eq!(
            save_export(&retry, b"new synthetic export").outcome,
            "saved"
        );
        assert!(
            save_export(&directory.join("missing/file.sessionpack"), b"synthetic")
                .message
                .unwrap()
                .contains("writable folder")
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(retry).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
