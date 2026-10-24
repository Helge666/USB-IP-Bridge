//! Device classes derived from USB class codes (device and interface descriptors).

use serde::{Deserialize, Serialize};

/// What kind of device this is, as far as USB-IP-Bridge cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Audio,
    Midi,
    Video,
    Storage,
    Keyboard,
    Mouse,
    /// Other HID devices (tablets, game controllers, vendor controls).
    Hid,
    Serial,
    Network,
    Bluetooth,
    Printer,
    /// Still image / MTP / PTP (cameras, phones).
    Image,
    SmartCard,
    Hub,
    /// Firmware update interface (DFU).
    Firmware,
    VendorSpecific,
}

impl DeviceClass {
    pub const ALL: [DeviceClass; 16] = [
        Self::Audio,
        Self::Midi,
        Self::Video,
        Self::Storage,
        Self::Keyboard,
        Self::Mouse,
        Self::Hid,
        Self::Serial,
        Self::Network,
        Self::Bluetooth,
        Self::Printer,
        Self::Image,
        Self::SmartCard,
        Self::Hub,
        Self::Firmware,
        Self::VendorSpecific,
    ];

    /// Stable name, as in profiles and on the wire ("audio", "smart_card", ...).
    pub fn name(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Midi => "midi",
            Self::Video => "video",
            Self::Storage => "storage",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Hid => "hid",
            Self::Serial => "serial",
            Self::Network => "network",
            Self::Bluetooth => "bluetooth",
            Self::Printer => "printer",
            Self::Image => "image",
            Self::SmartCard => "smart_card",
            Self::Hub => "hub",
            Self::Firmware => "firmware",
            Self::VendorSpecific => "vendor_specific",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// "audio,midi" – compact form for the USB/IP path field.
pub fn join_names(classes: &[DeviceClass]) -> String {
    classes.iter().map(|c| c.name()).collect::<Vec<_>>().join(",")
}

pub fn parse_names(s: &str) -> Vec<DeviceClass> {
    s.split(',').filter_map(|n| DeviceClass::from_name(n.trim())).collect()
}

/// (class, subclass, protocol) of an interface.
pub type InterfaceTriple = (u8, u8, u8);

fn class_of_interface((class, sub, proto): InterfaceTriple) -> Option<DeviceClass> {
    use DeviceClass::*;
    Some(match class {
        0x01 if sub == 0x03 => Midi,
        0x01 => Audio,
        0x02 if sub == 0x02 => Serial,
        0x02 if matches!(sub, 0x06 | 0x0a | 0x0b | 0x0c | 0x0d | 0x0e) => Network,
        0x02 => Serial,
        0x03 if sub == 0x01 && proto == 0x01 => Keyboard,
        0x03 if sub == 0x01 && proto == 0x02 => Mouse,
        0x03 => Hid,
        0x06 => Image,
        0x07 => Printer,
        0x08 => Storage,
        0x09 => Hub,
        0x0a => return None, // CDC data, belongs to a CDC control interface
        0x0b => SmartCard,
        0x0e | 0x10 => Video,
        0xe0 if sub == 0x01 && proto == 0x01 => Bluetooth,
        0xe0 => Network,
        0xfe if sub == 0x01 => Firmware,
        0xff => VendorSpecific,
        _ => return None,
    })
}

/// Classes of a device from its device class and its interfaces (sorted, unique).
/// Vendor-specific and firmware interfaces only count if nothing else is known.
pub fn classify(device_class: u8, interfaces: &[InterfaceTriple]) -> Vec<DeviceClass> {
    // USB MIDI devices carry an audio *control* interface but no audio data: count as
    // audio only if there is an audio streaming interface (subclass 2).
    let has_audio_streaming = interfaces.iter().any(|&(c, s, _)| c == 0x01 && s == 0x02);
    let mut out: Vec<DeviceClass> = interfaces
        .iter()
        .filter(|&&(c, s, _)| !(c == 0x01 && s == 0x01 && !has_audio_streaming))
        .filter_map(|&i| class_of_interface(i))
        .collect();
    if device_class == 0x09 {
        out.push(DeviceClass::Hub);
    }
    out.sort();
    out.dedup();
    let specific: Vec<DeviceClass> =
        out.iter().copied().filter(|c| !matches!(c, DeviceClass::VendorSpecific | DeviceClass::Firmware)).collect();
    if specific.is_empty() {
        out
    } else {
        specific
    }
}

/// Parses "Class_01&SubClass_03&Prot_00" style parts of Windows compatible IDs.
pub fn parse_windows_compatible_id(id: &str) -> Option<InterfaceTriple> {
    let up = id.to_ascii_uppercase();
    let hex = |key: &str| {
        let i = up.find(key)? + key.len();
        u8::from_str_radix(up.get(i..i + 2)?, 16).ok()
    };
    if up.contains("DEVCLASS_") {
        return None;
    }
    Some((hex("CLASS_")?, hex("SUBCLASS_")?, hex("PROT_")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceClass::*;

    #[test]
    fn xr18_is_audio_and_midi() {
        // audio control, 2x streaming, audio control (MIDI), MIDI streaming, DFU
        let ifs = [(1, 1, 0x20), (1, 2, 0x20), (1, 2, 0x20), (1, 1, 0), (1, 3, 0), (0xfe, 1, 1)];
        assert_eq!(classify(0xef, &ifs), vec![Audio, Midi]);
    }

    #[test]
    fn usb_midi_is_not_audio() {
        assert_eq!(classify(0, &[(1, 1, 0), (1, 3, 0)]), vec![Midi]);
    }

    #[test]
    fn keyboard_and_vendor() {
        assert_eq!(classify(0, &[(3, 1, 1), (0xff, 0, 0)]), vec![Keyboard]);
        assert_eq!(classify(0, &[(0xff, 0, 0)]), vec![VendorSpecific]);
        assert_eq!(classify(0, &[(0x0e, 1, 0), (0x0e, 2, 0), (1, 1, 0), (1, 2, 0), (3, 0, 0)]), vec![Audio, Video, Hid]);
    }

    #[test]
    fn names_roundtrip() {
        for c in DeviceClass::ALL {
            assert_eq!(DeviceClass::from_name(c.name()), Some(c));
            assert_eq!(toml::Value::try_from(c).unwrap().as_str(), Some(c.name()));
        }
        assert_eq!(parse_names("audio,midi,bogus"), vec![Audio, Midi]);
    }

    #[test]
    fn windows_compatible_ids() {
        assert_eq!(parse_windows_compatible_id(r"USB\Class_01&SubClass_03&Prot_00"), Some((1, 3, 0)));
        assert_eq!(parse_windows_compatible_id(r"USB\DevClass_EF&SubClass_02&Prot_01"), None);
        assert_eq!(parse_windows_compatible_id(r"USB\COMPOSITE"), None);
    }
}
