//! `canopy <command>`: Insync's command line (its headless commands), over
//! canopyd's socket. Plain text out; exit status 1 on errors.

use anyhow::{anyhow, bail, Result};
use canopy_core::api;
use serde_json::{json, Value};
use std::io::BufRead;

pub const COMMANDS: &[&str] = &[
    "status", "preview", "pause", "resume", "sync", "account", "ignore-rules", "export-option", "conflict", "error", "selective-sync", "open-cloud", "share-link", "config", "quit", "start", "activity", "add-folder",
];

pub const USAGE: &str = "canopy: Google Drive sync

  canopy                         open the window
  canopy start                   start syncing in the background (canopyd)
  canopy status [-f FILE]        overall state, or one file's (synced, syncing...)
  canopy preview [-a EMAIL]      what syncing would do now (changes nothing)
  canopy pause | resume          stop or continue syncing (every account)
  canopy sync                    look for changes now
  canopy activity [N]            the last N things canopy did
  canopy account list | add [--paste] | remove EMAIL
  canopy ignore-rules [-a EMAIL] list | add RULE... | clear
  canopy export-option [-a EMAIL] [LINK|MS_OFFICE|OPEN_DOCUMENT]
  canopy conflict list | resolve PATH local|cloud|both [--always]
  canopy error list | retry-all | dismiss-all
  canopy selective-sync [-a EMAIL] [PATH on|off]
  canopy add-folder LOCAL [--backup|--download-only]   sync any folder
  canopy open-cloud FILE         open a file (or link file) on drive.google.com
  canopy share-link FILE         print a file's link
  canopy config [-l] [KEY [VALUE]]
  canopy quit                    stop canopyd

  canopy --share FILE            open the Share window for a file
  canopy --add-folder FOLDER     open \"Sync this folder\" in the window
";

fn take_account(args: &mut Vec<String>) -> String {
    if let Some(i) = args.iter().position(|a| a == "-a" || a == "--account") {
        if i + 1 < args.len() {
            let v = args.remove(i + 1);
            args.remove(i);
            return v;
        }
    }
    String::new()
}

fn abs(p: &str) -> String {
    let pb = std::path::Path::new(p);
    if pb.is_absolute() {
        p.into()
    } else {
        std::env::current_dir().unwrap_or_default().join(pb).to_string_lossy().into_owned()
    }
}

fn call(cmd: &str, args: Value) -> Result<Value> {
    api::Client::connect_or_start()?.call(cmd, args)
}

fn words(state: &str) -> &str {
    match state {
        "S" => "synced",
        "P" => "syncing",
        "E" => "error",
        "C" => "conflict",
        "I" => "ignored",
        other => other,
    }
}

pub fn run(cmd: &str, mut args: Vec<String>) -> Result<()> {
    let account = take_account(&mut args);
    match cmd {
        "start" => {
            api::Client::connect_or_start()?;
            println!("canopy is syncing in the background.");
        }
        "status" => {
            if let Some(i) = args.iter().position(|a| a == "-f" || a == "--file-path") {
                let p = args.get(i + 1).ok_or_else(|| anyhow!("-f needs a file"))?;
                let v = call("file_status", json!({"path": abs(p)}))?;
                println!("{}", words(v["state"].as_str().unwrap_or("")));
                for i in v["issues"].as_array().into_iter().flatten() {
                    println!("  {}", i["message"].as_str().unwrap_or(""));
                }
                return Ok(());
            }
            let v = call("status", json!({}))?;
            println!("{}", v["state"].as_str().unwrap_or("?"));
            for a in v["accounts"].as_array().into_iter().flatten() {
                let st = &a["state"];
                println!("  {} — {}{}", a["email"].as_str().unwrap_or(""), st["phase"].as_str().unwrap_or(""), st["message"].as_str().filter(|m| !m.is_empty()).map(|m| format!(": {m}")).unwrap_or_default());
                for r in a["roots"].as_array().into_iter().flatten() {
                    println!("    {} → {} ({})", r["name"].as_str().unwrap_or(""), r["local"].as_str().unwrap_or(""), r["mode"].as_str().unwrap_or(""));
                }
                if let Some(h) = st["held"].as_object() {
                    for (_, why) in h {
                        println!("    held: {}", why.as_str().unwrap_or(""));
                    }
                }
                let (e, c) = (a["errors"].as_u64().unwrap_or(0), a["conflicts_count"].as_u64().unwrap_or(0));
                if e + c > 0 {
                    println!("    {e} error(s), {c} conflict(s): canopy error list / canopy conflict list");
                }
            }
            for t in v["transfers"].as_array().into_iter().flatten() {
                let (d, n) = (t["done"].as_u64().unwrap_or(0), t["total"].as_u64().unwrap_or(0).max(1));
                println!("  {} {} {}%", if t["up"].as_bool() == Some(true) { "↑" } else { "↓" }, t["path"].as_str().unwrap_or(""), d * 100 / n);
            }
        }
        "preview" => {
            let st = call("status", json!({}))?;
            let a = pick(&st, &account)?;
            let v = call("preview", json!({"account": a["id"]}))?;
            for (_, r) in v.as_object().into_iter().flatten() {
                let plan = r["plan"].as_array().cloned().unwrap_or_default();
                println!("{} ({}): {}", r["name"].as_str().unwrap_or(""), r["local"].as_str().unwrap_or(""), if plan.is_empty() { "nothing to do".to_string() } else { format!("{} change(s)", plan.len()) });
                if let Some(h) = r["held"].as_str() {
                    println!("  held: {h}");
                }
                for l in plan {
                    println!("  {}", l.as_str().unwrap_or(""));
                }
            }
        }
        "pause" | "resume" => {
            call(cmd, json!({"account": account}))?;
        }
        "sync" => {
            call("sync_now", json!({}))?;
        }
        "quit" => {
            if api::call("quit", json!({})).is_err() {
                println!("canopy wasn't running.");
            }
        }
        "activity" => {
            let n: u64 = args.first().and_then(|x| x.parse().ok()).unwrap_or(20);
            for a in call("activity", json!({"limit": n}))?.as_array().into_iter().flatten() {
                let t = chrono_like(a["at"].as_f64().unwrap_or(0.0));
                println!("{t}  {:<12} {} {}", a["kind"].as_str().unwrap_or(""), a["path"].as_str().unwrap_or(""), a["detail"].as_str().unwrap_or(""));
            }
        }
        "account" => match args.first().map(|s| s.as_str()).unwrap_or("list") {
            "list" => {
                for a in call("status", json!({}))?["accounts"].as_array().into_iter().flatten() {
                    println!("{}  {}", a["email"].as_str().unwrap_or(""), a["base"].as_str().unwrap_or(""));
                }
            }
            "add" => {
                let mut c = api::Client::connect_or_start()?;
                let b = c.call("login_begin", json!({}))?;
                let url = b["url"].as_str().unwrap_or("");
                let login = b["login"].as_str().unwrap_or("").to_string();
                println!("Sign in to Google in your browser:\n\n  {url}\n");
                let r = if args.iter().any(|a| a == "--paste") {
                    println!("This computer has no browser? Open the address on another one. When it ends on a page that doesn't load, copy that page's address and paste it here:");
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    c.call("login_paste", json!({"login": login, "text": line.trim()}))?
                } else {
                    let _ = std::process::Command::new("xdg-open").arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
                    println!("Waiting for the browser… (no browser here? run `canopy account add --paste`)");
                    c.call("login_wait", json!({"login": login}))?
                };
                println!("Added {}.", r["email"].as_str().unwrap_or(""));
            }
            "remove" => {
                let who = args.get(1).cloned().unwrap_or(account);
                let r = call("account_remove", json!({"account": who}))?;
                println!("Removed. Your files are still in {}.", r["kept"].as_str().unwrap_or("their folder"));
            }
            x => bail!("unknown account command {x}"),
        },
        "ignore-rules" => {
            let st = call("status", json!({}))?;
            let a = pick(&st, &account)?;
            let mut rules: Vec<String> = serde_json::from_value(a["ignore_rules"].clone()).unwrap_or_default();
            match args.first().map(|s| s.as_str()).unwrap_or("list") {
                "list" => rules.iter().for_each(|r| println!("{r}")),
                "add" => {
                    rules.extend(args[1..].iter().cloned());
                    call("set_account", json!({"account": a["id"], "ignore_rules": rules}))?;
                }
                "clear" => {
                    call("set_account", json!({"account": a["id"], "ignore_rules": []}))?;
                }
                x => bail!("unknown ignore-rules command {x}"),
            }
        }
        "export-option" => {
            let st = call("status", json!({}))?;
            let a = pick(&st, &account)?;
            match args.first() {
                None => println!("{}", a["export"].as_str().unwrap_or("").to_uppercase()),
                Some(v) => {
                    call("set_account", json!({"account": a["id"], "export": v.to_lowercase()}))?;
                }
            }
        }
        "conflict" | "error" => {
            let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
            let issues = call("issues", json!({}))?;
            let want = |i: &Value| (i["kind"] == "conflict") == (cmd == "conflict");
            match sub {
                "list" => {
                    for i in issues.as_array().into_iter().flatten().filter(|i| want(i)) {
                        println!("{}  {}", i["local"].as_str().unwrap_or(i["path"].as_str().unwrap_or("")), i["message"].as_str().unwrap_or(""));
                    }
                }
                "retry-all" => {
                    call("retry", json!({}))?;
                }
                "dismiss-all" => {
                    for i in issues.as_array().into_iter().flatten().filter(|i| want(i)) {
                        call("dismiss", json!({"account": i["account"], "id": i["id"]}))?;
                    }
                }
                "resolve" => {
                    let path = abs(args.get(1).ok_or_else(|| anyhow!("which file?"))?);
                    let choice = match args.get(2).map(|s| s.as_str()) {
                        Some("local") => "keep_local",
                        Some("cloud") => "keep_cloud",
                        Some("both") | None => "keep_both",
                        Some(x) => bail!("choose local, cloud or both (not {x})"),
                    };
                    let i = issues.as_array().into_iter().flatten().find(|i| i["local"].as_str() == Some(&path)).ok_or_else(|| anyhow!("no conflict for {path}"))?;
                    call("resolve", json!({"account": i["account"], "root": i["root"], "path": i["path"], "choice": choice, "always": args.iter().any(|a| a == "--always")}))?;
                }
                x => bail!("unknown {cmd} command {x}"),
            }
        }
        "selective-sync" => {
            let st = call("status", json!({}))?;
            let a = pick(&st, &account)?;
            if args.len() >= 2 {
                let on = matches!(args[1].as_str(), "on" | "yes" | "true" | "sync");
                call("select", json!({"account": a["id"], "root": "mydrive", "path": args[0], "synced": on}))?;
                return Ok(());
            }
            let items = call("browse", json!({"account": a["id"], "section": "my_drive"}))?;
            println!("My Drive (canopy selective-sync PATH on|off):");
            for i in items.as_array().into_iter().flatten() {
                let mark = match i["sync"].as_str().unwrap_or("") {
                    "synced" | "pending" => "[x]",
                    "partial" => "[-]",
                    _ => "[ ]",
                };
                println!("  {mark} {}{}", i["name"].as_str().unwrap_or(""), if i["folder"].as_bool() == Some(true) { "/" } else { "" });
            }
        }
        "add-folder" => {
            let st = call("status", json!({}))?;
            let a = pick(&st, &account)?;
            let local = abs(args.first().ok_or_else(|| anyhow!("which folder?"))?);
            let mode = if args.iter().any(|x| x == "--backup") {
                "local_to_cloud"
            } else if args.iter().any(|x| x == "--download-only") {
                "cloud_to_local"
            } else {
                "two_way"
            };
            call("add_folder", json!({"account": a["id"], "local": local, "mode": mode}))?;
            println!("Syncing {local} with Google Drive.");
        }
        "open-cloud" | "share-link" => {
            let p = abs(args.first().ok_or_else(|| anyhow!("which file?"))?);
            let v = call("link", json!({"path": p}))?;
            let url = v["url"].as_str().unwrap_or("");
            if cmd == "share-link" {
                println!("{url}");
            } else {
                std::process::Command::new("xdg-open").arg(url).spawn()?;
            }
        }
        "config" => {
            let s = call("settings", json!({}))?;
            let list = args.iter().any(|a| a == "-l" || a == "--list");
            args.retain(|a| a != "-l" && a != "--list");
            match (args.first(), args.get(1)) {
                (Some(k), Some(v)) => {
                    let val: Value = serde_json::from_str(v).unwrap_or(json!(v));
                    call("set_settings", json!({"settings": {k.as_str(): val}}))?;
                }
                (Some(k), None) => println!("{}", s.get(k.as_str()).map(|x| x.to_string()).unwrap_or_default()),
                _ => {
                    let _ = list;
                    for (k, v) in s.as_object().into_iter().flatten() {
                        if k != "client_secret" {
                            println!("{k} = {v}");
                        }
                    }
                }
            }
        }
        _ => bail!("unknown command {cmd}\n\n{USAGE}"),
    }
    Ok(())
}

fn pick<'a>(st: &'a Value, account: &str) -> Result<&'a Value> {
    let accts = st["accounts"].as_array().ok_or_else(|| anyhow!("no accounts"))?;
    if account.is_empty() {
        if accts.len() == 1 {
            return Ok(&accts[0]);
        }
        bail!("several accounts: choose one with -a EMAIL");
    }
    accts.iter().find(|a| a["email"].as_str() == Some(account) || a["id"].as_str() == Some(account)).ok_or_else(|| anyhow!("no account {account}"))
}

fn chrono_like(t: f64) -> String {
    chrono::DateTime::from_timestamp(t as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}
