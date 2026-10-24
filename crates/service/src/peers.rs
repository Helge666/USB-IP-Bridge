//! Other computers: discovery beacons, the control server for them, and one poller per
//! known computer (see usbipb_core::peer for the protocol).

use crate::engine::Engine;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use usbipb_core::peer::*;

/// A computer is offline when it has not answered for this long.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);
/// Computers found by discovery are forgotten after being offline this long.
const FORGET_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Offline computers are asked less often (a beacon announces them anyway).
const OFFLINE_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// How often the list of known computers is saved while they answer.
const SAVE_KNOWN_INTERVAL: Duration = Duration::from_secs(3600);
const BEACON_INTERVAL: Duration = Duration::from_secs(10);

struct Entry {
    status: PeerStatus,
    last_ok: Option<Instant>,
    /// Unix time of the last answer (also from earlier runs, see known.json).
    last_seen_unix: Option<u64>,
    added_unix: u64,
    saved: Option<Instant>,
}

/// A computer found earlier, remembered across restarts (known.json).
#[derive(serde::Serialize, serde::Deserialize)]
struct Known {
    address: String,
    host_id: String,
    host_name: String,
    last_seen: u64,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Entries for the same computer (several addresses: LAN, Tailscale, new DHCP lease).
fn group_key(s: &PeerStatus) -> String {
    s.hello.as_ref().map(|h| h.host_id.clone()).or_else(|| s.known_id.clone()).unwrap_or_else(|| s.address.clone())
}

#[derive(Default)]
pub struct Peers {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Peers {
    /// Known computers, one per host id (the most recently seen address wins).
    pub fn list(&self) -> Vec<PeerStatus> {
        let entries = self.entries.lock().unwrap();
        let mut groups: HashMap<String, Vec<&Entry>> = HashMap::new();
        for e in entries.values() {
            groups.entry(group_key(&e.status)).or_default().push(e);
        }
        let now = unix_now();
        let mut out: Vec<PeerStatus> = groups
            .into_values()
            .map(|group| {
                let best = group.iter().max_by_key(|e| (e.last_ok, e.last_seen_unix)).unwrap();
                let mut s = best.status.clone();
                s.online = best.last_ok.is_some_and(|t| t.elapsed() < ONLINE_TIMEOUT);
                s.last_seen_secs = group.iter().filter_map(|e| e.last_seen_unix).max().map(|t| now.saturating_sub(t));
                s.added = group.iter().any(|e| e.status.added);
                let configured = group.iter().any(|e| e.status.manual && !e.status.added);
                s.removable = s.added || (!s.online && !configured);
                s
            })
            .collect();
        out.sort_by_key(|s| s.name().to_lowercase());
        out
    }

    /// Adds an address; returns true if it is new (a poller must be started).
    fn add(&self, address: String, manual: bool) -> bool {
        let mut entries = self.entries.lock().unwrap();
        if let Some(e) = entries.get_mut(&address) {
            e.status.manual |= manual;
            return false;
        }
        log::info!("peer {address}: {}", if manual { "configured" } else { "found in the network" });
        let status = PeerStatus {
            address: address.clone(),
            hello: None,
            online: false,
            last_seen_secs: None,
            manual,
            added: false,
            removable: false,
            known_id: None,
            known_name: None,
            devices: Vec::new(),
            error: None,
            paired: false,
            fingerprint: None,
        };
        entries.insert(address, Entry { status, last_ok: None, last_seen_unix: None, added_unix: unix_now(), saved: None });
        true
    }

    /// Computers found in the network before (not configured ones).
    fn save_known(&self) {
        let known: Vec<Known> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| !e.status.manual)
            .filter_map(|e| {
                Some(Known {
                    address: e.status.address.clone(),
                    host_id: e.status.hello.as_ref().map(|h| h.host_id.clone()).or_else(|| e.status.known_id.clone())?,
                    host_name: e.status.name(),
                    last_seen: e.last_seen_unix?,
                })
            })
            .collect();
        if let Err(err) = serde_json::to_string_pretty(&known).map_err(anyhow::Error::from).and_then(|j| Ok(std::fs::write(known_path(), j)?)) {
            log::warn!("cannot save known computers: {err:#}");
        }
    }

    /// Addresses of computers added by hand (beacons go to them directly).
    fn manual_addresses(&self) -> Vec<String> {
        self.entries.lock().unwrap().values().filter(|e| e.status.manual).map(|e| e.status.address.clone()).collect()
    }

    fn save_added(&self) -> Result<()> {
        let added: Vec<String> = self.entries.lock().unwrap().values().filter(|e| e.status.added).map(|e| e.status.address.clone()).collect();
        std::fs::write(added_path(), serde_json::to_string_pretty(&added)?)?;
        Ok(())
    }
}

/// Computers added in the GUI.
fn added_path() -> std::path::PathBuf {
    crate::engine::state_dir().join("peers.json")
}

/// Computers found in the network before.
fn known_path() -> std::path::PathBuf {
    crate::engine::state_dir().join("known.json")
}

/// Adds a computer by name or address (GUI "Add computer"); remembered across restarts.
pub fn add_peer(engine: &Arc<Engine>, input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no name or address given");
    }
    let address = with_port(input, PEER_PORT);
    std::net::ToSocketAddrs::to_socket_addrs(&address).with_context(|| format!("cannot find {input} (check the name or address)"))?.next().with_context(|| format!("cannot find {input}"))?;
    let new = engine.peers.add(address.clone(), true);
    if let Some(e) = engine.peers.entries.lock().unwrap().get_mut(&address) {
        e.status.added = true;
    }
    engine.peers.save_added()?;
    if new {
        spawn_poller(Arc::downgrade(engine), address.clone());
    }
    Ok(format!("{address} added"))
}

/// Removes a computer from the list: the addresses added in the GUI, and – while it is
/// offline – the ones found in the network. Configured addresses stay.
pub fn remove_peer(engine: &Engine, address: &str) -> Result<String> {
    let mut entries = engine.peers.entries.lock().unwrap();
    let Some(key) = entries.get(address).map(|e| group_key(&e.status)) else { bail!("{address} is not in the list") };
    let online = entries.values().any(|e| group_key(&e.status) == key && e.last_ok.is_some_and(|t| t.elapsed() < ONLINE_TIMEOUT));
    let remove: Vec<String> = entries
        .values()
        .filter(|e| group_key(&e.status) == key && (e.status.added || (!e.status.manual && !online)))
        .map(|e| e.status.address.clone())
        .collect();
    if remove.is_empty() {
        bail!("{address} cannot be removed here (it is configured in config.toml, or it is online and would be found again)");
    }
    for a in &remove {
        entries.remove(a);
        log::info!("peer {a}: removed by the user");
    }
    drop(entries);
    engine.peers.save_added()?;
    engine.peers.save_known();
    Ok(format!("{} removed", remove.join(", ")))
}

fn with_port(host: &str, port: u16) -> String {
    // "name", "1.2.3.4", "name:port", "[v6]:port"
    if host.parse::<SocketAddr>().is_ok() || (host.contains(':') && !host.contains("::") && host.rsplit(':').next().is_some_and(|p| p.parse::<u16>().is_ok())) {
        host.to_string()
    } else {
        format!("{host}:{port}")
    }
}

/// `paired`: the asking computer is paired with this one.
pub fn hello(engine: &Engine, paired: bool) -> Hello {
    Hello {
        protocol: PROTOCOL_VERSION,
        host_id: engine.host_id.clone(),
        host_name: crate::engine::hostname(),
        version: usbipb_core::VERSION.to_string(),
        auth: "pairing".into(),
        paired,
        usbip_port: engine.config.usbip_port,
        can_export: cfg!(windows),
        can_import: cfg!(target_os = "linux"),
        system: Some(usbipb_host::system::info()),
    }
}

fn beacon(engine: &Engine, query: bool) -> Vec<u8> {
    let b = Beacon {
        app: BEACON_APP.into(),
        protocol: PROTOCOL_VERSION,
        host_id: engine.host_id.clone(),
        host_name: crate::engine::hostname(),
        port: PEER_PORT,
        query,
    };
    serde_json::to_vec(&b).unwrap_or_default()
}

/// Starts discovery, the control server and the pollers.
pub fn spawn(engine: Arc<Engine>) -> Result<()> {
    // Configured computers and the hosts of import rules are always known.
    let mut manual: Vec<String> = engine.config.peers.iter().map(|p| with_port(p, PEER_PORT)).collect();
    manual.extend(engine.config.import.iter().map(|r| with_port(&r.host, PEER_PORT)));
    for address in manual {
        if engine.peers.add(address.clone(), true) {
            spawn_poller(Arc::downgrade(&engine), address);
        }
    }
    // Computers found earlier: listed (offline) until they answer again.
    let known: Vec<Known> = std::fs::read_to_string(known_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for k in known {
        if unix_now().saturating_sub(k.last_seen) > FORGET_AFTER.as_secs() || k.host_id == engine.host_id {
            continue;
        }
        let new = engine.peers.add(k.address.clone(), false);
        if let Some(e) = engine.peers.entries.lock().unwrap().get_mut(&k.address) {
            e.status.known_id = Some(k.host_id);
            e.status.known_name = Some(k.host_name);
            e.last_seen_unix = Some(k.last_seen);
        }
        if new {
            spawn_poller(Arc::downgrade(&engine), k.address);
        }
    }
    let added: Vec<String> = std::fs::read_to_string(added_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    for address in added {
        let new = engine.peers.add(address.clone(), true);
        if let Some(e) = engine.peers.entries.lock().unwrap().get_mut(&address) {
            e.status.added = true;
        }
        if new {
            spawn_poller(Arc::downgrade(&engine), address);
        }
    }

    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, PEER_PORT)).with_context(|| format!("bind control port {PEER_PORT}"))?;
    listener.set_nonblocking(true)?;
    log::info!("control port TCP {PEER_PORT} (encrypted; paired computers only, this computer's key {})", engine.security.fingerprint());
    let e = engine.clone();
    std::thread::spawn(move || serve(e, listener));

    if engine.config.discovery {
        match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PEER_PORT)) {
            Ok(socket) => {
                socket.set_broadcast(true)?;
                socket.set_read_timeout(Some(Duration::from_secs(1)))?;
                let sender = socket.try_clone()?;
                let e = engine.clone();
                std::thread::spawn(move || discovery_receive(e, socket));
                let e = engine.clone();
                std::thread::spawn(move || discovery_send(e, sender));
                log::info!("discovery on UDP {PEER_PORT}");
            }
            Err(err) => log::warn!("discovery disabled: cannot bind UDP {PEER_PORT}: {err}"),
        }
    }
    Ok(())
}

fn discovery_send(engine: Arc<Engine>, socket: UdpSocket) {
    let broadcast = SocketAddr::from((Ipv4Addr::BROADCAST, PEER_PORT));
    let mut query = true; // the first beacon asks everybody to answer at once
    while !engine.stop.load(Ordering::Relaxed) {
        let msg = beacon(&engine, query);
        let _ = socket.send_to(&msg, broadcast);
        for p in engine.peers.manual_addresses() {
            if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&p) {
                for a in addrs.filter(|a| a.is_ipv4()) {
                    let _ = socket.send_to(&msg, a);
                }
            }
        }
        query = false;
        let end = Instant::now() + BEACON_INTERVAL;
        while Instant::now() < end && !engine.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn discovery_receive(engine: Arc<Engine>, socket: UdpSocket) {
    let mut buf = [0u8; 2048];
    while !engine.stop.load(Ordering::Relaxed) {
        let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
        let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) else { continue };
        if b.app != BEACON_APP || b.host_id == engine.host_id {
            continue;
        }
        let ip = match from.ip() {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => format!("[{v6}]"),
        };
        let address = format!("{ip}:{}", b.port);
        if engine.peers.add(address.clone(), false) {
            spawn_poller(Arc::downgrade(&engine), address);
        }
        if b.query {
            let _ = socket.send_to(&beacon(&engine, false), from);
        }
    }
}

/// Polls one computer until the service stops or the computer is forgotten.
fn spawn_poller(engine: Weak<Engine>, address: String) {
    std::thread::spawn(move || {
        let mut client: Option<PeerClient> = None;
        loop {
            let Some(engine) = engine.upgrade() else { return };
            if engine.stop.load(Ordering::Relaxed) {
                return;
            }
            let result = (|| -> Result<(Hello, Vec<PeerDevice>, bool, String)> {
                if client.is_none() {
                    client = Some(PeerClient::connect(&address, &engine.security.identity)?);
                }
                let c = client.as_mut().unwrap();
                let hello = match c.call(&PeerRequest::Hello)? {
                    PeerResponse::Hello(h) => h,
                    other => anyhow::bail!("unexpected answer {other:?}"),
                };
                let key = c.conn().remote_key().to_vec();
                engine.security.check_identity(&hello.host_id, &key)?;
                let paired = engine.security.is_paired(&key);
                // Devices only between paired computers (both sides have the other's key).
                let devices = if paired && hello.paired {
                    match c.call(&PeerRequest::Devices)? {
                        PeerResponse::Devices { devices } => devices,
                        PeerResponse::NotPaired => Vec::new(),
                        PeerResponse::Error { message } => anyhow::bail!(message),
                        other => anyhow::bail!("unexpected answer {other:?}"),
                    }
                } else {
                    Vec::new()
                };
                if paired && hello.paired {
                    if let Ok(a) = c.conn().tcp().peer_addr() {
                        engine.authorize(a.ip());
                    }
                }
                Ok((hello, devices, paired, usbipb_core::secure::fingerprint(&key)))
            })();
            let mut save_known = false;
            let online;
            {
                let mut entries = engine.peers.entries.lock().unwrap();
                let Some(e) = entries.get_mut(&address) else { return };
                match result {
                    Ok((hello, devices, paired, fingerprint)) => {
                        if hello.host_id == engine.host_id {
                            // that is us (e.g. our own address in `peers`)
                            entries.remove(&address);
                            return;
                        }
                        if e.last_ok.is_none_or(|t| t.elapsed() >= ONLINE_TIMEOUT) {
                            log::info!("peer {address}: {} online (v{})", hello.host_name, hello.version);
                        }
                        if !e.status.manual && (e.saved.is_none_or(|t| t.elapsed() > SAVE_KNOWN_INTERVAL) || e.status.known_id.as_deref() != Some(hello.host_id.as_str())) {
                            e.saved = Some(Instant::now());
                            save_known = true;
                        }
                        e.status.known_id = Some(hello.host_id.clone());
                        e.status.known_name = Some(hello.host_name.clone());
                        e.status.hello = Some(hello);
                        e.status.devices = devices;
                        e.status.error = None;
                        e.status.paired = paired;
                        e.status.fingerprint = Some(fingerprint);
                        e.last_ok = Some(Instant::now());
                        e.last_seen_unix = Some(unix_now());
                    }
                    Err(err) => {
                        client = None;
                        let was_online = e.last_ok.is_some_and(|t| t.elapsed() < ONLINE_TIMEOUT);
                        e.status.error = Some(format!("{err:#}"));
                        if was_online {
                            log::info!("peer {address}: no answer ({err:#})");
                        }
                        let since = e.last_seen_unix.unwrap_or(e.added_unix);
                        if !e.status.manual && unix_now().saturating_sub(since) > FORGET_AFTER.as_secs() {
                            log::info!("peer {address}: forgotten");
                            entries.remove(&address);
                            return;
                        }
                    }
                }
                online = entries.get(&address).is_some_and(|e| e.last_ok.is_some_and(|t| t.elapsed() < ONLINE_TIMEOUT));
            }
            if save_known {
                engine.peers.save_known();
            }
            let stop = engine.stop.clone();
            drop(engine);
            let end = Instant::now() + if online { POLL_INTERVAL } else { OFFLINE_POLL_INTERVAL };
            while Instant::now() < end && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    });
}

fn serve(engine: Arc<Engine>, listener: TcpListener) {
    while !engine.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let e = engine.clone();
                std::thread::spawn(move || {
                    if let Err(err) = handle(&e, stream) {
                        log::debug!("control connection {peer}: {err:#}");
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                log::error!("control port: {e}");
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn answer(r: Result<String>) -> PeerResponse {
    match r {
        Ok(message) => PeerResponse::Ok { message },
        Err(e) => PeerResponse::Error { message: format!("{e:#}") },
    }
}

fn handle(engine: &Engine, stream: TcpStream) -> Result<()> {
    let ip = match stream.peer_addr()?.ip() {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };
    let from = ip.to_string();
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let secure = usbipb_core::secure::SecureStream::accept(stream, &engine.security.identity)?;
    let key = secure.remote_key().to_vec();
    let mut reader = BufReader::new(secure);
    let mut pending = None;
    let mut line = String::new();
    while !engine.stop.load(Ordering::Relaxed) {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let paired = engine.security.is_paired(&key);
        if paired {
            engine.authorize(ip);
        }
        let response = match serde_json::from_str::<PeerRequest>(&line) {
            Ok(PeerRequest::Hello) => PeerResponse::Hello(hello(engine, paired)),
            Ok(PeerRequest::PairStart { host_id, host_name, pake }) => crate::security::pair_incoming(engine, reader.get_ref(), &from, host_id, host_name, &pake, &mut pending),
            Ok(PeerRequest::PairConfirm { confirm }) => crate::security::pair_confirm(engine, reader.get_ref(), &confirm, &mut pending),
            Ok(_) if !paired => PeerResponse::NotPaired,
            Ok(PeerRequest::Unpair) => crate::security::unpaired_by_peer(engine, &key, &from),
            Ok(PeerRequest::Devices) => PeerResponse::Devices { devices: engine.peer_devices() },
            // The other computer gives us one of its devices, or wants it back.
            Ok(PeerRequest::Attach { device, usbip_port }) => {
                log::info!("{from} offers {:04x}:{:04x}", device.vendor_id, device.product_id);
                answer(engine.use_device(&from, usbip_port, device, true))
            }
            Ok(PeerRequest::Release { device }) => {
                log::info!("{from} wants {:04x}:{:04x} back", device.vendor_id, device.product_id);
                answer(engine.use_device(&from, 0, device, false))
            }
            Err(e) => PeerResponse::Error { message: format!("bad request: {e}") },
        };
        let mut out = serde_json::to_string(&response)?;
        out.push('\n');
        reader.get_mut().write_all(out.as_bytes())?;
    }
    Ok(())
}
