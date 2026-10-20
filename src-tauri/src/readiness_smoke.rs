use crate::{browser, commands, readiness};
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tauri::{
    webview::PageLoadEvent, AppHandle, Manager, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

const LABEL: &str = "readiness-smoke";
const COOKIES: [(&str, &str); 4] = [
    ("sessionid", "SYNTHETIC_SESSION"),
    ("tt-target-idc", "SYNTHETIC_REGION"),
    ("msToken", "SYNTHETIC_TOKEN=="),
    ("s_v_web_id", "SYNTHETIC_BROWSER"),
];

#[derive(Deserialize)]
struct View {
    document: String,
    text: String,
    background: String,
    revision: f64,
    denied: bool,
}
impl View {
    fn ready(&self) -> bool {
        self.text.contains("All 4 cookies are available") && self.background == "rgb(185, 243, 207)"
    }
    fn partial(&self) -> bool {
        self.text.contains("3/4 cookies available") && self.yellow()
    }
    fn yellow(&self) -> bool {
        self.background == "rgb(255, 224, 138)"
    }
}

struct FixtureServer(Arc<AtomicBool>);
impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn allowed(url: &Url) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.path() == "/readiness"
        && url.username().is_empty()
        && url.password().is_none()
}

fn server() -> Result<(Url, FixtureServer), &'static str> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "readiness fixture bind failed")?;
    let address = listener
        .local_addr()
        .map_err(|_| "readiness fixture address failed")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "readiness fixture setup failed")?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        while !stopped.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(90) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = [0; 4096];
            if stream.read(&mut request).is_err() {
                continue;
            }
            let body = r#"<!doctype html><title>Readiness fixture</title><script>
            window.__fixtureDenied=false;
            addEventListener('DOMContentLoaded',async()=>{
              if(!window.__TAURI__?.core?.invoke){window.__fixtureDenied=true;return;}
              try{await window.__TAURI__.core.invoke('check_session');}
              catch(error){window.__fixtureDenied=String(error).includes('not allowed');}
            });</script><body>Synthetic local readiness fixture.</body>"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'none'; connect-src ipc: http://ipc.localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Ok((
        format!("http://{address}/readiness")
            .parse()
            .map_err(|_| "readiness fixture URL failed")?,
        FixtureServer(stop),
    ))
}

fn window(
    app: &AppHandle,
    url: Url,
    loads: Arc<AtomicUsize>,
) -> Result<(WebviewWindow, Arc<readiness::Session>), &'static str> {
    let session = readiness::watch(app, LABEL, allowed);
    let observed = session.clone();
    let script = format!(
        r#"(()=>{{
      const original=Element.prototype.attachShadow;
      Element.prototype.attachShadow=function(options){{
        const root=original.call(this,options);
        if(this.hasAttribute('data-session-lantern-notice'))window.__fixtureNotice=root;
        return root;
      }};
      try{{({})();}}finally{{Element.prototype.attachShadow=original;}}
      window.__fixtureRevision=0;
      addEventListener('session-lantern-readiness',event=>{{window.__fixtureRevision=event.detail?.revision||0;}});
    }})();"#,
        browser::LOGIN_NOTICE
    );
    let result = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .visible(false)
        .data_directory(
            app.path()
                .app_local_data_dir()
                .map_err(|_| "readiness fixture profile failed")?
                .join("readiness-smoke"),
        )
        .incognito(true)
        .initialization_script(script)
        .on_navigation(allowed)
        .on_download(|_, _| false)
        .on_page_load(move |window, payload| {
            readiness::loaded(window, observed.clone(), payload.event());
            if payload.event() == PageLoadEvent::Finished {
                loads.fetch_add(1, Ordering::AcqRel);
            }
        })
        .build();
    let window = match result {
        Ok(window) => window,
        Err(_) => {
            readiness::closed(app, &session);
            return Err("readiness fixture window failed");
        }
    };
    let app = app.clone();
    let closing = session.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            readiness::closed(&app, &closing);
        }
    });
    Ok((window, session))
}

fn evaluate(window: &WebviewWindow, script: &str) -> Result<serde_json::Value, &'static str> {
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .eval_with_callback(script, move |result| {
            let _ = sender.send(result);
        })
        .map_err(|_| "readiness fixture evaluation failed")?;
    let value = receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "readiness fixture evaluation timed out")?;
    serde_json::from_str(&value).map_err(|_| "readiness fixture evaluation was malformed")
}

fn view(window: &WebviewWindow) -> Result<View, &'static str> {
    let value = evaluate(
        window,
        r#"(()=>{
      const content=window.__fixtureNotice?.firstElementChild;
      return {document:window.__sessionLanternDocument||'',text:content?.textContent||'',
        background:content?getComputedStyle(content).backgroundColor:'',
        revision:window.__fixtureRevision||0,denied:window.__fixtureDenied===true};
    })()"#,
    )?;
    if !value.is_object() {
        return Err("readiness fixture state was not an object");
    }
    serde_json::from_value(value).map_err(|_| "readiness fixture state fields were malformed")
}

fn wait(mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(12) {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

fn wait_view(
    window: &WebviewWindow,
    condition: impl Fn(&View) -> bool,
) -> Result<View, &'static str> {
    let mut result = None;
    if !wait(|| {
        if let Ok(state) = view(window) {
            if condition(&state) {
                result = Some(state);
                return true;
            }
        }
        false
    }) {
        return Err("readiness fixture state did not settle");
    }
    result.ok_or("readiness fixture state absent")
}

fn set_cookie(window: &WebviewWindow, name: &str, value: &str) -> Result<(), &'static str> {
    window
        .set_cookie(
            tauri::webview::Cookie::build((name.to_owned(), value.to_owned()))
                .domain("www.tiktok.com")
                .path("/")
                .secure(true)
                .http_only(true)
                .build(),
        )
        .map_err(|_| "readiness synthetic cookie write failed")
}

fn seed(window: &WebviewWindow) -> Result<(), &'static str> {
    for (name, value) in COOKIES {
        set_cookie(window, name, value)?;
    }
    Ok(())
}

fn no_ready(window: &WebviewWindow) -> Result<(), &'static str> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(250) {
        if view(window)?.ready() {
            return Err("readiness published while operations were held");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

pub fn verify(app: &AppHandle) -> Result<(), &'static str> {
    let (url, _server) = server()?;
    let operations = app.state::<commands::Operations>();
    let guard = operations
        .begin_for(app)
        .map_err(|_| "readiness startup guard unavailable")?;
    let loads = Arc::new(AtomicUsize::new(0));
    let (first, old_session) = window(app, url.clone(), loads.clone())?;
    let mut current = first.clone();
    let mut session = old_session.clone();
    let result = (|| {
        seed(&first)?;
        if !wait(|| loads.load(Ordering::Acquire) > 0) {
            return Err("readiness initial load did not finish");
        }
        wait_view(&first, |state| state.denied && state.yellow())?;
        if browser::authorize(&first).is_ok() {
            return Err("readiness fixture received control authorization");
        }
        no_ready(&first).map_err(|_| "readiness initial guard assertion failed")?;
        drop(guard);
        let ready = wait_view(&first, View::ready)?;
        if ready.document.len() != 32 {
            return Err("readiness document nonce absent");
        }

        let guard = operations
            .begin_for(app)
            .map_err(|_| "readiness refresh guard unavailable")?;
        let root: Url = "https://www.tiktok.com/"
            .parse()
            .map_err(|_| "readiness cookie URL failed")?;
        let snapshot = first
            .cookies_for_url(root.clone())
            .map_err(|_| "readiness native cookie read failed")?;
        let cookie = snapshot
            .iter()
            .find(|cookie| cookie.name() == "msToken")
            .ok_or("readiness synthetic token absent")?;
        first
            .delete_cookie(cookie.clone())
            .map_err(|_| "readiness synthetic token removal failed")?;
        let before = loads.load(Ordering::Acquire);
        first.reload().map_err(|_| "readiness refresh failed")?;
        if !wait(|| loads.load(Ordering::Acquire) > before) {
            return Err("readiness refresh did not finish");
        }
        let pending = wait_view(&first, |state| {
            state.document != ready.document && state.yellow() && !state.ready()
        })?;
        no_ready(&first).map_err(|_| "readiness refreshed guard assertion failed")?;
        let document = serde_json::to_string(&ready.document)
            .map_err(|_| "readiness nonce encoding failed")?;
        first.eval(format!("window.dispatchEvent(new CustomEvent('session-lantern-readiness',{{detail:{{document:{document},revision:9007199254740000,available:4,complete:true}}}}));"))
            .map_err(|_| "readiness stale document injection failed")?;
        no_ready(&first).map_err(|_| "readiness stale document assertion failed")?;
        drop(guard);
        let partial = wait_view(&first, View::partial)?;
        let document = serde_json::to_string(&pending.document)
            .map_err(|_| "readiness nonce encoding failed")?;
        first.eval(format!("window.dispatchEvent(new CustomEvent('session-lantern-readiness',{{detail:{{document:{document},revision:0,available:4,complete:true}}}}));"))
            .map_err(|_| "readiness stale counter injection failed")?;
        if !view(&first)?.partial() {
            return Err("readiness accepted a stale revision");
        }
        if partial.revision == 0.0 {
            return Err("readiness native revision absent");
        }

        set_cookie(&first, COOKIES[2].0, COOKIES[2].1)?;
        wait_view(&first, View::ready)?;
        let before = loads.load(Ordering::Acquire);
        first
            .reload()
            .map_err(|_| "readiness restored refresh failed")?;
        if !wait(|| loads.load(Ordering::Acquire) > before) {
            return Err("readiness restored load did not finish");
        }
        wait_view(&first, |state| {
            state.ready() && state.document != pending.document
        })?;
        let guard = operations
            .begin_for(app)
            .map_err(|_| "readiness clear guard unavailable")?;
        readiness::closed(app, &old_session);
        first
            .clear_all_browsing_data()
            .map_err(|_| "readiness clear failed")?;
        if !first
            .cookies_for_url(root)
            .map_err(|_| "readiness clear reread failed")?
            .is_empty()
        {
            return Err("readiness clear retained synthetic cookies");
        }
        first
            .destroy()
            .map_err(|_| "readiness fixture close failed")?;
        if !wait(|| app.get_webview_window(LABEL).is_none()) {
            return Err("readiness fixture did not close");
        }
        let before = loads.load(Ordering::Acquire);
        let (reopened, next_session) = window(app, url, loads.clone())?;
        current = reopened;
        session = next_session;
        seed(&current)?;
        if !wait(|| loads.load(Ordering::Acquire) > before) {
            return Err("readiness reopened load did not finish");
        }
        readiness::closed(app, &old_session);
        readiness::loaded(current.clone(), old_session.clone(), PageLoadEvent::Started);
        readiness::loaded(
            current.clone(),
            old_session.clone(),
            PageLoadEvent::Finished,
        );
        no_ready(&current).map_err(|_| "readiness reopened guard assertion failed")?;
        drop(guard);
        wait_view(&current, |state| state.denied && state.ready())?;
        Ok(())
    })();
    readiness::closed(app, &session);
    let _ = current.clear_all_browsing_data();
    let _ = current.destroy();
    result
}
