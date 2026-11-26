//! canopy on rclone, end to end: the "remote" is a folder on this disk
//! (rclone's local backend), so someone else changing the cloud is plain
//! file operations there, and the memory backend stands in for services
//! without empty folders (S3). Skipped when rclone isn't installed.
//!
//! The randomized test checks what fuzz.rs checks for Google Drive: no
//! version that wasn't synced yet is lost (it's afterwards in the local
//! folder, the local trash, the remote, the remote's .canopy-trash, or the
//! other client's own trash), and once changes stop both sides are the same
//! and another pass does nothing. CANOPY_RCLONE_SEEDS=N runs more seeds.

use canopy_core::cloud::Cloud;
use canopy_core::config::{ConflictPolicy, ExportOption, Mode, Root};
use canopy_core::db::Db;
use canopy_core::engine::{md5_file, Ctx, Engine, PassOptions, Report};
use canopy_core::ignore::Ignore;
use canopy_core::mirror;
use canopy_core::rclone::{self, RcloneCloud, ROOT, TRASH_DIR};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Once};

static SETUP: Once = Once::new();

/// rclone on a short socket path and an empty config (never the user's).
fn setup() -> bool {
    SETUP.call_once(|| {
        let d = std::env::temp_dir().join(format!("cnrc-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("rclone.conf"), "").unwrap();
        std::env::set_var("CANOPY_RUNTIME_DIR", &d);
        std::env::set_var("RCLONE_CONFIG", d.join("rclone.conf"));
    });
    if !rclone::available() {
        eprintln!("rclone isn't installed: skipped");
        return false;
    }
    true
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

struct World {
    cloud: Arc<RcloneCloud>,
    eng: Engine,
    local: PathBuf,
    remote: PathBuf,
    trash: PathBuf,
    /// Where "someone else" puts what they delete on the remote.
    other_trash: PathBuf,
    _dir: tempfile::TempDir,
    counter: u64,
    log: Vec<String>,
}

const NAMES: [&str; 5] = ["a.txt", "b.txt", "c", "d.md", "e"];
const DIRS: [&str; 3] = ["x", "y", "z"];

fn engine(cloud: Arc<RcloneCloud>, db: Arc<Db>, local: &Path, trash: &Path, transfers: usize) -> Engine {
    let ctx = Ctx {
        email: "remote".into(),
        export: ExportOption::Link,
        conflicts: ConflictPolicy::default(),
        ignore: Ignore::new(&[]),
        trash_dir: Some(trash.to_path_buf()),
        transfers,
        sync_deletions: true,
        host: "h".into(),
    };
    let root = Root { id: "r".into(), local: local.to_path_buf(), name: "Remote".into(), mode: Mode::TwoWay, ..Default::default() };
    Engine {
        cloud,
        db,
        ctx,
        root,
        root_id: ROOT.into(),
        sink: Arc::new(|_| {}),
        cancel: Arc::new(AtomicBool::new(false)),
        busy: Default::default(),
        dry: Default::default(),
        checked: Default::default(),
    }
}

fn world(seed: u64) -> World {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("root");
    let remote = dir.path().join("remote");
    let trash = dir.path().join("trash");
    let other_trash = dir.path().join("other-trash");
    for d in [&local, &remote, &other_trash] {
        std::fs::create_dir_all(d).unwrap();
    }
    let db = Arc::new(Db::memory().unwrap());
    let mut c = RcloneCloud::new(&remote.to_string_lossy(), "Remote", db.clone()).unwrap();
    c.relist_secs = 0;
    // Half the seeds hash as OneDrive does (rclone hashes the local files).
    if seed % 2 == 0 {
        c.info.hash = Some("quickxor".into());
    }
    let cloud = Arc::new(c);
    let eng = engine(cloud.clone(), db, &local, &trash, 1 + (seed % 3) as usize);
    World { cloud, eng, local, remote, trash, other_trash, _dir: dir, counter: 0, log: vec![] }
}

fn files_under(dir: &Path, skip_part: bool) -> Vec<PathBuf> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if skip_part && e.file_name() == ".canopy-part" {
            continue;
        }
        if p.is_dir() {
            out.extend(files_under(&p, skip_part));
        } else {
            out.push(p);
        }
    }
    out
}

/// Dirs ("" first) and files of a tree (relative), leaving `skip` out.
fn entries(top: &Path, skip: &str) -> (Vec<String>, Vec<String>) {
    let mut dirs = vec![String::new()];
    let mut files = vec![];
    fn walk(d: &Path, rel: &str, skip: &str, dirs: &mut Vec<String>, files: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut v: Vec<_> = rd.flatten().collect();
        v.sort_by_key(|e| e.file_name());
        for e in v {
            let n = e.file_name().to_string_lossy().into_owned();
            if rel.is_empty() && n == skip {
                continue;
            }
            let r = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
            if e.path().is_dir() {
                dirs.push(r.clone());
                walk(&e.path(), &r, skip, dirs, files);
            } else {
                files.push(r);
            }
        }
    }
    walk(top, "", skip, &mut dirs, &mut files);
    (dirs, files)
}

fn bump(p: &Path, by: i64) {
    if let Ok(m) = std::fs::metadata(p) {
        let t = filetime::FileTime::from_last_modification_time(&m);
        let _ = filetime::set_file_mtime(p, filetime::FileTime::from_unix_time(t.unix_seconds() + by, 0));
    }
}

impl World {
    fn content(&mut self) -> Vec<u8> {
        self.counter += 1;
        format!("v{:07}", self.counter).into_bytes()
    }

    /// One random change in a tree (the local folder, or the remote as
    /// another client would make it).
    fn op(&mut self, r: &mut Rng, remote: bool) {
        let (top, skip, who) = if remote { (self.remote.clone(), TRASH_DIR, "remote") } else { (self.local.clone(), ".canopy-part", "local") };
        let (dirs, files) = entries(&top, skip);
        match r.below(7) {
            0 | 1 => {
                let d = &dirs[r.below(dirs.len())];
                let rel = if d.is_empty() { NAMES[r.below(5)].to_string() } else { format!("{d}/{}", NAMES[r.below(5)]) };
                let p = top.join(&rel);
                if p.exists() {
                    return;
                }
                let c = self.content();
                std::fs::write(&p, &c).unwrap();
                self.log.push(format!("{who} write {rel} {}", String::from_utf8_lossy(&c)));
            }
            2 if !files.is_empty() => {
                let rel = files[r.below(files.len())].clone();
                let c = self.content();
                let p = top.join(&rel);
                std::fs::write(&p, &c).unwrap();
                bump(&p, 7);
                self.log.push(format!("{who} edit {rel} {}", String::from_utf8_lossy(&c)));
            }
            3 if !files.is_empty() => {
                let rel = files[r.below(files.len())].clone();
                if remote {
                    // Another client's delete: into its own trash.
                    let dst = self.other_trash.join(format!("{}", self.counter)).join(&rel);
                    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
                    self.counter += 1;
                    std::fs::rename(top.join(&rel), dst).unwrap();
                } else {
                    std::fs::remove_file(top.join(&rel)).unwrap();
                }
                self.log.push(format!("{who} delete {rel}"));
            }
            4 => {
                let d = &dirs[r.below(dirs.len())];
                let rel = if d.is_empty() { DIRS[r.below(3)].to_string() } else { format!("{d}/{}", DIRS[r.below(3)]) };
                if rel.matches('/').count() < 2 && std::fs::create_dir(top.join(&rel)).is_ok() {
                    self.log.push(format!("{who} mkdir {rel}"));
                }
            }
            5 if dirs.len() > 1 => {
                let rel = dirs[1 + r.below(dirs.len() - 1)].clone();
                if remote {
                    let dst = self.other_trash.join(format!("{}", self.counter)).join(&rel);
                    self.counter += 1;
                    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
                    std::fs::rename(top.join(&rel), dst).unwrap();
                } else {
                    std::fs::remove_dir_all(top.join(&rel)).unwrap();
                }
                self.log.push(format!("{who} rmdir {rel}"));
            }
            6 => {
                let all: Vec<String> = dirs.iter().skip(1).cloned().chain(files.iter().cloned()).collect();
                if all.is_empty() {
                    return;
                }
                let from = all[r.below(all.len())].clone();
                let to_dir = dirs[r.below(dirs.len())].clone();
                if to_dir == from || to_dir.starts_with(&format!("{from}/")) {
                    return;
                }
                let name = if r.chance(50) { from.rsplit('/').next().unwrap().to_string() } else if top.join(&from).is_dir() { DIRS[r.below(3)].to_string() } else { NAMES[r.below(5)].to_string() };
                let to = if to_dir.is_empty() { name } else { format!("{to_dir}/{name}") };
                if to == from || top.join(&to).exists() || to.matches('/').count() > 2 {
                    return;
                }
                std::fs::rename(top.join(&from), top.join(&to)).unwrap();
                self.log.push(format!("{who} move {from} -> {to}"));
            }
            _ => {}
        }
    }

    fn unsynced(&self) -> HashSet<String> {
        let synced: HashSet<String> = self.eng.db.base_all("r").unwrap().iter().filter(|b| b.kind == 'f').filter_map(|b| b.c_hash.clone()).collect();
        let mut out = HashSet::new();
        for f in files_under(&self.local, true) {
            // (The engine's own hash kind, to compare with what base holds.)
            let h = self.cloud.hash_local(&f).unwrap();
            if !synced.contains(&h) {
                out.insert(md5_file(&f).unwrap());
            }
        }
        for (_, files) in [entries(&self.remote, TRASH_DIR)] {
            for f in files {
                let p = self.remote.join(&f);
                if !synced.contains(&self.cloud.hash_local(&p).unwrap()) {
                    out.insert(md5_file(&p).unwrap());
                }
            }
        }
        out
    }

    fn everywhere(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for f in files_under(&self.local, true).into_iter().chain(files_under(&self.trash, false)).chain(files_under(&self.remote, false)).chain(files_under(&self.other_trash, false)) {
            if let Ok(m) = md5_file(&f) {
                out.insert(m);
            }
        }
        out
    }

    fn fail(&self, seed: u64, what: &str) -> ! {
        panic!("seed {seed}: {what}\n--- operations ---\n{}", self.log.join("\n"));
    }

    fn pass(&mut self, seed: u64, refresh: bool) -> Report {
        let before = self.unsynced();
        if refresh {
            if let Err(e) = mirror::refresh(self.cloud.as_ref(), &self.eng.db, None) {
                self.fail(seed, &format!("refresh: {e:#}"));
            }
        }
        let opts = PassOptions { confirm_first: true, allow_empty: true, confirm_deletes: true, ..Default::default() };
        let rep = match self.eng.pass(&opts) {
            Ok(r) => r,
            Err(e) => self.fail(seed, &format!("pass: {e:#}")),
        };
        self.log.push(format!("  pass{}: {rep:?}", if refresh { "" } else { " (stale)" }));
        let now = self.everywhere();
        let lost: Vec<String> = before.difference(&now).map(|m| self.name_of_md5(m)).collect();
        if !lost.is_empty() {
            self.fail(seed, &format!("lost unsynced versions {lost:?}"));
        }
        rep
    }

    fn name_of_md5(&self, m: &str) -> String {
        for i in 1..=self.counter {
            let c = format!("v{i:07}");
            let d = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(d.path(), &c).unwrap();
            if md5_file(d.path()).unwrap() == m {
                return c;
            }
        }
        m.to_string()
    }

    fn trees(&self) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        let tree = |top: &Path, skip: &str| {
            let (dirs, files) = entries(top, skip);
            let mut m: BTreeMap<String, String> = dirs.into_iter().filter(|d| !d.is_empty()).map(|d| (d, "/".into())).collect();
            for f in files {
                m.insert(f.clone(), String::from_utf8_lossy(&std::fs::read(top.join(&f)).unwrap()).into_owned());
            }
            m
        };
        (tree(&self.local, ".canopy-part"), tree(&self.remote, TRASH_DIR))
    }
}

fn run(seed: u64) {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut w = world(seed);
    for _ in 0..14 {
        let kind = r.below(4);
        for _ in 0..1 + r.below(4) {
            let remote = !r.chance(55);
            w.op(&mut r, remote);
        }
        match kind {
            // The remote changes after canopy looked.
            0 => {
                mirror::refresh(w.cloud.as_ref(), &w.eng.db, None).unwrap();
                w.op(&mut r, true);
                w.pass(seed, false);
            }
            1 => {
                w.pass(seed, false);
            }
            _ => {
                w.pass(seed, true);
            }
        }
    }
    for _ in 0..6 {
        w.pass(seed, true);
    }
    let rep = w.pass(seed, true);
    if rep.uploads + rep.downloads + rep.moves + rep.deletes + rep.errors > 0 {
        let plan = w.eng.pass(&PassOptions { dry_run: true, ..Default::default() }).map(|r| r.plan).unwrap_or_default();
        let base: Vec<String> = w.eng.db.base_all("r").unwrap().iter().map(|b| format!("{} {} {:?} {}", b.path, b.cl_id, b.c_hash, b.l_mtime)).collect();
        w.fail(seed, &format!("still changing after settling: {rep:?}\nplan: {plan:?}\nbase: {base:?}\ntrees: {:?}", w.trees()));
    }
    let (l, c) = w.trees();
    if l != c {
        let only_l: Vec<_> = l.iter().filter(|(k, v)| c.get(*k) != Some(v)).collect();
        let only_c: Vec<_> = c.iter().filter(|(k, v)| l.get(*k) != Some(v)).collect();
        w.fail(seed, &format!("sides differ:\n here: {only_l:?}\n remote: {only_c:?}"));
    }
}

#[test]
fn rclone_random_changes_never_lose_data() {
    if !setup() {
        return;
    }
    if let Some(one) = std::env::var("CANOPY_RCLONE_SEED").ok().and_then(|v| v.parse().ok()) {
        return run(one);
    }
    let n: u64 = std::env::var("CANOPY_RCLONE_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    for seed in 1..=n {
        run(seed);
    }
}

/// The basics, step by step, on a local-folder remote.
#[test]
fn rclone_basics() {
    if !setup() {
        return;
    }
    let mut w = world(1);
    let (l, rm) = (w.local.clone(), w.remote.clone());
    std::fs::create_dir_all(rm.join("Docs")).unwrap();
    std::fs::write(rm.join("Docs/a.txt"), "from remote").unwrap();
    std::fs::write(l.join("b.txt"), "from here").unwrap();
    w.pass(1, true);
    assert_eq!(std::fs::read_to_string(l.join("Docs/a.txt")).unwrap(), "from remote");
    assert_eq!(std::fs::read_to_string(rm.join("b.txt")).unwrap(), "from here");

    // A folder renamed here is renamed there (not uploaded again).
    std::fs::rename(l.join("Docs"), l.join("Papers")).unwrap();
    let rep = w.pass(1, true);
    assert_eq!((rep.moves, rep.uploads), (1, 0), "{rep:?}");
    assert!(rm.join("Papers/a.txt").exists() && !rm.join("Docs").exists());

    // Deleted here: into the remote's .canopy-trash, never gone.
    std::fs::remove_file(l.join("b.txt")).unwrap();
    w.pass(1, true);
    assert!(!rm.join("b.txt").exists());
    let kept = files_under(&rm.join(TRASH_DIR), false);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "from here");
    // ...and the trash folder isn't synced back.
    w.pass(1, true);
    assert!(!l.join(TRASH_DIR).exists());

    // Edited on both sides: both kept.
    std::fs::write(l.join("Papers/a.txt"), "mine").unwrap();
    bump(&l.join("Papers/a.txt"), 5);
    std::fs::write(rm.join("Papers/a.txt"), "theirs").unwrap();
    bump(&rm.join("Papers/a.txt"), 9);
    w.pass(1, true);
    w.pass(1, true);
    let (here, there) = w.trees();
    assert_eq!(here, there);
    let vals: Vec<&String> = here.values().collect();
    assert!(vals.contains(&&"mine".to_string()) && vals.contains(&&"theirs".to_string()), "{here:?}");
}

/// A service without empty folders (rclone's memory backend, like S3): a
/// folder made here stays a folder there until something goes in it, and
/// doesn't vanish when it empties.
#[test]
fn rclone_without_empty_folders() {
    if !setup() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("root");
    let trash = dir.path().join("trash");
    std::fs::create_dir_all(&local).unwrap();
    let db = Arc::new(Db::memory().unwrap());
    let fs = format!(":memory:canopy-test-{}", std::process::id());
    let mut c = RcloneCloud::new(&fs, "Bucket", db.clone()).unwrap();
    c.relist_secs = 0;
    assert!(!c.info.empty_dirs);
    let cloud = Arc::new(c);
    let eng = engine(cloud.clone(), db, &local, &trash, 2);
    let sync = || {
        mirror::refresh(cloud.as_ref(), &eng.db, None).unwrap();
        eng.pass(&PassOptions { confirm_first: true, ..Default::default() }).unwrap()
    };
    std::fs::create_dir_all(local.join("Empty")).unwrap();
    std::fs::create_dir_all(local.join("Full")).unwrap();
    std::fs::write(local.join("Full/f.txt"), "x").unwrap();
    sync();
    for _ in 0..2 {
        let rep = sync();
        assert_eq!(rep.uploads + rep.downloads + rep.deletes + rep.errors, 0, "{rep:?}");
    }
    assert!(local.join("Empty").is_dir());
    // The last file goes: the folder stays on both sides.
    std::fs::remove_file(local.join("Full/f.txt")).unwrap();
    sync();
    sync();
    assert!(local.join("Full").is_dir(), "an emptied folder must not be deleted here");
    // A file into the empty folder lands.
    std::fs::write(local.join("Empty/g.txt"), "y").unwrap();
    sync();
    let it = cloud.get(&eng.db.base_get("r", "Empty/g.txt").unwrap().unwrap().cl_id).unwrap().unwrap();
    assert_eq!(it.size, Some(1));
}
