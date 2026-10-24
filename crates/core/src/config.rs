//! Service configuration (TOML).
//!
//! Windows: `%ProgramData%\USB-IP-Bridge\config.toml`
//! Linux:   `/etc/usb-ip-bridge/config.toml`

use crate::model::DeviceMatch;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default TCP port for USB/IP traffic. 3240 is the USB/IP standard port, but usbipd-win
/// (whose VBoxUSB drivers we use) already listens there.
pub const DEFAULT_USBIP_PORT: u16 = 3242;
/// Default port of the local API (bound to 127.0.0.1 only).
pub const DEFAULT_API_PORT: u16 = 3243;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_usbip_port")]
    pub usbip_port: u16,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    /// Devices this host shares.
    #[serde(default)]
    pub export: Vec<ExportRule>,
    /// Remote devices this host attaches.
    #[serde(default)]
    pub import: Vec<ImportRule>,
    /// Devices that are never shared, whatever `export` says.
    #[serde(default)]
    pub never_share: Vec<DeviceMatch>,
    /// Find other USB-IP-Bridge computers in the local network (UDP broadcast).
    #[serde(default = "yes")]
    pub discovery: bool,
    /// Other computers to contact even if discovery does not find them
    /// (host name or address, optionally with ":port"), e.g. over Tailscale.
    #[serde(default)]
    pub peers: Vec<String>,
    /// Only paired computers may use shared devices (USB/IP). `false` lets any computer in
    /// the network use them, e.g. standard `usbip` clients – trusted networks only.
    #[serde(default = "yes")]
    pub require_pairing: bool,
    /// Devices that look essential (their profile warns of lockout or network loss: keyboard,
    /// mouse, network, Bluetooth) are never shared unless in `unlocked`. Set by the service,
    /// not read from the file.
    #[serde(skip)]
    pub lock_essential: bool,
    /// Devices unlocked in the app although they look essential.
    #[serde(skip)]
    pub unlocked: Vec<DeviceMatch>,
}

fn default_usbip_port() -> u16 {
    DEFAULT_USBIP_PORT
}
fn default_api_port() -> u16 {
    DEFAULT_API_PORT
}

impl Default for Config {
    fn default() -> Self {
        Self {
            usbip_port: DEFAULT_USBIP_PORT,
            api_port: DEFAULT_API_PORT,
            export: Vec::new(),
            import: Vec::new(),
            never_share: Vec::new(),
            discovery: true,
            peers: Vec::new(),
            require_pairing: true,
            lock_essential: false,
            unlocked: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportRule {
    #[serde(flatten)]
    pub device: DeviceMatch,
    /// Client IP addresses allowed to attach; empty = any paired computer.
    #[serde(default)]
    pub allowed_clients: Vec<String>,
    /// Programs that keep the device open on this host (e.g. a vendor control panel).
    /// The GUI offers to close them when they block sharing.
    #[serde(default)]
    pub blocking_processes: Vec<String>,
}

/// When an import rule attaches its device.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectMode {
    /// Only after the user asks for it (`usbipb attach`, GUI); stays attached
    /// (with automatic reconnect) until the user releases it.
    #[default]
    Manual,
    /// Always, from service start.
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportRule {
    pub host: String,
    #[serde(default = "default_usbip_port")]
    pub port: u16,
    /// The remote device, found in the server's device list.
    #[serde(flatten)]
    pub device: DeviceMatch,
    #[serde(default)]
    pub mode: ConnectMode,
    /// Reconnect automatically after a broken connection while the device is wanted.
    #[serde(default = "yes")]
    pub auto_reconnect: bool,
}

fn yes() -> bool {
    true
}

pub fn config_dir() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
        PathBuf::from(base).join("USB-IP-Bridge")
    } else {
        PathBuf::from("/etc/usb-ip-bridge")
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    /// Never shared: listed in `never_share`, or essential and not unlocked (see `lock_essential`).
    pub fn is_blocked(&self, d: &crate::model::UsbDevice) -> bool {
        self.never_share.iter().any(|m| m.matches(d))
            || (self.lock_essential && !self.unlocked.iter().any(|m| m.matches(d)) && crate::profiles::is_essential(d.vendor_id, d.product_id, &d.classes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_example() {
        let text = r#"
            usbip_port = 3242

            [[export]]
            vendor_id = 0x1397
            product_id = 0x00d4
            serial = "ABCD1234"
            allowed_clients = ["192.168.1.20"]

            [[import]]
            host = "192.168.1.10"
            vendor_id = 0x1397
            product_id = 0x00d4

            [[never_share]]
            vendor_id = 0x17f6
            product_id = 0x0862
        "#;
        let c: Config = toml::from_str(text).unwrap();
        assert_eq!(c.api_port, DEFAULT_API_PORT);
        assert_eq!(c.export[0].device.serial.as_deref(), Some("ABCD1234"));
        assert_eq!(c.import[0].port, DEFAULT_USBIP_PORT);
        assert!(c.import[0].auto_reconnect);
        assert_eq!(c.never_share[0].vendor_id, 0x17f6);
    }

    #[test]
    fn essential_devices_are_locked_unless_unlocked() {
        use crate::classes::DeviceClass;
        let dev = |vendor_id, classes: Vec<DeviceClass>| crate::model::UsbDevice {
            bus_id: "1-1".into(),
            vendor_id,
            product_id: 1,
            manufacturer: None,
            product: None,
            serial: None,
            platform_id: String::new(),
            driver: None,
            state: crate::model::DeviceState::Local,
            classes,
        };
        let keyboard = dev(0x1111, vec![DeviceClass::Keyboard]);
        let nic = dev(0x2222, vec![DeviceClass::Network]);
        let audio = dev(0x3333, vec![DeviceClass::Audio, DeviceClass::Midi]);
        let mut c = Config::default();
        assert!(!c.is_blocked(&keyboard), "only the service turns the automatic locks on");
        c.lock_essential = true;
        assert!(c.is_blocked(&keyboard) && c.is_blocked(&nic) && !c.is_blocked(&audio));
        c.unlocked.push(DeviceMatch { vendor_id: 0x1111, product_id: 1, serial: None });
        assert!(!c.is_blocked(&keyboard));
        c.never_share.push(DeviceMatch { vendor_id: 0x3333, product_id: 1, serial: None });
        assert!(c.is_blocked(&audio));
    }
}
