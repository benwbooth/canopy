//! Signing in to Google: OAuth 2 for installed apps with PKCE and a
//! loopback redirect (the browser comes back to 127.0.0.1), or, on a
//! machine without a browser, pasting the address the browser ended on.
//! Refresh tokens live in the Secret Service keyring (oo7); without one
//! (a headless server) in a file only the user can read.

use crate::drive::{HttpError, Tokens};
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive";

#[derive(Clone)]
pub struct Client {
    pub id: String,
    pub secret: String,
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).expect("no randomness");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// A sign-in in progress.
pub struct Pending {
    pub url: String,
    verifier: String,
    state: String,
    redirect: String,
    listener: Option<TcpListener>,
    client: Client,
}

pub fn begin(client: &Client) -> Result<Pending> {
    if client.id.is_empty() {
        bail!("No Google Cloud client is set up yet. Open Settings › Google sign-in (Help explains the setup).");
    }
    let listener = TcpListener::bind("127.0.0.1:0").context("can't listen for the browser's answer")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_b64(48);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_b64(16);
    let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        enc(&client.id),
        enc(&redirect),
        enc(SCOPE),
        challenge,
        state
    );
    Ok(Pending { url, verifier, state, redirect, listener: Some(listener), client: client.clone() })
}

fn query_params(s: &str) -> HashMap<String, String> {
    let q = s.split_once('?').map(|x| x.1).unwrap_or(s);
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned()))
        .collect()
}

impl Pending {
    /// Wait for the browser to come back (up to `timeout`). Answers the
    /// refresh token.
    pub fn wait(mut self, agent: &ureq::Agent, timeout: Duration) -> Result<String> {
        let l = self.listener.take().ok_or_else(|| anyhow!("already used"))?;
        l.set_nonblocking(true)?;
        let start = Instant::now();
        loop {
            match l.accept() {
                Ok((mut s, _)) => {
                    s.set_nonblocking(false)?;
                    s.set_read_timeout(Some(Duration::from_secs(10)))?;
                    let mut line = String::new();
                    BufReader::new(&s).read_line(&mut line)?;
                    let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    let p = query_params(&target);
                    if !p.contains_key("code") && !p.contains_key("error") {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                        continue;
                    }
                    let result = self.finish_params(agent, &p);
                    let body = match &result {
                        Ok(_) => "<h2>Signed in to canopy.</h2><p>You can close this tab.</p>".to_string(),
                        Err(e) => format!("<h2>canopy couldn't sign in</h2><p>{e}</p>"),
                    };
                    let page = format!("<!doctype html><meta charset=utf-8><title>canopy</title><body style=\"font:16px sans-serif;margin:3em\">{body}</body>");
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
                    return result;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() > timeout {
                        bail!("Sign-in timed out (no answer from the browser).");
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Finish with the address the browser ended on (pasted by the user
    /// when it ran on another machine).
    pub fn finish_pasted(&self, agent: &ureq::Agent, pasted: &str) -> Result<String> {
        let p = query_params(pasted.trim());
        if !p.contains_key("code") {
            // A bare code.
            let mut m = HashMap::new();
            m.insert("code".to_string(), pasted.trim().to_string());
            m.insert("state".to_string(), self.state.clone());
            return self.finish_params(agent, &m);
        }
        self.finish_params(agent, &p)
    }

    fn finish_params(&self, agent: &ureq::Agent, p: &HashMap<String, String>) -> Result<String> {
        if let Some(e) = p.get("error") {
            bail!("Google said: {e}");
        }
        if p.get("state") != Some(&self.state) {
            bail!("The answer didn't match this sign-in (state). Try again.");
        }
        let code = p.get("code").ok_or_else(|| anyhow!("no code in the answer"))?;
        let v: Value = token_request(
            agent,
            &[
                ("client_id", &self.client.id),
                ("client_secret", &self.client.secret),
                ("code", code),
                ("code_verifier", &self.verifier),
                ("grant_type", "authorization_code"),
                ("redirect_uri", &self.redirect),
            ],
        )?;
        v.get("refresh_token").and_then(|x| x.as_str()).map(String::from).ok_or_else(|| anyhow!("Google gave no refresh token; remove canopy under your Google Account's third-party access and sign in again"))
    }
}

fn token_request(agent: &ureq::Agent, form: &[(&str, &str)]) -> Result<Value> {
    match agent.post(TOKEN_URL).send_form(form) {
        Ok(r) => Ok(r.into_json()?),
        Err(ureq::Error::Status(status, r)) => {
            let v: Value = r.into_json().unwrap_or(Value::Null);
            let reason = v.get("error").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let desc = v.get("error_description").and_then(|x| x.as_str()).unwrap_or(&reason).to_string();
            Err(HttpError { status, reason, message: format!("Google sign-in failed: {desc}") }.into())
        }
        Err(e) => Err(anyhow::Error::new(e).context("Network Error: can't reach Google")),
    }
}

/// Access tokens for one account, refreshed when they expire.
pub struct Session {
    client: Client,
    refresh: String,
    agent: ureq::Agent,
    access: Mutex<Option<(String, Instant)>>,
}

impl Session {
    pub fn new(client: Client, refresh: String, agent: ureq::Agent) -> Self {
        Session { client, refresh, agent, access: Mutex::new(None) }
    }
}

impl Tokens for Session {
    fn access_token(&self, force: bool) -> Result<String> {
        let mut a = self.access.lock();
        if let Some((t, until)) = a.as_ref() {
            if !force && Instant::now() < *until {
                return Ok(t.clone());
            }
        }
        let v = token_request(&self.agent, &[("client_id", &self.client.id), ("client_secret", &self.client.secret), ("refresh_token", &self.refresh), ("grant_type", "refresh_token")])?;
        let t = v.get("access_token").and_then(|x| x.as_str()).ok_or_else(|| anyhow!("no access token"))?.to_string();
        let secs = v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(3600);
        *a = Some((t.clone(), Instant::now() + Duration::from_secs(secs.saturating_sub(120))));
        Ok(t)
    }
}

// --- where refresh tokens are kept ---

fn attrs(account: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", "canopy"), ("account", account)])
}

fn token_file(account: &str) -> std::path::PathBuf {
    paths::account_dir(account).join("token")
}

fn keyring_disabled() -> bool {
    std::env::var_os("CANOPY_NO_KEYRING").is_some()
}

pub fn store_token(account: &str, email: &str, token: &str) -> Result<()> {
    if !keyring_disabled() {
        let r: Result<()> = futures_lite::future::block_on(async {
            let k = oo7::Keyring::new().await?;
            k.create_item(&format!("canopy: Google Drive ({email})"), &attrs(account), token, true).await?;
            Ok(())
        });
        match r {
            Ok(()) => {
                let _ = std::fs::remove_file(token_file(account));
                return Ok(());
            }
            Err(e) => eprintln!("canopy: no keyring ({e:#}); keeping the sign-in in a private file"),
        }
    }
    use std::os::unix::fs::OpenOptionsExt;
    let p = token_file(account);
    std::fs::create_dir_all(p.parent().unwrap())?;
    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&p)?;
    f.write_all(token.as_bytes())?;
    Ok(())
}

pub fn load_token(account: &str) -> Result<String> {
    if let Ok(t) = std::fs::read_to_string(token_file(account)) {
        return Ok(t.trim().to_string());
    }
    if keyring_disabled() {
        bail!("not signed in");
    }
    futures_lite::future::block_on(async {
        let k = oo7::Keyring::new().await?;
        let items = k.search_items(&attrs(account)).await?;
        let it = items.first().ok_or_else(|| anyhow!("not signed in (no keyring entry)"))?;
        let s = it.secret().await?;
        Ok(String::from_utf8_lossy(s.as_bytes()).into_owned())
    })
}

pub fn forget_token(account: &str) {
    let _ = std::fs::remove_file(token_file(account));
    if !keyring_disabled() {
        let _: Result<()> = futures_lite::future::block_on(async {
            let k = oo7::Keyring::new().await?;
            k.delete(&attrs(account)).await?;
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_params() {
        let p = begin(&Client { id: "abc.apps.googleusercontent.com".into(), secret: "s".into() }).unwrap();
        assert!(p.url.contains("code_challenge_method=S256"));
        assert!(p.url.contains("auth%2Fdrive"));
        let q = query_params("http://127.0.0.1:1234/?state=x&code=4%2F0Ab&scope=a+b");
        assert_eq!(q["code"], "4/0Ab");
        assert_eq!(q["scope"], "a b");
    }
}
