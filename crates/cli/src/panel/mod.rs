//! `usbipb panel`: an interactive panel for the terminal with what the app offers – this
//! computer and the other computers with their USB devices, details, and the actions
//! (share, give to, take back, use here, give back, lock, pair, add computers).
//!
//! Draws plain ASCII only, so it works in any terminal, over SSH and on a server console.
//! Talks to the local service like the app does; the model and the actions follow the
//! app (gui/ui/app.js).

mod term;
mod texts;

use anyhow::Result;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use term::{wrap, Canvas, Key, Terminal, BOLD, DIM, REVERSE};
use texts::{t, tv, Lang};
use usbipb_core::api::{self, ExportStatus, ImportState, ImportStatus, PairingState, Request, Response, Status};
use usbipb_core::classes::DeviceClass;
use usbipb_core::config::{config_path, Config, ExportRule};
use usbipb_core::model::{DeviceMatch, DeviceState, UsbDevice};
use usbipb_core::peer::{Offer, PeerStatus};
use usbipb_core::profiles::{Effective, Profiles, Warning};

const REFRESH: Duration = Duration::from_secs(2);
const RISKY: [Warning; 3] = [Warning::DataLoss, Warning::Lockout, Warning::NetworkLoss];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    // devices of this computer
    Local,
    Shared,
    Lent,
    Blocked,
    // devices of other computers
    Here,
    Used,
    PeerShared,
    NotShared,
    Never,
    /// Its computer does not list it right now (not plugged in, or offline).
    Absent,
}

struct Dev {
    key: String,
    name: String,
    device: UsbDevice,
    classes: Vec<DeviceClass>,
    warnings: Vec<Warning>,
    profile: Effective,
    state: State,
    /// Why a device of this computer is locked: "config", "user" or "auto".
    lock: Option<&'static str>,
    used_by: Option<String>,
    problem: Option<ExportStatus>,
    imp: Option<ImportStatus>,
}

struct Host {
    key: String,
    local: bool,
    name: String,
    online: bool,
    /// Paired on both sides (only then they talk about devices).
    paired: bool,
    peer: Option<PeerStatus>,
    devices: Vec<Dev>,
}

/// A device of this computer, as listed here (with shares and locks from the service).
struct LocalRow {
    device: UsbDevice,
    shared: bool,
    blocked: bool,
    lock: Option<&'static str>,
    profile: Effective,
}

#[derive(Clone)]
struct Action {
    label: String,
    confirm: Option<String>,
    request: Request,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    AddPeer,
    PairCode(u64),
}

enum Popup {
    Menu { title: String, actions: Vec<Action>, sel: usize },
    Confirm { text: String, action: Action },
    Input { title: String, text: String, label: String, value: String, purpose: Purpose, error: Option<String> },
    Info { title: String, text: String, scroll: usize },
    /// The code to type on the other computer (outgoing pairing `id`).
    Code { id: u64 },
}

struct App {
    lang: Lang,
    config: Config,
    profiles: Profiles,
    can_share: bool,
    can_import: bool,
    status: Option<Status>,
    status_error: Option<String>,
    local: Vec<LocalRow>,
    model: Vec<Host>,
    /// Computers that can use devices of this one: (name, control address).
    importers: Vec<(String, String)>,
    /// Visible rows: host index, device index.
    flat: Vec<(usize, Option<usize>)>,
    selected: Option<String>,
    collapsed: HashSet<String>,
    only_shared: bool,
    details_focus: bool,
    details_scroll: usize,
    tree_scroll: usize,
    popup: Option<Popup>,
    message: Option<(String, bool)>,
    /// An audio device could not come here because the audio rules are not active yet
    /// (only then the reminder to reload audio is worth showing).
    audio_wanted: bool,
    /// Pairings already reported (or answered); codes the user closed.
    reported: HashSet<u64>,
    hidden_codes: HashSet<u64>,
    quit: bool,
}

pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let lang = args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1)).map(String::as_str);
    let mut app = App {
        lang: Lang::detect(lang),
        config: Config::load(&config_path()).unwrap_or_default(),
        profiles: Profiles::load(),
        can_share: usbipb_host::can_export(),
        can_import: usbipb_host::can_import(),
        status: None,
        status_error: None,
        local: Vec::new(),
        model: Vec::new(),
        importers: Vec::new(),
        flat: Vec::new(),
        selected: None,
        collapsed: HashSet::new(),
        only_shared: false,
        details_focus: false,
        details_scroll: 0,
        tree_scroll: 0,
        popup: None,
        message: None,
        audio_wanted: false,
        reported: HashSet::new(),
        hidden_codes: HashSet::new(),
        quit: false,
    };
    let mut term = Terminal::enter()?;
    app.refresh();
    let mut last = Instant::now();
    while !app.quit {
        app.draw(&mut term);
        let wait = REFRESH.saturating_sub(last.elapsed()).min(Duration::from_millis(500));
        match term.input.recv_timeout(wait) {
            Ok(bytes) => {
                for k in term::parse(&bytes) {
                    app.key(k);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        if last.elapsed() >= REFRESH {
            app.refresh();
            last = Instant::now();
        }
    }
    Ok(())
}

fn hex(n: u16) -> String {
    format!("{n:04x}")
}

/// "host:port" → "host" (also "[v6]:port").
fn ip_of(address: &str) -> String {
    let a = match address.rfind(':') {
        Some(i) if !address[i + 1..].is_empty() && address[i + 1..].bytes().all(|b| b.is_ascii_digit()) => &address[..i],
        _ => address,
    };
    a.trim_start_matches('[').trim_end_matches(']').to_string()
}

fn same_serial(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => true,
    }
}

fn matching(d: &UsbDevice) -> DeviceMatch {
    DeviceMatch { vendor_id: d.vendor_id, product_id: d.product_id, serial: d.serial.clone() }
}

/// A device known only by its ids (used here while its computer does not list it).
fn bare_device(m: &DeviceMatch) -> UsbDevice {
    UsbDevice {
        bus_id: String::new(),
        vendor_id: m.vendor_id,
        product_id: m.product_id,
        manufacturer: None,
        product: None,
        serial: m.serial.clone(),
        platform_id: String::new(),
        driver: None,
        state: DeviceState::Local,
        classes: Vec::new(),
    }
}

/// The serde name of a value ("snake_case"), for text keys.
fn name_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

impl App {
    fn t(&self, key: &str) -> &'static str {
        t(self.lang, key)
    }

    fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String {
        tv(self.lang, key, vars)
    }

    fn kind_text(&self, classes: &[DeviceClass]) -> String {
        classes.iter().map(|c| self.t(&format!("class_{}", c.name()))).collect::<Vec<_>>().join(" + ")
    }

    fn secs_text(&self, s: u64) -> String {
        if s < 120 {
            self.tv("secsAgo", &[("s", &s.to_string())])
        } else {
            self.tv("minsAgo", &[("m", &((s + 30) / 60).to_string())])
        }
    }

    // ------------------------------------------------------------------ data

    fn refresh(&mut self) {
        match api::call(self.config.api_port, &Request::Status) {
            Ok(Response::Status(s)) => {
                self.status = Some(s);
                self.status_error = None;
            }
            Ok(Response::Error { message } | Response::Ok { message }) => {
                self.status = None;
                self.status_error = Some(message);
            }
            Err(e) => {
                self.status = None;
                self.status_error = Some(format!("{e:#}"));
            }
        }
        // What is shared and locked comes from the service (it includes changes made in the app).
        let mut cfg = self.config.clone();
        let mut locks = api::Locks { config: cfg.never_share.clone(), ..Default::default() };
        if let Some(s) = &self.status {
            cfg.export = s.shared.iter().cloned().map(|device| ExportRule { device, allowed_clients: vec![], blocking_processes: vec![] }).collect();
            locks = s.locks.clone();
        }
        locks.apply(&mut cfg);
        self.local = usbipb_host::list_devices()
            .into_iter()
            .map(|d| {
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
                LocalRow { shared: cfg.export.iter().any(|r| r.device.matches(&d)), blocked, lock, profile: self.profiles.resolve(d.vendor_id, d.product_id, &d.classes), device: d }
            })
            .collect();
        self.model = self.build();
        self.update_flat();
        self.check_pairings();
    }

    fn build(&mut self) -> Vec<Host> {
        let empty = Vec::new();
        let st = self.status.as_ref();
        let exports = st.map_or(&empty, |s| &s.exports);
        let lent: Vec<&ExportStatus> = exports.iter().filter(|e| e.client.is_some() && e.device.is_some()).collect();
        let peers: &[PeerStatus] = st.map_or(&[], |s| &s.peers);
        let imports: &[ImportStatus] = st.map_or(&[], |s| &s.imports);
        self.importers = peers
            .iter()
            .filter(|p| p.online && p.paired && p.hello.as_ref().is_some_and(|h| h.can_import && h.paired))
            .map(|p| (p.hello.as_ref().unwrap().host_name.clone(), p.address.clone()))
            .collect();

        // this computer
        let mut local = Host {
            key: "host:local".into(),
            local: true,
            name: st.map_or_else(|| self.t("thisComputer").to_string(), |s| s.host.clone()),
            online: st.is_some(),
            paired: true,
            peer: None,
            devices: Vec::new(),
        };
        for row in &self.local {
            // While a device is used by another computer, Windows lists the capture driver
            // (VirtualBox USB, 80ee:cafe) in its place, Linux the device bound to usbip-host:
            // show the real device as lent.
            let vbox = row.device.vendor_id == 0x80ee && row.device.product_id == 0xcafe;
            let lent_to = lent.iter().find(|e| e.device.as_ref().is_some_and(|d| d.bus_id == row.device.bus_id));
            if vbox && lent_to.is_none() {
                continue;
            }
            if row.device.state == DeviceState::Attached {
                continue; // a device of another computer, shown there
            }
            let (device, profile) = match lent_to {
                Some(e) => {
                    let d = e.device.clone().unwrap();
                    let p = self.profiles.resolve(d.vendor_id, d.product_id, &d.classes);
                    (d, p)
                }
                None => (row.device.clone(), row.profile.clone()),
            };
            local.devices.push(Dev {
                key: format!("dev:local:{}:{}:{}", hex(device.vendor_id), hex(device.product_id), device.serial.clone().unwrap_or_else(|| device.bus_id.clone())),
                name: profile.name.clone().unwrap_or_else(|| device.display_name()),
                classes: profile.classes.clone(),
                warnings: profile.warnings.clone(),
                state: if lent_to.is_some() {
                    State::Lent
                } else if row.blocked {
                    State::Blocked
                } else if row.shared {
                    State::Shared
                } else {
                    State::Local
                },
                lock: if lent_to.is_some() { None } else { row.lock },
                used_by: lent_to.map(|e| e.client_name.clone().or_else(|| e.client.clone()).unwrap_or_default()),
                problem: exports.iter().find(|e| e.problem.is_some() && e.device.as_ref().is_some_and(|d| d.bus_id == device.bus_id)).cloned(),
                imp: None,
                profile,
                device,
            });
        }
        // shared devices that are not plugged in (as they were last seen)
        for e in exports.iter().filter(|e| !e.present) {
            let Some(device) = e.device.clone() else { continue };
            let profile = self.profiles.resolve(device.vendor_id, device.product_id, &device.classes);
            local.devices.push(Dev {
                key: format!("dev:local:{}:{}:{}", hex(device.vendor_id), hex(device.product_id), device.serial.clone().unwrap_or_else(|| device.bus_id.clone())),
                name: profile.name.clone().unwrap_or_else(|| device.display_name()),
                classes: profile.classes.clone(),
                warnings: profile.warnings.clone(),
                state: State::Absent,
                lock: None,
                used_by: None,
                problem: None,
                imp: None,
                profile,
                device,
            });
        }
        let mut hosts = vec![local];

        // other computers
        for p in peers {
            let name = p.hello.as_ref().map(|h| h.host_name.clone()).or_else(|| p.known_name.clone()).unwrap_or_else(|| ip_of(&p.address));
            let id = p.hello.as_ref().map(|h| h.host_id.clone()).or_else(|| p.known_id.clone()).unwrap_or_else(|| p.address.clone());
            let paired = p.paired && p.hello.as_ref().is_some_and(|h| h.paired);
            let key = format!("host:{id}");
            let ip = ip_of(&p.address);
            let here: Vec<&ImportStatus> = imports.iter().filter(|i| i.device.is_some() && ip_of(&i.remote) == ip).collect();
            let import_for = |d: &UsbDevice| {
                here.iter()
                    .find(|i| {
                        let m = i.device.as_ref().unwrap();
                        m.vendor_id == d.vendor_id && m.product_id == d.product_id && same_serial(m.serial.as_deref(), d.serial.as_deref())
                    })
                    .map(|i| (*i).clone())
            };
            let mut devices = Vec::new();
            for pd in &p.devices {
                let imp = import_for(&pd.device);
                let state = if imp.as_ref().is_some_and(|i| i.wanted) {
                    State::Here
                } else if !pd.present {
                    State::Absent
                } else if pd.used_by.is_some() {
                    State::Used
                } else {
                    match pd.offer {
                        Offer::Shared => State::PeerShared,
                        Offer::NotShared => State::NotShared,
                        Offer::Never => State::Never,
                    }
                };
                let d = &pd.device;
                devices.push(Dev {
                    key: format!("dev:{key}:{}:{}:{}", hex(d.vendor_id), hex(d.product_id), d.serial.clone().unwrap_or_else(|| d.bus_id.clone())),
                    name: pd.name.clone(),
                    classes: pd.classes.clone(),
                    warnings: pd.warnings.clone(),
                    profile: self.profiles.resolve(d.vendor_id, d.product_id, &pd.classes),
                    state,
                    lock: None,
                    used_by: pd.used_by.clone(),
                    problem: None,
                    imp,
                    device: pd.device.clone(),
                });
            }
            // devices used here that the computer does not list (it is offline, or the device is gone)
            for imp in &here {
                if devices.iter().any(|d| d.imp.as_ref().is_some_and(|i| i.rule == imp.rule)) {
                    continue;
                }
                let m = imp.device.as_ref().unwrap();
                devices.push(Dev {
                    key: format!("dev:{key}:{}:{}:{}", hex(m.vendor_id), hex(m.product_id), m.serial.clone().unwrap_or_else(|| "-".into())),
                    name: imp.device_name.clone().unwrap_or_else(|| format!("{}:{}", hex(m.vendor_id), hex(m.product_id))),
                    classes: imp.classes.clone(),
                    warnings: imp.warnings.clone(),
                    profile: self.profiles.resolve(m.vendor_id, m.product_id, &imp.classes),
                    state: if imp.wanted { State::Here } else { State::Absent },
                    lock: None,
                    used_by: None,
                    problem: None,
                    imp: Some((*imp).clone()),
                    device: bare_device(m),
                });
            }
            hosts.push(Host { key, local: false, name, online: p.online, paired, peer: Some(p.clone()), devices });
        }
        for h in &mut hosts {
            h.devices.sort_by_key(|d| d.name.to_lowercase());
        }
        hosts[1..].sort_by_key(|h| h.name.to_lowercase());
        hosts
    }

    fn visible(&self, d: &Dev) -> bool {
        !self.only_shared || matches!(d.state, State::Shared | State::Lent | State::Here | State::Used | State::PeerShared) || d.problem.is_some()
    }

    fn update_flat(&mut self) {
        let old = self.selected_index();
        self.flat.clear();
        for (hi, h) in self.model.iter().enumerate() {
            self.flat.push((hi, None));
            if !self.collapsed.contains(&h.key) {
                for (di, d) in h.devices.iter().enumerate() {
                    if self.visible(d) {
                        self.flat.push((hi, Some(di)));
                    }
                }
            }
        }
        // keep the selection; if it is gone, stay at the same place
        if self.selected.as_ref().is_none_or(|k| !self.flat.iter().any(|&r| self.key_of(r) == *k)) {
            let i = old.unwrap_or(0).min(self.flat.len().saturating_sub(1));
            self.selected = self.flat.get(i).map(|&r| self.key_of(r));
        }
    }

    fn key_of(&self, (hi, di): (usize, Option<usize>)) -> String {
        match di {
            Some(di) => self.model[hi].devices[di].key.clone(),
            None => self.model[hi].key.clone(),
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let k = self.selected.as_ref()?;
        self.flat.iter().position(|&r| self.key_of(r) == *k)
    }

    fn current(&self) -> Option<(usize, Option<usize>)> {
        self.selected_index().map(|i| self.flat[i])
    }

    /// Reports finished pairings, asks for the code of incoming ones, shows the code of
    /// outgoing ones.
    fn check_pairings(&mut self) {
        let list = self.status.as_ref().map(|s| s.pairings.clone()).unwrap_or_default();
        let waiting = |id: u64| list.iter().any(|p| p.id == id && p.state == PairingState::Waiting);
        match &self.popup {
            Some(Popup::Input { purpose: Purpose::PairCode(id), .. }) | Some(Popup::Code { id }) if !waiting(*id) => self.popup = None,
            _ => {}
        }
        for p in &list {
            let vars = [("host", p.host_name.as_str())];
            match p.state {
                PairingState::Paired if self.reported.insert(p.id) => self.message = Some((self.tv("pairDone", &vars), false)),
                PairingState::Failed if self.reported.insert(p.id) => {
                    let why = p.message.clone().map(|m| format!(": {m}")).unwrap_or_default();
                    self.message = Some((format!("{}{why}", self.tv("pairFailed", &vars)), true));
                }
                PairingState::Waiting if self.popup.is_none() && !self.reported.contains(&p.id) => {
                    if p.outgoing {
                        if !self.hidden_codes.contains(&p.id) {
                            self.popup = Some(Popup::Code { id: p.id });
                        }
                    } else {
                        self.popup = Some(Popup::Input {
                            title: self.tv("pairInTitle", &vars),
                            text: self.t("pairInText").into(),
                            label: self.tv("pairInLabel", &vars),
                            value: String::new(),
                            purpose: Purpose::PairCode(p.id),
                            error: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------ actions

    fn risky_confirm(&self, d: &Dev) -> Option<String> {
        let risky: Vec<String> = d.warnings.iter().filter(|w| RISKY.contains(w)).map(|w| format!("! {}", self.t(&format!("warnText_{}", w.name())))).collect();
        (!risky.is_empty()).then(|| self.tv("confirmShare", &[("device", &d.name), ("warnings", &risky.join("\n"))]))
    }

    fn actions_of(&self, (hi, di): (usize, Option<usize>)) -> Vec<Action> {
        let host = &self.model[hi];
        let act = |label: String, confirm: Option<String>, request: Request| Action { label, confirm, request };
        let mut out = Vec::new();
        let Some(di) = di else {
            if host.local {
                if self.status.as_ref().and_then(|s| s.audio_rules).is_some_and(|a| !a.active) {
                    out.push(act(self.t("reloadAudio").into(), None, Request::ReloadAudio));
                }
                return out;
            }
            let p = host.peer.as_ref().unwrap();
            let pairing = self.status.as_ref().is_some_and(|s| s.pairings.iter().any(|x| x.outgoing && x.state == PairingState::Waiting && x.address == p.address));
            if p.online && p.hello.is_some() && !host.paired && !pairing {
                out.push(act(self.t("pair").into(), None, Request::Pair { address: p.address.clone() }));
            }
            let host_id = p.hello.as_ref().map(|h| h.host_id.clone()).or_else(|| p.known_id.clone());
            if let (true, Some(host_id)) = (p.paired, host_id) {
                out.push(act(self.t("unpair").into(), Some(self.tv("confirmUnpair", &[("host", &host.name)])), Request::Unpair { host_id }));
            }
            if p.removable {
                out.push(act(self.t("removeComputer").into(), Some(self.tv("confirmRemove", &[("host", &host.name)])), Request::RemovePeer { address: p.address.clone() }));
            }
            return out;
        };
        let d = &host.devices[di];
        let device = matching(&d.device);
        if host.local {
            if d.state == State::Absent {
                out.push(act(self.t("unshare").into(), None, Request::Share { device, share: false }));
                return out;
            }
            if d.state == State::Lent {
                if self.can_share {
                    out.push(act(self.t("takeBack").into(), None, Request::TakeBack { device }));
                }
                return out;
            }
            // locked in config.toml: only there
            if d.state == State::Blocked {
                if d.lock != Some("config") {
                    out.push(act(self.t("unlock").into(), None, Request::Lock { device, lock: false }));
                }
                return out;
            }
            let lock = act(self.t("lock").into(), None, Request::Lock { device: device.clone(), lock: true });
            if !self.can_share {
                return vec![lock];
            }
            for (name, address) in &self.importers {
                let confirm = if d.state == State::Shared { None } else { self.risky_confirm(d) };
                out.push(act(self.tv("giveTo", &[("host", name)]), confirm, Request::Give { peer: address.clone(), device: device.clone() }));
            }
            if d.state == State::Shared {
                out.push(act(self.t("unshare").into(), None, Request::Share { device, share: false }));
            } else {
                out.push(act(self.t("share").into(), self.risky_confirm(d), Request::Share { device, share: true }));
            }
            out.push(lock);
            return out;
        }
        let p = host.peer.as_ref().unwrap();
        let Some(hello) = p.hello.as_ref().filter(|h| self.can_import && h.can_export) else { return out };
        let (h, port) = (ip_of(&p.address), hello.usbip_port);
        match d.state {
            State::Here => out.push(act(self.t("giveBack").into(), None, Request::Use { host: h, port, device, attach: false })),
            State::PeerShared if host.online => out.push(act(self.t("useHere").into(), None, Request::Use { host: h, port, device, attach: true })),
            _ => {}
        }
        out
    }

    fn run_action(&mut self, a: &Action) {
        self.message = Some(match api::call(self.config.api_port, &a.request) {
            Ok(Response::Error { message }) => {
                self.audio_wanted |= message.contains("audio rules");
                (self.tv("failed", &[("action", &a.label), ("error", &message)]), true)
            }
            Ok(_) => (self.tv("done", &[("action", &a.label)]), false),
            Err(e) => (self.tv("failed", &[("action", &a.label), ("error", &format!("{e:#}"))]), true),
        });
        self.refresh();
    }

    // ------------------------------------------------------------------ keys

    fn key(&mut self, k: Key) {
        if k == Key::CtrlC {
            self.quit = true;
            return;
        }
        if let Some(p) = self.popup.take() {
            self.popup_key(p, k);
            return;
        }
        if self.details_focus {
            match k {
                Key::Up => self.details_scroll = self.details_scroll.saturating_sub(1),
                Key::Down => self.details_scroll += 1,
                Key::PageUp => self.details_scroll = self.details_scroll.saturating_sub(10),
                Key::PageDown => self.details_scroll += 10,
                Key::Home => self.details_scroll = 0,
                Key::Tab | Key::Esc | Key::Left => self.details_focus = false,
                Key::Char('q') => self.quit = true,
                _ => {}
            }
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let select = |app: &mut App, i: usize| {
            let i = i.min(app.flat.len().saturating_sub(1));
            if let Some(&r) = app.flat.get(i) {
                app.selected = Some(app.key_of(r));
                app.details_scroll = 0;
            }
        };
        match k {
            Key::Up => select(self, i.saturating_sub(1)),
            Key::Down => select(self, i + 1),
            Key::PageUp => select(self, i.saturating_sub(10)),
            Key::PageDown => select(self, i + 10),
            Key::Home => select(self, 0),
            Key::End => select(self, usize::MAX),
            Key::Right => {
                if let Some((hi, None)) = self.current() {
                    let key = self.model[hi].key.clone();
                    if self.collapsed.remove(&key) {
                        self.update_flat();
                    } else {
                        select(self, i + 1);
                    }
                }
            }
            Key::Left => match self.current() {
                Some((hi, None)) => {
                    self.collapsed.insert(self.model[hi].key.clone());
                    self.update_flat();
                }
                Some((hi, Some(_))) => self.selected = Some(self.model[hi].key.clone()),
                None => {}
            },
            Key::Enter | Key::Char(' ') => {
                if let Some(cur) = self.current() {
                    let actions = self.actions_of(cur);
                    if actions.is_empty() {
                        self.message = Some((self.t("noActions").into(), false));
                    } else {
                        let title = match cur {
                            (hi, Some(di)) => self.model[hi].devices[di].name.clone(),
                            (hi, None) => self.model[hi].name.clone(),
                        };
                        self.popup = Some(Popup::Menu { title, actions, sel: 0 });
                    }
                }
            }
            Key::Tab => self.details_focus = true,
            Key::Char('a') => {
                self.popup = Some(Popup::Input {
                    title: self.t("addComputer").into(),
                    text: self.t("addHelp").into(),
                    label: self.t("addLabel").into(),
                    value: String::new(),
                    purpose: Purpose::AddPeer,
                    error: None,
                })
            }
            Key::Char('f') => {
                self.only_shared = !self.only_shared;
                self.message = Some((self.t(if self.only_shared { "filterOn" } else { "filterOff" }).into(), false));
                self.update_flat();
            }
            Key::Char('l') => {
                self.lang = self.lang.other();
                self.model = self.build();
                self.update_flat();
            }
            Key::Char('r') => self.refresh(),
            Key::Char('?') | Key::Char('h') => self.popup = Some(Popup::Info { title: self.t("helpTitle").into(), text: self.t("help").into(), scroll: 0 }),
            Key::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn popup_key(&mut self, popup: Popup, k: Key) {
        match popup {
            Popup::Menu { title, actions, mut sel } => {
                let choose = |app: &mut App, a: Action| match a.confirm.clone() {
                    Some(text) => app.popup = Some(Popup::Confirm { text, action: a }),
                    None => app.run_action(&a),
                };
                match k {
                    Key::Up => sel = sel.saturating_sub(1),
                    Key::Down => sel = (sel + 1).min(actions.len() - 1),
                    Key::Esc | Key::Left => return,
                    Key::Enter | Key::Char(' ') => return choose(self, actions[sel].clone()),
                    Key::Char(c @ '1'..='9') => {
                        let n = c as usize - '1' as usize;
                        if n < actions.len() {
                            return choose(self, actions[n].clone());
                        }
                    }
                    _ => {}
                }
                self.popup = Some(Popup::Menu { title, actions, sel });
            }
            Popup::Confirm { text, action } => match k {
                Key::Enter | Key::Char('j' | 'J' | 'y' | 'Y') => self.run_action(&action),
                Key::Esc | Key::Char('n' | 'N') => {}
                _ => self.popup = Some(Popup::Confirm { text, action }),
            },
            Popup::Input { title, text, label, mut value, purpose, mut error } => {
                match k {
                    Key::Esc => {
                        // closing the code request declines it
                        if let Purpose::PairCode(id) = purpose {
                            self.reported.insert(id);
                            let _ = api::call(self.config.api_port, &Request::PairAnswer { id, code: None });
                        }
                        return;
                    }
                    Key::Enter => {
                        let request = match purpose {
                            Purpose::AddPeer => Request::AddPeer { address: value.trim().to_string() },
                            Purpose::PairCode(id) => Request::PairAnswer { id, code: Some(value.clone()) },
                        };
                        match api::call(self.config.api_port, &request) {
                            Ok(Response::Error { message }) => error = Some(message),
                            Err(e) => error = Some(format!("{e:#}")),
                            Ok(_) => {
                                if purpose == Purpose::AddPeer {
                                    self.message = Some((self.tv("done", &[("action", self.t("addComputer"))]), false));
                                }
                                self.refresh();
                                return;
                            }
                        }
                    }
                    Key::Backspace => {
                        value.pop();
                    }
                    Key::Char(c) if value.len() < 64 => {
                        let ok = match purpose {
                            Purpose::PairCode(_) => c.is_ascii_digit() || c == ' ',
                            Purpose::AddPeer => c.is_ascii_graphic(),
                        };
                        if ok {
                            value.push(c);
                        }
                    }
                    _ => {}
                }
                self.popup = Some(Popup::Input { title, text, label, value, purpose, error });
            }
            Popup::Info { title, text, mut scroll } => match k {
                Key::Esc | Key::Enter | Key::Char('q') => {}
                Key::Up => self.popup = Some(Popup::Info { title, text, scroll: scroll.saturating_sub(1) }),
                Key::Down => {
                    scroll += 1;
                    self.popup = Some(Popup::Info { title, text, scroll });
                }
                _ => self.popup = Some(Popup::Info { title, text, scroll }),
            },
            Popup::Code { id } => match k {
                Key::Esc | Key::Enter => {
                    self.hidden_codes.insert(id);
                }
                _ => self.popup = Some(Popup::Code { id }),
            },
        }
    }

    // ------------------------------------------------------------------ details

    /// (marker, text) for the status line of an item: "*" fine/in use, "!" attention.
    fn status_line(&self, (hi, di): (usize, Option<usize>)) -> (char, String) {
        let h = &self.model[hi];
        let Some(di) = di else {
            if h.local {
                return if self.status.is_some() { ('*', self.t("serviceOk").into()) } else { ('!', self.t("serviceDown").into()) };
            }
            let p = h.peer.as_ref().unwrap();
            if h.online && !h.paired {
                return ('!', if p.paired { self.tv("stateUnpairedThere", &[("host", &h.name)]) } else { self.t("stateNotPaired").into() });
            }
            if h.online {
                return ('*', self.t("peerOnline").into());
            }
            return ('-', format!("{}{}", self.t("peerOffline"), p.error.as_ref().map(|e| format!(" – {e}")).unwrap_or_default()));
        };
        let d = &h.devices[di];
        let host = h.name.as_str();
        if h.local {
            return match d.state {
                State::Shared => ('*', self.t("stateShared").into()),
                State::Absent => ('-', self.t("stateAbsentLocal").into()),
                State::Lent => ('*', self.tv("stateLent", &[("host", d.used_by.as_deref().unwrap_or("?"))])),
                State::Blocked => (
                    '-',
                    self.t(match d.lock {
                        Some("auto") => "stateLockedAuto",
                        Some("user") => "stateLockedUser",
                        _ => "stateBlocked",
                    })
                    .into(),
                ),
                _ => ('-', self.t("stateLocal").into()),
            };
        }
        if let Some(imp) = d.imp.as_ref().filter(|i| !i.wanted && i.state == ImportState::Disabled && i.last_error.is_some()) {
            return ('!', self.tv("disabled", &[("error", imp.last_error.as_deref().unwrap_or(""))]));
        }
        match (d.state, &d.imp) {
            (State::Here, Some(imp)) => {
                let vars = [("host", host), ("error", imp.last_error.as_deref().unwrap_or(""))];
                match imp.state {
                    ImportState::Attached if imp.last_error.is_some() => ('!', format!("{} – {}", self.t("attached"), imp.last_error.as_deref().unwrap_or(""))),
                    ImportState::Attached => ('*', self.t("attached").into()),
                    ImportState::Waiting => ('!', self.tv("waiting", &vars)),
                    ImportState::Disabled => ('-', self.tv("disabled", &vars)),
                    ImportState::Connecting | ImportState::Idle => ('!', self.tv("connecting", &vars)),
                }
            }
            (State::Used, _) => ('*', self.tv("statePeerUsed", &[("user", d.used_by.as_deref().unwrap_or("?"))])),
            (State::PeerShared, _) if h.online => ('*', self.tv("statePeerShared", &[("host", host)])),
            (State::PeerShared, _) => ('-', self.t("peerOffline").into()),
            (State::Never, _) => ('-', self.tv("statePeerNever", &[("host", host)])),
            (State::Absent, _) => ('-', self.tv("stateAbsent", &[("host", host)])),
            _ => ('-', self.tv("statePeerNotShared", &[("host", host)])),
        }
    }

    /// The details of the selected item as lines of at most `width` columns.
    fn details(&self, width: usize) -> Vec<(String, u8)> {
        let mut out: Vec<(String, u8)> = Vec::new();
        let Some(cur) = self.current() else { return out };
        let (hi, di) = cur;
        let h = &self.model[hi];
        let text = |out: &mut Vec<(String, u8)>, s: &str, attr: u8| out.extend(wrap(s, width).into_iter().map(|l| (l, attr)));
        let section = |out: &mut Vec<(String, u8)>, title: &str| {
            out.push((String::new(), 0));
            out.push((title.to_string(), BOLD));
        };
        let fact = |out: &mut Vec<(String, u8)>, label: &str, value: Option<String>| {
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                let lines = wrap(&format!("{label}: {v}"), width);
                for (i, l) in lines.into_iter().enumerate() {
                    out.push((if i == 0 { l } else { format!("  {l}") }, 0));
                }
            }
        };

        let (name, sub) = match di {
            Some(di) => (h.devices[di].name.clone(), self.kind_text(&h.devices[di].classes)),
            None => (h.name.clone(), if h.local { self.t("thisComputerTag").into() } else { String::new() }),
        };
        text(&mut out, &name, BOLD);
        if !sub.is_empty() {
            text(&mut out, &sub, DIM);
        }
        let (mark, line) = self.status_line(cur);
        text(&mut out, &format!("{mark} {line}"), 0);

        if let Some(e) = di.and_then(|di| h.devices[di].problem.as_ref()) {
            let d = &h.devices[di.unwrap()];
            let programs = e.blocking_processes.join(", ");
            let client = e.problem_client_name.clone().or_else(|| e.problem_client.clone()).unwrap_or_else(|| "?".into());
            let vars = [("client", client.as_str()), ("device", d.name.as_str()), ("programs", programs.as_str())];
            out.push((String::new(), 0));
            text(&mut out, &format!("! {}", self.tv("blockedTitle", &vars)), BOLD);
            text(&mut out, &self.tv(if programs.is_empty() { "blockedUnknown" } else { "blockedKnownPanel" }, &vars), 0);
        }

        let actions = self.actions_of(cur);
        if !actions.is_empty() {
            out.push((String::new(), 0));
            let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
            text(&mut out, &format!("[{}] {}", self.t("actionsHint"), labels.join(" | ")), REVERSE);
        } else if let Some(di) = di {
            let d = &h.devices[di];
            let hint = if h.local && !self.can_share {
                Some(self.t("hintNoShare").to_string())
            } else if !h.local && d.state == State::NotShared {
                Some(self.tv("hintAskShare", &[("host", &h.name)]))
            } else {
                None
            };
            if let Some(hint) = hint {
                out.push((String::new(), 0));
                text(&mut out, &hint, DIM);
            }
        }

        let Some(di) = di else {
            // a computer
            let p = h.peer.as_ref();
            let sys = if h.local { self.status.as_ref().and_then(|s| s.system.clone()) } else { p.and_then(|p| p.hello.as_ref()).and_then(|h| h.system.clone()) };
            if let Some(sys) = sys {
                section(&mut out, self.t("sectionSystem"));
                fact(&mut out, self.t("sysOs"), sys.os.clone());
                fact(&mut out, self.t("sysCpu"), sys.cpu.clone().map(|c| match sys.threads {
                    Some(n) => format!("{c} ({})", self.tv("threads", &[("n", &n.to_string())])),
                    None => c,
                }));
                fact(&mut out, self.t("sysGpu"), Some(sys.gpus.join(", ")));
                fact(&mut out, self.t("sysRam"), sys.ram_mb.map(|mb| if mb >= 1024 { format!("{} GB", (mb + 512) / 1024) } else { format!("{mb} MB") }));
            }
            section(&mut out, self.t("sectionDetails"));
            let can = |e: bool, i: bool| [e.then(|| self.t("canExport")), i.then(|| self.t("canImport"))].into_iter().flatten().collect::<Vec<_>>().join(", ");
            if h.local {
                let st = self.status.as_ref();
                fact(&mut out, self.t("hostVersion"), st.map(|s| s.version.clone()));
                fact(&mut out, self.t("hostCan"), Some(can(self.can_share, self.can_import)));
                fact(&mut out, self.t("hostKey"), st.map(|s| s.fingerprint.clone()));
                fact(&mut out, self.t("hostDevices"), Some(h.devices.len().to_string()));
                fact(&mut out, self.t("factAudioRules"), st.and_then(|s| s.audio_rules).map(|a| self.t(if a.active { "audioActive" } else { "audioInactive" }).to_string()));
                fact(&mut out, self.t("hostId"), st.map(|s| s.host_id.clone()));
                if self.status.is_none() {
                    out.push((String::new(), 0));
                    text(&mut out, self.t("serviceDownText"), 0);
                    if let Some(e) = &self.status_error {
                        text(&mut out, e, DIM);
                    }
                }
            } else if let Some(p) = p {
                let how = if p.added {
                    "how_added"
                } else if p.manual {
                    "how_config"
                } else {
                    "how_discovered"
                };
                fact(&mut out, self.t("hostAddress"), Some(p.address.clone()));
                fact(&mut out, self.t("hostVersion"), p.hello.as_ref().map(|h| h.version.clone()));
                fact(&mut out, self.t("hostCan"), p.hello.as_ref().map(|h| can(h.can_export, h.can_import)));
                fact(&mut out, self.t("hostHow"), Some(self.t(how).into()));
                fact(&mut out, self.t("hostPaired"), Some(self.t(if h.paired { "yes" } else { "no" }).into()));
                fact(&mut out, self.t("hostKey"), p.fingerprint.clone());
                fact(&mut out, self.t("hostSeen"), p.last_seen_secs.map(|s| self.secs_text(s)));
                fact(&mut out, self.t("hostDevices"), Some(h.devices.len().to_string()));
                fact(&mut out, self.t("hostId"), p.hello.as_ref().map(|h| h.host_id.clone()));
            }
            return out;
        };

        // a device
        let d = &h.devices[di];
        let dev = &d.device;
        let warnings = if d.warnings.is_empty() { &d.profile.warnings } else { &d.warnings };
        if !warnings.is_empty() {
            section(&mut out, self.t("sectionWarnings"));
            for w in warnings {
                let mark = if RISKY.contains(w) { "!" } else { "-" };
                for (i, l) in wrap(self.t(&format!("warnText_{}", w.name())), width.saturating_sub(2)).into_iter().enumerate() {
                    out.push((format!("{} {l}", if i == 0 { mark } else { " " }), 0));
                }
            }
        }
        if let Some(i) = d.imp.as_ref().filter(|i| d.state == State::Here || i.attach_count > 0) {
            section(&mut out, self.t("sectionConnection"));
            fact(&mut out, self.t("connPort"), i.bus_id.clone());
            fact(&mut out, self.t("connSince"), (i.state == ImportState::Attached).then(|| self.secs_text(i.since_secs)));
            fact(&mut out, self.t("connCount"), (i.attach_count > 0).then(|| self.tv("times", &[("n", &i.attach_count.to_string())])));
            fact(&mut out, self.t("connError"), i.last_error.clone());
        }
        section(&mut out, self.t("sectionDetails"));
        fact(&mut out, self.t("factComputer"), Some(h.name.clone()));
        fact(&mut out, self.t("factKind"), Some(self.kind_text(&d.classes)));
        fact(&mut out, self.t("factIds"), Some(format!("{}:{}", hex(dev.vendor_id), hex(dev.product_id))));
        fact(&mut out, self.t("factSerial"), dev.serial.clone());
        fact(&mut out, self.t("factMaker"), dev.manufacturer.clone());
        fact(&mut out, self.t("factProduct"), dev.product.clone());
        fact(&mut out, self.t("factPort"), Some(dev.bus_id.clone()));
        fact(&mut out, self.t("factDriver"), dev.driver.clone());
        fact(&mut out, self.t("factPlatformId"), h.local.then(|| dev.platform_id.clone()));
        let p = &d.profile;
        let ready: Vec<&str> = p.ready.iter().map(|r| self.t(&format!("ready_{}", name_of(r)))).collect();
        let audio = p.linux_audio.as_ref().map(|la| {
            let mut parts = Vec::new();
            if la.never_default == Some(true) {
                parts.push(self.t("laNeverDefault").to_string());
            }
            if la.pro_audio == Some(true) {
                parts.push(self.t("laProAudio").to_string());
            }
            if la.headroom.is_some() || la.period_size.is_some() {
                let s = |v: Option<u32>| v.map_or("-".to_string(), |v| v.to_string());
                parts.push(self.tv("laBuffer", &[("h", &s(la.headroom)), ("p", &s(la.period_size))]));
            }
            parts.join(", ")
        });
        let mut prof = Vec::new();
        fact(&mut prof, self.t("profName"), p.name.clone());
        fact(&mut prof, self.t("profBlocking"), Some(p.blocking_processes.join(", ")));
        fact(&mut prof, self.t("profReady"), Some(ready.join(", ")));
        fact(&mut prof, self.t("profReconnect"), Some(self.t(&format!("reconnect_{}", name_of(&p.reconnect))).into()));
        fact(&mut prof, self.t("profLinuxAudio"), audio);
        if !prof.is_empty() {
            section(&mut out, self.t("sectionProfile"));
            out.extend(prof);
        }
        out
    }

    // ------------------------------------------------------------------ drawing

    fn chip(&self, d: &Dev) -> String {
        if d.problem.is_some() {
            return self.t("chipProblem").into();
        }
        match d.state {
            State::Shared | State::PeerShared => self.t("chipShared").into(),
            State::Lent | State::Used => format!("-> {}", d.used_by.as_deref().unwrap_or("?")),
            State::Here if d.imp.as_ref().is_some_and(|i| i.state == ImportState::Waiting) => self.t("chipWaiting").into(),
            State::Here if d.imp.as_ref().is_some_and(|i| i.state != ImportState::Attached) => self.t("chipConnecting").into(),
            State::Here => self.t("chipHere").into(),
            State::Blocked | State::Never => self.t("chipLocked").into(),
            State::Absent => self.t("chipAbsent").into(),
            State::Local | State::NotShared => String::new(),
        }
    }

    fn alerts(&self) -> Vec<String> {
        let mut out = Vec::new();
        match &self.status {
            None => out.push(format!("{}: {}", self.t("serviceDown"), self.t("serviceDownText"))),
            Some(s) => {
                if self.audio_wanted && s.audio_rules.is_some_and(|a| !a.active) {
                    out.push(self.t("audioRulesTitle").into());
                }
                for e in s.exports.iter().filter(|e| e.problem.is_some()) {
                    let device = e.device.as_ref().map(|d| d.display_name()).unwrap_or_else(|| e.rule.clone());
                    let client = e.problem_client_name.clone().or_else(|| e.problem_client.clone()).unwrap_or_else(|| "?".into());
                    out.push(self.tv("blockedTitle", &[("client", &client), ("device", &device)]));
                }
            }
        }
        out
    }

    fn draw(&mut self, term: &mut Terminal) {
        let (w, h) = term.size();
        let mut c = Canvas::new(w, h);
        if w < 40 || h < 12 {
            c.put(0, 0, self.t("tooSmall"), BOLD);
            term.draw(c.lines());
            return;
        }

        // title bar
        c.fill(0, 0, w, 1, REVERSE);
        let host = self.status.as_ref().map_or_else(|| self.t("thisComputer").to_string(), |s| s.host.clone());
        c.put(1, 0, &format!("USB-IP-Bridge {} - {host}", usbipb_core::VERSION), REVERSE | BOLD);
        let right = format!("{}  |  ?: {} ", self.t(if self.status.is_some() { "serviceOk" } else { "serviceDown" }), self.t("helpTitle"));
        c.put(w.saturating_sub(right.len()), 0, &right, REVERSE);

        let mut y = 1;
        for a in self.alerts().into_iter().take(3) {
            c.put_max(0, y, &format!("! {a}"), BOLD, w);
            y += 1;
        }

        // tree and details
        let body_h = h - y - 2;
        let split = w >= 72;
        let tree_w = if split { (w * 2 / 5).clamp(30, 52) } else { w };
        if split || !self.details_focus {
            self.draw_tree(&mut c, 0, y, tree_w, body_h);
        }
        if split {
            self.draw_details(&mut c, tree_w - 1, y, w - tree_w + 1, body_h);
        } else if self.details_focus {
            self.draw_details(&mut c, 0, y, w, body_h);
        }

        // message and keys
        if let Some((msg, error)) = &self.message {
            c.put_max(1, h - 2, &format!("{}{msg}", if *error { "! " } else { "" }), if *error { BOLD } else { 0 }, w - 2);
        } else if self.only_shared {
            c.put_max(1, h - 2, self.t("filterOn"), DIM, w - 2);
        }
        c.fill(0, h - 1, w, 1, REVERSE);
        let keys = match &self.popup {
            Some(Popup::Menu { .. }) => "keysMenu",
            Some(Popup::Confirm { .. }) => "keysConfirm",
            Some(Popup::Input { purpose: Purpose::PairCode(_), .. }) => "keysPairIn",
            Some(Popup::Input { .. }) => "keysInput",
            Some(Popup::Info { .. }) | Some(Popup::Code { .. }) => "keysClose",
            None if self.details_focus => "keysDetails",
            None => "keysMain",
        };
        c.put_max(1, h - 1, self.t(keys), REVERSE, w - 2);

        self.draw_popup(&mut c);
        term.draw(c.lines());
    }

    fn draw_tree(&mut self, c: &mut Canvas, x: usize, y: usize, w: usize, h: usize) {
        c.frame(x, y, w, h, self.t("treeTitle"), if self.details_focus { 0 } else { BOLD });
        let rows = h.saturating_sub(2);
        let inner = w.saturating_sub(2);
        let sel = self.selected_index().unwrap_or(0);
        if sel < self.tree_scroll {
            self.tree_scroll = sel;
        } else if sel >= self.tree_scroll + rows {
            self.tree_scroll = sel + 1 - rows;
        }
        for (row, &(hi, di)) in self.flat.iter().enumerate().skip(self.tree_scroll).take(rows) {
            let yy = y + 1 + row - self.tree_scroll;
            let host = &self.model[hi];
            let (label, right, dim) = match di {
                None => {
                    let meta = if host.local {
                        self.t("thisComputerTag")
                    } else if !host.online {
                        self.t("peerOffline")
                    } else if !host.paired {
                        self.t("notPaired")
                    } else {
                        ""
                    };
                    let mark = if self.collapsed.contains(&host.key) { "[+]" } else { "[-]" };
                    (format!("{mark} {}", host.name), meta.to_string(), !host.online)
                }
                Some(di) => {
                    let d = &host.devices[di];
                    (format!("    {}", d.name), self.chip(d), matches!(d.state, State::NotShared | State::Never | State::Blocked | State::Absent))
                }
            };
            let selected = row == sel;
            let attr = if selected {
                if self.details_focus {
                    BOLD
                } else {
                    REVERSE
                }
            } else if dim {
                DIM
            } else if di.is_none() {
                BOLD
            } else {
                0
            };
            c.fill(x + 1, yy, inner, 1, if selected { attr } else { 0 });
            let right = if right.is_empty() { String::new() } else { format!(" {right}") };
            let room = inner.saturating_sub(right.len() + 1);
            c.put_max(x + 1, yy, &label, attr, room);
            c.put_max(x + 1 + inner - right.len().min(inner), yy, &right, attr, inner);
        }
        if self.flat.len() > self.tree_scroll + rows {
            c.put(x + w - 4, y + h - 1, "v", 0);
        }
        if self.tree_scroll > 0 {
            c.put(x + w - 4, y, "^", 0);
        }
    }

    fn draw_details(&mut self, c: &mut Canvas, x: usize, y: usize, w: usize, h: usize) {
        c.frame(x, y, w, h, self.t("detailsTitle"), if self.details_focus { BOLD | REVERSE } else { 0 });
        let lines = self.details(w.saturating_sub(4));
        let rows = h.saturating_sub(2);
        self.details_scroll = self.details_scroll.min(lines.len().saturating_sub(rows));
        for (i, (line, attr)) in lines.iter().skip(self.details_scroll).take(rows).enumerate() {
            c.put_max(x + 2, y + 1 + i, line, *attr, w.saturating_sub(4));
        }
        if lines.len() > self.details_scroll + rows {
            c.put(x + w - 4, y + h - 1, "v", 0);
        }
        if self.details_scroll > 0 {
            c.put(x + w - 4, y, "^", 0);
        }
    }

    fn draw_popup(&self, c: &mut Canvas) {
        let Some(popup) = &self.popup else { return };
        let max_w = c.w.saturating_sub(4).min(76);
        let tw = max_w.saturating_sub(4);
        // (title, lines with attributes)
        let (title, lines): (String, Vec<(String, u8)>) = match popup {
            Popup::Menu { title, actions, sel } => {
                (title.clone(), actions.iter().enumerate().map(|(i, a)| (format!(" {}  {} ", i + 1, a.label), if i == *sel { REVERSE } else { 0 })).collect())
            }
            Popup::Confirm { text, .. } => (self.t("actionsTitle").into(), wrap(text, tw).into_iter().map(|l| (l, 0)).collect()),
            Popup::Input { title, text, label, value, error, .. } => {
                let mut l: Vec<(String, u8)> = wrap(text, tw).into_iter().map(|l| (l, 0)).collect();
                l.push((String::new(), 0));
                l.push((label.clone(), BOLD));
                l.push((format!("> {value}_"), REVERSE));
                if let Some(e) = error {
                    l.push((String::new(), 0));
                    l.extend(wrap(&format!("! {e}"), tw).into_iter().map(|x| (x, BOLD)));
                }
                (title.clone(), l)
            }
            Popup::Info { title, text, scroll } => {
                let all = wrap(text, tw);
                let rows = c.h.saturating_sub(8);
                let start = (*scroll).min(all.len().saturating_sub(rows));
                (title.clone(), all.into_iter().skip(start).take(rows).map(|l| (l, 0)).collect())
            }
            Popup::Code { id } => {
                let p = self.status.as_ref().and_then(|s| s.pairings.iter().find(|p| p.id == *id));
                let host = p.map(|p| p.host_name.clone()).unwrap_or_default();
                let code = p.and_then(|p| p.code.clone()).unwrap_or_default();
                let code = if code.len() == 6 { format!("{} {}", &code[..3], &code[3..]) } else { code };
                let mut l: Vec<(String, u8)> = wrap(&self.tv("pairOutText", &[("host", &host)]), tw).into_iter().map(|l| (l, 0)).collect();
                l.push((String::new(), 0));
                l.push((format!("      {code}      "), BOLD | REVERSE));
                (self.tv("pairOutTitle", &[("host", &host)]), l)
            }
        };
        let content_w = lines.iter().map(|(l, _)| term::ascii(l).len()).chain([term::ascii(&title).len() + 4]).max().unwrap_or(10);
        let bw = (content_w + 4).clamp(24, max_w.max(24)).min(c.w);
        let bh = (lines.len() + 4).min(c.h);
        let bx = (c.w - bw) / 2;
        let by = (c.h.saturating_sub(bh)) / 2;
        c.frame(bx, by, bw, bh, &title, BOLD);
        for (i, (line, attr)) in lines.iter().enumerate().take(bh.saturating_sub(4)) {
            c.put_max(bx + 2, by + 2 + i, line, *attr, bw - 4);
        }
    }
}
