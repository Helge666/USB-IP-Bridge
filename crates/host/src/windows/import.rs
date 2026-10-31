//! Import backend for Windows: the usbip-win2 driver attaches the device (it appears on
//! its own virtual host controller); the service drives it through usbip-win2's command
//! line tool.
//!
//! The service decides about reconnecting (device profile), so attaches use `--once` and
//! any attempts the driver keeps up on its own are stopped when a connection is lost.

use anyhow::{anyhow, bail, Context, Result};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use usbip_protocol::UsbDevice;
use usbipb_core::classes::DeviceClass;
use usbipb_core::config::ImportRule;
use usbipb_core::model::DeviceState;

/// How often an attached port is checked (each check runs the tool).
pub const POLL: Duration = Duration::from_secs(1);

fn exe() -> PathBuf {
    let dir = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    dir.join("USBip").join("usbip.exe")
}

/// Is usbip-win2 installed (its tool and its driver)?
pub fn available() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_ANY};
    let key = HSTRING::from(r"SYSTEM\CurrentControlSet\Services\usbip2_ude");
    let value = HSTRING::from("ImagePath");
    let driver = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &key, &value, RRF_RT_ANY, None, None, None) }.is_ok();
    driver && exe().is_file()
}

/// Runs usbip.exe; its error message if it fails.
fn usbip(args: &[&str]) -> Result<String> {
    let out = Command::new(exe())
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .context("usbip-win2 (usbip.exe)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        bail!("{}", err.strip_prefix("error: ").unwrap_or(err));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn prepare() -> Result<()> {
    if !available() {
        bail!("usbip-win2 is not installed (it attaches devices of other computers on Windows)");
    }
    Ok(())
}

pub fn attach(rule: &ImportRule, remote: &UsbDevice) -> Result<(u32, String)> {
    let tcp = rule.port.to_string();
    let before = crate::windows::audio::defaults();
    let out = usbip(&["-t", &tcp, "attach", "-r", &rule.host, "-b", &remote.busid, "--once", "--terse"]).map_err(|e| {
        // usbip-win2 says "Device busy (already exported)" for every "busy" answer
        if e.to_string().starts_with("Device busy") {
            anyhow!("{} is in use on {} (another client, or a program there holds it)", remote.busid, rule.host)
        } else {
            anyhow!("{} refused to attach {}: {e}", rule.host, remote.busid)
        }
    })?;
    let port = out.trim().parse().with_context(|| format!("unexpected answer of usbip.exe: {}", out.trim()))?;
    if let Some(before) = before {
        crate::windows::audio::keep(before, crate::import::rule_name(rule));
    }
    Ok((port, remote.busid.clone()))
}

pub fn detach(rule: &ImportRule, port: u32, busid: &str) {
    if let Err(e) = usbip(&["detach", "-p", &port.to_string()]) {
        log::warn!("detach port {port}: {e:#}");
    }
    lost(rule, port, busid);
}

/// The connection is gone: the driver must not try again on its own.
pub fn lost(rule: &ImportRule, _port: u32, busid: &str) {
    if !busid.is_empty() {
        let _ = usbip(&["-t", &rule.port.to_string(), "attach", "-r", &rule.host, "-b", busid, "--stop"]);
    }
}

/// A device on usbip-win2's host controller, as `usbip port` lists it.
#[derive(Debug, PartialEq)]
struct Imported {
    port: u32,
    vendor_id: u16,
    product_id: u16,
    /// `usbip://HOST:PORT/BUSID`
    url: String,
}

/// Parses the output of `usbip port`:
/// ```text
/// Port 01: device in use at Full Speed(12Mbps)
///          unknown vendor : unknown product (1290:1947)
///            -> usbip://192.168.178.49:3242/8-2
/// ```
fn parse_ports(text: &str) -> Vec<Imported> {
    let mut out: Vec<Imported> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Port ") {
            if let Some(port) = rest.split(':').next().and_then(|n| n.trim().parse().ok()) {
                out.push(Imported { port, vendor_id: 0, product_id: 0, url: String::new() });
            }
        } else if let Some(cur) = out.last_mut() {
            if let Some(url) = line.strip_prefix("-> usbip://") {
                cur.url = format!("usbip://{url}");
            } else if cur.vendor_id == 0 && line.ends_with(')') {
                let ids = line.rsplit('(').next().unwrap_or("").trim_end_matches(')');
                if let Some((v, p)) = ids.split_once(':') {
                    cur.vendor_id = u16::from_str_radix(v, 16).unwrap_or(0);
                    cur.product_id = u16::from_str_radix(p, 16).unwrap_or(0);
                }
            }
        }
    }
    out
}

fn imported() -> Option<Vec<Imported>> {
    usbip(&["port"]).ok().map(|t| parse_ports(&t))
}

pub fn port_in_use(port: u32) -> bool {
    // if the tool cannot tell, assume the device is still there
    imported().is_none_or(|ps| ps.iter().any(|p| p.port == port))
}

/// A port attached by an earlier run of the service for this rule.
pub fn adopt(rule: &ImportRule) -> Option<(u32, String)> {
    let prefix = format!("usbip://{}:{}/", rule.host, rule.port);
    imported()?
        .into_iter()
        .find(|p| p.url.starts_with(&prefix) && p.vendor_id == rule.device.vendor_id && p.product_id == rule.device.product_id)
        .map(|p| (p.port, p.url[prefix.len()..].to_string()))
}

/// Ejects a storage device the safe way first (Windows writes its caches); an error
/// says which program still uses it.
pub fn before_release(rule: &ImportRule) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    let Some(dev) = crate::windows::devices::list().into_iter().find(|d| {
        d.state == DeviceState::Attached
            && d.vendor_id == rule.device.vendor_id
            && d.product_id == rule.device.product_id
            && d.classes.contains(&DeviceClass::Storage)
    }) else {
        return Ok(());
    };
    unsafe {
        let mut devinst = 0u32;
        if CM_Locate_DevNodeW(&mut devinst, &HSTRING::from(dev.platform_id.as_str()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return Ok(());
        }
        let mut veto = PNP_VETO_TYPE::default();
        let mut name = [0u16; 260];
        let r = CM_Request_Device_EjectW(devinst, Some(&mut veto), Some(&mut name), 0);
        if r == CR_SUCCESS && veto == PNP_VetoTypeUnknown {
            return Ok(());
        }
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let who = String::from_utf16_lossy(&name[..end]);
        let drives = drive_letters(&who);
        Err(if !drives.is_empty() {
            format!("Windows cannot eject the device yet – {} is still in use (close the files and programs on it)", drives.join(", "))
        } else if who.is_empty() {
            "Windows cannot eject the device yet – a program still uses it".into()
        } else {
            format!("Windows cannot eject the device yet – in use ({who})")
        })
    }
}

/// Drive letters ("F:") of a volume, from its device instance id (`STORAGE\Volume\…`).
fn drive_letters(instance_id: &str) -> Vec<String> {
    use windows::core::{GUID, HSTRING, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows::Win32::Storage::FileSystem::{GetVolumeNameForVolumeMountPointW, GetVolumePathNamesForVolumeNameW};
    // GUID_DEVINTERFACE_VOLUME
    const VOLUME: GUID = GUID::from_u128(0x53f5630d_b6bf_11d0_94f2_00a0c91efb8b);
    if instance_id.is_empty() {
        return Vec::new();
    }
    unsafe {
        let id = HSTRING::from(instance_id);
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, &VOLUME, &id, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS || len < 2 {
            return Vec::new();
        }
        let mut list = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(&VOLUME, &id, &mut list, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        // first interface path, as a mount point (trailing backslash)
        let end = list.iter().position(|&c| c == 0).unwrap_or(list.len());
        let mut path = list[..end].to_vec();
        path.extend("\\\0".encode_utf16());
        let mut volume = [0u16; 64];
        if GetVolumeNameForVolumeMountPointW(PCWSTR(path.as_ptr()), &mut volume).is_err() {
            return Vec::new();
        }
        let mut names = [0u16; 512];
        let mut needed = 0u32;
        if GetVolumePathNamesForVolumeNameW(PCWSTR(volume.as_ptr()), Some(&mut names), &mut needed).is_err() {
            return Vec::new();
        }
        names
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf16_lossy(s).trim_end_matches('\\').to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        let text = "Imported USB devices\n====================\nPort 01: device in use at Full Speed(12Mbps)\n         unknown vendor : unknown product (1290:1947)\n           -> usbip://192.168.178.49:3242/8-2\n           -> remote bus/dev: 008/002\n           -> serial: \n           -> mode: zero-copy\n";
        assert_eq!(
            parse_ports(text),
            vec![Imported { port: 1, vendor_id: 0x1290, product_id: 0x1947, url: "usbip://192.168.178.49:3242/8-2".into() }]
        );
        assert!(parse_ports("Imported USB devices\n====================\n").is_empty());
    }
}
