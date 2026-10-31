//! Import client: attaches a remote device and keeps it attached – if the connection
//! breaks (network, server restart, device replugged on the server) it reconnects
//! automatically, as the device profile allows.
//!
//! The platform attaches: Linux through the kernel's vhci_hcd, Windows through the
//! usbip-win2 driver. This part finds the device, follows the user's wish and watches.

use anyhow::{anyhow, bail, Result};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usbip_protocol::*;
use usbipb_core::api::{ImportState, ImportStatus};
use usbipb_core::classes::{classify, parse_names, DeviceClass};
use usbipb_core::config::ImportRule;
use usbipb_core::model::DeviceState;
use usbipb_core::profiles::{Profiles, Reconnect};

#[cfg(target_os = "linux")]
use crate::linux::import as backend;
#[cfg(windows)]
use crate::windows::import as backend;

pub fn rule_name(rule: &ImportRule) -> String {
    let d = &rule.device;
    let serial = d.serial.as_deref().map(|s| format!(" serial {s}")).unwrap_or_default();
    format!("{:04x}:{:04x}{serial} from {}:{}", d.vendor_id, d.product_id, rule.host, rule.port)
}

/// The remote device record matching the rule (from OP_REP_DEVLIST) and its classes (as
/// detected by the remote host, else from the interface list).
pub fn find_remote_device(rule: &ImportRule) -> Result<(UsbDevice, Vec<DeviceClass>)> {
    let mut s = crate::net::connect(&rule.host, rule.port)?;
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

/// Names for the GUI: from the server's device list (also while idle, refreshed now and
/// then) or, while attached, from the device itself.
/// Updates name, remote host name, classes and profile warnings in the status.
fn refresh_names(rule: &ImportRule, profiles: &Profiles, status: &Mutex<ImportStatus>, attached: bool) {
    let mut classes = None;
    if attached {
        let local = crate::list_devices()
            .into_iter()
            .find(|d| d.state == DeviceState::Attached && d.vendor_id == rule.device.vendor_id && d.product_id == rule.device.product_id);
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

/// Keeps one import rule attached while `wanted` is set, until `stop` is set.
///
/// On service stop the device stays attached (the driver keeps the connection); the
/// next start adopts it. Releasing happens only when the user no longer wants it.
pub fn run(rule: ImportRule, status: Arc<Mutex<ImportStatus>>, wanted: Arc<AtomicBool>, stop: Arc<AtomicBool>) {
    let name = rule_name(&rule);
    let profiles = Profiles::load();
    let mut last_refresh: Option<Instant> = None;
    let mut changed = Instant::now();
    let mut backoff = Duration::from_secs(1);
    // logged once while it stays the same (a device that is not plugged in waits for hours)
    let mut last_failure: Option<String> = None;
    // (port, remote bus id) while attached
    let mut port = backend::adopt(&rule);
    if let Some((p, busid)) = &port {
        log::info!("{name}: already attached on port {p}, monitoring");
        if !busid.is_empty() {
            status.lock().unwrap().bus_id = Some(busid.clone());
        }
    }
    while !stop.load(Ordering::Relaxed) {
        status.lock().unwrap().wanted = wanted.load(Ordering::Relaxed);
        if !wanted.load(Ordering::Relaxed) {
            if let Some((p, busid)) = &port {
                // Never pull a file system away: unmount (eject) first, keep the device
                // while something still uses it.
                if let Err(e) = backend::before_release(&rule) {
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
                log::info!("{name}: released by the user, detaching port {p}");
                backend::detach(&rule, *p, busid);
                port = None;
            }
            // "stopped" (connection lost, no automatic reconnect) stays visible until
            // the user attaches again
            if status.lock().unwrap().state != ImportState::Disabled {
                set(&status, ImportState::Idle, None, &mut changed);
            }
            last_failure = None;
            if last_refresh.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                refresh_names(&rule, &profiles, &status, false);
                last_refresh = Some(Instant::now());
            }
            sleep_unless(&stop, Duration::from_millis(300));
            continue;
        }
        let (p, busid) = match port.clone() {
            Some(a) => a,
            None => {
                set(&status, ImportState::Connecting, None, &mut changed);
                match backend::prepare().and_then(|_| find_remote_device(&rule)).and_then(|(remote, _)| backend::attach(&rule, &remote)) {
                    Ok((p, busid)) => {
                        log::info!("{name}: attached remote {busid} on port {p}");
                        let mut s = status.lock().unwrap();
                        s.attach_count += 1;
                        s.bus_id = Some(busid.clone());
                        drop(s);
                        port = Some((p, busid.clone()));
                        backoff = Duration::from_secs(1);
                        last_failure = None;
                        (p, busid)
                    }
                    Err(e) => {
                        let e = format!("{e:#}");
                        if last_failure.as_deref() != Some(e.as_str()) {
                            log::warn!("{name}: {e}");
                            last_failure = Some(e.clone());
                        }
                        set(&status, ImportState::Waiting, Some(e), &mut changed);
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
        // Monitor the port; the driver frees it when the connection drops.
        while !stop.load(Ordering::Relaxed) && wanted.load(Ordering::Relaxed) && backend::port_in_use(p) {
            sleep_unless(&stop, backend::POLL);
            status.lock().unwrap().since_secs = changed.elapsed().as_secs();
        }
        if stop.load(Ordering::Relaxed) || !wanted.load(Ordering::Relaxed) {
            continue;
        }
        log::warn!("{name}: connection lost");
        backend::lost(&rule, p, &busid);
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
