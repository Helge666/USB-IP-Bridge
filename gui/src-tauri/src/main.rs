//! USB-IP-Bridge desktop app: shows this computer's devices and what the local service
//! shares and attaches; offers to close programs that block sharing.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};
use usbipb_core::api::{self, Request, Response, Status};
use usbipb_core::config::{config_path, Config};
use usbipb_core::classes::DeviceClass;
use usbipb_core::model::UsbDevice;
use usbipb_core::profiles::{Effective, Profiles, Warning};

/// Programs this app closed to free a device, restarted once the device is back.
#[derive(Default)]
struct Closed(Mutex<Vec<String>>);

#[derive(Serialize)]
struct DeviceRow {
    #[serde(flatten)]
    device: UsbDevice,
    name: String,
    shared: bool,
    blocked: bool,
    /// Why it is locked: "config" (config.toml), "user" (locked in the app) or "auto"
    /// (looks essential: keyboard, mouse, network, Bluetooth).
    lock: Option<&'static str>,
    /// Classes and warnings from the device profile.
    kind: Vec<DeviceClass>,
    warnings: Vec<Warning>,
    /// Everything the device profile says (for the details pane).
    profile: Effective,
}

fn config() -> Config {
    Config::load(&config_path()).unwrap_or_default()
}

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({ "title": usbipb_core::title(), "version": usbipb_core::VERSION, "can_share": usbipb_host::can_export(), "can_import": usbipb_host::can_import() })
}

#[tauri::command]
fn list_devices() -> Vec<DeviceRow> {
    let mut cfg = config();
    let mut locks = usbipb_core::api::Locks { config: cfg.never_share.clone(), ..Default::default() };
    // What is shared and locked comes from the service (it includes changes made in the app).
    if let Ok(Response::Status(s)) = api::call(cfg.api_port, &Request::Status) {
        cfg.export = s.shared.into_iter().map(|device| usbipb_core::config::ExportRule { device, allowed_clients: vec![], blocking_processes: vec![] }).collect();
        locks = s.locks;
    }
    locks.apply(&mut cfg);
    let profiles = Profiles::load();
    usbipb_host::list_devices()
        .into_iter()
        .map(|d| {
            let p = profiles.resolve(d.vendor_id, d.product_id, &d.classes);
            let blocked = cfg.is_blocked(&d);
            let lock = blocked.then(|| {
                if locks.config.iter().any(|m| m.matches(&d)) {
                    "config"
                } else if locks.user.iter().any(|m| m.matches(&d)) {
                    "user"
                } else {
                    "auto"
                }
            });
            DeviceRow {
                name: p.name.clone().unwrap_or_else(|| d.display_name()),
                shared: cfg.export.iter().any(|r| r.device.matches(&d)),
                blocked,
                lock,
                kind: p.classes.clone(),
                warnings: p.warnings.clone(),
                profile: p,
                device: d,
            }
        })
        .collect()
}

#[tauri::command]
fn service_status() -> Result<Status, String> {
    match api::call(config().api_port, &Request::Status).map_err(|e| format!("{e:#}"))? {
        Response::Status(s) => Ok(s),
        Response::Error { message } | Response::Ok { message } => Err(message),
    }
}

#[tauri::command]
fn set_attached(index: usize, attach: bool) -> Result<String, String> {
    let req = if attach { Request::Attach { index: Some(index) } } else { Request::Detach { index: Some(index) } };
    match api::call(config().api_port, &req).map_err(|e| format!("{e:#}"))? {
        Response::Ok { message } => Ok(message),
        Response::Error { message } => Err(message),
        Response::Status(_) => Ok(String::new()),
    }
}

fn call(req: Request) -> Result<String, String> {
    match api::call(config().api_port, &req).map_err(|e| format!("{e:#}"))? {
        Response::Ok { message } => Ok(message),
        Response::Error { message } => Err(message),
        Response::Status(_) => Ok(String::new()),
    }
}

fn device_match(vendor_id: u16, product_id: u16, serial: Option<String>) -> usbipb_core::model::DeviceMatch {
    usbipb_core::model::DeviceMatch { vendor_id, product_id, serial }
}

/// The device profile for a device (of any computer), for the details pane.
#[tauri::command]
fn profile(vendor_id: u16, product_id: u16, classes: Vec<DeviceClass>) -> Effective {
    Profiles::load().resolve(vendor_id, product_id, &classes)
}

/// Linux: makes the audio rules active (the service restarts WirePlumber while no audio
/// is in use, and refuses otherwise).
#[tauri::command]
fn reload_audio() -> Result<String, String> {
    call(Request::ReloadAudio)
}

/// Adds a computer that is not found automatically (name or address).
#[tauri::command]
fn add_peer(address: String) -> Result<String, String> {
    call(Request::AddPeer { address })
}

/// Removes a computer added with `add_peer`.
#[tauri::command]
fn remove_peer(address: String) -> Result<String, String> {
    call(Request::RemovePeer { address })
}

/// Starts pairing with another computer; the code to type there appears in the status.
#[tauri::command]
fn pair(address: String) -> Result<String, String> {
    call(Request::Pair { address })
}

/// Answers another computer's pairing request with the code shown there (None: decline).
#[tauri::command]
fn pair_answer(id: u64, code: Option<String>) -> Result<String, String> {
    call(Request::PairAnswer { id, code })
}

#[tauri::command]
fn unpair(host_id: String) -> Result<String, String> {
    call(Request::Unpair { host_id })
}

/// Makes the window ask for attention (e.g. another computer wants to pair).
#[tauri::command]
fn attention(window: tauri::WebviewWindow) {
    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
}

/// Uses a device of another computer here, or gives it back.
#[tauri::command]
fn use_device(host: String, port: u16, vendor_id: u16, product_id: u16, serial: Option<String>, attach: bool) -> Result<String, String> {
    call(Request::Use { host, port, device: device_match(vendor_id, product_id, serial), attach })
}

/// Gives a device of this computer to another computer.
#[tauri::command]
fn give(peer: String, vendor_id: u16, product_id: u16, serial: Option<String>) -> Result<String, String> {
    call(Request::Give { peer, device: device_match(vendor_id, product_id, serial) })
}

/// Locks a device of this computer (never shared) or unlocks it.
#[tauri::command]
fn set_locked(vendor_id: u16, product_id: u16, serial: Option<String>, lock: bool) -> Result<String, String> {
    call(Request::Lock { device: device_match(vendor_id, product_id, serial), lock })
}

/// Asks the computer using a device of this computer to give it back.
#[tauri::command]
fn take_back(vendor_id: u16, product_id: u16, serial: Option<String>) -> Result<String, String> {
    call(Request::TakeBack { device: device_match(vendor_id, product_id, serial) })
}

#[tauri::command]
fn set_shared(vendor_id: u16, product_id: u16, serial: Option<String>, share: bool) -> Result<String, String> {
    let device = usbipb_core::model::DeviceMatch { vendor_id, product_id, serial };
    match api::call(config().api_port, &Request::Share { device, share }).map_err(|e| format!("{e:#}"))? {
        Response::Ok { message } => Ok(message),
        Response::Error { message } => Err(message),
        Response::Status(_) => Ok(String::new()),
    }
}

#[cfg(windows)]
fn command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(program);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Full path of a running program (to start it again later).
#[cfg(windows)]
fn process_path(image: &str) -> Option<String> {
    let name = image.trim_end_matches(".exe").trim_end_matches(".EXE");
    let out = command("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-Process -Name '{name}' -ErrorAction SilentlyContinue | Select-Object -First 1).Path")])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// Closes the given programs (politely first) and remembers them for a later restart.
#[tauri::command]
fn close_programs(names: Vec<String>, closed: State<Closed>) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        let mut done = Vec::new();
        for name in names {
            let Some(path) = process_path(&name) else { continue };
            let _ = command("taskkill").args(["/IM", &name]).status();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            if process_path(&name).is_some() {
                let _ = command("taskkill").args(["/IM", &name, "/F"]).status();
            }
            closed.0.lock().unwrap().push(path);
            done.push(name);
        }
        Ok(done)
    }
    #[cfg(not(windows))]
    {
        let _ = (names, closed);
        Err("not supported on this platform yet".into())
    }
}

/// Starts programs closed earlier again (called when the device is back on this computer).
#[tauri::command]
fn restart_closed_programs(closed: State<Closed>) -> Vec<String> {
    let paths: Vec<String> = closed.0.lock().unwrap().drain(..).collect();
    for p in &paths {
        let _ = std::process::Command::new(p).spawn();
    }
    paths
}

#[tauri::command]
fn closed_programs(closed: State<Closed>) -> Vec<String> {
    closed.0.lock().unwrap().clone()
}

fn main() {
    tauri::Builder::default()
        .manage(Closed::default())
        .setup(|app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&usbipb_core::title());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_devices,
            service_status,
            set_attached,
            set_shared,
            use_device,
            give,
            take_back,
            set_locked,
            profile,
            add_peer,
            remove_peer,
            pair,
            pair_answer,
            unpair,
            attention,
            reload_audio,
            close_programs,
            restart_closed_programs,
            closed_programs
        ])
        .run(tauri::generate_context!())
        .expect("error while running USB-IP-Bridge");
}
