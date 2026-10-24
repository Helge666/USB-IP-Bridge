//! Encrypted, authenticated control connections and pairing.
//!
//! Every installation has its own X25519 key pair. Control connections use the Noise
//! protocol (pattern XX): both sides learn each other's public key and everything after the
//! handshake is encrypted. Two computers trust each other once they are *paired*: each keeps
//! the other's public key in its list of paired computers.
//!
//! Pairing: one computer shows a short code, the user types it on the other. Both run SPAKE2
//! with that code, bound to the Noise session (its handshake hash), and prove to each other
//! that they derived the same key. Someone in the middle gets exactly one guess per attempt.

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;

const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"usb-ip-bridge control v2";
const MAX_FRAME: usize = 65535;
const TAG_LEN: usize = 16;

/// This installation's key pair.
pub struct Identity {
    private: Vec<u8>,
    pub public: Vec<u8>,
}

impl Identity {
    /// Reads the key pair from `path`, or creates it there (readable by administrators only).
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => bail!("cannot read this computer's key {}: {e}", path.display()),
            Ok(text) => {
                let mut lines = text.lines();
                if let (Some(private), Some(public)) = (lines.next(), lines.next()) {
                    let (private, public) = (from_hex(private)?, from_hex(public)?);
                    if private.len() == 32 && public.len() == 32 {
                        return Ok(Self { private, public });
                    }
                }
                bail!("{} is damaged (delete it to create a new key; paired computers must then be paired again)", path.display());
            }
        }
        let params = NOISE.parse().map_err(|e| anyhow!("{e}"))?;
        let keys = snow::Builder::new(params).generate_keypair().map_err(|e| anyhow!("create key: {e}"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(path, format!("{}\n{}\n", to_hex(&keys.private), to_hex(&keys.public)).as_bytes()).with_context(|| format!("save key to {}", path.display()))?;
        log::info!("created this computer's key ({})", fingerprint(&keys.public));
        Ok(Self {
            private: keys.private,
            public: keys.public,
        })
    }

    fn builder(&self) -> Result<snow::Builder<'_>> {
        let params = NOISE.parse().map_err(|e| anyhow!("{e}"))?;
        snow::Builder::new(params)
            .prologue(PROLOGUE)
            .map_err(|e| anyhow!("{e}"))?
            .local_private_key(&self.private)
            .map_err(|e| anyhow!("{e}"))
    }
}

#[cfg(unix)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(data)
}

#[cfg(windows)]
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    std::fs::write(path, data)?;
    if cfg!(test) {
        return Ok(());
    }
    // SYSTEM and Administrators only (SIDs, independent of the Windows language).
    use std::os::windows::process::CommandExt;
    let status = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", "*S-1-5-18:F", "*S-1-5-32-544:F"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        log::warn!("cannot restrict access to {}", path.display());
    }
    Ok(())
}

/// Short, readable form of a public key, e.g. "3fa2 91c0 77de 0b14".
pub fn fingerprint(public: &[u8]) -> String {
    let h = Sha256::digest(public);
    h[..8].chunks(2).map(to_hex).collect::<Vec<_>>().join(" ")
}

pub fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        bail!("odd length");
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("not hex")).collect()
}

/// A control connection after the handshake: an encrypted byte stream.
pub struct SecureStream {
    stream: TcpStream,
    noise: snow::TransportState,
    remote_key: Vec<u8>,
    hash: Vec<u8>,
    buf: Vec<u8>,
    pos: usize,
}

fn send_frame(mut s: &TcpStream, data: &[u8]) -> io::Result<()> {
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
    s.write_all(&out)
}

/// Reads one frame; None at a clean end of the connection.
fn recv_frame(mut s: &TcpStream) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 2];
    if s.read(&mut len[..1])? == 0 {
        return Ok(None);
    }
    s.read_exact(&mut len[1..])?;
    let mut data = vec![0u8; u16::from_be_bytes(len) as usize];
    s.read_exact(&mut data)?;
    Ok(Some(data))
}

fn noise_err(e: snow::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("secure channel: {e}"))
}

impl SecureStream {
    /// Handshake as the connecting side.
    pub fn connect(stream: TcpStream, id: &Identity) -> Result<Self> {
        let mut hs = id.builder()?.build_initiator().map_err(|e| anyhow!("{e}"))?;
        let mut msg = vec![0u8; MAX_FRAME];
        let n = hs.write_message(&[], &mut msg).map_err(noise_err)?;
        send_frame(&stream, &msg[..n])?;
        let reply = recv_frame(&stream)?.context("connection closed during the handshake (an older USB-IP-Bridge version?)")?;
        hs.read_message(&reply, &mut msg).map_err(noise_err)?;
        let n = hs.write_message(&[], &mut msg).map_err(noise_err)?;
        send_frame(&stream, &msg[..n])?;
        Self::finish(stream, hs)
    }

    /// Handshake as the accepting side.
    pub fn accept(stream: TcpStream, id: &Identity) -> Result<Self> {
        let mut hs = id.builder()?.build_responder().map_err(|e| anyhow!("{e}"))?;
        let mut msg = vec![0u8; MAX_FRAME];
        let first = recv_frame(&stream)?.context("connection closed")?;
        hs.read_message(&first, &mut msg).map_err(noise_err)?;
        let n = hs.write_message(&[], &mut msg).map_err(noise_err)?;
        send_frame(&stream, &msg[..n])?;
        let last = recv_frame(&stream)?.context("connection closed during the handshake")?;
        hs.read_message(&last, &mut msg).map_err(noise_err)?;
        Self::finish(stream, hs)
    }

    fn finish(stream: TcpStream, hs: snow::HandshakeState) -> Result<Self> {
        let hash = hs.get_handshake_hash().to_vec();
        let remote_key = hs.get_remote_static().context("no key from the other side")?.to_vec();
        let noise = hs.into_transport_mode().map_err(|e| anyhow!("{e}"))?;
        Ok(Self {
            stream,
            noise,
            remote_key,
            hash,
            buf: Vec::new(),
            pos: 0,
        })
    }

    /// The other side's public key.
    pub fn remote_key(&self) -> &[u8] {
        &self.remote_key
    }

    /// Identifies this connection (the same on both sides).
    pub fn handshake_hash(&self) -> &[u8] {
        &self.hash
    }

    pub fn tcp(&self) -> &TcpStream {
        &self.stream
    }
}

impl Read for SecureStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            let Some(frame) = recv_frame(&self.stream)? else { return Ok(0) };
            self.buf.resize(frame.len(), 0);
            let n = self.noise.read_message(&frame, &mut self.buf).map_err(noise_err)?;
            self.buf.truncate(n);
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Write for SecureStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = data.len().min(MAX_FRAME - TAG_LEN);
        let mut frame = vec![0u8; n + TAG_LEN];
        let len = self.noise.write_message(&data[..n], &mut frame).map_err(noise_err)?;
        send_frame(&self.stream, &frame[..len])?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&self.stream).flush()
    }
}

// ------------------------------------------------------------------ pairing

/// A new random pairing code (6 digits).
pub fn new_code() -> Result<String> {
    let mut b = [0u8; 4];
    getrandom::getrandom(&mut b).map_err(|e| anyhow!("random: {e}"))?;
    Ok(format!("{:06}", u32::from_be_bytes(b) % 1_000_000))
}

/// The digits of a code as typed ("123 456", "123-456" -> "123456").
pub fn normalize_code(code: &str) -> String {
    code.chars().filter(char::is_ascii_digit).collect()
}

pub type Pake = spake2::Spake2<spake2::Ed25519Group>;

/// Starts SPAKE2 with the code, bound to this connection; returns the message for the other side.
pub fn pake_start(code: &str, conn: &SecureStream) -> (Pake, Vec<u8>) {
    Pake::start_symmetric(&spake2::Password::new(normalize_code(code).as_bytes()), &spake2::Identity::new(conn.handshake_hash()))
}

/// The shared key (only equal on both sides if both used the same code on the same connection).
pub fn pake_finish(pake: Pake, their_message: &[u8]) -> Result<Vec<u8>> {
    pake.finish(their_message).map_err(|e| anyhow!("pairing message: {e:?}"))
}

/// Proof of the shared key; `role` keeps the two directions apart.
pub fn confirmation(key: &[u8], role: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"usb-ip-bridge pairing ");
    h.update(role.as_bytes());
    h.update(key);
    to_hex(&h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn identity(dir: &Path, name: &str) -> Identity {
        Identity::load_or_create(&dir.join(name)).unwrap()
    }

    #[test]
    fn channel_and_pairing() {
        let dir = std::env::temp_dir().join(format!("usbipb-secure-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = identity(&dir, "a");
        let b = identity(&dir, "b");
        let reloaded = Identity::load_or_create(&dir.join("a")).unwrap();
        assert_eq!(a.public, reloaded.public);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut conn = SecureStream::accept(s, &b).unwrap();
            let mut got = vec![0u8; 100_000];
            conn.read_exact(&mut got).unwrap();
            assert!(got.iter().enumerate().all(|(i, x)| *x == (i % 251) as u8));
            conn.write_all(b"ok").unwrap();
            let (pake, msg) = pake_start("123 456", &conn);
            (conn.remote_key().to_vec(), conn.handshake_hash().to_vec(), pake, msg, b.public)
        });
        let mut conn = SecureStream::connect(TcpStream::connect(addr).unwrap(), &a).unwrap();
        let big: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        conn.write_all(&big).unwrap();
        let mut ok = [0u8; 2];
        conn.read_exact(&mut ok).unwrap();
        assert_eq!(&ok, b"ok");
        let (seen_key, hash, b_pake, b_msg, b_public) = server.join().unwrap();
        assert_eq!(seen_key, a.public);
        assert_eq!(conn.remote_key(), b_public.as_slice());
        assert_eq!(conn.handshake_hash(), hash.as_slice());

        // same code: same key; wrong code: different keys
        let (a_pake, a_msg) = pake_start("123456", &conn);
        let ka = pake_finish(a_pake, &b_msg).unwrap();
        let kb = pake_finish(b_pake, &a_msg).unwrap();
        assert_eq!(confirmation(&ka, "x"), confirmation(&kb, "x"));
        let (wrong, wrong_msg) = pake_start("123457", &conn);
        let (right, right_msg) = pake_start("123456", &conn);
        assert_ne!(pake_finish(wrong, &right_msg).unwrap(), pake_finish(right, &wrong_msg).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codes() {
        let c = new_code().unwrap();
        assert_eq!(c.len(), 6);
        assert_eq!(normalize_code(" 123-456 "), "123456");
        assert_eq!(from_hex(&to_hex(&[0, 255, 16])).unwrap(), vec![0, 255, 16]);
    }
}
