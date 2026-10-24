//! USB device enumeration on Windows (SetupAPI / CfgMgr32).

use super::vbox::{config_descriptor, location};
use crate::descriptors::ConfigInfo;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use usbipb_core::classes::{classify, DeviceClass, InterfaceTriple};
use usbipb_core::model::{DeviceState, UsbDevice};
use windows::core::HSTRING;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::*;
use windows::Win32::Foundation::DEVPROPKEY;

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// Reads a string device property, e.g. `DEVPKEY_Device_DriverInfPath`.
pub fn devnode_string(instance_id: &str, key: &DEVPROPKEY) -> Option<String> {
    unsafe {
        let mut devinst = 0u32;
        if CM_Locate_DevNodeW(&mut devinst, &HSTRING::from(instance_id), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return None;
        }
        let mut ty = DEVPROPTYPE::default();
        let mut buf = vec![0u8; 1024];
        let mut size = buf.len() as u32;
        if CM_Get_DevNode_PropertyW(devinst, key, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0) != CR_SUCCESS {
            return None;
        }
        let words: Vec<u16> = buf[..size as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let s = wide_to_string(&words);
        (!s.is_empty()).then_some(s)
    }
}

/// Interface (class, subclass, protocol) triples of a device, cached by instance id.
pub fn interfaces(instance_id: &str, port: u16) -> Vec<InterfaceTriple> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<InterfaceTriple>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(instance_id) {
        return v.clone();
    }
    let triples: Vec<InterfaceTriple> = config_descriptor(instance_id, port)
        .and_then(|raw| ConfigInfo::parse(raw).ok())
        .map(|c| c.interfaces.iter().filter(|i| i.1 == 0).map(|i| (i.2, i.3, i.4)).collect())
        .unwrap_or_default();
    if !triples.is_empty() {
        cache.lock().unwrap().insert(instance_id.to_string(), triples.clone());
    }
    triples
}

fn classes_of(instance_id: &str, port: u16) -> Vec<DeviceClass> {
    let mut classes = classify(0, &interfaces(instance_id, port));
    // Vendor-specific devices: the Windows setup class of the driver tells more.
    if classes.is_empty() || classes == [DeviceClass::VendorSpecific] {
        let setup_class = devnode_string(instance_id, &DEVPKEY_Device_Class).unwrap_or_default().to_ascii_lowercase();
        let by_setup = match setup_class.as_str() {
            "net" => Some(DeviceClass::Network),
            "bluetooth" => Some(DeviceClass::Bluetooth),
            "media" => Some(DeviceClass::Audio),
            "camera" | "image" => Some(DeviceClass::Video),
            "keyboard" => Some(DeviceClass::Keyboard),
            "mouse" => Some(DeviceClass::Mouse),
            "diskdrive" | "scsiadapter" => Some(DeviceClass::Storage),
            "ports" => Some(DeviceClass::Serial),
            _ => None,
        };
        if let Some(c) = by_setup {
            classes = vec![c];
        }
    }
    classes
}

fn hex_after(id: &str, key: &str) -> Option<u16> {
    let i = id.find(key)? + key.len();
    u16::from_str_radix(id.get(i..i + 4)?, 16).ok()
}

/// Present USB devices (not hubs, not interfaces of composite devices).
pub fn list() -> Vec<UsbDevice> {
    let mut out = Vec::new();
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(None, &HSTRING::from("USB"), None, DIGCF_PRESENT | DIGCF_ALLCLASSES) else {
            return out;
        };
        for i in 0.. {
            let mut did = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set, i, &mut did).is_err() {
                break;
            }
            let mut idbuf = [0u16; 512];
            if SetupDiGetDeviceInstanceIdW(set, &did, Some(&mut idbuf), None).is_err() {
                continue;
            }
            let id = wide_to_string(&idbuf);
            let upper = id.to_ascii_uppercase();
            if !upper.starts_with(r"USB\VID_") || upper.contains("&MI_") {
                continue;
            }
            let driver = devnode_string(&id, &DEVPKEY_Device_Service);
            if driver.as_deref().is_some_and(|d| d.to_ascii_lowercase().starts_with("usbhub")) {
                continue;
            }
            let (Some(vid), Some(pid)) = (hex_after(&upper, "VID_"), hex_after(&upper, "PID_")) else { continue };
            let Some((hub, port)) = location(&id) else { continue };
            let serial = id.rsplit('\\').next().filter(|s| !s.contains('&')).map(str::to_owned);
            let product = devnode_string(&id, &DEVPKEY_Device_BusReportedDeviceDesc)
                .or_else(|| devnode_string(&id, &DEVPKEY_Device_FriendlyName))
                .or_else(|| devnode_string(&id, &DEVPKEY_Device_DeviceDesc))
                .map(|p| p.trim().to_string()); // some sticks pad their name with spaces
            out.push(UsbDevice {
                bus_id: format!("{hub}-{port}"),
                vendor_id: vid,
                product_id: pid,
                manufacturer: None,
                product,
                serial,
                classes: classes_of(&id, port),
                platform_id: id,
                driver,
                state: DeviceState::Local,
            });
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    // numeric order: 2-4 before 10-1
    out.sort_by_key(|d| d.bus_id.split('-').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    out
}
