//! USB/IP server on top of VBoxUSB (see vbox.rs).
//!
//! One worker thread per endpoint owns all state of that endpoint: it starts the
//! overlapped VBoxUSB requests, waits for completions in order and sends the replies.
//!
//! Isochronous endpoints are re-framed: Windows needs one USB frame (1 ms) per request
//! on high-speed endpoints, i.e. exactly 8 packets for a 125 µs interval, while Linux
//! sends URBs with any packet count (snd-usb-audio uses 7 for XR18 playback). The
//! worker queues the packets of all Linux URBs and submits them frame by frame; a
//! Linux URB is answered once all of its packets have been transferred.

use super::vbox::*;
use crate::descriptors::{ConfigInfo, EndpointInfo};
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usbip_protocol::*;
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};

macro_rules! trace {
    ($($t:tt)*) => { log::trace!($($t)*) };
}

/// One overlapped SEND_URB request.
struct Request {
    urb: Box<SupUrb>,
    buf: Vec<u8>,
    ov: Box<OVERLAPPED>,
    event: HANDLE,
    /// Synchronous submit error (no completion will follow).
    failed: bool,
}
unsafe impl Send for Request {}

impl Request {
    fn submit(dev: HANDLE, mut urb: SupUrb, mut buf: Vec<u8>) -> Self {
        urb.buf = if buf.is_empty() { 0 } else { buf.as_mut_ptr() as u64 };
        let mut urb = Box::new(urb);
        let event = unsafe { CreateEventW(None, true, false, None).expect("CreateEvent") };
        let mut ov = Box::new(OVERLAPPED { hEvent: event, ..Default::default() });
        let p = &mut *urb as *mut SupUrb as *mut core::ffi::c_void;
        let res = unsafe { DeviceIoControl(dev, SUPUSB_IOCTL_SEND_URB, Some(p), 104, Some(p), 104, None, Some(&mut *ov)) };
        let failed = matches!(&res, Err(e) if e.code() != ERROR_IO_PENDING.to_hresult());
        if let Err(e) = &res {
            if failed {
                trace!("SEND_URB failed: {e}");
            }
        }
        Self { urb, buf, ov, event, failed }
    }

    fn is_done(&self, timeout_ms: u32) -> bool {
        self.failed || unsafe { WaitForSingleObject(self.event, timeout_ms) } == WAIT_OBJECT_0
    }

    /// Waits for the request. Returns false if it did not complete normally
    /// (submit error or aborted); the URB's own status is in `self.urb`.
    fn io_ok(&mut self, dev: HANDLE) -> bool {
        if self.failed {
            return false;
        }
        let mut n = 0u32;
        unsafe { GetOverlappedResult(dev, &*self.ov, &mut n, true) }.is_ok()
    }

    /// Final status as negative errno (0 = ok).
    fn finish(&mut self, dev: HANDLE) -> i32 {
        if self.io_ok(dev) {
            error_to_errno(self.urb.error)
        } else if self.failed {
            -errno::EPROTO
        } else {
            -errno::ECONNRESET
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.event);
        }
    }
}

enum Msg {
    Submit { seq: u32, cmd: CmdSubmit, dir_in: bool, data: Vec<u8>, iso: Vec<IsoPacket> },
    Immediate { seq: u32, status: i32 },
    Unlink { seq: u32, target: u32 },
}

struct Shared {
    dev: Handle,
    writer: Mutex<TcpStream>,
    urbs: AtomicU64,
    iso_frames: AtomicU64,
    iso_bytes: AtomicU64,
    errors: AtomicU64,
    /// Isochronous stream ran dry: no frame in flight while packets were waiting.
    iso_starved: AtomicU64,
    /// Partial frames padded with empty packets.
    iso_padded: AtomicU64,
    /// Isochronous packets the host controller reported with an error.
    iso_packet_errors: AtomicU64,
    cap: Arc<Captured>,
}

impl Shared {
    fn send(&self, msg: &[u8]) {
        if let Err(e) = self.writer.lock().unwrap().write_all(msg) {
            trace!("send failed: {e}");
        }
    }
}

/// Endpoint parameters known at worker start.
#[derive(Clone, Copy)]
struct EpParams {
    address: u8,
    transfer_type: u8,
    /// Packets per request for isochronous endpoints.
    frame_packets: usize,
    /// High speed: exactly `frame_packets` per request; full speed: up to that many.
    exact_frames: bool,
}

// ---------------------------------------------------------------- non-isochronous

struct Pending {
    seq: u32,
    dir_in: bool,
    is_control: bool,
    req: Option<Request>,
    immediate_status: i32,
    unlink_seq: Option<u32>,
}

fn run_simple_endpoint(sh: Arc<Shared>, ep: EpParams, rx: mpsc::Receiver<Msg>) {
    let dev = sh.dev.0;
    let mut queue: VecDeque<Pending> = VecDeque::new();
    loop {
        // Receive (blocking only when nothing is in flight).
        let msg = if queue.is_empty() {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = sh.cap.abort_endpoint(ep.address);
                    return;
                }
            }
        };
        match msg {
            Some(Msg::Submit { seq, cmd, dir_in, data, .. }) => {
                let is_control = ep.address & 0x0f == 0;
                let len = cmd.transfer_buffer_length.max(0) as usize;
                let (urb, buf) = if is_control {
                    let mut buf = vec![0u8; 8 + len];
                    buf[..8].copy_from_slice(&cmd.setup);
                    if !dir_in {
                        buf[8..8 + data.len().min(len)].copy_from_slice(&data[..data.len().min(len)]);
                    }
                    let urb = SupUrb { ty: TRANSFER_MSG, ep: 0, dir: if dir_in { DIR_IN } else { DIR_OUT }, flags: FLAG_SHORT_OK, len: buf.len() as u64, ..Default::default() };
                    (urb, buf)
                } else {
                    let buf = if dir_in { vec![0u8; len] } else { data };
                    let ty = if ep.transfer_type == 3 { TRANSFER_INTR } else { TRANSFER_BULK };
                    let flags = if dir_in && cmd.transfer_flags & 1 == 0 { FLAG_SHORT_OK } else { 0 };
                    let urb = SupUrb { ty, ep: (ep.address & 0x0f) as u32, dir: if dir_in { DIR_IN } else { DIR_OUT }, flags, len: buf.len() as u64, ..Default::default() };
                    (urb, buf)
                };
                let req = Request::submit(dev, urb, buf);
                queue.push_back(Pending { seq, dir_in, is_control, req: Some(req), immediate_status: 0, unlink_seq: None });
            }
            Some(Msg::Immediate { seq, status }) => {
                queue.push_back(Pending { seq, dir_in: false, is_control: true, req: None, immediate_status: status, unlink_seq: None });
            }
            Some(Msg::Unlink { seq, target }) => match queue.iter_mut().find(|p| p.seq == target) {
                Some(p) => {
                    p.unlink_seq = Some(seq);
                    // VBoxUSB cannot cancel a single request; abort the pipe (all pending
                    // requests on it complete, Linux unlinks the others right after).
                    let _ = sh.cap.abort_endpoint(ep.address);
                }
                None => sh.send(&encode_ret_unlink(seq, 0)),
            },
            None => {}
        }

        // Complete finished requests in order (wait up to 1 ms for the oldest one).
        while let Some(front) = queue.front() {
            let done = match &front.req {
                None => true,
                Some(r) => r.is_done(1),
            };
            if !done {
                break;
            }
            let mut p = queue.pop_front().unwrap();
            let (status, data, actual) = match p.req.as_mut() {
                None => (p.immediate_status, Vec::new(), 0usize),
                Some(r) => {
                    let status = r.finish(dev);
                    let header = if p.is_control { 8 } else { 0 };
                    let n = (r.urb.len as usize).saturating_sub(header);
                    if status != 0 {
                        sh.errors.fetch_add(1, Ordering::Relaxed);
                        (status, Vec::new(), 0)
                    } else if p.dir_in {
                        let d = r.buf[header..(header + n).min(r.buf.len())].to_vec();
                        let l = d.len();
                        (0, d, l)
                    } else {
                        (0, Vec::new(), n)
                    }
                }
            };
            if let Some(unlink_seq) = p.unlink_seq {
                sh.send(&encode_ret_unlink(unlink_seq, -errno::ECONNRESET));
            } else {
                let ret = RetSubmit { status, actual_length: actual as i32, ..Default::default() };
                let mut m = encode_ret_submit(p.seq, &ret).to_vec();
                m.extend_from_slice(&data);
                sh.send(&m);
            }
            sh.urbs.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------- isochronous

struct IsoUrb {
    seq: u32,
    dir_in: bool,
    start_frame: i32,
    packets: Vec<IsoPacket>,
    /// OUT: payload from Linux. IN: receive buffer at Linux packet offsets.
    data: Vec<u8>,
    done: usize,
    unlink_seq: Option<u32>,
}

struct Frame {
    req: Request,
    /// (Linux URB seq, packet index, offset in frame buffer) per packet; None = padding.
    slots: Vec<Option<(u32, usize)>>,
    offsets: Vec<usize>,
}

fn run_iso_endpoint(sh: Arc<Shared>, ep: EpParams, rx: mpsc::Receiver<Msg>) {
    let dev = sh.dev.0;
    let mut urbs: VecDeque<IsoUrb> = VecDeque::new();
    let mut fifo: VecDeque<(u32, usize)> = VecDeque::new();
    let mut frames: VecDeque<Frame> = VecDeque::new();
    let mut last_submit = Instant::now();
    // Becomes true with the first frame; reset when the endpoint goes idle.
    let mut started = false;
    const MAX_FRAMES_IN_FLIGHT: usize = 24;
    let dir_in = ep.address & 0x80 != 0;

    loop {
        let busy = !frames.is_empty() || !fifo.is_empty();
        if !busy && urbs.is_empty() {
            started = false;
        }
        let msg = if !busy {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(Duration::from_micros(200)) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = sh.cap.abort_endpoint(ep.address);
                    return;
                }
            }
        };

        match msg {
            Some(Msg::Submit { seq, cmd, dir_in, data, iso }) => {
                let len = cmd.transfer_buffer_length.max(0) as usize;
                let data = if dir_in { vec![0u8; len] } else { data };
                for i in 0..iso.len() {
                    fifo.push_back((seq, i));
                }
                urbs.push_back(IsoUrb { seq, dir_in, start_frame: cmd.start_frame, packets: iso, data, done: 0, unlink_seq: None });
                last_submit = Instant::now();
            }
            Some(Msg::Unlink { seq, target }) => {
                if let Some(pos) = urbs.iter().position(|u| u.seq == target) {
                    let in_flight = frames.iter().any(|f| f.slots.iter().flatten().any(|(s, _)| *s == target));
                    if in_flight {
                        // Abort the pipe so the frames on the wire finish now; packets
                        // still queued here count as done without being sent.
                        let u = &mut urbs[pos];
                        u.unlink_seq = Some(seq);
                        let queued = fifo.iter().filter(|(s, _)| *s == target).count();
                        fifo.retain(|(s, _)| *s != target);
                        u.done += queued;
                        let _ = sh.cap.abort_endpoint(ep.address);
                    } else {
                        // Not yet on the wire: drop its packets and answer right away.
                        fifo.retain(|(s, _)| *s != target);
                        urbs.remove(pos);
                        sh.send(&encode_ret_unlink(seq, -errno::ECONNRESET));
                    }
                } else {
                    sh.send(&encode_ret_unlink(seq, 0));
                }
            }
            Some(Msg::Immediate { .. }) => {}
            None => {}
        }

        // Submit complete frames; on full speed also partial ones. If the stream ran dry
        // with a partial frame waiting, pad it so the waiting URB can complete.
        loop {
            if frames.len() >= MAX_FRAMES_IN_FLIGHT || fifo.is_empty() {
                break;
            }
            let pad = frames.is_empty() && last_submit.elapsed() > Duration::from_millis(3);
            if frames.is_empty() && started {
                sh.iso_starved.fetch_add(1, Ordering::Relaxed);
            }
            let take = if fifo.len() >= ep.frame_packets {
                ep.frame_packets
            } else if !ep.exact_frames || pad {
                fifo.len()
            } else {
                break;
            };
            let count = if ep.exact_frames { ep.frame_packets } else { take };
            if take < count {
                sh.iso_padded.fetch_add(1, Ordering::Relaxed);
            }
            started = true;
            let mut slots = Vec::with_capacity(count);
            let mut offsets = Vec::with_capacity(count);
            let mut buf = Vec::new();
            let mut urb = SupUrb { ty: TRANSFER_ISOC, ep: (ep.address & 0x0f) as u32, dir: if dir_in { DIR_IN } else { DIR_OUT }, flags: FLAG_SHORT_OK, num_iso_pkts: count as u32, ..Default::default() };
            for i in 0..count {
                let slot = if i < take { fifo.pop_front() } else { None };
                let (len, payload): (usize, Option<&[u8]>) = match slot {
                    Some((s, idx)) => {
                        let u = urbs.iter().find(|u| u.seq == s).unwrap();
                        let p = u.packets[idx];
                        let off = p.offset as usize;
                        (p.length as usize, (!dir_in).then(|| &u.data[off..off + p.length as usize]))
                    }
                    None => (0, None),
                };
                offsets.push(buf.len());
                urb.iso_pkts[i] = IsoPkt { cb: len as u16, off: buf.len() as u16, stat: 0 };
                match payload {
                    Some(d) => buf.extend_from_slice(d),
                    None => buf.resize(buf.len() + len, 0),
                }
                slots.push(slot);
            }
            urb.len = buf.len() as u64;
            let req = Request::submit(dev, urb, buf);
            frames.push_back(Frame { req, slots, offsets });
        }

        // Retire completed frames in order.
        while let Some(f) = frames.front() {
            if !f.req.is_done(0) {
                break;
            }
            let mut f = frames.pop_front().unwrap();
            let io_ok = f.req.io_ok(dev);
            if !io_ok {
                sh.errors.fetch_add(1, Ordering::Relaxed);
            }
            sh.iso_frames.fetch_add(1, Ordering::Relaxed);
            for (i, slot) in f.slots.iter().enumerate() {
                let Some((s, idx)) = *slot else { continue };
                let Some(u) = urbs.iter_mut().find(|u| u.seq == s) else { continue };
                let pk = f.req.urb.iso_pkts[i];
                let (actual, pstat) = if io_ok { (pk.cb as usize, error_to_errno(pk.stat)) } else { (0, -errno::ECONNRESET) };
                if io_ok && pstat != 0 {
                    sh.iso_packet_errors.fetch_add(1, Ordering::Relaxed);
                }
                let lp = &mut u.packets[idx];
                let actual = actual.min(lp.length as usize);
                if u.dir_in && actual > 0 {
                    let src = f.offsets[i];
                    let dst = lp.offset as usize;
                    u.data[dst..dst + actual].copy_from_slice(&f.req.buf[src..src + actual]);
                }
                lp.actual_length = actual as u32;
                lp.status = pstat;
                u.done += 1;
                sh.iso_bytes.fetch_add(actual as u64, Ordering::Relaxed);
            }
        }

        // Answer Linux URBs whose packets are all done, in submission order.
        while let Some(u) = urbs.front() {
            if u.done < u.packets.len() {
                break;
            }
            let u = urbs.pop_front().unwrap();
            if let Some(unlink_seq) = u.unlink_seq {
                sh.send(&encode_ret_unlink(unlink_seq, -errno::ECONNRESET));
                continue;
            }
            let msg = if u.dir_in {
                build_iso_in_reply(u.seq, u.start_frame, &u.data, &u.packets)
            } else {
                let actual: u32 = u.packets.iter().map(|p| p.actual_length).sum();
                let errors = u.packets.iter().filter(|p| p.status != 0).count() as i32;
                let ret = RetSubmit { status: 0, actual_length: actual as i32, start_frame: u.start_frame, number_of_packets: u.packets.len() as i32, error_count: errors };
                let mut m = encode_ret_submit(u.seq, &ret).to_vec();
                encode_iso_descriptors(&u.packets, &mut m);
                m
            };
            sh.send(&msg);
            sh.urbs.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------- session

pub fn descriptors(cap: &Captured) -> Result<([u8; 18], ConfigInfo)> {
    let dd = cap.control_in([0x80, 6, 0, 1, 0, 0, 18, 0], 18)?;
    let mut dev_desc = [0u8; 18];
    dev_desc[..dd.len().min(18)].copy_from_slice(&dd[..dd.len().min(18)]);
    let head = cap.control_in([0x80, 6, 0, 2, 0, 0, 9, 0], 9)?;
    let total = u16::from_le_bytes([head[2], head[3]]);
    let t = total.to_le_bytes();
    let raw = cap.control_in([0x80, 6, 0, 2, 0, 0, t[0], t[1]], total as usize)?;
    Ok((dev_desc, ConfigInfo::parse(raw)?))
}

pub fn run_session(cap: Arc<Captured>, config: ConfigInfo, speed: Speed, stream: TcpStream, mut reader: BufReader<TcpStream>) -> Result<()> {
    let sh = Arc::new(Shared {
        dev: cap.file,
        writer: Mutex::new(stream),
        urbs: AtomicU64::new(0),
        iso_frames: AtomicU64::new(0),
        iso_bytes: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        iso_starved: AtomicU64::new(0),
        iso_padded: AtomicU64::new(0),
        iso_packet_errors: AtomicU64::new(0),
        cap: cap.clone(),
    });
    let alive = Arc::new(AtomicBool::new(true));
    {
        let (s, alive) = (sh.clone(), alive.clone());
        std::thread::spawn(move || {
            let mut last = (0, 0, 0);
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(2));
                let now = (s.urbs.load(Ordering::Relaxed), s.iso_frames.load(Ordering::Relaxed), s.iso_bytes.load(Ordering::Relaxed));
                if now != last {
                    log::debug!(
                        "stats/2s: {} URBs answered, {} iso frames ({}/s), {:.1} KB/s iso data, {} errors total, iso starved {} / padded {} / packet errors {} total",
                        now.0 - last.0,
                        now.1 - last.1,
                        (now.1 - last.1) / 2,
                        (now.2 - last.2) as f64 / 2048.0,
                        s.errors.load(Ordering::Relaxed),
                        s.iso_starved.load(Ordering::Relaxed),
                        s.iso_padded.load(Ordering::Relaxed),
                        s.iso_packet_errors.load(Ordering::Relaxed)
                    );
                }
                last = now;
            }
        });
    }

    let mut alt: HashMap<u8, u8> = HashMap::new();
    let mut workers: HashMap<u8, mpsc::Sender<Msg>> = HashMap::new();
    let mut ep_of_seq: HashMap<u32, u8> = HashMap::new();

    let endpoint_params = |addr: u8, alt: &HashMap<u8, u8>| -> EpParams {
        if addr & 0x0f == 0 {
            return EpParams { address: 0, transfer_type: 0, frame_packets: 0, exact_frames: false };
        }
        let e: Option<EndpointInfo> = config.endpoint(addr, |i| *alt.get(&i).unwrap_or(&0));
        let e = e.unwrap_or(EndpointInfo { address: addr, attributes: 2, max_packet: 512, interval: 0, interface: 0, alt: 0 });
        let (frame_packets, exact) = if e.transfer_type() == 1 && matches!(speed, Speed::High | Speed::Super) {
            let interval_uf = 1usize << (e.interval.clamp(1, 16) - 1);
            ((8 / interval_uf).max(1), true)
        } else {
            (MAX_ISO_PACKETS, false)
        };
        EpParams { address: addr, transfer_type: e.transfer_type(), frame_packets, exact_frames: exact }
    };

    let result = (|| -> Result<()> {
        loop {
            let urb = match read_client_urb(&mut reader) {
                Ok(u) => u,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e.into()),
            };
            match urb {
                ClientUrb::Submit(basic, cmd) => {
                    let dir_in = basic.direction == USBIP_DIR_IN;
                    let out_len = if dir_in { 0 } else { cmd.transfer_buffer_length.max(0) as usize };
                    let mut data = vec![0u8; out_len];
                    reader.read_exact(&mut data)?;
                    let iso = read_iso_descriptors(&mut reader, cmd.iso_packet_count())?;
                    let key = if basic.ep == 0 { 0 } else { (basic.ep as u8) | if dir_in { 0x80 } else { 0 } };
                    trace!("SUBMIT seq {} ep {key:#04x} len {} pkts {} setup {:02x?}", basic.seqnum, cmd.transfer_buffer_length, cmd.number_of_packets, cmd.setup);

                    // Standard requests that change the driver's pipe setup are executed
                    // here, before any later URB of another endpoint can be started.
                    let mut msg = Msg::Submit { seq: basic.seqnum, cmd, dir_in, data, iso };
                    if key == 0 {
                        let s = cmd.setup;
                        let value = u16::from_le_bytes([s[2], s[3]]);
                        let index = u16::from_le_bytes([s[4], s[5]]);
                        let status = match (s[0], s[1]) {
                            (0x00, 0x09) => Some(cap.set_config(value as u8).map(|_| 0).unwrap_or(-errno::EPIPE)),
                            (0x01, 0x0b) => {
                                let r = cap.select_interface(index as u8, value as u8);
                                log::debug!("SET_INTERFACE {} -> {}: {}", index, value, if r.is_ok() { "ok".into() } else { format!("{r:?}") });
                                if r.is_ok() {
                                    alt.insert(index as u8, value as u8);
                                    // Endpoint parameters may change with the alt setting.
                                    for e in config.endpoints.iter().filter(|e| e.interface == index as u8) {
                                        workers.remove(&e.address);
                                    }
                                }
                                Some(r.map(|_| 0).unwrap_or(-errno::EPIPE))
                            }
                            (0x02, 0x01) if value == 0 => Some(cap.clear_endpoint(index as u8).map(|_| 0).unwrap_or(-errno::EPIPE)),
                            _ => None,
                        };
                        if let Some(status) = status {
                            msg = Msg::Immediate { seq: basic.seqnum, status };
                        }
                    }
                    ep_of_seq.insert(basic.seqnum, key);
                    if ep_of_seq.len() > 100_000 {
                        ep_of_seq.clear();
                    }
                    let tx = workers.entry(key).or_insert_with(|| {
                        let p = endpoint_params(key, &alt);
                        let (tx, rx) = mpsc::channel::<Msg>();
                        let s = sh.clone();
                        if p.transfer_type == 1 {
                            log::debug!("iso endpoint {key:#04x}: {} packets per request ({})", p.frame_packets, if p.exact_frames { "1 frame" } else { "full speed" });
                        }
                        std::thread::Builder::new()
                            .name(format!("ep{key:02x}"))
                            .spawn(move || if p.transfer_type == 1 { run_iso_endpoint(s, p, rx) } else { run_simple_endpoint(s, p, rx) })
                            .expect("spawn endpoint worker");
                        tx
                    });
                    tx.send(msg).ok();
                }
                ClientUrb::Unlink(basic, target) => {
                    trace!("UNLINK seq {} -> {target}", basic.seqnum);
                    match ep_of_seq.get(&target).and_then(|k| workers.get(k)) {
                        Some(tx) => {
                            tx.send(Msg::Unlink { seq: basic.seqnum, target }).ok();
                        }
                        None => sh.send(&encode_ret_unlink(basic.seqnum, 0)),
                    }
                }
            }
        }
    })();

    for e in &config.endpoints {
        let _ = cap.abort_endpoint(e.address);
    }
    drop(workers);
    std::thread::sleep(Duration::from_millis(300));
    alive.store(false, Ordering::Relaxed);
    drop(sh);
    result
}
