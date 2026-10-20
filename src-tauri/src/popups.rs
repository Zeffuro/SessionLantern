use tauri::{
    webview::{NewWindowFeatures, NewWindowResponse},
    AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder,
};

pub const AUTH_POPUP: &str = "auth-popup";
static LOGIN_ALIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LOGIN_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn login_opened() {
    LOGIN_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    LOGIN_ALIVE.store(true, std::sync::atomic::Ordering::Release);
}

fn secure_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn facebook(host: &str) -> bool {
    matches!(
        host,
        "facebook.com" | "www.facebook.com" | "m.facebook.com" | "web.facebook.com"
    )
}

fn callback(url: &Url) -> bool {
    crate::browser::login_url(url)
        && (url.path() == "/"
            || url.path() == "/login"
            || ["/login/", "/passport/", "/auth/"]
                .iter()
                .any(|path| url.path().starts_with(path)))
}

pub fn initial_url(url: &Url) -> bool {
    if url.as_str() == "about:blank" {
        return true;
    }
    if !secure_url(url) {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let path = url.path();
    if host == "accounts.google.com" {
        return matches!(path, "/ServiceLogin" | "/AccountChooser")
            || ["/o/oauth2/", "/signin/", "/v3/signin/"]
                .iter()
                .any(|prefix| path.starts_with(prefix));
    }
    if facebook(host) {
        return path == "/login.php"
            || path == "/login/"
            || path == "/dialog/oauth"
            || path.strip_prefix("/v").is_some_and(|rest| {
                rest.split_once('/').is_some_and(|(version, suffix)| {
                    !version.is_empty()
                        && version
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || byte == b'.')
                        && suffix == "dialog/oauth"
                })
            });
    }
    callback(url) && path != "/"
}

pub fn navigation_url(url: &Url) -> bool {
    secure_url(url)
        && (matches!(url.host_str(), Some("accounts.google.com"))
            || facebook(url.host_str().unwrap_or_default())
            || callback(url))
}

pub fn respond(
    app: &AppHandle,
    url: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<tauri::Wry> {
    let generation = LOGIN_GENERATION.load(std::sync::atomic::Ordering::Acquire);
    if !initial_url(&url) || !login_alive(app) {
        return NewWindowResponse::Deny;
    }
    match create(app, features, navigation_url, true) {
        NewWindowResponse::Create { window }
            if !login_alive(app)
                || generation != LOGIN_GENERATION.load(std::sync::atomic::Ordering::Acquire) =>
        {
            let _ = window.destroy();
            NewWindowResponse::Deny
        }
        response => response,
    }
}

fn login_alive(app: &AppHandle) -> bool {
    LOGIN_ALIVE.load(std::sync::atomic::Ordering::Acquire)
        && app.get_webview_window(crate::browser::LOGIN).is_some()
}

fn create(
    app: &AppHandle,
    features: NewWindowFeatures,
    navigation: fn(&Url) -> bool,
    visible: bool,
) -> NewWindowResponse<tauri::Wry> {
    if app.get_webview_window(AUTH_POPUP).is_some() {
        return NewWindowResponse::Deny;
    }
    let Ok(profile) = app.path().app_local_data_dir() else {
        return NewWindowResponse::Deny;
    };
    // Tauri transfers the opener environment; WebView2 attaches the actual opener URL.
    let Ok(blank) = "about:blank".parse() else {
        return NewWindowResponse::Deny;
    };
    let committed = std::sync::atomic::AtomicBool::new(false);
    let result = WebviewWindowBuilder::new(app, AUTH_POPUP, WebviewUrl::External(blank))
        .window_features(features)
        .title("Provider sign-in · Session Lantern")
        .inner_size(600.0, 760.0)
        .min_inner_size(420.0, 500.0)
        .center()
        .visible(visible)
        .data_directory(profile.join("login-webview"))
        .incognito(true)
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            if url.as_str() == "about:blank" {
                return !committed.load(std::sync::atomic::Ordering::Acquire);
            }
            let allowed = navigation(url);
            if allowed {
                committed.store(true, std::sync::atomic::Ordering::Release);
            }
            allowed
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build();
    match result {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}

pub fn close_auth_popups(app: &AppHandle) -> Result<(), String> {
    LOGIN_ALIVE.store(false, std::sync::atomic::Ordering::Release);
    LOGIN_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    if let Some(window) = app.get_webview_window(AUTH_POPUP) {
        window
            .destroy()
            .map_err(|_| "The provider sign-in window could not close.")?;
    }
    Ok(())
}

#[cfg(all(feature = "native-smoke", debug_assertions))]
pub fn respond_fixture(
    app: &AppHandle,
    url: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<tauri::Wry> {
    if url.as_str() != "about:blank" && !fixture_url(&url) {
        return NewWindowResponse::Deny;
    }
    create(app, features, fixture_url, false)
}

#[cfg(all(feature = "native-smoke", debug_assertions))]
fn fixture_url(url: &Url) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && matches!(url.path(), "/popup" | "/callback")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_popups_only_target_login_endpoints() {
        for value in [
            "about:blank",
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=synthetic",
            "https://accounts.google.com/ServiceLogin",
            "https://www.facebook.com/v20.0/dialog/oauth",
            "https://m.facebook.com/dialog/oauth",
            "https://web.facebook.com/login.php",
            "https://www.tiktok.com/passport/web/auth/third_party/v2/",
            "https://www.tiktok.com/login?callback=synthetic",
        ] {
            assert!(initial_url(&value.parse().unwrap()), "{value}");
        }
        for value in [
            "about:blank#unrelated",
            "about:blank?unrelated",
            "javascript:alert(1)",
            "tauri://localhost/",
            "http://accounts.google.com/o/oauth2/auth",
            "https://user@accounts.google.com/o/oauth2/auth",
            "https://accounts.google.com:8443/o/oauth2/auth",
            "https://accounts.google.com.evil.test/o/oauth2/auth",
            "https://mail.google.com/",
            "https://evil.facebook.com/dialog/oauth",
            "https://www.facebook.com/photo.php",
            "https://accounts.google.com/avatar",
            "https://www.tiktok.com/video/123",
            "https://www.tiktok.com/",
            "https://www.tiktok.com/login-unrelated",
            "https://127.0.0.1/login",
        ] {
            assert!(!initial_url(&value.parse().unwrap()), "{value}");
        }
    }

    #[test]
    fn redirect_navigation_keeps_exact_provider_and_callback_boundaries() {
        for value in [
            "https://accounts.google.com/v3/signin/challenge/pwd",
            "https://www.facebook.com/checkpoint/",
            "https://facebook.com/login.php",
            "https://www.tiktok.com/passport/web/auth/callback/",
            "https://www.tiktok.com/",
        ] {
            assert!(navigation_url(&value.parse().unwrap()), "{value}");
        }
        for value in [
            "https://google.com/",
            "https://business.facebook.com/",
            "https://www.tiktok.com/video/1",
            "http://www.facebook.com/login.php",
            "https://www.facebook.com.evil.test/login.php",
            "about:blank",
        ] {
            assert!(!navigation_url(&value.parse().unwrap()), "{value}");
        }
    }
}
