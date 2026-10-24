//! USB descriptor parsing shared by all platforms.

use anyhow::{bail, Result};

/// An endpoint as described in the configuration descriptor.
#[derive(Clone, Copy, Debug)]
pub struct EndpointInfo {
    pub address: u8,
    pub attributes: u8,
    pub max_packet: u16,
    pub interval: u8,
    pub interface: u8,
    pub alt: u8,
}

impl EndpointInfo {
    pub fn transfer_type(&self) -> u8 {
        self.attributes & 0x03 // 0 control, 1 iso, 2 bulk, 3 interrupt
    }
}

/// Parsed configuration descriptor.
#[derive(Clone, Debug, Default)]
pub struct ConfigInfo {
    pub raw: Vec<u8>,
    pub configuration_value: u8,
    pub num_interfaces: u8,
    /// (interface number, alt setting, class, subclass, protocol)
    pub interfaces: Vec<(u8, u8, u8, u8, u8)>,
    pub endpoints: Vec<EndpointInfo>,
}

impl ConfigInfo {
    pub fn parse(raw: Vec<u8>) -> Result<Self> {
        if raw.len() < 9 || raw[1] != 2 {
            bail!("not a configuration descriptor");
        }
        let mut info = ConfigInfo { configuration_value: raw[5], num_interfaces: raw[4], ..Default::default() };
        let (mut cur_if, mut cur_alt) = (0u8, 0u8);
        let mut i = 0;
        while i + 2 <= raw.len() {
            let len = raw[i] as usize;
            if len < 2 || i + len > raw.len() {
                break;
            }
            match raw[i + 1] {
                4 if len >= 9 => {
                    cur_if = raw[i + 2];
                    cur_alt = raw[i + 3];
                    info.interfaces.push((cur_if, cur_alt, raw[i + 5], raw[i + 6], raw[i + 7]));
                }
                5 if len >= 7 => info.endpoints.push(EndpointInfo {
                    address: raw[i + 2],
                    attributes: raw[i + 3],
                    max_packet: u16::from_le_bytes([raw[i + 4], raw[i + 5]]),
                    interval: raw[i + 6],
                    interface: cur_if,
                    alt: cur_alt,
                }),
                _ => {}
            }
            i += len;
        }
        info.raw = raw;
        Ok(info)
    }

    /// Interface that owns an endpoint address (in any alt setting).
    pub fn interface_of(&self, ep_address: u8) -> Option<u8> {
        self.endpoints.iter().find(|e| e.address == ep_address).map(|e| e.interface)
    }

    pub fn endpoint(&self, ep_address: u8, alt_of: impl Fn(u8) -> u8) -> Option<EndpointInfo> {
        self.endpoints
            .iter()
            .find(|e| e.address == ep_address && e.alt == alt_of(e.interface))
            .copied()
    }

    /// Distinct interfaces with their alt-0 class triple, for OP_REP_DEVLIST.
    pub fn interface_classes(&self) -> Vec<(u8, u8, u8)> {
        self.interfaces.iter().filter(|i| i.1 == 0).map(|i| (i.2, i.3, i.4)).collect()
    }
}
