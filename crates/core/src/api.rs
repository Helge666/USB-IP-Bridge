//! Local API between the service and GUI/CLI: one JSON request per line, one JSON
//! response per line, on 127.0.0.1:`api_port`.

use crate::classes::DeviceClass;
use crate::model::{DeviceMatch, UsbDevice};
use crate::peer::PeerStatus;
use crate::profiles::Warning;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    /// Attach import rule `index` (all rules if None) and keep it attached.
    Attach { index: Option<usize> },
    /// Release import rule `index` (all if None) back to its owner.
    Detach { index: Option<usize> },
    /// Share a device of this computer with others, or stop sharing it.
    Share { device: DeviceMatch, share: bool },
    /// Use a device of another computer here (`host`: its address, `port`: its USB/IP
    /// port), or give it back (`attach: false`).
    Use { host: String, port: u16, device: DeviceMatch, attach: bool },
    /// Give a device of this computer to another computer (`peer`: its control address).
    Give { peer: String, device: DeviceMatch },
    /// Ask the computer that uses a device of this computer to give it back.
    TakeBack { device: DeviceMatch },
    /// Add a computer that discovery does not find (name or address, optional ":port").
    AddPeer { address: String },
    /// Remove a computer added with `AddPeer`.
    RemovePeer { address: String },
    /// Linux: make the audio rules active (restarts WirePlumber while no audio is in use).
    ReloadAudio,
    /// Pair with another computer (`address`: its control address). This computer shows a
    /// code (see `Status::pairings`) that the user types on the other computer.
    Pair { address: String },
    /// Answer another computer's pairing request `id` with the code it shows, or decline it
    /// (`code: None`).
    PairAnswer { id: u64, code: Option<String> },
    /// Remove the pairing with a computer (also on that computer, if it is reachable).
    Unpair { host_id: String },
    /// Lock a device of this computer (never shared) or unlock it.
    Lock { device: DeviceMatch, lock: bool },
}

/// Which devices of this computer are never shared, and why.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Locks {
    /// Locked in config.toml (`never_share`; cannot be unlocked in the app).
    pub config: Vec<DeviceMatch>,
    /// Locked in the app.
    pub user: Vec<DeviceMatch>,
    /// Unlocked in the app although they look essential (keyboard, mouse, network, Bluetooth
    /// are locked automatically otherwise).
    pub unlocked: Vec<DeviceMatch>,
}

/// A pairing in progress (or just finished), as shown to the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pairing {
    pub id: u64,
    /// Started on this computer (it shows `code`) or asked for by the other one (the user
    /// types the code shown there).
    pub outgoing: bool,
    pub address: String,
    pub host_name: String,
    pub code: Option<String>,
    pub state: PairingState,
    /// Why it failed.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    Waiting,
    Paired,
    Failed,
}

/// A computer this computer is paired with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairedComputer {
    pub host_id: String,
    pub host_name: String,
    /// Its public key (hex).
    pub key: String,
    /// Unix time of the pairing.
    pub since: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub host: String,
    #[serde(default)]
    pub host_id: String,
    pub exports: Vec<ExportStatus>,
    pub imports: Vec<ImportStatus>,
    /// Other computers running USB-IP-Bridge.
    #[serde(default)]
    pub peers: Vec<PeerStatus>,
    /// Devices this computer shares (configuration plus changes made in the GUI).
    #[serde(default)]
    pub shared: Vec<DeviceMatch>,
    /// Linux: PipeWire rules for attached audio devices.
    #[serde(default)]
    pub audio_rules: Option<AudioRules>,
    /// Fingerprint of this computer's key.
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub paired: Vec<PairedComputer>,
    #[serde(default)]
    pub pairings: Vec<Pairing>,
    #[serde(default)]
    pub locks: Locks,
    /// This computer's operating system and hardware.
    #[serde(default)]
    pub system: Option<crate::peer::SystemInfo>,
}

impl Locks {
    /// The configuration with these locks in effect (for showing what is locked).
    pub fn apply(&self, config: &mut crate::config::Config) {
        config.never_share = self.config.iter().chain(&self.user).cloned().collect();
        config.unlocked = self.unlocked.clone();
        config.lock_essential = true;
    }
}

/// PipeWire/WirePlumber rules for audio devices attached over USB/IP (Linux).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioRules {
    pub installed: bool,
    /// Every running WirePlumber has read them (it reads rules only when it starts).
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportStatus {
    pub device: Option<UsbDevice>,
    pub rule: String,
    /// Remote address of the attached client, if any.
    pub client: Option<String>,
    /// Host name of that client (reverse DNS), if known.
    pub client_name: Option<String>,
    pub note: Option<String>,
    /// Why the last attach attempt failed on this host (e.g. a program holds the device).
    pub problem: Option<String>,
    /// Seconds since `problem` was recorded.
    pub problem_age_secs: Option<u64>,
    /// Who asked for the device when `problem` happened.
    pub problem_client: Option<String>,
    pub problem_client_name: Option<String>,
    /// Programs configured as possible blockers for this device.
    pub blocking_processes: Vec<String>,
    /// The device is plugged in (or lent). If not, `device` is how it looked when it was
    /// last seen, if known.
    #[serde(default = "crate::model::yes")]
    pub present: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportState {
    /// Not wanted: the device stays with its owner.
    Idle,
    Connecting,
    Attached,
    Waiting,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStatus {
    pub remote: String,
    pub rule: String,
    pub state: ImportState,
    pub bus_id: Option<String>,
    /// Seconds since the last state change.
    pub since_secs: u64,
    pub attach_count: u64,
    pub last_error: Option<String>,
    /// The user wants this device attached here.
    pub wanted: bool,
    /// Device name, as reported by the remote host or read from the attached device.
    #[serde(default)]
    pub device_name: Option<String>,
    /// Name of the remote host, as reported by it.
    #[serde(default)]
    pub remote_name: Option<String>,
    /// The device this import is for.
    #[serde(default)]
    pub device: Option<DeviceMatch>,
    /// Device classes, as reported by the remote host or read from the attached device.
    #[serde(default)]
    pub classes: Vec<DeviceClass>,
    /// Warnings from the device profile.
    #[serde(default)]
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Status(Status),
    Ok { message: String },
    Error { message: String },
}

/// Sends one request to the local service and returns its response.
pub fn call(port: u16, req: &Request) -> Result<Response> {
    let addr = ("127.0.0.1", port);
    let mut s = TcpStream::connect(addr).with_context(|| format!("service not reachable on 127.0.0.1:{port} (is it running?)"))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    if reply.is_empty() {
        bail!("empty reply from service");
    }
    Ok(serde_json::from_str(&reply)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_format() {
        assert_eq!(serde_json::to_string(&Request::Status).unwrap(), r#"{"cmd":"status"}"#);
    }
}
