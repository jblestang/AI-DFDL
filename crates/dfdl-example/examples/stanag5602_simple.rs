//! Demonstrates parsing, validation, XML dump, and wire-speed VM decoding
//! of the NATO STANAG 5602: Standard Interface for Multiple Platform Link Evaluation (SIMPLE) protocol.
//!
//! # Protocol Overview
//! NATO STANAG 5602 (SIMPLE) provides standard IP (UDP/TCP) encapsulation for military Tactical Data Links
//! including Link 16 (TADIL J), Link 11 (TADIL A/B), Link 22, and Variable Message Format (VMF).
//! It is used in operational command & control (C2) gateways, simulation environments (DIS/HLA),
//! and interoperability testing benches across NATO forces.

use std::time::Instant;

use dfdl_core::error::DFDLResult;
use dfdl_example::{
    compile_simple_schema, compile_simple_vm, decode_simple_with_vm, infoset_to_xml,
    parse_simple_packet,
};

/// Builds a binary STANAG 5602 SIMPLE packet given header fields and payload.
fn build_simple_packet(
    version: u8,
    security_class: u8,
    msg_type: u16,
    tdl_type: u8,
    flags: u8,
    seq: u32,
    timestamp_nanos: u64,
    tx_id: u32,
    rx_id: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(30 + payload.len());
    // 0. Synchronization Header (2 bytes: 0x49 = 'I', 0x36 = '6')
    buf.push(0x49);
    buf.push(0x36);
    // 1. Version (1 byte)
    buf.push(version);
    // 2. Security Classification (1 byte: 0=Unclass, 1=Restricted, 2=Confidential, 3=Secret, 4=TopSecret)
    buf.push(security_class);
    // 3. Message Type (2 bytes BE: 1=TDL Data, 2=Time Sync, 3=Status)
    buf.extend_from_slice(&msg_type.to_be_bytes());
    // 4. TDL Type (1 byte: 1=Link 11, 3=Link 16, 4=Link 22, 5=VMF)
    buf.push(tdl_type);
    // 5. Control Flags (1 byte: bit 0=simulated, bit 1=loopback, bit 2=test)
    buf.push(flags);
    // 6. Sequence Number (4 bytes BE)
    buf.extend_from_slice(&seq.to_be_bytes());
    // 7. Transmit Timestamp (8 bytes BE nanos)
    buf.extend_from_slice(&timestamp_nanos.to_be_bytes());
    // 8. Transmitter ID (4 bytes BE)
    buf.extend_from_slice(&tx_id.to_be_bytes());
    // 9. Receiver ID (4 bytes BE: 0xFFFFFFFF = Broadcast)
    buf.extend_from_slice(&rx_id.to_be_bytes());
    // 10. Payload Length (2 bytes BE)
    let payload_len = payload.len() as u16;
    buf.extend_from_slice(&payload_len.to_be_bytes());
    // 11. Encapsulated TDL Payload Data
    buf.extend_from_slice(payload);
    buf
}

fn main() -> DFDLResult<()> {
    println!("================================================================================");
    println!("       NATO STANAG 5602: SIMPLE Protocol DFDL Parser & Benchmark                ");
    println!("       Standard Interface for Multiple Platform Link Evaluation                 ");
    println!("================================================================================\n");

    // 1. Compile Schema
    println!("[1/5] Compiling STANAG 5602 SIMPLE DFDL schema...");
    let t0 = Instant::now();
    let schema = compile_simple_schema(Some("SimplePacket"))?;
    let vm_prog = compile_simple_vm(&schema)?;
    println!(
        "      Schema & VM Bytecode compiled successfully in {:.2} ms ({} instructions)\n",
        t0.elapsed().as_secs_f64() * 1000.0,
        vm_prog.instructions.len()
    );

    // 2. Synthesize authentic NATO Tactical Data Link & SIMPLE Control sample packets
    println!("[2/5] Generating authentic Tactical Data Link & SIMPLE Control sample packets...");

    // Sample A: Link 16 J3.2 Air Track message encapsulated in STANAG 5602 (tdlType = 3, msgType = 1)
    let link16_j32_air_track_payload = dfdl_benchmark::data::TEST_MSG_01_DAT;
    let packet_air_track = build_simple_packet(
        2,                                // Version 2
        3,                                // Classification: Secret
        1,                                // Message Type: TDL Data
        3,                                // TDL Type: Link 16 (STANAG 5516 / FakeTDL)
        0,                                // Flags: Operational Live
        1001,                             // Sequence #1001
        1_700_000_000_123_456_789,        // Timestamp UTC nanos
        0x0001_0042,                      // Source Platform ID: C2 Frigate
        0xFFFF_FFFF,                      // Destination: Broadcast
        link16_j32_air_track_payload,
    );
    println!("      Packet A: Link 16 J3.2 Air Track         (tdlType = 3, msgType = 1, {} bytes)", packet_air_track.len());

    // Sample B: Native SIMPLE Radio Terminal Status / Keep-Alive (tdlType = 0, msgType = 3)
    let mut status_payload = Vec::new();
    status_payload.push(3);                                    // terminalState = Synchronized
    status_payload.push(12);                                   // syncQuality = 12
    status_payload.extend_from_slice(&127u16.to_be_bytes());   // netNumber = 127
    status_payload.extend_from_slice(&200.0f32.to_be_bytes()); // rfPowerWatts = 200.0 W
    status_payload.push(3);                                    // cryptoState = Active
    status_payload.extend_from_slice(&43.5f32.to_be_bytes());  // latitude = 43.5
    status_payload.extend_from_slice(&1.5f32.to_be_bytes());   // longitude = 1.5
    status_payload.extend_from_slice(&1200.0f32.to_be_bytes()); // altitudeMeters = 1200.0
    status_payload.extend_from_slice(&0u32.to_be_bytes());     // biteStatus = 0 (Operational)
    let packet_status = build_simple_packet(
        2,
        2,                                // Classification: Confidential
        3,                                // Message Type: Platform / Radio Terminal Status
        0,                                // TDL Type: Native SIMPLE Control (0)
        0,
        1002,
        1_700_000_000_200_000_000,
        0x0001_0042,
        0xFFFF_FFFF,
        &status_payload,
    );
    println!("      Packet B: Native SIMPLE Terminal Status (tdlType = 0, msgType = 3, {} bytes)", packet_status.len());

    // Sample C: Unsupported Link 11A Frame (tdlType = 1) -> Must be rejected cleanly
    let packet_invalid_tdl = build_simple_packet(
        2,
        0,
        1,
        1,                                // TDL Type: Link 11A (Unsupported by this profile)
        0,
        1003,
        1_700_000_000_300_000_000,
        0x0000_0001,
        0xFFFF_FFFF,
        &[0xAA, 0xBB, 0xCC, 0xDD],
    );
    println!("      Packet C: Unsupported Link 11A Frame     (tdlType = 1, {} bytes)", packet_invalid_tdl.len());

    // 3. Parse with ParserEngine and Print XML Infosets
    println!("\n[3/5] Parsing packets with DFDL ParserEngine and generating XML infosets...");
    let doc_a = parse_simple_packet(&schema, &packet_air_track)?;
    let xml_a = infoset_to_xml(&doc_a);
    println!("--- [Packet A: STANAG 5602 Link 16 XML Infoset] ---\n{}", xml_a);

    let doc_b = parse_simple_packet(&schema, &packet_status)?;
    let xml_b = infoset_to_xml(&doc_b);
    println!("--- [Packet B: STANAG 5602 Native Control Status XML Infoset] ---\n{}", xml_b);

    // Verify rejection of Packet C
    println!("      Testing rejection of Packet C (tdlType = 1)...");
    match parse_simple_packet(&schema, &packet_invalid_tdl) {
        Ok(_) => panic!("Packet C should have been rejected!"),
        Err(e) => println!("      [EXPECTED REJECTION] Packet C rejected cleanly: {}", e),
    }

    // 4. Verify Bytecode VM Decoding Parity
    println!("\n[4/5] Verifying zero-allocation DFDL-VM decoding parity...");
    let vm_doc_a = decode_simple_with_vm(&vm_prog, &packet_air_track)?;
    let vm_xml_a = infoset_to_xml(&vm_doc_a);
    assert_eq!(xml_a, vm_xml_a, "VM decoded Packet A infoset must match ParserEngine");

    let vm_doc_b = decode_simple_with_vm(&vm_prog, &packet_status)?;
    let vm_xml_b = infoset_to_xml(&vm_doc_b);
    assert_eq!(xml_b, vm_xml_b, "VM decoded Packet B infoset must match ParserEngine");
    println!("  [PASS] VM execution matches ParserEngine output byte-for-byte on both Link 16 and Native Control!\n");

    // 5. Benchmark Wire-Speed Performance
    println!("[5/5] Benchmarking decoding latency across 100,000 iterations...");
    const ITERS: usize = 100_000;

    // Warmup
    for _ in 0..1_000 {
        let _ = decode_simple_with_vm(&vm_prog, &packet_air_track)?;
    }

    let t_start = Instant::now();
    for _ in 0..ITERS {
        let doc = decode_simple_with_vm(&vm_prog, &packet_air_track)?;
        std::hint::black_box(doc);
    }
    let elapsed = t_start.elapsed();
    let per_pkt_ns = elapsed.as_nanos() as f64 / ITERS as f64;
    let mpps = (ITERS as f64 / elapsed.as_secs_f64()) / 1_000_000.0;
    let mbit_s = (ITERS as f64 * packet_air_track.len() as f64 * 8.0 / elapsed.as_secs_f64()) / 1_000_000.0;

    println!("================================================================================");
    println!("       STANAG 5602 (SIMPLE) Latency Benchmark Results                           ");
    println!("================================================================================");
    println!("  • Total Iterations     : {}", ITERS);
    println!("  • Elapsed Time         : {:.2?}", elapsed);
    println!("  • VM Decoding Latency  : {:.2} ns / packet", per_pkt_ns);
    println!("  • Packet Throughput    : {:.2} Million packets / sec", mpps);
    println!("  • Network Wire Rate    : {:.2} Mbit / sec", mbit_s);
    println!("================================================================================\n");

    Ok(())
}
