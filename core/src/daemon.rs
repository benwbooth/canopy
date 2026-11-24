//! canopyd: accounts, one sync worker per account, the inotify watcher, the
//! socket server (handlers.rs answers the commands), the tray icon and
//! notifications.
//!
//! A worker waits for a trigger (a change on disk, the poll timer, "sync
//! now"), brings the account's mirror up to date from the changes feed and
//! runs a pass of each root that may have changed. Errors on single files
//! become issues; losing the network makes the account "offline" and it
//! backs off; a revoked sign-in makes it "signed_out".

use crate::auth;
use crate::cloud::Cloud;
use crate::config::{Account, Accounts, Mode, Resolution, Root, RootKind, Settings};
use crate::db::Db;
use crate::drive::{self, Drive};
use crate::engine::{Ctx, Engine, Event, PassOptions, Sink};
use crate::ignore::Ignore;
use crate::mirror;
use crate::paths;
use crate::throttle;
use crate::watch::{RootKey, Watcher};
use anyhow::{anyhow, Result};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use parking_lot::{Mutex, RwLock};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub enum Trig {
    /// Something changed on disk in this root.
    Local(String),
    /// Sync everything now.
    Now,
    Stop,
}

#[derive(Default, Clone, serde::Serialize)]
pub struct AcctState {
    /// idle, syncing, paused, offline, signed_out, error, starting.
    pub phase: String,
    pub message: String,
    pub last_sync: f64,
    /// root id -> why it's held (missing folder, mass delete...).
    pub held: BTreeMap<String, String>,
    pub quota_used: Option<u64>,
    pub quota_total: Option<u64>,
}

pub struct Acct {
    pub cfg: RwLock<Account>,
    pub db: Arc<Db>,
    pub cloud: Arc<dyn Cloud>,
    /// The real Drive (sharing, search...); None in tests.
    pub drive: Option<Arc<Drive>>,
    pub root_id: RwLock<String>,
    pub tx: Sender<Trig>,
    pub state: Mutex<AcctState>,
    /// root id -> paths with an action running.
    pub busy: Mutex<HashMap<String, Arc<Mutex<HashSet<String>>>>>,
    pub cancel: Arc<AtomicBool>,
    pub forced: Mutex<HashMap<(String, String), Resolution>>,
    pub confirmed: Mutex<HashSet<(String, String)>>,
    /// Held while a cycle runs (base folder moves wait for it).
    pub cycle: Mutex<()>,
}

impl Acct {
    pub fn id(&self) -> String {
        self.cfg.read().id.clone()
    }

    pub fn busy_for(&self, root: &str) -> Arc<Mutex<HashSet<String>>> {
        self.busy.lock().entry(root.to_string()).or_default().clone()
    }

    pub fn set_phase(&self, phase: &str, message: &str) -> bool {
        let mut s = self.state.lock();
        let changed = s.phase != phase || s.message != message;
        s.phase = phase.into();
        s.message = message.into();
        changed
    }
}

pub struct Shared {
    pub settings: RwLock<Settings>,
    pub accounts: RwLock<Vec<Arc<Acct>>>,
    subs: Mutex<Vec<Sender<String>>>,
    /// "account\0root\0path" -> {account, root, path, up, done, total}
    pub transfers: Mutex<BTreeMap<String, (Value, Instant)>>,
    pub logins: Mutex<HashMap<String, auth::Pending>>,
    pub watcher: Option<Watcher>,
    pub quit: AtomicBool,
    /// Roots that had their first pass since start (offline deletions).
    started: Mutex<HashSet<String>>,
    pub tray: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// hearth's Internet Accounts service is there.
    pub hearth: AtomicBool,
    last_note: Mutex<HashMap<String, Instant>>,
}

pub type S = Arc<Shared>;

impl Shared {
    pub fn broadcast(&self, v: Value) {
        let line = v.to_string();
        self.subs.lock().retain(|s| s.send(line.clone()).is_ok());
    }

    pub fn subscribe(&self) -> Receiver<String> {
        let (tx, rx) = unbounded();
        self.subs.lock().push(tx);
        rx
    }

    pub fn changed(&self) {
        self.broadcast(json!({"event": "changed"}));
        if let Some(f) = self.tray.lock().as_ref() {
            f();
        }
    }

    pub fn account(&self, id: &str) -> Result<Arc<Acct>> {
        let accts = self.accounts.read();
        if id.is_empty() && accts.len() == 1 {
            return Ok(accts[0].clone());
        }
        accts.iter().find(|a| a.cfg.read().id == id || a.cfg.read().email == id).cloned().ok_or_else(|| anyhow!("no account {id}"))
    }

    pub fn save_accounts(&self) -> Result<()> {
        let accounts = Accounts { accounts: self.accounts.read().iter().map(|a| a.cfg.read().clone()).collect() };
        accounts.save()
    }

    /// Overall state for the tray and the window: synced, syncing, paused,
    /// offline, error, signed_out, empty.
    pub fn overall(&self) -> String {
        if self.settings.read().paused {
            return "paused".into();
        }
        let accts = self.accounts.read();
        if accts.is_empty() {
            return "empty".into();
        }
        let phases: Vec<String> = accts.iter().map(|a| a.state.lock().phase.clone()).collect();
        let issues = accts.iter().map(|a| a.db.issues().map(|v| v.len()).unwrap_or(0)).sum::<usize>();
        if phases.iter().any(|p| p == "signed_out") {
            "signed_out".into()
        } else if phases.iter().any(|p| p == "syncing" || p == "starting") || !self.transfers.lock().is_empty() {
            "syncing".into()
        } else if phases.iter().all(|p| p == "offline") {
            "offline".into()
        } else if issues > 0 || accts.iter().any(|a| !a.state.lock().held.is_empty()) {
            "error".into()
        } else if phases.iter().all(|p| p == "paused") {
            "paused".into()
        } else {
            "synced".into()
        }
    }

    /// A desktop notification (throttled per key), off the calling thread.
    pub fn notify(&self, key: &str, summary: &str, body: &str) {
        if !self.settings.read().notifications {
            return;
        }
        {
            let mut last = self.last_note.lock();
            if last.get(key).map(|t| t.elapsed() < Duration::from_secs(120)).unwrap_or(false) {
                return;
            }
            last.insert(key.into(), Instant::now());
        }
        let (summary, body) = (summary.to_string(), body.to_string());
        std::thread::spawn(move || crate::tray::notify(&summary, &body));
    }

    fn sink_for(self: &Arc<Self>, account: &str) -> Sink {
        let s = Arc::downgrade(self);
        let account = account.to_string();
        Arc::new(move |e: Event| {
            let Some(s) = s.upgrade() else { return };
            let mut v = serde_json::to_value(&e).unwrap_or(Value::Null);
            v["account"] = json!(account);
            match &e {
                Event::Transfer { root, path, .. } => {
                    let key = format!("{account}\0{root}\0{path}");
                    let mut t = s.transfers.lock();
                    let prev = t.get(&key).map(|x| x.1);
                    let send = prev.map(|at| at.elapsed() > Duration::from_millis(250)).unwrap_or(true);
                    let at = if send { Instant::now() } else { prev.unwrap_or_else(Instant::now) };
                    t.insert(key, (v.clone(), at));
                    drop(t);
                    if send {
                        s.broadcast(v);
                    }
                    return;
                }
                Event::TransferDone { root, path, .. } => {
                    s.transfers.lock().remove(&format!("{account}\0{root}\0{path}"));
                }
                Event::Status { state: 'C', path, .. } => s.notify(&format!("conflict:{path}"), "canopy: a conflict needs you", &format!("{path} changed in two places. Open canopy › Conflicts to choose.")),
                _ => {}
            }
            s.broadcast(v);
        })
    }
}

pub fn ctx_for(acct: &Account, settings: &Settings, first: bool) -> Ctx {
    Ctx {
        email: acct.email.clone(),
        export: acct.export,
        conflicts: acct.conflicts.clone(),
        ignore: Ignore::new(&acct.ignore_rules),
        trash_dir: std::env::var_os("CANOPY_TRASH_DIR").map(Into::into),
        transfers: settings.transfers.clamp(1, 16),
        sync_deletions: !first || settings.sync_offline_deletions,
        host: paths::hostname(),
    }
}

/// An engine for one root of an account, as configured now.
pub fn engine_for(s: &S, a: &Arc<Acct>, root: &Root, first: bool) -> Engine {
    let cfg = a.cfg.read().clone();
    let root_id = if root.cloud_id.is_empty() { a.root_id.read().clone() } else { root.cloud_id.clone() };
    Engine {
        cloud: a.cloud.clone(),
        db: a.db.clone(),
        ctx: ctx_for(&cfg, &s.settings.read(), first),
        root: root.clone(),
        root_id,
        sink: s.sink_for(&cfg.id),
        cancel: a.cancel.clone(),
        busy: a.busy_for(&root.id),
    }
}

/// The default roots of a new account.
pub fn default_roots(base: &std::path::Path) -> Vec<Root> {
    vec![Root { id: "mydrive".into(), kind: RootKind::MyDrive, local: base.join("My Drive"), name: "My Drive".into(), mode: Mode::TwoWay, ..Default::default() }]
}

fn ensure_shared_drives(s: &S, a: &Arc<Acct>) -> Result<bool> {
    if !a.cfg.read().sync_shared_drives {
        return Ok(false);
    }
    let drives = a.cloud.shared_drives()?;
    let mut changed = false;
    let mut cfg = a.cfg.write();
    let base = cfg.base.clone();
    for d in drives {
        let id = format!("sd-{}", d.id);
        if !cfg.roots.iter().any(|r| r.id == id) {
            cfg.roots.push(Root {
                id: id.clone(),
                kind: RootKind::SharedDrive,
                local: base.join("Shared drives").join(crate::engine::local_safe(&d.name)),
                cloud_id: d.id.clone(),
                drive_id: Some(d.id.clone()),
                name: d.name.clone(),
                ..Default::default()
            });
            if let Some(w) = &s.watcher {
                let r = cfg.roots.last().unwrap();
                let _ = std::fs::create_dir_all(&r.local);
                w.watch_root((cfg.id.clone(), id), &r.local);
            }
            changed = true;
        }
    }
    Ok(changed)
}

fn worker(s: S, a: Arc<Acct>, rx: Receiver<Trig>) {
    let mut dirty: HashSet<String> = HashSet::new();
    let mut full = true; // first cycle: every root
    let mut backoff = 0u32;
    let mut last_full = Instant::now();
    let mut last_drives = Instant::now() - Duration::from_secs(3600);
    loop {
        if s.quit.load(Ordering::Relaxed) {
            return;
        }
        let poll = Duration::from_secs(s.settings.read().poll_seconds.clamp(5, 600) * (1 << backoff.min(5)));
        if !full && dirty.is_empty() {
            match rx.recv_timeout(poll) {
                Ok(Trig::Local(r)) => {
                    dirty.insert(r);
                }
                Ok(Trig::Now) => full = true,
                Ok(Trig::Stop) => return,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Let a burst of writes settle.
            let settle = Instant::now();
            while settle.elapsed() < Duration::from_millis(800) {
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(Trig::Local(r)) => {
                        dirty.insert(r);
                    }
                    Ok(Trig::Now) => full = true,
                    Ok(Trig::Stop) => return,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(_) => return,
                }
            }
        }
        if s.settings.read().paused || a.cfg.read().paused {
            if a.set_phase("paused", "") {
                s.changed();
            }
            // Keep what changed for when it resumes.
            full = true;
            dirty.clear();
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Trig::Stop) => return,
                _ => continue,
            }
        }
        a.cancel.store(false, Ordering::Relaxed);
        if last_full.elapsed() > Duration::from_secs(15 * 60) {
            full = true; // a rescan now and then catches missed events
        }
        let r = cycle(&s, &a, full, &dirty, &mut last_drives);
        match r {
            Ok(()) => {
                backoff = 0;
                if full {
                    last_full = Instant::now();
                }
                full = false;
                dirty.clear();
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if drive::is_auth_error(&e) {
                    let hearth = a.cfg.read().hearth_id.is_some();
                    a.set_phase("signed_out", &if hearth { format!("{msg}") } else { "Sign in again to keep syncing.".into() });
                    s.notify("auth", "canopy: sign in again", &format!("Google Drive access for {} stopped working. Open canopy to sign in again{}.", a.cfg.read().email, if hearth { " (Settings › Internet Accounts)" } else { "" }));
                    backoff = 6;
                } else if drive::is_offline(&e) {
                    a.set_phase("offline", "No connection to Google Drive. canopy keeps trying.");
                    backoff = (backoff + 1).min(4);
                } else {
                    eprintln!("canopy: {}: {msg}", a.cfg.read().email);
                    a.set_phase("error", &msg);
                    backoff = (backoff + 1).min(4);
                }
                s.changed();
            }
        }
    }
}

fn cycle(s: &S, a: &Arc<Acct>, full: bool, dirty: &HashSet<String>, last_drives: &mut Instant) -> Result<()> {
    let _g = a.cycle.lock();
    if a.root_id.read().is_empty() {
        a.set_phase("starting", "Reading your Google Drive…");
        s.changed();
        let about = a.cloud.about()?;
        *a.root_id.write() = about.root_id.clone();
        a.db.set_kv("root_id", &about.root_id)?;
        let mut st = a.state.lock();
        st.quota_used = about.quota_used;
        st.quota_total = about.quota_total;
    }
    if last_drives.elapsed() > Duration::from_secs(600) {
        *last_drives = Instant::now();
        if ensure_shared_drives(s, a)? {
            s.save_accounts()?;
        }
    }
    let roots = a.cfg.read().roots.clone();
    let mut cloud_changed = full;
    let r = mirror::refresh(a.cloud.as_ref(), &a.db, None)?;
    cloud_changed |= !r.is_empty();
    let mut drives_changed: HashSet<String> = HashSet::new();
    for root in roots.iter().filter(|r| r.drive_id.is_some()) {
        let d = root.drive_id.clone().unwrap();
        if !mirror::refresh(a.cloud.as_ref(), &a.db, Some(&d))?.is_empty() {
            drives_changed.insert(root.id.clone());
        }
    }
    let todo: Vec<&Root> = roots.iter().filter(|r| !r.paused && (cloud_changed || dirty.contains(&r.id) || drives_changed.contains(&r.id))).collect();
    if todo.is_empty() {
        if a.set_phase("idle", "") {
            s.changed();
        }
        return Ok(());
    }
    a.set_phase("syncing", "");
    s.changed();
    let acct_id = a.id();
    for root in todo {
        if a.cancel.load(Ordering::Relaxed) {
            break;
        }
        let key = format!("{acct_id}/{}", root.id);
        let first = !s.started.lock().contains(&key);
        let eng = engine_for(s, a, root, first);
        let mut opts = PassOptions::default();
        for ((r, p), how) in a.forced.lock().iter() {
            if r == &root.id {
                opts.forced.insert(p.clone(), *how);
            }
        }
        {
            let c = a.confirmed.lock();
            opts.allow_empty = c.contains(&(root.id.clone(), "empty_root".into()));
            opts.confirm_deletes = c.contains(&(root.id.clone(), "mass_delete".into()));
        }
        let rep = eng.pass(&opts)?;
        s.started.lock().insert(key);
        a.forced.lock().retain(|(r, _), _| r != &root.id);
        a.confirmed.lock().retain(|(r, _)| r != &root.id);
        {
            let mut st = a.state.lock();
            match &rep.held {
                Some(h) => {
                    if st.held.insert(root.id.clone(), h.clone()).is_none() {
                        s.notify(&format!("held:{}", root.id), "canopy paused a folder", h);
                    }
                }
                None => {
                    st.held.remove(&root.id);
                }
            }
        }
        if !rep.unsynced.is_empty() {
            let mut cfg = a.cfg.write();
            if let Some(r) = cfg.roots.iter_mut().find(|r| r.id == root.id) {
                for p in &rep.unsynced {
                    r.selection.insert(p.clone(), false);
                }
            }
            drop(cfg);
            s.save_accounts()?;
        }
        if rep.errors > 0 {
            s.notify("errors", "canopy couldn't sync some files", &format!("{} item(s) need attention. Open canopy to see why.", rep.errors));
        }
    }
    {
        let mut st = a.state.lock();
        st.last_sync = crate::db::now();
    }
    a.set_phase("idle", "");
    s.changed();
    Ok(())
}

/// Bring up an account's runtime (worker thread, watches).
pub fn start_account(s: &S, cfg: Account, cloud: Arc<dyn Cloud>, drive: Option<Arc<Drive>>) -> Result<Arc<Acct>> {
    let db = Arc::new(Db::open(&paths::account_dir(&cfg.id).join("state.db"))?);
    let root_id = db.kv("root_id")?.unwrap_or_default();
    let (tx, rx) = unbounded();
    let a = Arc::new(Acct {
        cfg: RwLock::new(cfg),
        db,
        cloud,
        drive,
        root_id: RwLock::new(root_id),
        tx,
        state: Mutex::new(AcctState { phase: "starting".into(), ..Default::default() }),
        busy: Mutex::default(),
        cancel: Arc::new(AtomicBool::new(false)),
        forced: Mutex::default(),
        confirmed: Mutex::default(),
        cycle: Mutex::new(()),
    });
    if let Some(w) = &s.watcher {
        let cfg = a.cfg.read();
        for r in &cfg.roots {
            let _ = std::fs::create_dir_all(&r.local);
            w.watch_root((cfg.id.clone(), r.id.clone()), &r.local);
        }
    }
    s.accounts.write().push(a.clone());
    let (s2, a2) = (s.clone(), a.clone());
    std::thread::Builder::new().name(format!("sync {}", a.cfg.read().email)).spawn(move || worker(s2, a2, rx))?;
    Ok(a)
}

/// The Drive client of a signed-in account: tokens from hearth's Internet
/// Accounts when it was added there, else canopy's own sign-in.
pub fn drive_for(s: &S, cfg: &Account) -> Result<Arc<Drive>> {
    let settings = s.settings.read().clone();
    let agent = drive::agent(&settings.proxy);
    let tokens: Arc<dyn drive::Tokens> = match &cfg.hearth_id {
        Some(h) => Arc::new(crate::hearth::HearthTokens::new(h)),
        None => {
            let token = auth::load_token(&cfg.id)?;
            Arc::new(auth::Session::new(auth::Client { id: settings.client_id.clone(), secret: settings.client_secret.clone() }, token, agent.clone()))
        }
    };
    let db = Arc::new(Db::open(&paths::account_dir(&cfg.id).join("state.db"))?);
    Ok(Arc::new(Drive::new(agent, tokens, Some(db))))
}

pub fn new_shared(watch: bool) -> (S, Option<Receiver<RootKey>>) {
    let settings = Settings::load();
    throttle::UP.set_kib(settings.upload_limit);
    throttle::DOWN.set_kib(settings.download_limit);
    let (wtx, wrx) = unbounded();
    let watcher = if watch { Watcher::start(wtx).map_err(|e| eprintln!("canopy: no inotify ({e}); changes are found by scans")).ok() } else { None };
    let s = Arc::new(Shared {
        settings: RwLock::new(settings),
        accounts: RwLock::default(),
        subs: Mutex::default(),
        transfers: Mutex::default(),
        logins: Mutex::default(),
        watcher,
        quit: AtomicBool::new(false),
        started: Mutex::default(),
        tray: Mutex::new(None),
        hearth: AtomicBool::new(false),
        last_note: Mutex::default(),
    });
    (s, Some(wrx))
}

/// Run the daemon until `quit`.
pub fn run() -> Result<()> {
    let (s, wrx) = new_shared(true);
    // Only one canopyd.
    let sock = paths::socket_path();
    if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
        return Err(anyhow!("canopyd is already running"));
    }
    s.hearth.store(crate::hearth::available(), Ordering::Relaxed);
    for cfg in Accounts::load().accounts {
        let id = cfg.id.clone();
        match drive_for(&s, &cfg) {
            Ok(d) => {
                start_account(&s, cfg, d.clone(), Some(d))?;
            }
            Err(e) => {
                eprintln!("canopy: {id}: {e:#}");
                let a = start_account(&s, cfg, Arc::new(SignedOut), None)?;
                a.set_phase("signed_out", "Sign in again to keep syncing.");
            }
        }
    }
    if std::env::var_os("CANOPY_DEMO").is_some() && s.accounts.read().is_empty() {
        demo(&s)?;
    }
    // Disk changes to the right worker.
    if let Some(wrx) = wrx {
        let s2 = s.clone();
        std::thread::spawn(move || {
            for (acct, root) in wrx {
                if let Ok(a) = s2.account(&acct) {
                    let _ = a.tx.send(Trig::Local(root));
                }
            }
        });
    }
    // Internet Accounts changed (signed in again, allowed Drive, removed):
    // accounts that come from there try again.
    if s.hearth.load(Ordering::Relaxed) {
        let w = Arc::downgrade(&s);
        crate::hearth::watch(move || {
            let Some(s) = w.upgrade() else { return };
            for a in s.accounts.read().iter() {
                if a.cfg.read().hearth_id.is_some() {
                    let _ = a.tx.send(Trig::Now);
                }
            }
            s.broadcast(json!({"event": "hearth_changed"}));
            s.changed();
        });
    }
    crate::tray::start(&s);
    crate::handlers::serve(s.clone(), &sock)?;
    Ok(())
}

/// Stand-in cloud for an account that isn't signed in.
pub struct SignedOut;

impl Cloud for SignedOut {
    fn about(&self) -> Result<crate::cloud::About> {
        Err(drive::HttpError { status: 401, reason: "signed_out".into(), message: "not signed in".into() }.into())
    }
    fn shared_drives(&self) -> Result<Vec<crate::cloud::SharedDrive>> {
        self.about().map(|_| vec![])
    }
    fn start_token(&self, _: Option<&str>) -> Result<String> {
        self.about().map(|_| String::new())
    }
    fn changes(&self, _: Option<&str>, _: &str) -> Result<crate::cloud::ChangePage> {
        Err(self.about().unwrap_err())
    }
    fn list_all(&self, _: Option<&str>) -> Result<Vec<crate::cloud::CloudItem>> {
        self.about().map(|_| vec![])
    }
    fn get(&self, _: &str) -> Result<Option<crate::cloud::CloudItem>> {
        self.about().map(|_| None)
    }
    fn create_folder(&self, _: &str, _: &str) -> Result<crate::cloud::CloudItem> {
        Err(self.about().unwrap_err())
    }
    fn upload(&self, _: &str, _: &str, _: Option<&str>, _: &std::path::Path, _: Option<&str>, _: f64, _: crate::cloud::Progress) -> Result<crate::cloud::CloudItem> {
        Err(self.about().unwrap_err())
    }
    fn download(&self, _: &str, _: &std::path::Path, _: crate::cloud::Progress) -> Result<()> {
        self.about().map(|_| ())
    }
    fn export(&self, _: &str, _: &str, _: &std::path::Path) -> Result<()> {
        self.about().map(|_| ())
    }
    fn update_meta(&self, _: &str, _: Option<&str>, _: Option<(&str, &str)>) -> Result<crate::cloud::CloudItem> {
        Err(self.about().unwrap_err())
    }
    fn trash(&self, _: &str) -> Result<()> {
        self.about().map(|_| ())
    }
    fn copy(&self, _: &str, _: &str, _: &str) -> Result<crate::cloud::CloudItem> {
        Err(self.about().unwrap_err())
    }
    fn set_starred(&self, _: &str, _: bool) -> Result<()> {
        self.about().map(|_| ())
    }
}

/// CANOPY_DEMO=1: a sample account on the in-memory Drive (tests,
/// screenshots). Nothing leaves the machine.
pub fn demo(s: &S) -> Result<()> {
    use crate::fake::{FakeCloud, ROOT};
    let f = Arc::new(FakeCloud::new());
    let docs = f.mkdir(ROOT, "Documents");
    f.put_gdoc(&docs, "Budget 2026", "application/vnd.google-apps.spreadsheet");
    f.put_gdoc(&docs, "Letter to the school", "application/vnd.google-apps.document");
    f.put(&docs, "Tax return.pdf", &vec![b'%'; 180_000]);
    let photos = f.mkdir(ROOT, "Photos");
    let y = f.mkdir(&photos, "2026");
    for i in 1..=6 {
        f.put(&y, &format!("IMG_{:04}.jpg", 4100 + i), &vec![i as u8; 40_000 * i]);
    }
    let proj = f.mkdir(ROOT, "Projects");
    f.put(&proj, "notes.md", b"# Notes\n");
    f.put_gdoc(&proj, "Slides for Friday", "application/vnd.google-apps.presentation");
    let archive = f.mkdir(ROOT, "Archive");
    f.put(&archive, "old backup.zip", &vec![0u8; 300_000]);
    f.put(ROOT, "ben.kdbx", b"keepass");
    let base = paths::default_base().join("demo@example.com");
    let mut roots = default_roots(&base);
    // Archive stays in the cloud only (selective sync).
    roots[0].selection.insert("Archive".into(), false);
    let cfg = Account { id: "demo@example.com".into(), email: "demo@example.com".into(), name: "Demo".into(), base, roots, ..Default::default() };
    start_account(s, cfg, f, None)?;
    Ok(())
}
