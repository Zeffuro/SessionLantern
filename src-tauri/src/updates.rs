use serde::{Deserialize, Serialize};
use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

const API: &str = "https://api.github.com/repos/Zeffuro/SessionLantern/releases/latest";
const RELEASES: &str = "https://github.com/Zeffuro/SessionLantern/releases";
const FAILED: &str = "Could not check GitHub releases. Try again later or open the releases page.";
const LIMIT: usize = 65_536;

#[derive(Serialize)]
pub struct Update {
    state: &'static str,
    current: &'static str,
    latest: Option<String>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn parse_release(bytes: &[u8], current: &str) -> Result<Update, &'static str> {
    if bytes.len() > LIMIT {
        return Err(FAILED);
    }
    let release: Release = serde_json::from_slice(bytes).map_err(|_| FAILED)?;
    if release.draft || release.prerelease || release.tag_name.len() > 128 {
        return Err(FAILED);
    }
    let latest = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| FAILED)?;
    let current = semver::Version::parse(current).map_err(|_| FAILED)?;
    if !latest.pre.is_empty() {
        return Err(FAILED);
    }
    Ok(Update {
        state: if latest.cmp_precedence(&current).is_gt() {
            "available"
        } else {
            "current"
        },
        current: env!("CARGO_PKG_VERSION"),
        latest: Some(latest.to_string()),
    })
}

#[tauri::command]
pub async fn check_updates(window: WebviewWindow) -> Result<Update, String> {
    crate::browser::authorize(&window)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("SessionLantern/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| FAILED)?;
    let mut response = client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| FAILED)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Update {
            state: "unpublished",
            current: env!("CARGO_PKG_VERSION"),
            latest: None,
        });
    }
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > LIMIT as u64)
    {
        return Err(FAILED.into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FAILED)? {
        if bytes.len() + chunk.len() > LIMIT {
            return Err(FAILED.into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_release(&bytes, env!("CARGO_PKG_VERSION")).map_err(String::from)
}

#[tauri::command]
pub fn open_releases(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    crate::browser::authorize(&window)?;
    app.opener()
        .open_url(RELEASES, None::<&str>)
        .map_err(|_| "Could not open the releases page in your browser.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_versions_numerically_and_rejects_unpublished_or_invalid_metadata() {
        let release = |tag: &str| {
            serde_json::to_vec(
                &serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false}),
            )
            .unwrap()
        };
        assert_eq!(
            parse_release(&release("v0.10.0"), "0.2.1").unwrap().state,
            "available"
        );
        assert_eq!(
            parse_release(&release("v0.2.1"), "0.2.1").unwrap().state,
            "current"
        );
        assert_eq!(
            parse_release(&release("v0.2.1+build.2"), "0.2.1")
                .unwrap()
                .state,
            "current"
        );
        assert_eq!(
            parse_release(&release("v0.1.9"), "0.2.1").unwrap().state,
            "current"
        );
        for tag in ["broken", "v0.3.0-beta.1", "https://example.test"] {
            assert!(parse_release(&release(tag), "0.2.1").is_err());
        }
        for field in ["draft", "prerelease"] {
            let mut value =
                serde_json::json!({"tag_name":"v0.3.0","draft":false,"prerelease":false});
            value[field] = true.into();
            assert!(parse_release(&serde_json::to_vec(&value).unwrap(), "0.2.1").is_err());
        }
        assert!(parse_release(&vec![b' '; LIMIT + 1], "0.2.1").is_err());
        assert!(parse_release(b"not JSON", "0.2.1").is_err());
    }
}
