use crate::{browser, cookies};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub fn run(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let body = r#"<!doctype html><title>Checking</title><script>
            addEventListener('DOMContentLoaded',async()=>{
              const notice=document.querySelector('[data-session-lantern-notice]');
              const bounds=notice?.getBoundingClientRect();
              if(!bounds || bounds.height<20 || bounds.height>100 || bounds.left!==0 || bounds.right>innerWidth || notice.shadowRoot!==null){document.title='UNEXPECTED';return;}
              if(!window.__TAURI__?.core?.invoke){document.title='DENIED';return;}
              try{await window.__TAURI__.core.invoke('check_session');document.title='UNEXPECTED';}
              catch(error){document.title=String(error).includes('not allowed')?'DENIED':'UNEXPECTED';}
            });</script><body><br><br><br><p>Synthetic local fixture. No TikTok account or network requests.</p></body>"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'none'; connect-src ipc: http://ipc.localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    let denied = Arc::new(AtomicU8::new(0));
    let title_result = denied.clone();
    let window = WebviewWindowBuilder::new(
        app,
        "native-smoke-remote",
        WebviewUrl::External(format!("http://{address}/").parse()?),
    )
    .visible(std::env::args().any(|argument| argument == "--preview-notice"))
    .inner_size(500.0, 600.0)
    .data_directory(app.path().app_local_data_dir()?.join("synthetic-smoke"))
    .incognito(true)
    .initialization_script(format!("({})();", browser::LOGIN_NOTICE))
    .on_document_title_changed(move |_, title| {
        if title == "DENIED" {
            title_result.store(1, Ordering::Release);
        }
        if title == "UNEXPECTED" {
            title_result.store(2, Ordering::Release);
        }
    })
    .build()?;
    let app = app.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), &'static str> {
            if browser::authorize(&app.get_webview_window("main").ok_or("control absent")?).is_err()
                || browser::authorize(&window).is_ok()
            {
                return Err("window authorization failed");
            }
            let root: tauri::Url = "https://www.tiktok.com/"
                .parse()
                .map_err(|_| "URL failed")?;
            for (name, value) in [
                ("sessionid", "SYNTHETIC_SESSION"),
                ("tt-target-idc", "SYNTHETIC_REGION"),
                ("msToken", "SYNTHETIC_TOKEN=="),
                ("s_v_web_id", "SYNTHETIC_BROWSER"),
            ] {
                window
                    .set_cookie(
                        tauri::webview::Cookie::build((name, value))
                            .domain("www.tiktok.com")
                            .path("/")
                            .http_only(true)
                            .secure(true)
                            .build(),
                    )
                    .map_err(|_| "native cookie write failed")?;
            }
            let snapshot = window
                .cookies_for_url(root.clone())
                .map_err(|_| "native cookie read failed")?;
            let control_cookies = app
                .get_webview_window("main")
                .ok_or("control absent")?
                .cookies_for_url(root.clone())
                .map_err(|_| "control isolation read failed")?;
            if !control_cookies.is_empty() {
                return Err("login cookie store was not isolated");
            }
            if !cookies::inspect(&snapshot, 0).complete {
                return Err("native cookie selection failed");
            }
            let values =
                cookies::collect(&snapshot, 0).map_err(|_| "native cookie mapping failed")?;
            if values.session_id != "SYNTHETIC_SESSION" || values.verify_fp != "SYNTHETIC_BROWSER" {
                return Err("native cookie values changed");
            }
            window
                .clear_all_browsing_data()
                .map_err(|_| "native cleanup failed")?;
            if !window
                .cookies_for_url(root)
                .map_err(|_| "native reread failed")?
                .is_empty()
            {
                return Err("native cookie data remained after clearing");
            }
            let started = Instant::now();
            while denied.load(Ordering::Acquire) == 0 && started.elapsed() < Duration::from_secs(15)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            if denied.load(Ordering::Acquire) != 1 {
                return Err("remote IPC was not denied as expected");
            }
            crate::popup_smoke::verify(&app)?;
            crate::readiness_smoke::verify(&app)?;
            Ok(())
        })();
        match result {
            Ok(()) => println!("Native smoke passed: reminder rendering, secure cookies, clear, isolation, remote IPC denial, linked popup session and cleanup. No TikTok network requests."),
            Err(message) => eprintln!("Native smoke failed: {message}"),
        }
        if result.is_err() || !std::env::args().any(|argument| argument == "--preview-notice") {
            app.exit(if result.is_ok() { 0 } else { 1 });
        }
    });
    Ok(())
}
