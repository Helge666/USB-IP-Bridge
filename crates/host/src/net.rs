//! TCP helpers shared by export and import.

use anyhow::{anyhow, Context, Result};
use socket2::{SockRef, TcpKeepalive};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Detect a dead peer within ~10 s instead of the OS default of hours, so a device
/// is released (server) or reattached (client) quickly after a network break.
pub fn configure_keepalive(s: &TcpStream) -> Result<()> {
    let ka = TcpKeepalive::new().with_time(Duration::from_secs(5)).with_interval(Duration::from_secs(2));
    #[cfg(not(windows))]
    let ka = ka.with_retries(3);
    let sock = SockRef::from(s);
    sock.set_tcp_keepalive(&ka)?;
    #[cfg(target_os = "linux")]
    sock.set_tcp_user_timeout(Some(Duration::from_secs(10)))?;
    Ok(())
}

/// Short host name for an IP address via reverse DNS (cached), e.g. "studio-pc".
pub fn host_name(ip: std::net::IpAddr) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Cache = Mutex<HashMap<std::net::IpAddr, (Option<String>, std::time::Instant)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((name, t)) = cache.lock().unwrap().get(&ip) {
        if t.elapsed() < Duration::from_secs(600) {
            return name.clone();
        }
    }
    let name = dns_lookup::lookup_addr(&ip)
        .ok()
        .map(|n| n.split('.').next().unwrap_or(&n).to_string())
        .filter(|n| !n.is_empty() && n.parse::<std::net::IpAddr>().is_err());
    cache.lock().unwrap().insert(ip, (name.clone(), std::time::Instant::now()));
    name
}

/// Key/value fields carried in the free-text `path` of a USB/IP device record, e.g.
/// `/usb-ip-bridge/serial=ABCD1234/name=X18%2FXR18/host=STUDIO-PC`.
pub fn encode_path(fields: &[(&str, &str)]) -> String {
    let enc = |v: &str| v.replace('%', "%25").replace('/', "%2F").replace('=', "%3D");
    let mut s = String::from("/usb-ip-bridge");
    for (k, v) in fields {
        s.push_str(&format!("/{k}={}", enc(v)));
    }
    s.chars().take(255).collect()
}

pub fn path_field(path: &str, key: &str) -> Option<String> {
    let dec = |v: &str| v.replace("%2F", "/").replace("%3D", "=").replace("%25", "%");
    let prefix = format!("{key}=");
    path.split('/').find_map(|part| part.strip_prefix(prefix.as_str())).map(dec).filter(|v| !v.is_empty())
}

pub fn connect(host: &str, port: u16) -> Result<TcpStream> {
    let addr = (host, port).to_socket_addrs()?.next().ok_or_else(|| anyhow!("cannot resolve {host}"))?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(3)).with_context(|| format!("connect {host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_fields_roundtrip() {
        let p = encode_path(&[("serial", "ABCD1234"), ("name", "X18/XR18 a=b 100%"), ("host", "STUDIO-PC")]);
        assert_eq!(p, "/usb-ip-bridge/serial=ABCD1234/name=X18%2FXR18 a%3Db 100%25/host=STUDIO-PC");
        assert_eq!(path_field(&p, "name").as_deref(), Some("X18/XR18 a=b 100%"));
        assert_eq!(path_field(&p, "host").as_deref(), Some("STUDIO-PC"));
        assert_eq!(path_field(&p, "missing"), None);
        // older servers only sent the serial
        assert_eq!(path_field("/usb-ip-bridge/serial=ABC", "serial").as_deref(), Some("ABC"));
    }
}
