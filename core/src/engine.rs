//! The sync engine for one root: compare the cloud (from the mirror), the
//! disk and the last agreed state ("base") for every path, and make the
//! two sides match.
//!
//! A pass:
//! 1. builds the cloud tree under the root (selected, not ignored, local
//!    names for Google files and duplicates),
//! 2. scans the local tree,
//! 3. turns moves on either side into renames on the other (by Drive id or
//!    inode), so nothing is transferred again,
//! 4. merges each path three ways into actions,
//! 5. runs them: folders, renames and link files in order; transfers in
//!    parallel; deletions last, deepest first, always into a trash.
//!
//! The base row of a path is written as soon as its action succeeds, so an
//! interrupted pass picks up where it stopped.

use crate::cloud::{Cloud, CloudItem};
use crate::config::{ConflictPolicy, ExportOption, Mode, Resolution, Root};
use crate::db::{Base, Db};
use crate::gdocs::{self, Local as GLocal};
use crate::ignore::{Ignore, PART_DIR};
use crate::local::{self, LocalItem};
use anyhow::{anyhow, bail, Result};
use md5::{Digest, Md5};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Per-account settings a pass needs.
pub struct Ctx {
    pub email: String,
    pub export: ExportOption,
    pub conflicts: ConflictPolicy,
    pub ignore: Ignore,
    /// Tests: a trash folder of their own (never the user's).
    pub trash_dir: Option<PathBuf>,
    pub transfers: usize,
    /// Local deletions delete in the cloud (false: they only unsync).
    pub sync_deletions: bool,
    pub host: String,
}

/// Things a pass tells the daemon as it goes.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// A transfer's progress (bytes).
    Transfer { root: String, path: String, up: bool, done: u64, total: u64 },
    TransferDone { root: String, path: String, up: bool, ok: bool },
    /// Something happened (also in the activity table).
    Activity { root: String, kind: String, path: String, detail: String },
    /// A path's sync state changed: P pending, S synced, E error, C conflict.
    Status { root: String, path: String, state: char },
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Default, Clone)]
pub struct PassOptions {
    /// Conflicts the user resolved: path -> choice.
    pub forced: HashMap<String, Resolution>,
    /// The user confirmed a large deletion.
    pub confirm_deletes: bool,
    /// The user confirmed the root really is empty now.
    pub allow_empty: bool,
}

#[derive(Default, Debug, Serialize, Clone)]
pub struct Report {
    pub uploads: usize,
    pub downloads: usize,
    pub moves: usize,
    pub deletes: usize,
    pub conflicts: usize,
    pub errors: usize,
    /// The pass stopped before changing anything (missing root, mass
    /// deletion...): the reason, also an attention issue.
    pub held: Option<String>,
    /// Paths that became unsynced (deleted locally while deletions only
    /// unsync): the daemon adds them to the root's selection as false.
    pub unsynced: Vec<String>,
    /// Paths with an action still planned when the pass stopped.
    pub pending: usize,
}

#[derive(Clone, Debug)]
struct CEntry {
    item: CloudItem,
    /// 'd', 'f' or 'g'.
    kind: char,
}

#[derive(Clone, Debug)]
enum Op {
    LocalMkdir(String, CloudItem),
    CloudMkdir(String),
    /// Rename a local file out of the way (conflict copy) to this name.
    SetAside(String, String),
    Download(String, CloudItem, Option<LocalItem>),
    WriteLink(String, CloudItem),
    Export(String, CloudItem, &'static str, Option<LocalItem>),
    /// Upload new (None) or over an existing cloud file.
    Upload(String, Option<CloudItem>),
    /// A copied link file: copy the Google file it points at.
    CopyCloud(String, String),
    Record(Base),
    Forget(String),
    TrashLocal(String),
    TrashCloud(String, String),
    Unsync(String),
}

pub struct Engine {
    pub cloud: Arc<dyn Cloud>,
    pub db: Arc<Db>,
    pub ctx: Ctx,
    pub root: Root,
    /// The cloud folder's id (My Drive's real id).
    pub root_id: String,
    pub sink: Sink,
    pub cancel: Arc<AtomicBool>,
    /// Paths with an action in flight (badges).
    pub busy: Arc<Mutex<HashSet<String>>>,
}

fn parent_of(p: &str) -> &str {
    p.rfind('/').map(|i| &p[..i]).unwrap_or("")
}

fn name_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn under(p: &str, dir: &str) -> bool {
    p.len() > dir.len() && p.starts_with(dir) && p.as_bytes()[dir.len()] == b'/'
}

/// Drive names can hold '/', Linux names can't: shown as '∕' (U+2215).
pub fn local_safe(name: &str) -> String {
    let n = name.replace('/', "\u{2215}").replace('\0', "");
    match n.as_str() {
        "" => "_".into(),
        "." => "._".into(),
        ".." => ".._".into(),
        _ => n,
    }
}

pub fn cloud_name(local: &str) -> String {
    local.replace('\u{2215}', "/")
}

fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

pub fn md5_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn rewrite_prefix<V: Clone>(m: &mut BTreeMap<String, V>, from: &str, to: &str) {
    let keys: Vec<String> = m.keys().filter(|k| *k == from || under(k, from)).cloned().collect();
    let moved: Vec<(String, V)> = keys.iter().map(|k| (format!("{to}{}", &k[from.len()..]), m.remove(k).unwrap())).collect();
    for (k, v) in moved {
        m.insert(k, v);
    }
}

impl Engine {
    fn local_path(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.root.local.clone()
        } else {
            self.root.local.join(rel)
        }
    }

    fn part_dir(&self) -> PathBuf {
        self.root.local.join(PART_DIR)
    }

    fn up(&self) -> bool {
        self.root.mode != Mode::CloudToLocal
    }

    fn down(&self) -> bool {
        self.root.mode != Mode::LocalToCloud
    }

    fn emit(&self, e: Event) {
        (self.sink)(e)
    }

    fn activity(&self, kind: &str, path: &str, detail: &str) {
        let _ = self.db.log(&self.root.id, kind, path, detail);
        self.emit(Event::Activity { root: self.root.id.clone(), kind: kind.into(), path: path.into(), detail: detail.into() });
    }

    fn error(&self, path: &str, e: &anyhow::Error) {
        let msg = format!("{e:#}");
        eprintln!("canopy: {}: {path}: {msg}", self.root.name);
        let _ = self.db.issue(&self.root.id, path, "error", "retry", &msg);
        self.emit(Event::Status { root: self.root.id.clone(), path: path.into(), state: 'E' });
    }

    fn ok(&self, path: &str) {
        let _ = self.db.clear_issue(&self.root.id, path, "error");
        let _ = self.db.clear_issue(&self.root.id, path, "conflict");
        self.emit(Event::Status { root: self.root.id.clone(), path: path.into(), state: 'S' });
    }

    /// The cloud tree under the root, keyed by local relative path.
    fn cloud_tree(&self, base: &BTreeMap<String, Base>) -> Result<BTreeMap<String, CEntry>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![(self.root_id.clone(), String::new())];
        while let Some((id, rel)) = stack.pop() {
            let mut kids = self.db.children(&id)?;
            // The oldest of same-named items keeps the name.
            kids.sort_by(|a, b| a.created.partial_cmp(&b.created).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.id.cmp(&b.id)));
            let mut taken: HashMap<String, u32> = HashMap::new();
            for it in kids {
                let kind = if it.is_folder() {
                    'd'
                } else if it.is_google_format() {
                    'g'
                } else {
                    'f'
                };
                let mut name = if kind == 'g' { local_safe(&gdocs::local_name(&it, self.ctx.export)) } else { local_safe(&it.name) };
                let n = taken.entry(name.clone()).or_insert(0);
                *n += 1;
                if *n > 1 {
                    let (stem, ext) = split_ext(&name);
                    name = format!("{stem} ({n}){ext}");
                }
                let path = join(&rel, &name);
                let known = base.contains_key(&path);
                if self.ctx.ignore.ignored(&path, kind == 'd') && !known {
                    continue;
                }
                if kind == 'd' {
                    if known || self.root.selected_below(&path) {
                        stack.push((it.id.clone(), path.clone()));
                        if known || self.root.selected(&path) || self.root.selected_below(&path) {
                            out.insert(path, CEntry { item: it, kind });
                        }
                    }
                } else if known || self.root.selected(&path) {
                    out.insert(path, CEntry { item: it, kind });
                }
            }
        }
        Ok(out)
    }

    fn local_changed(b: &Base, l: &LocalItem) -> bool {
        !l.is_dir && (l.size != b.l_size || (l.mtime - b.l_mtime).abs() > 0.0005)
    }

    fn cloud_changed(b: &Base, c: &CEntry) -> bool {
        if b.cl_id != c.item.id {
            return true;
        }
        match c.kind {
            'd' => false,
            'f' if c.item.md5.is_some() => c.item.md5 != b.c_md5,
            _ => c.item.version != b.c_version,
        }
    }

    fn base_for(path: &str, kind: char, c: &CloudItem, l: &LocalItem) -> Base {
        Base { path: path.into(), kind, cl_id: c.id.clone(), ino: l.ino, l_mtime: l.mtime, l_size: l.size, c_md5: c.md5.clone(), c_version: c.version }
    }

    fn conflict_name(&self, path: &str) -> String {
        let (stem, ext) = split_ext(name_of(path));
        let stamp = chrono::Local::now().format("%Y-%m-%d %H.%M");
        join(parent_of(path), &format!("{stem} (conflict {} {stamp}){ext}", self.ctx.host))
    }

    /// One pass. Errors stop it only when nothing can be done (offline,
    /// root missing); single items' errors become issues.
    pub fn pass(&self, opts: &PassOptions) -> Result<Report> {
        let mut rep = Report::default();
        let rid = self.root.id.clone();
        let mut base: BTreeMap<String, Base> = self.db.base_all(&rid)?.into_iter().map(|b| (b.path.clone(), b)).collect();

        // Safety: a root that isn't there (unplugged disk, renamed folder)
        // must never look like "everything was deleted".
        let root_path = &self.root.local;
        if !root_path.is_dir() {
            if base.is_empty() {
                std::fs::create_dir_all(root_path)?;
            } else {
                let msg = format!("The folder {} is missing. Plug in its disk or choose where it went (Locate…).", root_path.display());
                self.db.issue(&rid, "", "attention", "missing_root", &msg)?;
                rep.held = Some(msg);
                return Ok(rep);
            }
        }
        self.db.clear_issue(&rid, "", "attention")?;
        let _ = std::fs::create_dir_all(self.part_dir());

        let mut cloud = self.cloud_tree(&base)?;
        let scan = local::scan(root_path, &self.ctx.ignore);
        let mut localm = scan.items;
        for (p, e) in &scan.unreadable {
            self.db.issue(&rid, p, "error", "read", &format!("Can't read this: {e}"))?;
        }
        if !scan.symlinks.is_empty() {
            self.db.issue(&rid, &scan.symlinks[0], "attention", "symlink", &format!("{} symbolic link(s) aren't synced (Google Drive can't hold links). Sync the folders they point to instead.", scan.symlinks.len()))?;
        }

        if localm.is_empty() && base.values().filter(|b| b.kind != 'd').count() > 0 && !opts.allow_empty && self.up() {
            let msg = format!("{} is empty, but it had synced files. If you meant to delete them all, confirm; otherwise check the disk.", root_path.display());
            self.db.issue(&rid, "", "attention", "empty_root", &msg)?;
            rep.held = Some(msg);
            return Ok(rep);
        }

        // --- moves ---
        let mut guard = 0;
        let mut skip: HashSet<String> = HashSet::new();
        loop {
            guard += 1;
            if guard > 100_000 || self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let ids: HashMap<&str, &str> = cloud.iter().map(|(p, c)| (c.item.id.as_str(), p.as_str())).collect();
            let inos: HashMap<u64, &str> = localm.iter().filter(|(p, _)| !base.contains_key(*p)).map(|(p, l)| (l.ino, p.as_str())).collect();
            let mut found: Option<(bool, String, String)> = None;
            for b in base.values() {
                if b.path.is_empty() || skip.contains(&b.path) {
                    continue;
                }
                // Cloud move: the same id at a new path.
                if self.down() && !cloud.contains_key(&b.path) {
                    if let Some(np) = ids.get(b.cl_id.as_str()) {
                        if *np != b.path && !base.contains_key(*np) && localm.contains_key(&b.path) && !localm.contains_key(*np) {
                            found = Some((true, b.path.clone(), np.to_string()));
                            break;
                        }
                    }
                }
                // Local move: the same inode at a new path.
                if self.up() && !localm.contains_key(&b.path) && b.ino != 0 {
                    if let Some(np) = inos.get(&b.ino) {
                        let l = &localm[*np];
                        let same_kind = l.is_dir == (b.kind == 'd');
                        let cloud_here = cloud.get(&b.path).map(|c| c.item.id == b.cl_id).unwrap_or(false);
                        if same_kind && cloud_here && !cloud.contains_key(*np) {
                            found = Some((false, b.path.clone(), np.to_string()));
                            break;
                        }
                    }
                }
            }
            let Some((from_cloud, from, to)) = found else { break };
            let done = if from_cloud { self.apply_cloud_move(&from, &to) } else { self.apply_local_move(&from, &to, &base, &cloud) };
            match done {
                Ok(()) => {
                    rep.moves += 1;
                    self.db.base_move_tree(&rid, &from, &to)?;
                    rewrite_prefix(&mut base, &from, &to);
                    for (k, b) in base.iter_mut() {
                        b.path = k.clone();
                    }
                    if from_cloud {
                        rewrite_prefix(&mut localm, &from, &to);
                    } else {
                        rewrite_prefix(&mut cloud, &from, &to);
                    }
                    self.activity(if parent_of(&from) == parent_of(&to) { "rename" } else { "move" }, &to, &format!("from {from}{}", if from_cloud { " (in the cloud)" } else { "" }));
                }
                Err(e) => {
                    // Fall back to the plain merge (delete + add) for this one.
                    self.error(&to, &e);
                    skip.insert(from);
                }
            }
        }

        // --- merge ---
        let mut ops: Vec<Op> = Vec::new();
        let mut paths: Vec<&String> = base.keys().chain(localm.keys()).chain(cloud.keys()).collect();
        paths.sort();
        paths.dedup();
        // Folders deleted as a whole: skip what's inside.
        let mut gone_dirs: Vec<String> = Vec::new();
        for p in paths {
            if p.is_empty() || gone_dirs.iter().any(|d| under(p, d)) {
                continue;
            }
            let b = base.get(p);
            let l = localm.get(p);
            let c = cloud.get(p);
            let forced = opts.forced.get(p).copied();
            self.merge_one(p, b, l, c, forced, &base, &localm, &cloud, &mut ops, &mut gone_dirs, &mut rep);
        }

        // Mass deletion guard (as rclone's --max-delete).
        let cloud_trash = ops.iter().filter(|o| matches!(o, Op::TrashCloud(..))).count();
        let files = base.len().max(1);
        if cloud_trash > 50 && cloud_trash * 2 > files && !opts.confirm_deletes {
            let msg = format!("{cloud_trash} items would be deleted from Google Drive. Confirm if that's what you meant.");
            self.db.issue(&rid, "", "attention", "mass_delete", &msg)?;
            rep.held = Some(msg);
            ops.retain(|o| !matches!(o, Op::TrashCloud(..)));
        }

        self.run(ops, &mut rep);
        Ok(rep)
    }

    #[allow(clippy::too_many_arguments)]
    fn merge_one(
        &self,
        p: &str,
        b: Option<&Base>,
        l: Option<&LocalItem>,
        c: Option<&CEntry>,
        forced: Option<Resolution>,
        base: &BTreeMap<String, Base>,
        localm: &BTreeMap<String, LocalItem>,
        cloud: &BTreeMap<String, CEntry>,
        ops: &mut Vec<Op>,
        gone_dirs: &mut Vec<String>,
        rep: &mut Report,
    ) {
        let (up, down) = (self.up(), self.down());
        let pol = &self.ctx.conflicts;
        match (b, l, c) {
            (None, Some(l), None) => {
                if !up {
                    return;
                }
                if l.is_dir {
                    ops.push(Op::CloudMkdir(p.into()));
                } else if gdocs::is_link_name(p) {
                    match gdocs::read_link(&self.local_path(p)) {
                        Some(link) => ops.push(Op::CopyCloud(p.into(), link.file_id)),
                        None => ops.push(Op::Upload(p.into(), None)),
                    }
                } else {
                    ops.push(Op::Upload(p.into(), None));
                }
            }
            (None, None, Some(c)) => {
                if down {
                    self.push_download(p, c, None, ops);
                }
            }
            (None, Some(l), Some(c)) => {
                // Both sides have it and canopy never synced it: match.
                match (l.is_dir, c.kind) {
                    (true, 'd') => ops.push(Op::Record(Self::base_for(p, 'd', &c.item, l))),
                    (false, 'f') => {
                        let same = c.item.md5.as_ref().map(|m| md5_file(&self.local_path(p)).ok().as_ref() == Some(m)).unwrap_or(false);
                        if same {
                            ops.push(Op::Record(Self::base_for(p, 'f', &c.item, l)));
                        } else {
                            self.conflict(p, "edit_edit", forced.unwrap_or(pol.edit_edit), Some(l), Some(c), ops, rep);
                        }
                    }
                    (false, 'g') => {
                        let matched = match gdocs::local_form(&c.item, self.ctx.export) {
                            GLocal::Link(_) => gdocs::read_link(&self.local_path(p)).map(|k| k.file_id == c.item.id).unwrap_or(false),
                            GLocal::Export(..) => l.mtime + 1.0 >= c.item.mtime,
                        };
                        if matched {
                            ops.push(Op::Record(Self::base_for(p, 'g', &c.item, l)));
                        } else {
                            self.conflict(p, "edit_edit", forced.unwrap_or(pol.edit_edit), Some(l), Some(c), ops, rep);
                        }
                    }
                    _ => self.conflict(p, "edit_edit", forced.unwrap_or(pol.edit_edit), Some(l), Some(c), ops, rep),
                }
            }
            (Some(_), None, None) => ops.push(Op::Forget(p.into())),
            (Some(b), None, Some(c)) => {
                // Deleted here.
                if !up || (self.root.mode == Mode::CloudToLocal) {
                    self.push_download(p, c, None, ops);
                    return;
                }
                if Self::cloud_changed(b, c) && c.kind != 'd' {
                    self.conflict(p, "local_delete_cloud_edit", forced.unwrap_or(pol.local_delete_cloud_edit), None, Some(c), ops, rep);
                    return;
                }
                if !self.ctx.sync_deletions {
                    ops.push(Op::Unsync(p.into()));
                    gone_dirs.push(p.into());
                    return;
                }
                if c.kind == 'd' {
                    // Only if nothing inside changed in the cloud or was added.
                    let clean = cloud.iter().filter(|(k, _)| under(k, p)).all(|(k, ce)| base.get(k).map(|bb| !Self::cloud_changed(bb, ce)).unwrap_or(false))
                        && !localm.keys().any(|k| under(k, p));
                    if !clean {
                        // Keep the folder: re-create it; its contents merge one by one.
                        self.push_download(p, c, None, ops);
                        return;
                    }
                    gone_dirs.push(p.into());
                }
                ops.push(Op::TrashCloud(p.into(), c.item.id.clone()));
            }
            (Some(b), Some(l), None) => {
                // Gone from the cloud (deleted, trashed, unshared).
                if !down {
                    ops.push(if l.is_dir { Op::CloudMkdir(p.into()) } else { Op::Upload(p.into(), None) });
                    return;
                }
                if Self::local_changed(b, l) && b.kind != 'g' {
                    self.conflict(p, "local_edit_cloud_delete", forced.unwrap_or(pol.local_edit_cloud_delete), Some(l), None, ops, rep);
                    return;
                }
                if l.is_dir {
                    let clean = localm.iter().filter(|(k, _)| under(k, p)).all(|(k, li)| base.get(k).map(|bb| !Self::local_changed(bb, li)).unwrap_or(false));
                    if !clean {
                        ops.push(Op::CloudMkdir(p.into()));
                        return;
                    }
                    gone_dirs.push(p.into());
                }
                ops.push(Op::TrashLocal(p.into()));
            }
            (Some(b), Some(l), Some(c)) => {
                if l.is_dir != (c.kind == 'd') {
                    self.conflict(p, "edit_edit", forced.unwrap_or(pol.edit_edit), Some(l), Some(c), ops, rep);
                    return;
                }
                if l.is_dir {
                    if b.cl_id != c.item.id || b.ino != l.ino {
                        ops.push(Op::Record(Self::base_for(p, 'd', &c.item, l)));
                    }
                    return;
                }
                let lc = Self::local_changed(b, l);
                let cc = Self::cloud_changed(b, c);
                let link = c.kind == 'g' && matches!(gdocs::local_form(&c.item, self.ctx.export), GLocal::Link(_));
                match (lc, cc) {
                    (false, false) => {
                        if b.ino != l.ino {
                            ops.push(Op::Record(Self::base_for(p, b.kind, &c.item, l)));
                        }
                    }
                    (true, false) => {
                        if link {
                            // Link files aren't edited: write it again.
                            ops.push(Op::WriteLink(p.into(), c.item.clone()));
                        } else if c.kind == 'g' && !gdocs::can_upload_back(&c.item) {
                            ops.push(Op::Record(Self::base_for(p, 'g', &c.item, l)));
                        } else if up && self.root.mode != Mode::CloudToLocal {
                            ops.push(Op::Upload(p.into(), Some(c.item.clone())));
                        }
                    }
                    (false, true) => {
                        if link {
                            ops.push(Op::WriteLink(p.into(), c.item.clone()));
                        } else if down {
                            self.push_download(p, c, Some(l.clone()), ops);
                        } else {
                            ops.push(Op::Upload(p.into(), Some(c.item.clone())));
                        }
                    }
                    (true, true) => {
                        if link {
                            ops.push(Op::WriteLink(p.into(), c.item.clone()));
                            return;
                        }
                        let same = c.kind == 'f' && c.item.md5.as_ref().map(|m| md5_file(&self.local_path(p)).ok().as_ref() == Some(m)).unwrap_or(false);
                        if same {
                            ops.push(Op::Record(Self::base_for(p, 'f', &c.item, l)));
                        } else if self.root.mode == Mode::CloudToLocal {
                            self.push_download(p, c, Some(l.clone()), ops);
                        } else if self.root.mode == Mode::LocalToCloud {
                            ops.push(Op::Upload(p.into(), Some(c.item.clone())));
                        } else {
                            self.conflict(p, "edit_edit", forced.unwrap_or(pol.edit_edit), Some(l), Some(c), ops, rep);
                        }
                    }
                }
            }
            (None, None, None) => {}
        }
    }

    fn push_download(&self, p: &str, c: &CEntry, l: Option<LocalItem>, ops: &mut Vec<Op>) {
        match c.kind {
            'd' => ops.push(Op::LocalMkdir(p.into(), c.item.clone())),
            'f' => ops.push(Op::Download(p.into(), c.item.clone(), l)),
            _ => match gdocs::local_form(&c.item, self.ctx.export) {
                GLocal::Link(_) => ops.push(Op::WriteLink(p.into(), c.item.clone())),
                GLocal::Export(_, mime) => ops.push(Op::Export(p.into(), c.item.clone(), mime, l)),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict(&self, p: &str, sub: &str, how: Resolution, l: Option<&LocalItem>, c: Option<&CEntry>, ops: &mut Vec<Op>, rep: &mut Report) {
        rep.conflicts += 1;
        match (sub, how) {
            (_, Resolution::Ask) => {
                let msg = match sub {
                    "edit_edit" => "Changed both here and in Google Drive.",
                    "local_edit_cloud_delete" => "Changed here, but deleted in Google Drive.",
                    _ => "Deleted here, but changed in Google Drive.",
                };
                let _ = self.db.issue(&self.root.id, p, "conflict", sub, msg);
                self.emit(Event::Status { root: self.root.id.clone(), path: p.into(), state: 'C' });
            }
            ("edit_edit", Resolution::KeepBoth) => {
                let aside = self.conflict_name(p);
                ops.push(Op::SetAside(p.into(), aside.clone()));
                if let Some(c) = c {
                    self.push_download(p, c, None, ops);
                }
                if l.map(|l| l.is_dir).unwrap_or(false) {
                    ops.push(Op::CloudMkdir(aside));
                } else {
                    ops.push(Op::Upload(aside, None));
                }
                self.activity("conflict", p, "kept both versions");
            }
            ("edit_edit", Resolution::KeepLocal) => {
                if let Some(c) = c {
                    if c.kind == 'd' || l.map(|l| l.is_dir).unwrap_or(false) {
                        ops.push(Op::TrashCloud(p.into(), c.item.id.clone()));
                        ops.push(Op::Upload(p.into(), None));
                    } else {
                        ops.push(Op::Upload(p.into(), Some(c.item.clone())));
                    }
                }
            }
            ("edit_edit", Resolution::KeepCloud) => {
                if let Some(c) = c {
                    // The local version goes to the trash, never lost.
                    let aside = self.conflict_name(p);
                    ops.push(Op::SetAside(p.into(), aside.clone()));
                    ops.push(Op::TrashLocal(aside));
                    self.push_download(p, c, None, ops);
                }
            }
            ("local_edit_cloud_delete", Resolution::KeepCloud) => ops.push(Op::TrashLocal(p.into())),
            ("local_edit_cloud_delete", _) => {
                if l.map(|l| l.is_dir).unwrap_or(false) {
                    ops.push(Op::CloudMkdir(p.into()));
                } else {
                    ops.push(Op::Upload(p.into(), None));
                }
            }
            ("local_delete_cloud_edit", Resolution::KeepLocal) => {
                if let Some(c) = c {
                    ops.push(Op::TrashCloud(p.into(), c.item.id.clone()));
                }
            }
            (_, _) => {
                if let Some(c) = c {
                    self.push_download(p, c, None, ops);
                }
            }
        }
    }

    // --- moves ---

    fn apply_cloud_move(&self, from: &str, to: &str) -> Result<()> {
        let dest = self.local_path(to);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        if dest.symlink_metadata().is_ok() {
            bail!("{to} already exists here");
        }
        std::fs::rename(self.local_path(from), &dest)?;
        Ok(())
    }

    fn apply_local_move(&self, from: &str, to: &str, base: &BTreeMap<String, Base>, cloud: &BTreeMap<String, CEntry>) -> Result<()> {
        let b = &base[from];
        let c = cloud.get(from).ok_or_else(|| anyhow!("not in the cloud"))?;
        let new_parent = self.cloud_parent(parent_of(to), base)?;
        let old_parent = c.item.parents.first().cloned().unwrap_or_default();
        let mut new_name = cloud_name(name_of(to));
        if b.kind == 'g' {
            // Keep the Google name without the local extension.
            let ext = gdocs::local_name(&c.item, self.ctx.export);
            let ext = split_ext(&ext).1;
            if let Some(s) = new_name.strip_suffix(ext) {
                new_name = s.to_string();
            }
        }
        let rename = (new_name != c.item.name).then_some(new_name.as_str());
        let mv = (new_parent != old_parent).then_some((old_parent.as_str(), new_parent.as_str()));
        if rename.is_some() || mv.is_some() {
            self.cloud.update_meta(&b.cl_id, rename, mv)?;
        }
        Ok(())
    }

    /// The cloud folder id of a local folder path (from base).
    fn cloud_parent(&self, dir: &str, base: &BTreeMap<String, Base>) -> Result<String> {
        if dir.is_empty() {
            return Ok(self.root_id.clone());
        }
        match base.get(dir) {
            Some(b) if b.kind == 'd' => Ok(b.cl_id.clone()),
            _ => bail!("its folder {dir} isn't in Google Drive yet"),
        }
    }

    fn cloud_parent_now(&self, dir: &str) -> Result<String> {
        if dir.is_empty() {
            return Ok(self.root_id.clone());
        }
        self.db.base_get(&self.root.id, dir)?.filter(|b| b.kind == 'd').map(|b| b.cl_id).ok_or_else(|| anyhow!("its folder {dir} isn't in Google Drive yet"))
    }

    // --- running ---

    fn run(&self, ops: Vec<Op>, rep: &mut Report) {
        let (mut seq, mut par, mut del) = (Vec::new(), Vec::new(), Vec::new());
        for o in ops {
            match o {
                Op::Download(..) | Op::Upload(..) | Op::Export(..) => par.push(o),
                Op::TrashLocal(..) | Op::TrashCloud(..) | Op::Unsync(..) => del.push(o),
                _ => seq.push(o),
            }
        }
        rep.pending = seq.len() + par.len() + del.len();
        // Set-asides first: their names must be free before downloads.
        seq.sort_by_key(|o| !matches!(o, Op::SetAside(..)));
        for o in seq {
            if self.cancel.load(Ordering::Relaxed) {
                return;
            }
            self.do_op(o, rep);
            rep.pending -= 1;
        }
        // Transfers in parallel, uploads of a folder after its creation
        // (the sequential phase made the folders).
        let queue = Mutex::new(par);
        let shared = Mutex::new(std::mem::take(rep));
        let n = self.ctx.transfers.max(1);
        std::thread::scope(|s| {
            for _ in 0..n {
                s.spawn(|| loop {
                    if self.cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(o) = queue.lock().pop() else { return };
                    let mut r = Report::default();
                    self.do_op(o, &mut r);
                    let mut sh = shared.lock();
                    sh.uploads += r.uploads;
                    sh.downloads += r.downloads;
                    sh.errors += r.errors;
                    sh.pending -= 1;
                });
            }
        });
        *rep = shared.into_inner();
        // Deletions deepest first.
        del.sort_by(|a, b| {
            let pa = match a {
                Op::TrashLocal(p) | Op::TrashCloud(p, _) | Op::Unsync(p) => p,
                _ => unreachable!(),
            };
            let pb = match b {
                Op::TrashLocal(p) | Op::TrashCloud(p, _) | Op::Unsync(p) => p,
                _ => unreachable!(),
            };
            pb.cmp(pa)
        });
        for o in del {
            if self.cancel.load(Ordering::Relaxed) {
                return;
            }
            self.do_op(o, rep);
            rep.pending -= 1;
        }
    }

    fn do_op(&self, o: Op, rep: &mut Report) {
        let path = match &o {
            Op::LocalMkdir(p, _) | Op::CloudMkdir(p) | Op::SetAside(p, _) | Op::Download(p, ..) | Op::WriteLink(p, _) | Op::Export(p, ..) | Op::Upload(p, _) | Op::CopyCloud(p, _) | Op::Forget(p) | Op::TrashLocal(p) | Op::TrashCloud(p, _) | Op::Unsync(p) => p.clone(),
            Op::Record(b) => b.path.clone(),
        };
        let quiet = matches!(o, Op::Record(_) | Op::Forget(_));
        if !quiet {
            self.busy.lock().insert(path.clone());
            self.emit(Event::Status { root: self.root.id.clone(), path: path.clone(), state: 'P' });
        }
        let r = self.op(o, rep);
        self.busy.lock().remove(&path);
        match r {
            Ok(()) => {
                if !quiet {
                    self.ok(&path);
                }
            }
            Err(e) => {
                rep.errors += 1;
                self.error(&path, &e);
            }
        }
    }

    fn op(&self, o: Op, rep: &mut Report) -> Result<()> {
        let rid = &self.root.id;
        match o {
            Op::Record(b) => self.db.base_put(rid, &b),
            Op::Forget(p) => self.db.base_remove_tree(rid, &p),
            Op::LocalMkdir(p, item) => {
                let lp = self.local_path(&p);
                std::fs::create_dir_all(&lp)?;
                let l = local::stat(&lp).ok_or_else(|| anyhow!("couldn't make the folder"))?;
                self.db.base_put(rid, &Self::base_for(&p, 'd', &item, &l))?;
                self.activity("add", &p, "folder from Google Drive");
                Ok(())
            }
            Op::CloudMkdir(p) => {
                let parent = self.cloud_parent_now(parent_of(&p))?;
                let item = self.cloud.create_folder(&parent, &cloud_name(name_of(&p)))?;
                let l = local::stat(&self.local_path(&p)).ok_or_else(|| anyhow!("the folder is gone"))?;
                self.db.base_put(rid, &Self::base_for(&p, 'd', &item, &l))?;
                self.activity("add", &p, "folder to Google Drive");
                Ok(())
            }
            Op::SetAside(p, aside) => {
                std::fs::rename(self.local_path(&p), self.local_path(&aside))?;
                self.db.base_remove_tree(rid, &p)?;
                Ok(())
            }
            Op::WriteLink(p, item) => {
                let lp = self.local_path(&p);
                std::fs::write(&lp, gdocs::link_contents(&item, &self.ctx.email))?;
                let _ = filetime::set_file_mtime(&lp, filetime::FileTime::from_unix_time(item.mtime as i64, ((item.mtime.fract()) * 1e9) as u32));
                let l = local::stat(&lp).ok_or_else(|| anyhow!("couldn't write the link"))?;
                self.db.base_put(rid, &Self::base_for(&p, 'g', &item, &l))?;
                Ok(())
            }
            Op::Download(p, item, expect) => {
                let tmp = self.part_dir().join(&item.id);
                let total = item.size.unwrap_or(0);
                let (root, path) = (rid.clone(), p.clone());
                let sink = self.sink.clone();
                let prog = move |done: u64| sink(Event::Transfer { root: root.clone(), path: path.clone(), up: false, done, total });
                let r = self.cloud.download(&item.id, &tmp, &prog);
                self.emit(Event::TransferDone { root: rid.clone(), path: p.clone(), up: false, ok: r.is_ok() });
                r?;
                if let Some(m) = &item.md5 {
                    let got = md5_file(&tmp)?;
                    if &got != m {
                        let _ = std::fs::remove_file(&tmp);
                        bail!("the download was damaged (checksum mismatch); it will be tried again");
                    }
                }
                self.place(&p, &tmp, &item, expect.as_ref(), 'f')?;
                rep.downloads += 1;
                self.activity("download", &p, "");
                Ok(())
            }
            Op::Export(p, item, mime, expect) => {
                let tmp = self.part_dir().join(format!("{}.export", item.id));
                self.cloud.export(&item.id, mime, &tmp)?;
                self.place(&p, &tmp, &item, expect.as_ref(), 'g')?;
                rep.downloads += 1;
                self.activity("download", &p, "converted from Google format");
                Ok(())
            }
            Op::Upload(p, existing) => {
                let lp = self.local_path(&p);
                let before = local::stat(&lp).ok_or_else(|| anyhow!("the file is gone"))?;
                let parent = match &existing {
                    Some(e) => e.parents.first().cloned().unwrap_or_else(|| self.root_id.clone()),
                    None => self.cloud_parent_now(parent_of(&p))?,
                };
                let (mime, kind) = match &existing {
                    Some(e) if e.is_google_format() => match gdocs::local_form(e, self.ctx.export) {
                        GLocal::Export(_, m) => (Some(m), 'g'),
                        GLocal::Link(_) => bail!("link files can't be uploaded"),
                    },
                    _ => (None, 'f'),
                };
                let name = match &existing {
                    Some(e) => e.name.clone(),
                    None => cloud_name(name_of(&p)),
                };
                let (root, path) = (rid.clone(), p.clone());
                let sink = self.sink.clone();
                let total = before.size;
                let prog = move |done: u64| sink(Event::Transfer { root: root.clone(), path: path.clone(), up: true, done, total });
                let r = self.cloud.upload(&parent, &name, existing.as_ref().map(|e| e.id.as_str()), &lp, mime, before.mtime, &prog);
                self.emit(Event::TransferDone { root: rid.clone(), path: p.clone(), up: true, ok: r.is_ok() });
                let item = r?;
                // Changed while uploading: record what was sent, so the
                // next pass uploads again.
                let after = local::stat(&lp).unwrap_or(before.clone());
                let l = if after == before { after } else { before };
                self.db.base_put(rid, &Self::base_for(&p, kind, &item, &l))?;
                rep.uploads += 1;
                self.activity(if existing.is_some() { "edit" } else { "upload" }, &p, "");
                Ok(())
            }
            Op::CopyCloud(p, from_id) => {
                let parent = self.cloud_parent_now(parent_of(&p))?;
                let src = self.cloud.get(&from_id)?.ok_or_else(|| anyhow!("the file this link points to is gone"))?;
                let name = split_ext(&cloud_name(name_of(&p))).0.to_string();
                let item = self.cloud.copy(&from_id, &parent, &name)?;
                let lp = self.local_path(&p);
                std::fs::write(&lp, gdocs::link_contents(&item, &self.ctx.email))?;
                let l = local::stat(&lp).ok_or_else(|| anyhow!("couldn't write the link"))?;
                self.db.base_put(rid, &Self::base_for(&p, 'g', &item, &l))?;
                self.activity("upload", &p, &format!("copy of {}", src.name));
                Ok(())
            }
            Op::TrashLocal(p) => {
                let lp = self.local_path(&p);
                if lp.symlink_metadata().is_ok() {
                    crate::trash::trash(&lp, self.ctx.trash_dir.as_deref()).map_err(|e| anyhow!(e))?;
                }
                self.db.base_remove_tree(rid, &p)?;
                rep.deletes += 1;
                self.activity("local_delete", &p, "moved to the Trash (deleted in Google Drive)");
                Ok(())
            }
            Op::TrashCloud(p, id) => {
                self.cloud.trash(&id)?;
                self.db.base_remove_tree(rid, &p)?;
                rep.deletes += 1;
                self.activity("delete", &p, "moved to Google Drive's trash");
                Ok(())
            }
            Op::Unsync(p) => {
                self.db.base_remove_tree(rid, &p)?;
                rep.unsynced.push(p.clone());
                self.activity("unsync", &p, "deleted while canopy wasn't running; kept in Google Drive");
                Ok(())
            }
        }
    }

    /// Move a finished download into place, unless the local file changed
    /// since the pass looked (then the next pass sorts it out).
    fn place(&self, p: &str, tmp: &Path, item: &CloudItem, expect: Option<&LocalItem>, kind: char) -> Result<()> {
        let lp = self.local_path(p);
        let now = local::stat(&lp);
        if now.as_ref() != expect {
            let _ = std::fs::remove_file(tmp);
            bail!("changed here while downloading; it will be compared again");
        }
        let ft = filetime::FileTime::from_unix_time(item.mtime as i64, (item.mtime.fract() * 1e9) as u32);
        filetime::set_file_mtime(tmp, ft)?;
        if let Some(d) = lp.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::rename(tmp, &lp)?;
        let l = local::stat(&lp).ok_or_else(|| anyhow!("the file vanished"))?;
        self.db.base_put(&self.root.id, &Self::base_for(p, kind, item, &l))?;
        Ok(())
    }

    /// Stop syncing a path: the local copy goes to the trash, the cloud
    /// copy stays (Insync's Unsync). The caller updates the selection.
    pub fn unsync(&self, rel: &str) -> Result<()> {
        let lp = self.local_path(rel);
        if lp.symlink_metadata().is_ok() {
            crate::trash::trash(&lp, self.ctx.trash_dir.as_deref()).map_err(|e| anyhow!(e))?;
        }
        self.db.base_remove_tree(&self.root.id, rel)?;
        self.activity("unsync", rel, "removed from this computer, kept in Google Drive");
        Ok(())
    }
}
