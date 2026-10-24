//! PipeWire/WirePlumber rules for audio devices attached over USB/IP.
//!
//! One rule covers every attached audio device: the kernel names their sound cards
//! "… at usb-vhci_hcd.0-N", so the rule matches on that instead of on single devices.
//! Values come from the device profile of the audio class (never the default device,
//! pro-audio profile, extra buffering). Both WirePlumber 0.4 (Lua) and 0.5 (SPA-JSON)
//! files are written to /etc/wireplumber; each version reads only its own format.
//!
//! WirePlumber reads rules only when it starts. `reload_if_idle` restarts it – but only
//! while no audio stream, game stream (Sunshine) or JACK program runs, so nothing is
//! interrupted.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};
use usbipb_core::api::AudioRules;
use usbipb_core::profiles::LinuxAudio;

const LUA_PATH: &str = "/etc/wireplumber/main.lua.d/51-usb-ip-bridge.lua";
const CONF_PATH: &str = "/etc/wireplumber/wireplumber.conf.d/51-usb-ip-bridge.conf";

fn lua(la: &LinuxAudio) -> String {
    let mut node = Vec::new();
    if la.never_default.unwrap_or(true) {
        node.push("    [\"priority.session\"] = 100,\n    [\"priority.driver\"] = 100,".to_string());
    }
    if let Some(h) = la.headroom {
        node.push(format!("    [\"api.alsa.headroom\"] = {h},"));
    }
    if let Some(p) = la.period_size {
        node.push(format!("    [\"api.alsa.period-size\"] = {p},"));
    }
    let profile = if la.pro_audio.unwrap_or(false) { "    [\"device.profile\"] = \"pro-audio\",\n" } else { "" };
    format!(
        "-- Written by USB-IP-Bridge (from the device profile of the audio class). Do not edit:\n\
         -- the service rewrites it. Applies to sound cards attached over USB/IP only.\n\
         table.insert(alsa_monitor.rules, {{\n  matches = {{ {{ {{ \"api.alsa.card.longname\", \"matches\", \"*usb-vhci_hcd*\" }} }} }},\n  apply_properties = {{\n{profile}  }},\n}})\n\
         -- low priority: never chosen as the default device (keeps system and game audio where it is)\n\
         table.insert(alsa_monitor.rules, {{\n  matches = {{ {{ {{ \"alsa.long_card_name\", \"matches\", \"*usb-vhci_hcd*\" }} }} }},\n  apply_properties = {{\n{}\n  }},\n}})\n",
        node.join("\n")
    )
}

fn conf(la: &LinuxAudio) -> String {
    let mut node = Vec::new();
    if la.never_default.unwrap_or(true) {
        node.push("        priority.session = 100\n        priority.driver = 100".to_string());
    }
    if let Some(h) = la.headroom {
        node.push(format!("        api.alsa.headroom = {h}"));
    }
    if let Some(p) = la.period_size {
        node.push(format!("        api.alsa.period-size = {p}"));
    }
    let profile = if la.pro_audio.unwrap_or(false) { "        device.profile = \"pro-audio\"\n" } else { "" };
    format!(
        "# Written by USB-IP-Bridge (from the device profile of the audio class). Do not edit:\n\
         # the service rewrites it. Applies to sound cards attached over USB/IP only.\n\
         monitor.alsa.rules = [\n  {{\n    matches = [ {{ api.alsa.card.longname = \"~.*usb-vhci_hcd.*\" }} ]\n    actions = {{\n      update-props = {{\n{profile}      }}\n    }}\n  }}\n\
         \x20 # low priority: never chosen as the default device (keeps system and game audio where it is)\n\
         \x20 {{\n    matches = [ {{ alsa.long_card_name = \"~.*usb-vhci_hcd.*\" }} ]\n    actions = {{\n      update-props = {{\n{}\n      }}\n    }}\n  }}\n]\n",
        node.join("\n")
    )
}

/// Major.minor of the installed WirePlumber, e.g. (0, 4).
fn wireplumber_version() -> Option<(u32, u32)> {
    let out = Command::new("wireplumber").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.split("libwireplumber ").nth(1)?;
    let mut parts = v.split('.');
    Some((parts.next()?.trim().parse().ok()?, parts.next()?.trim().parse().ok()?))
}

/// Is WirePlumber installed at all (PipeWire systems)? Without it there is nothing to set up.
fn wireplumber_installed() -> bool {
    Command::new("wireplumber").arg("--version").output().is_ok()
}

/// Writes the rules if they differ from what is installed. Returns true if a file changed.
/// Only the format of the installed WirePlumber is written (0.5 warns about Lua files);
/// both if the version is unknown; nothing without WirePlumber.
pub fn install(la: &LinuxAudio) -> Result<bool> {
    if !wireplumber_installed() {
        log::info!("WirePlumber is not installed: no audio rules needed");
        return Ok(false);
    }
    let (lua_wanted, conf_wanted) = match wireplumber_version() {
        Some((0, minor)) if minor < 5 => (true, false),
        Some(_) => (false, true),
        None => (true, true),
    };
    let mut changed = false;
    for (path, wanted) in [(LUA_PATH, lua_wanted), (CONF_PATH, conf_wanted)] {
        if !wanted && Path::new(path).exists() {
            fs::remove_file(path).with_context(|| format!("remove {path}"))?;
            log::info!("removed {path} (not used by this WirePlumber version)");
        }
    }
    let files = [(LUA_PATH, lua(la), lua_wanted), (CONF_PATH, conf(la), conf_wanted)];
    for (path, content) in files.into_iter().filter(|f| f.2).map(|f| (f.0, f.1)) {
        if fs::read_to_string(path).ok().as_deref() == Some(content.as_str()) {
            continue;
        }
        if let Some(dir) = Path::new(path).parent() {
            fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        fs::write(path, content).with_context(|| format!("write {path}"))?;
        log::info!("audio rules written to {path} (active after WirePlumber restarts)");
        changed = true;
    }
    Ok(changed)
}

/// Running WirePlumber processes (one per logged-in user): (uid, start time).
fn wireplumbers() -> Vec<(u32, SystemTime)> {
    let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter(|e| fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "wireplumber"))
        .filter_map(|e| {
            let uid = fs::read_to_string(e.path().join("status"))
                .ok()?
                .lines()
                .find_map(|l| l.strip_prefix("Uid:"))?
                .split_whitespace()
                .next()?
                .parse()
                .ok()?;
            // /proc/<pid> is created when the process starts
            let start = fs::metadata(e.path()).and_then(|m| m.modified()).ok()?;
            Some((uid, start))
        })
        .collect()
}

fn wireplumber_starts() -> Vec<SystemTime> {
    wireplumbers().into_iter().map(|(_, t)| t).collect()
}

fn user_name(uid: u32) -> Option<String> {
    fs::read_to_string("/etc/passwd").ok()?.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() > 2 && f[2] == uid.to_string()).then(|| f[0].to_string())
    })
}

/// Programs using audio right now, from `pw-dump` output: playing or recording streams
/// (a game stream captures audio) and JACK programs (DAWs).
pub fn busy_programs(pw_dump: &[u8]) -> Vec<String> {
    let objects: Vec<serde_json::Value> = serde_json::from_slice(pw_dump).unwrap_or_default();
    let mut busy: Vec<String> = objects
        .iter()
        .map(|o| &o["info"]["props"])
        .filter(|p| p["media.class"].as_str().is_some_and(|c| c.starts_with("Stream/")) || p["client.api"].as_str() == Some("jack"))
        .filter_map(|p| p["application.name"].as_str().or(p["node.name"].as_str()).map(str::to_string))
        .collect();
    busy.sort();
    busy.dedup();
    busy
}

/// Restarts WirePlumber of every logged-in user so the rules become active – unless audio
/// is in use (then nothing is restarted and the error names the programs). Needs root.
pub fn reload_if_idle() -> Result<()> {
    let users: Vec<(u32, String)> = wireplumbers().into_iter().filter_map(|(uid, _)| Some((uid, user_name(uid)?))).collect();
    for (uid, name) in &users {
        let out = Command::new("runuser")
            .args(["-u", name, "--", "env", &format!("XDG_RUNTIME_DIR=/run/user/{uid}"), "pw-dump"])
            .output()
            .context("pw-dump")?;
        let busy = busy_programs(&out.stdout);
        if !busy.is_empty() {
            bail!("audio is in use right now ({})", busy.join(", "));
        }
    }
    for (_, name) in &users {
        log::info!("restarting WirePlumber of {name} so it reads the audio rules (no audio in use)");
        let ok = Command::new("systemctl").args(["--user", "-M", &format!("{name}@"), "restart", "wireplumber"]).status().is_ok_and(|s| s.success());
        if !ok {
            bail!("restarting WirePlumber of {name} failed");
        }
    }
    // give PipeWire a moment to set the devices up again
    std::thread::sleep(Duration::from_secs(2));
    Ok(())
}

/// Are the rules installed, and has every running WirePlumber read them?
pub fn status() -> AudioRules {
    let written = [LUA_PATH, CONF_PATH].iter().filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok()).max();
    let installed = written.is_some();
    let active = match written {
        // without WirePlumber (e.g. PulseAudio only) there is nothing to wait for
        None => !wireplumber_installed(),
        Some(w) => wireplumber_starts().iter().all(|&start| start > w),
    };
    AudioRules { installed, active }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_contain_profile_values() {
        let la = LinuxAudio { never_default: Some(true), pro_audio: Some(true), headroom: Some(256), period_size: Some(256) };
        let l = lua(&la);
        assert!(l.contains("\"api.alsa.headroom\"] = 256") && l.contains("pro-audio") && l.contains("*usb-vhci_hcd*"));
        let c = conf(&la);
        assert!(c.contains("api.alsa.period-size = 256") && c.contains("priority.session = 100") && c.contains("~.*usb-vhci_hcd.*"));
    }

    #[test]
    fn busy_programs_finds_streams_and_jack() {
        let dump = br#"[
          {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Sink","node.name":"alsa_output.hdmi"}}},
          {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Stream/Input/Audio","application.name":"sunshine"}}},
          {"type":"PipeWire:Interface:Client","info":{"props":{"client.api":"jack","application.name":"MorningDAW"}}},
          {"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Midi/Bridge","node.name":"Midi-Bridge"}}}
        ]"#;
        assert_eq!(busy_programs(dump), vec!["MorningDAW".to_string(), "sunshine".to_string()]);
        assert!(busy_programs(b"[]").is_empty());
    }

    /// `USBIPB_DUMP_RULES=dir cargo test` writes both files there, for checking them with
    /// WirePlumber's tools before they are installed.
    #[test]
    fn dump_rules() {
        if let Ok(dir) = std::env::var("USBIPB_DUMP_RULES") {
            let la = LinuxAudio { never_default: Some(true), pro_audio: Some(true), headroom: Some(256), period_size: Some(256) };
            fs::write(Path::new(&dir).join("51-usb-ip-bridge.lua"), lua(&la)).unwrap();
            fs::write(Path::new(&dir).join("51-usb-ip-bridge.conf"), conf(&la)).unwrap();
        }
    }
}
