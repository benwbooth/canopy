//! Watching the sync roots with inotify (one watch per folder, as Insync):
//! any change marks its root dirty; new folders are watched as they appear.
//! When the kernel's queue overflows, every root is marked dirty (a full
//! rescan catches up).

use crate::ignore::PART_DIR;
use crossbeam_channel::Sender;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// (account id, root id)
pub type RootKey = (String, String);

#[derive(Default)]
struct Maps {
    wd: HashMap<WatchDescriptor, (RootKey, PathBuf)>,
    roots: HashMap<RootKey, PathBuf>,
}

#[derive(Clone)]
pub struct Watcher {
    ino: Arc<Mutex<Inotify>>,
    maps: Arc<Mutex<Maps>>,
}

fn mask() -> WatchMask {
    WatchMask::CREATE | WatchMask::DELETE | WatchMask::MODIFY | WatchMask::CLOSE_WRITE | WatchMask::MOVED_FROM | WatchMask::MOVED_TO | WatchMask::ATTRIB | WatchMask::DELETE_SELF | WatchMask::MOVE_SELF | WatchMask::ONLYDIR
}

impl Watcher {
    /// Start the watcher; dirty roots are sent to `tx`.
    pub fn start(tx: Sender<RootKey>) -> std::io::Result<Watcher> {
        let ino = Inotify::init()?;
        let w = Watcher { ino: Arc::new(Mutex::new(ino)), maps: Arc::default() };
        let w2 = w.clone();
        std::thread::Builder::new().name("inotify".into()).spawn(move || w2.run(tx))?;
        Ok(w)
    }

    pub fn watch_root(&self, key: RootKey, path: &Path) {
        self.maps.lock().roots.insert(key.clone(), path.to_path_buf());
        self.add_tree(&key, path);
    }

    pub fn unwatch_root(&self, key: &RootKey) {
        let mut m = self.maps.lock();
        m.roots.remove(key);
        let gone: Vec<WatchDescriptor> = m.wd.iter().filter(|(_, (k, _))| k == key).map(|(w, _)| w.clone()).collect();
        let ino = self.ino.lock();
        for w in gone {
            let _ = ino.watches().remove(w.clone());
            m.wd.remove(&w);
        }
    }

    fn add_tree(&self, key: &RootKey, dir: &Path) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            if d.file_name().map(|n| n == PART_DIR).unwrap_or(false) {
                continue;
            }
            let wd = self.ino.lock().watches().add(&d, mask());
            match wd {
                Ok(wd) => {
                    self.maps.lock().wd.insert(wd, (key.clone(), d.clone()));
                }
                Err(e) => {
                    if e.raw_os_error() == Some(libc::ENOSPC) {
                        eprintln!("canopy: out of inotify watches (raise fs.inotify.max_user_watches); changes in {} are found by periodic scans", d.display());
                        return;
                    }
                    continue;
                }
            }
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        stack.push(e.path());
                    }
                }
            }
        }
    }

    fn run(&self, tx: Sender<RootKey>) {
        let mut buf = vec![0u8; 64 * 1024];
        // Read without holding the lock while blocked: poll the fd.
        let fd = {
            use std::os::fd::AsRawFd;
            self.ino.lock().as_raw_fd()
        };
        loop {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if r <= 0 {
                continue;
            }
            let mut dirty: Vec<RootKey> = Vec::new();
            let mut new_dirs: Vec<(RootKey, PathBuf)> = Vec::new();
            let mut dropped: Vec<WatchDescriptor> = Vec::new();
            {
                let mut ino = self.ino.lock();
                let events = match ino.read_events(&mut buf) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let maps = self.maps.lock();
                for ev in events {
                    if ev.mask.contains(EventMask::Q_OVERFLOW) {
                        dirty.extend(maps.roots.keys().cloned());
                        continue;
                    }
                    if ev.mask.contains(EventMask::IGNORED) {
                        dropped.push(ev.wd.clone());
                    }
                    let Some((key, dir)) = maps.wd.get(&ev.wd) else { continue };
                    if let Some(name) = ev.name {
                        if name == PART_DIR {
                            continue;
                        }
                        if ev.mask.contains(EventMask::ISDIR) && (ev.mask.contains(EventMask::CREATE) || ev.mask.contains(EventMask::MOVED_TO)) {
                            new_dirs.push((key.clone(), dir.join(name)));
                        }
                    }
                    if !dirty.contains(key) {
                        dirty.push(key.clone());
                    }
                }
            }
            for (k, d) in new_dirs {
                self.add_tree(&k, &d);
            }
            if !dropped.is_empty() {
                // Watches of deleted folders end by themselves (IN_IGNORED).
                let mut m = self.maps.lock();
                for w in dropped {
                    m.wd.remove(&w);
                }
            }
            for k in dirty {
                let _ = tx.send(k);
            }
        }
    }
}
