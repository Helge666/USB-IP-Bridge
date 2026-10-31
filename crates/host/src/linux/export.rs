//! Lending a device on Linux: the kernel's usbip-host driver serves it.
//!
//! The device keeps its normal driver until another computer imports it. Then it is
//! bound to usbip-host and the connected socket goes to the kernel (`usbip_sockfd`, like
//! the `usbipd` tool does); when the session ends, the device gets its normal driver
//! back. Needs root.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::BufReader;
use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use usbip_protocol::{InterfaceInfo, Speed};
use usbipb_core::model::UsbDevice;

const DRIVER: &str = "/sys/bus/usb/drivers/usbip-host";
const DEVICES: &str = "/sys/bus/usb/devices";
/// Devices lent by this service, so a later start can give back what a crash left behind.
const RECORDS: &str = "/run/usb-ip-bridge";
/// `enum usbip_device_status` in the kernel (server side).
const SDEV_ST_AVAILABLE: u32 = 1;
const SDEV_ST_USED: u32 = 2;

/// Can this computer lend devices (is the usbip-host driver there or loadable)?
pub fn available() -> bool {
    Path::new(DRIVER).exists() || std::process::Command::new("modprobe").args(["-n", "-q", "usbip-host"]).status().is_ok_and(|s| s.success())
}

fn ensure_driver() -> Result<()> {
    if !Path::new(DRIVER).exists() {
        let _ = std::process::Command::new("modprobe").arg("usbip-host").status();
    }
    if !Path::new(DRIVER).exists() {
        bail!("kernel module usbip-host is not available");
    }
    Ok(())
}

fn write(path: impl AsRef<Path>, value: &str) -> Result<()> {
    let path = path.as_ref();
    fs::write(path, value).with_context(|| format!("write '{}' to {}", value.trim(), path.display()))
}

fn read(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string())
}

fn dec<T: std::str::FromStr>(dir: &Path, name: &str) -> Option<T> {
    read(dir, name)?.parse().ok()
}

fn hex8(dir: &Path, name: &str) -> u8 {
    read(dir, name).and_then(|v| u8::from_str_radix(&v, 16).ok()).unwrap_or(0)
}

fn driver(dir: &Path) -> Option<String> {
    fs::read_link(dir.join("driver")).ok()?.file_name().map(|f| f.to_string_lossy().into_owned())
}

fn status(dir: &Path) -> Option<u32> {
    dec(dir, "usbip_status")
}

fn record(bus_id: &str) -> PathBuf {
    Path::new(RECORDS).join(format!("lent-{bus_id}"))
}

/// The kernel's speed from sysfs ("480" is Mbit/s).
fn speed(dir: &Path) -> Speed {
    match read(dir, "speed").as_deref() {
        Some("1.5") => Speed::Low,
        Some("12") => Speed::Full,
        Some("480") => Speed::High,
        Some("5000") => Speed::Super,
        Some(s) if s.parse::<u32>().is_ok_and(|v| v > 5000) => Speed::SuperPlus,
        _ => Speed::Unknown,
    }
}

/// Bus number, device number, speed and interfaces for the device list.
pub fn wire_details(d: &UsbDevice) -> (u32, u32, Speed, Vec<InterfaceInfo>) {
    let dir = Path::new(DEVICES).join(&d.bus_id);
    let interfaces = super::devices::interfaces(&dir)
        .into_iter()
        .map(|(class, subclass, protocol)| InterfaceInfo { class, subclass, protocol })
        .collect();
    (dec(&dir, "busnum").unwrap_or(0), dec(&dir, "devnum").unwrap_or(0), speed(&dir), interfaces)
}

/// A device bound to usbip-host for another computer; it gets its normal driver back
/// when dropped.
pub struct Lease {
    bus_id: String,
    dir: PathBuf,
}

/// Device files of the device that carry its data (sound, MIDI, video, raw HID, usbfs,
/// the disk itself), from sysfs. Not the sound card's control file and not input events:
/// the desktop keeps those open all the time and lets go by itself.
fn data_nodes(dir: &Path) -> Vec<String> {
    let mut nodes = Vec::new();
    let mut todo = vec![fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())];
    while let Some(d) = todo.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            // real subdirectories only (sysfs links lead all over the tree)
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push(e.path());
            }
        }
        let Some(name) = fs::read_to_string(d.join("uevent")).ok().and_then(|u| u.lines().find_map(|l| l.strip_prefix("DEVNAME=").map(str::to_string))) else {
            continue;
        };
        let base = name.rsplit('/').next().unwrap_or(&name);
        if ["control", "event", "mouse", "js"].iter().any(|p| base.starts_with(p)) {
            continue;
        }
        nodes.push(format!("/dev/{name}"));
    }
    nodes
}

/// Programs that have one of the device's data files open: "name (pid)".
fn holders(dir: &Path) -> Vec<String> {
    let nodes = data_nodes(dir);
    let mut found = Vec::new();
    let Ok(procs) = fs::read_dir("/proc") else { return found };
    for p in procs.flatten() {
        let Some(pid) = p.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == std::process::id() {
            continue;
        }
        let Ok(fds) = fs::read_dir(p.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|t| nodes.iter().any(|n| Path::new(n) == t))) {
            let name = read(&p.path(), "comm").unwrap_or_else(|| "?".into());
            let entry = format!("{name} ({pid})");
            if !found.contains(&entry) {
                found.push(entry);
            }
        }
    }
    found
}

/// Refuses to take a device away from a program that uses it (as Windows does).
fn check_not_in_use(dir: &Path) -> Result<()> {
    let holders = holders(dir);
    if holders.is_empty() {
        return Ok(());
    }
    let via_media_server = holders.iter().any(|h| ["pipewire", "pulseaudio", "jackd", "jackdbus"].iter().any(|s| h.starts_with(&format!("{s} "))));
    bail!(
        "in use on this computer by {}{}",
        holders.join(", "),
        if via_media_server { " – a program plays, records or films through it; stop that and the other computer tries again" } else { "" }
    )
}

impl Lease {
    pub fn capture(d: &UsbDevice) -> Result<Self> {
        ensure_driver()?;
        let dir = Path::new(DEVICES).join(&d.bus_id);
        check_not_in_use(&dir)?;
        // Never pull a mounted file system away.
        super::ready::unmount_all(&dir).map_err(|e| anyhow!(e))?;
        let lease = Self { bus_id: d.bus_id.clone(), dir };
        let _ = fs::create_dir_all(RECORDS);
        let _ = fs::write(record(&lease.bus_id), "");
        let bus = &lease.bus_id;
        if driver(&lease.dir).as_deref() != Some("usbip-host") {
            // as the `usbip bind` tool does
            if let Some(other) = driver(&lease.dir) {
                write(format!("/sys/bus/usb/drivers/{other}/unbind"), bus)?;
            }
            write(format!("{DRIVER}/match_busid"), &format!("add {bus}"))?;
            write(format!("{DRIVER}/bind"), bus)?;
        }
        let t = Instant::now();
        while status(&lease.dir) != Some(SDEV_ST_AVAILABLE) {
            if t.elapsed() > Duration::from_secs(3) {
                bail!("usbip-host did not take {bus} (status {:?})", status(&lease.dir));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(lease)
    }

    /// Fills in the device record from sysfs.
    pub fn complete(&self, info: &mut usbip_protocol::UsbDevice) -> Result<()> {
        let dir = &self.dir;
        info.bcd_device = read(dir, "bcdDevice").and_then(|v| u16::from_str_radix(&v, 16).ok()).unwrap_or(0);
        info.device_class = hex8(dir, "bDeviceClass");
        info.device_subclass = hex8(dir, "bDeviceSubClass");
        info.device_protocol = hex8(dir, "bDeviceProtocol");
        info.configuration_value = dec(dir, "bConfigurationValue").unwrap_or(1);
        info.num_configurations = dec(dir, "bNumConfigurations").unwrap_or(1);
        info.num_interfaces = dec(dir, "bNumInterfaces").unwrap_or(info.num_interfaces);
        info.speed = speed(dir);
        Ok(())
    }

    /// Hands the connected socket to the kernel (before the import reply, like `usbipd`).
    pub fn hand_over(&self, stream: &TcpStream) -> Result<()> {
        write(self.dir.join("usbip_sockfd"), &format!("{}\n", stream.as_raw_fd()))
    }

    /// Waits until the session ends (the other computer lets the device go, the connection
    /// breaks, or `stream` is shut down here).
    pub fn serve(self, _speed: Speed, stream: TcpStream, reader: BufReader<TcpStream>) -> Result<()> {
        drop(reader);
        while status(&self.dir) == Some(SDEV_ST_USED) {
            std::thread::sleep(Duration::from_millis(300));
        }
        drop(stream);
        Ok(())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        release(&self.bus_id);
    }
}

/// Ends a session that may still run and gives the device its normal driver back.
fn release(bus: &str) {
    let dir = Path::new(DEVICES).join(bus);
    if driver(&dir).as_deref() == Some("usbip-host") {
        if status(&dir) == Some(SDEV_ST_USED) {
            let _ = write(dir.join("usbip_sockfd"), "-1\n");
            let t = Instant::now();
            while status(&dir) == Some(SDEV_ST_USED) && t.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if let Err(e) = write(format!("{DRIVER}/unbind"), bus) {
            log::warn!("{e:#}");
        }
    }
    // as the `usbip unbind` tool does: "rebind" probes the normal driver and removes the
    // bus id from usbip-host's list
    let _ = write(format!("{DRIVER}/match_busid"), &format!("del {bus}"));
    let _ = write(format!("{DRIVER}/match_busid"), &format!("add {bus}"));
    if write(format!("{DRIVER}/rebind"), bus).is_err() {
        let _ = write(format!("{DRIVER}/match_busid"), &format!("del {bus}"));
        let _ = write("/sys/bus/usb/drivers_probe", bus);
    }
    let _ = fs::remove_file(record(bus));
    log::debug!("{bus}: back with its normal driver ({})", driver(&dir).unwrap_or_else(|| "none".into()));
}

/// Gives back devices an earlier run of the service left bound to usbip-host.
pub fn release_leftovers() {
    let Ok(entries) = fs::read_dir(RECORDS) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(bus) = name.strip_prefix("lent-") {
            log::info!("{bus}: lent before the service stopped – giving it back");
            release(bus);
        }
    }
}
