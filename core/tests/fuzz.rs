//! Randomized sync safety test: random changes on both sides (create,
//! edit, delete, move, rename, folders), sometimes racing canopy (a stale
//! mirror, Drive changing after canopy looked), then sync passes.
//!
//! After every pass:
//! - no change that wasn't synced yet is lost: every version present on
//!   either side before the pass that canopy hadn't synced is afterwards in
//!   the local folder, the local trash, or Google Drive (live or trashed);
//! and once the changes stop:
//! - both sides hold the same files with the same contents, and another
//!   pass does nothing.
//!
//! CANOPY_FUZZ_SEEDS=N (default 150) runs more seeds; a failure prints the
//! seed and the operations.

use canopy_core::cloud::Cloud;
use canopy_core::config::{ConflictPolicy, ExportOption, Mode, Root};
use canopy_core::db::Db;
use canopy_core::engine::{md5_file, Ctx, Engine, PassOptions};
use canopy_core::fake::{hex_md5, FakeCloud, ROOT};
use canopy_core::ignore::Ignore;
use canopy_core::mirror;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
    cloud: Arc<FakeCloud>,
    eng: Engine,
    local: PathBuf,
    trash: PathBuf,
    _dir: tempfile::TempDir,
    counter: u64,
    log: Vec<String>,
    traced: usize,
}

const NAMES: [&str; 5] = ["a.txt", "b.txt", "c", "d.md", "e"];
const DIRS: [&str; 3] = ["x", "y", "z"];

fn world(seed: u64) -> World {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("root");
    let trash = dir.path().join("trash");
    let cloud = Arc::new(FakeCloud::new());
    let ctx = Ctx {
        email: "t@example.com".into(),
        export: ExportOption::Link,
        conflicts: ConflictPolicy::default(),
        ignore: Ignore::new(&[]),
        trash_dir: Some(trash.clone()),
        transfers: 1 + (seed % 3) as usize,
        sync_deletions: true,
        host: "h".into(),
    };
    let root = Root { id: "r".into(), local: local.clone(), name: "My Drive".into(), mode: Mode::TwoWay, ..Default::default() };
    let eng = Engine {
        cloud: cloud.clone(),
        db: Arc::new(Db::memory().unwrap()),
        ctx,
        root,
        root_id: ROOT.into(),
        sink: Arc::new(|_| {}),
        cancel: Arc::new(AtomicBool::new(false)),
        busy: Default::default(),
        dry: Default::default(),
        checked: Default::default(),
    };
    std::fs::create_dir_all(&local).unwrap();
    World { cloud, eng, local, trash, _dir: dir, counter: 0, log: vec![], traced: 0 }
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

impl World {
    fn content(&mut self) -> Vec<u8> {
        self.counter += 1;
        format!("v{:07}", self.counter).into_bytes()
    }

    fn bump(p: &Path, by: i64) {
        if let Ok(m) = std::fs::metadata(p) {
            let t = filetime::FileTime::from_last_modification_time(&m);
            let _ = filetime::set_file_mtime(p, filetime::FileTime::from_unix_time(t.unix_seconds() + by, 0));
        }
    }

    /// Local dirs and files (relative).
    fn local_entries(&self) -> (Vec<String>, Vec<String>) {
        let mut dirs = vec![String::new()];
        let mut files = vec![];
        fn walk(d: &Path, rel: &str, dirs: &mut Vec<String>, files: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            let mut v: Vec<_> = rd.flatten().collect();
            v.sort_by_key(|e| e.file_name());
            for e in v {
                let n = e.file_name().to_string_lossy().into_owned();
                if n == ".canopy-part" {
                    continue;
                }
                let r = if rel.is_empty() { n.clone() } else { format!("{rel}/{n}") };
                if e.path().is_dir() {
                    dirs.push(r.clone());
                    walk(&e.path(), &r, dirs, files);
                } else {
                    files.push(r);
                }
            }
        }
        walk(&self.local, "", &mut dirs, &mut files);
        (dirs, files)
    }

    /// Live cloud items: (id, path, is_folder).
    fn cloud_entries(&self) -> Vec<(String, String, bool)> {
        let mut out = vec![];
        let mut stack = vec![(ROOT.to_string(), String::new())];
        while let Some((id, rel)) = stack.pop() {
            // Same-named items: the oldest keeps the name, the others are
            // "name (2).ext"... (as canopy shows them).
            let mut kids = self.cloud.children(&id);
            kids.sort_by(|a, b| a.created.partial_cmp(&b.created).unwrap().then_with(|| a.id.cmp(&b.id)));
            let mut seen: std::collections::HashMap<String, u32> = Default::default();
            for c in kids {
                let n = seen.entry(c.name.clone()).or_insert(0);
                *n += 1;
                let name = if *n == 1 {
                    c.name.clone()
                } else {
                    match c.name.rfind('.') {
                        Some(i) if i > 0 => format!("{} ({n}){}", &c.name[..i], &c.name[i..]),
                        _ => format!("{} ({n})", c.name),
                    }
                };
                let p = if rel.is_empty() { name } else { format!("{rel}/{name}") };
                if c.is_folder() {
                    stack.push((c.id.clone(), p.clone()));
                }
                out.push((c.id.clone(), p, c.is_folder()));
            }
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    fn local_op(&mut self, r: &mut Rng) {
        let (dirs, files) = self.local_entries();
        match r.below(7) {
            0 | 1 => {
                let d = &dirs[r.below(dirs.len())];
                let rel = if d.is_empty() { NAMES[r.below(5)].to_string() } else { format!("{d}/{}", NAMES[r.below(5)]) };
                let p = self.local.join(&rel);
                if p.exists() {
                    return;
                }
                let c = self.content();
                std::fs::write(&p, &c).unwrap();
                self.log.push(format!("local write {rel} {}", String::from_utf8_lossy(&c)));
            }
            2 if !files.is_empty() => {
                let rel = files[r.below(files.len())].clone();
                let c = self.content();
                let p = self.local.join(&rel);
                std::fs::write(&p, &c).unwrap();
                Self::bump(&p, 7);
                self.log.push(format!("local edit {rel} {}", String::from_utf8_lossy(&c)));
            }
            3 if !files.is_empty() => {
                let rel = files[r.below(files.len())].clone();
                std::fs::remove_file(self.local.join(&rel)).unwrap();
                self.log.push(format!("local delete {rel}"));
            }
            4 => {
                let d = &dirs[r.below(dirs.len())];
                let rel = if d.is_empty() { DIRS[r.below(3)].to_string() } else { format!("{d}/{}", DIRS[r.below(3)]) };
                if rel.matches('/').count() < 2 && std::fs::create_dir(self.local.join(&rel)).is_ok() {
                    self.log.push(format!("local mkdir {rel}"));
                }
            }
            5 if dirs.len() > 1 => {
                let rel = dirs[1 + r.below(dirs.len() - 1)].clone();
                std::fs::remove_dir_all(self.local.join(&rel)).unwrap();
                self.log.push(format!("local rmdir {rel}"));
            }
            6 => {
                // Move or rename a file or folder.
                let all: Vec<String> = dirs.iter().skip(1).cloned().chain(files.iter().cloned()).collect();
                if all.is_empty() {
                    return;
                }
                let from = all[r.below(all.len())].clone();
                let to_dir = dirs[r.below(dirs.len())].clone();
                if to_dir == from || to_dir.starts_with(&format!("{from}/")) {
                    return;
                }
                let name = if r.chance(50) { from.rsplit('/').next().unwrap().to_string() } else if self.local.join(&from).is_dir() { DIRS[r.below(3)].to_string() } else { NAMES[r.below(5)].to_string() };
                let to = if to_dir.is_empty() { name } else { format!("{to_dir}/{name}") };
                if to == from || self.local.join(&to).exists() || to.matches('/').count() > 2 {
                    return;
                }
                std::fs::rename(self.local.join(&from), self.local.join(&to)).unwrap();
                self.log.push(format!("local move {from} -> {to}"));
            }
            _ => {}
        }
    }

    fn cloud_op(&mut self, r: &mut Rng) {
        let ents = self.cloud_entries();
        let folders: Vec<(String, String)> = std::iter::once((ROOT.to_string(), String::new())).chain(ents.iter().filter(|e| e.2).map(|e| (e.0.clone(), e.1.clone()))).collect();
        let files: Vec<&(String, String, bool)> = ents.iter().filter(|e| !e.2).collect();
        let free = |parent: &str, name: &str| self.cloud.child(parent, name).is_none();
        match r.below(7) {
            0 | 1 => {
                let (fid, fp) = folders[r.below(folders.len())].clone();
                let name = NAMES[r.below(5)];
                if !free(&fid, name) {
                    return;
                }
                let c = self.content();
                self.cloud.put(&fid, name, &c);
                self.log.push(format!("cloud put {fp}/{name} {}", String::from_utf8_lossy(&c)));
            }
            2 if !files.is_empty() => {
                let (id, p, _) = files[r.below(files.len())].clone();
                let c = self.content();
                self.cloud.edit(&id, &c);
                self.log.push(format!("cloud edit {p} {}", String::from_utf8_lossy(&c)));
            }
            3 if !ents.is_empty() => {
                let (id, p, _) = ents[r.below(ents.len())].clone();
                self.cloud.trash(&id).unwrap();
                self.log.push(format!("cloud trash {p}"));
            }
            4 => {
                let (fid, fp) = folders[r.below(folders.len())].clone();
                let name = DIRS[r.below(3)];
                if fp.matches('/').count() < 1 && free(&fid, name) {
                    self.cloud.mkdir(&fid, name);
                    self.log.push(format!("cloud mkdir {fp}/{name}"));
                }
            }
            5 | 6 if !ents.is_empty() => {
                let (id, p, is_dir) = ents[r.below(ents.len())].clone();
                let (fid, fp) = folders[r.below(folders.len())].clone();
                if fid == id || fp == p || fp.starts_with(&format!("{p}/")) || fp.matches('/').count() > 1 {
                    return;
                }
                let name = if r.chance(50) { p.rsplit('/').next().unwrap().to_string() } else if is_dir { DIRS[r.below(3)].into() } else { NAMES[r.below(5)].into() };
                if !free(&fid, &name) {
                    return;
                }
                let cur = self.cloud.item(&id).unwrap();
                if cur.parents[0] != fid {
                    self.cloud.mv(&id, &fid);
                }
                if cur.name != name {
                    self.cloud.rename(&id, &name);
                }
                self.log.push(format!("cloud move {p} -> {fp}/{name}"));
            }
            _ => {}
        }
    }

    /// Versions on either side that canopy hasn't synced (md5s).
    fn unsynced(&self) -> HashSet<String> {
        let base = self.eng.db.base_all("r").unwrap();
        let synced: HashSet<String> = base.iter().filter_map(|b| b.c_hash.clone()).collect();
        let mut out = HashSet::new();
        for f in files_under(&self.local, true) {
            let m = md5_file(&f).unwrap();
            if !synced.contains(&m) {
                out.insert(m);
            }
        }
        for (id, _, dir) in self.cloud_entries() {
            if !dir {
                let m = hex_md5(&self.cloud.content(&id).unwrap());
                if !synced.contains(&m) {
                    out.insert(m);
                }
            }
        }
        out
    }

    /// Every version that still exists somewhere (md5s).
    fn everywhere(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for f in files_under(&self.local, true).into_iter().chain(files_under(&self.trash, false)) {
            if let Ok(m) = md5_file(&f) {
                out.insert(m);
            }
        }
        for c in self.cloud.all_contents() {
            out.insert(hex_md5(&c));
        }
        out
    }

    fn fail(&self, seed: u64, what: &str) -> ! {
        if let Ok(name) = std::env::var("CANOPY_FUZZ_DUMP") {
            for it in self.eng.db.items_where("name = ?1", &name, 50).unwrap_or_default() {
                eprintln!("mirror: {} parents={:?} trashed={} md5={:?} v={}", it.id, it.parents, it.trashed, it.hash, it.version);
            }
            for b in self.eng.db.base_all("r").unwrap() {
                eprintln!("base: {} {} cl={} md5={:?}", b.path, b.kind, b.cl_id, b.c_hash);
            }
            for (id, p, _) in self.cloud_entries() {
                let it = self.cloud.item(&id).unwrap();
                eprintln!("drive: {p} {id} trashed={} parents={:?}", it.trashed, it.parents);
            }
        }
        panic!("seed {seed}: {what}\n--- operations ---\n{}", self.log.join("\n"));
    }

    fn pass(&mut self, seed: u64, refresh: bool) {
        if std::env::var_os("CANOPY_FUZZ_TRACE").is_some() {
            for l in &self.log[self.traced..] {
                eprintln!("{l}");
            }
            eprintln!("-- pass{}", if refresh { "" } else { " (stale)" });
            self.traced = self.log.len();
        }
        let before = self.unsynced();
        if refresh {
            mirror::refresh(self.cloud.as_ref(), &self.eng.db, None).unwrap();
        }
        let opts = PassOptions { confirm_first: true, allow_empty: true, confirm_deletes: true, ..Default::default() };
        let rep = self.eng.pass(&opts).unwrap();
        self.log.push(format!("  pass{}: {rep:?}", if refresh { "" } else { " (stale)" }));
        let now = self.everywhere();
        let lost: Vec<_> = before.difference(&now).collect();
        if !lost.is_empty() {
            let names: Vec<String> = lost.iter().map(|m| self.name_of_md5(m)).collect();
            self.fail(seed, &format!("lost unsynced versions {names:?}"));
        }
    }

    fn name_of_md5(&self, m: &str) -> String {
        for i in 1..=self.counter {
            let c = format!("v{i:07}");
            if hex_md5(c.as_bytes()) == m {
                return c;
            }
        }
        m.to_string()
    }

    fn trees(&self) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        let (ldirs, lfiles) = self.local_entries();
        let mut l: BTreeMap<String, String> = ldirs.into_iter().filter(|d| !d.is_empty()).map(|d| (d, "/".into())).collect();
        for f in lfiles {
            l.insert(f.clone(), String::from_utf8_lossy(&std::fs::read(self.local.join(&f)).unwrap()).into_owned());
        }
        let mut c = BTreeMap::new();
        for (id, p, dir) in self.cloud_entries() {
            c.insert(p, if dir { "/".into() } else { String::from_utf8_lossy(&self.cloud.content(&id).unwrap()).into_owned() });
        }
        (l, c)
    }
}

fn run(seed: u64) {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut w = world(seed);
    let rounds = 18;
    for _ in 0..rounds {
        let kind = r.below(4);
        for _ in 0..1 + r.below(4) {
            if r.chance(55) {
                w.local_op(&mut r);
            } else {
                w.cloud_op(&mut r);
            }
        }
        match kind {
            // Drive changes after canopy looked (refresh, change, pass).
            0 => {
                mirror::refresh(w.cloud.as_ref(), &w.eng.db, None).unwrap();
                w.cloud_op(&mut r);
                w.pass(seed, false);
            }
            // A pass with a mirror that hasn't seen the latest changes.
            1 => w.pass(seed, false),
            _ => w.pass(seed, true),
        }
    }
    // Settle: no more changes; the two sides must end up the same.
    for _ in 0..6 {
        w.pass(seed, true);
    }
    w.cloud.calls();
    w.pass(seed, true);
    let calls = w.cloud.calls();
    if !calls.is_empty() {
        w.fail(seed, &format!("still changing after settling: {calls:?}"));
    }
    let (l, c) = w.trees();
    if l != c {
        let only_l: Vec<_> = l.iter().filter(|(k, v)| c.get(*k) != Some(v)).collect();
        let only_c: Vec<_> = c.iter().filter(|(k, v)| l.get(*k) != Some(v)).collect();
        w.fail(seed, &format!("sides differ:\n here: {only_l:?}\n drive: {only_c:?}"));
    }
}

#[test]
fn random_changes_never_lose_data() {
    if let Some(one) = std::env::var("CANOPY_FUZZ_SEED").ok().and_then(|v| v.parse().ok()) {
        return run(one);
    }
    let n: u64 = std::env::var("CANOPY_FUZZ_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
    let first: u64 = std::env::var("CANOPY_FUZZ_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for seed in first..=n {
        if std::env::var_os("CANOPY_FUZZ_VERBOSE").is_some() {
            eprintln!("seed {seed}");
        }
        run(seed);
    }
}
