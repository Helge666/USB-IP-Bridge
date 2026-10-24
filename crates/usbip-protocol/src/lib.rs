//! USB/IP wire protocol, compatible with the Linux kernel implementation
//! (`Documentation/usb/usbip_protocol.rst`). All fields are big-endian.
//!
//! The protocol has two phases on one TCP connection:
//! 1. Operation phase: `OP_REQ_DEVLIST` / `OP_REQ_IMPORT` and their replies.
//! 2. URB phase (after a successful import): `CMD_SUBMIT` / `CMD_UNLINK` from the
//!    client, `RET_SUBMIT` / `RET_UNLINK` from the server.

use std::io::{self, Read, Write};

pub const USBIP_VERSION: u16 = 0x0111;
pub const DEFAULT_PORT: u16 = 3240;

pub const OP_REQ_DEVLIST: u16 = 0x8005;
pub const OP_REP_DEVLIST: u16 = 0x0005;
pub const OP_REQ_IMPORT: u16 = 0x8003;
pub const OP_REP_IMPORT: u16 = 0x0003;

pub const USBIP_CMD_SUBMIT: u32 = 1;
pub const USBIP_CMD_UNLINK: u32 = 2;
pub const USBIP_RET_SUBMIT: u32 = 3;
pub const USBIP_RET_UNLINK: u32 = 4;

/// OP_REP status codes (`enum usbip_op_status` in the Linux tools).
pub mod op_status {
    pub const OK: u32 = 0;
    pub const NA: u32 = 1;
    pub const DEV_BUSY: u32 = 2;
    pub const DEV_ERR: u32 = 3;
    pub const NODEV: u32 = 4;
    pub const ERROR: u32 = 5;
}

pub const USBIP_DIR_OUT: u32 = 0;
pub const USBIP_DIR_IN: u32 = 1;

/// Size of a URB-phase header (basic header + command-specific union).
pub const URB_HEADER_SIZE: usize = 48;
/// Size of one isochronous packet descriptor on the wire.
pub const ISO_DESCRIPTOR_SIZE: usize = 16;
/// Size of `struct usbip_usb_device` on the wire.
pub const USB_DEVICE_SIZE: usize = 312;

/// Linux errno values used in URB status fields (sent negated).
pub mod errno {
    pub const EPIPE: i32 = 32;
    pub const ENOENT: i32 = 2;
    pub const EXDEV: i32 = 18;
    pub const EPROTO: i32 = 71;
    pub const EOVERFLOW: i32 = 75;
    pub const ECONNRESET: i32 = 104;
    pub const ETIMEDOUT: i32 = 110;
    pub const ESHUTDOWN: i32 = 108;
    pub const EINVAL: i32 = 22;
    pub const ENODEV: i32 = 19;
}

/// `enum usb_device_speed` from the Linux kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Speed {
    Unknown = 0,
    Low = 1,
    Full = 2,
    High = 3,
    Wireless = 4,
    Super = 5,
    SuperPlus = 6,
}

fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn be_i32(b: &[u8]) -> i32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn fixed_str<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = s.as_bytes();
    let n = bytes.len().min(N - 1);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn str_from_fixed(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// Common header of the operation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpHeader {
    pub version: u16,
    pub code: u16,
    pub status: u32,
}

impl OpHeader {
    pub fn new(code: u16, status: u32) -> Self {
        Self { version: USBIP_VERSION, code, status }
    }

    pub fn read(r: &mut impl Read) -> io::Result<Self> {
        let mut b = [0u8; 8];
        r.read_exact(&mut b)?;
        Ok(Self { version: be_u16(&b[0..]), code: be_u16(&b[2..]), status: be_u32(&b[4..]) })
    }

    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&self.version.to_be_bytes());
        b[2..4].copy_from_slice(&self.code.to_be_bytes());
        b[4..8].copy_from_slice(&self.status.to_be_bytes());
        w.write_all(&b)
    }
}

/// One interface entry in `OP_REP_DEVLIST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterfaceInfo {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

/// `struct usbip_usb_device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub path: String,
    pub busid: String,
    pub busnum: u32,
    pub devnum: u32,
    pub speed: Speed,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub configuration_value: u8,
    pub num_configurations: u8,
    pub num_interfaces: u8,
}

impl UsbDevice {
    pub fn to_bytes(&self) -> [u8; USB_DEVICE_SIZE] {
        let mut b = [0u8; USB_DEVICE_SIZE];
        b[0..256].copy_from_slice(&fixed_str::<256>(&self.path));
        b[256..288].copy_from_slice(&fixed_str::<32>(&self.busid));
        b[288..292].copy_from_slice(&self.busnum.to_be_bytes());
        b[292..296].copy_from_slice(&self.devnum.to_be_bytes());
        b[296..300].copy_from_slice(&(self.speed as u32).to_be_bytes());
        b[300..302].copy_from_slice(&self.id_vendor.to_be_bytes());
        b[302..304].copy_from_slice(&self.id_product.to_be_bytes());
        b[304..306].copy_from_slice(&self.bcd_device.to_be_bytes());
        b[306] = self.device_class;
        b[307] = self.device_subclass;
        b[308] = self.device_protocol;
        b[309] = self.configuration_value;
        b[310] = self.num_configurations;
        b[311] = self.num_interfaces;
        b
    }

    pub fn from_bytes(b: &[u8; USB_DEVICE_SIZE]) -> Self {
        let speed = match be_u32(&b[296..]) {
            1 => Speed::Low,
            2 => Speed::Full,
            3 => Speed::High,
            4 => Speed::Wireless,
            5 => Speed::Super,
            6 => Speed::SuperPlus,
            _ => Speed::Unknown,
        };
        Self {
            path: str_from_fixed(&b[0..256]),
            busid: str_from_fixed(&b[256..288]),
            busnum: be_u32(&b[288..]),
            devnum: be_u32(&b[292..]),
            speed,
            id_vendor: be_u16(&b[300..]),
            id_product: be_u16(&b[302..]),
            bcd_device: be_u16(&b[304..]),
            device_class: b[306],
            device_subclass: b[307],
            device_protocol: b[308],
            configuration_value: b[309],
            num_configurations: b[310],
            num_interfaces: b[311],
        }
    }
}

/// Writes a complete `OP_REP_DEVLIST` reply.
pub fn write_devlist_reply(w: &mut impl Write, devices: &[(UsbDevice, Vec<InterfaceInfo>)]) -> io::Result<()> {
    let mut buf = Vec::new();
    OpHeader::new(OP_REP_DEVLIST, 0).write(&mut buf)?;
    buf.extend_from_slice(&(devices.len() as u32).to_be_bytes());
    for (dev, ifaces) in devices {
        buf.extend_from_slice(&dev.to_bytes());
        for i in ifaces {
            buf.extend_from_slice(&[i.class, i.subclass, i.protocol, 0]);
        }
    }
    w.write_all(&buf)
}

/// Reads the bus id that follows an `OP_REQ_IMPORT` header.
pub fn read_import_busid(r: &mut impl Read) -> io::Result<String> {
    let mut b = [0u8; 32];
    r.read_exact(&mut b)?;
    Ok(str_from_fixed(&b))
}

/// Writes a failed `OP_REP_IMPORT` with a specific status (see [`op_status`]).
pub fn write_import_error(w: &mut impl Write, status: u32) -> io::Result<()> {
    let mut buf = Vec::new();
    OpHeader::new(OP_REP_IMPORT, status).write(&mut buf)?;
    w.write_all(&buf)
}

/// Writes an `OP_REP_IMPORT` reply. `device` is `None` for a failed import.
pub fn write_import_reply(w: &mut impl Write, device: Option<&UsbDevice>) -> io::Result<()> {
    let mut buf = Vec::new();
    match device {
        Some(dev) => {
            OpHeader::new(OP_REP_IMPORT, 0).write(&mut buf)?;
            buf.extend_from_slice(&dev.to_bytes());
        }
        None => OpHeader::new(OP_REP_IMPORT, 1).write(&mut buf)?,
    }
    w.write_all(&buf)
}

/// `struct usbip_header_basic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BasicHeader {
    pub command: u32,
    pub seqnum: u32,
    pub devid: u32,
    pub direction: u32,
    pub ep: u32,
}

/// `struct usbip_header_cmd_submit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CmdSubmit {
    pub transfer_flags: u32,
    pub transfer_buffer_length: i32,
    pub start_frame: i32,
    pub number_of_packets: i32,
    pub interval: i32,
    pub setup: [u8; 8],
}

impl CmdSubmit {
    /// Linux sends 0 or -1 (0xffffffff) for non-isochronous transfers.
    pub fn iso_packet_count(&self) -> usize {
        if self.number_of_packets > 0 {
            self.number_of_packets as usize
        } else {
            0
        }
    }
}

/// `struct usbip_header_ret_submit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetSubmit {
    pub status: i32,
    pub actual_length: i32,
    pub start_frame: i32,
    pub number_of_packets: i32,
    pub error_count: i32,
}

/// A URB-phase message sent by the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientUrb {
    Submit(BasicHeader, CmdSubmit),
    Unlink(BasicHeader, u32),
}

/// Reads the 48-byte header of the next client URB. For `CMD_SUBMIT` the caller must
/// then read the OUT payload and the iso descriptors (see [`read_iso_descriptors`]).
pub fn read_client_urb(r: &mut impl Read) -> io::Result<ClientUrb> {
    let mut b = [0u8; URB_HEADER_SIZE];
    r.read_exact(&mut b)?;
    let basic = BasicHeader {
        command: be_u32(&b[0..]),
        seqnum: be_u32(&b[4..]),
        devid: be_u32(&b[8..]),
        direction: be_u32(&b[12..]),
        ep: be_u32(&b[16..]),
    };
    match basic.command {
        USBIP_CMD_SUBMIT => {
            let mut setup = [0u8; 8];
            setup.copy_from_slice(&b[40..48]);
            Ok(ClientUrb::Submit(
                basic,
                CmdSubmit {
                    transfer_flags: be_u32(&b[20..]),
                    transfer_buffer_length: be_i32(&b[24..]),
                    start_frame: be_i32(&b[28..]),
                    number_of_packets: be_i32(&b[32..]),
                    interval: be_i32(&b[36..]),
                    setup,
                },
            ))
        }
        USBIP_CMD_UNLINK => Ok(ClientUrb::Unlink(basic, be_u32(&b[20..]))),
        other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown USB/IP command {other:#x}"))),
    }
}

/// Serializes a client `CMD_SUBMIT` header (used by tests and by a future client side).
pub fn encode_cmd_submit(basic: &BasicHeader, cmd: &CmdSubmit) -> [u8; URB_HEADER_SIZE] {
    let mut b = [0u8; URB_HEADER_SIZE];
    encode_basic(&mut b, basic);
    b[20..24].copy_from_slice(&cmd.transfer_flags.to_be_bytes());
    b[24..28].copy_from_slice(&cmd.transfer_buffer_length.to_be_bytes());
    b[28..32].copy_from_slice(&cmd.start_frame.to_be_bytes());
    b[32..36].copy_from_slice(&cmd.number_of_packets.to_be_bytes());
    b[36..40].copy_from_slice(&cmd.interval.to_be_bytes());
    b[40..48].copy_from_slice(&cmd.setup);
    b
}

fn encode_basic(b: &mut [u8; URB_HEADER_SIZE], basic: &BasicHeader) {
    b[0..4].copy_from_slice(&basic.command.to_be_bytes());
    b[4..8].copy_from_slice(&basic.seqnum.to_be_bytes());
    b[8..12].copy_from_slice(&basic.devid.to_be_bytes());
    b[12..16].copy_from_slice(&basic.direction.to_be_bytes());
    b[16..20].copy_from_slice(&basic.ep.to_be_bytes());
}

/// Serializes a `RET_SUBMIT` header. Like the Linux server, devid/direction/ep are 0.
pub fn encode_ret_submit(seqnum: u32, ret: &RetSubmit) -> [u8; URB_HEADER_SIZE] {
    let mut b = [0u8; URB_HEADER_SIZE];
    encode_basic(&mut b, &BasicHeader { command: USBIP_RET_SUBMIT, seqnum, ..Default::default() });
    b[20..24].copy_from_slice(&ret.status.to_be_bytes());
    b[24..28].copy_from_slice(&ret.actual_length.to_be_bytes());
    b[28..32].copy_from_slice(&ret.start_frame.to_be_bytes());
    b[32..36].copy_from_slice(&ret.number_of_packets.to_be_bytes());
    b[36..40].copy_from_slice(&ret.error_count.to_be_bytes());
    b
}

/// Serializes a `RET_UNLINK` header.
pub fn encode_ret_unlink(seqnum: u32, status: i32) -> [u8; URB_HEADER_SIZE] {
    let mut b = [0u8; URB_HEADER_SIZE];
    encode_basic(&mut b, &BasicHeader { command: USBIP_RET_UNLINK, seqnum, ..Default::default() });
    b[20..24].copy_from_slice(&status.to_be_bytes());
    b
}

/// `struct usbip_iso_packet_descriptor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IsoPacket {
    pub offset: u32,
    pub length: u32,
    pub actual_length: u32,
    pub status: i32,
}

pub fn read_iso_descriptors(r: &mut impl Read, count: usize) -> io::Result<Vec<IsoPacket>> {
    let mut raw = vec![0u8; count * ISO_DESCRIPTOR_SIZE];
    r.read_exact(&mut raw)?;
    Ok(raw
        .chunks_exact(ISO_DESCRIPTOR_SIZE)
        .map(|c| IsoPacket {
            offset: be_u32(&c[0..]),
            length: be_u32(&c[4..]),
            actual_length: be_u32(&c[8..]),
            status: be_i32(&c[12..]),
        })
        .collect())
}

pub fn encode_iso_descriptors(packets: &[IsoPacket], out: &mut Vec<u8>) {
    for p in packets {
        out.extend_from_slice(&p.offset.to_be_bytes());
        out.extend_from_slice(&p.length.to_be_bytes());
        out.extend_from_slice(&p.actual_length.to_be_bytes());
        out.extend_from_slice(&p.status.to_be_bytes());
    }
}

/// Builds a complete `RET_SUBMIT` message for an isochronous IN transfer.
///
/// `buffer` holds the packets at their original offsets (with gaps when a packet is
/// short). USB/IP sends the data compacted: only `actual_length` bytes per packet,
/// back to back, followed by the descriptors with the original offsets.
pub fn build_iso_in_reply(seqnum: u32, start_frame: i32, buffer: &[u8], packets: &[IsoPacket]) -> Vec<u8> {
    let actual: u32 = packets.iter().map(|p| p.actual_length).sum();
    let errors = packets.iter().filter(|p| p.status != 0).count() as i32;
    let ret = RetSubmit {
        status: 0,
        actual_length: actual as i32,
        start_frame,
        number_of_packets: packets.len() as i32,
        error_count: errors,
    };
    let mut out = Vec::with_capacity(URB_HEADER_SIZE + actual as usize + packets.len() * ISO_DESCRIPTOR_SIZE);
    out.extend_from_slice(&encode_ret_submit(seqnum, &ret));
    for p in packets {
        let start = p.offset as usize;
        out.extend_from_slice(&buffer[start..start + p.actual_length as usize]);
    }
    encode_iso_descriptors(packets, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_device() -> UsbDevice {
        UsbDevice {
            path: "/sys/devices/usb-ip-bridge/8-1".into(),
            busid: "8-1".into(),
            busnum: 8,
            devnum: 1,
            speed: Speed::High,
            id_vendor: 0x1397,
            id_product: 0x00d4,
            bcd_device: 0x0101,
            device_class: 0xef,
            device_subclass: 0x02,
            device_protocol: 0x01,
            configuration_value: 1,
            num_configurations: 1,
            num_interfaces: 6,
        }
    }

    #[test]
    fn device_roundtrip_and_layout() {
        let d = sample_device();
        let b = d.to_bytes();
        assert_eq!(b.len(), 312);
        assert_eq!(&b[256..259], b"8-1");
        assert_eq!(&b[300..302], &[0x13, 0x97]);
        assert_eq!(b[311], 6);
        assert_eq!(UsbDevice::from_bytes(&b), d);
    }

    #[test]
    fn devlist_reply_layout() {
        let mut out = Vec::new();
        let ifaces = vec![InterfaceInfo { class: 1, subclass: 1, protocol: 0x20 }];
        write_devlist_reply(&mut out, &[(sample_device(), ifaces)]).unwrap();
        // header 8 + count 4 + device 312 + 1 interface * 4
        assert_eq!(out.len(), 8 + 4 + 312 + 4);
        assert_eq!(&out[0..4], &[0x01, 0x11, 0x00, 0x05]);
        assert_eq!(&out[8..12], &[0, 0, 0, 1]);
        assert_eq!(&out[324..328], &[1, 1, 0x20, 0]);
    }

    #[test]
    fn op_header_roundtrip() {
        let h = OpHeader::new(OP_REQ_IMPORT, 0);
        let mut v = Vec::new();
        h.write(&mut v).unwrap();
        assert_eq!(v, [0x01, 0x11, 0x80, 0x03, 0, 0, 0, 0]);
        assert_eq!(OpHeader::read(&mut &v[..]).unwrap(), h);
    }

    #[test]
    fn submit_roundtrip() {
        let basic = BasicHeader { command: USBIP_CMD_SUBMIT, seqnum: 42, devid: 0x00080001, direction: USBIP_DIR_IN, ep: 2 };
        let cmd = CmdSubmit {
            transfer_flags: 0x200,
            transfer_buffer_length: 4608,
            start_frame: 0,
            number_of_packets: 8,
            interval: 1,
            setup: [0; 8],
        };
        let bytes = encode_cmd_submit(&basic, &cmd);
        match read_client_urb(&mut &bytes[..]).unwrap() {
            ClientUrb::Submit(b, c) => {
                assert_eq!(b, basic);
                assert_eq!(c, cmd);
                assert_eq!(c.iso_packet_count(), 8);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn non_iso_packet_count_is_zero_for_minus_one() {
        let cmd = CmdSubmit { number_of_packets: -1, ..Default::default() };
        assert_eq!(cmd.iso_packet_count(), 0);
    }

    #[test]
    fn unlink_parse() {
        let mut b = [0u8; URB_HEADER_SIZE];
        b[0..4].copy_from_slice(&USBIP_CMD_UNLINK.to_be_bytes());
        b[4..8].copy_from_slice(&7u32.to_be_bytes());
        b[20..24].copy_from_slice(&5u32.to_be_bytes());
        assert_eq!(
            read_client_urb(&mut &b[..]).unwrap(),
            ClientUrb::Unlink(BasicHeader { command: USBIP_CMD_UNLINK, seqnum: 7, ..Default::default() }, 5)
        );
    }

    #[test]
    fn iso_in_reply_is_compacted() {
        // Two packets of max 4 bytes; the first returned 2 bytes, the second 4 bytes.
        let buffer = [1, 2, 0xee, 0xee, 3, 4, 5, 6];
        let packets = [
            IsoPacket { offset: 0, length: 4, actual_length: 2, status: 0 },
            IsoPacket { offset: 4, length: 4, actual_length: 4, status: 0 },
        ];
        let msg = build_iso_in_reply(9, 0, &buffer, &packets);
        assert_eq!(msg.len(), 48 + 6 + 2 * 16);
        assert_eq!(&msg[24..28], &6i32.to_be_bytes()); // actual_length
        assert_eq!(&msg[32..36], &2i32.to_be_bytes()); // number_of_packets
        assert_eq!(&msg[48..54], &[1, 2, 3, 4, 5, 6]);
        let descs = read_iso_descriptors(&mut &msg[54..], 2).unwrap();
        assert_eq!(descs, packets);
    }

    #[test]
    fn ret_unlink_layout() {
        let b = encode_ret_unlink(11, -errno::ECONNRESET);
        assert_eq!(&b[0..4], &USBIP_RET_UNLINK.to_be_bytes());
        assert_eq!(&b[4..8], &11u32.to_be_bytes());
        assert_eq!(&b[20..24], &(-104i32).to_be_bytes());
    }
}
