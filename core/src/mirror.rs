//! Keeping the database's copy of the cloud's metadata current: a full
//! listing the first time, then the changes feed from the stored token.

use crate::cloud::{Change, Cloud};
use crate::db::Db;
use anyhow::Result;

/// What changed in the cloud since the last refresh (item ids).
#[derive(Default, Debug)]
pub struct Refresh {
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub full: bool,
}

impl Refresh {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && !self.full
    }
}

/// `drive`: None for the user's own corpus, or a shared drive's id.
pub fn refresh(cloud: &dyn Cloud, db: &Db, drive: Option<&str>) -> Result<Refresh> {
    let key = drive.unwrap_or("");
    let mut out = Refresh::default();
    let Some(mut token) = db.token(key)? else {
        // First time: note where the feed starts, then read everything, so
        // nothing changed during the listing is missed.
        let start = cloud.start_token(drive)?;
        let items = cloud.list_all(drive)?;
        // A full listing is the whole truth: drop anything noted before it
        // (it may have been trashed since, and listings leave trash out).
        if drive.is_none() {
            db.clear_items()?;
        }
        db.put_items(&items)?;
        db.set_token(key, &start)?;
        out.full = true;
        return Ok(out);
    };
    loop {
        let page = cloud.changes(drive, &token)?;
        let mut put = Vec::new();
        for c in page.changes {
            match c {
                Change::Item(it) => {
                    out.changed.push(it.id.clone());
                    put.push(it);
                }
                Change::Removed(id) => out.removed.push(id),
            }
        }
        db.put_items(&put)?;
        db.remove_items(&out.removed)?;
        if let Some(n) = page.new_start {
            db.set_token(key, &n)?;
            break;
        }
        match page.next {
            Some(n) => token = n,
            None => break,
        }
    }
    Ok(out)
}
