//! Windows makes a newly arriving audio device the default one (playback and recording) –
//! the sound of games and programs would move to a device of another computer. Like the
//! WirePlumber rule on Linux: the default devices from before stay the default devices.

use std::collections::HashSet;
use std::time::{Duration, Instant};
use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};

/// Sets default devices; not documented, but the way the sound settings of Windows do it.
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn get_mix_format(&self) -> HRESULT;
    fn get_device_format(&self) -> HRESULT;
    fn reset_device_format(&self) -> HRESULT;
    fn set_device_format(&self) -> HRESULT;
    fn get_processing_period(&self) -> HRESULT;
    fn set_processing_period(&self) -> HRESULT;
    fn get_share_mode(&self) -> HRESULT;
    fn set_share_mode(&self) -> HRESULT;
    fn get_property_value(&self) -> HRESULT;
    fn set_property_value(&self) -> HRESULT;
    fn set_default_endpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
}

const POLICY_CONFIG_CLIENT: windows::core::GUID = windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const SLOTS: [(EDataFlow, ERole); 6] =
    [(eRender, eConsole), (eRender, eMultimedia), (eRender, eCommunications), (eCapture, eConsole), (eCapture, eMultimedia), (eCapture, eCommunications)];

/// The default devices and all active audio devices at one moment.
pub struct Defaults {
    slots: Vec<Option<String>>,
    present: HashSet<String>,
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
    }
}

fn id_of(d: &IMMDevice) -> Option<String> {
    unsafe {
        let p = d.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

pub fn defaults() -> Option<Defaults> {
    let e = enumerator()?;
    unsafe {
        let slots = SLOTS.iter().map(|(f, r)| e.GetDefaultAudioEndpoint(*f, *r).ok().and_then(|d| id_of(&d))).collect();
        let all = e.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE).ok()?;
        let present = (0..all.GetCount().ok()?).filter_map(|i| all.Item(i).ok().and_then(|d| id_of(&d))).collect();
        Some(Defaults { slots, present })
    }
}

/// For a while after a device arrived: wherever a new audio device became the default,
/// the one from `before` is made the default again.
pub fn keep(before: Defaults, name: String) {
    std::thread::spawn(move || {
        let policy = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
        };
        let Ok(policy) = policy else {
            log::warn!("{name}: cannot keep the default audio devices (no policy interface)");
            return;
        };
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(250));
            let Some(now) = defaults() else { continue };
            for (i, (cur, old)) in now.slots.iter().zip(&before.slots).enumerate() {
                let (Some(cur), Some(old)) = (cur, old) else { continue };
                if cur != old && !before.present.contains(cur) {
                    let id = windows::core::HSTRING::from(old.as_str());
                    let r = unsafe { policy.set_default_endpoint(PCWSTR(id.as_ptr()), SLOTS[i].1) };
                    log::info!("{name}: Windows made the new audio device the default, set back ({r:?})");
                }
            }
        }
    });
}
