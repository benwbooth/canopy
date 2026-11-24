//! One SQLite database per account (like Insync's data/ and live/ files):
//! - `items`, `tokens`: the cloud mirror and changes-feed tokens.
//! - `base`: per root, what each path was when both sides last agreed.
//! - `activity`, `issues`: the feeds the window shows.
//! - `uploads`: resumable upload sessions (valid a week).

use crate::cloud::CloudItem;
use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

pub struct Db {
    c: Mutex<Connection>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Base {
    pub path: String,
    /// 'd' folder, 'f' file, 'g' Google-format file (link or export).
    pub kind: char,
    pub cl_id: String,
    pub ino: u64,
    pub l_mtime: f64,
    pub l_size: u64,
    pub c_md5: Option<String>,
    pub c_version: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Activity {
    pub id: i64,
    pub at: f64,
    pub root: String,
    /// upload, download, add, edit, move, rename, delete, local_delete,
    /// conflict, ignore, unsync, share.
    pub kind: String,
    pub path: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Issue {
    pub id: i64,
    pub at: f64,
    pub root: String,
    pub path: String,
    /// error, conflict, attention.
    pub kind: String,
    /// For conflicts: edit_edit, local_edit_cloud_delete,
    /// local_delete_cloud_edit. For errors: retryable or not.
    pub sub: String,
    pub message: String,
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let c = Connection::open(path)?;
        Self::init(c)
    }

    pub fn memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(c: Connection) -> Result<Db> {
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS items (id TEXT PRIMARY KEY, parent TEXT, name TEXT NOT NULL, trashed INTEGER NOT NULL, drive TEXT, json TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS items_parent ON items(parent);
             CREATE INDEX IF NOT EXISTS items_name ON items(name);
             CREATE TABLE IF NOT EXISTS tokens (drive TEXT PRIMARY KEY, token TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS base (root TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL, cl_id TEXT NOT NULL, ino INTEGER NOT NULL, l_mtime REAL NOT NULL, l_size INTEGER NOT NULL, c_md5 TEXT, c_version INTEGER NOT NULL, PRIMARY KEY(root, path));
             CREATE INDEX IF NOT EXISTS base_cl ON base(cl_id);
             CREATE TABLE IF NOT EXISTS activity (id INTEGER PRIMARY KEY AUTOINCREMENT, at REAL NOT NULL, root TEXT NOT NULL, kind TEXT NOT NULL, path TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '');
             CREATE TABLE IF NOT EXISTS issues (id INTEGER PRIMARY KEY AUTOINCREMENT, at REAL NOT NULL, root TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL, sub TEXT NOT NULL DEFAULT '', message TEXT NOT NULL, UNIQUE(root, path, kind));
             CREATE TABLE IF NOT EXISTS uploads (root TEXT NOT NULL, path TEXT NOT NULL, session TEXT NOT NULL, size INTEGER NOT NULL, mtime REAL NOT NULL, started REAL NOT NULL, PRIMARY KEY(root, path));
             CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(Db { c: Mutex::new(c) })
    }

    // --- mirror ---

    pub fn put_items(&self, items: &[CloudItem]) -> Result<()> {
        let mut c = self.c.lock();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached("INSERT OR REPLACE INTO items (id, parent, name, trashed, drive, json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for it in items {
                st.execute(params![it.id, it.parents.first(), it.name, it.trashed, it.drive_id, serde_json::to_string(it)?])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_items(&self, ids: &[String]) -> Result<()> {
        let mut c = self.c.lock();
        let tx = c.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM items WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn row_item(json: String) -> Option<CloudItem> {
        serde_json::from_str(&json).ok()
    }

    pub fn item(&self, id: &str) -> Result<Option<CloudItem>> {
        let c = self.c.lock();
        let j: Option<String> = c.query_row("SELECT json FROM items WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(j.and_then(Self::row_item))
    }

    /// Live (not trashed) children of a folder.
    pub fn children(&self, parent: &str) -> Result<Vec<CloudItem>> {
        let c = self.c.lock();
        let mut st = c.prepare_cached("SELECT json FROM items WHERE parent = ?1 AND trashed = 0")?;
        let v = st.query_map([parent], |r| r.get::<_, String>(0))?.filter_map(|j| j.ok().and_then(Self::row_item)).collect();
        Ok(v)
    }

    /// Items matching a filter over the whole mirror (search, starred,
    /// shared with me). `sql` is a WHERE clause over `items`.
    pub fn items_where(&self, sql: &str, arg: &str, limit: usize) -> Result<Vec<CloudItem>> {
        let c = self.c.lock();
        let mut st = c.prepare(&format!("SELECT json FROM items WHERE trashed = 0 AND ({sql}) LIMIT {limit}"))?;
        let v = st.query_map([arg], |r| r.get::<_, String>(0))?.filter_map(|j| j.ok().and_then(Self::row_item)).collect();
        Ok(v)
    }

    pub fn item_count(&self) -> Result<i64> {
        Ok(self.c.lock().query_row("SELECT count(*) FROM items", [], |r| r.get(0))?)
    }

    pub fn clear_items(&self) -> Result<()> {
        self.c.lock().execute_batch("DELETE FROM items; DELETE FROM tokens;")?;
        Ok(())
    }

    pub fn token(&self, drive: &str) -> Result<Option<String>> {
        Ok(self.c.lock().query_row("SELECT token FROM tokens WHERE drive = ?1", [drive], |r| r.get(0)).optional()?)
    }

    pub fn set_token(&self, drive: &str, token: &str) -> Result<()> {
        self.c.lock().execute("INSERT OR REPLACE INTO tokens (drive, token) VALUES (?1, ?2)", [drive, token])?;
        Ok(())
    }

    // --- base ---

    pub fn base_all(&self, root: &str) -> Result<Vec<Base>> {
        let c = self.c.lock();
        let mut st = c.prepare_cached("SELECT path, kind, cl_id, ino, l_mtime, l_size, c_md5, c_version FROM base WHERE root = ?1")?;
        let v = st
            .query_map([root], |r| {
                Ok(Base {
                    path: r.get(0)?,
                    kind: r.get::<_, String>(1)?.chars().next().unwrap_or('f'),
                    cl_id: r.get(2)?,
                    ino: r.get::<_, i64>(3)? as u64,
                    l_mtime: r.get(4)?,
                    l_size: r.get::<_, i64>(5)? as u64,
                    c_md5: r.get(6)?,
                    c_version: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn base_get(&self, root: &str, path: &str) -> Result<Option<Base>> {
        let c = self.c.lock();
        let r = c
            .query_row("SELECT path, kind, cl_id, ino, l_mtime, l_size, c_md5, c_version FROM base WHERE root = ?1 AND path = ?2", [root, path], |r| {
                Ok(Base {
                    path: r.get(0)?,
                    kind: r.get::<_, String>(1)?.chars().next().unwrap_or('f'),
                    cl_id: r.get(2)?,
                    ino: r.get::<_, i64>(3)? as u64,
                    l_mtime: r.get(4)?,
                    l_size: r.get::<_, i64>(5)? as u64,
                    c_md5: r.get(6)?,
                    c_version: r.get(7)?,
                })
            })
            .optional()?;
        Ok(r)
    }

    pub fn base_put(&self, root: &str, b: &Base) -> Result<()> {
        self.c.lock().execute(
            "INSERT OR REPLACE INTO base (root, path, kind, cl_id, ino, l_mtime, l_size, c_md5, c_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![root, b.path, b.kind.to_string(), b.cl_id, b.ino as i64, b.l_mtime, b.l_size as i64, b.c_md5, b.c_version],
        )?;
        Ok(())
    }

    /// Forget a path and everything below it.
    pub fn base_remove_tree(&self, root: &str, path: &str) -> Result<()> {
        self.c.lock().execute("DELETE FROM base WHERE root = ?1 AND (path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')", params![root, path])?;
        Ok(())
    }

    /// A path and everything below it moved to `to`.
    pub fn base_move_tree(&self, root: &str, from: &str, to: &str) -> Result<()> {
        let mut c = self.c.lock();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM base WHERE root = ?1 AND (path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')", params![root, to])?;
        tx.execute(
            "UPDATE base SET path = ?3 || substr(path, length(?2) + 1) WHERE root = ?1 AND (path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')",
            params![root, from, to],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn base_clear_root(&self, root: &str) -> Result<()> {
        self.c.lock().execute("DELETE FROM base WHERE root = ?1", [root])?;
        Ok(())
    }

    // --- feeds ---

    pub fn log(&self, root: &str, kind: &str, path: &str, detail: &str) -> Result<i64> {
        let c = self.c.lock();
        c.execute("INSERT INTO activity (at, root, kind, path, detail) VALUES (?1, ?2, ?3, ?4, ?5)", params![now(), root, kind, path, detail])?;
        let id = c.last_insert_rowid();
        // Keep the feed bounded (Insync limits it too).
        if id % 500 == 0 {
            c.execute("DELETE FROM activity WHERE id < ?1", [id - 20000])?;
        }
        Ok(id)
    }

    pub fn activity(&self, before: i64, limit: usize) -> Result<Vec<Activity>> {
        let c = self.c.lock();
        let mut st = c.prepare("SELECT id, at, root, kind, path, detail FROM activity WHERE id < ?1 ORDER BY id DESC LIMIT ?2")?;
        let v = st
            .query_map(params![if before <= 0 { i64::MAX } else { before }, limit as i64], |r| {
                Ok(Activity { id: r.get(0)?, at: r.get(1)?, root: r.get(2)?, kind: r.get(3)?, path: r.get(4)?, detail: r.get(5)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    pub fn issue(&self, root: &str, path: &str, kind: &str, sub: &str, message: &str) -> Result<()> {
        self.c.lock().execute(
            "INSERT INTO issues (at, root, path, kind, sub, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(root, path, kind) DO UPDATE SET at = excluded.at, sub = excluded.sub, message = excluded.message",
            params![now(), root, path, kind, sub, message],
        )?;
        Ok(())
    }

    pub fn clear_issue(&self, root: &str, path: &str, kind: &str) -> Result<()> {
        self.c.lock().execute("DELETE FROM issues WHERE root = ?1 AND path = ?2 AND kind = ?3", params![root, path, kind])?;
        Ok(())
    }

    pub fn clear_issue_id(&self, id: i64) -> Result<()> {
        self.c.lock().execute("DELETE FROM issues WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn issues(&self) -> Result<Vec<Issue>> {
        let c = self.c.lock();
        let mut st = c.prepare("SELECT id, at, root, path, kind, sub, message FROM issues ORDER BY at DESC")?;
        let v = st
            .query_map([], |r| Ok(Issue { id: r.get(0)?, at: r.get(1)?, root: r.get(2)?, path: r.get(3)?, kind: r.get(4)?, sub: r.get(5)?, message: r.get(6)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    }

    // --- resumable uploads ---

    pub fn upload_session(&self, root: &str, path: &str, size: u64, mtime: f64) -> Result<Option<String>> {
        let c = self.c.lock();
        let r: Option<(String, i64, f64, f64)> =
            c.query_row("SELECT session, size, mtime, started FROM uploads WHERE root = ?1 AND path = ?2", [root, path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        // Sessions last a week; only for the same file content.
        Ok(r.filter(|(_, s, m, st)| *s as u64 == size && *m == mtime && now() - st < 6.0 * 86400.0).map(|r| r.0))
    }

    pub fn set_upload_session(&self, root: &str, path: &str, session: Option<&str>, size: u64, mtime: f64) -> Result<()> {
        let c = self.c.lock();
        match session {
            Some(s) => c.execute("INSERT OR REPLACE INTO uploads (root, path, session, size, mtime, started) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![root, path, s, size as i64, mtime, now()])?,
            None => c.execute("DELETE FROM uploads WHERE root = ?1 AND path = ?2", [root, path])?,
        };
        Ok(())
    }

    pub fn kv(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c.lock().query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.c.lock().execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", [key, value])?;
        Ok(())
    }
}
