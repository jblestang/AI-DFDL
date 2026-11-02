//! Example: Generating a comprehensive PCAP capture for NATO STANAG 5602 (SIMPLE) protocol
//! containing valid frames, syntactic anomalies, and semantic anomalies.
//!
//! Run with:
//! `cargo run -p dfdl-payload-generator --example generate_stanag5602_pcap`

use dfdl_core::infoset::value::DfdlValue;
use dfdl_example::compile_simple_schema;
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, PayloadMode, PcapFile, SemanticAnomaly,
    SyntaxAnomaly, TransportProtocol,
};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("==================================================================");
    println!(" DFDL Payload & PCAP Generator: NATO STANAG 5602 (SIMPLE) Suite  ");
    println!("==================================================================\n");

    // 1. Compile schema
    println!("[1/5] Compiling STANAG 5602 SIMPLE DFDL schema...");
    let schema = compile_simple_schema(Some("SimplePacket"))
        .map_err(|e| format!("{:?}", e))?;
    println!("      Schema compiled successfully (root element ID: {:?})", schema.root_element_id);

    // 2. Initialize generator
    let seed = 42;
    let mut generator = PayloadGenerator::new(&schema, seed);

    println!("\n[2/5] Inspecting schema structure for exhaustiveness coverage:");
    println!("      - Discovered {} choice branches", generator.analysis().choice_branches.len());
    for branch in &generator.analysis().choice_branches {
        println!(
            "        * Choice branch index {} (key: {:?})",
            branch.branch_index, branch.branch_key
        );
    }
    println!("      - Discovered {} enumeration fields", generator.analysis().enum_fields.len());
    for en in &generator.analysis().enum_fields {
        println!(
            "        * Enum '{}': allowed values = {:?}",
            en.name, en.enumerations
        );
    }
    println!("      - Discovered {} bounded fields", generator.analysis().bounded_fields.len());
    println!("      - Discovered {} explicit length fields", generator.analysis().explicit_length_fields.len());

    // 3. Generate test suite
    println!("\n[3/5] Generating test payloads (valid + syntactic anomalies + semantic anomalies)...");
    let mut suite = Vec::new();

    // Valid payloads
    let valid1 = generator.generate_valid().map_err(|e| format!("{:?}", e))?;
    suite.push(valid1);

    // Syntactic anomalies
    let truncated = generator.generate_syntax_anomaly(SyntaxAnomaly::Truncated {
        cut_bytes: 4,
    }).map_err(|e| format!("{:?}", e))?;
    suite.push(truncated);

    let garbage = generator.generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage {
        garbage_bytes: 4,
    }).map_err(|e| format!("{:?}", e))?;
    suite.push(garbage);

    let length_mismatch = generator.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
        field_offset: 26, // SIMPLE header payloadLength field offset
        field_width: 2,
        delta: 20,
    }).map_err(|e| format!("{:?}", e))?;
    suite.push(length_mismatch);

    let bit_misaligned = generator.generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned {
        extra_bits: 5,
    }).map_err(|e| format!("{:?}", e))?;
    suite.push(bit_misaligned);

    // Semantic anomalies
    if let Some(en) = generator.analysis().enum_fields.first() {
        let wrong_enum = generator.generate_semantic_anomaly(SemanticAnomaly::WrongEnum {
            target_name: en.name.clone(),
            invalid_value: "99".to_string(), // Invalid classification/TDLType
        }).map_err(|e| format!("{:?}", e))?;
        suite.push(wrong_enum);
    }

    if let Some(bound) = generator.analysis().bounded_fields.first() {
        let out_of_bounds = generator.generate_semantic_anomaly(SemanticAnomaly::FacetOutOfBounds {
            target_name: bound.name.clone(),
            invalid_value: DfdlValue::UnsignedByte(250),
        }).map_err(|e| format!("{:?}", e))?;
        suite.push(out_of_bounds);
    }

    let wrong_dispatch = generator.generate_semantic_anomaly(SemanticAnomaly::WrongDispatchKey {
        discriminator_name: "tdlType".to_string(),
        invalid_key: DfdlValue::UnsignedByte(99),
    }).map_err(|e| format!("{:?}", e))?;
    suite.push(wrong_dispatch);

    // 4. Summarize generated suite
    println!("\n[4/5] Generated payload summary:");
    println!("--------------------------------------------------------------------------------------");
    println!("{:<4} | {:<25} | {:<8} | {}", "#", "Category / Type", "Length", "Description");
    println!("--------------------------------------------------------------------------------------");
    for (i, p) in suite.iter().enumerate() {
        let cat = match &p.mode {
            PayloadMode::Valid => "VALID".to_string(),
            PayloadMode::SyntaxAnomaly(s) => format!("SYNTAX ({:?})", s),
            PayloadMode::SemanticAnomaly(s) => format!("SEMANTIC ({:?})", s),
        };
        let cat_short = if cat.len() > 25 {
            format!("{}...", &cat[..22])
        } else {
            cat
        };
        println!("{:<4} | {:<25} | {:<8} | {}", i + 1, cat_short, p.bytes.len(), p.description);
    }
    println!("--------------------------------------------------------------------------------------");

    // 5. Export to PCAP file
    println!("\n[5/5] Exporting payloads into standard PCAP capture...");
    let pcap_filename = "stanag5602_test_suite.pcap";
    let net_cfg = NetworkConfig {
        src_mac: [0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E],
        dst_mac: [0x00, 0x50, 0x56, 0xC0, 0x00, 0x01],
        src_ip: [192, 168, 1, 10],
        dst_ip: [192, 168, 1, 20],
    };
    let transport = TransportProtocol::Udp {
        src_port: 5602,
        dst_port: 5602,
    };

    let mut pcap_file = fs::File::create(pcap_filename)?;
    generator.write_pcap(&mut pcap_file, &suite, &net_cfg, &transport)?;
    let pcap_data = fs::read(pcap_filename)?;
    println!(
        "      PCAP written successfully to '{}' ({} bytes total)",
        pcap_filename,
        pcap_data.len()
    );

    // Verify PCAP with parser
    let parsed_pcap = PcapFile::parse(&pcap_data)?;
    println!(
        "      Verified PCAP integrity: parsed {} packets with magic {:#010x}",
        parsed_pcap.packets.len(),
        parsed_pcap.magic
    );

    println!("\nDone! The PCAP can now be analyzed with Wireshark and DFDL dissector plugins.");
    Ok(())
}
