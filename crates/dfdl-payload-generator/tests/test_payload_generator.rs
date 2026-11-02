//! Integration tests for `dfdl-payload-generator`.
//!
//! Tests exhaustiveness generation, syntactic mutations, semantic mutations,
//! and standard PCAP Ethernet/IPv4/UDP packet encapsulation.

use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::io::{BitOrder, BitReader, ByteOrder, SliceByteSource};
use dfdl_core::kernel::ParserEngine;
use dfdl_example::{compile_control_schema, compile_simple_schema};
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, PayloadMode, PcapFile, SemanticAnomaly,
    SyntaxAnomaly, TransportProtocol,
};

#[test]
fn test_valid_payload_generation_simple_control() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 42);

    let payload = generator
        .generate_valid()
        .expect("generate valid simple control payload");

    assert!(!payload.bytes.is_empty(), "Generated payload must not be empty");
    assert!(
        payload.description.contains("Valid"),
        "Payload must be described as valid"
    );

    // Verify parsing of the generated payload
    let source = SliceByteSource::new(payload.bytes.as_slice());
    let mut reader = BitReader::new(
        source,
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut budget = dfdl_core::WorkBudget::new(100_000);
    let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
    let parse_res = parser.parse_document();
    assert!(
        parse_res.is_ok(),
        "Generated valid payload must parse cleanly: {:?}",
        parse_res.err()
    );
}

#[test]
fn test_valid_payload_generation_stanag5602_simple() {
    let schema = compile_simple_schema(Some("SimplePacket"))
        .expect("compile stanag5602 simple packet schema");
    let mut generator = PayloadGenerator::new(&schema, 12345);

    let payload = generator
        .generate_valid()
        .expect("generate valid simple packet");
    assert!(!payload.bytes.is_empty());
    assert_eq!(payload.mode, PayloadMode::Valid);
}

#[test]
fn test_syntactic_anomalies_truncation() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 777);

    let valid_payload = generator.generate_valid().expect("valid payload");
    assert!(valid_payload.bytes.len() > 4);

    let cut_bytes = 4;
    let anomaly = generator
        .generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes })
        .expect("generate truncated syntax anomaly");

    assert_eq!(anomaly.bytes.len(), valid_payload.bytes.len().saturating_sub(cut_bytes));
    assert_eq!(
        anomaly.mode,
        PayloadMode::SyntaxAnomaly(SyntaxAnomaly::Truncated { cut_bytes })
    );

    // Parsing truncated payload must fail with premature EOF or unexpected end
    let source = SliceByteSource::new(anomaly.bytes.as_slice());
    let mut reader = BitReader::new(
        source,
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut budget = dfdl_core::WorkBudget::new(100_000);
    let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
    let res = parser.parse_document();
    assert!(res.is_err(), "Truncated payload should fail parsing");
}

#[test]
fn test_syntactic_anomalies_trailing_garbage() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 888);

    let valid_len = generator.generate_valid().expect("valid").bytes.len();
    let anomaly = generator
        .generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 6 })
        .expect("generate trailing garbage anomaly");

    assert_eq!(anomaly.bytes.len(), valid_len + 6);
    assert_eq!(
        anomaly.mode,
        PayloadMode::SyntaxAnomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 6 })
    );
}

#[test]
fn test_syntactic_anomalies_declared_length_mismatch() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 999);

    let anomaly = generator
        .generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
            field_offset: 0,
            field_width: 2,
            delta: 50,
        })
        .expect("generate length mismatch anomaly");

    assert!(!anomaly.bytes.is_empty());
}

#[test]
fn test_syntactic_anomalies_bit_misalignment() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 101);

    let anomaly = generator
        .generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 })
        .expect("generate bit misaligned anomaly");

    assert!(!anomaly.bytes.is_empty());
}

#[test]
fn test_semantic_anomalies_wrong_enum() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 202);

    let enums = generator.analysis().enum_fields.clone();
    if let Some(target) = enums.first() {
        let anomaly_def = SemanticAnomaly::WrongEnum {
            target_name: target.name.clone(),
            invalid_value: "999".to_string(),
        };
        let anomaly = generator
            .generate_semantic_anomaly(anomaly_def.clone())
            .expect("generate wrong enum anomaly");

        assert!(!anomaly.bytes.is_empty());
        assert_eq!(
            anomaly.mode,
            PayloadMode::SemanticAnomaly(anomaly_def)
        );
    }
}

#[test]
fn test_semantic_anomalies_facet_out_of_bounds() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 303);

    let bounds = generator.analysis().bounded_fields.clone();
    if let Some(target) = bounds.first() {
        let anomaly_def = SemanticAnomaly::FacetOutOfBounds {
            target_name: target.name.clone(),
            invalid_value: DfdlValue::UnsignedByte(255),
        };
        let anomaly = generator
            .generate_semantic_anomaly(anomaly_def.clone())
            .expect("generate facet out of bounds anomaly");

        assert!(!anomaly.bytes.is_empty());
        assert_eq!(
            anomaly.mode,
            PayloadMode::SemanticAnomaly(anomaly_def)
        );
    }
}

#[test]
fn test_semantic_anomalies_wrong_dispatch_key() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 404);

    let anomaly_def = SemanticAnomaly::WrongDispatchKey {
        discriminator_name: "messageType".to_string(),
        invalid_key: DfdlValue::UnsignedShort(9999),
    };
    let anomaly = generator
        .generate_semantic_anomaly(anomaly_def.clone())
        .expect("generate wrong dispatch key anomaly");

    assert!(!anomaly.bytes.is_empty());
    assert_eq!(
        anomaly.mode,
        PayloadMode::SemanticAnomaly(anomaly_def)
    );
}

#[test]
fn test_exhaustive_suite_coverage() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 505);

    let suite = generator
        .generate_exhaustive_suite()
        .expect("generate exhaustive test suite");

    assert!(!suite.is_empty(), "Exhaustive suite must contain payloads");

    let has_valid = suite.iter().any(|p| matches!(p.mode, PayloadMode::Valid));
    let has_syntax = suite
        .iter()
        .any(|p| matches!(p.mode, PayloadMode::SyntaxAnomaly(_)));
    let has_semantic = suite
        .iter()
        .any(|p| matches!(p.mode, PayloadMode::SemanticAnomaly(_)));

    assert!(has_valid, "Suite must contain valid payloads");
    assert!(has_syntax, "Suite must contain syntactic invalid payloads");
    assert!(has_semantic, "Suite must contain semantic invalid payloads");
}

#[test]
fn test_pcap_serialization_and_parsing() {
    let pcap_writer = dfdl_payload_generator::PcapWriter::default();
    let net_cfg = NetworkConfig::default();
    let transport = TransportProtocol::default();

    let payload1 = b"HELLO_DFDL_WORLD";
    let payload2 = b"ANOTHER_PACKET_ANOMALY";

    let mut raw_pcap = Vec::new();
    pcap_writer.write_header(&mut raw_pcap).expect("write header");
    pcap_writer
        .write_encapsulated_packet(&mut raw_pcap, 100, 0, &net_cfg, &transport, payload1)
        .expect("write p1");
    pcap_writer
        .write_encapsulated_packet(&mut raw_pcap, 100, 10_000, &net_cfg, &transport, payload2)
        .expect("write p2");

    assert!(!raw_pcap.is_empty());

    // Parse back the PCAP
    let parsed = PcapFile::parse(&raw_pcap).expect("parse PCAP bytes");
    assert_eq!(parsed.packets.len(), 2);

    // Verify PCAP global header
    assert_eq!(parsed.magic, 0xa1b2c3d4);
    assert_eq!(parsed.version_major, 2);
    assert_eq!(parsed.version_minor, 4);
    assert_eq!(parsed.link_type, 1); // LINKTYPE_ETHERNET

    // Verify packet 1
    let p1 = &parsed.packets[0];
    assert_eq!(p1.data.len(), 14 + 20 + 8 + payload1.len());
    let udp_payload1 = p1.extract_udp_payload().expect("extract UDP payload 1");
    assert_eq!(udp_payload1, payload1);

    // Verify packet 2
    let p2 = &parsed.packets[1];
    let udp_payload2 = p2.extract_udp_payload().expect("extract UDP payload 2");
    assert_eq!(udp_payload2, payload2);
}

#[test]
fn test_export_exhaustive_suite_to_pcap() {
    let schema = compile_control_schema(Some("SimpleControlPayload"))
        .expect("compile stanag5602 control schema");
    let mut generator = PayloadGenerator::new(&schema, 606);

    let suite = generator
        .generate_exhaustive_suite()
        .expect("generate exhaustive test suite");

    let net_cfg = NetworkConfig::default();
    let transport = TransportProtocol::default();

    let pcap_bytes = generator
        .export_pcap_bytes(&suite, &net_cfg, &transport)
        .expect("export suite to PCAP");

    let parsed = PcapFile::parse(&pcap_bytes).expect("parse exported PCAP");
    assert_eq!(parsed.packets.len(), suite.len());

    for (i, pcap_pkt) in parsed.packets.iter().enumerate() {
        let extracted = pcap_pkt.extract_udp_payload().expect("extract UDP payload");
        assert_eq!(extracted, suite[i].bytes);
    }
}
