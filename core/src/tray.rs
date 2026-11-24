//! The tray icon (StatusNotifierItem, shown by hearth's bar) and desktop
//! notifications (org.freedesktop.Notifications, hearth's own server).
//! Both need a session bus; without one (a server) they're simply absent.

use crate::daemon::{Shared, Trig, S};
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

struct CanopyTray {
    s: Weak<Shared>,
}

fn icon_dir() -> String {
    if let Ok(d) = std::env::var("CANOPY_ICON_DIR") {
        return d;
    }
    // <prefix>/bin/canopyd -> <prefix>/share/icons
    std::env::current_exe().ok().and_then(|e| e.parent().and_then(|b| b.parent()).map(|p| p.join("share/icons"))).filter(|p| p.is_dir()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// hearth's light/dark mode, for the bar icon's colour.
fn dark_bar() -> bool {
    let p = crate::paths::config_home().join("hearth/settings.json");
    let Ok(t) = std::fs::read_to_string(p) else { return true };
    let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
    match v.pointer("/appearance/mode").and_then(|x| x.as_str()).unwrap_or("dark") {
        "light" => false,
        "auto" => {
            let h: u32 = chrono::Local::now().format("%H").to_string().parse().unwrap_or(12);
            !(7..19).contains(&h)
        }
        _ => true,
    }
}

fn open_window() {
    let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("canopy"))).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("canopy"));
    let _ = std::process::Command::new(exe).spawn();
}

fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

fn label(state: &str, transfers: usize) -> String {
    match state {
        "syncing" if transfers > 0 => format!("Syncing {transfers} file{}…", if transfers == 1 { "" } else { "s" }),
        "syncing" => "Syncing…".into(),
        "paused" => "Paused".into(),
        "offline" => "Offline: waiting for the network".into(),
        "error" => "Some items need attention".into(),
        "signed_out" => "Sign in again to keep syncing".into(),
        "empty" => "No accounts yet: open canopy to add one".into(),
        _ => "Up to date".into(),
    }
}

impl ksni::Tray for CanopyTray {
    fn id(&self) -> String {
        "canopy".into()
    }
    fn title(&self) -> String {
        "canopy".into()
    }
    fn icon_theme_path(&self) -> String {
        icon_dir()
    }
    fn icon_name(&self) -> String {
        let state = self.s.upgrade().map(|s| s.overall()).unwrap_or_default();
        let base = match state.as_str() {
            "syncing" => "syncing",
            "paused" => "paused",
            "offline" => "offline",
            "error" | "signed_out" => "error",
            _ => "synced",
        };
        format!("canopy-tray-{base}-{}", if dark_bar() { "dark" } else { "light" })
    }
    fn status(&self) -> ksni::Status {
        let state = self.s.upgrade().map(|s| s.overall()).unwrap_or_default();
        if state == "error" || state == "signed_out" {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        let (state, n) = self.s.upgrade().map(|s| (s.overall(), s.transfers.lock().len())).unwrap_or_default();
        ksni::ToolTip { title: "canopy: Google Drive".into(), description: label(&state, n), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let Some(s) = self.s.upgrade() else { return vec![] };
        let state = s.overall();
        let mut m: Vec<MenuItem<Self>> = vec![
            StandardItem { label: label(&state, s.transfers.lock().len()), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            StandardItem { label: "Open canopy".into(), icon_name: "folder-cloud".into(), activate: Box::new(|_| open_window()), ..Default::default() }.into(),
        ];
        let accts = s.accounts.read().clone();
        let many = accts.len() > 1;
        for a in &accts {
            let cfg = a.cfg.read();
            for r in &cfg.roots {
                let p = r.local.clone();
                let l = if many { format!("Open {} ({})", r.name, cfg.email) } else { format!("Open {}", r.name) };
                m.push(StandardItem { label: l, icon_name: "folder".into(), activate: Box::new(move |_| open_path(&p)), ..Default::default() }.into());
            }
        }
        // Recent activity.
        let mut recent: Vec<(f64, String, PathBuf)> = Vec::new();
        for a in &accts {
            let roots = a.cfg.read().roots.clone();
            for x in a.db.activity(0, 8).unwrap_or_default() {
                if let Some(r) = roots.iter().find(|r| r.id == x.root) {
                    recent.push((x.at, format!("{} {}", verb(&x.kind), x.path.rsplit('/').next().unwrap_or(&x.path)), r.local.join(&x.path)));
                }
            }
        }
        recent.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if !recent.is_empty() {
            let items: Vec<MenuItem<Self>> = recent
                .into_iter()
                .take(8)
                .map(|(_, l, p)| {
                    let dir = if p.is_dir() { p.clone() } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p.clone()) };
                    StandardItem { label: l, activate: Box::new(move |_| open_path(&dir)), ..Default::default() }.into()
                })
                .collect();
            m.push(SubMenu { label: "Recent".into(), submenu: items, ..Default::default() }.into());
        }
        m.push(MenuItem::Separator);
        let paused = s.settings.read().paused;
        let w = self.s.clone();
        m.push(
            StandardItem {
                label: if paused { "Resume syncing".into() } else { "Pause syncing".into() },
                icon_name: if paused { "media-playback-start".into() } else { "media-playback-pause".into() },
                activate: Box::new(move |_| {
                    if let Some(s) = w.upgrade() {
                        let _ = crate::handlers::handle(&s, if paused { "resume" } else { "pause" }, &serde_json::json!({}));
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        let w = self.s.clone();
        m.push(
            StandardItem {
                label: "Sync now".into(),
                icon_name: "view-refresh".into(),
                activate: Box::new(move |_| {
                    if let Some(s) = w.upgrade() {
                        for a in s.accounts.read().iter() {
                            let _ = a.tx.send(Trig::Now);
                        }
                    }
                }),
                ..Default::default()
            }
            .into(),
        );
        m.push(MenuItem::Separator);
        m.push(StandardItem { label: "Quit canopy".into(), icon_name: "application-exit".into(), activate: Box::new(|_| std::process::exit(0)), ..Default::default() }.into());
        m
    }
}

fn verb(kind: &str) -> &'static str {
    match kind {
        "upload" => "↑",
        "download" => "↓",
        "edit" => "↑",
        "delete" | "local_delete" => "✕",
        "move" | "rename" => "→",
        "conflict" => "!",
        _ => "•",
    }
}

pub fn start(s: &S) {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() || std::env::var_os("CANOPY_NO_TRAY").is_some() {
        return;
    }
    let t = CanopyTray { s: Arc::downgrade(s) };
    match t.spawn() {
        Ok(h) => {
            *s.tray.lock() = Some(Box::new(move || {
                // Re-reads icon, tooltip and menu.
                h.update(|_| {});
            }));
        }
        Err(e) => eprintln!("canopy: no tray icon ({e})"),
    }
}

/// Send a desktop notification (blocks briefly; call off busy threads).
pub fn notify(summary: &str, body: &str) {
    static CONN: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    let Some(c) = CONN.get_or_init(|| zbus::blocking::Connection::session().ok()) else { return };
    let hints: std::collections::HashMap<&str, zbus::zvariant::Value> = std::collections::HashMap::from([("desktop-entry", zbus::zvariant::Value::from("canopy"))]);
    let _ = c.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("canopy", 0u32, "canopy", summary, body, Vec::<&str>::new(), hints, 8000i32),
    );
}
