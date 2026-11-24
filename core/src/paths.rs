//! Where canopy keeps things (XDG), overridable for tests with
//! CANOPY_CONFIG_DIR / CANOPY_DATA_DIR / CANOPY_RUNTIME_DIR.

use std::path::PathBuf;

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    env_dir("HOME").unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_home() -> PathBuf {
    env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
}

/// ~/.config/canopy: settings.json, accounts.json.
pub fn config_dir() -> PathBuf {
    env_dir("CANOPY_CONFIG_DIR").unwrap_or_else(|| config_home().join("canopy"))
}

/// ~/.local/share/canopy: one folder per account with its databases.
pub fn data_dir() -> PathBuf {
    env_dir("CANOPY_DATA_DIR").unwrap_or_else(|| env_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share")).join("canopy"))
}

/// $XDG_RUNTIME_DIR/canopy: the socket.
pub fn runtime_dir() -> PathBuf {
    env_dir("CANOPY_RUNTIME_DIR").unwrap_or_else(|| {
        env_dir("XDG_RUNTIME_DIR").unwrap_or_else(|| std::env::temp_dir().join(format!("canopy-{}", unsafe { libc::getuid() }))).join("canopy")
    })
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("canopy.sock")
}

pub fn account_dir(account: &str) -> PathBuf {
    data_dir().join("accounts").join(account)
}

/// The default folder holding every account's base folders.
pub fn default_base() -> PathBuf {
    home().join("Canopy")
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "this computer".into())
}
