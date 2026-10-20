use crate::{browser, popups};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

pub fn verify(app: &AppHandle) -> Result<(), &'static str> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "popup fixture bind failed")?;
    let address = listener
        .local_addr()
        .map_err(|_| "popup fixture address failed")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "popup fixture setup failed")?;
    std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(45) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = [0; 4096];
            let Ok(count) = stream.read(&mut request) else {
                continue;
            };
            let popup = String::from_utf8_lossy(&request[..count]).starts_with("GET /popup ");
            let body = if popup {
                r#"<!doctype html><title>Popup fixture</title><script>
                addEventListener('DOMContentLoaded',async()=>{
                  document.cookie='SYNTHETIC_POPUP=popup; Path=/; SameSite=Lax';
                  let denied=!window.__TAURI__?.core?.invoke;
                  if(!denied){
                    try{await window.__TAURI__.core.invoke('check_session');}
                    catch(error){denied=String(error).includes('not allowed');}
                  }
                  const shared=document.cookie.includes('SYNTHETIC_OPENER=opener');
                  if(window.opener)window.opener.postMessage(denied&&shared?'LINKED_SHARED_DENIED':'FAILED',location.origin);
                });</script>"#
            } else {
                r#"<!doctype html><title>READY</title><script>
                document.cookie='SYNTHETIC_OPENER=opener; Path=/; SameSite=Lax';
                addEventListener('message',event=>{
                  if(event.origin===location.origin&&event.source===window.fixturePopup)
                    document.title=event.data;
                });
                </script>"#
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let status = Arc::new(AtomicU8::new(0));
    let title_status = status.clone();
    let popup_app = app.clone();
    let parent = WebviewWindowBuilder::new(
        app,
        "popup-smoke-opener",
        WebviewUrl::External(
            format!("http://{address}/opener")
                .parse()
                .map_err(|_| "popup URL failed")?,
        ),
    )
    .visible(false)
    .data_directory(
        app.path()
            .app_local_data_dir()
            .map_err(|_| "popup fixture profile failed")?
            .join("popup-smoke"),
    )
    .incognito(true)
    .on_new_window(move |url, features| popups::respond_fixture(&popup_app, url, features))
    .on_document_title_changed(move |_, title| {
        title_status.store(
            match title.as_str() {
                "READY" => 1,
                "LINKED_SHARED_DENIED" => 2,
                "FAILED" => 3,
                _ => return,
            },
            Ordering::Release,
        );
    })
    .build()
    .map_err(|_| "popup fixture window failed")?;
    browser::attach_popup_cleanup(&parent);
    let result = (|| {
        if !wait_for(|| status.load(Ordering::Acquire) == 1) {
            return Err("popup opener fixture did not load");
        }
        parent
            .set_cookie(
                tauri::webview::Cookie::build(("SYNTHETIC_HTTP_ONLY", "shared"))
                    .domain("127.0.0.1")
                    .path("/")
                    .http_only(true)
                    .build(),
            )
            .map_err(|_| "popup synthetic cookie write failed")?;
        parent
            .eval("window.fixturePopup=window.open('/popup','synthetic-provider')")
            .map_err(|_| "popup fixture open failed")?;
        if !wait_for(|| status.load(Ordering::Acquire) >= 2) || status.load(Ordering::Acquire) != 2
        {
            return Err("popup opener/shared JS cookie/remote IPC proof failed");
        }
        let child = app
            .get_webview_window(popups::AUTH_POPUP)
            .ok_or("popup was not tracked")?;
        if browser::authorize(&child).is_ok() {
            return Err("popup received local authorization");
        }
        let root: tauri::Url = format!("http://{address}/")
            .parse()
            .map_err(|_| "popup cookie URL failed")?;
        for window in [&parent, &child] {
            let cookies = window
                .cookies_for_url(root.clone())
                .map_err(|_| "popup native cookie read failed")?;
            for (name, value) in [
                ("SYNTHETIC_OPENER", "opener"),
                ("SYNTHETIC_POPUP", "popup"),
                ("SYNTHETIC_HTTP_ONLY", "shared"),
            ] {
                if !cookies
                    .iter()
                    .any(|cookie| cookie.name() == name && cookie.value() == value)
                {
                    return Err("popup native cookie jar was not shared");
                }
            }
        }
        let control = app
            .get_webview_window("main")
            .ok_or("control absent during popup smoke")?;
        if control
            .cookies_for_url(root)
            .map_err(|_| "popup isolation read failed")?
            .iter()
            .any(|cookie| cookie.name().starts_with("SYNTHETIC_"))
        {
            return Err("popup cookie jar reached control window");
        }
        popups::close_auth_popups(app).map_err(|_| "popup clear cleanup failed")?;
        if !wait_for(|| app.get_webview_window(popups::AUTH_POPUP).is_none()) {
            return Err("popup survived session cleanup");
        }
        status.store(1, Ordering::Release);
        parent
            .eval("document.title='READY'; window.fixturePopup=window.open('about:blank','synthetic-provider-2'); setTimeout(()=>{if(window.fixturePopup)window.fixturePopup.location.href='/popup'},0)")
            .map_err(|_| "popup reopen failed")?;
        if !wait_for(|| app.get_webview_window(popups::AUTH_POPUP).is_some()) {
            return Err("popup could not reopen after cleanup");
        }
        if !wait_for(|| status.load(Ordering::Acquire) >= 2) || status.load(Ordering::Acquire) != 2
        {
            return Err("blank-then-navigate popup bootstrap failed");
        }
        parent.destroy().map_err(|_| "popup parent close failed")?;
        if !wait_for(|| app.get_webview_window(popups::AUTH_POPUP).is_none()) {
            return Err("popup survived its opener closing");
        }
        Ok(())
    })();
    let _ = popups::close_auth_popups(app);
    if app.get_webview_window(parent.label()).is_some() {
        let _ = parent.destroy();
    }
    result
}
