//! Device profiles: defaults per device class plus additions per device.
//! Built-in profiles live in `profiles/builtin.toml`; users can add their own files.

use crate::classes::DeviceClass;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    /// Unplugging (or a broken connection) can lose data.
    DataLoss,
    /// Giving it away can lock the user out of this computer.
    Lockout,
    /// Giving it away can cut this computer's own connection.
    NetworkLoss,
    HighBandwidth,
    LatencySensitive,
}

impl Warning {
    pub fn name(self) -> &'static str {
        match self {
            Self::DataLoss => "data_loss",
            Self::Lockout => "lockout",
            Self::NetworkLoss => "network_loss",
            Self::HighBandwidth => "high_bandwidth",
            Self::LatencySensitive => "latency_sensitive",
        }
    }
}

/// Does the device look essential for operating this computer (its profile warns of
/// lockout or network loss: keyboard, mouse, network adapter, Bluetooth)?
pub fn is_essential(vendor_id: u16, product_id: u16, classes: &[DeviceClass]) -> bool {
    static PROFILES: std::sync::OnceLock<Profiles> = std::sync::OnceLock::new();
    let warnings = PROFILES.get_or_init(Profiles::load).resolve(vendor_id, product_id, classes).warnings;
    warnings.iter().any(|w| matches!(w, Warning::Lockout | Warning::NetworkLoss))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyCheck {
    Audio,
    Midi,
    Video,
    Block,
    Tty,
    Input,
    Driver,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reconnect {
    #[default]
    Auto,
    Ask,
}

/// PipeWire settings for audio devices on Linux.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LinuxAudio {
    #[serde(default)]
    pub never_default: Option<bool>,
    #[serde(default)]
    pub pro_audio: Option<bool>,
    #[serde(default)]
    pub headroom: Option<u32>,
    #[serde(default)]
    pub period_size: Option<u32>,
}

impl LinuxAudio {
    fn merge(&mut self, o: &LinuxAudio) {
        self.never_default = o.never_default.or(self.never_default);
        self.pro_audio = o.pro_audio.or(self.pro_audio);
        self.headroom = o.headroom.or(self.headroom);
        self.period_size = o.period_size.or(self.period_size);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassProfile {
    pub class: DeviceClass,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default)]
    pub ready: Option<ReadyCheck>,
    #[serde(default)]
    pub reconnect: Option<Reconnect>,
    #[serde(default)]
    pub linux_audio: Option<LinuxAudio>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    #[serde(default)]
    pub name: Option<String>,
    pub vendor_id: u16,
    pub product_id: u16,
    /// Overrides the classes detected from the descriptors.
    #[serde(default)]
    pub classes: Option<Vec<DeviceClass>>,
    #[serde(default)]
    pub warnings: Vec<Warning>,
    #[serde(default)]
    pub ready: Option<Vec<ReadyCheck>>,
    #[serde(default)]
    pub reconnect: Option<Reconnect>,
    #[serde(default)]
    pub blocking_processes_windows: Vec<String>,
    #[serde(default)]
    pub blocking_processes_linux: Vec<String>,
    #[serde(default)]
    pub linux_audio: Option<LinuxAudio>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profiles {
    #[serde(default, rename = "class")]
    pub classes: Vec<ClassProfile>,
    #[serde(default, rename = "device")]
    pub devices: Vec<DeviceProfile>,
}

/// Everything USB-IP-Bridge knows about how to treat one device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Effective {
    pub name: Option<String>,
    pub classes: Vec<DeviceClass>,
    pub warnings: Vec<Warning>,
    pub ready: Vec<ReadyCheck>,
    pub reconnect: Reconnect,
    pub blocking_processes: Vec<String>,
    pub linux_audio: Option<LinuxAudio>,
}

const BUILTIN: &str = include_str!("../profiles/builtin.toml");

pub fn user_profile_dir() -> PathBuf {
    crate::config::config_dir().join("profiles")
}

impl Profiles {
    pub fn builtin() -> Self {
        toml::from_str(BUILTIN).expect("built-in profiles are valid")
    }

    /// Built-in profiles plus the user's profile files (theirs take precedence).
    pub fn load() -> Self {
        let mut p = Self::builtin();
        if let Ok(dir) = std::fs::read_dir(user_profile_dir()) {
            let mut files: Vec<_> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
            files.sort();
            for f in files {
                match std::fs::read_to_string(&f).map_err(anyhow::Error::from).and_then(|t| Ok(toml::from_str::<Profiles>(&t)?)) {
                    Ok(user) => {
                        p.classes.extend(user.classes);
                        p.devices.extend(user.devices);
                    }
                    Err(e) => log::warn!("profile {}: {e:#}", f.display()),
                }
            }
        }
        p
    }

    /// Resolves the profile for a device with the given detected classes.
    pub fn resolve(&self, vendor_id: u16, product_id: u16, detected: &[DeviceClass]) -> Effective {
        // later entries (user files) win
        let device = self.devices.iter().rev().find(|d| d.vendor_id == vendor_id && d.product_id == product_id);
        let classes = device.and_then(|d| d.classes.clone()).unwrap_or_else(|| detected.to_vec());
        let mut e = Effective { classes: classes.clone(), ..Default::default() };
        for c in &classes {
            if let Some(cp) = self.classes.iter().rev().find(|cp| cp.class == *c) {
                e.warnings.extend(cp.warnings.iter().copied());
                e.ready.extend(cp.ready);
                if cp.reconnect == Some(Reconnect::Ask) {
                    e.reconnect = Reconnect::Ask;
                }
                if let Some(la) = &cp.linux_audio {
                    e.linux_audio.get_or_insert_with(Default::default).merge(la);
                }
            }
        }
        if let Some(d) = device {
            e.name = d.name.clone();
            e.warnings.extend(d.warnings.iter().copied());
            if let Some(r) = &d.ready {
                e.ready = r.clone();
            }
            if let Some(r) = d.reconnect {
                e.reconnect = r;
            }
            e.blocking_processes = if cfg!(windows) { d.blocking_processes_windows.clone() } else { d.blocking_processes_linux.clone() };
            if let Some(la) = &d.linux_audio {
                e.linux_audio.get_or_insert_with(Default::default).merge(la);
            }
        }
        e.warnings.sort_by_key(|w| *w as u8);
        e.warnings.dedup();
        let mut seen = Vec::new();
        e.ready.retain(|r| if seen.contains(r) { false } else { seen.push(*r); true });
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceClass::*;

    #[test]
    fn builtin_parses_and_xr18_resolves() {
        let p = Profiles::builtin();
        let e = p.resolve(0x1397, 0x00d4, &[Audio, Midi]);
        assert_eq!(e.name.as_deref(), Some("Behringer X18/XR18"));
        assert_eq!(e.ready, vec![ReadyCheck::Audio, ReadyCheck::Midi]);
        let la = e.linux_audio.unwrap();
        assert_eq!((la.headroom, la.period_size, la.never_default), (Some(256), Some(256), Some(true)));
        if cfg!(windows) {
            assert_eq!(e.blocking_processes, vec!["X18AudioCplApp.exe".to_string()]);
        }
    }

    #[test]
    fn storage_asks_before_reconnect() {
        let e = Profiles::builtin().resolve(0x1234, 0x5678, &[Storage]);
        assert_eq!(e.reconnect, Reconnect::Ask);
        assert_eq!(e.warnings, vec![Warning::DataLoss]);
    }

    #[test]
    fn device_override_wins() {
        let mut p = Profiles::builtin();
        p.devices.push(DeviceProfile {
            name: Some("Mine".into()),
            vendor_id: 0x1397,
            product_id: 0x00d4,
            classes: None,
            warnings: vec![],
            ready: Some(vec![ReadyCheck::Driver]),
            reconnect: None,
            blocking_processes_windows: vec![],
            blocking_processes_linux: vec![],
            linux_audio: Some(LinuxAudio { headroom: Some(512), ..Default::default() }),
        });
        let e = p.resolve(0x1397, 0x00d4, &[Audio]);
        assert_eq!(e.name.as_deref(), Some("Mine"));
        assert_eq!(e.ready, vec![ReadyCheck::Driver]);
        assert_eq!(e.linux_audio.unwrap().headroom, Some(512));
    }
}
