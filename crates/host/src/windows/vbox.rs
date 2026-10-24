//! Access to USB devices through the VirtualBox USB drivers that usbipd-win installs:
//! VBoxUSBMon (filter/monitor) captures a device, VBoxUSB.sys then lets a user-mode
//! process send URBs to it. Unlike WinUSB, VBoxUSB accepts a size per isochronous
//! packet, which audio playback needs.
//!
//! The capture sequence follows usbipd-win (GPL-3.0, Frans van Dorsselaer):
//! add a capture filter, remove the device subtree, cycle the hub port, wait for
//! the VBoxUSB device to appear, claim it. Two differences:
//! - we send SUPUSB_IOCTL_GET_DEVICE, without which VBoxUSB schedules high-speed
//!   isochronous transfers like full-speed ones;
//! - isochronous URBs must be submitted one frame (1 ms) at a time, see server_vbox.rs.

use super::devices::devnode_string;
use anyhow::{anyhow, bail, Context, Result};
use std::time::{Duration, Instant};
use windows::core::{GUID, HSTRING};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::*;
use windows::Win32::Devices::Usb::{GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, IOCTL_USB_HUB_CYCLE_PORT};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::{DeviceIoControl, OVERLAPPED};

pub const GUID_CLASS_VBOXUSB: GUID = GUID::from_u128(0x00873fdf_cafe_80ee_aa5e_00c04fb1720b);

const fn ctl(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_UNKNOWN, function, METHOD_BUFFERED, FILE_WRITE_ACCESS)
    (0x22 << 16) | (2 << 14) | (function << 2)
}
pub const SUPUSB_IOCTL_GET_DEVICE: u32 = ctl(0x603);
pub const SUPUSB_IOCTL_SEND_URB: u32 = ctl(0x607);
pub const SUPUSB_IOCTL_USB_SELECT_INTERFACE: u32 = ctl(0x609);
pub const SUPUSB_IOCTL_USB_SET_CONFIG: u32 = ctl(0x60a);
pub const SUPUSB_IOCTL_USB_CLAIM_DEVICE: u32 = ctl(0x60b);
pub const SUPUSB_IOCTL_USB_CLEAR_ENDPOINT: u32 = ctl(0x60e);
pub const SUPUSB_IOCTL_GET_VERSION: u32 = ctl(0x60f);
pub const SUPUSB_IOCTL_USB_ABORT_ENDPOINT: u32 = ctl(0x610);
const SUPUSBFLT_IOCTL_GET_VERSION: u32 = ctl(0x610);
const SUPUSBFLT_IOCTL_ADD_FILTER: u32 = ctl(0x611);
const SUPUSBFLT_IOCTL_REMOVE_FILTER: u32 = ctl(0x612);

pub const TRANSFER_ISOC: u32 = 1;
pub const TRANSFER_BULK: u32 = 2;
pub const TRANSFER_INTR: u32 = 3;
pub const TRANSFER_MSG: u32 = 4;
pub const DIR_IN: u32 = 1;
pub const DIR_OUT: u32 = 2;
pub const FLAG_SHORT_OK: u32 = 1;
pub const MAX_ISO_PACKETS: usize = 8;

/// `USBSUP_ISOCPKT` (pack 4).
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default, Debug)]
pub struct IsoPkt {
    pub cb: u16,
    pub off: u16,
    pub stat: u32,
}

/// `USBSUP_URB` (pack 4), 104 bytes on x64.
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default, Debug)]
pub struct SupUrb {
    pub ty: u32,
    pub ep: u32,
    pub dir: u32,
    pub flags: u32,
    pub error: u32,
    pub len: u64,
    pub buf: u64,
    pub num_iso_pkts: u32,
    pub iso_pkts: [IsoPkt; MAX_ISO_PACKETS],
}
const _: () = assert!(std::mem::size_of::<SupUrb>() == 104);

/// Maps `USBSUP_ERROR` to a negative Linux errno.
pub fn error_to_errno(e: u32) -> i32 {
    use usbip_protocol::errno::*;
    match e {
        0 => 0,
        1 => -EPIPE,
        2 => -62, // ETIME
        3 => -84, // EILSEQ
        5 => -121, // EREMOTEIO (underrun)
        6 => -EOVERFLOW,
        _ => -EPROTO,
    }
}

#[derive(Clone, Copy)]
pub struct Handle(pub HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

fn open(path: &str, overlapped: bool) -> Result<HANDLE> {
    let flags = if overlapped { FILE_FLAG_OVERLAPPED } else { FILE_ATTRIBUTE_NORMAL };
    unsafe {
        CreateFileW(
            &HSTRING::from(path),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(flags.0),
            None,
        )
        .with_context(|| format!("open {path}"))
    }
}

/// Synchronous METHOD_BUFFERED ioctl on a non-overlapped handle.
fn ioctl_sync(h: HANDLE, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            h,
            code,
            (!input.is_empty()).then_some(input.as_ptr() as *const _),
            input.len() as u32,
            (!output.is_empty()).then_some(output.as_mut_ptr() as *mut _),
            output.len() as u32,
            Some(&mut returned),
            None,
        )?;
    }
    Ok(returned)
}

/// Ioctl on an overlapped handle, waiting for completion.
pub fn ioctl_wait(h: HANDLE, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
    use windows::Win32::System::IO::GetOverlappedResult;
    use windows::Win32::System::Threading::CreateEventW;
    unsafe {
        let event = CreateEventW(None, true, false, None)?;
        let mut ov = OVERLAPPED { hEvent: event, ..Default::default() };
        let res = DeviceIoControl(
            h,
            code,
            (!input.is_empty()).then_some(input.as_ptr() as *const _),
            input.len() as u32,
            (!output.is_empty()).then_some(output.as_mut_ptr() as *mut _),
            output.len() as u32,
            None,
            Some(&mut ov),
        );
        let mut n = 0u32;
        let out = match res {
            Err(e) if e.code() != windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() => Err(e.into()),
            _ => GetOverlappedResult(h, &ov, &mut n, true).map(|_| n).map_err(Into::into),
        };
        let _ = CloseHandle(event);
        out
    }
}

/// Hub number and port from `Port_#0001.Hub_#0008`.
pub fn location(instance_id: &str) -> Option<(u16, u16)> {
    let loc = devnode_string(instance_id, &DEVPKEY_Device_LocationInfo)?;
    let port = loc.split("Port_#").nth(1)?.get(0..4)?.parse().ok()?;
    let hub = loc.split("Hub_#").nth(1)?.get(0..4)?.parse().ok()?;
    Some((hub, port))
}

fn devnode(instance_id: &str) -> Result<u32> {
    let mut dn = 0u32;
    let cr = unsafe { CM_Locate_DevNodeW(&mut dn, &HSTRING::from(instance_id), CM_LOCATE_DEVNODE_NORMAL) };
    if cr != CR_SUCCESS {
        bail!("device {instance_id} not present (CONFIGRET {})", cr.0);
    }
    Ok(dn)
}

fn interface_paths(class: &GUID, instance_id: Option<&str>) -> Vec<String> {
    unsafe {
        let id = instance_id.map(HSTRING::from);
        let mut len = 0u32;
        let cr = match &id {
            Some(id) => CM_Get_Device_Interface_List_SizeW(&mut len, class, id, CM_GET_DEVICE_INTERFACE_LIST_PRESENT),
            None => CM_Get_Device_Interface_List_SizeW(&mut len, class, None, CM_GET_DEVICE_INTERFACE_LIST_PRESENT),
        };
        if cr != CR_SUCCESS || len <= 1 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        let cr = match &id {
            Some(id) => CM_Get_Device_Interface_ListW(class, id, &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT),
            None => CM_Get_Device_Interface_ListW(class, None, &mut buf, CM_GET_DEVICE_INTERFACE_LIST_PRESENT),
        };
        if cr != CR_SUCCESS {
            return Vec::new();
        }
        buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    }
}

fn interface_instance_id(path: &str) -> Option<String> {
    unsafe {
        let mut ty = DEVPROPTYPE::default();
        let mut buf = vec![0u8; 1024];
        let mut size = buf.len() as u32;
        let cr = CM_Get_Device_Interface_PropertyW(&HSTRING::from(path), &DEVPKEY_Device_InstanceId, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0);
        if cr != CR_SUCCESS {
            return None;
        }
        let w: Vec<u16> = buf[..size as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&w).trim_end_matches('\0').to_string())
    }
}

/// Removes a device subtree (fails with a readable veto if something holds the device).
fn remove_subtree(dn: u32) -> Result<()> {
    let mut veto = PNP_VETO_TYPE(0);
    let mut name = [0u16; 260];
    let cr = unsafe { CM_Query_And_Remove_SubTreeW(dn, Some(&mut veto), Some(&mut name), CM_REMOVE_NO_RESTART | CM_REMOVE_UI_NOT_OK) };
    if cr == CR_REMOVE_VETOED {
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        bail!(
            "Windows refused to release the device (veto type {}, {}). Close programs that use it (e.g. the vendor's control panel or audio apps).",
            veto.0,
            String::from_utf16_lossy(&name[..end])
        );
    }
    if cr != CR_SUCCESS {
        bail!("CM_Query_And_Remove_SubTree failed (CONFIGRET {})", cr.0);
    }
    Ok(())
}

/// Power-cycles the hub port (unplug/replug) and marks the devnode ready again.
fn cycle_port(instance_id: &str, port: u16) -> Result<()> {
    std::thread::sleep(Duration::from_millis(100));
    let parent = devnode_string(instance_id, &DEVPKEY_Device_Parent).ok_or_else(|| anyhow!("no parent hub"))?;
    let hub_path = interface_paths(&GUID_DEVINTERFACE_USB_HUB, Some(&parent)).into_iter().next().ok_or_else(|| anyhow!("hub interface not found"))?;
    let hub = open(&hub_path, false)?;
    let mut params = [0u8; 8];
    params[0..4].copy_from_slice(&(port as u32).to_le_bytes());
    let mut out = params;
    let res = ioctl_sync(hub, IOCTL_USB_HUB_CYCLE_PORT, &params, &mut out);
    unsafe {
        let _ = CloseHandle(hub);
        if let Ok(dn) = devnode(instance_id) {
            let _ = CM_Setup_DevNode(dn, CM_SETUP_DEVNODE_READY);
        }
    }
    res.map(|_| ())
}

/// Connection speed from the parent hub: 0 low, 1 full, 2 high, 3 super.
pub fn port_speed(instance_id: &str, port: u16) -> Option<u8> {
    let parent = devnode_string(instance_id, &DEVPKEY_Device_Parent)?;
    let hub_path = interface_paths(&GUID_DEVINTERFACE_USB_HUB, Some(&parent)).into_iter().next()?;
    let hub = open(&hub_path, false).ok()?;
    let mut buf = vec![0u8; 512];
    buf[0..4].copy_from_slice(&(port as u32).to_le_bytes());
    let input = buf.clone();
    let res = ioctl_sync(hub, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, &input, &mut buf);
    unsafe {
        let _ = CloseHandle(hub);
    }
    res.ok()?;
    let speed = buf[23]; // ConnectionIndex(4) + DeviceDescriptor(18) + CurrentConfigurationValue(1)
    // This IOCTL reports SuperSpeed devices as high speed. A device operating at SuperSpeed
    // has bMaxPacketSize0 = 9 (2^9 = 512 bytes) in its device descriptor.
    let max_packet0 = buf[4 + 7];
    Some(if speed == 2 && max_packet0 == 9 { 3 } else { speed })
}

/// Configuration descriptor of the device on `port` of its parent hub, read through the
/// hub (like USBView) - works without opening the device and while VBoxUSB holds it.
pub fn config_descriptor(instance_id: &str, port: u16) -> Option<Vec<u8>> {
    use windows::Win32::Devices::Usb::IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION;
    let parent = devnode_string(instance_id, &DEVPKEY_Device_Parent)?;
    let hub_path = interface_paths(&GUID_DEVINTERFACE_USB_HUB, Some(&parent)).into_iter().next()?;
    let hub = unsafe {
        CreateFileW(&HSTRING::from(hub_path.as_str()), GENERIC_WRITE.0, FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None).ok()?
    };
    // USB_DESCRIPTOR_REQUEST: ConnectionIndex (u32), SetupPacket (8 bytes), then data
    let request = |len: u16| -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 12 + len as usize];
        buf[0..4].copy_from_slice(&(port as u32).to_le_bytes());
        buf[4] = 0x80; // bmRequest: device-to-host
        buf[5] = 6; // GET_DESCRIPTOR
        buf[6..8].copy_from_slice(&(2u16 << 8).to_le_bytes()); // configuration descriptor, index 0
        buf[10..12].copy_from_slice(&len.to_le_bytes());
        let input = buf.clone();
        let n = ioctl_sync(hub, IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION, &input, &mut buf).ok()? as usize;
        (n > 12).then(|| buf[12..n].to_vec())
    };
    let head = request(9);
    let full = head.and_then(|h| {
        let total = u16::from_le_bytes([*h.get(2)?, *h.get(3)?]);
        request(total)
    });
    unsafe {
        let _ = CloseHandle(hub);
    }
    full
}

/// A device captured by VBoxUSB and claimed by this process.
pub struct Captured {
    pub file: Handle,
    pub vbox_instance_id: String,
    pub original_instance_id: String,
    pub vid: u16,
    pub pid: u16,
    pub hub: u16,
    pub port: u16,
    mon: Handle,
    filter_id: u64,
}

impl Captured {
    pub fn capture(instance_id: &str) -> Result<Self> {
        let upper = instance_id.to_ascii_uppercase();
        let hex = |key: &str| -> Result<u16> {
            let i = upper.find(key).ok_or_else(|| anyhow!("{key} not in instance id"))? + key.len();
            Ok(u16::from_str_radix(&upper[i..i + 4], 16)?)
        };
        let (vid, pid) = (hex("VID_")?, hex("PID_")?);
        let (hub, port) = location(instance_id).ok_or_else(|| anyhow!("cannot determine hub/port of {instance_id}"))?;

        let mon = open(r"\\.\VBoxUSBMon", false).context("VBoxUSBMon not available (is usbipd-win installed?)")?;
        let mut ver = [0u8; 8];
        ioctl_sync(mon, SUPUSBFLT_IOCTL_GET_VERSION, &[], &mut ver)?;
        let major = u32::from_le_bytes(ver[0..4].try_into().unwrap());
        if major != 5 {
            bail!("unsupported VBoxUSBMon version {major}");
        }

        let dn = devnode(instance_id)?;
        remove_subtree(dn)?;

        // USBFILTER: magic, type CAPTURE, 11 fields {u16 match, u16 value}, offCurEnd, 256-byte string table.
        let mut filter = vec![0u8; 312];
        filter[0..4].copy_from_slice(&0x1967_0408u32.to_le_bytes());
        filter[4..8].copy_from_slice(&4u32.to_le_bytes()); // USBFILTERTYPE_CAPTURE
        for i in 0..11 {
            filter[8 + i * 4..10 + i * 4].copy_from_slice(&1u16.to_le_bytes()); // IGNORE
        }
        let mut set = |idx: usize, value: u16| {
            filter[8 + idx * 4..10 + idx * 4].copy_from_slice(&3u16.to_le_bytes()); // NUM_EXACT
            filter[10 + idx * 4..12 + idx * 4].copy_from_slice(&value.to_le_bytes());
        };
        set(0, vid);
        set(1, pid);
        set(7, port);
        let mut out = [0u8; 12];
        let filter_res = ioctl_sync(mon, SUPUSBFLT_IOCTL_ADD_FILTER, &filter, &mut out);
        let cycle_res = cycle_port(instance_id, port);
        filter_res?;
        let rc = i32::from_le_bytes(out[8..12].try_into().unwrap());
        if rc != 0 {
            bail!("VBoxUSBMon ADD_FILTER failed: rc {rc}");
        }
        let filter_id = u64::from_le_bytes(out[0..8].try_into().unwrap());
        if let Err(e) = cycle_res {
            log::warn!("port cycle failed (continuing): {e:#}");
        }

        // Wait for the VBoxUSB device on the same hub port.
        let deadline = Instant::now() + Duration::from_secs(10);
        let (path, vbox_id) = loop {
            let found = interface_paths(&GUID_CLASS_VBOXUSB, None).into_iter().find_map(|p| {
                let id = interface_instance_id(&p)?;
                (location(&id) == Some((hub, port))).then_some((p, id))
            });
            if let Some(f) = found {
                break f;
            }
            if Instant::now() > deadline {
                bail!("device did not come back under VBoxUSB within 10 s");
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        let file = open(&path, true)?;
        let mut cap = Self {
            file: Handle(file),
            vbox_instance_id: vbox_id,
            original_instance_id: instance_id.to_string(),
            vid,
            pid,
            hub,
            port,
            mon: Handle(mon),
            filter_id,
        };
        let mut v = [0u8; 8];
        ioctl_wait(file, SUPUSB_IOCTL_GET_VERSION, &[], &mut v)?;
        let mut claim = [0u8, 0u8];
        ioctl_wait(file, SUPUSB_IOCTL_USB_CLAIM_DEVICE, &[0, 0], &mut claim)?;
        if claim[1] == 0 {
            cap.vbox_instance_id.clear();
            bail!("VBoxUSB refused the claim");
        }
        let mut getdev = [0u8; 8];
        let input = getdev;
        ioctl_wait(file, SUPUSB_IOCTL_GET_DEVICE, &input, &mut getdev).context("GET_DEVICE (high-speed flag)")?;
        Ok(cap)
    }

    pub fn set_config(&self, value: u8) -> Result<()> {
        ioctl_wait(self.file.0, SUPUSB_IOCTL_USB_SET_CONFIG, &[value], &mut [])?;
        Ok(())
    }

    pub fn select_interface(&self, iface: u8, alt: u8) -> Result<()> {
        ioctl_wait(self.file.0, SUPUSB_IOCTL_USB_SELECT_INTERFACE, &[iface, alt], &mut [])?;
        Ok(())
    }

    pub fn clear_endpoint(&self, ep: u8) -> Result<()> {
        ioctl_wait(self.file.0, SUPUSB_IOCTL_USB_CLEAR_ENDPOINT, &[ep], &mut [])?;
        Ok(())
    }

    pub fn abort_endpoint(&self, ep: u8) -> Result<()> {
        ioctl_wait(self.file.0, SUPUSB_IOCTL_USB_ABORT_ENDPOINT, &[ep], &mut [])?;
        Ok(())
    }

    /// Synchronous control transfer (used before the session starts).
    pub fn control_in(&self, setup: [u8; 8], len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 8 + len];
        buf[..8].copy_from_slice(&setup);
        let mut urb = SupUrb { ty: TRANSFER_MSG, ep: 0, dir: DIR_IN, flags: FLAG_SHORT_OK, len: buf.len() as u64, buf: buf.as_mut_ptr() as u64, ..Default::default() };
        let mut out = urb;
        let inp = unsafe { std::slice::from_raw_parts(&urb as *const _ as *const u8, 104) };
        let outp = unsafe { std::slice::from_raw_parts_mut(&mut out as *mut _ as *mut u8, 104) };
        ioctl_wait(self.file.0, SUPUSB_IOCTL_SEND_URB, inp, outp)?;
        urb = out;
        if urb.error != 0 {
            bail!("control transfer failed: USBSUP error {}", { urb.error });
        }
        let n = (urb.len as usize).saturating_sub(8).min(len);
        Ok(buf[8..8 + n].to_vec())
    }
}

impl Drop for Captured {
    /// Gives the device back to Windows: close VBoxUSB, remove the filter, re-enumerate.
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.file.0);
        }
        if !self.vbox_instance_id.is_empty() {
            if let Ok(dn) = devnode(&self.vbox_instance_id) {
                if let Err(e) = remove_subtree(dn) {
                    log::warn!("release: {e:#}");
                }
            }
        }
        let mut out = [0u8; 4];
        if let Err(e) = ioctl_sync(self.mon.0, SUPUSBFLT_IOCTL_REMOVE_FILTER, &self.filter_id.to_le_bytes(), &mut out) {
            log::warn!("release: REMOVE_FILTER failed: {e:#}");
        }
        let id = if self.vbox_instance_id.is_empty() { &self.original_instance_id } else { &self.vbox_instance_id };
        if let Err(e) = cycle_port(id, self.port) {
            log::warn!("release: port cycle failed: {e:#}");
        }
        unsafe {
            let _ = CloseHandle(self.mon.0);
        }
        log::info!("device released back to Windows");
    }
}
