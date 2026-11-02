//! Integration tests for NATO STANAG 5602 (SIMPLE) protocol DFDL parsing and VM execution.

use dfdl_example::{
    compile_control_schema, compile_link16_schema, compile_simple_schema, compile_simple_vm,
    decode_simple_with_vm, infoset_to_xml, parse_simple_packet,
};

fn create_sample_packet_full(
    seq: u32,
    msg_type: u16,
    tdl_type: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(28 + payload.len());
    buf.push(2);                                // Version 2
    buf.push(3);                                // Classification: Secret (3)
    buf.extend_from_slice(&msg_type.to_be_bytes()); // Message Type
    buf.push(tdl_type);                         // TDL Type: 0 = SIMPLE Control, 3 = Link 16
    buf.push(0);                                // Flags: Live (0)
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(&1_700_000_000_000_000_000u64.to_be_bytes()); // Timestamp
    buf.extend_from_slice(&0x0001_0042u32.to_be_bytes()); // Transmitter ID
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // Receiver ID: Broadcast
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn create_sample_packet(seq: u32, tdl_type: u8, payload: &[u8]) -> Vec<u8> {
    create_sample_packet_full(seq, 1, tdl_type, payload)
}

fn create_sample_status_payload() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(3);                                    // terminalState = 3 (Synchronized)
    buf.push(12);                                   // syncQuality = 12
    buf.extend_from_slice(&127u16.to_be_bytes());   // netNumber = 127
    buf.extend_from_slice(&200.0f32.to_be_bytes()); // rfPowerWatts = 200.0 W
    buf.push(3);                                    // cryptoState = 3 (Active)
    buf.extend_from_slice(&43.5f32.to_be_bytes());  // latitude = 43.5
    buf.extend_from_slice(&1.5f32.to_be_bytes());   // longitude = 1.5
    buf.extend_from_slice(&1200.0f32.to_be_bytes()); // altitudeMeters = 1200.0
    buf.extend_from_slice(&0u32.to_be_bytes());     // biteStatus = 0 (Operational)
    buf
}

fn create_sample_time_sync_payload() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1_700_000_000_000_000_000u64.to_be_bytes()); // t1
    buf.extend_from_slice(&1_700_000_000_010_000_000u64.to_be_bytes()); // t2
    buf.extend_from_slice(&1_700_000_000_012_000_000u64.to_be_bytes()); // t3
    buf.extend_from_slice(&1_700_000_000_022_000_000u64.to_be_bytes()); // t4
    buf.extend_from_slice(&20_000u32.to_be_bytes());                     // roundTripDelayNanos
    buf.extend_from_slice(&(-1000i64).to_be_bytes());                    // clockOffsetNanos
    buf
}

#[test]
fn test_compile_simple_schema_all_roots() {
    let roots = ["SimplePacket", "SimpleStream"];
    for root in roots {
        let schema = compile_simple_schema(Some(root))
            .unwrap_or_else(|e| panic!("Failed to compile root '{}': {:?}", root, e));
        assert!(schema.root_element_id.0 > 0);
    }
}

#[test]
fn test_compile_and_parse_standalone_link16_schema() {
    let schema = compile_link16_schema(Some("Link16Message")).expect("compile standalone link16 schema");
    assert!(schema.root_element_id.0 > 0);

    let doc = parse_simple_packet(&schema, dfdl_benchmark::data::TEST_MSG_01_DAT).expect("parse pure link16");
    let xml = infoset_to_xml(&doc);
    assert!(xml.contains("<Link16Message>"));
    assert!(xml.contains("<track>"));
    assert!(xml.contains("<source>AG123</source>"));
    assert!(xml.contains("<speed>200</speed>"));
}

#[test]
fn test_compile_and_parse_standalone_control_schema() {
    let schema = compile_control_schema(Some("SimpleTerminalStatus"))
        .expect("compile standalone control schema");
    assert!(schema.root_element_id.0 > 0);

    let payload = create_sample_status_payload();
    let doc = parse_simple_packet(&schema, &payload).expect("parse pure status");
    let xml = infoset_to_xml(&doc);
    assert!(xml.contains("<SimpleTerminalStatus>"));
    assert!(xml.contains("<terminalState>3</terminalState>"));
    assert!(xml.contains("<netNumber>127</netNumber>"));
    assert!(xml.contains("<rfPowerWatts>200</rfPowerWatts>"));
}

#[test]
fn test_parse_stanag5602_simple_link16_packet() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");

    // Authentic Link 16 J3.2 Air Track Message (64 bytes) encapsulated in STANAG 5602 (tdlType = 3)
    let link16_track_payload = dfdl_benchmark::data::TEST_MSG_01_DAT;
    assert_eq!(link16_track_payload.len(), 64);

    let packet = create_sample_packet(42, 3, link16_track_payload);
    assert_eq!(packet.len(), 28 + 64);

    let doc = parse_simple_packet(&schema, &packet).expect("parse simple packet");
    let xml = infoset_to_xml(&doc);

    // 1. Verify Layer 1 (STANAG 5602 Transport Header)
    assert!(xml.contains("<version>2</version>"));
    assert!(xml.contains("<securityClassification>3</securityClassification>"));
    assert!(xml.contains("<messageType>1</messageType>"));
    assert!(xml.contains("<tdlType>3</tdlType>"));
    assert!(xml.contains("<sequenceNumber>42</sequenceNumber>"));
    assert!(xml.contains("<transmitterId>65602</transmitterId>"));
    assert!(xml.contains("<receiverId>4294967295</receiverId>"));
    assert!(xml.contains("<payloadLength>64</payloadLength>"));

    // 2. Verify Layer 2 (Link 16 J3.2 Air Track Message)
    assert!(xml.contains("<link16>"));
    assert!(xml.contains("<track>"));
    assert!(xml.contains("<source>AG123</source>"));
    assert!(xml.contains("<trackNum>UU777</trackNum>"));
    assert!(xml.contains("<course>75</course>"));
    assert!(xml.contains("<speed>200</speed>"));
    assert!(xml.contains("<pointType>W</pointType>"));
    assert!(xml.contains("<quality>A</quality>"));
}

#[test]
fn test_parse_stanag5602_simple_control_status_packet() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");
    let vm_prog = compile_simple_vm(&schema).expect("compile VM");

    let status_payload = create_sample_status_payload();
    // messageType = 3 (Platform / Terminal Status), tdlType = 0 (SIMPLE Control)
    let packet = create_sample_packet_full(1001, 3, 0, &status_payload);

    let doc = parse_simple_packet(&schema, &packet).expect("parse simple control status");
    let xml = infoset_to_xml(&doc);

    // Verify Layer 1 Header
    assert!(xml.contains("<messageType>3</messageType>"));
    assert!(xml.contains("<tdlType>0</tdlType>"));
    assert!(xml.contains("<sequenceNumber>1001</sequenceNumber>"));

    // Verify Layer 2 Control Status Payload
    assert!(xml.contains("<control>"));
    assert!(xml.contains("<status>"));
    assert!(xml.contains("<terminalState>3</terminalState>"));
    assert!(xml.contains("<syncQuality>12</syncQuality>"));
    assert!(xml.contains("<netNumber>127</netNumber>"));
    assert!(xml.contains("<rfPowerWatts>200</rfPowerWatts>"));
    assert!(xml.contains("<cryptoState>3</cryptoState>"));
    assert!(xml.contains("<latitude>43.5</latitude>"));
    assert!(xml.contains("<longitude>1.5</longitude>"));
    assert!(xml.contains("<altitudeMeters>1200</altitudeMeters>"));
    assert!(xml.contains("<biteStatus>0</biteStatus>"));

    // Verify VM Parity
    let vm_doc = decode_simple_with_vm(&vm_prog, &packet).expect("decode via VM");
    let vm_xml = infoset_to_xml(&vm_doc);
    assert_eq!(xml, vm_xml, "VM output must match ParserEngine output for control status");
}

#[test]
fn test_parse_stanag5602_simple_control_timesync_packet() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");
    let vm_prog = compile_simple_vm(&schema).expect("compile VM");

    let time_sync_payload = create_sample_time_sync_payload();
    // messageType = 2 (Time Synchronization), tdlType = 0 (SIMPLE Control)
    let packet = create_sample_packet_full(1002, 2, 0, &time_sync_payload);

    let doc = parse_simple_packet(&schema, &packet).expect("parse simple control time sync");
    let xml = infoset_to_xml(&doc);

    assert!(xml.contains("<messageType>2</messageType>"));
    assert!(xml.contains("<tdlType>0</tdlType>"));
    assert!(xml.contains("<control>"));
    assert!(xml.contains("<timeSync>"));
    assert!(xml.contains("<t1TransmitTime>1700000000000000000</t1TransmitTime>"));
    assert!(xml.contains("<roundTripDelayNanos>20000</roundTripDelayNanos>"));
    assert!(xml.contains("<clockOffsetNanos>-1000</clockOffsetNanos>"));

    // Verify VM Parity
    let vm_doc = decode_simple_with_vm(&vm_prog, &packet).expect("decode via VM");
    let vm_xml = infoset_to_xml(&vm_doc);
    assert_eq!(xml, vm_xml, "VM output must match ParserEngine output for time sync");
}

#[test]
fn test_stanag5602_simple_vm_execution() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");
    let vm_prog = compile_simple_vm(&schema).expect("compile VM");

    // Link 16 J3.2 track message (tdlType = 3)
    let payload = dfdl_benchmark::data::TEST_MSG_01_DAT;
    let packet = create_sample_packet(101, 3, payload);

    let parser_doc = parse_simple_packet(&schema, &packet).expect("parse via engine");
    let vm_doc = decode_simple_with_vm(&vm_prog, &packet).expect("decode via VM");

    let parser_xml = infoset_to_xml(&parser_doc);
    let vm_xml = infoset_to_xml(&vm_doc);

    assert_eq!(parser_xml, vm_xml, "VM output must match ParserEngine output for Link 16 packet");
    assert!(parser_xml.contains("<tdlType>3</tdlType>"));
    assert!(parser_xml.contains("<source>AG123</source>"));
}

#[test]
fn test_stanag5602_simple_stream_parsing() {
    let schema = compile_simple_schema(Some("SimpleStream")).expect("compile stream schema");

    // Packet 1: SIMPLE Native Status Keep-Alive (tdlType = 0, msgType = 3)
    let p1 = create_sample_packet_full(1, 3, 0, &create_sample_status_payload());
    // Packet 2: Link 16 J3.2 Track message (tdlType = 3, msgType = 1)
    let p2 = create_sample_packet_full(2, 1, 3, dfdl_benchmark::data::TEST_MSG_01_DAT);

    let mut stream_bytes = Vec::new();
    stream_bytes.extend_from_slice(&p1);
    stream_bytes.extend_from_slice(&p2);

    let doc = parse_simple_packet(&schema, &stream_bytes).expect("parse stream");
    let xml = infoset_to_xml(&doc);

    assert!(xml.contains("<sequenceNumber>1</sequenceNumber>"));
    assert!(xml.contains("<terminalState>3</terminalState>"));
    assert!(xml.contains("<sequenceNumber>2</sequenceNumber>"));
    assert!(xml.contains("<source>AG123</source>"));
}

#[test]
fn test_stanag5602_simple_reject_unsupported_tdl() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");
    let payload = [0xDE, 0xAD, 0xBE, 0xEF];

    // Test rejection with tdlType = 1 (Link 11A - unsupported in this profile)
    let packet_tdl_1 = create_sample_packet(102, 1, &payload);
    let result_1 = parse_simple_packet(&schema, &packet_tdl_1);
    assert!(
        result_1.is_err(),
        "STANAG 5602 packet with tdlType = 1 must be rejected"
    );

    // Test rejection with tdlType = 4 (Link 22 - unsupported in this profile)
    let packet_tdl_4 = create_sample_packet(103, 4, &payload);
    let result_4 = parse_simple_packet(&schema, &packet_tdl_4);
    assert!(
        result_4.is_err(),
        "STANAG 5602 packet with tdlType = 4 must be rejected"
    );
}

#[test]
fn test_stanag5602_simple_facet_restrictions() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");

    let mut checked_fields = 0;
    for term in &schema.terms {
        match term.name.local_name.as_str() {
            "version" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("1"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("2"));
                checked_fields += 1;
            }
            "securityClassification" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("4"));
                checked_fields += 1;
            }
            "messageType" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("1"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("5"));
                checked_fields += 1;
            }
            "tdlType" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("3"));
                checked_fields += 1;
            }
            "flags" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("7"));
                checked_fields += 1;
            }
            "payloadLength" => {
                assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65507"));
                checked_fields += 1;
            }
            _ => {}
        }
    }
    assert_eq!(checked_fields, 6, "All 6 strict facet fields must be present and verified");
}

#[test]
fn test_stanag5602_simple_payload_length_padding() {
    let schema = compile_simple_schema(Some("SimplePacket")).expect("compile schema");
    let vm_prog = compile_simple_vm(&schema).expect("compile VM");

    // Status payload is 25 bytes. We declare payloadLength = 32 in the header,
    // appending 7 bytes of zero padding at the end of the payload.
    let mut padded_payload = create_sample_status_payload();
    assert_eq!(padded_payload.len(), 25);
    padded_payload.extend_from_slice(&[0xAA; 7]); // 7 bytes of trailing padding
    assert_eq!(padded_payload.len(), 32);

    let packet = create_sample_packet_full(2001, 3, 0, &padded_payload);
    assert_eq!(packet.len(), 28 + 32);

    let parser_doc = parse_simple_packet(&schema, &packet).expect("parse via engine with explicit length padding");
    let vm_doc = decode_simple_with_vm(&vm_prog, &packet).expect("decode via VM with PadToRegLength");

    let parser_xml = infoset_to_xml(&parser_doc);
    let vm_xml = infoset_to_xml(&vm_doc);

    assert_eq!(parser_xml, vm_xml, "VM output must match ParserEngine output for padded payload");
    assert!(parser_xml.contains("<payloadLength>32</payloadLength>"));
    assert!(parser_xml.contains("<terminalState>3</terminalState>"));
    assert!(parser_xml.contains("<netNumber>127</netNumber>"));
}


