//! canopyd's unix socket ($XDG_RUNTIME_DIR/canopy/canopy.sock): one JSON
//! object per line each way.
//!
//!   → {"id": 1, "cmd": "status"}
//!   ← {"id": 1, "ok": true, "result": {...}}
//!   ← {"id": 1, "ok": false, "error": "plain words"}
//!   → {"id": 2, "cmd": "subscribe"}     then events keep coming:
//!   ← {"event": "status", ...}          (engine::Event and "changed")
//!
//! Commands (arguments are fields next to "cmd"); see daemon.rs `handle`:
//!   status, file_status {path}, folder_status {dir}, pause/resume
//!   {account?}, sync_now, activity {before, limit}, issues, retry {id?},
//!   dismiss {id?}, resolve {account, root, path, choice}, login_begin,
//!   login_wait {login}, login_paste {login, text}, account_remove
//!   {account}, settings, set_settings {settings}, set_account {account,
//!   ...}, browse {account, section, folder?}, search {text}, select
//!   {account, root, path, synced}, add_folder {account, local, parent?,
//!   mode}, remove_root {account, root}, link {path}, share_info {path},
//!   share {path, ...}, star {path, on}, create {account, parent, name,
//!   kind}, move_base {account, to}, confirm {account, root, what}, quit.

use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// A connection to canopyd.
pub struct Client {
    w: UnixStream,
    r: BufReader<UnixStream>,
    next: u64,
}

impl Client {
    pub fn connect() -> Result<Client> {
        let s = UnixStream::connect(paths::socket_path()).context("canopy isn't running (no canopyd)")?;
        s.set_read_timeout(Some(Duration::from_secs(600)))?;
        Ok(Client { r: BufReader::new(s.try_clone()?), w: s, next: 1 })
    }

    /// Connect, starting canopyd first if it isn't running.
    pub fn connect_or_start() -> Result<Client> {
        if let Ok(c) = Self::connect() {
            return Ok(c);
        }
        start_daemon()?;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(c) = Self::connect() {
                return Ok(c);
            }
        }
        bail!("canopyd didn't start")
    }

    /// Send a command and wait for its answer (events in between are
    /// skipped; use `subscribe` on a connection of its own).
    pub fn call(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        let mut req = if args.is_object() { args } else { json!({}) };
        req["id"] = json!(id);
        req["cmd"] = json!(cmd);
        writeln!(self.w, "{req}")?;
        self.w.flush()?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.r.read_line(&mut line)? == 0 {
                bail!("canopyd closed the connection");
            }
            let v: Value = serde_json::from_str(&line)?;
            if v.get("id").and_then(|x| x.as_u64()) != Some(id) {
                continue;
            }
            if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            return Err(anyhow!("{}", v.get("error").and_then(|x| x.as_str()).unwrap_or("canopyd failed")));
        }
    }

    /// Turn this connection into an event stream: `f` gets each event until
    /// it returns false or the daemon goes away.
    pub fn subscribe(mut self, mut f: impl FnMut(Value) -> bool) -> Result<()> {
        self.call("subscribe", json!({}))?;
        self.w.set_read_timeout(None)?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.r.read_line(&mut line)? == 0 {
                return Ok(());
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("event").is_some() && !f(v) {
                    return Ok(());
                }
            }
        }
    }
}

/// One call on a fresh connection.
pub fn call(cmd: &str, args: Value) -> Result<Value> {
    Client::connect()?.call(cmd, args)
}

/// Start canopyd in the background: the systemd user unit when there is
/// one, else the binary next to ours.
pub fn start_daemon() -> Result<()> {
    let unit = std::process::Command::new("systemctl").args(["--user", "start", "canopyd.service"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    if matches!(unit, Ok(s) if s.success()) {
        return Ok(());
    }
    let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("canopyd"))).filter(|p| p.exists()).unwrap_or_else(|| "canopyd".into());
    use std::os::unix::process::CommandExt;
    std::process::Command::new(exe).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).process_group(0).spawn().context("can't start canopyd")?;
    Ok(())
}
