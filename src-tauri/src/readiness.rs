use crate::{browser, commands, cookies};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{webview::PageLoadEvent, AppHandle, Manager, Url, WebviewWindow};

#[derive(Clone, Serialize)]
pub struct Update {
    revision: u64,
    load: u64,
    readiness: Option<cookies::Readiness>,
}

#[derive(Default)]
pub struct Monitor {
    revision: AtomicU64,
    current: Mutex<Option<Arc<Session>>>,
}

pub struct Session {
    active: AtomicBool,
    load: AtomicU64,
    pending: AtomicBool,
    label: &'static str,
    allowed: fn(&Url) -> bool,
}

#[derive(Clone)]
pub struct Ticket {
    session: Arc<Session>,
    load: u64,
}

impl Monitor {
    fn next(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn start(&self, label: &'static str, allowed: fn(&Url) -> bool) -> Arc<Session> {
        let mut current = self.current.lock().unwrap();
        let session = Arc::new(Session {
            active: AtomicBool::new(true),
            load: AtomicU64::new(self.next()),
            pending: AtomicBool::new(false),
            label,
            allowed,
        });
        if let Some(previous) = current.replace(session.clone()) {
            previous.active.store(false, Ordering::Release);
        }
        session
    }

    fn capture(&self, expected: Option<&Arc<Session>>) -> Option<Ticket> {
        let current = self.current.lock().unwrap();
        let session = current.as_ref()?;
        if !session.active.load(Ordering::Acquire)
            || expected.is_some_and(|expected| !Arc::ptr_eq(expected, session))
        {
            return None;
        }
        Some(Ticket {
            session: session.clone(),
            load: session.load.load(Ordering::Acquire),
        })
    }

    fn invalidate(&self, expected: Option<&Arc<Session>>, close: bool) -> Option<Update> {
        let mut current = self.current.lock().unwrap();
        if expected.is_some_and(|expected| {
            !current
                .as_ref()
                .is_some_and(|session| Arc::ptr_eq(expected, session))
        }) {
            return None;
        }
        let revision = self.next();
        if let Some(session) = current.as_ref() {
            session.load.store(revision, Ordering::Release);
            session.pending.store(false, Ordering::Release);
            if close {
                session.active.store(false, Ordering::Release);
            }
        }
        if close {
            current.take();
        }
        Some(Update {
            revision,
            load: revision,
            readiness: None,
        })
    }

    fn finish(&self, ticket: &Ticket, readiness: cookies::Readiness) -> Option<Update> {
        let current = self.current.lock().unwrap();
        if !current
            .as_ref()
            .is_some_and(|session| Arc::ptr_eq(session, &ticket.session))
            || !ticket.valid()
        {
            return None;
        }
        Some(Update {
            revision: self.next(),
            load: ticket.load,
            readiness: Some(readiness),
        })
    }

    fn request(&self, ticket: &Ticket, claim: bool) -> bool {
        let current = self.current.lock().unwrap();
        if !current
            .as_ref()
            .is_some_and(|session| Arc::ptr_eq(session, &ticket.session))
            || !ticket.valid()
        {
            return false;
        }
        ticket.session.pending.store(!claim, Ordering::Release);
        true
    }

    fn take_pending(&self) -> Option<Ticket> {
        let current = self.current.lock().unwrap();
        let session = current.as_ref()?;
        if !session.active.load(Ordering::Acquire) || !session.pending.swap(false, Ordering::AcqRel)
        {
            return None;
        }
        Some(Ticket {
            session: session.clone(),
            load: session.load.load(Ordering::Acquire),
        })
    }
}

impl Ticket {
    fn valid(&self) -> bool {
        self.session.active.load(Ordering::Acquire)
            && self.session.load.load(Ordering::Acquire) == self.load
    }
}

pub fn watch(app: &AppHandle, label: &'static str, allowed: fn(&Url) -> bool) -> Arc<Session> {
    app.state::<Monitor>().start(label, allowed)
}

pub fn ticket(app: &AppHandle) -> Result<Ticket, String> {
    app.state::<Monitor>()
        .capture(None)
        .ok_or("Keep the TikTok sign-in window open while checking your session.".into())
}

fn control(app: &AppHandle, update: &Update) {
    if let Some(window) = app.get_webview_window("main") {
        if browser::authorize(&window).is_ok() {
            if let Ok(json) = serde_json::to_string(update) {
                let _ = window.eval(format!("window.sessionLanternReadiness?.({json});"));
            }
        }
    }
}

fn bind(window: &WebviewWindow, revision: u64) {
    let _ = window.eval(format!(
        "window.dispatchEvent(new CustomEvent('session-lantern-document',{{detail:{revision}}}));"
    ));
}

fn deliver(app: &AppHandle, window: &WebviewWindow, update: &Update, document: Option<&str>) {
    if let Some(readiness) = &update.readiness {
        let count = readiness
            .items
            .iter()
            .filter(|item| item.state == cookies::State::Present)
            .count();
        if let Some(document) = document {
            if let Ok(document) = serde_json::to_string(document) {
                let _ = window.eval(format!(
                    "window.dispatchEvent(new CustomEvent('session-lantern-readiness',{{detail:{{revision:{},document:{document},available:{count},complete:{}}}}}));",
                    update.revision, readiness.complete
                ));
            }
        }
    } else {
        bind(window, update.load);
    }
    control(app, update);
}

pub fn invalidate(app: &AppHandle) {
    if let Some(update) = app.state::<Monitor>().invalidate(None, false) {
        if let Some(window) = app.get_webview_window(browser::LOGIN) {
            bind(&window, update.revision);
        }
        control(app, &update);
    }
}

pub fn closed(app: &AppHandle, session: &Arc<Session>) {
    if let Some(update) = app.state::<Monitor>().invalidate(Some(session), true) {
        control(app, &update);
    }
}

pub fn loaded(window: WebviewWindow, session: Arc<Session>, event: PageLoadEvent) {
    let app = window.app_handle().clone();
    if event == PageLoadEvent::Started {
        if let Some(update) = app.state::<Monitor>().invalidate(Some(&session), false) {
            deliver(&app, &window, &update, None);
        }
    } else if let Some(ticket) = app.state::<Monitor>().capture(Some(&session)) {
        check(window, ticket);
    }
}

fn check(window: WebviewWindow, ticket: Ticket) {
    let app = window.app_handle().clone();
    std::thread::spawn(move || {
        for delay in [0, 2] {
            if delay > 0 {
                std::thread::sleep(Duration::from_secs(delay));
            }
            if !ticket.valid() || !window.url().is_ok_and(|url| (ticket.session.allowed)(&url)) {
                return;
            }
            if !app.state::<Monitor>().request(&ticket, false) {
                return;
            }
            let operations = app.state::<commands::Operations>();
            let Ok(_guard) = operations.begin_for(&app) else {
                return;
            };
            if !app.state::<Monitor>().request(&ticket, true) {
                return;
            }
            let Ok(url) = "https://www.tiktok.com/".parse() else {
                return;
            };
            let document = document(&window);
            let Ok(snapshot) = crate::browser::cookie_snapshot(&window, url) else {
                return;
            };
            let readiness = cookies::inspect(&snapshot, commands::now());
            let complete = readiness.complete;
            if let Some(update) = app.state::<Monitor>().finish(&ticket, readiness) {
                deliver(&app, &window, &update, document.as_deref());
            }
            if complete {
                return;
            }
        }
    });
}

pub fn resume(app: &AppHandle) {
    let Some(ticket) = app.state::<Monitor>().take_pending() else {
        return;
    };
    if let Some(window) = app.get_webview_window(ticket.session.label) {
        check(window, ticket);
    }
}

pub fn recheck(app: &AppHandle) {
    let Some(ticket) = app.state::<Monitor>().capture(None) else {
        return;
    };
    if let Some(window) = app.get_webview_window(ticket.session.label) {
        check(window, ticket);
    }
}

pub fn manual(
    app: &AppHandle,
    ticket: &Ticket,
    readiness: cookies::Readiness,
    document: Option<String>,
) -> Result<Update, String> {
    let window = app
        .get_webview_window(browser::LOGIN)
        .filter(|window| window.url().is_ok_and(|url| browser::login_url(&url)))
        .ok_or("Keep the TikTok sign-in window open while checking your session.")?;
    let update = app
        .state::<Monitor>()
        .finish(ticket, readiness)
        .ok_or("The sign-in page changed. Check your session again.")?;
    deliver(app, &window, &update, document.as_deref());
    Ok(update)
}

fn document(window: &WebviewWindow) -> Option<String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    window.eval_with_callback("typeof window.__sessionLanternDocument==='string' && /^[0-9a-f]{32}$/.test(window.__sessionLanternDocument) ? window.__sessionLanternDocument : null", move |result| { let _ = sender.send(result); }).ok()?;
    let result = receiver.recv_timeout(Duration::from_secs(2)).ok()?;
    serde_json::from_str::<Option<String>>(&result)
        .ok()
        .flatten()
}

pub async fn login_document(app: &AppHandle) -> Option<String> {
    let window = app.get_webview_window(browser::LOGIN)?;
    tauri::async_runtime::spawn_blocking(move || document(&window))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_is_bound_to_the_captured_window_and_load() {
        let monitor = Monitor::default();
        let first = monitor.start(browser::LOGIN, browser::login_url);
        let before = monitor.capture(Some(&first)).unwrap();
        let ready = cookies::inspect(&[], 0);
        let initial = monitor.finish(&before, ready.clone()).unwrap();
        let reset = monitor.invalidate(Some(&first), false).unwrap();
        assert!(reset.revision > initial.revision);
        assert!(monitor.finish(&before, ready.clone()).is_none());
        let refreshed = monitor.capture(Some(&first)).unwrap();
        assert!(monitor.request(&refreshed, false));
        assert!(!monitor.request(&before, true));
        assert_eq!(monitor.take_pending().unwrap().load, refreshed.load);
        assert!(monitor.take_pending().is_none());
        assert!(monitor.finish(&refreshed, ready.clone()).is_some());
        let second = monitor.start(browser::LOGIN, browser::login_url);
        assert!(monitor.invalidate(Some(&first), true).is_none());
        assert!(monitor.finish(&refreshed, ready.clone()).is_none());
        let current = monitor.capture(Some(&second)).unwrap();
        assert!(monitor.finish(&current, ready.clone()).is_some());
        monitor.invalidate(Some(&second), true).unwrap();
        assert!(monitor.finish(&current, ready).is_none());
        assert!(monitor.capture(None).is_none());
    }
}
