//! canopyd over its socket, as the window, the CLI and grove use it: a real
//! daemon with the demo account (in-memory Drive), in scratch folders.

use canopy_core::api::Client;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

struct Daemon {
    child: Child,
    _dir: tempfile::TempDir,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::create_dir_all(d.join("run")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_canopyd"))
        .env("HOME", d.join("home"))
        .env("XDG_CONFIG_HOME", d.join("config"))
        .env("XDG_DATA_HOME", d.join("data"))
        .env("CANOPY_RUNTIME_DIR", d.join("run"))
        .env("CANOPY_TRASH_DIR", d.join("trash"))
        .env("CANOPY_DEMO", "1")
        .env("CANOPY_NO_KEYRING", "1")
        .env("CANOPY_NO_TRAY", "1")
        .env("CANOPY_NO_HEARTH", "1")
        .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
        .spawn()
        .unwrap();
    std::env::set_var("CANOPY_RUNTIME_DIR", d.join("run"));
    for _ in 0..100 {
        if d.join("run/canopy.sock").exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Daemon { child, _dir: dir }
}

fn wait_synced(c: &mut Client) {
    for _ in 0..100 {
        let s = c.call("status", json!({})).unwrap();
        if s["state"] == "synced" {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("demo account never synced");
}

#[test]
fn id_arguments_survive_and_old_clients_work() {
    let _d = start();
    let mut c = Client::connect().unwrap();
    wait_synced(&mut c);
    let items = c.call("browse", json!({"account": "demo@example.com", "section": "my_drive"})).unwrap();
    let kdbx = items.as_array().unwrap().iter().find(|i| i["name"] == "ben.kdbx").unwrap()["id"].as_str().unwrap().to_string();
    // "id" here is the file, not the request number.
    c.call("star", json!({"account": "demo@example.com", "id": kdbx, "on": true})).unwrap();
    let starred: Vec<Value> = serde_json::from_value(c.call("browse", json!({"account": "demo@example.com", "section": "starred"})).unwrap()).unwrap();
    assert!(starred.iter().any(|i| i["id"] == kdbx.as_str()), "the file asked for was starred: {starred:?}");

    // An older client (grove) numbering requests with "id".
    let sock = std::env::var("CANOPY_RUNTIME_DIR").unwrap();
    let mut s = UnixStream::connect(Path::new(&sock).join("canopy.sock")).unwrap();
    writeln!(s, "{}", json!({"id": 7, "cmd": "status"})).unwrap();
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!((v["id"].as_u64(), v["ok"].as_bool()), (Some(7), Some(true)));
}
