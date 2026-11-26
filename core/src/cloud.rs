//! What the sync engine needs from a cloud drive. `drive::Drive` talks to
//! Google; `rclone::RcloneCloud` to any of rclone's services (Dropbox,
//! OneDrive, S3, SFTP...); `fake::FakeCloud` keeps everything in memory for
//! tests.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
pub const SHORTCUT_MIME: &str = "application/vnd.google-apps.shortcut";

/// One file or folder in the cloud (the fields canopy keeps).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CloudItem {
    pub id: String,
    pub name: String,
    pub mime: String,
    #[serde(default)]
    pub parents: Vec<String>,
    /// The content hash, of the kind the cloud keeps (`Cloud::hash_local`
    /// makes the same of a local file): MD5 for Google Drive, Dropbox's or
    /// OneDrive's own for those... None: files the cloud has no hash for
    /// (Google Docs; WebDAV, FTP), compared by `version` instead.
    /// (Stored as "md5", its name when Drive was the only cloud.)
    #[serde(default, rename = "md5")]
    pub hash: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Seconds since the epoch.
    #[serde(default)]
    pub mtime: f64,
    /// Goes up with every change, content or not.
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub trashed: bool,
    /// The shared drive holding it (None: the user's own corpus).
    #[serde(default)]
    pub drive_id: Option<String>,
    #[serde(default)]
    pub web_link: Option<String>,
    /// Shortcuts: the item they point at.
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub owned_by_me: bool,
    /// Seconds since the epoch (orders duplicates by age).
    #[serde(default)]
    pub created: f64,
}

impl CloudItem {
    pub fn is_folder(&self) -> bool {
        self.mime == FOLDER_MIME
    }
    /// A Google Docs, Sheets, Slides... file (no content to download, only
    /// exports).
    pub fn is_google_format(&self) -> bool {
        self.mime.starts_with("application/vnd.google-apps.") && !self.is_folder()
    }
}

/// One entry of the changes feed.
#[derive(Clone, Debug)]
pub enum Change {
    /// Created or changed (it may now be trashed or moved).
    Item(CloudItem),
    /// Gone for good, or no longer visible to the account.
    Removed(String),
}

#[derive(Clone, Debug)]
pub struct ChangePage {
    pub changes: Vec<Change>,
    /// Continue from here (more pages to read).
    pub next: Option<String>,
    /// Read to the end: start from this one next time.
    pub new_start: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct About {
    pub email: String,
    pub name: String,
    pub photo: Option<String>,
    /// The id of My Drive's root folder.
    pub root_id: String,
    pub quota_used: Option<u64>,
    pub quota_total: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedDrive {
    pub id: String,
    pub name: String,
}

/// Bytes moved so far (called from the transfer thread).
pub type Progress<'a> = &'a (dyn Fn(u64) + Sync);

/// The operations the engine uses. Every call blocks; the engine runs on
/// its own threads.
pub trait Cloud: Send + Sync {
    fn about(&self) -> Result<About>;
    fn shared_drives(&self) -> Result<Vec<SharedDrive>>;
    /// Where the changes feed starts now (drive None: the user's corpus).
    fn start_token(&self, drive: Option<&str>) -> Result<String>;
    fn changes(&self, drive: Option<&str>, token: &str) -> Result<ChangePage>;
    /// Every item in a drive that isn't trashed (the first full read).
    fn list_all(&self, drive: Option<&str>) -> Result<Vec<CloudItem>>;
    fn get(&self, id: &str) -> Result<Option<CloudItem>>;
    fn create_folder(&self, parent: &str, name: &str) -> Result<CloudItem>;
    /// Upload `src` as a new file in `parent`, or as new content of
    /// `existing`. `mtime` becomes the cloud's modifiedTime.
    fn upload(&self, parent: &str, name: &str, existing: Option<&str>, src: &Path, mime: Option<&str>, mtime: f64, progress: Progress) -> Result<CloudItem>;
    fn download(&self, id: &str, dest: &Path, progress: Progress) -> Result<()>;
    /// A Google-format file converted to `mime` (docx, xlsx, pdf...).
    fn export(&self, id: &str, mime: &str, dest: &Path) -> Result<()>;
    /// Rename and/or move (parents swapped from `from` to `to`).
    fn update_meta(&self, id: &str, name: Option<&str>, move_from_to: Option<(&str, &str)>) -> Result<CloudItem>;
    fn trash(&self, id: &str) -> Result<()>;
    fn copy(&self, id: &str, parent: &str, name: &str) -> Result<CloudItem>;
    fn set_starred(&self, id: &str, starred: bool) -> Result<()>;
    /// Keep the current version of a file forever in Drive's version
    /// history (before it's replaced by a version Drive never had).
    fn keep_revision(&self, _id: &str) -> Result<()> {
        Ok(())
    }

    // --- what kind of cloud this is (defaults: Google Drive) ---

    /// What the cloud is called in messages ("Google Drive", "Dropbox").
    fn label(&self) -> String {
        "Google Drive".into()
    }
    /// Google's own features: Docs/Sheets/Slides (link files, exports),
    /// shared drives, sharing, stars.
    fn is_google(&self) -> bool {
        true
    }
    /// A local file's content hash, in the form `CloudItem::hash` has.
    fn hash_local(&self, path: &Path) -> Result<String> {
        crate::engine::md5_file(path)
    }
    /// Names that differ only in case are the same file there (Dropbox,
    /// OneDrive): two such local files can't both be synced.
    fn case_insensitive(&self) -> bool {
        false
    }
    /// Where deleted items go, in messages ("Google Drive's trash").
    fn trash_label(&self) -> String {
        format!("{}'s trash", self.label())
    }
    /// The user asked to sync now: the next changes read looks again even
    /// if the cloud rate-limits its look-ups.
    fn wake(&self) {}
}
