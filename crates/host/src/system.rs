//! What kind of computer this is (operating system, CPU, graphics, memory), shown to the user
//! of other computers. Read once; nothing here changes while the service runs.

use usbipb_core::peer::SystemInfo;

/// This computer's description (cached).
pub fn info() -> SystemInfo {
    static INFO: std::sync::OnceLock<SystemInfo> = std::sync::OnceLock::new();
    INFO.get_or_init(read).clone()
}

fn threads() -> Option<u32> {
    std::thread::available_parallelism().ok().map(|n| n.get() as u32)
}

#[cfg(windows)]
fn read() -> SystemInfo {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let reg = |key: &str, value: &str| -> Option<String> {
        let (key, value) = (HSTRING::from(key), HSTRING::from(value));
        let mut buf = vec![0u16; 512];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), PCWSTR(value.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
        };
        if r.is_err() {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..(len as usize / 2)]);
        Some(s.trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
    };

    const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build: u32 = reg(NT, "CurrentBuild").and_then(|b| b.parse().ok()).unwrap_or(0);
    let mut product = reg(NT, "ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 still calls itself "Windows 10" here.
    if build >= 22000 {
        product = product.replace("Windows 10", "Windows 11");
    }
    let os = match reg(NT, "DisplayVersion") {
        Some(v) => format!("{product} {v} (build {build})"),
        None => format!("{product} (build {build})"),
    };

    let cpu = reg(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString");

    // display adapters (setup class "Display"), without virtual and remote-desktop ones
    let mut gpus = Vec::new();
    for i in 0..32 {
        let key = format!(r"SYSTEM\CurrentControlSet\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}\{i:04}");
        if let Some(name) = reg(&key, "DriverDesc") {
            let lower = name.to_lowercase();
            if !["virtual", "remote", "basic display", "basic render", "indirect"].iter().any(|w| lower.contains(w)) && !gpus.contains(&name) {
                gpus.push(name);
            }
        }
    }

    let mut kb = 0u64;
    let ram_mb = unsafe { windows::Win32::System::SystemInformation::GetPhysicallyInstalledSystemMemory(&mut kb) }.ok().map(|_| kb / 1024);

    SystemInfo { os: Some(os), cpu, threads: threads(), gpus, ram_mb }
}

#[cfg(target_os = "linux")]
fn read() -> SystemInfo {
    let file = |p: &str| std::fs::read_to_string(p).ok();
    let os_name = file("/etc/os-release").and_then(|s| {
        s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string())
    });
    let kernel = file("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string());
    let os = match (os_name, kernel) {
        (Some(n), Some(k)) => Some(format!("{n} (kernel {k})")),
        (n, k) => n.or(k.map(|k| format!("Linux {k}"))),
    };
    let cpu = file("/proc/cpuinfo").and_then(|s| s.lines().find_map(|l| l.strip_prefix("model name")).map(|v| v.trim_start_matches([' ', '\t', ':']).trim().to_string()));
    let ram_mb = file("/proc/meminfo")
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("MemTotal:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok()))
        .map(|kb| kb / 1024);
    SystemInfo { os, cpu, threads: threads(), gpus: linux_gpus(), ram_mb }
}

/// Graphics cards: the PCI devices behind /sys/class/drm/card*, named by `lspci` if available.
#[cfg(target_os = "linux")]
fn linux_gpus() -> Vec<String> {
    let mut slots: Vec<String> = std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().strip_prefix("card").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())))
        .filter_map(|e| std::fs::canonicalize(e.path().join("device")).ok())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|slot| slot.contains(':'))
        .collect();
    slots.sort();
    slots.dedup();
    let mut out = Vec::new();
    for slot in slots {
        let name = std::process::Command::new("lspci")
            .args(["-mm", "-s", &slot])
            .output()
            .ok()
            .and_then(|o| {
                // "01:00.0" "VGA compatible controller" "NVIDIA Corporation" "GA104 [GeForce RTX 3070]" ...
                let line = String::from_utf8_lossy(&o.stdout).to_string();
                let fields: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
                (fields.len() >= 3).then(|| format!("{} {}", fields[1], fields[2]))
            })
            .or_else(|| {
                let vendor = std::fs::read_to_string(format!("/sys/bus/pci/devices/{slot}/vendor")).ok()?;
                Some(match vendor.trim() {
                    "0x10de" => "NVIDIA".to_string(),
                    "0x1002" => "AMD".to_string(),
                    "0x8086" => "Intel".to_string(),
                    other => format!("PCI vendor {other}"),
                })
            });
        if let Some(n) = name {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read() -> SystemInfo {
    SystemInfo { os: Some(std::env::consts::OS.to_string()), cpu: None, threads: threads(), gpus: Vec::new(), ram_mb: None }
}
