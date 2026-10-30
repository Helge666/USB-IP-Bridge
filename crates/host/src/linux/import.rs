//! Import backend for Linux: attaches a remote device through the kernel's vhci_hcd.
//!
//! This talks to the kernel directly (like the `usbip attach` tool): OP_REQ_IMPORT over
//! TCP, then the connected socket is handed to vhci_hcd via sysfs. Needs root.

use crate::import::rule_name;
use crate::net::{configure_keepalive, connect};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;
use usbip_protocol::*;
use usbipb_core::config::ImportRule;

const VHCI: &str = "/sys/devices/platform/vhci_hcd.0";
const RECORDS: &str = "/run/usb-ip-bridge";
/// `enum usbip_device_status` in the kernel.
const VDEV_ST_NULL: u32 = 4;
/// Attached, device not yet addressed by the USB core.
const VDEV_ST_NOTASSIGNED: u32 = 5;
const VDEV_ST_USED: u32 = 6;

/// How often an attached port is checked.
pub const POLL: Duration = Duration::from_millis(500);

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

/// Loads vhci_hcd if needed.
pub fn prepare() -> Result<()> {
    if Path::new(VHCI).exists() {
        return Ok(());
    }
    let ok = std::process::Command::new("modprobe").arg("vhci-hcd").status().map(|s| s.success()).unwrap_or(false);
    if !ok || !Path::new(VHCI).exists() {
        bail!("kernel module vhci-hcd is not available");
    }
    Ok(())
}

/// Imports the device and hands the socket to vhci_hcd. Returns the vhci port and the
/// remote bus id.
pub fn attach(rule: &ImportRule, remote: &UsbDevice) -> Result<(u32, String)> {
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
    let _ = fs::write(Path::new(RECORDS).join(format!("port{port}")), format!("{}\n{}", rule_name(rule), remote.busid));
    Ok((port, dev.busid))
}

pub fn detach(rule: &ImportRule, port: u32, busid: &str) {
    let _ = fs::write(Path::new(VHCI).join("detach"), port.to_string());
    lost(rule, port, busid);
}

/// The connection of `port` is gone: forget the record.
pub fn lost(_rule: &ImportRule, port: u32, _busid: &str) {
    let _ = fs::remove_file(Path::new(RECORDS).join(format!("port{port}")));
}

pub fn port_in_use(port: u32) -> bool {
    vhci_ports()
        .map(|ps| ps.iter().any(|p| p.port == port && matches!(p.sta, VDEV_ST_NOTASSIGNED | VDEV_ST_USED)))
        .unwrap_or(false)
}

/// A port attached by an earlier run of the service for this rule. The record holds the
/// rule's name and the remote bus id (older records: the name only).
pub fn adopt(rule: &ImportRule) -> Option<(u32, String)> {
    let name = rule_name(rule);
    for e in fs::read_dir(RECORDS).ok()?.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(port) = file.strip_prefix("port").and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(record) = fs::read_to_string(e.path()) else { continue };
        let mut lines = record.lines();
        if lines.next() == Some(name.as_str()) && port_in_use(port) {
            return Some((port, lines.next().unwrap_or("").to_string()));
        }
    }
    None
}

/// Unmounts the device's file systems; an error says what still uses it.
pub fn before_release(rule: &ImportRule) -> Result<(), String> {
    match crate::linux::ready::attached_device_dir(rule.device.vendor_id, rule.device.product_id) {
        Some(dir) => crate::linux::ready::unmount_all(&dir),
        None => Ok(()),
    }
}
