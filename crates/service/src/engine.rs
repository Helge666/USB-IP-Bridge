//! Starts and stops the service's workers: export server (Windows), import workers
//! (Linux), the local API and the connection to other computers.

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use usbipb_core::api::{ImportState, ImportStatus, Status};
use usbipb_core::config::{config_path, ConnectMode, Config, ExportRule, ImportRule};
use usbipb_core::model::{DeviceMatch, DeviceState};
use usbipb_core::peer::{Offer, PeerDevice};
use usbipb_core::profiles::Profiles;

pub struct Engine {
    /// The configuration file as loaded at start.
    pub config: Arc<Config>,
    /// Shares changed in the GUI, and the configuration in effect with them.
    shares: Mutex<Shares>,
    effective: Mutex<Arc<Config>>,
    pub stop: Arc<AtomicBool>,
    /// Stable identity of this installation (names it in lists; trust comes from its key).
    pub host_id: String,
    pub security: crate::security::Security,
    pub peers: crate::peers::Peers,
    profiles: Profiles,
    threads: Vec<JoinHandle<()>>,
    #[cfg(windows)]
    export: Arc<usbipb_host::windows::export::ExportState>,
    /// Devices of other computers this computer can use: from config.toml, then the ones
    /// added in the GUI (or by the other computer). Indices stay stable while running.
    imports: Mutex<Vec<ImportSlot>>,
}

struct ImportSlot {
    rule: ImportRule,
    status: Arc<Mutex<ImportStatus>>,
    /// The user wants the device attached here.
    wanted: Arc<AtomicBool>,
    /// Added at run time (saved in imports.json), not from config.toml.
    added: bool,
}

/// Starts the worker for one import rule.
fn new_slot(rule: ImportRule, saved_wanted: &[String], stop: &Arc<AtomicBool>, added: bool) -> ImportSlot {
    let want = Arc::new(AtomicBool::new(rule.mode == ConnectMode::Always || saved_wanted.contains(&rule_name(&rule))));
    let status = Arc::new(Mutex::new(ImportStatus {
        remote: format!("{}:{}", rule.host, rule.port),
        rule: rule_name(&rule),
        state: ImportState::Connecting,
        bus_id: None,
        since_secs: 0,
        attach_count: 0,
        last_error: None,
        wanted: want.load(Ordering::Relaxed),
        device_name: None,
        remote_name: None,
        device: Some(rule.device.clone()),
        classes: Vec::new(),
        warnings: Vec::new(),
    }));
    #[cfg(target_os = "linux")]
    {
        let (r, st, w, s) = (rule.clone(), stop.clone(), want.clone(), status.clone());
        std::thread::spawn(move || usbipb_host::linux::import::run(r, s, w, st));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stop;
        let mut s = status.lock().unwrap();
        s.state = ImportState::Disabled;
        s.last_error = Some("importing is not supported on this platform yet".into());
    }
    ImportSlot { rule, status, wanted: want, added }
}

fn imports_path() -> std::path::PathBuf {
    state_dir().join("imports.json")
}

/// Where the service keeps its state (attach wishes, host id).
pub fn state_dir() -> std::path::PathBuf {
    if cfg!(windows) {
        usbipb_core::config::config_dir()
    } else {
        std::path::PathBuf::from("/var/lib/usb-ip-bridge")
    }
}

fn wanted_path() -> std::path::PathBuf {
    state_dir().join("wanted.json")
}

/// Changes to the configured shares made in the GUI (kept apart from config.toml).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Shares {
    #[serde(default)]
    added: Vec<DeviceMatch>,
    #[serde(default)]
    removed: Vec<DeviceMatch>,
    /// Computers a device was given to in the GUI: allowed even where the configuration
    /// limits a share to certain clients.
    #[serde(default)]
    granted: Vec<(DeviceMatch, String)>,
    /// Devices locked in the app (never shared).
    #[serde(default)]
    locked: Vec<DeviceMatch>,
    /// Devices unlocked in the app although they look essential.
    #[serde(default)]
    unlocked: Vec<DeviceMatch>,
}

fn shares_path() -> std::path::PathBuf {
    state_dir().join("shares.json")
}

/// Same device (a rule without serial covers every serial).
fn same_device(a: &DeviceMatch, b: &DeviceMatch) -> bool {
    a.vendor_id == b.vendor_id && a.product_id == b.product_id && (a.serial.is_none() || b.serial.is_none() || a.serial.as_deref().map(str::to_lowercase) == b.serial.as_deref().map(str::to_lowercase))
}

impl Shares {
    fn apply(&self, base: &Config) -> Config {
        let mut c = base.clone();
        c.never_share.extend(self.locked.iter().cloned());
        c.unlocked = self.unlocked.clone();
        c.lock_essential = true;
        c.export.retain(|r| !self.removed.iter().any(|m| same_device(m, &r.device)));
        for m in &self.added {
            if !c.export.iter().any(|r| same_device(&r.device, m)) {
                c.export.push(ExportRule { device: m.clone(), allowed_clients: Vec::new(), blocking_processes: Vec::new() });
            }
        }
        for r in c.export.iter_mut().filter(|r| !r.allowed_clients.is_empty()) {
            for (m, ip) in &self.granted {
                if same_device(m, &r.device) && !r.allowed_clients.contains(ip) {
                    r.allowed_clients.push(ip.clone());
                }
            }
        }
        c
    }
}

/// The host id: created once, then read from the state directory.
fn load_host_id() -> String {
    let path = state_dir().join("host-id");
    if let Some(id) = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string()).filter(|s| s.len() == 32) {
        return id;
    }
    use std::hash::{BuildHasher, Hasher};
    let random = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(salt);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.write_u32(std::process::id());
        h.finish()
    };
    let id = format!("{:016x}{:016x}", random(1), random(2));
    let _ = std::fs::create_dir_all(state_dir());
    if let Err(e) = std::fs::write(&path, &id) {
        log::warn!("cannot save host id to {}: {e}", path.display());
    }
    id
}

fn load_wanted() -> Vec<String> {
    std::fs::read_to_string(wanted_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "unknown".into())
}

impl Engine {
    pub fn start() -> Result<Arc<Self>> {
        let path = config_path();
        let config = Arc::new(Config::load(&path)?);
        let shares: Shares = std::fs::read_to_string(shares_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let effective = Arc::new(shares.apply(&config));
        log::info!("{} starting on {} (config {})", usbipb_core::title(), hostname(), path.display());
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg_attr(not(windows), allow(unused_mut))] // only the export server is joined
        let mut threads = Vec::new();

        #[cfg(windows)]
        let export = Arc::new(usbipb_host::windows::export::ExportState::default());
        #[cfg(windows)]
        export.set_config(effective.clone());
        #[cfg(windows)]
        export.set_allow_unpaired(!config.require_pairing);
        if !config.require_pairing {
            log::warn!("require_pairing = false: any computer in the network may use the shared devices");
        }
        #[cfg(windows)]
        {
            let (c, s, st) = (config.clone(), export.clone(), stop.clone());
            threads.push(std::thread::spawn(move || {
                // Keep the export server up even if binding fails for a while (e.g. at boot).
                while !st.load(Ordering::Relaxed) {
                    if let Err(e) = usbipb_host::windows::export::run(c.clone(), s.clone(), st.clone()) {
                        log::error!("export server: {e:#}; retrying in 5 s");
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
            }));
        }

        // Linux: rules that keep attached audio devices from becoming the default device.
        #[cfg(target_os = "linux")]
        {
            let profiles = Profiles::load();
            let la = profiles.resolve(0, 0, &[usbipb_core::classes::DeviceClass::Audio]).linux_audio.unwrap_or_default();
            if let Err(e) = usbipb_host::linux::audio::install(&la) {
                log::error!("audio rules: {e:#}");
            }
        }

        let saved = load_wanted();
        let mut imports: Vec<ImportSlot> = config.import.iter().map(|r| new_slot(r.clone(), &saved, &stop, false)).collect();
        let added: Vec<ImportRule> = std::fs::read_to_string(imports_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for rule in added {
            if !imports.iter().any(|s| s.rule.host == rule.host && same_device(&s.rule.device, &rule.device)) {
                imports.push(new_slot(rule, &saved, &stop, true));
            }
        }

        let engine = Arc::new(Self {
            config,
            shares: Mutex::new(shares),
            effective: Mutex::new(effective),
            stop,
            host_id: load_host_id(),
            security: crate::security::Security::load()?,
            peers: Default::default(),
            profiles: Profiles::load(),
            threads,
            #[cfg(windows)]
            export,
            imports: Mutex::new(imports),
        });
        crate::api::spawn(engine.clone())?;
        if let Err(e) = crate::peers::spawn(engine.clone()) {
            log::error!("other computers: {e:#}");
        }
        Ok(engine)
    }

    pub fn status(&self) -> Status {
        Status {
            version: usbipb_core::VERSION.to_string(),
            host: hostname(),
            host_id: self.host_id.clone(),
            peers: self.peers.list(),
            shared: self.effective().export.iter().map(|r| r.device.clone()).collect(),
            #[cfg(target_os = "linux")]
            audio_rules: Some(usbipb_host::linux::audio::status()),
            #[cfg(not(target_os = "linux"))]
            audio_rules: None,
            exports: self.status_exports(),
            imports: self.imports.lock().unwrap().iter().map(|s| s.status.lock().unwrap().clone()).collect(),
            fingerprint: self.security.fingerprint(),
            locks: {
                let shares = self.shares.lock().unwrap();
                usbipb_core::api::Locks { config: self.config.never_share.clone(), user: shares.locked.clone(), unlocked: shares.unlocked.clone() }
            },
            system: Some(usbipb_host::system::info()),
            paired: self.security.paired(),
            pairings: self.security.pairings(),
        }
    }

    /// The computer at `ip` proved to be paired: it may use shared devices (USB/IP).
    pub fn authorize(&self, ip: std::net::IpAddr) {
        #[cfg(windows)]
        self.export.authorize(ip);
        #[cfg(not(windows))]
        let _ = ip;
    }

    /// The configuration in effect (file plus shares changed in the GUI).
    pub fn effective(&self) -> Arc<Config> {
        self.effective.lock().unwrap().clone()
    }

    /// Shares a device of this computer, or stops sharing it; remembered across restarts.
    pub fn set_shared(&self, device: DeviceMatch, share: bool) -> Result<String> {
        if !cfg!(windows) {
            anyhow::bail!("sharing devices is not supported on this platform yet");
        }
        let name = format!("{:04x}:{:04x}", device.vendor_id, device.product_id);
        if share {
            let effective = self.effective();
            let present = usbipb_host::list_devices().into_iter().find(|d| device.matches(d));
            if self.config.never_share.iter().any(|m| same_device(m, &device)) || present.is_some_and(|d| effective.is_blocked(&d)) {
                anyhow::bail!("{name} is locked on this computer (unlock it first)");
            }
        }
        #[cfg(windows)]
        if !share && self.export.lent().iter().any(|(_, _, d)| device.matches(d)) {
            anyhow::bail!("{name} is in use by another computer – it has to be given back first");
        }
        let mut shares = self.shares.lock().unwrap();
        shares.added.retain(|m| !same_device(m, &device));
        shares.removed.retain(|m| !same_device(m, &device));
        let in_file = self.config.export.iter().any(|r| same_device(&r.device, &device));
        match (share, in_file) {
            (true, false) => shares.added.push(device.clone()),
            (false, true) => shares.removed.push(device.clone()),
            _ => {}
        }
        std::fs::write(shares_path(), serde_json::to_string_pretty(&*shares)?)?;
        let effective = Arc::new(shares.apply(&self.config));
        *self.effective.lock().unwrap() = effective.clone();
        #[cfg(windows)]
        self.export.set_config(effective);
        log::info!("{name}: {} by the user", if share { "shared" } else { "no longer shared" });
        Ok(format!("{name} {}", if share { "shared" } else { "no longer shared" }))
    }

    /// Locks a device of this computer (never shared) or unlocks it; remembered across restarts.
    pub fn set_locked(&self, device: DeviceMatch, lock: bool) -> Result<String> {
        let name = format!("{:04x}:{:04x}", device.vendor_id, device.product_id);
        if !lock && self.config.never_share.iter().any(|m| same_device(m, &device)) {
            anyhow::bail!("{name} is locked in the configuration file ({}) – it can only be unlocked there", config_path().display());
        }
        #[cfg(windows)]
        if lock && self.export.lent().iter().any(|(_, _, d)| device.matches(d)) {
            anyhow::bail!("{name} is in use by another computer – take it back first");
        }
        let mut shares = self.shares.lock().unwrap();
        shares.locked.retain(|m| !same_device(m, &device));
        shares.unlocked.retain(|m| !same_device(m, &device));
        if lock {
            shares.locked.push(device);
        } else {
            shares.unlocked.push(device);
        }
        std::fs::write(shares_path(), serde_json::to_string_pretty(&*shares)?)?;
        let effective = Arc::new(shares.apply(&self.config));
        *self.effective.lock().unwrap() = effective.clone();
        #[cfg(windows)]
        self.export.set_config(effective);
        log::info!("{name}: {} by the user", if lock { "locked" } else { "unlocked" });
        Ok(format!("{name} {}", if lock { "locked" } else { "unlocked" }))
    }

    /// This computer's devices as other computers see them.
    pub fn peer_devices(&self) -> Vec<PeerDevice> {
        let config = self.effective();
        let describe = |d: usbipb_core::model::UsbDevice, used_by: Option<String>| {
            let p = self.profiles.resolve(d.vendor_id, d.product_id, &d.classes);
            let offer = if config.is_blocked(&d) {
                Offer::Never
            } else if config.export.iter().any(|r| r.device.matches(&d)) {
                Offer::Shared
            } else {
                Offer::NotShared
            };
            PeerDevice { name: p.name.unwrap_or_else(|| d.display_name()), classes: p.classes, warnings: p.warnings, offer, used_by, device: d }
        };
        // While a device is lent, Windows lists the capture driver (VirtualBox USB) in its place.
        let mut out: Vec<PeerDevice> =
            usbipb_host::list_devices().into_iter().filter(|d| !(d.vendor_id == 0x80ee && d.product_id == 0xcafe)).map(|d| describe(d, None)).collect();
        for e in self.status_exports() {
            if let (Some(d), Some(client)) = (e.device, e.client) {
                if d.state == DeviceState::InUseRemotely {
                    out.push(describe(d, Some(e.client_name.unwrap_or(client))));
                }
            }
        }
        out.sort_by(|a, b| a.device.bus_id.cmp(&b.device.bus_id));
        out
    }

    fn status_exports(&self) -> Vec<usbipb_core::api::ExportStatus> {
        #[cfg(windows)]
        return self.export.status(&self.effective());
        #[cfg(not(windows))]
        Vec::new()
    }

    /// Sets the user's wish for import rule `index` (all rules if None) and saves it.
    pub fn set_wanted(&self, index: Option<usize>, want: bool) -> Result<String> {
        let slots = self.imports.lock().unwrap();
        let indices: Vec<usize> = match index {
            Some(i) if i < slots.len() => vec![i],
            Some(i) => anyhow::bail!("no import rule #{} (there are {})", i + 1, slots.len()),
            None => (0..slots.len()).collect(),
        };
        if indices.is_empty() {
            anyhow::bail!("no import rules configured");
        }
        for &i in &indices {
            slots[i].wanted.store(want, Ordering::Relaxed);
            slots[i].status.lock().unwrap().wanted = want;
            log::info!("{}: {} by the user", rule_name(&slots[i].rule), if want { "attach requested" } else { "release requested" });
        }
        let names: Vec<String> = slots.iter().filter(|s| s.wanted.load(Ordering::Relaxed)).map(|s| rule_name(&s.rule)).collect();
        let path = wanted_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, serde_json::to_string_pretty(&names)?)?;
        Ok(format!("{} {} rule(s)", if want { "attaching" } else { "releasing" }, indices.len()))
    }

    /// Uses a device of another computer here (adding an import for it if needed), or
    /// gives it back. `host` is the other computer's address, `port` its USB/IP port.
    pub fn use_device(&self, host: &str, port: u16, device: DeviceMatch, attach: bool) -> Result<String> {
        // An audio device must not arrive before the rules that keep it from becoming the
        // default device are active (game audio would move to it).
        #[cfg(target_os = "linux")]
        if attach && self.is_audio(host, &device) && !usbipb_host::linux::audio::status().active {
            if let Err(e) = usbipb_host::linux::audio::reload_if_idle() {
                anyhow::bail!(
                    "{} cannot take the audio device yet: its audio rules (never the default device) are not active, \
                     and they can only be loaded while no audio is in use – {e:#}. Try again when no game stream or audio program runs.",
                    hostname()
                );
            }
        }
        let mut slots = self.imports.lock().unwrap();
        let index = match slots.iter().position(|s| s.rule.host == host && same_device(&s.rule.device, &device)) {
            Some(i) => i,
            None if !attach => anyhow::bail!("{:04x}:{:04x} from {host} is not used here", device.vendor_id, device.product_id),
            None => {
                if !cfg!(target_os = "linux") {
                    anyhow::bail!("using devices of other computers is not supported on this platform yet");
                }
                let rule = ImportRule { host: host.to_string(), port, device, mode: ConnectMode::Manual, auto_reconnect: true };
                log::info!("{}: added", rule_name(&rule));
                slots.push(new_slot(rule, &[], &self.stop, true));
                let added: Vec<&ImportRule> = slots.iter().filter(|s| s.added).map(|s| &s.rule).collect();
                std::fs::write(imports_path(), serde_json::to_string_pretty(&added)?)?;
                slots.len() - 1
            }
        };
        drop(slots);
        self.set_wanted(Some(index), attach)
    }

    /// Makes the audio rules active now, if no audio is in use (Linux).
    pub fn reload_audio(&self) -> Result<String> {
        #[cfg(target_os = "linux")]
        {
            if usbipb_host::linux::audio::status().active {
                return Ok("audio rules are active".into());
            }
            usbipb_host::linux::audio::reload_if_idle()?;
            Ok("audio reloaded".into())
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("only needed on Linux")
    }

    /// Is this device of computer `host` an audio device (as that computer reports it)?
    #[cfg(target_os = "linux")]
    fn is_audio(&self, host: &str, device: &DeviceMatch) -> bool {
        let ip = |a: &str| a.rsplit_once(':').map(|(h, _)| h.trim_matches(|c| c == '[' || c == ']').to_string()).unwrap_or_else(|| a.to_string());
        self.peers.list().iter().filter(|p| ip(&p.address) == host).flat_map(|p| p.devices.iter()).any(|d| {
            same_device(device, &DeviceMatch { vendor_id: d.device.vendor_id, product_id: d.device.product_id, serial: d.device.serial.clone() })
                && d.classes.contains(&usbipb_core::classes::DeviceClass::Audio)
        })
    }

    /// Gives a device of this computer to another computer (`peer`: its control address);
    /// shares it first if needed.
    pub fn give(&self, peer: &str, device: DeviceMatch) -> Result<String> {
        if !self.effective().export.iter().any(|r| same_device(&r.device, &device)) {
            self.set_shared(device.clone(), true)?;
        }
        self.grant(peer, &device)?;
        let mut c = usbipb_core::peer::PeerClient::connect(peer, &self.security.identity)?;
        c.call_ok(&usbipb_core::peer::PeerRequest::Attach { device, usbip_port: self.config.usbip_port })
    }

    /// Allows the computer at control address `peer` to use the device, also where the
    /// configuration limits the share to other clients (the user chose that computer).
    fn grant(&self, peer: &str, device: &DeviceMatch) -> Result<()> {
        let addr = std::net::ToSocketAddrs::to_socket_addrs(peer)?.next().ok_or_else(|| anyhow::anyhow!("cannot resolve {peer}"))?;
        let ip = addr.ip().to_string();
        let mut shares = self.shares.lock().unwrap();
        if shares.granted.iter().any(|(m, g)| same_device(m, device) && *g == ip) {
            return Ok(());
        }
        shares.granted.push((device.clone(), ip.clone()));
        std::fs::write(shares_path(), serde_json::to_string_pretty(&*shares)?)?;
        let effective = Arc::new(shares.apply(&self.config));
        *self.effective.lock().unwrap() = effective.clone();
        #[cfg(windows)]
        self.export.set_config(effective);
        log::info!("{:04x}:{:04x}: {ip} allowed (given to it by the user)", device.vendor_id, device.product_id);
        Ok(())
    }

    /// Asks the computer that uses a device of this computer to give it back.
    pub fn take_back(&self, device: DeviceMatch) -> Result<String> {
        #[cfg(windows)]
        {
            let Some((client, _)) = self.export.lent().into_iter().map(|(_, c, d)| (c, d)).find(|(_, d)| device.matches(d)) else {
                anyhow::bail!("{:04x}:{:04x} is not used by another computer", device.vendor_id, device.product_id);
            };
            let address = format!("{client}:{}", usbipb_core::peer::PEER_PORT);
            let mut c = usbipb_core::peer::PeerClient::connect(&address, &self.security.identity)?;
            c.call_ok(&usbipb_core::peer::PeerRequest::Release { device })
        }
        #[cfg(not(windows))]
        {
            let _ = device;
            anyhow::bail!("sharing devices is not supported on this platform yet")
        }
    }

    /// Signals all workers to stop and waits for them (bounded).
    pub fn shutdown(self: Arc<Self>) {
        log::info!("stopping");
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(engine) = Arc::try_unwrap(self) {
            for t in engine.threads {
                let _ = t.join();
            }
        } else {
            std::thread::sleep(Duration::from_secs(3));
        }
        log::info!("stopped");
    }
}

fn rule_name(rule: &usbipb_core::config::ImportRule) -> String {
    let d = &rule.device;
    let serial = d.serial.as_deref().map(|s| format!(" serial {s}")).unwrap_or_default();
    format!("{:04x}:{:04x}{serial} from {}:{}", d.vendor_id, d.product_id, rule.host, rule.port)
}

/// Foreground mode: runs until Ctrl-C / SIGTERM.
pub fn run_until_signal() -> Result<()> {
    // Block SIGINT/SIGTERM before any thread starts, so only `wait_for_signal` sees them.
    #[cfg(unix)]
    let signals = block_signals();
    let engine = Engine::start()?;
    let stop = engine.stop.clone();
    #[cfg(unix)]
    wait_for_signal(&signals, stop);
    #[cfg(windows)]
    wait_for_signal(stop);
    engine.shutdown();
    Ok(())
}

#[cfg(unix)]
fn block_signals() -> libc_sigset {
    // Minimal SIGINT/SIGTERM handling without extra crates: block the signals, wait for them.
    unsafe {
        let mut set: libc_sigset = std::mem::zeroed();
        sigemptyset(&mut set);
        sigaddset(&mut set, 2); // SIGINT
        sigaddset(&mut set, 15); // SIGTERM
        pthread_sigmask(0, &set, std::ptr::null_mut()); // SIG_BLOCK
        set
    }
}

#[cfg(unix)]
fn wait_for_signal(set: &libc_sigset, stop: Arc<AtomicBool>) {
    let mut sig = 0;
    unsafe {
        sigwait(set, &mut sig);
    }
    log::info!("signal {sig} received");
    stop.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
#[allow(non_camel_case_types)]
type libc_sigset = [u64; 16];
#[cfg(unix)]
extern "C" {
    fn sigemptyset(set: *mut libc_sigset) -> i32;
    fn sigaddset(set: *mut libc_sigset, sig: i32) -> i32;
    fn pthread_sigmask(how: i32, set: *const libc_sigset, old: *mut libc_sigset) -> i32;
    fn sigwait(set: *const libc_sigset, sig: *mut i32) -> i32;
}

#[cfg(windows)]
fn wait_for_signal(stop: Arc<AtomicBool>) {
    // Console mode on Windows: stop when Enter is pressed (the service uses the SCM instead).
    eprintln!("running; press Enter to stop");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    stop.store(true, Ordering::Relaxed);
}
