//! PCAP (Packet Capture) file parser for extracting ASTERIX network datagrams.
//!
//! Provides zero-panic, bounds-checked decoding of standard libpcap capture files,
//! Ethernet frames, IPv4 packets, and UDP datagrams containing Eurocontrol ASTERIX
//! surveillance data blocks.
//!
//! # Specification Conformance
//! Follows standard `libpcap` file format specifications (magic `0xa1b2c3d4` / `0xd4c3b2a1`),
//! IEEE 802.3 Ethernet, IETF RFC 791 (IPv4), and IETF RFC 768 (UDP).

use core::fmt;

/// Errors that can occur during PCAP and network packet extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcapError {
    /// Input data is too short to contain a valid header or packet.
    UnexpectedEof {
        /// Expected minimum number of bytes.
        expected: usize,
        /// Actual bytes available.
        actual: usize,
    },
    /// Invalid PCAP magic number in global header.
    InvalidMagic(u32),
    /// Unsupported link layer type in PCAP global header (only Ethernet DLT_EN10MB = 1 supported).
    UnsupportedLinkType(u32),
    /// Invalid or truncated Ethernet frame.
    InvalidEthernetFrame(&'static str),
    /// Invalid or truncated IPv4 packet.
    InvalidIpPacket(&'static str),
    /// Invalid or truncated UDP datagram.
    InvalidUdpDatagram(&'static str),
    /// Invalid ASTERIX framing within UDP payload.
    InvalidAsterixFraming(&'static str),
}

impl fmt::Display for PcapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { expected, actual } => {
                write!(f, "Unexpected EOF: expected at least {expected} bytes, found {actual}")
            }
            Self::InvalidMagic(m) => write!(f, "Invalid PCAP magic number: 0x{m:08x}"),
            Self::UnsupportedLinkType(lt) => write!(f, "Unsupported link layer type: {lt}"),
            Self::InvalidEthernetFrame(msg) => write!(f, "Invalid Ethernet frame: {msg}"),
            Self::InvalidIpPacket(msg) => write!(f, "Invalid IPv4 packet: {msg}"),
            Self::InvalidUdpDatagram(msg) => write!(f, "Invalid UDP datagram: {msg}"),
            Self::InvalidAsterixFraming(msg) => write!(f, "Invalid ASTERIX framing: {msg}"),
        }
    }
}

impl std::error::Error for PcapError {}

/// Standard libpcap global file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcapHeader {
    /// Magic number indicating file format and endianness.
    pub magic: u32,
    /// Major version number (typically 2).
    pub version_major: u16,
    /// Minor version number (typically 4).
    pub version_minor: u16,
    /// GMT to local correction in seconds.
    pub thiszone: i32,
    /// Accuracy of timestamps.
    pub sigfigs: u32,
    /// Snapshot length (maximum capture length per packet).
    pub snaplen: u32,
    /// Link-layer header type (1 = Ethernet).
    pub network: u32,
    /// Whether multi-byte integers in packet records are little-endian.
    pub is_little_endian: bool,
}

/// Extracted raw packet record from a PCAP file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcapPacket<'a> {
    /// Timestamp seconds since UNIX epoch.
    pub timestamp_sec: u32,
    /// Timestamp microsecond or nanosecond fraction.
    pub timestamp_frac: u32,
    /// Captured packet data slice.
    pub data: &'a [u8],
}

/// Extracted ASTERIX data block containing Category, declared length, and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsterixBlock<'a> {
    /// ASTERIX Data Category (e.g. 34, 48, 1, 2).
    pub category: u8,
    /// Declared block length in bytes (including 3-byte header).
    pub length: u16,
    /// Complete raw ASTERIX block slice (starts with category, length, and records).
    pub raw_data: &'a [u8],
}

/// Bounds-checked helper to read a 16-bit unsigned integer with specified endianness.
fn read_u16(slice: &[u8], offset: usize, is_le: bool) -> Result<u16, PcapError> {
    let end = match offset.checked_add(2) {
        Some(e) => e,
        None => {
            return Err(PcapError::UnexpectedEof {
                expected: usize::MAX,
                actual: slice.len(),
            })
        }
    };
    let b = slice.get(offset..end).ok_or(PcapError::UnexpectedEof {
        expected: end,
        actual: slice.len(),
    })?;
    let b0 = *b.first().unwrap_or(&0) as u16;
    let b1 = *b.get(1).unwrap_or(&0) as u16;
    if is_le {
        Ok(b0 | (b1 << 8))
    } else {
        Ok((b0 << 8) | b1)
    }
}

/// Bounds-checked helper to read a 32-bit unsigned integer with specified endianness.
fn read_u32(slice: &[u8], offset: usize, is_le: bool) -> Result<u32, PcapError> {
    let end = match offset.checked_add(4) {
        Some(e) => e,
        None => {
            return Err(PcapError::UnexpectedEof {
                expected: usize::MAX,
                actual: slice.len(),
            })
        }
    };
    let b = slice.get(offset..end).ok_or(PcapError::UnexpectedEof {
        expected: end,
        actual: slice.len(),
    })?;
    let b0 = *b.first().unwrap_or(&0) as u32;
    let b1 = *b.get(1).unwrap_or(&0) as u32;
    let b2 = *b.get(2).unwrap_or(&0) as u32;
    let b3 = *b.get(3).unwrap_or(&0) as u32;
    if is_le {
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    } else {
        Ok((b0 << 24) | (b1 << 16) | (b2 << 8) | b3)
    }
}

/// Parses the 24-byte PCAP file global header.
///
/// # Errors
/// Returns [`PcapError`] if the header is truncated or has an invalid magic number.
pub fn parse_pcap_header(data: &[u8]) -> Result<(PcapHeader, usize), PcapError> {
    if data.len() < 24 {
        return Err(PcapError::UnexpectedEof {
            expected: 24,
            actual: data.len(),
        });
    }

    // Inspect first 4 bytes for magic
    let raw_magic = read_u32(data, 0, false)?;
    let (magic, is_le) = match raw_magic {
        0xa1b2c3d4 => (raw_magic, false),
        0xd4c3b2a1 => (0xa1b2c3d4, true),
        0xa1b23c4d => (raw_magic, false), // nanosecond pcap
        0x4d3cb2a1 => (0xa1b23c4d, true),
        other => return Err(PcapError::InvalidMagic(other)),
    };

    let version_major = read_u16(data, 4, is_le)?;
    let version_minor = read_u16(data, 6, is_le)?;
    let thiszone = read_u32(data, 8, is_le)? as i32;
    let sigfigs = read_u32(data, 12, is_le)?;
    let snaplen = read_u32(data, 16, is_le)?;
    let network = read_u32(data, 20, is_le)?;

    Ok((
        PcapHeader {
            magic,
            version_major,
            version_minor,
            thiszone,
            sigfigs,
            snaplen,
            network,
            is_little_endian: is_le,
        },
        24,
    ))
}

/// Parses all packet records from a PCAP byte buffer.
///
/// # Errors
/// Returns [`PcapError`] if any packet record header is corrupted or truncated.
pub fn parse_pcap_packets<'a>(
    data: &'a [u8],
    header: &PcapHeader,
) -> Result<Vec<PcapPacket<'a>>, PcapError> {
    let mut packets = Vec::new();
    let mut offset = 24;

    while offset < data.len() {
        let remaining = match data.len().checked_sub(offset) {
            Some(r) => r,
            None => break,
        };
        if remaining < 16 {
            break;
        }

        let ts_sec = read_u32(data, offset, header.is_little_endian)?;
        let ts_frac = read_u32(
            data,
            match offset.checked_add(4) {
                Some(o) => o,
                None => break,
            },
            header.is_little_endian,
        )?;
        let incl_len = read_u32(
            data,
            match offset.checked_add(8) {
                Some(o) => o,
                None => break,
            },
            header.is_little_endian,
        )? as usize;
        let _orig_len = read_u32(
            data,
            match offset.checked_add(12) {
                Some(o) => o,
                None => break,
            },
            header.is_little_endian,
        )?;

        let pkt_data_start = match offset.checked_add(16) {
            Some(s) => s,
            None => break,
        };
        let pkt_data_end = match pkt_data_start.checked_add(incl_len) {
            Some(e) => e,
            None => break,
        };

        let pkt_slice = data
            .get(pkt_data_start..pkt_data_end)
            .ok_or(PcapError::UnexpectedEof {
                expected: pkt_data_end,
                actual: data.len(),
            })?;

        packets.push(PcapPacket {
            timestamp_sec: ts_sec,
            timestamp_frac: ts_frac,
            data: pkt_slice,
        });

        offset = pkt_data_end;
    }

    Ok(packets)
}

/// Extracts UDP payload from a raw Ethernet frame.
///
/// Handles Ethernet framing (14 bytes), IPv4 headers with variable IHL,
/// and UDP headers (8 bytes), stripping off Ethernet padding.
///
/// # Errors
/// Returns [`PcapError`] if the frame is not valid IPv4/UDP.
pub fn extract_udp_payload(frame: &[u8]) -> Result<&[u8], PcapError> {
    if frame.len() < 14 {
        return Err(PcapError::InvalidEthernetFrame("Frame shorter than 14 bytes"));
    }

    // EtherType at bytes 12..14
    let ethertype = read_u16(frame, 12, false)?;
    if ethertype != 0x0800 {
        return Err(PcapError::InvalidEthernetFrame(
            "Non-IPv4 EtherType (expected 0x0800)",
        ));
    }

    let ip_data = frame
        .get(14..)
        .ok_or(PcapError::InvalidEthernetFrame("Missing IP payload"))?;
    if ip_data.len() < 20 {
        return Err(PcapError::InvalidIpPacket("IPv4 header shorter than 20 bytes"));
    }

    let first_byte = *ip_data.first().unwrap_or(&0);
    let ip_version = first_byte >> 4;
    let ihl = (first_byte & 0x0f) as usize;
    let ip_header_len = match ihl.checked_mul(4) {
        Some(l) => l,
        None => return Err(PcapError::InvalidIpPacket("Invalid IHL overflow")),
    };

    if ip_version != 4 {
        return Err(PcapError::InvalidIpPacket("Expected IPv4 packet"));
    }
    if ip_header_len < 20 || ip_data.len() < ip_header_len {
        return Err(PcapError::InvalidIpPacket("Truncated IPv4 options"));
    }

    // Check protocol at byte 9 (17 for UDP)
    let protocol = *ip_data.get(9).unwrap_or(&0);
    if protocol != 17 {
        return Err(PcapError::InvalidIpPacket("Expected UDP protocol (17)"));
    }

    let udp_data = ip_data
        .get(ip_header_len..)
        .ok_or(PcapError::InvalidIpPacket("Missing UDP payload"))?;
    if udp_data.len() < 8 {
        return Err(PcapError::InvalidUdpDatagram(
            "UDP header shorter than 8 bytes",
        ));
    }

    // UDP length at bytes 4..6 of UDP header
    let udp_len = read_u16(udp_data, 4, false)? as usize;
    if udp_len < 8 {
        return Err(PcapError::InvalidUdpDatagram("UDP length less than 8 bytes"));
    }

    let payload_len = match udp_len.checked_sub(8) {
        Some(l) => l,
        None => return Err(PcapError::InvalidUdpDatagram("UDP length arithmetic underflow")),
    };

    let payload_end = match 8usize.checked_add(payload_len) {
        Some(e) => e,
        None => return Err(PcapError::InvalidUdpDatagram("UDP payload range overflow")),
    };

    udp_data
        .get(8..payload_end)
        .ok_or(PcapError::InvalidUdpDatagram("Truncated UDP payload"))
}

/// Extracts all ASTERIX Data Blocks from a UDP datagram payload.
///
/// Each ASTERIX Data Block begins with:
/// - Category (1 byte, e.g. 34, 48, 1, 2)
/// - Length Indicator (2 bytes, big-endian length of block including 3-byte header).
///
/// # Errors
/// Returns [`PcapError`] if any block length is less than 3 or extends beyond available data.
pub fn extract_asterix_blocks<'a>(payload: &'a [u8]) -> Result<Vec<AsterixBlock<'a>>, PcapError> {
    let mut blocks = Vec::new();
    let mut offset = 0;

    while offset < payload.len() {
        let remaining = match payload.len().checked_sub(offset) {
            Some(r) => r,
            None => break,
        };
        if remaining < 3 {
            return Err(PcapError::InvalidAsterixFraming(
                "Incomplete ASTERIX header at end of payload",
            ));
        }

        let category = *payload.get(offset).unwrap_or(&0);
        let length_offset = match offset.checked_add(1) {
            Some(o) => o,
            None => break,
        };
        let length = read_u16(payload, length_offset, false)?;

        if length < 3 {
            return Err(PcapError::InvalidAsterixFraming(
                "ASTERIX block length cannot be less than 3 bytes",
            ));
        }

        let block_len = length as usize;
        let block_end = match offset.checked_add(block_len) {
            Some(e) => e,
            None => {
                return Err(PcapError::InvalidAsterixFraming(
                    "ASTERIX block length overflow",
                ))
            }
        };

        let raw_block = payload
            .get(offset..block_end)
            .ok_or(PcapError::InvalidAsterixFraming(
                "ASTERIX block extends beyond UDP payload boundary",
            ))?;

        blocks.push(AsterixBlock {
            category,
            length,
            raw_data: raw_block,
        });

        offset = block_end;
    }

    Ok(blocks)
}
