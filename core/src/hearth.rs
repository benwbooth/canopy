//! hearth's Internet Accounts (Settings › Internet Accounts): Google
//! accounts set up once for every app, through the session D-Bus service
//! io.github.benwbooth.Accounts (hearth/helpers/hearth-accounts), the same
//! one grove and almanac use.
//!
//! When the service is there, canopy's Google accounts come from it: canopy
//! keeps no Google client or sign-in of its own and asks for an access
//! token for the "sync" purpose (the full Drive scope). Without it (another
//! desktop, a server) canopy signs in by itself (auth.rs).
//!
//! Every call blocks on D-Bus: worker threads only.

use crate::drive::{HttpError, Tokens};
use anyhow::Result;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use zbus::blocking::Connection;

pub const NAME: &str = "io.github.benwbooth.Accounts";
pub const PATH: &str = "/io/github/benwbooth/Accounts";
pub const IFACE: &str = "io.github.benwbooth.Accounts1";
/// The purpose canopy asks for: read and change every Drive file.
pub const PURPOSE: &str = "sync";

fn conn() -> Option<Connection> {
    static C: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    let mut c = C.get_or_init(Default::default).lock();
    if c.is_none() {
        if std::env::var_os("CANOPY_NO_HEARTH").is_some() {
            return None;
        }
        *c = Connection::session().ok();
    }
    c.clone()
}

/// A D-Bus error from the service as canopy's error: SignedOut and
/// NotAllowed look like Google's 401 (the account shows "sign in again").
fn err(e: zbus::Error) -> anyhow::Error {
    match e {
        zbus::Error::MethodError(name, detail, _) => {
            let msg = detail.unwrap_or_else(|| name.to_string());
            match name.as_str().rsplit('.').next().unwrap_or("") {
                "SignedOut" => HttpError { status: 401, reason: "signed_out".into(), message: msg }.into(),
                "NotAllowed" => HttpError { status: 401, reason: "not_allowed".into(), message: format!("{msg} (canopy needs “See, edit, create and delete all of your Google Drive files”.)") }.into(),
                _ => anyhow::anyhow!("Internet Accounts: {msg}"),
            }
        }
        e => anyhow::anyhow!("Internet Accounts didn't answer ({e})"),
    }
}

fn call<B>(method: &str, body: &B) -> Result<String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let c = conn().ok_or_else(|| anyhow::anyhow!("no session bus"))?;
    let m = c.call_method(Some(NAME), PATH, Some(IFACE), method, body).map_err(err)?;
    Ok(m.body().deserialize::<String>()?)
}

/// Whether the service is running or D-Bus can start it.
pub fn available() -> bool {
    let Some(c) = conn() else { return false };
    let ask = |m: &str| c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), m, &());
    if c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(NAME,)).ok().and_then(|r| r.body().deserialize::<bool>().ok()) == Some(true) {
        return true;
    }
    ask("ListActivatableNames").ok().and_then(|r| r.body().deserialize::<Vec<String>>().ok()).is_some_and(|v| v.iter().any(|n| n == NAME))
}

/// The Google accounts set up there (no secrets): [{id, email, name,
/// uses: [...], signed_out, error}].
pub fn google_accounts() -> Result<Vec<Value>> {
    let v: Vec<Value> = serde_json::from_str(&call("List", &())?)?;
    Ok(v.into_iter().filter(|a| a["kind"] == "google").collect())
}

/// Whether a Google client is registered there (needed to sign in).
pub fn google_registered() -> bool {
    call("Registrations", &()).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).map(|v| v.pointer("/google/client_id").and_then(|x| x.as_str()).is_some_and(|x| !x.is_empty())).unwrap_or(false)
}

/// Open Settings › Internet Accounts at `arg` (an account id, or
/// "signin:google", "setup:google").
pub fn open_settings(arg: &str) -> bool {
    conn().and_then(|c| c.call_method(Some(NAME), PATH, Some(IFACE), "OpenSettings", &(arg,)).ok()).and_then(|m| m.body().deserialize::<bool>().ok()).unwrap_or(false)
}

/// Sign in to a Google account again in the browser (to allow Drive sync);
/// finishing is reported by the Changed signal.
pub fn sign_in_again(id: &str) -> Result<()> {
    call("SignIn", &("google", id)).map(|_| ())
}

/// Access tokens for one Internet Accounts account.
pub struct HearthTokens {
    pub id: String,
    cache: Mutex<Option<(String, Instant)>>,
}

impl HearthTokens {
    pub fn new(id: &str) -> Self {
        HearthTokens { id: id.into(), cache: Mutex::new(None) }
    }
}

impl Tokens for HearthTokens {
    fn access_token(&self, force: bool) -> Result<String> {
        let mut c = self.cache.lock();
        if let Some((t, until)) = c.as_ref() {
            if !force && Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let v: Value = serde_json::from_str(&call("AccessToken", &(self.id.as_str(), PURPOSE))?)?;
        let t = v["token"].as_str().ok_or_else(|| anyhow::anyhow!("Internet Accounts gave no token"))?.to_string();
        let secs = v["expires_in"].as_u64().unwrap_or(600);
        *c = Some((t.clone(), Instant::now() + Duration::from_secs(secs.saturating_sub(60).max(30))));
        Ok(t)
    }
}

/// Call `f` each time the accounts change there (a thread of its own).
pub fn watch(f: impl Fn() + Send + 'static) {
    std::thread::Builder::new()
        .name("hearth-accounts".into())
        .spawn(move || loop {
            if let Some(c) = conn() {
                if let Ok(p) = zbus::blocking::Proxy::new(&c, NAME, PATH, IFACE) {
                    if let Ok(sigs) = p.receive_signal("Changed") {
                        for _ in sigs {
                            f();
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(30));
        })
        .ok();
}
