//! One canopy window at a time. The first `canopy` takes the session-bus
//! name io.github.benwbooth.Canopy (before Qt starts, so two launches at
//! once can't both win); a later one hands over what it was asked to open,
//! plus its XDG activation token, and exits. The running window then raises
//! itself (and opens the Share/Add sheet if asked).

use crate::bridge::ffi;
use cxx_qt::CxxQtThread;
use std::sync::{Mutex, OnceLock};

const NAME: &str = "io.github.benwbooth.Canopy";
const PATH: &str = "/io/github/benwbooth/Canopy";

type Qt = CxxQtThread<ffi::Canopy>;

/// Requests that came in before the window's controller existed.
struct State {
    qt: Option<Qt>,
    pending: Vec<String>,
}
static STATE: Mutex<State> = Mutex::new(State { qt: None, pending: Vec::new() });
/// Holds the name for the life of the process.
static CONN: OnceLock<zbus::blocking::Connection> = OnceLock::new();

struct Own;

#[zbus::interface(name = "io.github.benwbooth.Canopy")]
impl Own {
    /// `request`: JSON {share?, add?, token?}.
    fn activate(&self, request: &str) {
        deliver(request.to_string());
    }
}

fn deliver(req: String) {
    let mut st = STATE.lock().unwrap();
    match &st.qt {
        Some(qt) => {
            let _ = qt.queue(move |s| crate::controller::activate(s, req));
        }
        None => st.pending.push(req),
    }
}

/// The controller exists: deliver what waited, and everything after.
pub fn attach(qt: Qt) {
    let mut st = STATE.lock().unwrap();
    for req in st.pending.drain(..) {
        let _ = qt.queue(move |s| crate::controller::activate(s, req));
    }
    st.qt = Some(qt);
}

/// True: this process is the window (it now holds the name, or there's no
/// session bus). False: a running canopy took the request; exit.
pub fn claim_or_forward(request: &serde_json::Value) -> bool {
    let conn = zbus::blocking::connection::Builder::session().and_then(|b| b.serve_at(PATH, Own)).and_then(|b| b.build());
    let Ok(c) = conn else { return true };
    use zbus::fdo::{RequestNameFlags, RequestNameReply};
    match c.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => {
            let _ = CONN.set(c);
            true
        }
        Ok(_) | Err(zbus::Error::NameTaken) => {
            // (If the running one doesn't answer, open a window here after all.)
            c.call_method(Some(NAME), PATH, Some(NAME), "Activate", &(request.to_string(),)).is_err()
        }
        Err(_) => true,
    }
}
