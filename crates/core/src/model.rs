//! Device model shared by all platforms.

use serde::{Deserialize, Serialize};

/// A USB device as seen on one host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbDevice {
    /// Bus id in USB/IP notation, e.g. "8-1" (hub 8, port 1) on Windows or "1-4.2" on Linux.
    pub bus_id: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    /// Platform-specific identifier (Windows device instance id, Linux sysfs path).
    pub platform_id: String,
    /// Driver currently bound on this host, if known.
    pub driver: Option<String>,
    pub state: DeviceState,
    /// Device classes from the USB descriptors (empty if unknown).
    #[serde(default)]
    pub classes: Vec<crate::classes::DeviceClass>,
}

/// `true`, for serde defaults.
pub fn yes() -> bool {
    true
}

impl UsbDevice {
    /// Human-readable name: product string, else "VID:PID".
    pub fn display_name(&self) -> String {
        match (&self.manufacturer, &self.product) {
            (Some(m), Some(p)) if !p.starts_with(m.as_str()) => format!("{m} {p}"),
            (_, Some(p)) => p.clone(),
            _ => format!("{:04x}:{:04x}", self.vendor_id, self.product_id),
        }
    }
}

/// What USB-IP-Bridge does with a device on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    /// Plugged in here and used locally.
    Local,
    /// Configured for sharing, waiting for a client.
    Shared,
    /// Shared and currently used by a remote host.
    InUseRemotely,
    /// Never shared (block list).
    Blocked,
    /// A remote device attached on this host.
    Attached,
}

/// Matches a device by ids; empty fields match anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMatch {
    pub vendor_id: u16,
    pub product_id: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}

impl DeviceMatch {
    pub fn matches(&self, d: &UsbDevice) -> bool {
        self.vendor_id == d.vendor_id
            && self.product_id == d.product_id
            && self.serial.as_deref().is_none_or(|s| d.serial.as_deref().is_some_and(|ds| ds.eq_ignore_ascii_case(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(serial: Option<&str>) -> UsbDevice {
        UsbDevice {
            bus_id: "8-1".into(),
            vendor_id: 0x1397,
            product_id: 0x00d4,
            manufacturer: Some("BEHRINGER".into()),
            product: Some("X18/XR18".into()),
            serial: serial.map(str::to_owned),
            platform_id: String::new(),
            driver: None,
            state: DeviceState::Local,
            classes: Vec::new(),
        }
    }

    #[test]
    fn match_by_ids_and_serial() {
        let m = DeviceMatch { vendor_id: 0x1397, product_id: 0x00d4, serial: Some("abcd1234".into()) };
        assert!(m.matches(&dev(Some("ABCD1234"))));
        assert!(!m.matches(&dev(Some("OTHER"))));
        assert!(!m.matches(&dev(None)));
        let any = DeviceMatch { vendor_id: 0x1397, product_id: 0x00d4, serial: None };
        assert!(any.matches(&dev(None)));
    }

    #[test]
    fn display_name() {
        assert_eq!(dev(None).display_name(), "BEHRINGER X18/XR18");
    }
}
