//! Example: Generating a comprehensive PCAP capture for NATO STANAG 5602 (SIMPLE) protocol
//! containing valid frames, syntactic anomalies, and semantic anomalies.
//!
//! Run with:
//! `cargo run -p dfdl-payload-generator --example generate_stanag5602_pcap`

use dfdl_example::compile_simple_schema;
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, PayloadMode, PcapFile, TransportProtocol,
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
    let suite = generator.generate_exhaustive_suite()
        .map_err(|e| format!("{:?}", e))?;

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

    // 5. Export exclusively VALID PCAP capture
    println!("\n[5/6] Exporting exclusively VALID PCAP capture (10 valid packets)...");
    let valid_pcap_filename = "stanag5602_valid.pcap";
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

    let mut valid_pcap_file = fs::File::create(valid_pcap_filename)?;
    generator.write_valid_pcap(&mut valid_pcap_file, 10, &net_cfg, &transport)
        .map_err(|e| format!("{:?}", e))?;
    let valid_pcap_data = fs::read(valid_pcap_filename)?;
    println!(
        "      Valid PCAP written successfully to '{}' ({} bytes total)",
        valid_pcap_filename,
        valid_pcap_data.len()
    );

    let parsed_valid_pcap = PcapFile::parse(&valid_pcap_data)?;
    println!(
        "      Verified Valid PCAP integrity: parsed {} valid packets",
        parsed_valid_pcap.packets.len()
    );

    // 6. Export full test suite PCAP file (valid + syntactic + semantic anomalies)
    println!("\n[6/6] Exporting full anomaly test suite PCAP capture...");
    let pcap_filename = "stanag5602_test_suite.pcap";
    let mut pcap_file = fs::File::create(pcap_filename)?;
    generator.write_pcap(&mut pcap_file, &suite, &net_cfg, &transport)?;
    let pcap_data = fs::read(pcap_filename)?;
    println!(
        "      Anomaly PCAP written successfully to '{}' ({} bytes total)",
        pcap_filename,
        pcap_data.len()
    );

    // Verify PCAP with parser
    let parsed_pcap = PcapFile::parse(&pcap_data)?;
    println!(
        "      Verified Anomaly PCAP integrity: parsed {} packets with magic {:#010x}",
        parsed_pcap.packets.len(),
        parsed_pcap.magic
    );

    println!("\nDone! Valid and anomaly PCAPs are ready for Wireshark and DFDL dissector verification.");
    Ok(())
}
