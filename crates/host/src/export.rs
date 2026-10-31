//! Export server: shares configured devices over USB/IP.
//!
//! Runs until `stop` is set; accepts any number of clients over time. A device is taken
//! from this computer only when a client imports it (Windows: VBoxUSB captures it,
//! Linux: the usbip-host driver takes it) and handed back when the session ends, so the
//! device works locally whenever nobody uses it remotely.

#[cfg(target_os = "linux")]
use crate::linux::export::{wire_details, Lease};
use crate::net::configure_keepalive;
#[cfg(windows)]
use crate::windows::lease::{wire_details, Lease};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::BufReader;
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usbip_protocol::*;
use usbipb_core::api::ExportStatus;
use usbipb_core::config::{Config, ExportRule};
use usbipb_core::model::{DeviceState, UsbDevice};
use usbipb_core::profiles::Profiles;

/// Shared state of the export server, read by the local API.
#[derive(Default)]
pub struct ExportState {
    /// bus id -> (client address, stream used to end the session, the device as it looked
    /// before capture - while captured, Windows only shows "VirtualBox USB")
    active: Mutex<HashMap<String, (String, TcpStream, UsbDevice)>>,
    /// device serial or bus id -> (problem, when, client) of the last failed capture
    problems: Mutex<HashMap<String, (String, Instant, String)>>,
    /// The configuration in effect, if it changed since `run` started (shares set in the GUI).
    current: Mutex<Option<Arc<Config>>>,
    /// Addresses of paired computers (proven on the encrypted control channel), and when.
    authorized: Mutex<HashMap<IpAddr, Instant>>,
    /// Any computer may import (config `require_pairing = false`).
    allow_unpaired: AtomicBool,
    /// When a refusal for an unpaired address was last logged (they retry every few seconds).
    refusals_logged: Mutex<HashMap<IpAddr, Instant>>,
}

/// How long a paired computer's address stays allowed after its last control contact
/// (paired computers talk every few seconds).
const AUTHORIZED_FOR: Duration = Duration::from_secs(60);

fn problem_key(d: &UsbDevice) -> String {
    d.serial.clone().unwrap_or_else(|| d.bus_id.clone())
}

fn profiles() -> &'static Profiles {
    static PROFILES: std::sync::OnceLock<Profiles> = std::sync::OnceLock::new();
    PROFILES.get_or_init(Profiles::load)
}

impl ExportState {
    /// Replaces the configuration for new connections (running sessions continue).
    pub fn set_config(&self, config: Arc<Config>) {
        *self.current.lock().unwrap() = Some(config);
    }

    /// The computer at `ip` proved to be paired: it may import shared devices for a while.
    pub fn authorize(&self, ip: IpAddr) {
        self.authorized.lock().unwrap().insert(ip, Instant::now());
    }

    pub fn set_allow_unpaired(&self, allow: bool) {
        self.allow_unpaired.store(allow, Ordering::Relaxed);
    }

    fn is_authorized(&self, ip: IpAddr) -> bool {
        self.allow_unpaired.load(Ordering::Relaxed) || self.authorized.lock().unwrap().get(&ip).is_some_and(|t| t.elapsed() < AUTHORIZED_FOR)
    }

    /// Devices currently used by another computer: (bus id, client address, device).
    pub fn lent(&self) -> Vec<(String, String, UsbDevice)> {
        self.active.lock().unwrap().iter().map(|(bus, (client, _, d))| (bus.clone(), client.clone(), d.clone())).collect()
    }

    pub fn status(&self, config: &Config) -> Vec<ExportStatus> {
        let devices = crate::list_devices();
        let active = self.active.lock().unwrap();
        config
            .export
            .iter()
            .map(|rule| {
                let present = devices.iter().find(|d| d.state == DeviceState::Local && rule.device.matches(d)).cloned().map(|mut d| {
                    d.state = if config.is_blocked(&d) { DeviceState::Blocked } else { DeviceState::Shared };
                    d
                });
                let session = active.values().find(|(_, _, d)| rule.device.matches(d));
                let client = session.map(|(c, _, _)| c.clone());
                // A lent device is still listed on Linux (bound to usbip-host).
                let device = session
                    .map(|(_, _, d)| {
                        let mut d = d.clone();
                        d.state = DeviceState::InUseRemotely;
                        d
                    })
                    .or(present);
                let note = match (&device, &client) {
                    (None, _) => Some("not plugged in".into()),
                    _ => None,
                };
                let problems = self.problems.lock().unwrap();
                // Problems older than 30 s are stale: the client has given up or succeeded.
                let problem = device
                    .as_ref()
                    .and_then(|d| problems.get(&problem_key(d)))
                    .filter(|(_, t, _)| t.elapsed() < Duration::from_secs(30));
                // configured blockers plus the ones the device profile knows
                let mut blocking_processes = rule.blocking_processes.clone();
                if let Some(d) = &device {
                    for p in profiles().resolve(d.vendor_id, d.product_id, &d.classes).blocking_processes {
                        if !blocking_processes.iter().any(|b| b.eq_ignore_ascii_case(&p)) {
                            blocking_processes.push(p);
                        }
                    }
                }
                let name_of = |c: Option<&String>| c.and_then(|c| c.parse().ok()).and_then(crate::net::host_name);
                ExportStatus {
                    present: device.is_some(),
                    device,
                    rule: rule_name(rule),
                    client_name: name_of(client.as_ref()),
                    problem_client_name: name_of(problem.map(|(_, _, c)| c)),
                    client,
                    note,
                    problem: problem.map(|(p, _, _)| p.clone()),
                    problem_age_secs: problem.map(|(_, t, _)| t.elapsed().as_secs()),
                    problem_client: problem.map(|(_, _, c)| c.clone()),
                    blocking_processes,
                }
            })
            .collect()
    }
}

fn rule_name(rule: &ExportRule) -> String {
    let d = &rule.device;
    match &d.serial {
        Some(s) => format!("{:04x}:{:04x} serial {s}", d.vendor_id, d.product_id),
        None => format!("{:04x}:{:04x}", d.vendor_id, d.product_id),
    }
}

/// Devices the given client may import right now (never devices this computer only
/// uses from another one).
fn exportable(config: &Config, peer: IpAddr) -> Vec<UsbDevice> {
    crate::list_devices()
        .into_iter()
        .filter(|d| d.state == DeviceState::Local && !config.is_blocked(d))
        .filter(|d| {
            config.export.iter().any(|r| {
                r.device.matches(d) && (r.allowed_clients.is_empty() || r.allowed_clients.iter().any(|c| c.parse::<IpAddr>().ok() == Some(peer)))
            })
        })
        .collect()
}

/// The device record for the device list, with its interfaces (first alternate setting).
fn wire_device(d: &UsbDevice) -> (UsbDevice2, Vec<InterfaceInfo>) {
    let (busnum, devnum, speed, interfaces) = wire_details(d);
    let record = UsbDevice2 {
        // The free-text path carries serial, device name, host name and the detected
        // classes (they include what only Windows knows, e.g. the driver's setup class).
        path: crate::net::encode_path(&[
            ("serial", d.serial.as_deref().unwrap_or("")),
            ("name", &d.display_name()),
            ("host", &crate::system::host_name()),
            ("classes", &usbipb_core::classes::join_names(&d.classes)),
        ]),
        busid: d.bus_id.clone(),
        busnum,
        devnum,
        speed,
        id_vendor: d.vendor_id,
        id_product: d.product_id,
        bcd_device: 0,
        device_class: 0,
        device_subclass: 0,
        device_protocol: 0,
        configuration_value: 1,
        num_configurations: 1,
        num_interfaces: interfaces.len() as u8,
    };
    (record, interfaces)
}

type UsbDevice2 = usbip_protocol::UsbDevice;

pub fn run(config: Arc<Config>, state: Arc<ExportState>, stop: Arc<AtomicBool>) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", config.usbip_port)).with_context(|| format!("bind TCP {}", config.usbip_port))?;
    listener.set_nonblocking(true)?;
    #[cfg(target_os = "linux")]
    crate::linux::export::release_leftovers();
    let rules = state.current.lock().unwrap().as_ref().map_or(config.export.len(), |c| c.export.len());
    log::info!("export server listening on TCP {} ({rules} shared device rule(s))", config.usbip_port);
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let config = state.current.lock().unwrap().clone().unwrap_or_else(|| config.clone());
                let state = state.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, peer.ip(), &config, &state) {
                        log::warn!("{peer}: {e:#}");
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(e.into()),
        }
    }
    // End running sessions; their threads give the devices back to this computer.
    for (bus, (client, s, _)) in state.active.lock().unwrap().iter() {
        log::info!("stopping: ending session {bus} with {client}");
        let _ = s.shutdown(Shutdown::Both);
    }
    let t = Instant::now();
    while !state.active.lock().unwrap().is_empty() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, peer: IpAddr, config: &Config, state: &ExportState) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    configure_keepalive(&stream)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let header = OpHeader::read(&mut reader)?;
    let authorized = state.is_authorized(peer);
    if !authorized {
        let mut logged = state.refusals_logged.lock().unwrap();
        if logged.get(&peer).is_none_or(|t| t.elapsed() > Duration::from_secs(300)) {
            logged.insert(peer, Instant::now());
            log::warn!("{peer}: not a paired computer (or it has not contacted this one in the last {} s) – no devices", AUTHORIZED_FOR.as_secs());
        }
    }
    let exportable = |config: &Config, peer: IpAddr| if authorized { exportable(config, peer) } else { Vec::new() };
    match header.code {
        OP_REQ_DEVLIST => {
            let devices: Vec<_> = exportable(config, peer).iter().map(wire_device).collect();
            log::debug!("{peer}: device list ({} device(s))", devices.len());
            let mut s = stream;
            write_devlist_reply(&mut s, &devices)?;
            Ok(())
        }
        OP_REQ_IMPORT => {
            let busid = read_import_busid(&mut reader)?;
            let mut s = stream.try_clone()?;
            let Some(device) = exportable(config, peer).into_iter().find(|d| d.bus_id == busid) else {
                write_import_reply(&mut s, None)?;
                bail!("import of {busid} refused (not shared, blocked or not allowed for {peer})");
            };
            if state.active.lock().unwrap().contains_key(&busid) {
                write_import_error(&mut s, op_status::DEV_BUSY)?;
                bail!("{busid} is already in use by another client");
            }
            let t0 = Instant::now();
            let lease = match Lease::capture(&device) {
                Ok(l) => l,
                Err(e) => {
                    let text = format!("{e:#}");
                    let mut problems = state.problems.lock().unwrap();
                    // the client tries again every few seconds: log the same problem once
                    let repeated = problems.get(&problem_key(&device)).is_some_and(|(p, t, _)| *p == text && t.elapsed() < Duration::from_secs(30));
                    problems.insert(problem_key(&device), (text, Instant::now(), peer.to_string()));
                    drop(problems);
                    // "busy" tells the client that something on this host holds the device.
                    write_import_error(&mut s, op_status::DEV_BUSY)?;
                    if repeated {
                        return Ok(());
                    }
                    return Err(e.context(format!("capture {}", device.display_name())));
                }
            };
            state.problems.lock().unwrap().remove(&problem_key(&device));
            let (mut info, _) = wire_device(&device);
            lease.complete(&mut info)?;
            // The 10 s timeout only guards the operation phase. On Windows a socket option
            // set before try_clone() stays on each duplicated handle, so clear it on the
            // handle the session reads from, too - otherwise an idle device (no URBs for 10 s)
            // would end the session. (On Linux the kernel would obey it, too.)
            stream.set_read_timeout(None)?;
            reader.get_ref().set_read_timeout(None)?;
            lease.hand_over(&stream)?;
            write_import_reply(&mut s, Some(&info))?;
            log::info!("{peer}: attached {} ({busid}, {:?}) after {} ms", device.display_name(), info.speed, t0.elapsed().as_millis());
            state.active.lock().unwrap().insert(busid.clone(), (peer.to_string(), stream.try_clone()?, device.clone()));
            let result = lease.serve(info.speed, stream, reader);
            state.active.lock().unwrap().remove(&busid);
            log::info!("{peer}: session for {busid} ended ({})", match &result {
                Ok(()) => "client detached".to_string(),
                Err(e) => format!("{e:#}"),
            });
            Ok(())
        }
        other => bail!("unexpected USB/IP operation {other:#06x}"),
    }
}
