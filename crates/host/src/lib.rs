//! Platform layer of USB-IP-Bridge: device enumeration, exporting and importing USB
//! devices over USB/IP (Windows, Linux).

pub mod descriptors;
#[cfg(any(windows, target_os = "linux"))]
pub mod export;
#[cfg(any(windows, target_os = "linux"))]
pub mod import;
pub mod net;
pub mod system;

#[cfg(windows)]
pub mod windows {
    pub mod audio;
    pub mod devices;
    pub mod import;
    pub mod lease;
    pub mod session;
    pub mod vbox;
}

#[cfg(target_os = "linux")]
pub mod linux {
    pub mod audio;
    pub mod devices;
    pub mod export;
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

/// Can this computer lend its devices to others (is the driver for it there)?
pub fn can_export() -> bool {
    static CAN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CAN.get_or_init(|| {
        #[cfg(windows)]
        return true;
        #[cfg(target_os = "linux")]
        return linux::export::available();
        #[allow(unreachable_code)]
        false
    })
}

/// Can this computer use devices of others? (Windows: is usbip-win2 installed – it can
/// be installed while the service runs.)
pub fn can_import() -> bool {
    #[cfg(windows)]
    {
        windows::import::available()
    }
    #[cfg(not(windows))]
    {
        cfg!(target_os = "linux")
    }
}
