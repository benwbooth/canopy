//! canopy: Google Drive sync for Linux.
//!
//!   canopy                     the window
//!   canopy <command>           the command line (cli.rs; `canopy --help`)
//!   canopy --share FILE        the window's Share sheet for a file
//!   canopy --add-folder DIR    the window's "Sync this folder" sheet
//!   canopy --screenshot FILE   (tests) grab the window and quit;
//!                              CANOPY_TEST="page=activity;shot=x" runs steps

mod bridge;
mod cli;
mod controller;
mod desktop;
mod single;

use bridge::ffi;
use cxx_qt_lib::QString;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(cmd) = args.first() {
        if cli::COMMANDS.contains(&cmd.as_str()) {
            if let Err(e) = cli::run(cmd, args[1..].to_vec()) {
                eprintln!("canopy: {e:#}");
                std::process::exit(1);
            }
            return;
        }
    }
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).cloned().unwrap_or_default();
        let abs = || std::env::current_dir().unwrap_or_default().join(&next).to_string_lossy().into_owned();
        match args[i].as_str() {
            "--screenshot" => {
                std::env::set_var("CANOPY_SCREENSHOT", abs());
                i += 1;
            }
            "--share" => {
                std::env::set_var("CANOPY_OPEN_SHARE", abs());
                i += 1;
            }
            "--add-folder" => {
                std::env::set_var("CANOPY_OPEN_ADD", abs());
                i += 1;
            }
            "--help" | "-h" | "help" => {
                print!("{}", cli::USAGE);
                return;
            }
            "--version" | "version" => {
                println!("canopy {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("canopy: unknown option {other}\n\n{}", cli::USAGE);
                std::process::exit(2);
            }
        }
        i += 1;
    }
    // One window at a time: a running canopy raises its window instead (and
    // opens the sheet asked for here). Tests run their own window.
    if std::env::var_os("CANOPY_SCREENSHOT").is_none() && std::env::var_os("CANOPY_TEST").is_none() {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        let request = serde_json::json!({
            "share": env("CANOPY_OPEN_SHARE"),
            "add": env("CANOPY_OPEN_ADD"),
            "token": env("XDG_ACTIVATION_TOKEN"),
        });
        if !single::claim_or_forward(&request) {
            return;
        }
    }
    // hearth's light/dark mode and accent, so canopy matches the shell.
    let hearth = canopy_core::paths::config_home().join("hearth/settings.json");
    if let Ok(t) = std::fs::read_to_string(hearth) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
            std::env::set_var("CANOPY_HEARTH", v["appearance"].to_string());
        }
    }
    set_qt_env("QT_QUICK_CONTROLS_STYLE", "Basic");
    if let Some(p) = option_env!("CANOPY_QT_PLUGINS").filter(|p| std::path::Path::new(p).is_dir()) {
        let old = std::env::var("QT_PLUGIN_PATH").unwrap_or_default();
        set_qt_env("QT_PLUGIN_PATH", if old.is_empty() { p.to_string() } else { format!("{p}:{old}") });
    }
    ffi::canopy_create_app();
    let import = option_env!("CANOPY_QT_QML").filter(|p| std::path::Path::new(p).exists()).unwrap_or("");
    let code = ffi::canopy_run(&QString::from("qrc:/qt/qml/Canopy/qml/Main.qml"), &QString::from(import));
    std::process::exit(code);
}

/// Sets an environment variable Qt reads while starting, noting what it was
/// (as `__APP_ORIG_<name>`: empty = unset, else "=value") so
/// `restore_launch_env` puts it back once the window is loaded: programs this
/// app starts get the desktop's own. (QT_QUICK_CONTROLS_STYLE=Basic leaking
/// from our apps made KDE apps opened from them draw plain light controls,
/// 2026-11-25.)
fn set_qt_env(name: &str, value: impl AsRef<std::ffi::OsStr>) {
    let note = format!("__APP_ORIG_{name}");
    if std::env::var_os(&note).is_none() {
        let was = std::env::var_os(name)
            .map(|v| {
                let mut s = std::ffi::OsString::from("=");
                s.push(v);
                s
            })
            .unwrap_or_default();
        #[allow(unused_unsafe)]
        unsafe { std::env::set_var(&note, was) };
    }
    #[allow(unused_unsafe)]
    unsafe { std::env::set_var(name, value) };
}

/// Puts back what `set_qt_env` (or the C++ start-up) changed, once the main
/// QML has loaded: Qt has read them all by then.
#[allow(dead_code)]
fn restore_launch_env() {
    for (k, v) in std::env::vars_os() {
        let Some(name) = k.to_str().and_then(|k| k.strip_prefix("__APP_ORIG_")).map(str::to_owned) else { continue };
        let v = v.to_string_lossy().into_owned();
        #[allow(unused_unsafe)]
        unsafe {
            match v.strip_prefix('=') {
                Some(old) => std::env::set_var(&name, old),
                None => std::env::remove_var(&name),
            }
            std::env::remove_var(&k);
        }
    }
}
