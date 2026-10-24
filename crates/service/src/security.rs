//! This computer's key, the computers it is paired with, and pairings in progress
//! (see usbipb_core::secure for the cryptography).
//!
//! Pairing: the user starts it on one computer, which shows a code; the other computer asks
//! its user for that code. Both prove to each other that they used the same code on this
//! very connection, then each remembers the other's key (paired.json).

use crate::engine::{hostname, state_dir, Engine};
use anyhow::{bail, Context, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usbipb_core::api::{PairedComputer, Pairing, PairingState};
use usbipb_core::peer::{PeerClient, PeerRequest, PeerResponse};
use usbipb_core::secure::{self, Identity, SecureStream};

/// How long the user has to type the code.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(300);
/// Finished pairings stay visible this long.
const SHOW_FINISHED: Duration = Duration::from_secs(60);
/// Pairing requests from other computers waiting for the user at the same time.
const MAX_WAITING: usize = 3;

struct Entry {
    info: Pairing,
    updated: Instant,
    /// The user's answer to a request of another computer: the code, or None (declined).
    answer: Option<Option<String>>,
}

pub struct Security {
    pub identity: Identity,
    paired: Mutex<Vec<PairedComputer>>,
    pairings: Mutex<Vec<Entry>>,
    next_id: AtomicU64,
}

fn paired_path() -> std::path::PathBuf {
    state_dir().join("paired.json")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A request from another computer's pairing, waiting for this side's PairConfirm.
pub struct PendingPair {
    id: u64,
    key: Vec<u8>,
    host_id: String,
    host_name: String,
}

impl Security {
    pub fn load() -> Result<Self> {
        let identity = Identity::load_or_create(&state_dir().join("host-key"))?;
        let paired = std::fs::read_to_string(paired_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Ok(Self { identity, paired: Mutex::new(paired), pairings: Mutex::new(Vec::new()), next_id: AtomicU64::new(1) })
    }

    pub fn fingerprint(&self) -> String {
        secure::fingerprint(&self.identity.public)
    }

    pub fn paired(&self) -> Vec<PairedComputer> {
        self.paired.lock().unwrap().clone()
    }

    pub fn is_paired(&self, key: &[u8]) -> bool {
        let key = secure::to_hex(key);
        self.paired.lock().unwrap().iter().any(|p| p.key == key)
    }

    /// Refuses a computer that claims to be a paired one but has another key.
    pub fn check_identity(&self, host_id: &str, key: &[u8]) -> Result<()> {
        let key = secure::to_hex(key);
        if let Some(p) = self.paired.lock().unwrap().iter().find(|p| p.host_id == host_id && p.key != key) {
            bail!(
                "{} answers with a different key than when it was paired – not trusted. If it was reinstalled, remove the pairing and pair again.",
                p.host_name
            );
        }
        Ok(())
    }

    fn save(&self, list: &[PairedComputer]) -> Result<()> {
        std::fs::write(paired_path(), serde_json::to_string_pretty(list)?).context("save paired.json")
    }

    fn add_paired(&self, host_id: &str, host_name: &str, key: &[u8]) -> Result<()> {
        let key = secure::to_hex(key);
        let mut list = self.paired.lock().unwrap();
        list.retain(|p| p.host_id != host_id && p.key != key);
        list.push(PairedComputer { host_id: host_id.into(), host_name: host_name.into(), key, since: unix_now() });
        self.save(&list)?;
        log::info!("paired with {host_name}");
        Ok(())
    }

    /// Removes a pairing by host id or key; returns the removed entry.
    fn remove(&self, host_id: Option<&str>, key: Option<&[u8]>) -> Result<Option<PairedComputer>> {
        let key = key.map(secure::to_hex);
        let mut list = self.paired.lock().unwrap();
        let Some(i) = list.iter().position(|p| Some(p.host_id.as_str()) == host_id || Some(&p.key) == key.as_ref()) else { return Ok(None) };
        let p = list.remove(i);
        self.save(&list)?;
        Ok(Some(p))
    }

    /// Pairings in progress and the ones finished a moment ago.
    pub fn pairings(&self) -> Vec<Pairing> {
        let mut list = self.pairings.lock().unwrap();
        list.retain(|e| e.info.state == PairingState::Waiting || e.updated.elapsed() < SHOW_FINISHED);
        list.iter().map(|e| e.info.clone()).collect()
    }

    fn new_pairing(&self, outgoing: bool, address: &str, host_name: &str, code: Option<String>) -> Result<u64> {
        let mut list = self.pairings.lock().unwrap();
        if !outgoing && list.iter().filter(|e| !e.info.outgoing && e.info.state == PairingState::Waiting).count() >= MAX_WAITING {
            bail!("too many pairing requests at the same time");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let info = Pairing { id, outgoing, address: address.into(), host_name: host_name.into(), code, state: PairingState::Waiting, message: None };
        list.push(Entry { info, updated: Instant::now(), answer: None });
        Ok(id)
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Pairing)) {
        if let Some(e) = self.pairings.lock().unwrap().iter_mut().find(|e| e.info.id == id) {
            f(&mut e.info);
            e.updated = Instant::now();
        }
    }

    fn finish(&self, id: u64, result: &Result<String>) {
        self.update(id, |p| match result {
            Ok(_) => p.state = PairingState::Paired,
            Err(e) => {
                p.state = PairingState::Failed;
                p.message = Some(format!("{e:#}"));
            }
        });
    }

    /// The user's answer to another computer's pairing request (`code: None`: declined).
    pub fn answer(&self, id: u64, code: Option<String>) -> Result<String> {
        let mut list = self.pairings.lock().unwrap();
        let Some(e) = list.iter_mut().find(|e| e.info.id == id && !e.info.outgoing && e.info.state == PairingState::Waiting) else {
            bail!("this pairing request is no longer waiting");
        };
        if let Some(c) = &code {
            if secure::normalize_code(c).len() != 6 {
                bail!("the code has 6 digits");
            }
        }
        e.answer = Some(code);
        Ok("answered".into())
    }

    /// Waits for the user's answer; None if none came in time.
    fn wait_answer(&self, id: u64, stop: &AtomicBool) -> Option<Option<String>> {
        let end = Instant::now() + PAIRING_TIMEOUT;
        while Instant::now() < end && !stop.load(Ordering::Relaxed) {
            if let Some(answer) = self.pairings.lock().unwrap().iter_mut().find(|e| e.info.id == id).and_then(|e| e.answer.take()) {
                return Some(answer);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        None
    }
}

/// Starts pairing with the computer at control address `address`; this computer shows the
/// code (in the status) until the user typed it there.
pub fn start_pairing(engine: &Arc<Engine>, address: &str) -> Result<String> {
    let code = secure::new_code()?;
    let name = engine.peers.list().into_iter().find(|p| p.address == address).map(|p| p.name()).unwrap_or_else(|| address.to_string());
    let id = engine.security.new_pairing(true, address, &name, Some(code.clone()))?;
    log::info!("pairing with {name} ({address}) started");
    let message = format!("type the code {code} on {name}");
    let (engine, address) = (engine.clone(), address.to_string());
    std::thread::spawn(move || {
        let result = pair_outgoing(&engine, id, &address, &code);
        match &result {
            Ok(name) => log::info!("pairing with {name}: done"),
            Err(e) => log::info!("pairing with {address}: {e:#}"),
        }
        engine.security.finish(id, &result);
    });
    Ok(message)
}

fn pair_outgoing(engine: &Engine, id: u64, address: &str, code: &str) -> Result<String> {
    let sec = &engine.security;
    let mut c = PeerClient::connect(address, &sec.identity)?;
    let hello = match c.call(&PeerRequest::Hello)? {
        PeerResponse::Hello(h) => h,
        other => bail!("unexpected answer {other:?}"),
    };
    if hello.host_id == engine.host_id {
        bail!("that is this computer");
    }
    sec.update(id, |p| p.host_name = hello.host_name.clone());
    let (pake, msg) = secure::pake_start(code, c.conn());
    c.set_timeout(PAIRING_TIMEOUT + Duration::from_secs(30))?;
    let request = PeerRequest::PairStart { host_id: engine.host_id.clone(), host_name: hostname(), pake: secure::to_hex(&msg) };
    let (theirs, confirm) = match c.call(&request)? {
        PeerResponse::PairReply { pake, confirm } => (secure::from_hex(&pake)?, confirm),
        PeerResponse::Error { message } => bail!(message),
        other => bail!("unexpected answer {other:?}"),
    };
    let key = secure::pake_finish(pake, &theirs)?;
    c.set_timeout(Duration::from_secs(10))?;
    if confirm != secure::confirmation(&key, "responder") {
        let _ = c.call(&PeerRequest::PairConfirm { confirm: String::new() });
        bail!("the code typed on {} was not the one shown here – start again", hello.host_name);
    }
    c.call_ok(&PeerRequest::PairConfirm { confirm: secure::confirmation(&key, "initiator") })?;
    sec.add_paired(&hello.host_id, &hello.host_name, c.conn().remote_key())?;
    Ok(hello.host_name)
}

/// Another computer asks to pair (control connection `conn` from `from`): waits for the
/// user's code, then answers with this side's part of the key exchange.
pub fn pair_incoming(engine: &Engine, conn: &SecureStream, from: &str, host_id: String, host_name: String, pake: &str, pending: &mut Option<PendingPair>) -> PeerResponse {
    let sec = &engine.security;
    if host_id == engine.host_id {
        return PeerResponse::Error { message: "that is this computer".into() };
    }
    let id = match sec.new_pairing(false, from, &host_name, None) {
        Ok(id) => id,
        Err(e) => return PeerResponse::Error { message: format!("{e:#}") },
    };
    log::info!("{host_name} ({from}) asks to pair – waiting for the code");
    let result = (|| -> Result<PeerResponse> {
        let theirs = secure::from_hex(pake)?;
        let code = match sec.wait_answer(id, &engine.stop) {
            None => bail!("no code was typed on {} in time", hostname()),
            Some(None) => bail!("pairing was declined on {}", hostname()),
            Some(Some(code)) => code,
        };
        let (mine, msg) = secure::pake_start(&code, conn);
        let key = secure::pake_finish(mine, &theirs)?;
        let confirm = secure::confirmation(&key, "responder");
        *pending = Some(PendingPair { id, key, host_id, host_name });
        Ok(PeerResponse::PairReply { pake: secure::to_hex(&msg), confirm })
    })();
    match result {
        Ok(r) => r,
        Err(e) => {
            log::info!("pairing with {from}: {e:#}");
            sec.finish(id, &Err(anyhow::anyhow!("{e:#}")));
            PeerResponse::Error { message: format!("{e:#}") }
        }
    }
}

/// The other computer's proof after `pair_incoming`.
pub fn pair_confirm(engine: &Engine, conn: &SecureStream, confirm: &str, pending: &mut Option<PendingPair>) -> PeerResponse {
    let Some(p) = pending.take() else { return PeerResponse::Error { message: "no pairing in progress".into() } };
    let result = if confirm == secure::confirmation(&p.key, "initiator") {
        engine.security.add_paired(&p.host_id, &p.host_name, conn.remote_key()).map(|_| format!("paired with {}", hostname()))
    } else {
        Err(anyhow::anyhow!("the code typed here was not the one shown on {} – start again", p.host_name))
    };
    engine.security.finish(p.id, &result);
    match result {
        Ok(message) => PeerResponse::Ok { message },
        Err(e) => PeerResponse::Error { message: format!("{e:#}") },
    }
}

/// The other computer removed the pairing.
pub fn unpaired_by_peer(engine: &Engine, key: &[u8], from: &str) -> PeerResponse {
    match engine.security.remove(None, Some(key)) {
        Ok(Some(p)) => {
            log::info!("{} ({from}) removed the pairing", p.host_name);
            PeerResponse::Ok { message: "pairing removed".into() }
        }
        Ok(None) => PeerResponse::NotPaired,
        Err(e) => PeerResponse::Error { message: format!("{e:#}") },
    }
}

/// Removes the pairing with a computer, and tells it if it is reachable.
pub fn unpair(engine: &Engine, host_id: &str) -> Result<String> {
    let Some(p) = engine.security.paired().into_iter().find(|p| p.host_id == host_id) else { bail!("not paired with that computer") };
    // Tell it first: it only listens to paired computers.
    let peer = engine.peers.list().into_iter().find(|s| s.hello.as_ref().map(|h| h.host_id.as_str()).or(s.known_id.as_deref()) == Some(host_id));
    let told = peer.is_some_and(|s| PeerClient::connect(&s.address, &engine.security.identity).and_then(|mut c| c.call_ok(&PeerRequest::Unpair)).is_ok());
    engine.security.remove(Some(host_id), None)?;
    log::info!("pairing with {} removed by the user", p.host_name);
    Ok(if told { format!("pairing with {} removed (on both computers)", p.host_name) } else { format!("pairing with {} removed here ({} was not reachable)", p.host_name, p.host_name) })
}
