//! The `Canopy` object's invokables (declared in bridge.rs).

use crate::bridge::ffi;
use crate::desktop;
use canopy_core::api;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn qs(s: &str) -> QString {
    QString::from(s)
}

pub struct CanopyRust {
    pub(crate) status_json: QString,
    pub(crate) connected: bool,
    started: bool,
    /// A status refresh is already on its way.
    pending: Arc<AtomicBool>,
}

impl Default for CanopyRust {
    fn default() -> Self {
        CanopyRust { status_json: qs("{}"), connected: false, started: false, pending: Arc::new(AtomicBool::new(false)) }
    }
}

type Qt = cxx_qt::CxxQtThread<ffi::Canopy>;

/// Fetch the status in the background (coalesced: one at a time, a short
/// wait so a burst of events makes one request).
fn refresh_status(qt: Qt, pending: Arc<AtomicBool>, delay: u64) {
    if pending.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay));
        pending.store(false, Ordering::SeqCst);
        if let Ok(v) = api::call("status", json!({})) {
            let t = v.to_string();
            let _ = qt.queue(move |mut s| {
                if s.status_json().to_string() != t {
                    s.as_mut().set_status_json(qs(&t));
                }
                s.as_mut().set_connected(true);
            });
        }
    });
}

impl ffi::Canopy {
    fn answer(&self, key: &QString, f: impl FnOnce() -> Value + Send + 'static) {
        let key = key.to_string();
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            let v = f().to_string();
            let _ = qt.queue(move |s| s.async_reply(qs(&key), qs(&v)));
        });
    }

    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().started {
            return;
        }
        self.as_mut().rust_mut().started = true;
        let qt = self.qt_thread();
        crate::single::attach(qt.clone());
        let pending = self.rust().pending.clone();
        std::thread::Builder::new()
            .name("canopyd events".into())
            .spawn(move || loop {
                match api::Client::connect_or_start() {
                    Ok(c) => {
                        refresh_status(qt.clone(), pending.clone(), 0);
                        let (qt2, p2) = (qt.clone(), pending.clone());
                        let _ = c.subscribe(move |ev| {
                            let kind = ev.get("event").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let t = ev.to_string();
                            let _ = qt2.queue(move |s| s.event(qs(&t)));
                            if kind != "transfer" {
                                refresh_status(qt2.clone(), p2.clone(), 300);
                            }
                            true
                        });
                    }
                    Err(e) => eprintln!("canopy: {e:#}"),
                }
                let _ = qt.queue(|mut s| s.as_mut().set_connected(false));
                std::thread::sleep(Duration::from_secs(2));
            })
            .expect("thread");
    }

    pub fn call(&self, key: &QString, cmd: &QString, args_json: &QString) {
        let cmd = cmd.to_string();
        let args: Value = serde_json::from_str(&args_json.to_string()).unwrap_or(json!({}));
        self.answer(key, move || match api::call(&cmd, args) {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
        });
    }

    pub fn refresh(&self) {
        refresh_status(self.qt_thread(), self.rust().pending.clone(), 0);
    }

    pub fn open_grove(&self) {
        std::thread::spawn(desktop::open_grove);
    }

    pub fn open_url(&self, url: &QString) {
        let u = url.to_string();
        std::thread::spawn(move || desktop::open_file(&u));
    }

    pub fn open_path(&self, path: &QString) {
        let p = path.to_string();
        std::thread::spawn(move || desktop::open_file(&p));
    }

    pub fn show_in_files(&self, path: &QString) {
        let p = path.to_string();
        std::thread::spawn(move || desktop::show_in_file_manager(&p));
    }

    pub fn choose_folder(&self, key: &QString, current: &QString) {
        let cur = current.to_string();
        self.answer(key, move || match desktop::choose_folder(&cur) {
            Ok(Some(f)) => json!({"folder": f}),
            Ok(None) => json!({"cancelled": true}),
            Err(e) => json!({"error": e.to_string()}),
        });
    }

    pub fn copy_text(&self, text: &QString) {
        ffi::canopy_copy_text(text);
    }

    pub fn env(&self, name: &QString) -> QString {
        qs(&std::env::var(name.to_string()).unwrap_or_default())
    }

    pub fn set_icon_theme(&self, dark: bool) {
        ffi::canopy_set_icon_theme(dark);
    }

    pub fn grab_windows(&self, path: &QString) {
        ffi::canopy_grab_windows(path);
    }
}

/// Another `canopy` handed over (single.rs): its activation token lets the
/// compositor give this window focus (Qt's Wayland requestActivate reads
/// XDG_ACTIVATION_TOKEN), then QML raises the window.
pub fn activate(s: Pin<&mut ffi::Canopy>, req: String) {
    if let Some(t) = serde_json::from_str::<Value>(&req).ok().and_then(|v| v["token"].as_str().map(str::to_string)).filter(|t| !t.is_empty()) {
        std::env::set_var("XDG_ACTIVATION_TOKEN", t);
    }
    s.activate_requested(qs(&req));
}
