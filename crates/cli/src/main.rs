//! usbipb – command line tool.
//!
//!   usbipb panel [--lang de|en]
//!                    interactive panel for the terminal (plain ASCII)
//!   usbipb list      USB devices on this computer
//!   usbipb status    what the service shares and attaches
//!   usbipb attach/detach [N]
//!   usbipb share/unshare VID:PID[:SERIAL]
//!   usbipb pair COMPUTER      pair with another computer (shows a code to type there)
//!   usbipb pair-answer ID CODE|no
//!                    answer another computer's pairing request (see `usbipb status`)
//!   usbipb unpair COMPUTER
//!   usbipb run [--device D] [--wait S] -- PROGRAM [ARGS...]
//!                    attach a device, wait until it is usable, run the program, and
//!                    give the device back afterwards (if it was not here before)

mod panel;

use anyhow::Result;
use usbipb_core::api::{self, ImportState, PairingState, Request, Response};
use usbipb_core::config::{config_path, Config};
use usbipb_core::model::DeviceState;
use usbipb_core::profiles::Profiles;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("panel") => panel::run(),
        Some("list") => list(),
        Some("status") => status(),
        Some("attach") => wish(true),
        Some("detach") => wish(false),
        Some("run") => run(),
        Some("descriptors") => descriptors(),
        Some("share") => share(true),
        Some("unshare") => share(false),
        Some("pair") => pair(),
        Some("pair-answer") => pair_answer(),
        Some("unpair") => unpair(),
        Some("--version") | Some("-V") => {
            println!("{}", usbipb_core::title());
            Ok(())
        }
        _ => {
            eprintln!("{}\nusage: usbipb panel [--lang de|en] | list | status | attach [N] | detach [N] | run [--device D] [--wait S] -- PROGRAM [ARGS] | share/unshare VID:PID[:SERIAL] | pair COMPUTER | pair-answer ID CODE|no | unpair COMPUTER | --version", usbipb_core::title());
            std::process::exit(2);
        }
    }
}

fn list() -> Result<()> {
    let mut config = Config::load(&config_path()).unwrap_or_default();
    let mut locks = usbipb_core::api::Locks { config: config.never_share.clone(), ..Default::default() };
    // What is shared and locked comes from the service (it includes changes made in the app).
    if let Ok(Response::Status(s)) = api::call(config.api_port, &Request::Status) {
        config.export = s.shared.into_iter().map(|device| usbipb_core::config::ExportRule { device, allowed_clients: vec![], blocking_processes: vec![] }).collect();
        locks = s.locks;
    }
    locks.apply(&mut config);
    let devices = usbipb_host::list_devices();
    let profiles = usbipb_core::profiles::Profiles::load();
    println!("{:<8} {:<9} {:<12} {:<22} DEVICE", "BUS ID", "VID:PID", "STATE", "KIND");
    for d in devices {
        let state = if config.is_blocked(&d) {
            "locked"
        } else if config.export.iter().any(|r| r.device.matches(&d)) {
            "shared"
        } else {
            match d.state {
                DeviceState::Attached => "attached",
                _ => "local",
            }
        };
        let p = profiles.resolve(d.vendor_id, d.product_id, &d.classes);
        let kind: Vec<&str> = p.classes.iter().map(|c| c.name()).collect();
        let kind = if kind.is_empty() { "?".to_string() } else { kind.join("+") };
        let warn: Vec<&str> = p.warnings.iter().map(|w| w.name()).collect();
        let warn = if warn.is_empty() { String::new() } else { format!("  [! {}]", warn.join(", ")) };
        let name = p.name.clone().unwrap_or_else(|| d.display_name());
        println!("{:<8} {:04x}:{:04x} {:<12} {:<22} {name}{warn}", d.bus_id, d.vendor_id, d.product_id, state, kind);
    }
    Ok(())
}

/// Attach (bring here) or detach (give back) import rule N, or all.
fn wish(attach: bool) -> Result<()> {
    let config = Config::load(&config_path()).unwrap_or_default();
    let index = std::env::args().nth(2).map(|s| s.parse::<usize>()).transpose()?.map(|n| n.saturating_sub(1));
    let req = if attach { Request::Attach { index } } else { Request::Detach { index } };
    match api::call(config.api_port, &req)? {
        Response::Ok { message } => println!("{message}"),
        Response::Error { message } => anyhow::bail!(message),
        Response::Status(_) => {}
    }
    Ok(())
}

fn status() -> Result<()> {
    let config = Config::load(&config_path()).unwrap_or_default();
    match api::call(config.api_port, &Request::Status)? {
        Response::Status(s) => {
            println!("{} on {} (service v{}, key {})", usbipb_core::title(), s.host, s.version, s.fingerprint);
            for p in &s.pairings {
                let state = match p.state {
                    PairingState::Waiting if p.outgoing => format!("type the code {} on {}", p.code.as_deref().unwrap_or("?"), p.host_name),
                    PairingState::Waiting => format!("{} asks to pair: `usbipb pair-answer {} CODE` with the code shown there (or `no`)", p.host_name, p.id),
                    PairingState::Paired => format!("paired with {}", p.host_name),
                    PairingState::Failed => format!("pairing with {} failed: {}", p.host_name, p.message.as_deref().unwrap_or("")),
                };
                println!("  pairing: {state}");
            }
            if s.exports.is_empty() && s.imports.is_empty() {
                println!("nothing configured");
            }
            for e in &s.exports {
                let name = e.device.as_ref().map(|d| format!("{} ({})", d.display_name(), d.bus_id)).unwrap_or_else(|| e.rule.clone());
                let who = match e.client_name.as_ref().or(e.client.as_ref()) {
                    Some(c) => format!("in use by {c}"),
                    None if !e.present => "shared, not plugged in".into(),
                    None => "shared, waiting for a client".into(),
                };
                println!("  share  {name}: {who}{}", e.note.as_deref().map(|n| format!(" [{n}]")).unwrap_or_default());
                if let Some(p) = &e.problem {
                    println!(
                        "         ! {} wanted it {} s ago, but: {p}",
                        e.problem_client_name.as_ref().or(e.problem_client.as_ref()).map_or("a client", |c| c.as_str()),
                        e.problem_age_secs.unwrap_or(0)
                    );
                    if !e.blocking_processes.is_empty() {
                        println!("         ! known to hold the device: {}", e.blocking_processes.join(", "));
                    }
                }
            }
            for (n, i) in s.imports.iter().enumerate() {
                let state = match i.state {
                    ImportState::Idle => "not attached (stays with its owner; `usbipb attach` brings it here)".into(),
                    ImportState::Attached => format!(
                        "attached ({}), {} s{}",
                        i.bus_id.as_deref().unwrap_or("?"),
                        i.since_secs,
                        i.last_error.as_deref().map(|e| format!(" – {e}")).unwrap_or_default()
                    ),
                    ImportState::Connecting => "connecting".into(),
                    ImportState::Waiting => format!("waiting to reconnect: {}", i.last_error.as_deref().unwrap_or("")),
                    ImportState::Disabled => format!("stopped: {}", i.last_error.as_deref().unwrap_or("")),
                };
                println!("  #{} attach {}: {state} [attached {}x]", n + 1, i.rule, i.attach_count);
                if !i.classes.is_empty() {
                    let kind: Vec<&str> = i.classes.iter().map(|c| c.name()).collect();
                    let warn: Vec<&str> = i.warnings.iter().map(|w| w.name()).collect();
                    let warn = if warn.is_empty() { String::new() } else { format!("  [! {}]", warn.join(", ")) };
                    println!("     {} – {}{warn}", i.device_name.as_deref().unwrap_or("?"), kind.join("+"));
                }
            }
            for p in &s.peers {
                let state = if p.online { "online".to_string() } else { format!("offline ({})", p.error.as_deref().unwrap_or("not seen yet")) };
                let version = p.hello.as_ref().map(|h| format!(" v{}", h.version)).unwrap_or_default();
                let paired = match (p.paired, p.hello.as_ref().map(|h| h.paired)) {
                    (true, Some(true)) => "",
                    (true, Some(false)) => " – the pairing was removed there (`usbipb pair` again)",
                    _ => " – not paired (`usbipb pair`)",
                };
                println!("  computer {} ({}){version}: {state}{paired}", p.name(), p.address);
                for d in p.devices.iter().filter(|d| d.offer == usbipb_core::peer::Offer::Shared) {
                    let used = match &d.used_by {
                        Some(u) => format!(" – in use by {u}"),
                        None if !d.present => " – not plugged in".into(),
                        None => String::new(),
                    };
                    println!("     shares {} ({}){used}", d.name, d.classes.iter().map(|c| c.name()).collect::<Vec<_>>().join("+"));
                }
            }
        }
        Response::Error { message } => anyhow::bail!(message),
        Response::Ok { message } => println!("{message}"),
    }
    Ok(())
}

fn call_ok(req: &Request) -> Result<String> {
    let config = Config::load(&config_path()).unwrap_or_default();
    match api::call(config.api_port, req)? {
        Response::Ok { message } => Ok(message),
        Response::Error { message } => anyhow::bail!(message),
        Response::Status(_) => Ok(String::new()),
    }
}

fn service_status() -> Result<usbipb_core::api::Status> {
    let config = Config::load(&config_path()).unwrap_or_default();
    match api::call(config.api_port, &Request::Status)? {
        Response::Status(s) => Ok(s),
        Response::Error { message } | Response::Ok { message } => anyhow::bail!(message),
    }
}

/// A known computer by name or address, as its control address.
fn find_computer(s: &usbipb_core::api::Status, what: &str) -> Option<usbipb_core::peer::PeerStatus> {
    let what = what.to_lowercase();
    s.peers.iter().find(|p| p.name().to_lowercase() == what || p.address.to_lowercase() == what || p.address.to_lowercase().starts_with(&format!("{what}:"))).cloned()
}

/// Pairs with another computer: shows the code and waits until it was typed there.
fn pair() -> Result<()> {
    let Some(what) = std::env::args().nth(2) else { anyhow::bail!("usage: usbipb pair COMPUTER (name or address)") };
    let address = find_computer(&service_status()?, &what).map(|p| p.address).unwrap_or_else(|| if what.contains(':') { what.clone() } else { format!("{what}:{}", usbipb_core::peer::PEER_PORT) });
    call_ok(&Request::Pair { address: address.clone() })?;
    let mut shown = None;
    loop {
        let s = service_status()?;
        let Some(p) = s.pairings.iter().filter(|p| p.outgoing && p.address == address).max_by_key(|p| p.id) else { anyhow::bail!("the pairing disappeared") };
        match p.state {
            PairingState::Waiting => {
                if shown != Some(p.id) {
                    println!("Type this code on {} (USB-IP-Bridge asks for it there): {}", p.host_name, p.code.as_deref().unwrap_or("?"));
                    shown = Some(p.id);
                }
            }
            PairingState::Paired => {
                println!("Paired with {}.", p.host_name);
                return Ok(());
            }
            PairingState::Failed => anyhow::bail!("pairing with {} failed: {}", p.host_name, p.message.as_deref().unwrap_or("")),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn pair_answer() -> Result<()> {
    let (Some(id), Some(code)) = (std::env::args().nth(2), std::env::args().nth(3)) else { anyhow::bail!("usage: usbipb pair-answer ID CODE|no") };
    let id: u64 = id.parse()?;
    let code = if code.eq_ignore_ascii_case("no") { None } else { Some(code) };
    let declined = code.is_none();
    call_ok(&Request::PairAnswer { id, code })?;
    if declined {
        println!("declined");
        return Ok(());
    }
    loop {
        let s = service_status()?;
        match s.pairings.iter().find(|p| p.id == id) {
            Some(p) if p.state == PairingState::Paired => {
                println!("Paired with {}.", p.host_name);
                return Ok(());
            }
            Some(p) if p.state == PairingState::Failed => anyhow::bail!("{}", p.message.as_deref().unwrap_or("pairing failed")),
            None => anyhow::bail!("the pairing disappeared"),
            _ => std::thread::sleep(Duration::from_millis(300)),
        }
    }
}

fn unpair() -> Result<()> {
    let Some(what) = std::env::args().nth(2) else { anyhow::bail!("usage: usbipb unpair COMPUTER") };
    let s = service_status()?;
    let wanted = what.to_lowercase();
    let host_id = s
        .paired
        .iter()
        .find(|p| p.host_name.to_lowercase() == wanted || p.host_id == what)
        .map(|p| p.host_id.clone())
        .or_else(|| find_computer(&s, &what).and_then(|p| p.hello.map(|h| h.host_id).or(p.known_id)))
        .ok_or_else(|| anyhow::anyhow!("not paired with {what}"))?;
    println!("{}", call_ok(&Request::Unpair { host_id })?);
    Ok(())
}

/// Prints a message, and shows it as a desktop notification where available.
fn say(message: &str) {
    eprintln!("usbipb: {message}");
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("notify-send").args(["-a", "USB-IP-Bridge", "-i", "drive-removable-media", "USB-IP-Bridge", message]).status();
}

/// Import rule index for `--device`: a number (as in `usbipb status`), vid:pid, or part of the name.
fn pick_import(config: &Config, s: &api::Status, device: Option<&str>) -> Result<usize> {
    let Some(want) = device else {
        return match s.imports.len() {
            1 => Ok(0),
            0 => anyhow::bail!("no device to attach is configured"),
            _ => anyhow::bail!("several devices are configured, choose one with --device (number, vid:pid or name)"),
        };
    };
    if let Ok(n) = want.parse::<usize>() {
        return if (1..=s.imports.len()).contains(&n) { Ok(n - 1) } else { anyhow::bail!("there is no device #{n}") };
    }
    let want = want.to_lowercase();
    let hits: Vec<usize> = (0..s.imports.len().min(config.import.len()))
        .filter(|&n| {
            let (i, rule) = (&s.imports[n], &config.import[n]);
            let id = format!("{:04x}:{:04x}", rule.device.vendor_id, rule.device.product_id);
            id == want || i.rule.to_lowercase().contains(&want) || i.device_name.as_deref().is_some_and(|d| d.to_lowercase().contains(&want))
        })
        .collect();
    match hits.as_slice() {
        [n] => Ok(*n),
        [] => anyhow::bail!("no configured device matches \"{want}\""),
        _ => anyhow::bail!("\"{want}\" matches several devices, be more specific"),
    }
}

/// Is the attached device usable (everything its profile waits for)?
fn device_ready(config: &Config, profiles: &Profiles, index: usize, status: &api::ImportStatus) -> bool {
    if status.state != ImportState::Attached {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let d = &config.import[index].device;
        let checks = profiles.resolve(d.vendor_id, d.product_id, &status.classes).ready;
        let Some(dir) = usbipb_host::linux::ready::attached_device_dir(d.vendor_id, d.product_id) else { return false };
        return checks.into_iter().all(|c| usbipb_host::linux::ready::is_ready(&dir, c));
    }
    #[allow(unreachable_code)]
    {
        let _ = (config, profiles, index);
        true
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (opts, program) = (&args[..split], args.get(split + 1..).unwrap_or_default());
    let mut device = None;
    let mut wait = 60u64;
    let mut it = opts.iter();
    while let Some(o) = it.next() {
        match o.as_str() {
            "--device" | "-d" => device = it.next().cloned(),
            "--wait" | "-w" => wait = it.next().map(|s| s.parse()).transpose()?.unwrap_or(wait),
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    let Some((cmd, cmd_args)) = program.split_first() else {
        anyhow::bail!("usage: usbipb run [--device D] [--wait SECONDS] -- PROGRAM [ARGS...]");
    };

    let config = Config::load(&config_path()).unwrap_or_default();
    let profiles = Profiles::load();
    let status = || match api::call(config.api_port, &Request::Status) {
        Ok(Response::Status(s)) => Ok(s),
        Ok(_) => anyhow::bail!("unexpected answer from the service"),
        Err(e) => Err(e),
    };
    // We give the device back afterwards only if we brought it here.
    let mut attached_by_us = None;
    match status() {
        Err(e) => say(&format!("service not reachable ({e:#}) – starting {cmd} without the device")),
        Ok(s) => 'attach: {
            // A launcher must still start the program, so problems are reported, not fatal.
            let index = match pick_import(&config, &s, device.as_deref()) {
                Ok(i) => i,
                Err(e) => {
                    say(&format!("{e:#} – starting {cmd} without the device"));
                    break 'attach;
                }
            };
            let name = s.imports[index].device_name.clone().unwrap_or_else(|| s.imports[index].rule.clone());
            if s.imports[index].state != ImportState::Attached {
                api::call(config.api_port, &Request::Attach { index: Some(index) })?;
                attached_by_us = Some(index);
            }
            let start = Instant::now();
            let mut hinted = false;
            let ready = loop {
                let s = status()?;
                let i = &s.imports[index];
                if device_ready(&config, &profiles, index, i) {
                    break true;
                }
                if start.elapsed() > Duration::from_secs(wait) {
                    break false;
                }
                if !hinted && start.elapsed() > Duration::from_secs(8) {
                    let reason = i.last_error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default();
                    say(&format!("Waiting for {name}…{reason} If a program on the other computer holds it, release it there."));
                    hinted = true;
                }
                std::thread::sleep(Duration::from_millis(500));
            };
            if !ready {
                say(&format!("{name} is not ready yet – starting {cmd} anyway; the device joins once it is free."));
            } else if hinted {
                say(&format!("{name} is here."));
            }
        }
    }

    let code = std::process::Command::new(cmd).args(cmd_args).status().map(|s| s.code().unwrap_or(1));
    if let Some(index) = attached_by_us {
        match api::call(config.api_port, &Request::Detach { index: Some(index) }) {
            Ok(Response::Ok { message }) => eprintln!("usbipb: {message}"),
            Ok(Response::Error { message }) => say(&message),
            Ok(_) => {}
            Err(e) => say(&format!("could not give the device back: {e:#}")),
        }
    }
    std::process::exit(code.map_err(|e| anyhow::anyhow!("cannot start {cmd}: {e}"))?);
}

/// Diagnostics: the configuration descriptor of a local device, one descriptor per line.
fn descriptors() -> Result<()> {
    let busid = std::env::args().nth(2).ok_or_else(|| anyhow::anyhow!("usage: usbipb descriptors BUSID"))?;
    let d = usbipb_host::list_devices().into_iter().find(|d| d.bus_id == busid).ok_or_else(|| anyhow::anyhow!("no device {busid}"))?;
    #[cfg(windows)]
    let raw = {
        let port: u16 = busid.split('-').nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        usbipb_host::windows::vbox::config_descriptor(&d.platform_id, port)
    };
    #[cfg(not(windows))]
    let raw = std::fs::read(format!("/sys/bus/usb/devices/{busid}/descriptors")).ok().map(|r| r.get(18..).unwrap_or_default().to_vec());
    let raw = raw.ok_or_else(|| anyhow::anyhow!("cannot read the descriptors of {}", d.display_name()))?;
    let mut i = 0;
    while i + 2 <= raw.len() && raw[i] >= 2 {
        let len = raw[i] as usize;
        let what = match raw[i + 1] {
            2 => "config",
            4 => "interface",
            5 => "endpoint",
            0x0b => "iface assoc",
            0x24 => "class (interface)",
            0x25 => "class (endpoint)",
            0x30 => "ss ep companion",
            _ => "other",
        };
        let bytes: Vec<String> = raw[i..(i + len).min(raw.len())].iter().map(|b| format!("{b:02x}")).collect();
        println!("{what:<18} {}", bytes.join(" "));
        i += len;
    }
    Ok(())
}

/// Shares a device of this computer with others, or stops sharing it.
fn share(share: bool) -> Result<()> {
    let arg = std::env::args().nth(2).ok_or_else(|| anyhow::anyhow!("usage: usbipb share|unshare VID:PID[:SERIAL]"))?;
    let mut parts = arg.splitn(3, ':');
    let hex = |p: Option<&str>| -> Result<u16> { Ok(u16::from_str_radix(p.ok_or_else(|| anyhow::anyhow!("expected VID:PID"))?, 16)?) };
    let device = usbipb_core::model::DeviceMatch { vendor_id: hex(parts.next())?, product_id: hex(parts.next())?, serial: parts.next().map(str::to_string) };
    let config = Config::load(&config_path()).unwrap_or_default();
    match api::call(config.api_port, &Request::Share { device, share })? {
        Response::Ok { message } => println!("{message}"),
        Response::Error { message } => anyhow::bail!(message),
        Response::Status(_) => {}
    }
    Ok(())
}
