//! Lending a device on Windows: VBoxUSB captures it from Windows and this process
//! serves it over USB/IP (session.rs).

use super::session::{descriptors, run_session};
use super::vbox::{port_speed, Captured};
use crate::descriptors::ConfigInfo;
use anyhow::Result;
use std::io::BufReader;
use std::net::TcpStream;
use std::sync::Arc;
use usbip_protocol::{InterfaceInfo, Speed};
use usbipb_core::model::UsbDevice;

/// A device taken from Windows for another computer; Windows gets it back when dropped.
pub struct Lease {
    cap: Arc<Captured>,
    config: ConfigInfo,
    dev_desc: [u8; 18],
}

fn speed(code: Option<u8>) -> Speed {
    match code {
        Some(0) => Speed::Low,
        Some(2) => Speed::High,
        Some(3) => Speed::Super,
        _ => Speed::Full,
    }
}

/// The hub port is the second part of the bus id ("1-4").
fn port(d: &UsbDevice) -> u16 {
    d.bus_id.split('-').nth(1).and_then(|s| s.parse().ok()).unwrap_or(1)
}

/// Bus number, device number, speed and interfaces (first alternate setting) for the
/// device list.
pub fn wire_details(d: &UsbDevice) -> (u32, u32, Speed, Vec<InterfaceInfo>) {
    let busnum = d.bus_id.split('-').next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let port = port(d);
    let interfaces = super::devices::interfaces(&d.platform_id, port)
        .into_iter()
        .map(|(class, subclass, protocol)| InterfaceInfo { class, subclass, protocol })
        .collect();
    (busnum, port as u32, speed(port_speed(&d.platform_id, port)), interfaces)
}

impl Lease {
    pub fn capture(d: &UsbDevice) -> Result<Self> {
        let cap = Arc::new(Captured::capture(&d.platform_id)?);
        let (dev_desc, config) = descriptors(&cap)?;
        Ok(Self { cap, config, dev_desc })
    }

    /// Fills in what only the captured device tells (descriptors, actual speed).
    pub fn complete(&self, info: &mut usbip_protocol::UsbDevice) -> Result<()> {
        let dd = &self.dev_desc;
        info.bcd_device = u16::from_le_bytes([dd[12], dd[13]]);
        info.device_class = dd[4];
        info.device_subclass = dd[5];
        info.device_protocol = dd[6];
        info.configuration_value = self.config.configuration_value;
        info.num_configurations = dd[17];
        info.num_interfaces = self.config.num_interfaces;
        info.speed = speed(port_speed(&self.cap.vbox_instance_id, self.cap.port));
        Ok(())
    }

    /// Before the import reply: nothing to do, this process serves the connection.
    pub fn hand_over(&self, _stream: &TcpStream) -> Result<()> {
        Ok(())
    }

    /// Serves the device until the other computer lets it go or the connection ends.
    pub fn serve(self, speed: Speed, stream: TcpStream, reader: BufReader<TcpStream>) -> Result<()> {
        run_session(self.cap, self.config, speed, stream, reader)
    }
}
