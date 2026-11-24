//! Settings (app-wide and per account) and sync roots, as JSON files in
//! ~/.config/canopy. Edited through the window or `canopy config`; nobody
//! has to touch the files.

use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Changes go both ways.
    #[default]
    TwoWay,
    /// The computer mirrors the cloud: local edits are replaced, local
    /// deletions downloaded again.
    CloudToLocal,
    /// The cloud mirrors the computer (backup, "sync any folder").
    LocalToCloud,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportOption {
    /// Small link files that open the browser (Insync's default).
    #[default]
    Link,
    MsOffice,
    OpenDocument,
}

/// What to do in each kind of conflict (Insync's four cases).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Keep both: the local version is renamed "(conflict ...)" and both
    /// end up on both sides. Never loses anything.
    #[default]
    KeepBoth,
    /// Leave it and list it in Conflicts until the user picks.
    Ask,
    KeepLocal,
    KeepCloud,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ConflictPolicy {
    pub edit_edit: Resolution,
    /// Edited here, deleted in the cloud. KeepLocal uploads it again.
    pub local_edit_cloud_delete: Resolution,
    /// Deleted here, edited in the cloud. KeepCloud downloads it again.
    pub local_delete_cloud_edit: Resolution,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    #[default]
    MyDrive,
    SharedWithMe,
    SharedDrive,
    /// Any local folder synced to a cloud folder ("@" / Local Selective Sync).
    Custom,
}

/// A cloud folder kept in step with a local folder.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Root {
    /// Stable id (used for its database rows).
    pub id: String,
    pub kind: RootKind,
    pub local: PathBuf,
    /// The cloud folder ("" for My Drive: the account's root).
    pub cloud_id: String,
    /// Shared drives: the drive.
    pub drive_id: Option<String>,
    pub name: String,
    pub mode: Mode,
    /// Selective sync: "path/inside/root" -> synced or not. The nearest
    /// rule above a path wins; "" is the root itself.
    pub selection: BTreeMap<String, bool>,
    pub paused: bool,
}

impl Root {
    /// Whether a path (relative, '/'-separated) is chosen to sync.
    pub fn selected(&self, rel: &str) -> bool {
        let mut p = rel;
        loop {
            if let Some(v) = self.selection.get(p) {
                return *v;
            }
            if p.is_empty() {
                return true;
            }
            p = match p.rfind('/') {
                Some(i) => &p[..i],
                None => "",
            };
        }
    }

    /// Whether anything at or below `rel` is selected (folders on the way
    /// to a selected item are created).
    pub fn selected_below(&self, rel: &str) -> bool {
        if self.selected(rel) {
            return true;
        }
        let prefix = format!("{rel}/");
        self.selection.iter().any(|(k, v)| *v && (rel.is_empty() || k.starts_with(&prefix)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Account {
    /// Google's permission id for the user (stable).
    pub id: String,
    pub email: String,
    pub name: String,
    pub photo: Option<String>,
    /// The folder holding this account's base folders.
    pub base: PathBuf,
    pub roots: Vec<Root>,
    pub export: ExportOption,
    pub conflicts: ConflictPolicy,
    /// gitignore-style lines.
    pub ignore_rules: Vec<String>,
    pub paused: bool,
    pub sync_shared_with_me: bool,
    pub sync_shared_drives: bool,
    /// Signed in through hearth's Internet Accounts (its account id):
    /// tokens come from there, canopy keeps no sign-in of its own.
    pub hearth_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Proxy {
    /// "system", "none" or "manual".
    pub mode: String,
    /// Manual: "socks5://host:port", "http://user:pass@host:port"...
    pub url: String,
}

impl Default for Proxy {
    fn default() -> Self {
        Proxy { mode: "system".into(), url: String::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub paused: bool,
    pub notifications: bool,
    pub symlink_warnings: bool,
    /// Deletions made while canopy wasn't running: delete in the cloud
    /// (true) or only unsync (false, for dual boot).
    pub sync_offline_deletions: bool,
    /// KiB/s, 0 = no limit.
    pub upload_limit: u64,
    pub download_limit: u64,
    pub transfers: usize,
    pub proxy: Proxy,
    /// Google Cloud OAuth client ("Desktop app" type).
    pub client_id: String,
    pub client_secret: String,
    /// Seconds between changes-feed polls while active.
    pub poll_seconds: u64,
    /// Window: "auto", "dark", "light".
    pub appearance: String,
    /// Start syncing at login (an XDG autostart entry running `canopy start`).
    pub start_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            paused: false,
            notifications: true,
            symlink_warnings: true,
            sync_offline_deletions: true,
            upload_limit: 0,
            download_limit: 0,
            transfers: 4,
            proxy: Proxy::default(),
            client_id: String::new(),
            client_secret: String::new(),
            poll_seconds: 20,
            appearance: "auto".into(),
            start_at_login: true,
        }
    }
}

fn load<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    let p = paths::config_dir().join(name);
    match std::fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            eprintln!("canopy: {} is damaged ({e}); using defaults", p.display());
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Write atomically (a crash never leaves half a file).
pub fn save_json<T: Serialize>(path: &std::path::Path, v: &T) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Settings = load("settings.json");
        // Built-in client from the environment (packagers, tests).
        if s.client_id.is_empty() {
            if let Ok(id) = std::env::var("CANOPY_CLIENT_ID") {
                s.client_id = id;
                s.client_secret = std::env::var("CANOPY_CLIENT_SECRET").unwrap_or_default();
            }
        }
        s
    }
    pub fn save(&self) -> Result<()> {
        save_json(&paths::config_dir().join("settings.json"), self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Accounts {
    pub accounts: Vec<Account>,
}

impl Accounts {
    pub fn load() -> Self {
        load("accounts.json")
    }
    pub fn save(&self) -> Result<()> {
        save_json(&paths::config_dir().join("accounts.json"), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_nearest_rule_wins() {
        let mut r = Root::default();
        assert!(r.selected("a/b"));
        r.selection.insert("".into(), false);
        r.selection.insert("Photos".into(), true);
        r.selection.insert("Photos/Raw".into(), false);
        assert!(!r.selected("Docs/x"));
        assert!(r.selected("Photos"));
        assert!(r.selected("Photos/2024/a.jpg"));
        assert!(!r.selected("Photos/Raw/a.cr2"));
        assert!(r.selected_below(""));
        assert!(!r.selected_below("Docs"));
        r.selection.insert("Docs/Keep".into(), true);
        assert!(r.selected_below("Docs"));
    }
}
