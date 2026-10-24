//! Readiness checks for `usbipb run`: is an attached device usable yet?
//!
//! Each check looks for what the kernel (and for audio, PipeWire) creates once the
//! driver is bound: a sound card, a MIDI port, a video node, a block device, ...

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use usbipb_core::model::DeviceState;
use usbipb_core::profiles::ReadyCheck;

/// sysfs directory of the attached (vhci) device with this vendor/product id.
pub fn attached_device_dir(vendor_id: u16, product_id: u16) -> Option<PathBuf> {
    crate::linux::devices::list()
        .into_iter()
        .find(|d| d.state == DeviceState::Attached && d.vendor_id == vendor_id && d.product_id == product_id)
        .map(|d| Path::new("/sys/bus/usb/devices").join(d.bus_id))
}

/// All directories below `dir` (symlinks are not followed), with their parent's name.
fn walk(dir: &Path, depth: usize, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let parent = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for e in entries.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        out.push((parent.clone(), e.file_name().to_string_lossy().into_owned()));
        if depth > 0 {
            walk(&e.path(), depth - 1, out);
        }
    }
}

fn numbered(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Does PipeWire have a node for ALSA card `card`? True if PipeWire is not in use.
fn pipewire_has_card(card: u32) -> bool {
    let Ok(out) = Command::new("pw-dump").output() else { return true };
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("\"api.alsa.pcm.card\"")).any(|l| {
        let value: String = l.rsplit(':').next().unwrap_or("").chars().filter(char::is_ascii_digit).collect();
        value.parse() == Ok(card)
    })
}

/// Undoes the escaping in /proc/mounts (octal `\040` for a space etc.).
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Mounted file systems on the block devices (disks and partitions) of the device below
/// `dev`: (device node, mount point).
pub fn mounts(dev: &Path) -> Vec<(String, String)> {
    let mut dirs = Vec::new();
    walk(dev, 10, &mut dirs);
    let disks: Vec<&String> = dirs.iter().filter(|(p, _)| p == "block").map(|(_, n)| n).collect();
    let nodes: Vec<String> = dirs
        .iter()
        .filter(|(p, n)| p == "block" || (disks.contains(&p) && n.starts_with(p.as_str())))
        .map(|(_, n)| format!("/dev/{n}"))
        .collect();
    fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split(' ');
            let (src, target) = (f.next()?, f.next()?);
            nodes.iter().any(|n| n == src).then(|| (src.to_string(), unescape_mount(target)))
        })
        .collect()
}

/// Flushes and unmounts every file system of the device below `dev` (needs root).
/// Fails – leaving the rest mounted – if one is still in use.
pub fn unmount_all(dev: &Path) -> Result<(), String> {
    let m = mounts(dev);
    if m.is_empty() {
        return Ok(());
    }
    let _ = Command::new("sync").status();
    for (node, target) in m {
        let out = Command::new("umount").arg(&target).output().map_err(|e| format!("umount: {e}"))?;
        if !out.status.success() {
            return Err(format!("{target} is still in use – close the files there, then it is given back"));
        }
        log::info!("unmounted {node} from {target}");
    }
    Ok(())
}

/// Is the device below `dev` (its sysfs directory) ready for `check`?
pub fn is_ready(dev: &Path, check: ReadyCheck) -> bool {
    let mut dirs = Vec::new();
    walk(dev, 8, &mut dirs);
    let has = |pred: &dyn Fn(&str, &str) -> bool| dirs.iter().any(|(parent, name)| pred(parent, name));
    match check {
        ReadyCheck::Audio => dirs
            .iter()
            .filter(|(parent, name)| parent == "sound" && numbered(name, "card"))
            .filter_map(|(_, name)| name["card".len()..].parse().ok())
            .any(pipewire_has_card),
        ReadyCheck::Midi => has(&|_, n| n.starts_with("midiC")),
        ReadyCheck::Video => has(&|p, n| p == "video4linux" && numbered(n, "video")),
        ReadyCheck::Block => has(&|p, _| p == "block"),
        ReadyCheck::Tty => has(&|_, n| numbered(n, "ttyUSB") || numbered(n, "ttyACM")),
        ReadyCheck::Input => has(&|_, n| numbered(n, "event")),
        ReadyCheck::Driver => {
            let interfaces: Vec<_> = fs::read_dir(dev)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().contains(':'))
                .collect();
            !interfaces.is_empty() && interfaces.iter().all(|e| e.path().join("driver").exists())
        }
    }
}
