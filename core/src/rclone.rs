//! Any of rclone's services (Dropbox, OneDrive, Box, S3, SFTP, WebDAV,
//! pCloud...) as a `Cloud`: canopy syncs a real local folder with it,
//! rclone only carries the files. No mount (no FUSE).
//!
//! - One `rclone rcd` per process, on a unix socket in canopy's runtime
//!   folder (no port, no password; it dies with canopyd), started on first
//!   use. Remotes come from rclone's own config (the ones grove sets up).
//! - rclone addresses files by path; the engine by id. A path index (table
//!   `rpaths` in the account's database) gives every path an id of
//!   canopy's own that stays with the item through canopy's renames and
//!   moves, so they are renames in the engine too. Where the service has
//!   ids of its own (OneDrive, Dropbox, Box...), a move made elsewhere is
//!   recognised by that id; elsewhere it is a delete and an add (safe: the
//!   old copy goes to the trash, the new one is downloaded).
//! - There is no changes feed: a refresh lists the whole remote and
//!   compares it with the index, at most once a minute (Sync Now looks
//!   again at once). canopy's own changes go into the mirror as it makes
//!   them.
//! - Hashes: the first kind the service keeps that rclone can also make of
//!   a local file (MD5, SHA-1, Dropbox's, OneDrive's QuickXor...). Without
//!   one (WebDAV, FTP), files compare by size and time.
//! - Deleting: services with a trash of their own (Drive, Dropbox,
//!   OneDrive, Box, pCloud, Mega, Jottacloud, unless set to delete for
//!   good) delete into it; on all others canopy moves the item into a
//!   ".canopy-trash/<date>" folder at the top of the remote instead, so
//!   nothing it deletes is gone.
//! - Services without empty folders (S3, B2, the memory backend): a folder
//!   canopy makes is kept in the index ("virtual") until something is put
//!   in it.

use crate::cloud::*;
use crate::db::Db;
use anyhow::{anyhow, bail, Context, Result};
use parking_lot::Mutex;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// The top-level folder deletions go to on services without a trash.
pub const TRASH_DIR: &str = ".canopy-trash";
pub const ROOT: &str = "root";

/// rclone couldn't reach the service (no network): the account shows
/// "offline" and retries, as with Google.
#[derive(Debug)]
pub struct Offline(pub String);
impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Offline {}

pub fn is_offline_message(m: &str) -> bool {
    ["dial tcp", "no such host", "connection refused", "i/o timeout", "network is unreachable", "TLS handshake timeout", "connection reset"].iter().any(|k| m.contains(k))
}

// --- the rclone process ---

pub struct Rcd {
    sock: PathBuf,
    child: Mutex<Option<Child>>,
}

static RCD: OnceLock<Arc<Rcd>> = OnceLock::new();

/// The rclone binary (CANOPY_RCLONE, else PATH).
pub fn binary() -> String {
    std::env::var("CANOPY_RCLONE").unwrap_or_else(|_| "rclone".into())
}

pub fn available() -> bool {
    Command::new(binary()).arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// The process-wide rclone (started on first call).
pub fn rcd() -> Arc<Rcd> {
    RCD.get_or_init(|| Arc::new(Rcd { sock: crate::paths::runtime_dir().join("rclone.sock"), child: Mutex::new(None) })).clone()
}

impl Rcd {
    fn ensure(&self) -> Result<()> {
        let mut ch = self.child.lock();
        if let Some(c) = ch.as_mut() {
            if c.try_wait()?.is_none() {
                return Ok(());
            }
        }
        if let Some(d) = self.sock.parent() {
            std::fs::create_dir_all(d)?;
        }
        let _ = std::fs::remove_file(&self.sock);
        *ch = Some(spawn(&self.sock)?);
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(15) {
            if UnixStream::connect(&self.sock).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        bail!("rclone didn't start")
    }

    /// One remote-control call: `method` (operations/list...) with a JSON
    /// body; the answer, or rclone's error.
    pub fn call(&self, method: &str, body: &Value) -> Result<Value> {
        self.ensure()?;
        let b = body.to_string();
        let mut s = UnixStream::connect(&self.sock).context("rclone isn't answering")?;
        // HTTP/1.0: the answer comes whole (never chunked) and the
        // connection closes after it.
        write!(s, "POST /{method} HTTP/1.0\r\nHost: rclone\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len())?;
        let mut out = Vec::new();
        s.read_to_end(&mut out)?;
        let text = String::from_utf8_lossy(&out);
        let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| anyhow!("rclone gave a broken answer"))?;
        let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if status != 200 {
            let msg = v["error"].as_str().unwrap_or(body).to_string();
            if is_offline_message(&msg) {
                return Err(Offline(msg).into());
            }
            bail!("{msg}");
        }
        Ok(v)
    }

    /// Run a transfer as a job, reporting bytes as they move.
    fn transfer(&self, method: &str, mut body: Value, progress: Progress) -> Result<()> {
        let group = format!("canopy-{}", new_id());
        body["_async"] = json!(true);
        body["_group"] = json!(group);
        let job = self.call(method, &body)?["jobid"].as_i64().ok_or_else(|| anyhow!("rclone didn't start the transfer"))?;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if let Ok(st) = self.call("core/stats", &json!({"group": group})) {
                if let Some(b) = st["bytes"].as_u64() {
                    progress(b);
                }
            }
            let js = self.call("job/status", &json!({"jobid": job}))?;
            if js["finished"].as_bool() == Some(true) {
                let _ = self.call("core/stats-delete", &json!({"group": group}));
                if js["success"].as_bool() == Some(true) {
                    return Ok(());
                }
                let msg = js["error"].as_str().unwrap_or("the transfer failed").to_string();
                if is_offline_message(&msg) {
                    return Err(Offline(msg).into());
                }
                bail!("{msg}");
            }
        }
    }
}

/// Start `rclone rcd`. It must die with canopyd, and the kernel's
/// parent-death signal fires when the *thread* that started it ends: so
/// it's started from one thread that lives as long as the process.
fn spawn(sock: &Path) -> Result<Child> {
    type Req = (PathBuf, std::sync::mpsc::Sender<std::io::Result<Child>>);
    static KEEPER: OnceLock<Mutex<std::sync::mpsc::Sender<Req>>> = OnceLock::new();
    let tx = KEEPER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Req>();
        std::thread::Builder::new()
            .name("rclone keeper".into())
            .spawn(move || {
                for (sock, answer) in rx {
                    let mut cmd = Command::new(binary());
                    cmd.args(["rcd", "--rc-no-auth", "--rc-addr"]).arg(format!("unix://{}", sock.display())).args(["--log-level", "ERROR"]);
                    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit());
                    unsafe {
                        cmd.pre_exec(|| {
                            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                            Ok(())
                        });
                    }
                    let _ = answer.send(cmd.spawn());
                }
            })
            .expect("thread");
        Mutex::new(tx)
    });
    let (atx, arx) = std::sync::mpsc::channel();
    tx.lock().send((sock.to_path_buf(), atx)).map_err(|_| anyhow!("rclone keeper is gone"))?;
    arx.recv()?.with_context(|| format!("couldn't start rclone ({}); is it installed?", binary()))
}

fn new_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_time(s: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp() as f64 + d.timestamp_subsec_nanos() as f64 / 1e9).unwrap_or(0.0)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_of(p: &str) -> &str {
    p.rfind('/').map(|i| &p[..i]).unwrap_or("")
}

fn name_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn under(p: &str, dir: &str) -> bool {
    dir.is_empty() || (p.len() > dir.len() && p.starts_with(dir) && p.as_bytes()[dir.len()] == b'/')
}

/// A number that changes when a file does (files without a hash compare
/// by it).
fn version_of(size: i64, mtime: f64, hash: Option<&str>) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!("{size}|{:.3}|{}", mtime, hash.unwrap_or("")).bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h >> 1) as i64
}

/// What canopy needs to know about a remote (rclone's fsinfo).
#[derive(Clone, Debug, Default)]
pub struct Info {
    /// The hash kind used (None: compare by size and time).
    pub hash: Option<String>,
    pub empty_dirs: bool,
    pub case_insensitive: bool,
    pub own_trash: bool,
    pub about: bool,
    /// rclone's type of the remote (dropbox, s3, local...).
    pub kind: String,
}

/// Hash kinds rclone can make of a local file, best first.
const LOCAL_HASHES: [&str; 8] = ["md5", "sha1", "sha256", "dropbox", "quickxor", "crc32", "blake3", "whirlpool"];

/// Remote types whose delete goes to a trash of the service's own, and the
/// option that would make it delete for good instead.
const OWN_TRASH: [(&str, &str); 7] = [("drive", "use_trash"), ("dropbox", ""), ("onedrive", "hard_delete"), ("box", ""), ("pcloud", ""), ("mega", "hard_delete"), ("jottacloud", "hard_delete")];

/// A remote set up in rclone's config: [(name, type)], secrets left out.
pub fn remotes() -> Result<Vec<(String, String)>> {
    let r = rcd();
    let names = r.call("config/listremotes", &json!({}))?;
    let mut out = vec![];
    for n in names["remotes"].as_array().cloned().unwrap_or_default() {
        let n = n.as_str().unwrap_or("").to_string();
        let t = r.call("config/get", &json!({"name": n})).ok().and_then(|c| c["type"].as_str().map(String::from)).unwrap_or_default();
        out.push((n, t));
    }
    Ok(out)
}

/// rclone's name for a kind of remote ("Dropbox", "Microsoft OneDrive").
pub fn provider_label(kind: &str) -> String {
    let fallback = || {
        let mut c = kind.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let Ok(v) = rcd().call("config/providers", &json!({})) else { return fallback() };
    v["providers"].as_array().and_then(|ps| ps.iter().find(|p| p["Prefix"].as_str() == Some(kind) || p["Name"].as_str() == Some(kind))).and_then(|p| p["Description"].as_str()).map(|d| d.split(" (").next().unwrap_or(d).to_string()).unwrap_or_else(fallback)
}

pub fn info(fs: &str) -> Result<Info> {
    let r = rcd();
    let fi = r.call("operations/fsinfo", &json!({"fs": fs}))?;
    let have: Vec<String> = fi["Hashes"].as_array().map(|a| a.iter().filter_map(|h| h.as_str().map(String::from)).collect()).unwrap_or_default();
    let hash = LOCAL_HASHES.iter().find(|h| have.iter().any(|x| x == *h)).map(|h| h.to_string());
    let f = &fi["Features"];
    // The remote's type and options (for its trash): "name:" or a path.
    let name = fs.split_once(':').map(|(n, _)| n).filter(|n| !n.contains('/') && !n.is_empty()).unwrap_or("");
    let cfg = if name.is_empty() { json!({"type": "local"}) } else { r.call("config/get", &json!({"name": name})).unwrap_or(json!({})) };
    let kind = cfg["type"].as_str().unwrap_or("local").to_string();
    let own_trash = OWN_TRASH.iter().any(|(t, opt)| {
        *t == kind
            && match (*t, *opt) {
                (_, "") => true,
                ("drive", o) => cfg[o].as_str() != Some("false"),
                (_, o) => cfg[o].as_str() != Some("true"),
            }
    });
    Ok(Info {
        hash,
        empty_dirs: f["CanHaveEmptyDirectories"].as_bool().unwrap_or(true),
        case_insensitive: f["CaseInsensitive"].as_bool().unwrap_or(false),
        own_trash,
        about: f["About"].as_bool().unwrap_or(false),
        kind,
    })
}

// --- the cloud ---

pub struct RcloneCloud {
    rcd: Arc<Rcd>,
    /// The remote and folder ("dropbox:", "s3:bucket/canopy", "/srv/x").
    fs: String,
    label: String,
    db: Arc<Db>,
    pub info: Info,
    /// Seconds between whole listings (0: every refresh).
    pub relist_secs: u64,
    last_list: Mutex<Option<Instant>>,
    /// Serializes index updates.
    lock: Mutex<()>,
}

/// One row of the path index.
#[derive(Clone, Debug)]
struct Row {
    id: String,
    path: String,
    native: Option<String>,
    dir: bool,
    sig: String,
    virt: bool,
}

impl RcloneCloud {
    /// `fs`: an rclone remote and folder; `db`: the account's database
    /// (holds the path index).
    pub fn new(fs: &str, label: &str, db: Arc<Db>) -> Result<Self> {
        db.with(|c| {
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS rpaths (id TEXT PRIMARY KEY, path TEXT UNIQUE NOT NULL, native TEXT, dir INTEGER NOT NULL, sig TEXT NOT NULL DEFAULT '', virt INTEGER NOT NULL DEFAULT 0);
                 CREATE INDEX IF NOT EXISTS rpaths_native ON rpaths(native);",
            )?;
            c.execute("INSERT OR IGNORE INTO rpaths (id, path, dir) VALUES (?1, '', 1)", [ROOT])?;
            Ok(())
        })?;
        let rcd = rcd();
        let info = info(fs)?;
        Ok(RcloneCloud { rcd, fs: fs.to_string(), label: label.to_string(), db, info, relist_secs: 60, last_list: Mutex::new(None), lock: Mutex::new(()) })
    }

    /// The rclone fs of a folder inside the remote.
    fn sub_fs(&self, path: &str) -> String {
        if path.is_empty() {
            self.fs.clone()
        } else if self.fs.ends_with(':') || self.fs.ends_with('/') {
            format!("{}{path}", self.fs)
        } else {
            format!("{}/{path}", self.fs)
        }
    }

    fn hash_types(&self) -> Value {
        match &self.info.hash {
            Some(h) => json!([h]),
            None => json!([]),
        }
    }

    // --- the index ---

    fn row_by_id(&self, id: &str) -> Result<Option<Row>> {
        self.db.with(|c| Ok(c.query_row("SELECT id, path, native, dir, sig, virt FROM rpaths WHERE id = ?1", [id], row).optional()?))
    }

    fn row_by_path(&self, path: &str) -> Result<Option<Row>> {
        self.db.with(|c| Ok(c.query_row("SELECT id, path, native, dir, sig, virt FROM rpaths WHERE path = ?1", [path], row).optional()?))
    }

    fn all_rows(&self) -> Result<Vec<Row>> {
        self.db.with(|c| {
            let mut st = c.prepare("SELECT id, path, native, dir, sig, virt FROM rpaths")?;
            let rows = st.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    fn put_row(&self, r: &Row) -> Result<()> {
        self.db.with(|c| {
            // A path now held by this id frees it from whatever had it.
            c.execute("DELETE FROM rpaths WHERE path = ?1 AND id != ?2", params![r.path, r.id])?;
            c.execute(
                "INSERT INTO rpaths (id, path, native, dir, sig, virt) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET path = excluded.path, native = excluded.native, dir = excluded.dir, sig = excluded.sig, virt = excluded.virt",
                params![r.id, r.path, r.native, r.dir as i64, r.sig, r.virt as i64],
            )?;
            Ok(())
        })
    }

    /// Forget a path and everything under it.
    fn drop_tree(&self, path: &str) -> Result<()> {
        let like = format!("{}/%", path.replace('%', "\\%").replace('_', "\\_"));
        self.db.with(|c| {
            c.execute("DELETE FROM rpaths WHERE id != ?3 AND (path = ?1 OR path LIKE ?2 ESCAPE '\\')", params![path, like, ROOT])?;
            Ok(())
        })
    }

    /// Paths under `from` now under `to` (canopy moved them): same ids.
    fn move_tree(&self, from: &str, to: &str) -> Result<()> {
        let rows: Vec<Row> = self.all_rows()?.into_iter().filter(|r| r.path == from || under(&r.path, from) && !from.is_empty()).collect();
        self.drop_tree(to)?;
        for mut r in rows {
            r.path = format!("{to}{}", &r.path[from.len()..]);
            self.put_row(&r)?;
        }
        Ok(())
    }

    fn path_of(&self, id: &str) -> Result<String> {
        if id == ROOT {
            return Ok(String::new());
        }
        self.row_by_id(id)?.map(|r| r.path).ok_or_else(|| anyhow!("not known (it may have been deleted in {} meanwhile)", self.label))
    }

    fn id_for(&self, path: &str) -> Result<String> {
        if path.is_empty() {
            return Ok(ROOT.into());
        }
        Ok(self.row_by_path(path)?.map(|r| r.id).unwrap_or_else(|| format!("r{}", new_id())))
    }

    fn item(&self, r: &Row, e: Option<&Value>) -> CloudItem {
        let parent = if r.path.is_empty() { vec![] } else { vec![self.row_by_path(parent_of(&r.path)).ok().flatten().map(|p| p.id).unwrap_or_else(|| ROOT.into())] };
        let (size, mtime, hash) = match e {
            Some(e) => (e["Size"].as_i64().unwrap_or(0), parse_time(e["ModTime"].as_str().unwrap_or("")), self.info.hash.as_ref().and_then(|h| e["Hashes"][h].as_str()).filter(|h| !h.is_empty()).map(String::from)),
            None => (0, 0.0, None),
        };
        CloudItem {
            id: r.id.clone(),
            name: if r.path.is_empty() { self.label.clone() } else { name_of(&r.path).to_string() },
            mime: if r.dir { FOLDER_MIME.into() } else { String::new() },
            parents: parent,
            hash: if r.dir { None } else { hash.clone() },
            size: if r.dir { None } else { Some(size.max(0) as u64) },
            mtime,
            version: if r.dir { 0 } else { version_of(size, mtime, hash.as_deref()) },
            can_edit: true,
            owned_by_me: true,
            ..Default::default()
        }
    }

    fn sig(e: &Value, hash: Option<&str>) -> String {
        if e["IsDir"].as_bool() == Some(true) {
            return "d".into();
        }
        format!("{}|{}|{}", e["Size"].as_i64().unwrap_or(0), e["ModTime"].as_str().unwrap_or(""), hash.and_then(|h| e["Hashes"][h].as_str()).unwrap_or(""))
    }

    fn stat(&self, path: &str) -> Result<Option<Value>> {
        let v = self.rcd.call("operations/stat", &json!({"fs": self.fs, "remote": path, "opt": {"showHash": true, "hashTypes": self.hash_types(), "noMimeType": true}}))?;
        Ok(if v["item"].is_null() { None } else { Some(v["item"].clone()) })
    }

    /// The whole remote, as index rows (ids kept, or matched by the
    /// service's own id, or new) with their listing entries.
    fn list(&self) -> Result<Vec<(Row, Option<Value>)>> {
        let v = match self.rcd.call("operations/list", &json!({"fs": self.fs, "remote": "", "opt": {"recurse": true, "showHash": true, "hashTypes": self.hash_types(), "noMimeType": true, "showOrigIDs": false}})) {
            Ok(v) => v,
            // Nothing there yet (an empty bucket, a folder not made yet).
            Err(e) if e.to_string().contains("directory not found") => json!({"list": []}),
            Err(e) => return Err(e),
        };
        let mut entries: Vec<Value> = v["list"].as_array().cloned().unwrap_or_default();
        entries.retain(|e| {
            let p = e["Path"].as_str().unwrap_or("");
            p != TRASH_DIR && !p.starts_with(&format!("{TRASH_DIR}/"))
        });
        let old = self.all_rows()?;
        let by_path: HashMap<&str, &Row> = old.iter().map(|r| (r.path.as_str(), r)).collect();
        let by_native: HashMap<&str, &Row> = old.iter().filter_map(|r| r.native.as_deref().map(|n| (n, r))).collect();
        let mut out: Vec<(Row, Option<Value>)> = vec![];
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut used_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let hash = self.info.hash.clone();
        for e in &entries {
            let path = e["Path"].as_str().unwrap_or("").to_string();
            let native = e["ID"].as_str().filter(|s| !s.is_empty()).map(String::from);
            let dir = e["IsDir"].as_bool() == Some(true);
            // Same id: same path, else (moved elsewhere) the same native id.
            let id = match (by_path.get(path.as_str()), native.as_deref().and_then(|n| by_native.get(n))) {
                (Some(r), _) if r.dir == dir && !used_ids.contains(&r.id) => r.id.clone(),
                (_, Some(r)) if r.dir == dir && !used_ids.contains(&r.id) => r.id.clone(),
                _ => format!("r{}", new_id()),
            };
            used_ids.insert(id.clone());
            seen.insert(path.clone(), out.len());
            out.push((Row { id, path, native, dir, sig: Self::sig(e, hash.as_deref()), virt: false }, Some(e.clone())));
        }
        // Folders canopy made on a service without empty folders; and there,
        // any folder that emptied (its last file deleted) stays a folder
        // rather than looking deleted.
        for r in old.iter().filter(|r| !r.path.is_empty() && (r.virt || r.dir && !self.info.empty_dirs)) {
            if !seen.contains_key(&r.path) && !used_ids.contains(&r.id) {
                seen.insert(r.path.clone(), out.len());
                used_ids.insert(r.id.clone());
                out.push((Row { virt: true, ..r.clone() }, None));
            }
        }
        let mut k = 0;
        while k < out.len() {
            let p = parent_of(&out[k].0.path).to_string();
            if !p.is_empty() && !seen.contains_key(&p) {
                let id = by_path.get(p.as_str()).filter(|r| r.dir).map(|r| r.id.clone()).unwrap_or_else(|| format!("r{}", new_id()));
                seen.insert(p.clone(), out.len());
                out.push((Row { id, path: p, native: None, dir: true, sig: "d".into(), virt: false }, None));
            }
            k += 1;
        }
        Ok(out)
    }

    /// Replace the index with a listing; the items, and the ids no longer
    /// there.
    fn reindex(&self, listed: Vec<(Row, Option<Value>)>) -> Result<(Vec<CloudItem>, Vec<String>, Vec<Row>)> {
        let old = self.all_rows()?;
        let keep: std::collections::HashSet<&str> = listed.iter().map(|(r, _)| r.id.as_str()).collect();
        let gone: Vec<String> = old.iter().filter(|r| r.id != ROOT && !keep.contains(r.id.as_str())).map(|r| r.id.clone()).collect();
        self.db.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute("DELETE FROM rpaths WHERE id != ?1", [ROOT])?;
            for (r, _) in &listed {
                tx.execute("INSERT OR REPLACE INTO rpaths (id, path, native, dir, sig, virt) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![r.id, r.path, r.native, r.dir as i64, r.sig, r.virt as i64])?;
            }
            tx.commit()?;
            Ok(())
        })?;
        let items = listed.iter().map(|(r, e)| self.item(r, e.as_ref())).collect();
        Ok((items, gone, old))
    }

    fn due(&self) -> bool {
        match *self.last_list.lock() {
            Some(t) => t.elapsed() >= Duration::from_secs(self.relist_secs),
            None => true,
        }
    }

    /// Where an item is moved instead of deleted on services without a
    /// trash: ".canopy-trash/2026-11-26 09.30.12/path".
    fn trash_path(path: &str) -> String {
        format!("{TRASH_DIR}/{}/{path}", chrono::Local::now().format("%Y-%m-%d %H.%M.%S"))
    }

    fn move_path(&self, from: &str, to: &str, dir: bool) -> Result<()> {
        if dir {
            self.rcd.call("sync/move", &json!({"srcFs": self.sub_fs(from), "dstFs": self.sub_fs(to), "createEmptySrcDirs": true, "deleteEmptySrcDirs": true}))?;
            // (Its emptied source may linger where the service keeps empty
            // folders.)
            if self.stat(from)?.is_some() {
                let _ = self.rcd.call("operations/rmdirs", &json!({"fs": self.fs, "remote": from, "leaveRoot": false}));
            }
        } else {
            self.rcd.call("operations/movefile", &json!({"srcFs": self.fs, "srcRemote": from, "dstFs": self.fs, "dstRemote": to}))?;
        }
        Ok(())
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row { id: r.get(0)?, path: r.get(1)?, native: r.get(2)?, dir: r.get::<_, i64>(3)? != 0, sig: r.get(4)?, virt: r.get::<_, i64>(5)? != 0 })
}

impl Cloud for RcloneCloud {
    fn about(&self) -> Result<About> {
        let (mut used, mut total) = (None, None);
        if self.info.about {
            if let Ok(a) = self.rcd.call("operations/about", &json!({"fs": self.fs})) {
                used = a["used"].as_u64();
                total = a["total"].as_u64();
            }
        }
        Ok(About { email: self.label.clone(), name: self.label.clone(), photo: None, root_id: ROOT.into(), quota_used: used, quota_total: total })
    }

    fn shared_drives(&self) -> Result<Vec<SharedDrive>> {
        Ok(vec![])
    }

    fn start_token(&self, _drive: Option<&str>) -> Result<String> {
        Ok("0".into())
    }

    fn changes(&self, _drive: Option<&str>, token: &str) -> Result<ChangePage> {
        let _g = self.lock.lock();
        if !self.due() {
            return Ok(ChangePage { changes: vec![], next: None, new_start: Some(token.to_string()) });
        }
        let listed = self.list()?;
        let (_, gone, old) = self.reindex(listed.clone())?;
        *self.last_list.lock() = Some(Instant::now());
        let before: HashMap<&str, &Row> = old.iter().map(|r| (r.id.as_str(), r)).collect();
        let mut changes = vec![];
        for (r, e) in &listed {
            let same = before.get(r.id.as_str()).map(|o| o.path == r.path && o.sig == r.sig && o.dir == r.dir).unwrap_or(false);
            if !same {
                changes.push(Change::Item(self.item(r, e.as_ref())));
            }
        }
        changes.extend(gone.into_iter().map(Change::Removed));
        let n: u64 = token.parse().unwrap_or(0) + 1;
        Ok(ChangePage { changes, next: None, new_start: Some(n.to_string()) })
    }

    fn list_all(&self, _drive: Option<&str>) -> Result<Vec<CloudItem>> {
        let _g = self.lock.lock();
        let listed = self.list()?;
        let (items, _, _) = self.reindex(listed)?;
        *self.last_list.lock() = Some(Instant::now());
        let root = self.item(&Row { id: ROOT.into(), path: String::new(), native: None, dir: true, sig: "d".into(), virt: false }, None);
        Ok(std::iter::once(root).chain(items).collect())
    }

    fn get(&self, id: &str) -> Result<Option<CloudItem>> {
        let Some(r) = (if id == ROOT { Some(Row { id: ROOT.into(), path: String::new(), native: None, dir: true, sig: "d".into(), virt: false }) } else { self.row_by_id(id)? }) else {
            return Ok(None);
        };
        if r.path.is_empty() {
            return Ok(Some(self.item(&r, None)));
        }
        match self.stat(&r.path)? {
            Some(e) => {
                let dir = e["IsDir"].as_bool() == Some(true);
                if dir != r.dir {
                    return Ok(None);
                }
                Ok(Some(self.item(&r, Some(&e))))
            }
            // A folder made here on a service without empty folders.
            None if r.virt => Ok(Some(self.item(&r, None))),
            None => Ok(None),
        }
    }

    fn create_folder(&self, parent: &str, name: &str) -> Result<CloudItem> {
        let _g = self.lock.lock();
        let path = join(&self.path_of(parent)?, name);
        self.rcd.call("operations/mkdir", &json!({"fs": self.fs, "remote": path}))?;
        let e = self.stat(&path)?;
        if let Some(e) = &e {
            if e["IsDir"].as_bool() != Some(true) {
                bail!("a file named {name} is there already");
            }
        }
        let r = Row { id: self.id_for(&path)?, path: path.clone(), native: e.as_ref().and_then(|e| e["ID"].as_str().map(String::from)), dir: true, sig: "d".into(), virt: e.is_none() || !self.info.empty_dirs };
        self.put_row(&r)?;
        Ok(self.item(&r, e.as_ref()))
    }

    fn upload(&self, parent: &str, name: &str, existing: Option<&str>, src: &Path, _mime: Option<&str>, _mtime: f64, progress: Progress) -> Result<CloudItem> {
        let path = match existing {
            Some(id) => self.path_of(id)?,
            None => {
                let p = join(&self.path_of(parent)?, name);
                // Never over something canopy hasn't seen.
                if self.stat(&p)?.is_some() {
                    self.wake();
                    bail!("something named {name} appeared in {} meanwhile; it will be compared again", self.label);
                }
                p
            }
        };
        let dir = src.parent().ok_or_else(|| anyhow!("no folder"))?;
        let file = src.file_name().ok_or_else(|| anyhow!("no name"))?.to_string_lossy().into_owned();
        let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.len(), m.modified().ok()));
        let before = stamp(src);
        // (IgnoreTimes: rclone would otherwise skip a file there with the
        // same size and time, which a different edit can have.)
        self.rcd.transfer("operations/copyfile", json!({"srcFs": dir.to_string_lossy(), "srcRemote": file, "dstFs": self.fs, "dstRemote": path, "_config": {"IgnoreTimes": true}}), progress)?;
        let e = self.stat(&path)?.ok_or_else(|| anyhow!("the upload didn't arrive"))?;
        // What arrived is what's here (unless it changed while uploading:
        // the engine uploads that again on the next pass).
        if let Some(h) = self.info.hash.as_ref().and_then(|h| e["Hashes"][h].as_str()).filter(|h| !h.is_empty()) {
            if stamp(src) == before && self.hash_local(src)? != h {
                bail!("the upload didn't arrive whole; it will be tried again");
            }
        }
        let _g = self.lock.lock();
        let r = Row { id: match existing { Some(id) => id.to_string(), None => self.id_for(&path)? }, path: path.clone(), native: e["ID"].as_str().map(String::from), dir: false, sig: Self::sig(&e, self.info.hash.as_deref()), virt: false };
        self.put_row(&r)?;
        // Its folder is real now.
        if let Some(mut p) = self.row_by_path(parent_of(&path))? {
            if p.virt && self.info.empty_dirs {
                p.virt = false;
                self.put_row(&p)?;
            }
        }
        Ok(self.item(&r, Some(&e)))
    }

    fn download(&self, id: &str, dest: &Path, progress: Progress) -> Result<()> {
        let path = self.path_of(id)?;
        let dir = dest.parent().ok_or_else(|| anyhow!("no folder"))?;
        std::fs::create_dir_all(dir)?;
        let file = dest.file_name().ok_or_else(|| anyhow!("no name"))?.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(dest);
        self.rcd.transfer("operations/copyfile", json!({"srcFs": self.fs, "srcRemote": path, "dstFs": dir.to_string_lossy(), "dstRemote": file, "_config": {"IgnoreTimes": true}}), progress)
    }

    fn export(&self, _id: &str, _mime: &str, _dest: &Path) -> Result<()> {
        bail!("{} has no Google Docs to convert", self.label)
    }

    fn update_meta(&self, id: &str, name: Option<&str>, move_from_to: Option<(&str, &str)>) -> Result<CloudItem> {
        let _g = self.lock.lock();
        let r = self.row_by_id(id)?.ok_or_else(|| anyhow!("not known in {}", self.label))?;
        let dir = match move_from_to {
            Some((_, to)) => self.path_of(to)?,
            None => parent_of(&r.path).to_string(),
        };
        let to = join(&dir, name.unwrap_or(name_of(&r.path)));
        if to == r.path {
            return Ok(self.item(&r, self.stat(&r.path)?.as_ref()));
        }
        // Never onto something there (a name that differs only in case is
        // the same item on a case-insensitive service).
        let case_only = self.info.case_insensitive && to.to_lowercase() == r.path.to_lowercase();
        if !case_only && self.stat(&to)?.is_some() {
            self.wake();
            bail!("something named {} is there already in {}", name_of(&to), self.label);
        }
        if !(r.virt && self.stat(&r.path)?.is_none()) {
            self.move_path(&r.path, &to, r.dir)?;
        }
        self.move_tree(&r.path, &to)?;
        let nr = self.row_by_id(id)?.ok_or_else(|| anyhow!("lost track of it"))?;
        let e = self.stat(&to)?;
        Ok(self.item(&nr, e.as_ref()))
    }

    fn trash(&self, id: &str) -> Result<()> {
        let _g = self.lock.lock();
        let r = self.row_by_id(id)?.ok_or_else(|| anyhow!("not known in {}", self.label))?;
        if r.path.is_empty() {
            bail!("the top folder is never deleted");
        }
        let there = self.stat(&r.path)?.is_some();
        if there {
            if self.info.own_trash {
                if r.dir {
                    self.rcd.call("operations/purge", &json!({"fs": self.fs, "remote": r.path}))?;
                } else {
                    self.rcd.call("operations/deletefile", &json!({"fs": self.fs, "remote": r.path}))?;
                }
            } else {
                self.move_path(&r.path, &Self::trash_path(&r.path), r.dir)?;
            }
        }
        self.drop_tree(&r.path)?;
        Ok(())
    }

    fn copy(&self, _id: &str, _parent: &str, _name: &str) -> Result<CloudItem> {
        bail!("{} has no Google Docs to copy", self.label)
    }

    fn set_starred(&self, _id: &str, _starred: bool) -> Result<()> {
        bail!("{} has no stars", self.label)
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    fn is_google(&self) -> bool {
        false
    }

    fn hash_local(&self, path: &Path) -> Result<String> {
        match self.info.hash.as_deref() {
            Some("md5") | None => crate::engine::md5_file(path),
            Some(kind) => {
                let v = self.rcd.call("operations/hashsum", &json!({"fs": path.to_string_lossy(), "hashType": kind}))?;
                let line = v["hashsum"].as_array().and_then(|a| a.first()).and_then(|l| l.as_str()).ok_or_else(|| anyhow!("rclone couldn't hash {}", path.display()))?;
                Ok(line.split_whitespace().next().unwrap_or("").to_string())
            }
        }
    }

    fn case_insensitive(&self) -> bool {
        self.info.case_insensitive
    }

    fn trash_label(&self) -> String {
        if self.info.own_trash {
            format!("{}'s trash", self.label)
        } else {
            format!("the {TRASH_DIR} folder in {}", self.label)
        }
    }

    fn wake(&self) {
        *self.last_list.lock() = None;
    }
}
