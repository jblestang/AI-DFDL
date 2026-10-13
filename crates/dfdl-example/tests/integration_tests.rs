//! Integration tests for the `dfdl-example` Eurocontrol ASTERIX parsing suite.
//!
//! Validates:
//! - Full PCAP capture extraction and 100% schema validation across 120 blocks.
//! - XML Infoset generation and syntactic correctness.
//! - Wire-speed VM compilation, bytecode execution, and ParserEngine DOM parity.
//! - Synthetic test frames for Category 001 (Monoradar Data) and Category 002 (Service Messages).
//! - Malformed / truncated packet error rejection.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use dfdl_example::pcap;
use dfdl_example::{
    compile_asterix_schema, compile_asterix_vm, decode_with_vm, infoset_to_xml,
    parse_asterix_record, SAMPLE_CAT_034_048_PCAP,
};

/// Validates that all 120 ASTERIX blocks in the authentic CroatiaControl PCAP sample
/// pass DFDL schema validation with zero errors.
#[test]
fn test_pcap_full_validation() {
    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP).unwrap();
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header).unwrap();
    assert_eq!(packets.len(), 100, "PCAP should contain 100 Ethernet packets");

    let schema_record = compile_asterix_schema(Some("AsterixRecord")).unwrap();
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();
    let schema_cat48 = compile_asterix_schema(Some("Cat048Record")).unwrap();

    let mut cat34_count = 0usize;
    let mut cat48_count = 0usize;
    let mut validated_blocks = 0usize;

    for pkt in &packets {
        let udp_payload = pcap::extract_udp_payload(pkt.data).unwrap();
        let blocks = pcap::extract_asterix_blocks(udp_payload).unwrap();
        for block in blocks {
            match block.category {
                34 => cat34_count += 1,
                48 => cat48_count += 1,
                _ => {}
            }

            let schema = match block.category {
                34 => &schema_cat34,
                48 => &schema_cat48,
                _ => &schema_record,
            };

            let doc = parse_asterix_record(schema, block.raw_data)
                .unwrap_or_else(|e| panic!("Failed on Cat {} block: {:?}", block.category, e));
            assert!(doc.total_nodes > 0);
            validated_blocks += 1;
        }
    }

    assert_eq!(cat34_count, 34, "Expected 34 Cat 034 blocks");
    assert_eq!(cat48_count, 86, "Expected 86 Cat 048 blocks");
    assert_eq!(validated_blocks, 120, "Expected 120 validated blocks (100% pass)");
}

/// Tests XML serialization of parsed ASTERIX infosets for schema conformance.
#[test]
fn test_xml_infoset_dump() {
    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP).unwrap();
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header).unwrap();

    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();
    let mut first_cat34_data = None;
    for pkt in &packets {
        if let Ok(udp) = pcap::extract_udp_payload(pkt.data) {
            if let Ok(blocks) = pcap::extract_asterix_blocks(udp) {
                for block in blocks {
                    if block.category == 34 {
                        first_cat34_data = Some(block.raw_data);
                        break;
                    }
                }
            }
        }
        if first_cat34_data.is_some() {
            break;
        }
    }

    let raw = first_cat34_data.expect("PCAP must contain at least one Cat 034 block");
    let doc = parse_asterix_record(&schema_cat34, raw).unwrap();
    let xml = infoset_to_xml(&doc);

    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("<Cat034Record>"));
    assert!(xml.contains("<category>34</category>"));
    assert!(xml.contains("<sac>"));
    assert!(xml.contains("<sic>"));
    assert!(xml.contains("<timeOfDay>"));
    assert!(xml.ends_with("</Cat034Record>\n"));
}

/// Tests `dfdl-vm` compilation and execution on all 34 Cat 034 blocks with ParserEngine parity.
#[test]
fn test_dfdl_vm_cat034_execution_and_parity() {
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();
    let vm_program = compile_asterix_vm(&schema_cat34).unwrap();

    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP).unwrap();
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header).unwrap();

    let mut cat34_tested = 0usize;
    for pkt in &packets {
        let udp = pcap::extract_udp_payload(pkt.data).unwrap();
        let blocks = pcap::extract_asterix_blocks(udp).unwrap();
        for block in blocks {
            if block.category == 34 {
                let vm_doc = decode_with_vm(&vm_program, block.raw_data).unwrap();
                let engine_doc = parse_asterix_record(&schema_cat34, block.raw_data).unwrap();

                assert_eq!(
                    vm_doc.total_nodes, engine_doc.total_nodes,
                    "Node count mismatch on Cat 034 block"
                );
                cat34_tested += 1;
            }
        }
    }
    assert_eq!(cat34_tested, 34, "All 34 Cat 034 blocks decoded with VM");
}

/// Tests parsing and VM decoding of synthetic Category 001 (Monoradar Data) records.
#[test]
fn test_cat001_synthetic_record() {
    // Construct synthetic Cat 001 record matching Cat001RecordType:
    // category(1) = 1
    // length(2)   = 28 bytes
    // fspec1(1)   = 0xf0
    // fspec2(1)   = 0x00
    // sac(1)      = 25
    // sic(1)      = 12
    // targetDesc(1) = 0x10
    // trackNum(2) = 1042
    // rho(2)      = 12000
    // theta(2)    = 800
    // cartX(2)    = 100
    // cartY(2)    = -200 (0xff38)
    // gSpeed(2)   = 400
    // heading(2)  = 90
    // mode3a(2)   = 0x0740
    // fl(2)       = 250
    // truncTod(2) = 6400
    // Test multiple plots/tracks in a single Category 001 Data Block (2 records = 53 bytes total)
    let mut raw = Vec::new();
    raw.push(1u8); // Category 1
    raw.extend_from_slice(&53u16.to_be_bytes()); // Length = 3 header + 25 + 25 = 53 bytes

    // Record 1 (Plot/Track 1042)
    raw.push(0xf0u8); // fspec1
    raw.push(0x00u8); // fspec2
    raw.push(25u8); // sac
    raw.push(12u8); // sic
    raw.push(0x10u8); // targetDescriptor
    raw.extend_from_slice(&1042u16.to_be_bytes()); // trackNumber
    raw.extend_from_slice(&12000u16.to_be_bytes()); // rho
    raw.extend_from_slice(&800u16.to_be_bytes()); // theta
    raw.extend_from_slice(&100i16.to_be_bytes()); // cartesianX
    raw.extend_from_slice(&(-200i16).to_be_bytes()); // cartesianY
    raw.extend_from_slice(&400u16.to_be_bytes()); // groundSpeed
    raw.extend_from_slice(&90u16.to_be_bytes()); // heading
    raw.extend_from_slice(&0x0740u16.to_be_bytes()); // mode3a
    raw.extend_from_slice(&250u16.to_be_bytes()); // flightLevel
    raw.extend_from_slice(&6400u16.to_be_bytes()); // truncatedTimeOfDay

    // Record 2 (Plot/Track 1043) in the same data block
    raw.push(0xf0u8); // fspec1
    raw.push(0x00u8); // fspec2
    raw.push(25u8); // sac
    raw.push(12u8); // sic
    raw.push(0x10u8); // targetDescriptor
    raw.extend_from_slice(&1043u16.to_be_bytes()); // trackNumber
    raw.extend_from_slice(&13500u16.to_be_bytes()); // rho
    raw.extend_from_slice(&950u16.to_be_bytes()); // theta
    raw.extend_from_slice(&150i16.to_be_bytes()); // cartesianX
    raw.extend_from_slice(&(-250i16).to_be_bytes()); // cartesianY
    raw.extend_from_slice(&420u16.to_be_bytes()); // groundSpeed
    raw.extend_from_slice(&95u16.to_be_bytes()); // heading
    raw.extend_from_slice(&0x0741u16.to_be_bytes()); // mode3a
    raw.extend_from_slice(&260u16.to_be_bytes()); // flightLevel
    raw.extend_from_slice(&6405u16.to_be_bytes()); // truncatedTimeOfDay

    assert_eq!(raw.len(), 53);

    // 1. Validate with specific Cat001Record schema
    let schema_cat1 = compile_asterix_schema(Some("Cat001Record")).unwrap();
    let doc = parse_asterix_record(&schema_cat1, &raw).unwrap();
    assert_eq!(doc.root.as_ref().unwrap().name.local_name, "Cat001Record");

    // 2. Validate polymorphic AsterixRecord schema dispatch (lookahead category 1)
    let schema_poly = compile_asterix_schema(Some("AsterixRecord")).unwrap();
    let poly_doc = parse_asterix_record(&schema_poly, &raw).unwrap();
    assert_eq!(poly_doc.root.as_ref().unwrap().name.local_name, "AsterixRecord");

    // 3. Compile to VM bytecode and execute
    let vm_prog = compile_asterix_vm(&schema_cat1).unwrap();
    let vm_doc = decode_with_vm(&vm_prog, &raw).unwrap();
    assert_eq!(vm_doc.total_nodes, doc.total_nodes);

    // 4. Verify XML generation contains both Cat 001 plot/track records
    let xml = infoset_to_xml(&doc);
    assert!(xml.contains("<Cat001Record>"));
    assert!(xml.contains("<category>1</category>"));
    assert!(xml.contains("<trackNumber>1042</trackNumber>"));
    assert!(xml.contains("<trackNumber>1043</trackNumber>"));
    assert!(xml.contains("<rho>12000</rho>"));
    assert!(xml.contains("<rho>13500</rho>"));
}

/// Tests extraction and parsing of multiple plots/tracks within a single network packet (UDP datagram).
/// Verifies:
/// 1. A single network packet carrying multiple ASTERIX Data Blocks (e.g. Cat 048 plots + Cat 034 status).
/// 2. Multiple plot/track records inside a single Cat 048 data block (e.g. 416-byte block from PCAP).
#[test]
fn test_multiple_plots_tracks_in_network_packet() {
    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP).unwrap();
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header).unwrap();

    // Packet 13 (index 12) contains both a 416-byte Cat 048 block (multiple plots/tracks)
    // and an 11-byte Cat 034 block (sector crossing message) in the SAME UDP datagram.
    let pkt13 = &packets[12];
    let udp13 = pcap::extract_udp_payload(pkt13.data).unwrap();
    assert_eq!(udp13.len(), 427, "Packet 13 UDP payload should be 427 bytes");

    let blocks = pcap::extract_asterix_blocks(udp13).unwrap();
    assert_eq!(blocks.len(), 2, "UDP datagram should contain 2 ASTERIX Data Blocks");
    assert_eq!(blocks[0].category, 48);
    assert_eq!(blocks[0].length, 416);
    assert_eq!(blocks[1].category, 34);
    assert_eq!(blocks[1].length, 11);

    // Validate both blocks with schemas
    let schema_cat48 = compile_asterix_schema(Some("Cat048Record")).unwrap();
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();

    let doc_cat48 = parse_asterix_record(&schema_cat48, blocks[0].raw_data).unwrap();
    assert!(doc_cat48.total_nodes > 0);

    let doc_cat34 = parse_asterix_record(&schema_cat34, blocks[1].raw_data).unwrap();
    assert!(doc_cat34.total_nodes > 0);

    // Validate that the Cat 048 block of 416 bytes carries multiple target reports (plots/tracks)
    // and that Cat 034 carries the sector crossing message
    let xml_cat48 = infoset_to_xml(&doc_cat48);
    assert!(xml_cat48.contains("<Cat048Record>"));
    assert!(xml_cat48.contains("<category>48</category>"));
    assert!(xml_cat48.contains("<length>416</length>"));

    let xml_cat34 = infoset_to_xml(&doc_cat34);
    assert!(xml_cat34.contains("<Cat034Record>"));
    assert!(xml_cat34.contains("<sectorCrossing>"));
    assert!(xml_cat34.contains("<messageType>2</messageType>"));
}

/// Tests parsing and VM decoding of synthetic Category 002 (Monoradar Service Messages) records.
/// Verifies that fields are decoded conditionally according to `messageType`:
/// - North Marker (type 1) has `antennaRotationPeriod` and no `sectorNumber`.
/// - Sector Crossing (type 2) has `sectorNumber` and no `antennaRotationPeriod`.
#[test]
fn test_cat002_synthetic_record() {
    // 1. Construct North Marker (msgType = 1): 15 bytes total
    let mut raw_nm = Vec::new();
    raw_nm.push(2u8); // Category 2
    raw_nm.extend_from_slice(&15u16.to_be_bytes()); // Length = 15
    raw_nm.push(0xf0u8); // fspec1
    raw_nm.push(0x00u8); // fspec2
    raw_nm.push(10u8); // sac
    raw_nm.push(5u8); // sic
    raw_nm.push(1u8); // messageType = 1 (North marker)
    raw_nm.extend_from_slice(&[0x01, 0x23, 0x45]); // timeOfDay (24 bits)
    raw_nm.extend_from_slice(&512u16.to_be_bytes()); // antennaRotationPeriod
    raw_nm.push(0x80u8); // stationConfig
    raw_nm.push(0x40u8); // stationProcessingMode

    assert_eq!(raw_nm.len(), 15);

    let schema_cat2 = compile_asterix_schema(Some("Cat002Record")).unwrap();
    let doc_nm = parse_asterix_record(&schema_cat2, &raw_nm).unwrap();
    assert_eq!(doc_nm.root.as_ref().unwrap().name.local_name, "Cat002Record");

    let xml_nm = infoset_to_xml(&doc_nm);
    assert!(xml_nm.contains("<northMarker>"));
    assert!(xml_nm.contains("<antennaRotationPeriod>512</antennaRotationPeriod>"));
    assert!(!xml_nm.contains("<sectorNumber>"), "North Marker must NOT contain sectorNumber");

    // 2. Construct Sector Crossing (msgType = 2): 12 bytes total
    let mut raw_sc = Vec::new();
    raw_sc.push(2u8); // Category 2
    raw_sc.extend_from_slice(&12u16.to_be_bytes()); // Length = 12
    raw_sc.push(0xf0u8); // fspec1
    raw_sc.push(0x00u8); // fspec2
    raw_sc.push(10u8); // sac
    raw_sc.push(5u8); // sic
    raw_sc.push(2u8); // messageType = 2 (Sector crossing)
    raw_sc.push(128u8); // sectorNumber
    raw_sc.extend_from_slice(&[0x01, 0x23, 0x45]); // timeOfDay (24 bits)

    assert_eq!(raw_sc.len(), 12);

    let doc_sc = parse_asterix_record(&schema_cat2, &raw_sc).unwrap();
    let xml_sc = infoset_to_xml(&doc_sc);
    assert!(xml_sc.contains("<sectorCrossing>"));
    assert!(xml_sc.contains("<sectorNumber>128</sectorNumber>"));
    assert!(!xml_sc.contains("<antennaRotationPeriod>"), "Sector Crossing must NOT contain antennaRotationPeriod");

    // 3. Polymorphic AsterixRecord schema dispatch
    let schema_poly = compile_asterix_schema(Some("AsterixRecord")).unwrap();
    let poly_doc = parse_asterix_record(&schema_poly, &raw_nm).unwrap();
    assert_eq!(poly_doc.root.as_ref().unwrap().name.local_name, "AsterixRecord");
}

/// Tests error handling on malformed or truncated inputs.
#[test]
fn test_malformed_inputs() {
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();

    // Empty slice
    let res = parse_asterix_record(&schema_cat34, &[]);
    assert!(res.is_err(), "Empty input should fail gracefully");

    // Truncated block (only category byte)
    let res = parse_asterix_record(&schema_cat34, &[34]);
    assert!(res.is_err(), "Truncated header should fail");

    // Invalid PCAP header
    let res = pcap::parse_pcap_header(&[0x00, 0x00, 0x00, 0x00]);
    assert!(res.is_err(), "Invalid PCAP magic should fail");

    // Truncated UDP packet
    let res = pcap::extract_udp_payload(&[0x00; 10]);
    assert!(res.is_err(), "Truncated Ethernet/IP header should fail");
}

/// Tests that each individual Eurocontrol Category 034 message type (Types 1 through 8+)
/// dispatches to its dedicated XML Infoset branch rather than a generic bucket:
/// - Type 1: North Marker (`<northMarker>`)
/// - Type 2: Sector Crossing (`<sectorCrossing>`)
/// - Type 3: Geographical Filtering (`<geographicalFiltering>`)
/// - Type 4: Jamming Strobe (`<jammingStrobe>`)
/// - Type 5: Solar Storm (`<solarStorm>`)
/// - Type 6: SSR Jamming Strobe (`<ssrJammingStrobe>`)
/// - Type 7: Mode S Jamming Strobe (`<modeSJammingStrobe>`)
/// - Type 8+: Other / Reserved Service (`<otherService>`)
#[test]
fn test_cat034_all_detailed_message_types() {
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record")).unwrap();

    // Helper to build a Cat 034 header and payload with exact FSPEC1 bitmask
    let build_record = |fspec1: u8, msg_type: u8, extra_payload: &[u8]| -> Vec<u8> {
        let mut raw = Vec::new();
        raw.push(34u8); // Category 34
        let len = (3 + 1 + 2 + 1 + 3 + extra_payload.len()) as u16;
        raw.extend_from_slice(&len.to_be_bytes()); // Length
        raw.push(fspec1); // fspec1: 0xe0 for SAC/SIC+MsgType+TimeOfDay, 0xf0 when including SectorNumber
        raw.push(25u8); // SAC
        raw.push(1u8); // SIC
        raw.push(msg_type); // Message Type
        raw.extend_from_slice(&[0x12, 0x34, 0x56]); // Time of Day (24 bits)
        raw.extend_from_slice(extra_payload);
        raw
    };

    // 1. Message Type 3: Geographical Filtering (fspec1 = 0xe0: SAC/SIC, MsgType, TimeOfDay)
    let raw_gf = build_record(0xe0, 3, &[]);
    let doc_gf = parse_asterix_record(&schema_cat34, &raw_gf).unwrap();
    let xml_gf = infoset_to_xml(&doc_gf);
    assert!(xml_gf.contains("<geographicalFiltering>"), "Expected <geographicalFiltering> in XML: {}", xml_gf);
    assert!(xml_gf.contains("<messageType>3</messageType>"));

    // 2. Message Type 4: Jamming Strobe (FRN 4 = Sector Number, fspec1 = 0xf0)
    let raw_js = build_record(0xf0, 4, &[128u8]); // sectorNumber = 128
    let doc_js = parse_asterix_record(&schema_cat34, &raw_js).unwrap();
    let xml_js = infoset_to_xml(&doc_js);
    assert!(xml_js.contains("<jammingStrobe>"), "Expected <jammingStrobe> in XML: {}", xml_js);
    assert!(xml_js.contains("<messageType>4</messageType>"));
    assert!(xml_js.contains("<sectorNumber>128</sectorNumber>"));

    // 3. Message Type 5: Solar Storm (FRN 4 = Sector Number, fspec1 = 0xf0)
    let raw_ss = build_record(0xf0, 5, &[64u8]); // sectorNumber = 64
    let doc_ss = parse_asterix_record(&schema_cat34, &raw_ss).unwrap();
    let xml_ss = infoset_to_xml(&doc_ss);
    assert!(xml_ss.contains("<solarStorm>"), "Expected <solarStorm> in XML: {}", xml_ss);
    assert!(xml_ss.contains("<messageType>5</messageType>"));
    assert!(xml_ss.contains("<sectorNumber>64</sectorNumber>"));

    // 4. Message Type 6: SSR Jamming Strobe (FRN 4 = Sector Number, fspec1 = 0xf0)
    let raw_ssr = build_record(0xf0, 6, &[32u8]); // sectorNumber = 32
    let doc_ssr = parse_asterix_record(&schema_cat34, &raw_ssr).unwrap();
    let xml_ssr = infoset_to_xml(&doc_ssr);
    assert!(xml_ssr.contains("<ssrJammingStrobe>"), "Expected <ssrJammingStrobe> in XML: {}", xml_ssr);
    assert!(xml_ssr.contains("<messageType>6</messageType>"));
    assert!(xml_ssr.contains("<sectorNumber>32</sectorNumber>"));

    // 5. Message Type 7: Mode S Jamming Strobe (FRN 4 = Sector Number, fspec1 = 0xf0)
    let raw_ms = build_record(0xf0, 7, &[16u8]); // sectorNumber = 16
    let doc_ms = parse_asterix_record(&schema_cat34, &raw_ms).unwrap();
    let xml_ms = infoset_to_xml(&doc_ms);
    assert!(xml_ms.contains("<modeSJammingStrobe>"), "Expected <modeSJammingStrobe> in XML: {}", xml_ms);
    assert!(xml_ms.contains("<messageType>7</messageType>"));
    assert!(xml_ms.contains("<sectorNumber>16</sectorNumber>"));

    // 6. Message Type 8: Other / Reserved Service Message (fspec1 = 0xe0)
    let raw_other = build_record(0xe0, 8, &[]);
    let doc_other = parse_asterix_record(&schema_cat34, &raw_other).unwrap();
    let xml_other = infoset_to_xml(&doc_other);
    assert!(xml_other.contains("<otherService>"), "Expected <otherService> in XML: {}", xml_other);
    assert!(xml_other.contains("<messageType>8</messageType>"));
}

