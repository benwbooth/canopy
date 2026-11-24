//! Sync scenarios against the in-memory Drive. Every test works in its own
//! temporary folder and trash (never the user's).

use canopy_core::cloud::Cloud;
use canopy_core::config::{ConflictPolicy, ExportOption, Mode, Resolution, Root};
use canopy_core::db::Db;
use canopy_core::engine::{Ctx, Engine, PassOptions, Report};
use canopy_core::fake::{FakeCloud, ROOT};
use canopy_core::ignore::Ignore;
use canopy_core::mirror;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

struct T {
    cloud: Arc<FakeCloud>,
    eng: Engine,
    _dir: tempfile::TempDir,
    local: PathBuf,
    trash: PathBuf,
}

fn setup_with(mode: Mode, export: ExportOption, conflicts: ConflictPolicy, rules: &[&str]) -> T {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("My Drive");
    let trash = dir.path().join("Trash");
    let cloud = Arc::new(FakeCloud::new());
    let ctx = Ctx {
        email: "test@example.com".into(),
        export,
        conflicts,
        ignore: Ignore::new(&rules.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        trash_dir: Some(trash.clone()),
        transfers: 3,
        sync_deletions: true,
        host: "testhost".into(),
    };
    let root = Root { id: "r1".into(), local: local.clone(), name: "My Drive".into(), mode, ..Default::default() };
    let eng = Engine {
        cloud: cloud.clone(),
        db: Arc::new(Db::memory().unwrap()),
        ctx,
        root,
        root_id: ROOT.into(),
        sink: Arc::new(|_| {}),
        cancel: Arc::new(AtomicBool::new(false)),
        busy: Default::default(),
    };
    T { cloud, eng, _dir: dir, local, trash }
}

fn setup() -> T {
    setup_with(Mode::TwoWay, ExportOption::Link, ConflictPolicy::default(), &[])
}

impl T {
    fn sync_opts(&self, o: &PassOptions) -> Report {
        mirror::refresh(self.cloud.as_ref(), &self.eng.db, None).unwrap();
        let r = self.eng.pass(o).unwrap();
        // Our own changes come back through the feed: a second pass must
        // have nothing to do.
        mirror::refresh(self.cloud.as_ref(), &self.eng.db, None).unwrap();
        r
    }
    fn sync(&self) -> Report {
        self.sync_opts(&PassOptions::default())
    }
    fn settled(&self) {
        self.cloud.calls();
        let r = self.sync();
        let calls = self.cloud.calls();
        assert!(calls.is_empty(), "second pass still did {calls:?} ({r:?})");
        assert_eq!(r.errors, 0, "{r:?} {:?}", self.eng.db.issues().unwrap());
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.local.join(rel)
    }
    fn write(&self, rel: &str, data: &str) {
        let p = self.p(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, data).unwrap();
        bump(&p);
    }
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.p(rel)).unwrap()
    }
    fn trashed(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.trash.join("files")).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }
}

/// Make an edit visible even within the same second.
fn bump(p: &Path) {
    let m = std::fs::metadata(p).unwrap();
    let t = filetime::FileTime::from_last_modification_time(&m);
    filetime::set_file_mtime(p, filetime::FileTime::from_unix_time(t.unix_seconds() + 7, t.nanoseconds())).unwrap();
}

#[test]
fn first_sync_downloads_tree_and_settles() {
    let t = setup();
    let docs = t.cloud.mkdir(ROOT, "Docs");
    t.cloud.put(&docs, "a.txt", b"hello");
    t.cloud.put(ROOT, "top.bin", b"\x00\x01");
    let r = t.sync();
    assert_eq!(r.downloads, 2);
    assert_eq!(t.read("Docs/a.txt"), "hello");
    assert!(t.p("top.bin").is_file());
    // mtime follows the cloud
    let it = t.cloud.child(&docs, "a.txt").unwrap();
    let m = std::fs::metadata(t.p("Docs/a.txt")).unwrap();
    assert_eq!(filetime::FileTime::from_last_modification_time(&m).unix_seconds(), it.mtime as i64);
    t.settled();
}

#[test]
fn local_new_files_and_folders_upload() {
    let t = setup();
    t.sync();
    t.write("Notes/2024/plan.md", "x");
    t.write("readme.txt", "y");
    let r = t.sync();
    assert_eq!(r.uploads, 2, "{r:?}");
    let notes = t.cloud.child(ROOT, "Notes").unwrap();
    let y = t.cloud.child(&notes.id, "2024").unwrap();
    let plan = t.cloud.child(&y.id, "plan.md").unwrap();
    assert_eq!(t.cloud.content(&plan.id).unwrap(), b"x");
    t.settled();
}

#[test]
fn edits_flow_both_ways() {
    let t = setup();
    let id = t.cloud.put(ROOT, "f.txt", b"one");
    t.sync();
    t.cloud.edit(&id, b"two");
    t.sync();
    assert_eq!(t.read("f.txt"), "two");
    t.write("f.txt", "three");
    let r = t.sync();
    assert_eq!(r.uploads, 1);
    assert_eq!(t.cloud.content(&id).unwrap(), b"three");
    t.settled();
}

#[test]
fn cloud_delete_goes_to_local_trash() {
    let t = setup();
    let d = t.cloud.mkdir(ROOT, "Old");
    t.cloud.put(&d, "x.txt", b"x");
    let f = t.cloud.put(ROOT, "gone.txt", b"g");
    t.sync();
    t.cloud.trash(&f).unwrap();
    t.cloud.trash(&d).unwrap();
    let r = t.sync();
    assert_eq!(r.deletes, 2, "{r:?}");
    assert!(!t.p("gone.txt").exists() && !t.p("Old").exists());
    assert_eq!(t.trashed(), vec!["Old", "gone.txt"]);
    t.settled();
}

#[test]
fn local_delete_goes_to_cloud_trash() {
    let t = setup();
    let d = t.cloud.mkdir(ROOT, "Old");
    let x = t.cloud.put(&d, "x.txt", b"x");
    let f = t.cloud.put(ROOT, "gone.txt", b"g");
    t.cloud.put(ROOT, "keep.txt", b"k");
    t.sync();
    std::fs::remove_file(t.p("gone.txt")).unwrap();
    std::fs::remove_dir_all(t.p("Old")).unwrap();
    t.cloud.calls();
    t.sync();
    let calls = t.cloud.calls();
    assert!(t.cloud.item(&f).unwrap().trashed);
    assert!(t.cloud.item(&d).unwrap().trashed && t.cloud.item(&x).unwrap().trashed);
    // The folder is trashed whole, not file by file.
    assert_eq!(calls.iter().filter(|c| c.starts_with("trash")).count(), 2, "{calls:?}");
    t.settled();
}

#[test]
fn moves_are_renames_not_transfers() {
    let t = setup();
    let a = t.cloud.mkdir(ROOT, "A");
    let b = t.cloud.mkdir(ROOT, "B");
    let f = t.cloud.put(&a, "f.txt", b"data");
    let g = t.cloud.put(ROOT, "g.txt", b"gg");
    t.sync();
    // Cloud side: move f into B, rename g.
    t.cloud.mv(&f, &b);
    t.cloud.rename(&g, "g2.txt");
    t.cloud.calls();
    let r = t.sync();
    assert_eq!((r.moves, r.downloads, r.uploads), (2, 0, 0), "{r:?}");
    assert_eq!(t.read("B/f.txt"), "data");
    assert_eq!(t.read("g2.txt"), "gg");
    // Local side: move the whole folder B into A, rename it.
    std::fs::rename(t.p("B"), t.p("A/Bee")).unwrap();
    t.cloud.calls();
    let r = t.sync();
    assert_eq!((r.moves, r.downloads, r.uploads, r.deletes), (1, 0, 0, 0), "{r:?}");
    let bee = t.cloud.item(&b).unwrap();
    assert_eq!((bee.name.as_str(), bee.parents[0].as_str()), ("Bee", a.as_str()));
    assert!(!t.cloud.item(&f).unwrap().trashed);
    t.settled();
}

#[test]
fn edit_edit_keeps_both() {
    let t = setup();
    let id = t.cloud.put(ROOT, "doc.txt", b"base");
    t.sync();
    t.cloud.edit(&id, b"cloud");
    t.write("doc.txt", "local");
    let r = t.sync();
    assert_eq!(r.conflicts, 1);
    assert_eq!(t.read("doc.txt"), "cloud");
    let copies: Vec<_> = std::fs::read_dir(&t.local).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains("(conflict testhost")).collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(t.read(&copies[0]), "local");
    assert!(t.cloud.child(ROOT, &copies[0]).is_some(), "the local version is uploaded too");
    t.settled();
}

#[test]
fn edit_edit_ask_waits_then_resolves() {
    let pol = ConflictPolicy { edit_edit: Resolution::Ask, ..Default::default() };
    let t = setup_with(Mode::TwoWay, ExportOption::Link, pol, &[]);
    let id = t.cloud.put(ROOT, "doc.txt", b"base");
    t.sync();
    t.cloud.edit(&id, b"cloud");
    t.write("doc.txt", "local");
    t.sync();
    assert_eq!(t.read("doc.txt"), "local");
    assert_eq!(t.eng.db.issues().unwrap().iter().filter(|i| i.kind == "conflict").count(), 1);
    let mut o = PassOptions::default();
    o.forced.insert("doc.txt".into(), Resolution::KeepLocal);
    t.sync_opts(&o);
    assert_eq!(t.cloud.content(&id).unwrap(), b"local");
    assert!(t.eng.db.issues().unwrap().is_empty());
    t.settled();
}

#[test]
fn local_edit_cloud_delete_reuploads() {
    let t = setup();
    let id = t.cloud.put(ROOT, "keep.txt", b"v1");
    t.sync();
    t.cloud.trash(&id).unwrap();
    t.write("keep.txt", "v2");
    t.sync();
    assert_eq!(t.read("keep.txt"), "v2");
    let new = t.cloud.child(ROOT, "keep.txt").unwrap();
    assert_ne!(new.id, id);
    t.settled();
}

#[test]
fn existing_identical_files_are_matched_not_transferred() {
    let t = setup();
    t.cloud.put(ROOT, "same.txt", b"same");
    let d = t.cloud.mkdir(ROOT, "D");
    t.cloud.put(&d, "x", b"1");
    t.write("same.txt", "same");
    t.write("D/x", "1");
    t.cloud.calls();
    let r = t.sync();
    assert_eq!((r.uploads, r.downloads, r.conflicts), (0, 0, 0), "{r:?} {:?}", t.cloud.calls());
    t.settled();
}

#[test]
fn google_docs_as_links_and_copies() {
    let t = setup();
    let doc = t.cloud.put_gdoc(ROOT, "Budget", "application/vnd.google-apps.spreadsheet");
    t.sync();
    let link = canopy_core::gdocs::read_link(&t.p("Budget.gdsheet")).unwrap();
    assert_eq!(link.file_id, doc);
    // A cloud edit to a Doc only rewrites the link (nothing to download).
    t.cloud.rename(&doc, "Budget 2026");
    t.sync();
    assert!(t.p("Budget 2026.gdsheet").exists() && !t.p("Budget.gdsheet").exists());
    // Copying the link file copies the Google file.
    std::fs::copy(t.p("Budget 2026.gdsheet"), t.p("Copy.gdsheet")).unwrap();
    t.sync();
    let copy = t.cloud.child(ROOT, "Copy").expect("a cloud copy");
    assert_eq!(copy.mime, "application/vnd.google-apps.spreadsheet");
    t.settled();
}

#[test]
fn google_docs_converted_and_edited_back() {
    let t = setup_with(Mode::TwoWay, ExportOption::MsOffice, ConflictPolicy::default(), &[]);
    let doc = t.cloud.put_gdoc(ROOT, "Letter", "application/vnd.google-apps.document");
    t.sync();
    assert_eq!(t.read("Letter.docx"), "exported Letter");
    t.write("Letter.docx", "edited docx");
    let r = t.sync();
    assert_eq!(r.uploads, 1, "{r:?}");
    let it = t.cloud.item(&doc).unwrap();
    assert_eq!(it.mime, "application/vnd.google-apps.document", "stays a Google Doc");
    assert_eq!(t.cloud.content(&doc).unwrap(), b"edited docx");
    t.settled();
}

#[test]
fn ignore_rules_and_builtins() {
    let t = setup_with(Mode::TwoWay, ExportOption::Link, ConflictPolicy::default(), &["*.iso", "build/"]);
    t.write("a.iso", "x");
    t.write("build/out.o", "x");
    t.write(".~lock.doc#", "x");
    t.write("ok.txt", "x");
    t.cloud.put(ROOT, "cloud.iso", b"x");
    let r = t.sync();
    assert_eq!(r.uploads, 1);
    assert!(t.cloud.child(ROOT, "a.iso").is_none() && t.cloud.child(ROOT, "build").is_none());
    assert!(!t.p("cloud.iso").exists());
    t.settled();
}

#[test]
fn selective_sync_and_unsync() {
    let mut t = setup();
    let keep = t.cloud.mkdir(ROOT, "Keep");
    let skip = t.cloud.mkdir(ROOT, "Skip");
    t.cloud.put(&keep, "k", b"k");
    t.cloud.put(&skip, "s", b"s");
    t.eng.root.selection.insert("".into(), false);
    t.eng.root.selection.insert("Keep".into(), true);
    t.sync();
    assert!(t.p("Keep/k").exists());
    assert!(!t.p("Skip").exists());
    // Unsync Keep: local copy to the trash, cloud untouched.
    t.eng.unsync("Keep").unwrap();
    t.eng.root.selection.insert("Keep".into(), false);
    t.cloud.calls();
    t.sync();
    assert!(!t.p("Keep").exists());
    assert!(t.cloud.calls().is_empty());
    assert!(!t.cloud.item(&keep).unwrap().trashed);
    assert_eq!(t.trashed(), vec!["Keep"]);
}

#[test]
fn duplicate_names_get_numbers() {
    let t = setup();
    t.cloud.put(ROOT, "dup.txt", b"first");
    t.cloud.put(ROOT, "dup.txt", b"second");
    t.cloud.put(ROOT, "a/b", b"slash");
    t.sync();
    assert_eq!(t.read("dup.txt"), "first");
    assert_eq!(t.read("dup (2).txt"), "second");
    assert_eq!(t.read("a\u{2215}b"), "slash");
    t.settled();
}

#[test]
fn missing_or_emptied_root_holds() {
    let t = setup();
    t.cloud.put(ROOT, "f", b"f");
    t.sync();
    std::fs::remove_dir_all(&t.local).unwrap();
    t.cloud.calls();
    let r = t.sync();
    assert!(r.held.is_some());
    assert!(t.cloud.calls().is_empty(), "nothing deleted in the cloud");
    std::fs::create_dir_all(&t.local).unwrap();
    let r = t.sync();
    assert!(r.held.is_some(), "an emptied root asks first");
    assert!(t.cloud.calls().is_empty());
}

#[test]
fn one_way_modes() {
    // Cloud -> local: local deletions come back, local additions stay local.
    let t = setup_with(Mode::CloudToLocal, ExportOption::Link, ConflictPolicy::default(), &[]);
    t.cloud.put(ROOT, "f", b"f");
    t.sync();
    std::fs::remove_file(t.p("f")).unwrap();
    t.write("mine", "m");
    t.sync();
    assert!(t.p("f").exists());
    assert!(t.cloud.child(ROOT, "mine").is_none());
    // Local -> cloud (backup): cloud deletions are uploaded again, cloud
    // additions aren't downloaded.
    let t = setup_with(Mode::LocalToCloud, ExportOption::Link, ConflictPolicy::default(), &[]);
    t.write("photo.jpg", "p");
    t.sync();
    let id = t.cloud.child(ROOT, "photo.jpg").unwrap().id;
    t.cloud.trash(&id).unwrap();
    t.cloud.put(ROOT, "theirs", b"t");
    t.sync();
    assert!(t.cloud.child(ROOT, "photo.jpg").is_some());
    assert!(!t.p("theirs").exists());
}

#[test]
fn stale_mirror_never_overwrites_cloud() {
    let t = setup();
    let id = t.cloud.put(ROOT, "doc.txt", b"base");
    t.sync();
    // Someone edits in Drive; canopy hasn't polled yet (stale mirror).
    t.cloud.edit(&id, b"cloud edit");
    t.write("doc.txt", "local edit");
    let r = t.eng.pass(&PassOptions::default()).unwrap();
    assert_eq!(r.uploads, 0, "must not upload over an unseen version");
    assert_eq!(t.cloud.content(&id).unwrap(), b"cloud edit");
    // After the next poll it's a conflict: both kept.
    t.sync();
    assert_eq!(t.read("doc.txt"), "cloud edit");
    let copy = std::fs::read_dir(&t.local).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.contains("conflict")).expect("conflict copy");
    assert_eq!(t.read(&copy), "local edit");
    t.settled();
}

#[test]
fn stale_mirror_never_trashes_changed_cloud_file() {
    let t = setup();
    let id = t.cloud.put(ROOT, "keep.txt", b"v1");
    t.cloud.put(ROOT, "other.txt", b"o"); // (an emptied folder is held instead)
    t.sync();
    t.cloud.edit(&id, b"v2 in the cloud");
    std::fs::remove_file(t.p("keep.txt")).unwrap();
    t.eng.pass(&PassOptions::default()).unwrap();
    assert!(!t.cloud.item(&id).unwrap().trashed, "a version canopy never saw is not deleted");
    t.sync();
    assert_eq!(t.read("keep.txt"), "v2 in the cloud");
    t.settled();
}

#[test]
fn hidden_local_edit_survives_cloud_change() {
    let t = setup();
    let id = t.cloud.put(ROOT, "notes.txt", b"aaaa");
    t.sync();
    // Same size, same mtime: invisible to size/mtime checks.
    let p = t.p("notes.txt");
    let m = std::fs::metadata(&p).unwrap();
    let mt = filetime::FileTime::from_last_modification_time(&m);
    std::fs::write(&p, "bbbb").unwrap();
    filetime::set_file_mtime(&p, mt).unwrap();
    t.cloud.edit(&id, b"cloud");
    t.sync();
    assert_eq!(t.read("notes.txt"), "cloud");
    let copy = std::fs::read_dir(&t.local).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.contains("conflict")).expect("the hidden edit is kept");
    assert_eq!(t.read(&copy), "bbbb");
}

#[test]
fn keep_local_pins_the_replaced_cloud_version() {
    let pol = ConflictPolicy { edit_edit: Resolution::KeepLocal, ..Default::default() };
    let t = setup_with(Mode::TwoWay, ExportOption::Link, pol, &[]);
    let id = t.cloud.put(ROOT, "doc.txt", b"base");
    t.sync();
    t.cloud.edit(&id, b"cloud");
    t.write("doc.txt", "local");
    t.cloud.calls();
    t.sync();
    let calls = t.cloud.calls();
    assert!(calls.iter().any(|c| c == "pin doc.txt"), "{calls:?}");
    assert_eq!(t.cloud.content(&id).unwrap(), b"local");
}
