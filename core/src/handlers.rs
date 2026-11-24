//! The socket server and its commands (protocol in api.rs). Each
//! connection gets a thread; slow commands (sign-in, sharing) only hold up
//! their own connection.

use crate::auth;
use crate::cloud::CloudItem;
use crate::config::{Account, ConflictPolicy, ExportOption, Mode, Resolution, Root, RootKind, Settings};
use crate::daemon::{self, Acct, Trig, S};
use crate::engine::{self, local_safe};
use crate::gdocs;
use crate::ignore::Ignore;
use crate::paths;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub fn serve(s: S, sock: &Path) -> Result<()> {
    let dir = sock.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::remove_file(sock);
    let l = UnixListener::bind(sock)?;
    eprintln!("canopyd: listening on {}", sock.display());
    for c in l.incoming() {
        if s.quit.load(Ordering::Relaxed) {
            break;
        }
        let Ok(c) = c else { continue };
        let s2 = s.clone();
        std::thread::spawn(move || {
            let _ = connection(s2, c);
        });
    }
    Ok(())
}

fn connection(s: S, c: UnixStream) -> Result<()> {
    let w = Arc::new(parking_lot::Mutex::new(c.try_clone()?));
    let r = BufReader::new(c);
    for line in r.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(w.lock(), "{}", json!({"ok": false, "error": format!("bad request: {e}")}))?;
                continue;
            }
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let cmd = req.get("cmd").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if cmd == "subscribe" {
            let rx = s.subscribe();
            let w2 = w.clone();
            std::thread::spawn(move || {
                for ev in rx {
                    if writeln!(w2.lock(), "{ev}").is_err() {
                        return;
                    }
                }
            });
            writeln!(w.lock(), "{}", json!({"id": id, "ok": true, "result": null}))?;
            continue;
        }
        let reply = match handle(&s, &cmd, &req) {
            Ok(v) => json!({"id": id, "ok": true, "result": v}),
            Err(e) => json!({"id": id, "ok": false, "error": format!("{e:#}")}),
        };
        writeln!(w.lock(), "{reply}")?;
        if cmd == "quit" {
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        }
    }
    Ok(())
}

fn str_arg<'a>(req: &'a Value, k: &str) -> &'a str {
    req.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// The account and root holding a local path, and the path inside it.
pub fn locate(s: &S, path: &Path) -> Option<(Arc<Acct>, Root, String)> {
    let mut best: Option<(Arc<Acct>, Root, String)> = None;
    for a in s.accounts.read().iter() {
        for r in &a.cfg.read().roots {
            if let Ok(rel) = path.strip_prefix(&r.local) {
                let better = best.as_ref().map(|(_, b, _)| r.local.as_os_str().len() > b.local.as_os_str().len()).unwrap_or(true);
                if better {
                    best = Some((a.clone(), r.clone(), rel.to_string_lossy().into_owned()));
                }
            }
        }
    }
    best
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.into()
    } else {
        format!("{dir}/{name}")
    }
}

fn rank(c: char) -> u8 {
    match c {
        'C' => 5,
        'E' => 4,
        'P' => 3,
        'S' => 1,
        _ => 0,
    }
}

/// The badge of one path in a root: S synced, P syncing or waiting,
/// E error, C conflict, I ignored. Folders show the strongest inside.
fn path_state(a: &Acct, root: &Root, rel: &str, is_dir: bool, issues: &[crate::db::Issue], busy: &std::collections::HashSet<String>, ignore: &Ignore) -> char {
    let inside = |p: &str| p == rel || (is_dir && (rel.is_empty() || (p.len() > rel.len() && p.starts_with(rel) && p.as_bytes()[rel.len()] == b'/')));
    let mut best = ' ';
    for i in issues.iter().filter(|i| i.root == root.id && inside(&i.path)) {
        let c = if i.kind == "conflict" { 'C' } else if i.kind == "error" { 'E' } else { continue };
        if rank(c) > rank(best) {
            best = c;
        }
    }
    if best != ' ' {
        return best;
    }
    if busy.iter().any(|p| inside(p)) {
        return 'P';
    }
    if !rel.is_empty() && ignore.ignored(rel, is_dir) {
        return 'I';
    }
    if rel.is_empty() || a.db.base_get(&root.id, rel).ok().flatten().is_some() {
        return 'S';
    }
    'P'
}

fn folder_status(s: &S, dir: &Path) -> Result<Value> {
    let mut out = serde_json::Map::new();
    let names: Vec<(String, bool)> = std::fs::read_dir(dir)?.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().map(|t| t.is_dir()).unwrap_or(false))).collect();
    // Root folders listed here (e.g. ~/Canopy/me@x showing "My Drive").
    for a in s.accounts.read().iter() {
        let st = a.state.lock().clone();
        for r in &a.cfg.read().roots {
            if r.local.parent() == Some(dir) {
                let name = r.local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let issues = a.db.issues().unwrap_or_default();
                let busy = a.busy_for(&r.id).lock().clone();
                let ig = Ignore::new(&a.cfg.read().ignore_rules);
                let mut c = path_state(a, r, "", true, &issues, &busy, &ig);
                if st.held.contains_key(&r.id) || st.phase == "signed_out" {
                    c = 'E';
                } else if st.phase == "syncing" && c == 'S' && !busy.is_empty() {
                    c = 'P';
                }
                out.insert(name, json!(format!("{c}/canopy")));
            }
        }
    }
    if let Some((a, root, rel)) = locate(s, dir) {
        let issues = a.db.issues().unwrap_or_default();
        let busy = a.busy_for(&root.id).lock().clone();
        let ig = Ignore::new(&a.cfg.read().ignore_rules);
        for (n, is_dir) in names {
            if n == crate::ignore::PART_DIR {
                continue;
            }
            let c = path_state(&a, &root, &join(&rel, &n), is_dir, &issues, &busy, &ig);
            out.insert(n, json!(format!("{c}/canopy")));
        }
        out.insert("__root".into(), json!(root.local));
    }
    Ok(Value::Object(out))
}

/// Where a cloud item would be under a root (local relative path), if it
/// is under it.
fn cloud_rel(a: &Acct, root_id: &str, item: &CloudItem) -> Option<String> {
    let export = a.cfg.read().export;
    let mut parts = vec![if item.is_google_format() { local_safe(&gdocs::local_name(item, export)) } else { local_safe(&item.name) }];
    let mut cur = item.parents.first().cloned()?;
    for _ in 0..200 {
        if cur == root_id {
            parts.reverse();
            return Some(parts.join("/"));
        }
        let p = a.db.item(&cur).ok()??;
        parts.push(local_safe(&p.name));
        cur = p.parents.first().cloned()?;
    }
    None
}

fn item_json(a: &Acct, it: &CloudItem) -> Value {
    let roots = a.cfg.read().roots.clone();
    let my_root = a.root_id.read().clone();
    let mut sync = "cloud";
    let mut local = Value::Null;
    let mut root_id = Value::Null;
    let mut rel_v = Value::Null;
    for r in &roots {
        let rid = if r.cloud_id.is_empty() { my_root.clone() } else { r.cloud_id.clone() };
        if it.id == rid {
            sync = "synced";
            local = json!(r.local);
            root_id = json!(r.id);
            rel_v = json!("");
            break;
        }
        if let Some(rel) = cloud_rel(a, &rid, it) {
            root_id = json!(r.id);
            rel_v = json!(rel);
            let synced = a.db.base_get(&r.id, &rel).ok().flatten().is_some();
            sync = if synced {
                "synced"
            } else if r.selected(&rel) {
                "pending"
            } else if it.is_folder() && r.selected_below(&rel) {
                "partial"
            } else {
                "cloud"
            };
            if synced {
                local = json!(r.local.join(&rel));
            }
            break;
        }
    }
    json!({
        "id": it.id, "name": it.name, "mime": it.mime, "size": it.size, "mtime": it.mtime,
        "folder": it.is_folder(), "google": it.is_google_format(), "starred": it.starred,
        "shared": it.shared, "owned": it.owned_by_me, "canEdit": it.can_edit,
        "web": gdocs::web_url(it), "sync": sync, "local": local, "root": root_id, "rel": rel_v,
    })
}

fn sort_items(v: &mut [CloudItem]) {
    v.sort_by(|a, b| b.is_folder().cmp(&a.is_folder()).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

fn browse(s: &S, req: &Value) -> Result<Value> {
    let a = s.account(str_arg(req, "account"))?;
    let section = str_arg(req, "section");
    let folder = str_arg(req, "folder");
    let mut items = if !folder.is_empty() {
        a.db.children(folder)?
    } else {
        match section {
            "shared" => a.db.items_where("json_extract(json, '$.owned_by_me') = 0 AND (parent IS NULL OR parent NOT IN (SELECT id FROM items)) AND drive IS NULL AND ?1 = ?1", "", 2000)?,
            "starred" => a.db.items_where("json_extract(json, '$.starred') = 1 AND ?1 = ?1", "", 2000)?,
            "shared_drives" => {
                let drives = a.cloud.shared_drives()?;
                return Ok(json!(drives.iter().map(|d| json!({"id": d.id, "name": d.name, "folder": true, "mime": crate::cloud::FOLDER_MIME, "sync": if a.cfg.read().roots.iter().any(|r| r.drive_id.as_deref() == Some(&d.id)) { "synced" } else { "cloud" }})).collect::<Vec<_>>()));
            }
            _ => {
                let rid = a.root_id.read().clone();
                a.db.children(&rid)?
            }
        }
    };
    sort_items(&mut items);
    Ok(json!(items.iter().map(|it| item_json(&a, it)).collect::<Vec<_>>()))
}

/// The cloud item behind a local path (synced items and link files).
fn item_at(s: &S, path: &Path) -> Result<(Arc<Acct>, CloudItem)> {
    if gdocs::is_link_name(&path.to_string_lossy()) {
        if let Some(l) = gdocs::read_link(path) {
            let a = s.accounts.read().iter().find(|a| a.cfg.read().email == l.account_email).cloned().or_else(|| s.accounts.read().first().cloned()).ok_or_else(|| anyhow!("no account"))?;
            let it = a.db.item(&l.file_id)?.unwrap_or(CloudItem { id: l.file_id.clone(), web_link: Some(l.url.clone()), ..Default::default() });
            return Ok((a, it));
        }
    }
    let (a, root, rel) = locate(s, path).ok_or_else(|| anyhow!("{} isn't in a synced folder", path.display()))?;
    let id = if rel.is_empty() {
        if root.cloud_id.is_empty() { a.root_id.read().clone() } else { root.cloud_id.clone() }
    } else {
        a.db.base_get(&root.id, &rel)?.ok_or_else(|| anyhow!("{} isn't synced yet", path.display()))?.cl_id
    };
    let it = a.db.item(&id)?.ok_or_else(|| anyhow!("not found in Google Drive"))?;
    Ok((a, it))
}

fn target(s: &S, req: &Value) -> Result<(Arc<Acct>, CloudItem)> {
    let path = str_arg(req, "path");
    if !path.is_empty() {
        return item_at(s, Path::new(path));
    }
    let a = s.account(str_arg(req, "account"))?;
    let id = str_arg(req, "id");
    let it = a.db.item(id)?.ok_or_else(|| anyhow!("not found"))?;
    Ok((a, it))
}

fn status(s: &S) -> Value {
    let accts: Vec<Value> = s
        .accounts
        .read()
        .iter()
        .map(|a| {
            let cfg = a.cfg.read();
            let st = a.state.lock().clone();
            let issues = a.db.issues().unwrap_or_default();
            json!({
                "id": cfg.id, "email": cfg.email, "name": cfg.name, "photo": cfg.photo, "base": cfg.base,
                "paused": cfg.paused, "export": cfg.export, "conflicts": cfg.conflicts, "ignore_rules": cfg.ignore_rules,
                "sync_shared_drives": cfg.sync_shared_drives, "sync_shared_with_me": cfg.sync_shared_with_me, "hearth_id": cfg.hearth_id,
                "roots": cfg.roots, "state": st,
                "errors": issues.iter().filter(|i| i.kind != "conflict").count(),
                "conflicts_count": issues.iter().filter(|i| i.kind == "conflict").count(),
            })
        })
        .collect();
    let transfers: Vec<Value> = s.transfers.lock().values().map(|(v, _)| v.clone()).collect();
    json!({"state": s.overall(), "paused": s.settings.read().paused, "accounts": accts, "transfers": transfers, "has_client": !s.settings.read().client_id.is_empty(), "hearth": s.hearth.load(Ordering::Relaxed), "version": env!("CARGO_PKG_VERSION")})
}

/// Add (or sign in again to) an account. `hearth`: an Internet Accounts
/// account id (tokens come from there); otherwise `token` is canopy's own
/// refresh token.
fn add_account(s: &S, token: Option<String>, hearth: Option<String>) -> Result<Value> {
    let settings = s.settings.read().clone();
    let agent = crate::drive::agent(&settings.proxy);
    let tokens: Arc<dyn crate::drive::Tokens> = match (&hearth, &token) {
        (Some(h), _) => Arc::new(crate::hearth::HearthTokens::new(h)),
        (None, Some(t)) => Arc::new(auth::Session::new(auth::Client { id: settings.client_id.clone(), secret: settings.client_secret.clone() }, t.clone(), agent.clone())),
        _ => bail!("nothing to sign in with"),
    };
    let probe = crate::drive::Drive::new(agent, tokens, None);
    let about = crate::cloud::Cloud::about(&probe)?;
    let id = about.email.to_lowercase();
    if let Some(t) = &token {
        auth::store_token(&id, &about.email, t)?;
    }
    let cfg = match s.account(&id) {
        Ok(a) => {
            // Signing in again (or moving to Internet Accounts): new runtime,
            // same folders and choices.
            let _ = a.tx.send(Trig::Stop);
            a.cancel.store(true, Ordering::Relaxed);
            s.accounts.write().retain(|x| !Arc::ptr_eq(x, &a));
            let mut cfg = a.cfg.read().clone();
            if hearth.is_some() && cfg.hearth_id.is_none() {
                auth::forget_token(&id);
            }
            cfg.hearth_id = hearth.clone();
            cfg
        }
        Err(_) => {
            let base = paths::default_base().join(&about.email);
            Account { id: id.clone(), email: about.email.clone(), name: about.name.clone(), photo: about.photo.clone(), base: base.clone(), roots: daemon::default_roots(&base), hearth_id: hearth.clone(), ..Default::default() }
        }
    };
    let d = daemon::drive_for(s, &cfg)?;
    daemon::start_account(s, cfg, d.clone(), Some(d))?;
    s.save_accounts()?;
    let _ = set_autostart(s.settings.read().start_at_login);
    s.changed();
    Ok(json!({"account": id, "email": about.email}))
}

/// Change the account's export option: Google files on disk are replaced
/// by their new form (the old copies go to the trash).
fn reexport(s: &S, a: &Arc<Acct>) -> Result<()> {
    let _g = a.cycle.lock();
    for r in a.cfg.read().roots.clone() {
        for b in a.db.base_all(&r.id)?.into_iter().filter(|b| b.kind == 'g') {
            let p = r.local.join(&b.path);
            if p.exists() {
                let _ = crate::trash::trash(&p, std::env::var_os("CANOPY_TRASH_DIR").map(PathBuf::from).as_deref());
            }
            a.db.base_remove_tree(&r.id, &b.path)?;
        }
    }
    drop(_g);
    let _ = a.tx.send(Trig::Now);
    s.changed();
    Ok(())
}

/// Start at login: an XDG autostart entry (works under arbor/hearth and
/// any other desktop); a systemd user unit, when installed, is left to the
/// system configuration.
pub fn set_autostart(on: bool) -> Result<()> {
    let p = paths::config_home().join("autostart/canopy.desktop");
    if on {
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, "[Desktop Entry]\nType=Application\nName=Canopy\nComment=Google Drive sync\nExec=canopy start\nIcon=canopy\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n")?;
    } else {
        let _ = std::fs::remove_file(&p);
    }
    Ok(())
}

pub fn handle(s: &S, cmd: &str, req: &Value) -> Result<Value> {
    match cmd {
        "status" => Ok(status(s)),
        "folder_status" => folder_status(s, Path::new(str_arg(req, "dir"))),
        "file_status" => {
            let p = Path::new(str_arg(req, "path"));
            let (a, root, rel) = locate(s, p).ok_or_else(|| anyhow!("not in a synced folder"))?;
            let issues = a.db.issues()?;
            let busy = a.busy_for(&root.id).lock().clone();
            let ig = Ignore::new(&a.cfg.read().ignore_rules);
            let c = path_state(&a, &root, &rel, p.is_dir(), &issues, &busy, &ig);
            let item = a.db.base_get(&root.id, &rel)?.and_then(|b| a.db.item(&b.cl_id).ok().flatten());
            let mine: Vec<&crate::db::Issue> = issues.iter().filter(|i| i.root == root.id && i.path == rel).collect();
            Ok(json!({"state": c.to_string(), "account": a.id(), "root": root.id, "rel": rel, "item": item.map(|i| item_json(&a, &i)), "issues": mine}))
        }
        "pause" | "resume" => {
            let on = cmd == "pause";
            let acct = str_arg(req, "account");
            if acct.is_empty() {
                let mut st = s.settings.write();
                st.paused = on;
                st.save()?;
            } else {
                let a = s.account(acct)?;
                a.cfg.write().paused = on;
                s.save_accounts()?;
            }
            for a in s.accounts.read().iter() {
                if on {
                    a.cancel.store(true, Ordering::Relaxed);
                } else {
                    let _ = a.tx.send(Trig::Now);
                }
            }
            s.changed();
            Ok(Value::Null)
        }
        "sync_now" => {
            for a in s.accounts.read().iter() {
                let _ = a.tx.send(Trig::Now);
            }
            Ok(Value::Null)
        }
        "activity" => {
            let before = req.get("before").and_then(|x| x.as_f64()).unwrap_or(f64::MAX);
            let limit = req.get("limit").and_then(|x| x.as_u64()).unwrap_or(100) as usize;
            let mut all = Vec::new();
            for a in s.accounts.read().iter() {
                for x in a.db.activity(0, 2000)? {
                    if x.at < before {
                        let mut v = serde_json::to_value(&x)?;
                        v["account"] = json!(a.id());
                        all.push(v);
                    }
                }
            }
            all.sort_by(|a, b| b["at"].as_f64().partial_cmp(&a["at"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
            all.truncate(limit);
            Ok(json!(all))
        }
        "issues" => {
            let mut all = Vec::new();
            for a in s.accounts.read().iter() {
                let roots = a.cfg.read().roots.clone();
                for i in a.db.issues()? {
                    let mut v = serde_json::to_value(&i)?;
                    v["account"] = json!(a.id());
                    if let Some(r) = roots.iter().find(|r| r.id == i.root) {
                        v["local"] = json!(r.local.join(&i.path));
                        v["root_name"] = json!(r.name);
                    }
                    all.push(v);
                }
            }
            Ok(json!(all))
        }
        "retry" | "dismiss" => {
            let id = req.get("id").and_then(|x| x.as_i64());
            for a in s.accounts.read().iter() {
                if str_arg(req, "account").is_empty() || a.id() == str_arg(req, "account") {
                    for i in a.db.issues()? {
                        if id.map(|x| x == i.id).unwrap_or(i.kind == "error" || cmd == "dismiss") {
                            a.db.clear_issue_id(i.id)?;
                        }
                    }
                }
                if cmd == "retry" {
                    let _ = a.tx.send(Trig::Now);
                }
            }
            s.changed();
            Ok(Value::Null)
        }
        "resolve" => {
            let a = s.account(str_arg(req, "account"))?;
            let how: Resolution = serde_json::from_value(req.get("choice").cloned().unwrap_or(Value::Null))?;
            let (root, path) = (str_arg(req, "root").to_string(), str_arg(req, "path").to_string());
            if req.get("always").and_then(|x| x.as_bool()).unwrap_or(false) {
                // "Use this for all conflicts like this."
                let sub = a.db.issues()?.into_iter().find(|i| i.root == root && i.path == path).map(|i| i.sub).unwrap_or_default();
                let mut cfg = a.cfg.write();
                match sub.as_str() {
                    "edit_edit" => cfg.conflicts.edit_edit = how,
                    "local_edit_cloud_delete" => cfg.conflicts.local_edit_cloud_delete = how,
                    "local_delete_cloud_edit" => cfg.conflicts.local_delete_cloud_edit = how,
                    _ => {}
                }
                drop(cfg);
                s.save_accounts()?;
            }
            a.db.clear_issue(&root, &path, "conflict")?;
            a.forced.lock().insert((root, path), how);
            let _ = a.tx.send(Trig::Now);
            s.changed();
            Ok(Value::Null)
        }
        "preview" => {
            // What a pass would do now (nothing is changed): {root: [lines]}.
            let a = s.account(str_arg(req, "account"))?;
            let want = str_arg(req, "root");
            let mut out = serde_json::Map::new();
            let _g = a.cycle.lock();
            let _ = crate::mirror::refresh(a.cloud.as_ref(), &a.db, None);
            for r in a.cfg.read().roots.clone().into_iter().filter(|r| want.is_empty() || r.id == want) {
                let eng = daemon::engine_for(s, &a, &r, false);
                let rep = eng.pass(&crate::engine::PassOptions { dry_run: true, ..Default::default() })?;
                let lines: Vec<String> = rep.plan.into_iter().filter(|l| !l.starts_with("matches already")).collect();
                out.insert(r.id.clone(), json!({"name": r.name, "local": r.local, "plan": lines, "held": rep.held}));
            }
            Ok(Value::Object(out))
        }
        "confirm" => {
            let a = s.account(str_arg(req, "account"))?;
            let root = str_arg(req, "root").to_string();
            a.confirmed.lock().insert((root.clone(), str_arg(req, "what").to_string()));
            a.db.clear_issue(&root, "", "attention")?;
            let _ = a.tx.send(Trig::Now);
            Ok(Value::Null)
        }
        "login_begin" => {
            let st = s.settings.read().clone();
            let p = auth::begin(&auth::Client { id: st.client_id, secret: st.client_secret })?;
            let key = format!("l{}", crate::db::now().to_bits());
            let url = p.url.clone();
            s.logins.lock().insert(key.clone(), p);
            Ok(json!({"login": key, "url": url}))
        }
        "login_wait" => {
            let p = s.logins.lock().remove(str_arg(req, "login")).ok_or_else(|| anyhow!("that sign-in was cancelled"))?;
            let agent = crate::drive::agent(&s.settings.read().proxy);
            let token = p.wait(&agent, Duration::from_secs(600))?;
            add_account(s, Some(token), None)
        }
        "login_paste" => {
            let key = str_arg(req, "login");
            let token = {
                let logins = s.logins.lock();
                let p = logins.get(key).ok_or_else(|| anyhow!("that sign-in was cancelled"))?;
                p.finish_pasted(&crate::drive::agent(&s.settings.read().proxy), str_arg(req, "text"))?
            };
            s.logins.lock().remove(key);
            add_account(s, Some(token), None)
        }
        "hearth_accounts" => {
            // Google accounts in Settings › Internet Accounts, and whether
            // each can sync (signed in with the full Drive permission).
            if !crate::hearth::available() {
                return Ok(json!({"available": false}));
            }
            let mine: Vec<(String, Option<String>)> = s.accounts.read().iter().map(|a| (a.cfg.read().email.to_lowercase(), a.cfg.read().hearth_id.clone())).collect();
            let list: Vec<Value> = crate::hearth::google_accounts()?
                .into_iter()
                .map(|a| {
                    let email = a["email"].as_str().unwrap_or("").to_string();
                    let id = a["id"].as_str().unwrap_or("").to_string();
                    let uses: Vec<String> = serde_json::from_value(a["uses"].clone()).unwrap_or_default();
                    let linked = mine.iter().any(|(e, h)| h.as_deref() == Some(id.as_str()) || (e == &email.to_lowercase() && h.is_some()));
                    let in_canopy = mine.iter().any(|(e, _)| e == &email.to_lowercase());
                    json!({"id": id, "email": email, "name": a["name"], "signed_out": a["signed_out"].as_bool().unwrap_or(false),
                           "can_sync": uses.iter().any(|u| u == crate::hearth::PURPOSE), "linked": linked, "in_canopy": in_canopy})
                })
                .collect();
            Ok(json!({"available": true, "registered": crate::hearth::google_registered(), "accounts": list}))
        }
        "hearth_link" => add_account(s, None, Some(str_arg(req, "id").to_string())),
        "hearth_signin" => {
            crate::hearth::sign_in_again(str_arg(req, "id"))?;
            Ok(Value::Null)
        }
        "hearth_open" => Ok(json!(crate::hearth::open_settings(str_arg(req, "arg")))),
        "login_cancel" => {
            s.logins.lock().remove(str_arg(req, "login"));
            Ok(Value::Null)
        }
        "account_remove" => {
            let a = s.account(str_arg(req, "account"))?;
            let id = a.id();
            a.cancel.store(true, Ordering::Relaxed);
            let _ = a.tx.send(Trig::Stop);
            if let Some(w) = &s.watcher {
                for r in &a.cfg.read().roots {
                    w.unwatch_root(&(id.clone(), r.id.clone()));
                }
            }
            s.accounts.write().retain(|x| !Arc::ptr_eq(x, &a));
            if a.cfg.read().hearth_id.is_none() {
                auth::forget_token(&id);
            }
            let _ = std::fs::remove_dir_all(paths::account_dir(&id));
            s.save_accounts()?;
            s.changed();
            Ok(json!({"kept": a.cfg.read().base}))
        }
        "settings" => Ok(serde_json::to_value(&*s.settings.read())?),
        "set_settings" => {
            let mut v = serde_json::to_value(&*s.settings.read())?;
            if let (Some(o), Some(n)) = (v.as_object_mut(), req.get("settings").and_then(|x| x.as_object())) {
                for (k, x) in n {
                    o.insert(k.clone(), x.clone());
                }
            }
            let new: Settings = serde_json::from_value(v)?;
            crate::throttle::UP.set_kib(new.upload_limit);
            crate::throttle::DOWN.set_kib(new.download_limit);
            set_autostart(new.start_at_login)?;
            new.save()?;
            *s.settings.write() = new;
            s.changed();
            Ok(Value::Null)
        }
        "set_account" => {
            let a = s.account(str_arg(req, "account"))?;
            let mut reexp = false;
            {
                let mut cfg = a.cfg.write();
                if let Some(e) = req.get("export") {
                    let e: ExportOption = serde_json::from_value(e.clone())?;
                    reexp = e != cfg.export;
                    cfg.export = e;
                }
                if let Some(c) = req.get("conflicts") {
                    cfg.conflicts = serde_json::from_value::<ConflictPolicy>(c.clone())?;
                }
                if let Some(r) = req.get("ignore_rules") {
                    cfg.ignore_rules = serde_json::from_value(r.clone())?;
                }
                if let Some(b) = req.get("sync_shared_drives").and_then(|x| x.as_bool()) {
                    cfg.sync_shared_drives = b;
                }
                if let Some(b) = req.get("paused").and_then(|x| x.as_bool()) {
                    cfg.paused = b;
                }
            }
            s.save_accounts()?;
            if reexp {
                reexport(s, &a)?;
            } else {
                let _ = a.tx.send(Trig::Now);
            }
            s.changed();
            Ok(Value::Null)
        }
        "browse" => browse(s, req),
        "search" => {
            let text = str_arg(req, "text");
            let mut out = Vec::new();
            for a in s.accounts.read().iter() {
                let mut items = match &a.drive {
                    Some(d) if req.get("online").and_then(|x| x.as_bool()).unwrap_or(true) => d.search(text).unwrap_or_else(|_| a.db.items_where("name LIKE '%' || ?1 || '%'", text, 200).unwrap_or_default()),
                    _ => a.db.items_where("name LIKE '%' || ?1 || '%'", text, 200)?,
                };
                sort_items(&mut items);
                for it in items {
                    let mut v = item_json(a, &it);
                    v["account"] = json!(a.id());
                    out.push(v);
                }
            }
            Ok(json!(out))
        }
        "select" => {
            let a = s.account(str_arg(req, "account"))?;
            let root_id = str_arg(req, "root").to_string();
            let rel = str_arg(req, "path").to_string();
            let on = req.get("synced").and_then(|x| x.as_bool()).unwrap_or(true);
            let root = {
                let mut cfg = a.cfg.write();
                let r = cfg.roots.iter_mut().find(|r| r.id == root_id).ok_or_else(|| anyhow!("no such sync folder"))?;
                // Drop rules below (the new choice covers them).
                let prefix = format!("{rel}/");
                r.selection.retain(|k, _| !(rel.is_empty() || k.starts_with(&prefix)));
                r.selection.insert(rel.clone(), on);
                r.clone()
            };
            s.save_accounts()?;
            if !on {
                let (s2, a2) = (s.clone(), a.clone());
                std::thread::spawn(move || {
                    let _g = a2.cycle.lock();
                    let eng = daemon::engine_for(&s2, &a2, &root, false);
                    if let Err(e) = eng.unsync(&rel) {
                        eprintln!("canopy: unsync {rel}: {e:#}");
                    }
                    s2.changed();
                });
            } else {
                let _ = a.tx.send(Trig::Now);
            }
            s.changed();
            Ok(Value::Null)
        }
        "add_folder" => {
            // Sync any local folder (Local Selective Sync), or a cloud
            // folder to a place of your choosing.
            let a = s.account(str_arg(req, "account"))?;
            let local = PathBuf::from(str_arg(req, "local"));
            if local.as_os_str().is_empty() {
                bail!("choose a folder");
            }
            if locate(s, &local).is_some() {
                bail!("{} is already inside a synced folder", local.display());
            }
            let mode: Mode = req.get("mode").map(|m| serde_json::from_value(m.clone())).transpose()?.unwrap_or_default();
            let cloud_id = match str_arg(req, "cloud") {
                "" => {
                    let parent = match str_arg(req, "parent") {
                        "" => a.root_id.read().clone(),
                        p => p.to_string(),
                    };
                    let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Folder".into());
                    // Merge with a folder of that name if there is one.
                    match a.db.children(&parent)?.into_iter().find(|c| c.is_folder() && c.name == name) {
                        Some(f) => f.id,
                        None => a.cloud.create_folder(&parent, &engine::cloud_name(&name))?.id,
                    }
                }
                id => id.to_string(),
            };
            std::fs::create_dir_all(&local)?;
            let root = Root {
                id: format!("c-{}", &format!("{:x}", crate::db::now().to_bits())[4..]),
                kind: RootKind::Custom,
                name: local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                local: local.clone(),
                cloud_id,
                mode,
                ..Default::default()
            };
            a.cfg.write().roots.push(root.clone());
            s.save_accounts()?;
            if let Some(w) = &s.watcher {
                w.watch_root((a.id(), root.id.clone()), &local);
            }
            let _ = a.tx.send(Trig::Now);
            s.changed();
            Ok(serde_json::to_value(&root)?)
        }
        "remove_root" => {
            let a = s.account(str_arg(req, "account"))?;
            let rid = str_arg(req, "root").to_string();
            let _g = a.cycle.lock();
            a.cfg.write().roots.retain(|r| r.id != rid);
            a.db.base_clear_root(&rid)?;
            if let Some(w) = &s.watcher {
                w.unwatch_root(&(a.id(), rid));
            }
            drop(_g);
            s.save_accounts()?;
            s.changed();
            Ok(Value::Null)
        }
        "set_mode" => {
            let a = s.account(str_arg(req, "account"))?;
            let rid = str_arg(req, "root");
            let mode: Mode = serde_json::from_value(req.get("mode").cloned().unwrap_or(Value::Null))?;
            if let Some(r) = a.cfg.write().roots.iter_mut().find(|r| r.id == rid) {
                r.mode = mode;
            }
            s.save_accounts()?;
            let _ = a.tx.send(Trig::Now);
            s.changed();
            Ok(Value::Null)
        }
        "move_base" => {
            let a = s.account(str_arg(req, "account"))?;
            let to = PathBuf::from(str_arg(req, "to"));
            if to.exists() && std::fs::read_dir(&to).map(|mut d| d.next().is_some()).unwrap_or(true) {
                bail!("{} isn't empty", to.display());
            }
            let _g = a.cycle.lock();
            let from = a.cfg.read().base.clone();
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            let _ = std::fs::remove_dir(&to);
            if std::fs::rename(&from, &to).is_err() {
                bail!("can't move {} there (another disk?); move it with the file manager and choose Locate…", from.display());
            }
            {
                let mut cfg = a.cfg.write();
                cfg.base = to.clone();
                for r in cfg.roots.iter_mut() {
                    if let Ok(rel) = r.local.strip_prefix(&from) {
                        r.local = to.join(rel);
                    }
                }
            }
            if let Some(w) = &s.watcher {
                for r in &a.cfg.read().roots {
                    w.unwatch_root(&(a.id(), r.id.clone()));
                    w.watch_root((a.id(), r.id.clone()), &r.local);
                }
            }
            drop(_g);
            s.save_accounts()?;
            s.changed();
            Ok(Value::Null)
        }
        "locate_root" => {
            // A missing root found somewhere else.
            let a = s.account(str_arg(req, "account"))?;
            let rid = str_arg(req, "root");
            let to = PathBuf::from(str_arg(req, "to"));
            if let Some(r) = a.cfg.write().roots.iter_mut().find(|r| r.id == rid) {
                r.local = to.clone();
            }
            if let Some(w) = &s.watcher {
                w.watch_root((a.id(), rid.to_string()), &to);
            }
            s.save_accounts()?;
            let _ = a.tx.send(Trig::Now);
            s.changed();
            Ok(Value::Null)
        }
        "link" => {
            let (_, it) = item_at(s, Path::new(str_arg(req, "path")))?;
            Ok(json!({"url": gdocs::web_url(&it), "id": it.id, "name": it.name}))
        }
        "share_info" => {
            let (a, it) = target(s, req)?;
            let d = a.drive.clone().ok_or_else(|| anyhow!("not signed in"))?;
            let perms = d.permissions(&it.id)?;
            Ok(json!({"item": item_json(&a, &it), "permissions": perms}))
        }
        "share" => {
            // action: anyone (role or "" to turn off), add {email, role,
            // notify, message}, role {perm, role}, remove {perm}.
            let (a, it) = target(s, req)?;
            let d = a.drive.clone().ok_or_else(|| anyhow!("not signed in"))?;
            match str_arg(req, "action") {
                "anyone" => {
                    let perms = d.permissions(&it.id)?;
                    let existing = perms.iter().find(|p| p["type"] == "anyone").and_then(|p| p["id"].as_str().map(String::from));
                    let role = str_arg(req, "role");
                    match (existing, role) {
                        (Some(p), "") => d.remove_permission(&it.id, &p)?,
                        (Some(p), r) => {
                            d.update_permission(&it.id, &p, r)?;
                        }
                        (None, "") => {}
                        (None, r) => {
                            d.share(&it.id, &json!({"type": "anyone", "role": r}), false, "")?;
                        }
                    }
                }
                "add" => {
                    let notify = req.get("notify").and_then(|x| x.as_bool()).unwrap_or(true);
                    d.share(&it.id, &json!({"type": "user", "role": str_arg(req, "role"), "emailAddress": str_arg(req, "email")}), notify, str_arg(req, "message"))?;
                }
                "role" => {
                    d.update_permission(&it.id, str_arg(req, "perm"), str_arg(req, "role"))?;
                }
                "remove" => d.remove_permission(&it.id, str_arg(req, "perm"))?,
                x => bail!("unknown share action {x}"),
            }
            let _ = a.db.log("", "share", &it.name, str_arg(req, "action"));
            Ok(json!({"permissions": d.permissions(&it.id)?}))
        }
        "star" => {
            let (a, it) = target(s, req)?;
            let on = req.get("on").and_then(|x| x.as_bool()).unwrap_or(true);
            a.cloud.set_starred(&it.id, on)?;
            let _ = a.tx.send(Trig::Now);
            Ok(Value::Null)
        }
        "create" => {
            let a = s.account(str_arg(req, "account"))?;
            let parent = match str_arg(req, "parent") {
                "" => a.root_id.read().clone(),
                p => p.to_string(),
            };
            let name = str_arg(req, "name");
            let it = match str_arg(req, "kind") {
                "folder" => a.cloud.create_folder(&parent, name)?,
                k @ ("document" | "spreadsheet" | "presentation" | "drawing" | "form") => a.drive.clone().ok_or_else(|| anyhow!("not signed in"))?.create_google(&parent, name, k)?,
                k => bail!("can't create a {k}"),
            };
            let _ = a.tx.send(Trig::Now);
            Ok(item_json(&a, &it))
        }
        "quit" => {
            s.quit.store(true, Ordering::Relaxed);
            for a in s.accounts.read().iter() {
                a.cancel.store(true, Ordering::Relaxed);
            }
            Ok(Value::Null)
        }
        _ => bail!("unknown command {cmd:?}"),
    }
}
