//! Generic PCAP capture dissection driven strictly by DFDL schemas.
//!
//! Iterates through libpcap network captures, extracts payload datagrams (UDP, IPv4,
//! or raw Ethernet), and executes the generic DFDL dissector engine to produce
//! Wireshark dissection trees.
//!
//! This module contains zero protocol-specific or field-specific hardcoding.

use crate::dissector::DissectionTree;
use crate::engine::DfdlDissectorEngine;
use dfdl_example::pcap::{extract_udp_payload, parse_pcap_header, parse_pcap_packets};

/// Decoded packet summary holding packet metadata and its hierarchical dissection tree.
#[derive(Debug, Clone, PartialEq)]
pub struct DissectedPacket {
    /// 1-based sequential packet index in the PCAP capture.
    pub packet_index: usize,
    /// Packet timestamp seconds.
    pub timestamp_sec: u32,
    /// Packet timestamp fraction (microseconds).
    pub timestamp_frac: u32,
    /// Size of the payload passed to the DFDL engine.
    pub payload_len: usize,
    /// Dissection tree with converted engineering units and documentation.
    pub tree: DissectionTree,
}

/// Decodes packets from a PCAP capture byte slice using the provided DFDL engine.
///
/// # Errors
/// Returns a descriptive error message if PCAP framing is corrupt or parsing fails.
pub fn decode_pcap_with_engine(
    pcap_data: &[u8],
    engine: &DfdlDissectorEngine,
    limit: Option<usize>,
) -> Result<Vec<DissectedPacket>, String> {
    let (header, _) = parse_pcap_header(pcap_data)
        .map_err(|e| format!("Invalid PCAP header: {e}"))?;

    let raw_packets = parse_pcap_packets(pcap_data, &header)
        .map_err(|e| format!("Failed to extract PCAP packets: {e}"))?;

    let max_pkts = limit.unwrap_or(usize::MAX);
    let mut results = Vec::new();

    for (idx, pkt) in raw_packets.iter().enumerate() {
        if results.len() >= max_pkts {
            break;
        }

        // Generic payload extraction: try UDP first, then fallback to packet data
        let payload = extract_udp_payload(pkt.data).unwrap_or(pkt.data);
        if payload.is_empty() {
            continue;
        }

        match engine.dissect_payload(payload) {
            Ok(tree) => {
                let pkt_num = match idx.checked_add(1) {
                    Some(n) => n,
                    None => idx,
                };
                results.push(DissectedPacket {
                    packet_index: pkt_num,
                    timestamp_sec: pkt.timestamp_sec,
                    timestamp_frac: pkt.timestamp_frac,
                    payload_len: payload.len(),
                    tree,
                });
            }
            Err(_e) => {
                // If a non-matching packet is encountered, skip or log
                continue;
            }
        }
    }

    Ok(results)
}
