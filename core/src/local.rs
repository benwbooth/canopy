//! Reading a root's local tree: every folder and file with its size, mtime
//! and inode (inodes let moves be recognised), minus ignored names.

use crate::ignore::Ignore;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct LocalItem {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: f64,
    pub ino: u64,
}

#[derive(Default, Debug)]
pub struct Scan {
    /// Relative '/'-separated path -> item (the root itself is not listed).
    pub items: BTreeMap<String, LocalItem>,
    /// Symlinks found (not synced).
    pub symlinks: Vec<String>,
    /// Ignored by rules (shown in the activity feed once).
    pub ignored: Vec<String>,
    /// Couldn't be read (permissions).
    pub unreadable: Vec<(String, String)>,
}

pub fn mtime_of(m: &std::fs::Metadata) -> f64 {
    m.mtime() as f64 + m.mtime_nsec() as f64 / 1e9
}

pub fn item_of(m: &std::fs::Metadata) -> LocalItem {
    LocalItem { is_dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, mtime: mtime_of(m), ino: m.ino() }
}

pub fn stat(path: &Path) -> Option<LocalItem> {
    std::fs::symlink_metadata(path).ok().filter(|m| !m.file_type().is_symlink()).map(|m| item_of(&m))
}

pub fn scan(root: &Path, ignore: &Ignore) -> Scan {
    let mut out = Scan::default();
    walk(root, "", ignore, &mut out);
    out
}

fn walk(dir: &Path, rel: &str, ignore: &Ignore, out: &mut Scan) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            out.unreadable.push((rel.to_string(), e.to_string()));
            return;
        }
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            out.unreadable.push((format!("{rel}/{}", name.to_string_lossy()), "the name isn't valid UTF-8".into()));
            continue;
        };
        let path = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
        let Ok(m) = e.metadata() else { continue };
        if m.file_type().is_symlink() {
            out.symlinks.push(path);
            continue;
        }
        if !m.is_dir() && !m.is_file() {
            continue; // sockets, fifos, devices
        }
        if ignore.ignored(&path, m.is_dir()) {
            out.ignored.push(path);
            continue;
        }
        out.items.insert(path.clone(), item_of(&m));
        if m.is_dir() {
            walk(&e.path(), &path, ignore, out);
        }
    }
}
