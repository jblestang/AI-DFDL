//! Standalone test runner for dfdl-payload-generator verifying all test cases.

use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::io::{BitOrder, BitReader, ByteOrder, SliceByteSource};
use dfdl_core::kernel::ParserEngine;
use dfdl_example::{compile_control_schema, compile_simple_schema};
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, PayloadMode, PcapFile, SemanticAnomaly,
    SyntaxAnomaly, TransportProtocol,
};

fn main() -> Result<(), String> {
    println!("Running dfdl-payload-generator test suite...");

    // Test 1: Valid payload generation simple control
    print!("test test_valid_payload_generation_simple_control ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 42);
        let payload = generator
            .generate_valid()
            .map_err(|e| format!("{:?}", e))?;
        assert!(!payload.bytes.is_empty());
        assert!(payload.description.contains("Valid"));

        let source = SliceByteSource::new(payload.bytes.as_slice());
        let mut reader = BitReader::new(
            source,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget = dfdl_core::WorkBudget::new(100_000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let parse_res = parser.parse_document();
        assert!(parse_res.is_ok(), "Parse error: {:?}", parse_res.err());
    }
    println!("ok");

    // Test 2: Valid payload generation stanag5602 simple
    print!("test test_valid_payload_generation_stanag5602_simple ... ");
    {
        let schema = compile_simple_schema(Some("SimplePacket"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 12345);
        let payload = generator
            .generate_valid()
            .map_err(|e| format!("{:?}", e))?;
        assert!(!payload.bytes.is_empty());
        assert_eq!(payload.mode, PayloadMode::Valid);
    }
    println!("ok");

    // Test 3: Syntactic anomaly - truncation
    print!("test test_syntactic_anomalies_truncation ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 777);
        let valid_payload = generator.generate_valid().map_err(|e| format!("{:?}", e))?;
        assert!(valid_payload.bytes.len() > 4);

        let cut_bytes = 4;
        let anomaly = generator
            .generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes })
            .map_err(|e| format!("{:?}", e))?;
        assert_eq!(
            anomaly.bytes.len(),
            valid_payload.bytes.len().saturating_sub(cut_bytes)
        );

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
    println!("ok");

    // Test 4: Syntactic anomaly - trailing garbage
    print!("test test_syntactic_anomalies_trailing_garbage ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 888);
        let valid_len = generator
            .generate_valid()
            .map_err(|e| format!("{:?}", e))?
            .bytes
            .len();
        let anomaly = generator
            .generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 6 })
            .map_err(|e| format!("{:?}", e))?;
        assert_eq!(anomaly.bytes.len(), valid_len + 6);
    }
    println!("ok");

    // Test 5: Syntactic anomaly - declared length mismatch
    print!("test test_syntactic_anomalies_declared_length_mismatch ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 999);
        let anomaly = generator
            .generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
                field_offset: 0,
                field_width: 2,
                delta: 50,
            })
            .map_err(|e| format!("{:?}", e))?;
        assert!(!anomaly.bytes.is_empty());
    }
    println!("ok");

    // Test 6: Syntactic anomaly - bit misalignment
    print!("test test_syntactic_anomalies_bit_misalignment ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 101);
        let anomaly = generator
            .generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 })
            .map_err(|e| format!("{:?}", e))?;
        assert!(!anomaly.bytes.is_empty());
    }
    println!("ok");

    // Test 7: Semantic anomaly - wrong enum
    print!("test test_semantic_anomalies_wrong_enum ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 202);
        let enums = generator.analysis().enum_fields.clone();
        if let Some(target) = enums.first() {
            let anomaly_def = SemanticAnomaly::WrongEnum {
                target_name: target.name.clone(),
                invalid_value: "999".to_string(),
            };
            let anomaly = generator
                .generate_semantic_anomaly(anomaly_def.clone())
                .map_err(|e| format!("{:?}", e))?;
            assert!(!anomaly.bytes.is_empty());
            assert_eq!(anomaly.mode, PayloadMode::SemanticAnomaly(anomaly_def));
        }
    }
    println!("ok");

    // Test 8: Semantic anomaly - facet out of bounds
    print!("test test_semantic_anomalies_facet_out_of_bounds ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 303);
        let bounds = generator.analysis().bounded_fields.clone();
        if let Some(target) = bounds.first() {
            let anomaly_def = SemanticAnomaly::FacetOutOfBounds {
                target_name: target.name.clone(),
                invalid_value: DfdlValue::UnsignedByte(250),
            };
            let anomaly = generator
                .generate_semantic_anomaly(anomaly_def.clone())
                .map_err(|e| format!("{:?}", e))?;
            assert!(!anomaly.bytes.is_empty());
            assert_eq!(anomaly.mode, PayloadMode::SemanticAnomaly(anomaly_def));
        }
    }
    println!("ok");

    // Test 9: Semantic anomaly - wrong dispatch key
    print!("test test_semantic_anomalies_wrong_dispatch_key ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 404);
        let disc_name = generator
            .analysis()
            .find_choice_discriminators()
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| {
                generator
                    .analysis()
                    .all_elements
                    .first()
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| "element".to_string())
            });

        let anomaly_def = SemanticAnomaly::WrongDispatchKey {
            discriminator_name: disc_name,
            invalid_key: DfdlValue::UnsignedShort(9999),
        };
        let anomaly = generator
            .generate_semantic_anomaly(anomaly_def.clone())
            .map_err(|e| format!("{:?}", e))?;
        assert!(!anomaly.bytes.is_empty());
        assert_eq!(anomaly.mode, PayloadMode::SemanticAnomaly(anomaly_def));
    }
    println!("ok");

    // Test 10: Exhaustive suite coverage
    print!("test test_exhaustive_suite_coverage ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 505);
        let suite = generator
            .generate_exhaustive_suite()
            .map_err(|e| format!("{:?}", e))?;
        assert!(!suite.is_empty());

        let has_valid = suite.iter().any(|p| matches!(p.mode, PayloadMode::Valid));
        let has_syntax = suite
            .iter()
            .any(|p| matches!(p.mode, PayloadMode::SyntaxAnomaly(_)));
        let has_semantic = suite
            .iter()
            .any(|p| matches!(p.mode, PayloadMode::SemanticAnomaly(_)));
        assert!(has_valid);
        assert!(has_syntax);
        assert!(has_semantic);
    }
    println!("ok");

    // Test 11: PCAP serialization and parsing
    print!("test test_pcap_serialization_and_parsing ... ");
    {
        let pcap_writer = dfdl_payload_generator::PcapWriter::default();
        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();

        let payload1 = b"HELLO_DFDL_WORLD";
        let payload2 = b"ANOTHER_PACKET_ANOMALY";

        let mut raw_pcap = Vec::new();
        pcap_writer.write_header(&mut raw_pcap).map_err(|e| e.to_string())?;
        pcap_writer
            .write_encapsulated_packet(&mut raw_pcap, 100, 0, &net_cfg, &transport, payload1)
            .map_err(|e| e.to_string())?;
        pcap_writer
            .write_encapsulated_packet(&mut raw_pcap, 100, 10_000, &net_cfg, &transport, payload2)
            .map_err(|e| e.to_string())?;

        assert!(!raw_pcap.is_empty());

        let parsed = PcapFile::parse(&raw_pcap).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), 2);
        assert_eq!(parsed.magic, 0xa1b2c3d4);
        assert_eq!(parsed.version_major, 2);
        assert_eq!(parsed.version_minor, 4);
        assert_eq!(parsed.link_type, 1);

        let p1 = &parsed.packets[0];
        assert_eq!(p1.data.len(), 14 + 20 + 8 + payload1.len());
        let udp_p1 = p1.extract_udp_payload().expect("extract UDP payload 1");
        assert_eq!(udp_p1, payload1);

        let p2 = &parsed.packets[1];
        let udp_p2 = p2.extract_udp_payload().expect("extract UDP payload 2");
        assert_eq!(udp_p2, payload2);
    }
    println!("ok");

    // Test 12: Export exhaustive suite to PCAP
    print!("test test_export_exhaustive_suite_to_pcap ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 606);
        let suite = generator
            .generate_exhaustive_suite()
            .map_err(|e| format!("{:?}", e))?;

        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();

        let pcap_bytes = generator
            .export_pcap_bytes(&suite, &net_cfg, &transport)
            .map_err(|e| e.to_string())?;
        let parsed = PcapFile::parse(&pcap_bytes).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), suite.len());

        for (i, pcap_pkt) in parsed.packets.iter().enumerate() {
            let extracted = pcap_pkt.extract_udp_payload().expect("extract UDP payload");
            assert_eq!(extracted, suite[i].bytes);
        }
    }
    println!("ok");

    // Test 13: Generate valid suite
    print!("test test_generate_valid_suite ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 808);
        let valid_suite = generator
            .generate_valid_suite()
            .map_err(|e| format!("{:?}", e))?;

        assert!(!valid_suite.is_empty());
        assert!(valid_suite.iter().all(|p| matches!(p.mode, PayloadMode::Valid)));

        for (i, p) in valid_suite.iter().enumerate() {
            let source = SliceByteSource::new(p.bytes.as_slice());
            let mut reader = BitReader::new(
                source,
                BitOrder::MostSignificantBitFirst,
                ByteOrder::BigEndian,
            );
            let mut budget = dfdl_core::WorkBudget::new(100_000);
            let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
            let parse_res = parser.parse_document();
            assert!(parse_res.is_ok(), "Payload #{} ({}) parse error: {:?}", i, p.name, parse_res.err());
        }
    }
    println!("ok");

    // Test 14: Export valid PCAP bytes directly
    print!("test test_export_valid_pcap_bytes ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 909);
        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();

        let pcap_bytes = generator
            .export_valid_pcap_bytes(5, &net_cfg, &transport)
            .map_err(|e| format!("{:?}", e))?;
        let parsed = PcapFile::parse(&pcap_bytes).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), 5);

        for (i, pkt) in parsed.packets.iter().enumerate() {
            let udp_data = pkt.extract_udp_payload().expect("extract UDP payload");
            let source = SliceByteSource::new(udp_data);
            let mut reader = BitReader::new(
                source,
                BitOrder::MostSignificantBitFirst,
                ByteOrder::BigEndian,
            );
            let mut budget = dfdl_core::WorkBudget::new(100_000);
            let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
            assert!(parser.parse_document().is_ok(), "Packet #{} parse error", i);
        }
    }
    println!("ok");

    // Test 15: Generation limits on message count
    print!("test test_generation_limits_message_count ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 1001)
            .with_max_messages(3);

        let seq = generator.generate_valid_sequence(10).map_err(|e| format!("{:?}", e))?;
        assert_eq!(seq.len(), 3, "Message count must be strictly capped at 3");

        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();
        let mut pcap_buf = Vec::new();
        let report = generator
            .write_valid_pcap(&mut pcap_buf, 10, &net_cfg, &transport)
            .map_err(|e| format!("{:?}", e))?;

        assert_eq!(report.message_count, 3);
        assert!(report.reached_message_limit);

        let parsed = PcapFile::parse(&pcap_buf).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), 3);
    }
    println!("ok");

    // Test 16: Generation limits on max total size
    print!("test test_generation_limits_max_total_size ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let max_budget = 250;
        let mut generator = PayloadGenerator::new(&schema, 1002)
            .with_max_total_size(max_budget);

        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();
        let mut pcap_buf = Vec::new();
        let report = generator
            .write_valid_pcap(&mut pcap_buf, 100, &net_cfg, &transport)
            .map_err(|e| format!("{:?}", e))?;

        assert!(pcap_buf.len() <= max_budget, "PCAP size {} must be <= max budget {}", pcap_buf.len(), max_budget);
        assert!(report.total_bytes <= max_budget);
        assert!(report.reached_size_limit);
        assert!(report.message_count > 0, "At least one packet should fit within 250 bytes");

        let parsed = PcapFile::parse(&pcap_buf).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), report.message_count);
    }
    println!("ok");

    // Test 17: Generation limits on max payload size
    print!("test test_generation_limits_max_payload_size ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let max_len = 12;
        let mut generator = PayloadGenerator::new(&schema, 1003)
            .with_max_payload_size(max_len);

        let payload = generator.generate_valid().map_err(|e| format!("{:?}", e))?;
        assert!(
            payload.bytes.len() <= max_len,
            "Payload length {} must be <= {}",
            payload.bytes.len(),
            max_len
        );
    }
    println!("ok");

    // Test 18: Generation limits when budget is smaller than single packet
    print!("test test_generation_limits_budget_smaller_than_single_packet ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 1004)
            .with_max_total_size(50);

        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();
        let mut pcap_buf = Vec::new();
        let report = generator
            .write_exhaustive_pcap(&mut pcap_buf, 50, &net_cfg, &transport)
            .map_err(|e| format!("{:?}", e))?;

        assert_eq!(report.message_count, 0);
        assert!(report.reached_size_limit);
        assert!(!report.reached_message_limit);
        assert_eq!(report.total_bytes, 24);
        assert_eq!(pcap_buf.len(), 24);
    }
    println!("ok");

    // Test 19: Write exhaustive PCAP with packet count cycling
    print!("test test_write_exhaustive_pcap_cycling ... ");
    {
        let schema = compile_control_schema(Some("SimpleControlPayload"))
            .map_err(|e| format!("{:?}", e))?;
        let mut generator = PayloadGenerator::new(&schema, 1005);
        let net_cfg = NetworkConfig::default();
        let transport = TransportProtocol::default();
        let mut pcap_buf = Vec::new();
        let report = generator
            .write_exhaustive_pcap(&mut pcap_buf, 50, &net_cfg, &transport)
            .map_err(|e| format!("{:?}", e))?;

        assert_eq!(report.message_count, 50);
        assert!(report.reached_message_limit);
        assert!(!report.reached_size_limit);

        let parsed = PcapFile::parse(&pcap_buf).map_err(|e| e.to_string())?;
        assert_eq!(parsed.packets.len(), 50);
    }
    println!("ok");

    println!("\nAll 19 tests passed successfully!");
    Ok(())
}


