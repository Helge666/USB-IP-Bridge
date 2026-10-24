//! Import client: attaches a remote device through the kernel's vhci_hcd and keeps it
//! attached – if the connection breaks (network, server restart, device replugged on the
//! server) it reconnects automatically.
//!
//! This talks to the kernel directly (like the `usbip attach` tool): OP_REQ_IMPORT over
//! TCP, then the connected socket is handed to vhci_hcd via sysfs. Needs root.

use crate::net::{configure_keepalive, connect};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usbip_protocol::*;
use usbipb_core::api::{ImportState, ImportStatus};
use usbipb_core::classes::{classify, parse_names, DeviceClass};
use usbipb_core::config::ImportRule;
use usbipb_core::profiles::{Profiles, Reconnect};

const VHCI: &str = "/sys/devices/platform/vhci_hcd.0";
const RECORDS: &str = "/run/usb-ip-bridge";
/// `enum usbip_device_status` in the kernel.
const VDEV_ST_NULL: u32 = 4;
/// Attached, device not yet addressed by the USB core.
const VDEV_ST_NOTASSIGNED: u32 = 5;
const VDEV_ST_USED: u32 = 6;

pub fn rule_name(rule: &ImportRule) -> String {
    let d = &rule.device;
    let serial = d.serial.as_deref().map(|s| format!(" serial {s}")).unwrap_or_default();
    format!("{:04x}:{:04x}{serial} from {}:{}", d.vendor_id, d.product_id, rule.host, rule.port)
}

#[derive(Debug, Clone)]
struct PortStatus {
    hub_hs: bool,
    port: u32,
    sta: u32,
}

fn vhci_ports() -> Result<Vec<PortStatus>> {
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(VHCI)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("status")))
        .collect();
    files.sort();
    for f in files {
        for line in fs::read_to_string(&f)?.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 3 {
                continue;
            }
            out.push(PortStatus { hub_hs: cols[0] == "hs", port: cols[1].parse()?, sta: cols[2].parse()? });
        }
    }
    Ok(out)
}

fn ensure_vhci() -> Result<()> {
    if Path::new(VHCI).exists() {
        return Ok(());
    }
    let ok = std::process::Command::new("modprobe").arg("vhci-hcd").status().map(|s| s.success()).unwrap_or(false);
    if !ok || !Path::new(VHCI).exists() {
        bail!("kernel module vhci-hcd is not available");
    }
    Ok(())
}

/// Reads OP_REP_DEVLIST and returns the bus id of the device matching the rule.
/// The remote device record and its classes (as detected by the remote host, else from
/// the interface list).
fn find_remote_device(rule: &ImportRule) -> Result<(UsbDevice, Vec<DeviceClass>)> {
    let mut s = connect(&rule.host, rule.port)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    OpHeader::new(OP_REQ_DEVLIST, 0).write(&mut s)?;
    let h = OpHeader::read(&mut s)?;
    if h.code != OP_REP_DEVLIST || h.status != 0 {
        bail!("server refused the device list (status {})", h.status);
    }
    let mut n = [0u8; 4];
    s.read_exact(&mut n)?;
    let count = u32::from_be_bytes(n);
    let mut candidates = Vec::new();
    for _ in 0..count {
        let mut raw = [0u8; USB_DEVICE_SIZE];
        s.read_exact(&mut raw)?;
        let d = UsbDevice::from_bytes(&raw);
        let mut ifaces = vec![0u8; d.num_interfaces as usize * 4];
        s.read_exact(&mut ifaces)?;
        let classes = match crate::net::path_field(&d.path, "classes") {
            Some(names) => parse_names(&names),
            None => classify(d.device_class, &ifaces.chunks(4).map(|i| (i[0], i[1], i[2])).collect::<Vec<_>>()),
        };
        candidates.push((d, classes));
    }
    let want = &rule.device;
    candidates
        .into_iter()
        .find(|(d, _)| {
            let serial = crate::net::path_field(&d.path, "serial");
            let serial = serial.as_deref();
            d.id_vendor == want.vendor_id
                && d.id_product == want.product_id
                && match (&want.serial, serial) {
                    (Some(w), Some(s)) => w.eq_ignore_ascii_case(s),
                    (Some(_), None) => false,
                    (None, _) => true,
                }
        })
        .ok_or_else(|| anyhow!("device not offered by {} (not plugged in, not shared, in use, or the two computers are not paired)", rule.host))
}

/// Imports the device and hands the socket to vhci_hcd. Returns the vhci port.
fn attach(rule: &ImportRule) -> Result<(u32, String)> {
    ensure_vhci()?;
    let (remote, _) = find_remote_device(rule)?;
    let mut s: TcpStream = connect(&rule.host, rule.port)?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?; // capturing the device on Windows takes a moment
    let mut req = Vec::new();
    OpHeader::new(OP_REQ_IMPORT, 0).write(&mut req)?;
    let mut busid = [0u8; 32];
    busid[..remote.busid.len().min(31)].copy_from_slice(&remote.busid.as_bytes()[..remote.busid.len().min(31)]);
    req.extend_from_slice(&busid);
    s.write_all(&req)?;
    let h = OpHeader::read(&mut s)?;
    if h.code != OP_REP_IMPORT || h.status != op_status::OK {
        match h.status {
            op_status::DEV_BUSY => bail!("{} is in use on {} (another client, or a program there holds it)", remote.busid, rule.host),
            op_status::NODEV => bail!("{} is no longer offered by {}", remote.busid, rule.host),
            s => bail!("{} refused to attach {} (status {s})", rule.host, remote.busid),
        }
    }
    let mut raw = [0u8; USB_DEVICE_SIZE];
    s.read_exact(&mut raw)?;
    let dev = UsbDevice::from_bytes(&raw);
    s.set_read_timeout(None)?;
    s.set_nodelay(true)?;
    configure_keepalive(&s)?;

    let high_or_lower = !matches!(dev.speed, Speed::Super | Speed::SuperPlus);
    let port = vhci_ports()?
        .into_iter()
        .find(|p| p.hub_hs == high_or_lower && p.sta == VDEV_ST_NULL)
        .ok_or_else(|| anyhow!("no free vhci port"))?
        .port;
    let devid = (dev.busnum << 16) | dev.devnum;
    let cmd = format!("{} {} {} {}", port, s.as_raw_fd(), devid, dev.speed as u32);
    fs::write(Path::new(VHCI).join("attach"), &cmd).with_context(|| format!("vhci attach '{cmd}' (needs root)"))?;
    // vhci_hcd holds its own reference to the socket now.
    drop(s);
    let _ = fs::create_dir_all(RECORDS);
    let _ = fs::write(Path::new(RECORDS).join(format!("port{port}")), rule_name(rule));
    Ok((port, dev.busid))
}

fn detach(port: u32) {
    let _ = fs::write(Path::new(VHCI).join("detach"), port.to_string());
    let _ = fs::remove_file(Path::new(RECORDS).join(format!("port{port}")));
}

fn port_in_use(port: u32) -> bool {
    vhci_ports()
        .map(|ps| ps.iter().any(|p| p.port == port && matches!(p.sta, VDEV_ST_NOTASSIGNED | VDEV_ST_USED)))
        .unwrap_or(false)
}

/// A port attached by an earlier run of the service for this rule.
fn adopt(rule: &ImportRule) -> Option<u32> {
    let name = rule_name(rule);
    for e in fs::read_dir(RECORDS).ok()?.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let port: u32 = file.strip_prefix("port")?.parse().ok()?;
        if fs::read_to_string(e.path()).ok()? == name && port_in_use(port) {
            return Some(port);
        }
    }
    None
}

fn set(status: &Mutex<ImportStatus>, state: ImportState, error: Option<String>, changed: &mut Instant) {
    let mut s = status.lock().unwrap();
    if s.state != state {
        *changed = Instant::now();
    }
    s.state = state;
    if error.is_some() || matches!(state, ImportState::Attached | ImportState::Idle) {
        s.last_error = error;
    }
    s.since_secs = changed.elapsed().as_secs();
}

/// Keeps one import rule attached while `wanted` is set, until `stop` is set.
///
/// On service stop the device stays attached (the kernel keeps the connection); the
/// next start adopts it. Releasing happens only when the user no longer wants it.
/// Names for the GUI: from the server's device list (also while idle, refreshed now and
/// then) or, while attached, from the device itself.
/// Updates name, remote host name, classes and profile warnings in the status.
fn refresh_names(rule: &ImportRule, profiles: &Profiles, status: &Mutex<ImportStatus>, attached: bool) {
    let mut classes = None;
    if attached {
        let local = crate::linux::devices::list().into_iter().find(|d| {
            d.state == usbipb_core::model::DeviceState::Attached && d.vendor_id == rule.device.vendor_id && d.product_id == rule.device.product_id
        });
        if let Some(d) = local {
            status.lock().unwrap().device_name = Some(d.display_name());
            classes = Some(d.classes);
        }
    } else if let Ok((d, c)) = find_remote_device(rule) {
        let mut s = status.lock().unwrap();
        if let Some(n) = crate::net::path_field(&d.path, "name") {
            if s.device_name.is_none() {
                s.device_name = Some(n);
            }
        }
        if let Some(h) = crate::net::path_field(&d.path, "host") {
            s.remote_name = Some(h);
        }
        classes = Some(c);
    }
    if let Some(c) = classes.filter(|c| !c.is_empty()) {
        let p = profiles.resolve(rule.device.vendor_id, rule.device.product_id, &c);
        let mut s = status.lock().unwrap();
        if let Some(n) = p.name {
            s.device_name = Some(n);
        }
        s.classes = p.classes;
        s.warnings = p.warnings;
    }
}

/// Reconnect policy from the device profile (storage: ask, i.e. never silently).
fn reconnect_policy(rule: &ImportRule, profiles: &Profiles, status: &Mutex<ImportStatus>) -> Reconnect {
    let classes = status.lock().unwrap().classes.clone();
    profiles.resolve(rule.device.vendor_id, rule.device.product_id, &classes).reconnect
}

pub fn run(rule: ImportRule, status: Arc<Mutex<ImportStatus>>, wanted: Arc<AtomicBool>, stop: Arc<AtomicBool>) {
    let name = rule_name(&rule);
    let profiles = Profiles::load();
    let mut last_refresh: Option<Instant> = None;
    let mut changed = Instant::now();
    let mut backoff = Duration::from_secs(1);
    let mut port = adopt(&rule);
    if let Some(p) = port {
        log::info!("{name}: already attached on vhci port {p}, monitoring");
    }
    while !stop.load(Ordering::Relaxed) {
        status.lock().unwrap().wanted = wanted.load(Ordering::Relaxed);
        if !wanted.load(Ordering::Relaxed) {
            if let Some(p) = port {
                // Never pull a mounted file system away: unmount first, keep the
                // device while something still uses it.
                let dir = crate::linux::ready::attached_device_dir(rule.device.vendor_id, rule.device.product_id);
                if let Some(Err(e)) = dir.map(|d| crate::linux::ready::unmount_all(&d)) {
                    let mut s = status.lock().unwrap();
                    s.since_secs = changed.elapsed().as_secs();
                    if s.last_error.as_deref() != Some(e.as_str()) {
                        log::warn!("{name}: not giving back yet: {e}");
                        s.last_error = Some(e);
                    }
                    drop(s);
                    sleep_unless(&stop, Duration::from_secs(2));
                    continue;
                }
                port = None;
                log::info!("{name}: released by the user, detaching vhci port {p}");
                detach(p);
            }
            // "stopped" (connection lost, no automatic reconnect) stays visible until
            // the user attaches again
            if status.lock().unwrap().state != ImportState::Disabled {
                set(&status, ImportState::Idle, None, &mut changed);
            }
            if last_refresh.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                refresh_names(&rule, &profiles, &status, false);
                last_refresh = Some(Instant::now());
            }
            sleep_unless(&stop, Duration::from_millis(300));
            continue;
        }
        let p = match port {
            Some(p) => p,
            None => {
                set(&status, ImportState::Connecting, None, &mut changed);
                match attach(&rule) {
                    Ok((p, busid)) => {
                        log::info!("{name}: attached remote {busid} on vhci port {p}");
                        let mut s = status.lock().unwrap();
                        s.attach_count += 1;
                        s.bus_id = Some(busid);
                        drop(s);
                        port = Some(p);
                        backoff = Duration::from_secs(1);
                        p
                    }
                    Err(e) => {
                        log::warn!("{name}: {e:#}");
                        set(&status, ImportState::Waiting, Some(format!("{e:#}")), &mut changed);
                        sleep_unless(&stop, backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(10));
                        continue;
                    }
                }
            }
        };
        set(&status, ImportState::Attached, None, &mut changed);
        std::thread::sleep(Duration::from_millis(500));
        refresh_names(&rule, &profiles, &status, true);
        // Monitor the port; vhci_hcd frees it when the connection drops.
        while !stop.load(Ordering::Relaxed) && wanted.load(Ordering::Relaxed) && port_in_use(p) {
            std::thread::sleep(Duration::from_millis(500));
            status.lock().unwrap().since_secs = changed.elapsed().as_secs();
        }
        if stop.load(Ordering::Relaxed) || !wanted.load(Ordering::Relaxed) {
            continue;
        }
        log::warn!("{name}: connection lost");
        let _ = fs::remove_file(Path::new(RECORDS).join(format!("port{p}")));
        port = None;
        let ask = reconnect_policy(&rule, &profiles, &status) == Reconnect::Ask;
        if rule.auto_reconnect && !ask {
            set(&status, ImportState::Waiting, Some("connection lost, reconnecting".into()), &mut changed);
            sleep_unless(&stop, Duration::from_secs(1));
        } else {
            wanted.store(false, Ordering::Relaxed);
            let why = if ask {
                "connection lost; not reattached automatically (data may have been lost) – check the device, then attach again"
            } else {
                "connection lost"
            };
            log::warn!("{name}: {why}");
            set(&status, ImportState::Disabled, Some(why.into()), &mut changed);
        }
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
}
