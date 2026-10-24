//! USB device enumeration on Linux (sysfs).

use std::fs;
use std::path::Path;
use usbipb_core::classes::{classify, InterfaceTriple};
use usbipb_core::model::{DeviceState, UsbDevice};

fn read(p: &Path, name: &str) -> Option<String> {
    fs::read_to_string(p.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn hex(p: &Path, name: &str) -> Option<u16> {
    u16::from_str_radix(&read(p, name)?, 16).ok()
}

fn byte(p: &Path, name: &str) -> u8 {
    read(p, name).and_then(|v| u8::from_str_radix(&v, 16).ok()).unwrap_or(0)
}

/// Interface triples of the active configuration from sysfs (`<busid>:<cfg>.<n>` dirs).
pub fn interfaces(dev: &Path) -> Vec<InterfaceTriple> {
    let Some(name) = dev.file_name().map(|n| n.to_string_lossy().into_owned()) else { return Vec::new() };
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dev) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("{name}:")) {
                let p = e.path();
                out.push((byte(&p, "bInterfaceClass"), byte(&p, "bInterfaceSubClass"), byte(&p, "bInterfaceProtocol")));
            }
        }
    }
    out
}

/// Present USB devices (no root hubs, no hubs, no interfaces).
pub fn list() -> Vec<UsbDevice> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/bus/usb/devices") else { return out };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // "1-4.2" is a device; "usb1" a root hub; "1-4.2:1.0" an interface
        if name.starts_with("usb") || name.contains(':') {
            continue;
        }
        let p = e.path();
        if read(&p, "bDeviceClass").as_deref() == Some("09") {
            continue; // hub
        }
        let (Some(vid), Some(pid)) = (hex(&p, "idVendor"), hex(&p, "idProduct")) else { continue };
        let driver = fs::read_link(p.join("driver")).ok().and_then(|l| l.file_name().map(|f| f.to_string_lossy().into_owned()));
        let on_vhci = fs::canonicalize(&p).map(|c| c.to_string_lossy().contains("vhci_hcd")).unwrap_or(false);
        out.push(UsbDevice {
            bus_id: name.clone(),
            vendor_id: vid,
            product_id: pid,
            manufacturer: read(&p, "manufacturer"),
            product: read(&p, "product"),
            serial: read(&p, "serial"),
            platform_id: p.to_string_lossy().into_owned(),
            driver,
            state: if on_vhci { DeviceState::Attached } else { DeviceState::Local },
            classes: classify(byte(&p, "bDeviceClass"), &interfaces(&p)),
        });
    }
    // numeric order: 1-4.2 before 1-10
    out.sort_by_key(|d| d.bus_id.split(['-', '.']).map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    out
}
