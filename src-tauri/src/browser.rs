use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LOGIN: &str = "tiktok-login";
pub(crate) const LOGIN_NOTICE: &str = include_str!("login-notice.js");

fn login_notice_script() -> String {
    format!(
        "if(window.top===window && location.protocol==='https:' && (location.hostname==='tiktok.com' || location.hostname.endsWith('.tiktok.com'))){{({LOGIN_NOTICE})();}}"
    )
}

pub fn trusted_control(url: &tauri::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    ((url.scheme() == "tauri" && host == "localhost")
        || (url.scheme() == "http" && host == "tauri.localhost"))
        && matches!(url.path(), "/" | "/index.html")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

pub fn login_url(url: &tauri::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https"
        && (host == "tiktok.com" || host.ends_with(".tiktok.com"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

pub fn authorize(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" && window.url().is_ok_and(|url| trusted_control(&url)) {
        Ok(())
    } else {
        Err("This action is only available from the local control window.".into())
    }
}

pub fn create_control(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title(format!("Session Lantern · {}", env!("CARGO_PKG_VERSION")))
        .visible(visible)
        .inner_size(860.0, 860.0)
        .min_inner_size(500.0, 600.0)
        .data_directory(app.path().app_local_data_dir()?.join("control"))
        .on_navigation(trusted_control)
        .build()
}

pub fn open(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN) {
        crate::popups::login_opened();
        crate::readiness::recheck(app);
        window
            .show()
            .map_err(|_| "Could not show the login window.")?;
        return window
            .set_focus()
            .map_err(|_| "Could not focus the login window.".into());
    }
    let url = tauri::Url::parse("https://www.tiktok.com/login")
        .map_err(|_| "The login address is not valid.")?;
    let profile = app
        .path()
        .app_local_data_dir()
        .map_err(|_| "Private session storage is unavailable.")?
        .join("login-webview");
    let session = crate::readiness::watch(app, LOGIN, login_url);
    WebviewWindowBuilder::new(app, LOGIN, WebviewUrl::External(url))
        .title("TikTok sign-in · Session Lantern")
        .inner_size(1050.0, 800.0)
        .data_directory(profile)
        .incognito(true)
        .initialization_script(login_notice_script())
        .disable_drag_drop_handler()
        .on_navigation(login_url)
        .on_page_load({
            let session = session.clone();
            move |window, payload| {
                crate::readiness::loaded(window, session.clone(), payload.event())
            }
        })
        .on_new_window({
            let app = app.clone();
            move |url, features| crate::popups::respond(&app, url, features)
        })
        .on_download(|_, _| false)
        .build()
        .map(|window| {
            crate::popups::login_opened();
            attach_popup_cleanup(&window);
            let app = app.clone();
            let session = session.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    crate::readiness::closed(&app, &session);
                }
            });
        })
        .map_err(|_| {
            "Could not open TikTok sign-in. Check that the system webview is installed.".into()
        })
}

pub(crate) fn attach_popup_cleanup(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = crate::popups::close_auth_popups(&app);
        }
    });
}

pub async fn snapshot(app: &AppHandle) -> Result<Vec<tauri::webview::Cookie<'static>>, String> {
    let window = app.get_webview_window(LOGIN).ok_or(
        "Open the TikTok sign-in window and keep it open while checking or exporting your session.",
    )?;
    if !window.url().is_ok_and(|url| login_url(&url)) {
        return Err("Wait until the sign-in window is on TikTok.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let url = tauri::Url::parse("https://www.tiktok.com/")
            .map_err(|_| "The cookie address is not valid.")?;
        cookie_snapshot(&window, url).map_err(|_| "The browser session could not be read.")
    })
    .await
    .map_err(|_| "The browser session check could not finish.")?
    .map_err(String::from)
}

pub(crate) fn cookie_snapshot(
    window: &WebviewWindow,
    url: tauri::Url,
) -> tauri::Result<Vec<tauri::webview::Cookie<'static>>> {
    #[cfg(not(target_os = "macos"))]
    {
        window.cookies_for_url(url)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = url;
        // WKWebView's URL query omits parent-domain cookies; inspect only this isolated store.
        window.cookies()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_navigation_and_local_control_have_separate_boundaries() {
        for value in [
            "https://www.tiktok.com/login",
            "https://tiktok.com/",
            "https://live.tiktok.com/",
        ] {
            assert!(login_url(&tauri::Url::parse(value).unwrap()));
        }
        for value in [
            "https://tiktok.com.evil.test/",
            "http://www.tiktok.com/",
            "https://user@www.tiktok.com/",
            "https://www.tiktok.com:8443/",
            "tauri://localhost/",
        ] {
            assert!(!login_url(&tauri::Url::parse(value).unwrap()));
        }
        assert!(trusted_control(
            &tauri::Url::parse("http://tauri.localhost/index.html").unwrap()
        ));
        assert!(trusted_control(
            &tauri::Url::parse("tauri://localhost/").unwrap()
        ));
        for value in [
            "https://www.tiktok.com/",
            "http://tauri.localhost.evil.test/",
            "http://localhost/",
            "http://tauri.localhost/other.html",
        ] {
            assert!(!trusted_control(&tauri::Url::parse(value).unwrap()));
        }
    }
}
