//! High-performance, zero-dependency PCAP (Packet Capture) format generator.
//!
//! Conforms to Libpcap / tcpdump file format specification and IEEE 802.3 / IETF RFC 791 / RFC 768
//! standards for Ethernet, IPv4, and UDP encapsulation.

use std::io::{self, Write};

/// Standard PCAP magic numbers defining byte ordering and timestamp resolution.
pub mod magic {
    /// Standard microsecond resolution PCAP magic number (Little-Endian / host order).
    pub const TCPDUMP_MICROSECOND: u32 = 0xa1b2_c3d4;
    /// Nanosecond resolution PCAP magic number.
    pub const TCPDUMP_NANOSECOND: u32 = 0xa1b2_3c4d;
}

/// Link-layer header types as specified by tcpdump / libpcap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LinkType {
    /// IEEE 802.3 Ethernet link layer (LinkType 1).
    #[default]
    Ethernet = 1,
    /// Raw IPv4 / IPv6 without link layer header (LinkType 101).
    RawIp = 101,
    /// User private link layer 0 (LinkType 147).
    User0 = 147,
    /// Raw unencapsulated protocol payload (LinkType 228).
    Raw = 228,
}

/// Network addressing configuration for packet encapsulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkConfig {
    /// Source MAC address (6 octets).
    pub src_mac: [u8; 6],
    /// Destination MAC address (6 octets).
    pub dst_mac: [u8; 6],
    /// Source IPv4 address.
    pub src_ip: [u8; 4],
    /// Destination IPv4 address.
    pub dst_ip: [u8; 4],
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            src_mac: [0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E],
            dst_mac: [0x00, 0x50, 0x56, 0xC0, 0x00, 0x01],
            src_ip: [192, 168, 1, 100],
            dst_ip: [192, 168, 1, 200],
        }
    }
}

/// Layer 4 transport protocol encapsulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportProtocol {
    /// Raw link-layer or unencapsulated payload.
    Raw,
    /// UDP transport with source and destination ports.
    Udp {
        /// Source transport port.
        src_port: u16,
        /// Destination transport port (e.g. 5602 for STANAG 5602 SIMPLE).
        dst_port: u16,
    },
}

impl Default for TransportProtocol {
    fn default() -> Self {
        Self::Udp {
            src_port: 5602,
            dst_port: 5602,
        }
    }
}

/// Represents a single packet within a PCAP stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcapPacket {
    /// Timestamp seconds.
    pub ts_sec: u32,
    /// Timestamp microseconds.
    pub ts_usec: u32,
    /// Frame payload bytes.
    pub data: Vec<u8>,
}

/// Builder and serializer for standard PCAP capture files.
#[derive(Debug, Clone)]
pub struct PcapWriter {
    /// Configured link layer header type.
    pub link_type: LinkType,
    /// Maximum captured packet length (snaplen).
    pub snaplen: u32,
}

impl Default for PcapWriter {
    fn default() -> Self {
        Self {
            link_type: LinkType::Ethernet,
            snaplen: 65535,
        }
    }
}

impl PcapWriter {
    /// Creates a new PCAP writer configured with the given link type.
    pub fn new(link_type: LinkType) -> Self {
        Self {
            link_type,
            snaplen: 65535,
        }
    }

    /// Writes the 24-octet global PCAP header to the output stream.
    pub fn write_header<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let mut hdr = [0u8; 24];
        let magic = magic::TCPDUMP_MICROSECOND.to_ne_bytes();
        let v_major = 2u16.to_ne_bytes();
        let v_minor = 4u16.to_ne_bytes();
        let thiszone = 0i32.to_ne_bytes();
        let sigfigs = 0u32.to_ne_bytes();
        let snaplen = self.snaplen.to_ne_bytes();
        let network = (self.link_type as u32).to_ne_bytes();

        if let Some(dest) = hdr.get_mut(0..4) {
            dest.copy_from_slice(&magic);
        }
        if let Some(dest) = hdr.get_mut(4..6) {
            dest.copy_from_slice(&v_major);
        }
        if let Some(dest) = hdr.get_mut(6..8) {
            dest.copy_from_slice(&v_minor);
        }
        if let Some(dest) = hdr.get_mut(8..12) {
            dest.copy_from_slice(&thiszone);
        }
        if let Some(dest) = hdr.get_mut(12..16) {
            dest.copy_from_slice(&sigfigs);
        }
        if let Some(dest) = hdr.get_mut(16..20) {
            dest.copy_from_slice(&snaplen);
        }
        if let Some(dest) = hdr.get_mut(20..24) {
            dest.copy_from_slice(&network);
        }

        w.write_all(&hdr)
    }

    /// Writes a single packet with its 16-octet header to the output stream.
    pub fn write_packet<W: Write>(
        &self,
        w: &mut W,
        ts_sec: u32,
        ts_usec: u32,
        data: &[u8],
    ) -> io::Result<()> {
        let mut pkt_hdr = [0u8; 16];
        let incl_len = (data.len() as u32).min(self.snaplen);
        let orig_len = data.len() as u32;

        if let Some(dest) = pkt_hdr.get_mut(0..4) {
            dest.copy_from_slice(&ts_sec.to_ne_bytes());
        }
        if let Some(dest) = pkt_hdr.get_mut(4..8) {
            dest.copy_from_slice(&ts_usec.to_ne_bytes());
        }
        if let Some(dest) = pkt_hdr.get_mut(8..12) {
            dest.copy_from_slice(&incl_len.to_ne_bytes());
        }
        if let Some(dest) = pkt_hdr.get_mut(12..16) {
            dest.copy_from_slice(&orig_len.to_ne_bytes());
        }

        w.write_all(&pkt_hdr)?;
        if let Some(slice) = data.get(..incl_len as usize) {
            w.write_all(slice)?;
        }
        Ok(())
    }

    /// Encapsulates a DFDL payload into an Ethernet / IPv4 / UDP packet and writes it.
    pub fn write_encapsulated_packet<W: Write>(
        &self,
        w: &mut W,
        ts_sec: u32,
        ts_usec: u32,
        config: &NetworkConfig,
        transport: &TransportProtocol,
        payload: &[u8],
    ) -> io::Result<()> {
        let frame = match transport {
            TransportProtocol::Raw => payload.to_vec(),
            TransportProtocol::Udp { src_port, dst_port } => {
                Self::build_ethernet_ip_udp_frame(config, *src_port, *dst_port, payload)
            }
        };
        self.write_packet(w, ts_sec, ts_usec, &frame)
    }

    /// Serializes an in-memory collection of packets into a complete PCAP byte buffer.
    pub fn to_bytes(&self, packets: &[PcapPacket]) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_header(&mut out)?;
        for p in packets {
            self.write_packet(&mut out, p.ts_sec, p.ts_usec, &p.data)?;
        }
        Ok(out)
    }

    /// Builds a full Ethernet (14) + IPv4 (20) + UDP (8) frame around payload.
    pub fn build_ethernet_ip_udp_frame(
        config: &NetworkConfig,
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let udp_len = (8usize.saturating_add(payload.len()) as u16).max(8);
        let ip_total_len = (20usize.saturating_add(udp_len as usize) as u16).max(28);
        let total_frame_size = 14usize.saturating_add(ip_total_len as usize);

        let mut frame = Vec::with_capacity(total_frame_size);

        // 1. Ethernet Header (14 bytes)
        frame.extend_from_slice(&config.dst_mac);
        frame.extend_from_slice(&config.src_mac);
        frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4 EtherType

        // 2. IPv4 Header (20 bytes)
        let ip_start = frame.len();
        frame.push(0x45); // Version 4, IHL 5
        frame.push(0x00); // DSCP 0, ECN 0
        frame.extend_from_slice(&ip_total_len.to_be_bytes());
        frame.extend_from_slice(&0x0001u16.to_be_bytes()); // Identification
        frame.extend_from_slice(&0x4000u16.to_be_bytes()); // Flags (Don't Fragment)
        frame.push(64);   // TTL = 64
        frame.push(17);   // Protocol = 17 (UDP)
        frame.extend_from_slice(&0x0000u16.to_be_bytes()); // Placeholder checksum
        frame.extend_from_slice(&config.src_ip);
        frame.extend_from_slice(&config.dst_ip);

        // Calculate and insert IPv4 header checksum
        let ip_cksum = if let Some(ip_bytes) = frame.get(ip_start..ip_start.saturating_add(20)) {
            Self::compute_internet_checksum(ip_bytes)
        } else {
            0
        };
        if let Some(dest) = frame.get_mut(ip_start.saturating_add(10)..ip_start.saturating_add(12)) {
            dest.copy_from_slice(&ip_cksum.to_be_bytes());
        }

        // 3. UDP Header (8 bytes)
        let udp_start = frame.len();
        frame.extend_from_slice(&src_port.to_be_bytes());
        frame.extend_from_slice(&dst_port.to_be_bytes());
        frame.extend_from_slice(&udp_len.to_be_bytes());
        frame.extend_from_slice(&0x0000u16.to_be_bytes()); // Checksum placeholder

        // 4. Payload
        frame.extend_from_slice(payload);

        // Calculate and insert UDP checksum over IPv4 pseudo-header
        let udp_cksum = Self::compute_udp_checksum(
            &config.src_ip,
            &config.dst_ip,
            if let Some(udp_data) = frame.get(udp_start..) { udp_data } else { &[] },
        );
        let final_cksum = if udp_cksum == 0 { 0xFFFF } else { udp_cksum };
        if let Some(dest) = frame.get_mut(udp_start.saturating_add(6)..udp_start.saturating_add(8)) {
            dest.copy_from_slice(&final_cksum.to_be_bytes());
        }

        frame
    }

    /// Computes RFC 791 / RFC 1071 16-bit one's complement internet checksum.
    pub fn compute_internet_checksum(bytes: &[u8]) -> u16 {
        let mut sum = 0u32;
        let mut i = 0;
        while i < bytes.len() {
            let b1 = *bytes.get(i).unwrap_or(&0) as u32;
            let b2 = *bytes.get(i.saturating_add(1)).unwrap_or(&0) as u32;
            let word = if i.saturating_add(1) < bytes.len() {
                (b1 << 8) | b2
            } else {
                b1 << 8
            };
            sum = sum.saturating_add(word);
            i = i.saturating_add(2);
        }
        while (sum >> 16) > 0 {
            sum = (sum & 0xFFFF).saturating_add(sum >> 16);
        }
        !(sum as u16)
    }

    /// Computes RFC 768 UDP checksum with IPv4 pseudo-header.
    pub fn compute_udp_checksum(src_ip: &[u8; 4], dst_ip: &[u8; 4], udp_segment: &[u8]) -> u16 {
        let mut sum = 0u32;

        // Pseudo-header: Source IP (2 words)
        let s1 = ((*src_ip.first().unwrap_or(&0) as u32) << 8) | (*src_ip.get(1).unwrap_or(&0) as u32);
        let s2 = ((*src_ip.get(2).unwrap_or(&0) as u32) << 8) | (*src_ip.get(3).unwrap_or(&0) as u32);
        sum = sum.saturating_add(s1).saturating_add(s2);

        // Pseudo-header: Destination IP (2 words)
        let d1 = ((*dst_ip.first().unwrap_or(&0) as u32) << 8) | (*dst_ip.get(1).unwrap_or(&0) as u32);
        let d2 = ((*dst_ip.get(2).unwrap_or(&0) as u32) << 8) | (*dst_ip.get(3).unwrap_or(&0) as u32);
        sum = sum.saturating_add(d1).saturating_add(d2);

        // Pseudo-header: Zero (1 byte) + Protocol UDP 17 (1 byte)
        sum = sum.saturating_add(17);

        // Pseudo-header: UDP Length
        let udp_len = udp_segment.len() as u32;
        sum = sum.saturating_add(udp_len);

        // UDP Header + Data
        let mut i = 0;
        while i < udp_segment.len() {
            // Skip the checksum field itself (bytes 6 and 7 of UDP header)
            if i == 6 {
                i = i.saturating_add(2);
                continue;
            }
            let b1 = *udp_segment.get(i).unwrap_or(&0) as u32;
            let b2 = *udp_segment.get(i.saturating_add(1)).unwrap_or(&0) as u32;
            let word = if i.saturating_add(1) < udp_segment.len() {
                (b1 << 8) | b2
            } else {
                b1 << 8
            };
            sum = sum.saturating_add(word);
            i = i.saturating_add(2);
        }

        while (sum >> 16) > 0 {
            sum = (sum & 0xFFFF).saturating_add(sum >> 16);
        }
        !(sum as u16)
    }
}

impl PcapPacket {
    /// Extracts payload from an encapsulated Ethernet + IPv4 + UDP packet.
    pub fn extract_udp_payload(&self) -> Option<&[u8]> {
        if self.data.len() < 14 + 20 + 8 {
            return None;
        }
        let ihl = (*self.data.get(14)? & 0x0F) as usize * 4;
        let udp_start = 14 + ihl;
        let payload_start = udp_start + 8;
        if self.data.len() < payload_start {
            return None;
        }
        self.data.get(payload_start..)
    }
}

/// In-memory representation of a parsed PCAP capture file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcapFile {
    /// PCAP magic number (e.g. 0xa1b2c3d4).
    pub magic: u32,
    /// Major version number (typically 2).
    pub version_major: u16,
    /// Minor version number (typically 4).
    pub version_minor: u16,
    /// Maximum snapshot length.
    pub snaplen: u32,
    /// Link layer header type code.
    pub link_type: u32,
    /// Vector of captured packets.
    pub packets: Vec<PcapPacket>,
}

impl PcapFile {
    /// Parses raw PCAP bytes into a structured `PcapFile`.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 24 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "PCAP buffer too small for 24-byte global header",
            ));
        }

        let magic = u32::from_ne_bytes(
            bytes.get(0..4)
                .and_then(|s| s.try_into().ok())
                .unwrap_or_default(),
        );
        let version_major = u16::from_ne_bytes(
            bytes.get(4..6)
                .and_then(|s| s.try_into().ok())
                .unwrap_or_default(),
        );
        let version_minor = u16::from_ne_bytes(
            bytes.get(6..8)
                .and_then(|s| s.try_into().ok())
                .unwrap_or_default(),
        );
        let snaplen = u32::from_ne_bytes(
            bytes.get(16..20)
                .and_then(|s| s.try_into().ok())
                .unwrap_or_default(),
        );
        let link_type = u32::from_ne_bytes(
            bytes.get(20..24)
                .and_then(|s| s.try_into().ok())
                .unwrap_or_default(),
        );

        let mut packets = Vec::new();
        let mut offset = 24usize;

        while offset.saturating_add(16) <= bytes.len() {
            let ts_sec = u32::from_ne_bytes(
                bytes.get(offset..offset.saturating_add(4))
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or_default(),
            );
            let ts_usec = u32::from_ne_bytes(
                bytes.get(offset.saturating_add(4)..offset.saturating_add(8))
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or_default(),
            );
            let incl_len = u32::from_ne_bytes(
                bytes.get(offset.saturating_add(8)..offset.saturating_add(12))
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or_default(),
            ) as usize;

            offset = offset.saturating_add(16);

            if offset.saturating_add(incl_len) > bytes.len() {
                break;
            }

            let data = bytes.get(offset..offset.saturating_add(incl_len))
                .unwrap_or(&[])
                .to_vec();
            offset = offset.saturating_add(incl_len);

            packets.push(PcapPacket {
                ts_sec,
                ts_usec,
                data,
            });
        }

        Ok(Self {
            magic,
            version_major,
            version_minor,
            snaplen,
            link_type,
            packets,
        })
    }
}
