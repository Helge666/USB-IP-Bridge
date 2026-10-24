//! Control protocol between USB-IP-Bridge services on different computers.
//!
//! Discovery: every service sends a small JSON beacon to UDP `PEER_PORT` as broadcast (and
//! to manually configured hosts); a beacon with `query` set asks the others to answer at
//! once. Control: one JSON request per line, one JSON response per line, on TCP `PEER_PORT`,
//! inside an encrypted channel (see `secure`). Only paired computers get answers beyond
//! `Hello` and the pairing itself.

use crate::classes::DeviceClass;
use crate::model::UsbDevice;
use crate::profiles::Warning;
use crate::secure::{Identity, SecureStream};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Version of this protocol (independent of the app version).
pub const PROTOCOL_VERSION: u32 = 2;
/// UDP discovery and TCP control port.
pub const PEER_PORT: u16 = 3244;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Beacon {
    /// Always "usb-ip-bridge", so stray packets are ignored.
    pub app: String,
    pub protocol: u32,
    pub host_id: String,
    pub host_name: String,
    /// TCP control port.
    pub port: u16,
    /// Asks receivers to answer with their own beacon right away.
    #[serde(default)]
    pub query: bool,
}

pub const BEACON_APP: &str = "usb-ip-bridge";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum PeerRequest {
    Hello,
    Devices,
    /// "Use this device of mine": the receiver attaches it from the sender's address.
    Attach { device: crate::model::DeviceMatch, usbip_port: u16 },
    /// "Give my device back": the receiver releases it (after unmounting file systems).
    Release { device: crate::model::DeviceMatch },
    /// "Let us pair": the receiver asks its user for the code shown on the sender.
    /// `pake`: the sender's SPAKE2 message (hex).
    PairStart { host_id: String, host_name: String, pake: String },
    /// Proof that the sender derived the same key (after `PairReply`); empty if it did not.
    PairConfirm { confirm: String },
    /// "I removed the pairing": the receiver removes it, too.
    Unpair,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hello {
    pub protocol: u32,
    pub host_id: String,
    pub host_name: String,
    pub version: String,
    /// Authentication method: "pairing" (encrypted channel, paired computers only).
    pub auth: String,
    /// The asking computer is in this computer's list of paired computers.
    #[serde(default)]
    pub paired: bool,
    pub usbip_port: u16,
    /// This host can share its devices / use devices of others.
    pub can_export: bool,
    pub can_import: bool,
    /// What kind of computer it is (for the details shown to the user).
    #[serde(default)]
    pub system: Option<SystemInfo>,
}

/// Operating system and hardware of a computer, as text for the user.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    /// e.g. "Windows 11 Pro 25H2 (build 26300)", "Ubuntu 24.04.5 LTS (kernel 7.0.0-38-generic)"
    pub os: Option<String>,
    pub cpu: Option<String>,
    /// Logical processors.
    pub threads: Option<u32>,
    pub gpus: Vec<String>,
    /// Installed memory in MiB.
    pub ram_mb: Option<u64>,
}

/// How a device is offered to other computers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Offer {
    /// Shared: others may use it.
    Shared,
    /// Not shared (yet).
    NotShared,
    /// The owner never shares it (e.g. its keyboard).
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeerDevice {
    pub device: UsbDevice,
    /// Name from the device profile, else the device's own name.
    pub name: String,
    pub classes: Vec<DeviceClass>,
    pub warnings: Vec<Warning>,
    pub offer: Offer,
    /// Host name of the computer currently using it, if lent.
    #[serde(default)]
    pub used_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PeerResponse {
    Hello(Hello),
    Devices { devices: Vec<PeerDevice> },
    Ok { message: String },
    Error { message: String },
    /// Answer to `PairStart` once the user typed a code: the receiver's SPAKE2 message and
    /// its proof of the derived key (both hex).
    PairReply { pake: String, confirm: String },
    /// The asking computer is not paired with this one.
    NotPaired,
}

/// Another computer as this service sees it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeerStatus {
    /// "host:port" of its control port.
    pub address: String,
    pub hello: Option<Hello>,
    pub online: bool,
    /// Seconds since it last answered.
    pub last_seen_secs: Option<u64>,
    /// Added by hand (config `peers`, import rules, GUI) instead of found in the network.
    pub manual: bool,
    /// Added in the GUI (can be removed there).
    #[serde(default)]
    pub added: bool,
    /// Can be removed from the list (added in the GUI, or found earlier and now offline).
    #[serde(default)]
    pub removable: bool,
    /// Host id and name from an earlier contact (while `hello` is not known yet).
    #[serde(default)]
    pub known_id: Option<String>,
    #[serde(default)]
    pub known_name: Option<String>,
    pub devices: Vec<PeerDevice>,
    pub error: Option<String>,
    /// This computer has it in its list of paired computers (see also `hello.paired`).
    #[serde(default)]
    pub paired: bool,
    /// Fingerprint of its key, once it answered.
    #[serde(default)]
    pub fingerprint: Option<String>,
}

impl PeerStatus {
    pub fn name(&self) -> String {
        self.hello
            .as_ref()
            .map(|h| h.host_name.clone())
            .or_else(|| self.known_name.clone())
            .unwrap_or_else(|| self.address.split(':').next().unwrap_or_default().to_string())
    }
}

/// Opens an encrypted control connection; several requests can be sent over it.
pub struct PeerClient {
    reader: BufReader<SecureStream>,
}

impl PeerClient {
    pub fn connect(address: &str, id: &Identity) -> Result<Self> {
        let addr: SocketAddr = address.to_socket_addrs()?.next().with_context(|| format!("cannot resolve {address}"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).with_context(|| format!("connect {address}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_nodelay(true)?;
        let secure = SecureStream::connect(stream, id).with_context(|| format!("secure connection to {address}"))?;
        Ok(Self { reader: BufReader::new(secure) })
    }

    /// The connection (its keys and handshake hash).
    pub fn conn(&self) -> &SecureStream {
        self.reader.get_ref()
    }

    /// How long to wait for an answer (default 5 s).
    pub fn set_timeout(&self, t: Duration) -> Result<()> {
        Ok(self.conn().tcp().set_read_timeout(Some(t))?)
    }

    /// Sends a request that expects `Ok`; other answers become errors.
    pub fn call_ok(&mut self, req: &PeerRequest) -> Result<String> {
        match self.call(req)? {
            PeerResponse::Ok { message } => Ok(message),
            PeerResponse::Error { message } => bail!(message),
            PeerResponse::NotPaired => bail!("the other computer is not paired with this one (pair them first)"),
            other => bail!("unexpected answer {other:?}"),
        }
    }

    pub fn call(&mut self, req: &PeerRequest) -> Result<PeerResponse> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.reader.get_mut().write_all(line.as_bytes())?;
        let mut answer = String::new();
        if self.reader.read_line(&mut answer)? == 0 {
            bail!("connection closed");
        }
        Ok(serde_json::from_str(&answer)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        assert_eq!(serde_json::to_string(&PeerRequest::Devices).unwrap(), r#"{"cmd":"devices"}"#);
        let r: PeerResponse = serde_json::from_str(r#"{"result":"error","message":"x"}"#).unwrap();
        assert!(matches!(r, PeerResponse::Error { .. }));
    }
}
