//! An in-memory Drive for tests: folders, files with content, Google Docs,
//! trash, duplicate names and a changes feed, all behind the `Cloud` trait.

use crate::cloud::*;
use anyhow::{anyhow, bail, Result};
use md5::{Digest, Md5};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
struct State {
    items: HashMap<String, CloudItem>,
    content: HashMap<String, Vec<u8>>,
    /// Every change in order (id); a token is an index into this.
    log: Vec<String>,
    removed: Vec<bool>,
    next_id: u64,
    clock: f64,
    /// Calls made, for tests ("upload a", "trash b"...).
    calls: Vec<String>,
}

pub struct FakeCloud {
    s: Mutex<State>,
}

pub const ROOT: &str = "root-id";

impl Default for FakeCloud {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCloud {
    pub fn new() -> Self {
        let mut s = State { clock: 1_700_000_000.0, ..Default::default() };
        s.items.insert(ROOT.into(), CloudItem { id: ROOT.into(), name: "My Drive".into(), mime: FOLDER_MIME.into(), can_edit: true, owned_by_me: true, ..Default::default() });
        FakeCloud { s: Mutex::new(s) }
    }

    fn touch(s: &mut State, id: &str) {
        s.clock += 1.0;
        let clock = s.clock;
        if let Some(it) = s.items.get_mut(id) {
            it.version += 1;
            it.mtime = clock;
        }
        s.log.push(id.to_string());
        s.removed.push(false);
    }

    fn new_id(s: &mut State) -> String {
        s.next_id += 1;
        format!("f{}", s.next_id)
    }

    // --- test helpers (someone else changing the drive) ---

    pub fn mkdir(&self, parent: &str, name: &str) -> String {
        self.create_folder(parent, name).unwrap().id
    }

    pub fn put(&self, parent: &str, name: &str, data: &[u8]) -> String {
        let mut s = self.s.lock();
        let id = Self::new_id(&mut s);
        let clock = s.clock;
        s.items.insert(id.clone(), CloudItem { id: id.clone(), name: name.into(), mime: "application/octet-stream".into(), parents: vec![parent.into()], hash: Some(hex_md5(data)), size: Some(data.len() as u64), created: clock, can_edit: true, owned_by_me: true, ..Default::default() });
        s.content.insert(id.clone(), data.to_vec());
        Self::touch(&mut s, &id);
        id
    }

    pub fn put_gdoc(&self, parent: &str, name: &str, mime: &str) -> String {
        let mut s = self.s.lock();
        let id = Self::new_id(&mut s);
        let clock = s.clock;
        s.items.insert(id.clone(), CloudItem { id: id.clone(), name: name.into(), mime: mime.into(), parents: vec![parent.into()], created: clock, can_edit: true, owned_by_me: true, web_link: Some(format!("https://docs.google.com/document/d/{id}/edit")), ..Default::default() });
        s.content.insert(id.clone(), format!("exported {name}").into_bytes());
        Self::touch(&mut s, &id);
        id
    }

    pub fn edit(&self, id: &str, data: &[u8]) {
        let mut s = self.s.lock();
        let it = s.items.get_mut(id).expect("no such item");
        it.hash = Some(hex_md5(data));
        it.size = Some(data.len() as u64);
        s.content.insert(id.into(), data.to_vec());
        Self::touch(&mut s, id);
    }

    pub fn rename(&self, id: &str, name: &str) {
        self.update_meta(id, Some(name), None).unwrap();
    }

    pub fn mv(&self, id: &str, to: &str) {
        let from = self.s.lock().items[id].parents[0].clone();
        self.update_meta(id, None, Some((&from, to))).unwrap();
    }

    pub fn remove_forever(&self, id: &str) {
        let mut s = self.s.lock();
        s.items.remove(id);
        s.content.remove(id);
        s.log.push(id.into());
        s.removed.push(true);
    }

    pub fn content(&self, id: &str) -> Option<Vec<u8>> {
        self.s.lock().content.get(id).cloned()
    }

    pub fn item(&self, id: &str) -> Option<CloudItem> {
        self.s.lock().items.get(id).cloned()
    }

    /// The live (not trashed) child of `parent` called `name`.
    pub fn child(&self, parent: &str, name: &str) -> Option<CloudItem> {
        self.s.lock().items.values().find(|i| !i.trashed && i.name == name && i.parents.iter().any(|p| p == parent)).cloned()
    }

    pub fn children(&self, parent: &str) -> Vec<CloudItem> {
        let mut v: Vec<_> = self.s.lock().items.values().filter(|i| !i.trashed && i.parents.iter().any(|p| p == parent)).cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Every file's current content, trashed ones included (tests).
    pub fn all_contents(&self) -> Vec<Vec<u8>> {
        self.s.lock().content.values().cloned().collect()
    }

    pub fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.s.lock().calls)
    }
}

pub fn hex_md5(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

impl Cloud for FakeCloud {
    fn about(&self) -> Result<About> {
        Ok(About { email: "test@example.com".into(), name: "Test".into(), root_id: ROOT.into(), ..Default::default() })
    }

    fn shared_drives(&self) -> Result<Vec<SharedDrive>> {
        Ok(vec![])
    }

    fn start_token(&self, _drive: Option<&str>) -> Result<String> {
        Ok(self.s.lock().log.len().to_string())
    }

    fn changes(&self, _drive: Option<&str>, token: &str) -> Result<ChangePage> {
        let s = self.s.lock();
        let from: usize = token.parse().map_err(|_| anyhow!("bad token"))?;
        // Like Drive: one entry per item, its latest state.
        let mut seen = std::collections::HashSet::new();
        let mut changes = Vec::new();
        for i in (from..s.log.len()).rev() {
            let id = &s.log[i];
            if !seen.insert(id.clone()) {
                continue;
            }
            match s.items.get(id) {
                Some(it) if !s.removed[i] => changes.push(Change::Item(it.clone())),
                _ => changes.push(Change::Removed(id.clone())),
            }
        }
        changes.reverse();
        Ok(ChangePage { changes, next: None, new_start: Some(s.log.len().to_string()) })
    }

    fn list_all(&self, _drive: Option<&str>) -> Result<Vec<CloudItem>> {
        Ok(self.s.lock().items.values().filter(|i| !i.trashed).cloned().collect())
    }

    fn get(&self, id: &str) -> Result<Option<CloudItem>> {
        Ok(self.s.lock().items.get(id).cloned())
    }

    fn create_folder(&self, parent: &str, name: &str) -> Result<CloudItem> {
        let mut s = self.s.lock();
        if !s.items.contains_key(parent) {
            bail!("parent {parent} not found");
        }
        let id = Self::new_id(&mut s);
        s.calls.push(format!("mkdir {name}"));
        let clock = s.clock;
        s.items.insert(id.clone(), CloudItem { id: id.clone(), name: name.into(), mime: FOLDER_MIME.into(), parents: vec![parent.into()], created: clock, can_edit: true, owned_by_me: true, ..Default::default() });
        Self::touch(&mut s, &id);
        Ok(s.items[&id].clone())
    }

    fn upload(&self, parent: &str, name: &str, existing: Option<&str>, src: &Path, mime: Option<&str>, mtime: f64, progress: Progress) -> Result<CloudItem> {
        let data = std::fs::read(src)?;
        progress(data.len() as u64);
        let mut s = self.s.lock();
        let id = match existing {
            Some(id) => {
                s.calls.push(format!("update {name}"));
                id.to_string()
            }
            None => {
                s.calls.push(format!("upload {name}"));
                let id = Self::new_id(&mut s);
                let clock = s.clock;
                s.items.insert(id.clone(), CloudItem { id: id.clone(), name: name.into(), mime: mime.unwrap_or("application/octet-stream").into(), parents: vec![parent.into()], created: clock, can_edit: true, owned_by_me: true, ..Default::default() });
                id
            }
        };
        let it = s.items.get_mut(&id).ok_or_else(|| anyhow!("no such file {id}"))?;
        if it.is_google_format() {
            // A converted Google file updated from an Office file.
            s.content.insert(id.clone(), data);
        } else {
            it.hash = Some(hex_md5(&data));
            it.size = Some(data.len() as u64);
            s.content.insert(id.clone(), data);
        }
        Self::touch(&mut s, &id);
        let it = s.items.get_mut(&id).unwrap();
        it.mtime = mtime;
        Ok(it.clone())
    }

    fn download(&self, id: &str, dest: &Path, progress: Progress) -> Result<()> {
        let data = {
            let mut s = self.s.lock();
            let name = s.items.get(id).map(|i| i.name.clone()).unwrap_or_default();
            s.calls.push(format!("download {name}"));
            s.content.get(id).cloned().ok_or_else(|| anyhow!("no content for {id}"))?
        };
        progress(data.len() as u64);
        std::fs::write(dest, data)?;
        Ok(())
    }

    fn export(&self, id: &str, mime: &str, dest: &Path) -> Result<()> {
        let data = {
            let mut s = self.s.lock();
            let name = s.items.get(id).map(|i| i.name.clone()).unwrap_or_default();
            s.calls.push(format!("export {name} {mime}"));
            s.content.get(id).cloned().unwrap_or_default()
        };
        std::fs::write(dest, data)?;
        Ok(())
    }

    fn update_meta(&self, id: &str, name: Option<&str>, move_from_to: Option<(&str, &str)>) -> Result<CloudItem> {
        let mut s = self.s.lock();
        let old = s.items.get(id).map(|i| i.name.clone()).ok_or_else(|| anyhow!("no such item {id}"))?;
        if let Some(n) = name {
            s.calls.push(format!("rename {old} {n}"));
        }
        if let Some((from, to)) = move_from_to {
            s.calls.push(format!("move {old}"));
            if !s.items.contains_key(to) {
                bail!("parent {to} not found");
            }
            // As Drive: an item has one parent; adding without removing the
            // current one is refused (cannotAddParent).
            if !s.items[id].parents.iter().any(|p| p == from) {
                bail!("Increasing the number of parents is not allowed");
            }
            // ...and a folder can't go inside itself.
            let mut cur = Some(to.to_string());
            while let Some(c) = cur {
                if c == id {
                    bail!("A folder can't be moved into itself");
                }
                cur = s.items.get(&c).and_then(|i| i.parents.first().cloned());
            }
        }
        let it = s.items.get_mut(id).unwrap();
        if let Some(n) = name {
            it.name = n.into();
        }
        if let Some((from, to)) = move_from_to {
            it.parents.retain(|p| p != from);
            it.parents.push(to.into());
        }
        Self::touch(&mut s, id);
        Ok(s.items[id].clone())
    }

    fn trash(&self, id: &str) -> Result<()> {
        let mut s = self.s.lock();
        let name = s.items.get(id).map(|i| i.name.clone()).ok_or_else(|| anyhow!("no such item {id}"))?;
        s.calls.push(format!("trash {name}"));
        // Trashing a folder trashes what's inside (Drive reports each).
        let mut stack = vec![id.to_string()];
        while let Some(i) = stack.pop() {
            let kids: Vec<String> = s.items.values().filter(|c| c.parents.contains(&i)).map(|c| c.id.clone()).collect();
            stack.extend(kids);
            if let Some(it) = s.items.get_mut(&i) {
                it.trashed = true;
            }
            Self::touch(&mut s, &i);
        }
        Ok(())
    }

    fn copy(&self, id: &str, parent: &str, name: &str) -> Result<CloudItem> {
        let mut s = self.s.lock();
        let src = s.items.get(id).cloned().ok_or_else(|| anyhow!("no such item {id}"))?;
        s.calls.push(format!("copy {} {name}", src.name));
        let nid = Self::new_id(&mut s);
        let content = s.content.get(id).cloned();
        let clock = s.clock;
        s.items.insert(nid.clone(), CloudItem { id: nid.clone(), name: name.into(), parents: vec![parent.into()], version: 0, created: clock, ..src });
        if let Some(c) = content {
            s.content.insert(nid.clone(), c);
        }
        Self::touch(&mut s, &nid);
        Ok(s.items[&nid].clone())
    }

    fn keep_revision(&self, id: &str) -> Result<()> {
        let mut s = self.s.lock();
        let name = s.items.get(id).map(|i| i.name.clone()).unwrap_or_default();
        s.calls.push(format!("pin {name}"));
        Ok(())
    }

    fn set_starred(&self, id: &str, starred: bool) -> Result<()> {
        let mut s = self.s.lock();
        if let Some(it) = s.items.get_mut(id) {
            it.starred = starred;
        }
        Self::touch(&mut s, id);
        Ok(())
    }
}
