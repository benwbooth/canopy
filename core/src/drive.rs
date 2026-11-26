//! Google Drive v3 over REST (ureq): the `Cloud` operations plus sharing,
//! search and new Google files for the window.
//!
//! Every call retries with exponential backoff on network errors, 429,
//! 5xx and rate-limit 403s, and refreshes the access token once on 401.
//! Uploads over 5 MB are resumable (the session URL is kept in the
//! database for six days), downloads resume from a partial file.

use crate::cloud::*;
use crate::db::Db;
use crate::throttle::{Metered, DOWN, UP};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
pub const FIELDS: &str = "id,name,mimeType,parents,md5Checksum,size,modifiedTime,createdTime,version,trashed,driveId,webViewLink,shortcutDetails(targetId,targetMimeType),capabilities(canEdit,canDownload),shared,starred,ownedByMe";
const CHUNK: usize = 8 * 1024 * 1024; // a multiple of 256 KiB
const SIMPLE_MAX: u64 = 5 * 1024 * 1024;

/// Hands out access tokens (auth.rs refreshes them).
pub trait Tokens: Send + Sync {
    fn access_token(&self, refresh: bool) -> Result<String>;
}

pub struct Drive {
    agent: ureq::Agent,
    tokens: Arc<dyn Tokens>,
    /// Where resumable upload sessions are remembered.
    db: Option<Arc<Db>>,
}

/// The HTTP agent, with the proxy settings ("system" reads the usual
/// *_proxy variables).
pub fn agent(proxy: &crate::config::Proxy) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(120)).user_agent(concat!("canopy/", env!("CARGO_PKG_VERSION")));
    match proxy.mode.as_str() {
        "manual" if !proxy.url.is_empty() => match ureq::Proxy::new(&proxy.url) {
            Ok(p) => b = b.proxy(p),
            Err(e) => eprintln!("canopy: proxy {} not usable: {e}", proxy.url),
        },
        "none" => {}
        _ => b = b.try_proxy_from_env(true),
    }
    b.build()
}

fn rfc3339_to_secs(s: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp() as f64 + d.timestamp_subsec_nanos() as f64 / 1e9).unwrap_or(0.0)
}

fn secs_to_rfc3339(t: f64) -> String {
    let d = chrono::DateTime::from_timestamp(t.floor() as i64, ((t.fract()) * 1e9) as u32).unwrap_or_default();
    d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn item_from(v: &Value) -> CloudItem {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let sc = v.get("shortcutDetails");
    CloudItem {
        id: s("id").unwrap_or_default(),
        name: s("name").unwrap_or_default(),
        mime: s("mimeType").unwrap_or_default(),
        parents: v.get("parents").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        hash: s("md5Checksum"),
        size: s("size").and_then(|x| x.parse().ok()),
        mtime: s("modifiedTime").map(|t| rfc3339_to_secs(&t)).unwrap_or(0.0),
        created: s("createdTime").map(|t| rfc3339_to_secs(&t)).unwrap_or(0.0),
        version: s("version").and_then(|x| x.parse().ok()).unwrap_or(0),
        trashed: v.get("trashed").and_then(|x| x.as_bool()).unwrap_or(false),
        drive_id: s("driveId"),
        web_link: s("webViewLink"),
        target_id: sc.and_then(|x| x.get("targetId")).and_then(|x| x.as_str()).map(String::from),
        can_edit: v.pointer("/capabilities/canEdit").and_then(|x| x.as_bool()).unwrap_or(false),
        shared: v.get("shared").and_then(|x| x.as_bool()).unwrap_or(false),
        starred: v.get("starred").and_then(|x| x.as_bool()).unwrap_or(false),
        owned_by_me: v.get("ownedByMe").and_then(|x| x.as_bool()).unwrap_or(false),
    }
}

/// Why Google said no, in a sentence (its error JSON's message + reason).
fn google_error(status: u16, body: &str) -> (String, String) {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let msg = v.pointer("/error/message").and_then(|x| x.as_str()).unwrap_or(body).to_string();
    let reason = v.pointer("/error/errors/0/reason").and_then(|x| x.as_str()).unwrap_or("").to_string();
    (format!("Google Drive answered {status}: {msg}"), reason)
}

#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub reason: String,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for HttpError {}

pub fn is_offline(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<ureq::Transport>().is_some() || c.to_string().contains("Network Error"))
}

pub fn is_auth_error(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<HttpError>().map(|h| h.status == 401 || h.reason == "invalid_grant").unwrap_or(false))
}

impl Drive {
    pub fn new(agent: ureq::Agent, tokens: Arc<dyn Tokens>, db: Option<Arc<Db>>) -> Self {
        Drive { agent, tokens, db }
    }

    /// Run a request with auth, retries and backoff. `make` builds it for
    /// each try; `send` sends it.
    fn call<T>(&self, make: impl Fn(&ureq::Agent) -> ureq::Request, send: impl Fn(ureq::Request) -> Result<ureq::Response, ureq::Error>, done: impl Fn(ureq::Response) -> Result<T>) -> Result<T> {
        let mut refreshed = false;
        let mut delay = 1.0f64;
        for attempt in 0..8 {
            let token = self.tokens.access_token(false)?;
            let req = make(&self.agent).set("Authorization", &format!("Bearer {token}"));
            match send(req) {
                Ok(r) => return done(r),
                Err(ureq::Error::Status(status, r)) => {
                    let body = r.into_string().unwrap_or_default();
                    let (message, reason) = google_error(status, &body);
                    let retry = status == 429 || status >= 500 || (status == 403 && (reason.contains("ateLimitExceeded") || reason == "backendError"));
                    if status == 401 && !refreshed {
                        refreshed = true;
                        self.tokens.access_token(true)?;
                        continue;
                    }
                    if !retry || attempt == 7 {
                        return Err(HttpError { status, reason, message }.into());
                    }
                }
                Err(ureq::Error::Transport(t)) => {
                    if attempt >= 3 {
                        return Err(anyhow::Error::new(t).context("Network Error: can't reach Google Drive"));
                    }
                }
            }
            // Jittered exponential backoff.
            let jitter = (crate::db::now().fract() * 1000.0) / 1000.0;
            std::thread::sleep(Duration::from_secs_f64(delay + jitter));
            delay = (delay * 2.0).min(64.0);
        }
        bail!("Google Drive didn't answer after several tries")
    }

    fn get_json(&self, url: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.call(
            |a| {
                let mut r = a.get(url);
                for (k, v) in query {
                    r = r.query(k, v);
                }
                r
            },
            |r| r.call(),
            |r| Ok(r.into_json()?),
        )
    }

    fn send_json(&self, method: &str, url: &str, query: &[(&str, &str)], body: &Value) -> Result<Value> {
        self.call(
            |a| {
                let mut r = a.request(method, url);
                for (k, v) in query {
                    r = r.query(k, v);
                }
                r
            },
            |r| r.send_json(body.clone()),
            |r| {
                let t = r.into_string()?;
                Ok(if t.trim().is_empty() { Value::Null } else { serde_json::from_str(&t)? })
            },
        )
    }

    fn list_pages(&self, url: &str, query: &[(&str, &str)], key: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut token = String::new();
        loop {
            let mut q: Vec<(&str, &str)> = query.to_vec();
            if !token.is_empty() {
                q.push(("pageToken", &token));
            }
            let v = self.get_json(url, &q)?;
            if let Some(a) = v.get(key).and_then(|x| x.as_array()) {
                out.extend(a.iter().cloned());
            }
            match v.get("nextPageToken").and_then(|x| x.as_str()) {
                Some(t) => token = t.to_string(),
                None => break,
            }
        }
        Ok(out)
    }

    // --- extra operations for the window ---

    /// Files whose name or text matches (Drive's own search).
    pub fn search(&self, text: &str) -> Result<Vec<CloudItem>> {
        let esc = text.replace('\\', "\\\\").replace('\'', "\\'");
        let q = format!("trashed = false and (name contains '{esc}' or fullText contains '{esc}')");
        let v = self.get_json(&format!("{API}/files"), &[("q", &q), ("pageSize", "100"), ("fields", &format!("files({FIELDS})")), ("supportsAllDrives", "true"), ("includeItemsFromAllDrives", "true"), ("corpora", "allDrives")])?;
        Ok(v.get("files").and_then(|f| f.as_array()).map(|a| a.iter().map(item_from).collect()).unwrap_or_default())
    }

    pub fn create_google(&self, parent: &str, name: &str, kind: &str) -> Result<CloudItem> {
        let v = self.send_json("POST", &format!("{API}/files"), &[("supportsAllDrives", "true"), ("fields", FIELDS)], &json!({"name": name, "mimeType": format!("application/vnd.google-apps.{kind}"), "parents": [parent]}))?;
        Ok(item_from(&v))
    }

    /// [{id, type, role, emailAddress, displayName, allowFileDiscovery}]
    pub fn permissions(&self, id: &str) -> Result<Vec<Value>> {
        self.list_pages(&format!("{API}/files/{id}/permissions"), &[("supportsAllDrives", "true"), ("fields", "nextPageToken,permissions(id,type,role,emailAddress,displayName,photoLink,allowFileDiscovery,domain)")], "permissions")
    }

    /// Add a permission: {type: user|anyone|domain, role, emailAddress?}.
    pub fn share(&self, id: &str, perm: &Value, notify: bool, message: &str) -> Result<Value> {
        let notify = if notify { "true" } else { "false" };
        let mut q = vec![("supportsAllDrives", "true"), ("sendNotificationEmail", notify)];
        if !message.is_empty() {
            q.push(("emailMessage", message));
        }
        self.send_json("POST", &format!("{API}/files/{id}/permissions"), &q, perm)
    }

    pub fn update_permission(&self, id: &str, perm: &str, role: &str) -> Result<Value> {
        self.send_json("PATCH", &format!("{API}/files/{id}/permissions/{perm}"), &[("supportsAllDrives", "true")], &json!({"role": role}))
    }

    pub fn remove_permission(&self, id: &str, perm: &str) -> Result<()> {
        self.call(|a| a.delete(&format!("{API}/files/{id}/permissions/{perm}")).query("supportsAllDrives", "true"), |r| r.call(), |_| Ok(()))
    }

    // --- uploads ---

    fn metadata(name: &str, parent: Option<&str>, mime: Option<&str>, mtime: f64, google_target: Option<&str>) -> Value {
        let mut m = json!({"name": name, "modifiedTime": secs_to_rfc3339(mtime)});
        if let Some(p) = parent {
            m["parents"] = json!([p]);
        }
        if let Some(t) = google_target {
            // Upload an Office file as the content of a Google file.
            m["mimeType"] = json!(t);
        } else if let Some(mt) = mime {
            m["mimeType"] = json!(mt);
        }
        m
    }

    fn simple_upload(&self, url: &str, method: &str, meta: &Value, src: &Path, media_mime: &str, progress: Progress) -> Result<CloudItem> {
        let data = std::fs::read(src)?;
        let boundary = format!("canopy{}", crate::db::now().to_bits());
        let mut body = Vec::with_capacity(data.len() + 512);
        write!(body, "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: {media_mime}\r\n\r\n")?;
        body.extend_from_slice(&data);
        write!(body, "\r\n--{boundary}--\r\n")?;
        let total = data.len() as u64;
        let v: Value = self.call(
            |a| a.request(method, url).query("uploadType", "multipart").query("supportsAllDrives", "true").query("fields", FIELDS).set("Content-Type", &format!("multipart/related; boundary={boundary}")),
            |r| {
                let rd = Metered { inner: std::io::Cursor::new(&body), bucket: &UP, done: 0, progress: &|_| {} };
                r.set("Content-Length", &body.len().to_string()).send(rd)
            },
            |r| Ok(r.into_json()?),
        )?;
        progress(total);
        Ok(item_from(&v))
    }

    fn resumable_upload(&self, url: &str, method: &str, meta: &Value, src: &Path, media_mime: &str, mtime: f64, progress: Progress) -> Result<CloudItem> {
        let size = std::fs::metadata(src)?.len();
        let key = src.to_string_lossy().into_owned();
        let saved = self.db.as_ref().and_then(|d| d.upload_session("", &key, size, mtime).ok().flatten());
        let mut offset = 0u64;
        let mut session = None;
        if let Some(s) = saved {
            // Ask how far the earlier try got.
            match self.agent.put(&s).set("Content-Range", &format!("bytes */{size}")).send_bytes(&[]) {
                Ok(r) if r.status() == 200 || r.status() == 201 => return Ok(item_from(&r.into_json()?)),
                Ok(r) if r.status() == 308 => {
                    offset = r.header("Range").and_then(|h| h.rsplit('-').next()).and_then(|n| n.parse::<u64>().ok()).map(|n| n + 1).unwrap_or(0);
                    session = Some(s);
                }
                Err(ureq::Error::Status(308, r)) => {
                    offset = r.header("Range").and_then(|h| h.rsplit('-').next()).and_then(|n| n.parse::<u64>().ok()).map(|n| n + 1).unwrap_or(0);
                    session = Some(s);
                }
                _ => {}
            }
        }
        let session = match session {
            Some(s) => s,
            None => {
                let s: String = self.call(
                    |a| a.request(method, url).query("uploadType", "resumable").query("supportsAllDrives", "true").query("fields", FIELDS).set("X-Upload-Content-Type", media_mime).set("X-Upload-Content-Length", &size.to_string()),
                    |r| r.send_json(meta.clone()),
                    |r| r.header("Location").map(String::from).ok_or_else(|| anyhow!("no upload session")),
                )?;
                if let Some(d) = &self.db {
                    d.set_upload_session("", &key, Some(&s), size, mtime)?;
                }
                s
            }
        };
        let mut f = std::fs::File::open(src)?;
        let mut buf = vec![0u8; CHUNK];
        let mut tries = 0;
        loop {
            f.seek(SeekFrom::Start(offset))?;
            let n = read_full(&mut f, &mut buf)?;
            let end = offset + n as u64;
            let range = if size == 0 { format!("bytes */{size}") } else { format!("bytes {}-{}/{}", offset, end.saturating_sub(1), size) };
            let base = offset;
            let rd = Metered { inner: std::io::Cursor::new(&buf[..n]), bucket: &UP, done: 0, progress: &|d| progress(base + d) };
            match self.agent.put(&session).set("Content-Range", &range).set("Content-Length", &n.to_string()).send(rd) {
                Ok(r) if r.status() == 200 || r.status() == 201 => {
                    if let Some(d) = &self.db {
                        d.set_upload_session("", &key, None, size, mtime)?;
                    }
                    return Ok(item_from(&r.into_json()?));
                }
                Ok(r) | Err(ureq::Error::Status(308, r)) if r.status() == 308 => {
                    offset = r.header("Range").and_then(|h| h.rsplit('-').next()).and_then(|x| x.parse::<u64>().ok()).map(|x| x + 1).unwrap_or(end);
                    tries = 0;
                }
                Err(ureq::Error::Status(404, _)) | Err(ureq::Error::Status(410, _)) => {
                    if let Some(d) = &self.db {
                        d.set_upload_session("", &key, None, size, mtime)?;
                    }
                    bail!("the upload session expired; it will start again");
                }
                Err(e) => {
                    tries += 1;
                    if tries > 5 {
                        return Err(anyhow::Error::new(e).context("the upload stopped; it will resume later"));
                    }
                    std::thread::sleep(Duration::from_secs(1 << tries));
                    // Ask where to continue from.
                    if let Err(ureq::Error::Status(308, r)) | Ok(r) = self.agent.put(&session).set("Content-Range", &format!("bytes */{size}")).send_bytes(&[]) {
                        if r.status() == 308 {
                            offset = r.header("Range").and_then(|h| h.rsplit('-').next()).and_then(|x| x.parse::<u64>().ok()).map(|x| x + 1).unwrap_or(0);
                        }
                    }
                }
                Ok(r) => bail!("unexpected answer {} to an upload", r.status()),
            }
        }
    }

    /// Download `url` (authorized) into `dest`, resuming a partial file.
    fn fetch(&self, url: &str, query: &[(&str, &str)], dest: &Path, progress: Progress) -> Result<()> {
        let have = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        let r = self.call(
            |a| {
                let mut r = a.get(url);
                for (k, v) in query {
                    r = r.query(k, v);
                }
                if have > 0 {
                    r = r.set("Range", &format!("bytes={have}-"));
                }
                r
            },
            |r| r.call(),
            Ok,
        );
        let r = match r {
            Err(e) if have > 0 && e.chain().any(|c| c.downcast_ref::<HttpError>().map(|h| h.status == 416).unwrap_or(false)) => {
                std::fs::remove_file(dest)?;
                return self.fetch(url, query, dest, progress);
            }
            r => r?,
        };
        let append = r.status() == 206;
        let mut out = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(dest)?;
        let start = if append { have } else { 0 };
        let mut rd = Metered { inner: r.into_reader(), bucket: &DOWN, done: 0, progress: &|d| progress(start + d) };
        std::io::copy(&mut rd, &mut out).context("the download stopped; it will resume later")?;
        out.sync_all()?;
        Ok(())
    }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

/// The Google type an Office/OpenDocument file converts into.
fn google_target_for(mime: &str) -> Option<&'static str> {
    if mime.contains("wordprocessingml") || mime.ends_with("opendocument.text") {
        Some("application/vnd.google-apps.document")
    } else if mime.contains("spreadsheetml") || mime.ends_with("opendocument.spreadsheet") {
        Some("application/vnd.google-apps.spreadsheet")
    } else if mime.contains("presentationml") || mime.ends_with("opendocument.presentation") {
        Some("application/vnd.google-apps.presentation")
    } else {
        None
    }
}

fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("txt" | "md") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("zip") => "application/zip",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("odp") => "application/vnd.oasis.opendocument.presentation",
        _ => "application/octet-stream",
    }
}

impl Cloud for Drive {
    fn about(&self) -> Result<About> {
        let v = self.get_json(&format!("{API}/about"), &[("fields", "user(displayName,emailAddress,photoLink,permissionId),storageQuota(limit,usage)")])?;
        let root = self.get_json(&format!("{API}/files/root"), &[("fields", "id")])?;
        Ok(About {
            email: v.pointer("/user/emailAddress").and_then(|x| x.as_str()).unwrap_or_default().into(),
            name: v.pointer("/user/displayName").and_then(|x| x.as_str()).unwrap_or_default().into(),
            photo: v.pointer("/user/photoLink").and_then(|x| x.as_str()).map(String::from),
            root_id: root.get("id").and_then(|x| x.as_str()).unwrap_or_default().into(),
            quota_used: v.pointer("/storageQuota/usage").and_then(|x| x.as_str()).and_then(|x| x.parse().ok()),
            quota_total: v.pointer("/storageQuota/limit").and_then(|x| x.as_str()).and_then(|x| x.parse().ok()),
        })
    }

    fn shared_drives(&self) -> Result<Vec<SharedDrive>> {
        let v = self.list_pages(&format!("{API}/drives"), &[("pageSize", "100"), ("fields", "nextPageToken,drives(id,name)")], "drives")?;
        Ok(v.iter().map(|d| SharedDrive { id: d["id"].as_str().unwrap_or_default().into(), name: d["name"].as_str().unwrap_or_default().into() }).collect())
    }

    fn start_token(&self, drive: Option<&str>) -> Result<String> {
        let mut q = vec![("supportsAllDrives", "true")];
        if let Some(d) = drive {
            q.push(("driveId", d));
        }
        let v = self.get_json(&format!("{API}/changes/startPageToken"), &q)?;
        v.get("startPageToken").and_then(|x| x.as_str()).map(String::from).ok_or_else(|| anyhow!("no start token"))
    }

    fn changes(&self, drive: Option<&str>, token: &str) -> Result<ChangePage> {
        let fields = format!("nextPageToken,newStartPageToken,changes(removed,fileId,file({FIELDS}))");
        let mut q = vec![("pageToken", token), ("pageSize", "1000"), ("includeRemoved", "true"), ("supportsAllDrives", "true"), ("fields", fields.as_str())];
        match drive {
            Some(d) => {
                q.push(("driveId", d));
                q.push(("includeItemsFromAllDrives", "true"));
            }
            None => q.push(("restrictToMyDrive", "false")),
        }
        let v = self.get_json(&format!("{API}/changes"), &q)?;
        let changes = v
            .get("changes")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        let id = c.get("fileId")?.as_str()?.to_string();
                        if c.get("removed").and_then(|x| x.as_bool()).unwrap_or(false) {
                            return Some(Change::Removed(id));
                        }
                        c.get("file").map(|f| Change::Item(item_from(f))).or(Some(Change::Removed(id)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(ChangePage { changes, next: v.get("nextPageToken").and_then(|x| x.as_str()).map(String::from), new_start: v.get("newStartPageToken").and_then(|x| x.as_str()).map(String::from) })
    }

    fn list_all(&self, drive: Option<&str>) -> Result<Vec<CloudItem>> {
        let fields = format!("nextPageToken,files({FIELDS})");
        let mut q = vec![("q", "trashed = false"), ("pageSize", "1000"), ("fields", fields.as_str()), ("supportsAllDrives", "true")];
        match drive {
            Some(d) => {
                q.push(("corpora", "drive"));
                q.push(("driveId", d));
                q.push(("includeItemsFromAllDrives", "true"));
            }
            None => q.push(("corpora", "user")),
        }
        let v = self.list_pages(&format!("{API}/files"), &q, "files")?;
        Ok(v.iter().map(item_from).collect())
    }

    fn get(&self, id: &str) -> Result<Option<CloudItem>> {
        match self.get_json(&format!("{API}/files/{id}"), &[("fields", FIELDS), ("supportsAllDrives", "true")]) {
            Ok(v) => Ok(Some(item_from(&v))),
            Err(e) if e.chain().any(|c| c.downcast_ref::<HttpError>().map(|h| h.status == 404).unwrap_or(false)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn create_folder(&self, parent: &str, name: &str) -> Result<CloudItem> {
        let v = self.send_json("POST", &format!("{API}/files"), &[("supportsAllDrives", "true"), ("fields", FIELDS)], &json!({"name": name, "mimeType": FOLDER_MIME, "parents": [parent]}))?;
        Ok(item_from(&v))
    }

    fn upload(&self, parent: &str, name: &str, existing: Option<&str>, src: &Path, mime: Option<&str>, mtime: f64, progress: Progress) -> Result<CloudItem> {
        let size = std::fs::metadata(src)?.len();
        // `mime` set with `existing`: an export uploaded back into its
        // Google file (Drive converts it).
        let media_mime = mime.unwrap_or_else(|| guess_mime(src));
        let target = mime.and_then(google_target_for);
        let (url, method, meta) = match existing {
            Some(id) => (format!("{UPLOAD}/files/{id}"), "PATCH", Self::metadata(name, None, None, mtime, target)),
            None => (format!("{UPLOAD}/files"), "POST", Self::metadata(name, Some(parent), None, mtime, None)),
        };
        if size <= SIMPLE_MAX {
            self.simple_upload(&url, method, &meta, src, media_mime, progress)
        } else {
            self.resumable_upload(&url, method, &meta, src, media_mime, mtime, progress)
        }
    }

    fn download(&self, id: &str, dest: &Path, progress: Progress) -> Result<()> {
        self.fetch(&format!("{API}/files/{id}"), &[("alt", "media"), ("supportsAllDrives", "true")], dest, progress)
    }

    fn export(&self, id: &str, mime: &str, dest: &Path) -> Result<()> {
        let _ = std::fs::remove_file(dest);
        match self.fetch(&format!("{API}/files/{id}/export"), &[("mimeType", mime)], dest, &|_| {}) {
            Err(e) if e.chain().any(|c| c.downcast_ref::<HttpError>().map(|h| h.reason == "exportSizeLimitExceeded").unwrap_or(false)) => {
                // Over the 10 MB export limit: the file's export link.
                let v = self.get_json(&format!("{API}/files/{id}"), &[("fields", "exportLinks"), ("supportsAllDrives", "true")])?;
                let link = v.pointer("/exportLinks").and_then(|l| l.get(mime)).and_then(|x| x.as_str()).ok_or_else(|| anyhow!("this file is too big to convert"))?;
                let _ = std::fs::remove_file(dest);
                self.fetch(link, &[], dest, &|_| {})
            }
            r => r,
        }
    }

    fn update_meta(&self, id: &str, name: Option<&str>, move_from_to: Option<(&str, &str)>) -> Result<CloudItem> {
        let mut q = vec![("supportsAllDrives", "true"), ("fields", FIELDS)];
        if let Some((from, to)) = move_from_to {
            q.push(("removeParents", from));
            q.push(("addParents", to));
        }
        let body = match name {
            Some(n) => json!({"name": n}),
            None => json!({}),
        };
        Ok(item_from(&self.send_json("PATCH", &format!("{API}/files/{id}"), &q, &body)?))
    }

    fn trash(&self, id: &str) -> Result<()> {
        self.send_json("PATCH", &format!("{API}/files/{id}"), &[("supportsAllDrives", "true"), ("fields", "id")], &json!({"trashed": true}))?;
        Ok(())
    }

    fn copy(&self, id: &str, parent: &str, name: &str) -> Result<CloudItem> {
        Ok(item_from(&self.send_json("POST", &format!("{API}/files/{id}/copy"), &[("supportsAllDrives", "true"), ("fields", FIELDS)], &json!({"name": name, "parents": [parent]}))?))
    }

    fn keep_revision(&self, id: &str) -> Result<()> {
        let v = self.get_json(&format!("{API}/files/{id}"), &[("fields", "headRevisionId"), ("supportsAllDrives", "true")])?;
        // Google files keep their whole history anyway (no head revision).
        let Some(rev) = v.get("headRevisionId").and_then(|x| x.as_str()) else { return Ok(()) };
        self.send_json("PATCH", &format!("{API}/files/{id}/revisions/{rev}"), &[("fields", "id")], &json!({"keepForever": true}))?;
        Ok(())
    }

    fn set_starred(&self, id: &str, starred: bool) -> Result<()> {
        self.send_json("PATCH", &format!("{API}/files/{id}"), &[("supportsAllDrives", "true"), ("fields", "id")], &json!({"starred": starred}))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_files() {
        let v = json!({"id":"1","name":"a","mimeType":"text/plain","parents":["p"],"md5Checksum":"abc","size":"12","modifiedTime":"2024-01-02T03:04:05.500Z","version":"7","capabilities":{"canEdit":true}});
        let it = item_from(&v);
        assert_eq!((it.size, it.version, it.can_edit), (Some(12), 7, true));
        assert_eq!(it.mtime, 1704164645.5);
        assert_eq!(secs_to_rfc3339(1704164645.5), "2024-01-02T03:04:05.500Z");
    }
}
