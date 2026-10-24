//! Platform layer of USB-IP-Bridge: device enumeration, exporting (Windows) and
//! importing (Linux) USB devices over USB/IP.

pub mod descriptors;
pub mod net;
pub mod system;

#[cfg(windows)]
pub mod windows {
    pub mod devices;
    pub mod export;
    pub mod session;
    pub mod vbox;
}

#[cfg(target_os = "linux")]
pub mod linux {
    pub mod audio;
    pub mod devices;
    pub mod import;
    pub mod ready;
}

/// USB devices present on this host.
pub fn list_devices() -> Vec<usbipb_core::model::UsbDevice> {
    #[cfg(windows)]
    return windows::devices::list();
    #[cfg(target_os = "linux")]
    return linux::devices::list();
    #[allow(unreachable_code)]
    Vec::new()
}
