//! Example: Validating ASTERIX network captures against DFDL schema.
//!
//! Demonstrates end-to-end PCAP extraction, UDP payload decapsulation, and
//! content validation of Eurocontrol ASTERIX data blocks using [`dfdl_core::kernel::parser::ParserEngine`].

use dfdl_example::pcap;
use dfdl_example::{
    compile_asterix_schema, parse_asterix_record, ExampleError, SAMPLE_CAT_034_048_PCAP,
};
use std::env;
use std::fs;

fn main() -> Result<(), ExampleError> {
    println!("=== ASTERIX PCAP Validation against DFDL Schema ===");

    // Allow user to supply custom PCAP path via CLI or use embedded sample
    let pcap_data = match env::args().nth(1) {
        Some(path) => {
            println!("Loading custom PCAP from: {path}");
            fs::read(path)?
        }
        None => {
            println!("Using authentic CroatiaControl sample PCAP (100 packets)");
            SAMPLE_CAT_034_048_PCAP.to_vec()
        }
    };

    // Step 1: Parse PCAP global header
    let (header, _) = pcap::parse_pcap_header(&pcap_data)?;
    println!(
        "PCAP Header: magic=0x{:08x}, version={}.{}, link_type={}, snaplen={}",
        header.magic, header.version_major, header.version_minor, header.network, header.snaplen
    );

    // Step 2: Extract raw packets
    let packets = pcap::parse_pcap_packets(&pcap_data, &header)?;
    println!("Extracted {} raw packets from PCAP", packets.len());

    // Step 3: Compile ASTERIX DFDL schemas for polymorphic dispatch and specific categories
    println!("Compiling DFDL Schema: asterix.dfdl.xsd...");
    let schema_record = compile_asterix_schema(Some("AsterixRecord"))?;
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record"))?;
    let schema_cat48 = compile_asterix_schema(Some("Cat048Record"))?;
    println!("DFDL Schemas compiled successfully!");

    // Step 4: Validate all packets and data blocks
    let mut total_blocks = 0usize;
    let mut cat34_blocks = 0usize;
    let mut cat48_blocks = 0usize;
    let mut validated_blocks = 0usize;
    let mut validation_failures = 0usize;

    for (pkt_idx, pkt) in packets.iter().enumerate() {
        let udp_payload = match pcap::extract_udp_payload(pkt.data) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Packet {pkt_idx}: UDP extraction error: {e}");
                continue;
            }
        };

        let blocks = match pcap::extract_asterix_blocks(udp_payload) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Packet {pkt_idx}: ASTERIX block extraction error: {e}");
                continue;
            }
        };

        for block in blocks {
            total_blocks = total_blocks.saturating_add(1);
            match block.category {
                34 => cat34_blocks = cat34_blocks.saturating_add(1),
                48 => cat48_blocks = cat48_blocks.saturating_add(1),
                _ => {}
            }

            // Choose schema: specific category schema or polymorphic AsterixRecord schema
            let schema_to_use = match block.category {
                34 => &schema_cat34,
                48 => &schema_cat48,
                _ => &schema_record,
            };

            match parse_asterix_record(schema_to_use, block.raw_data) {
                Ok(doc) => {
                    validated_blocks = validated_blocks.saturating_add(1);
                    if total_blocks <= 3 {
                        let root_tag = doc
                            .root
                            .as_ref()
                            .map(|r| r.name.local_name.as_str())
                            .unwrap_or("unknown");
                        println!(
                            "  [VALID] Block {}: Cat {}, declared_len={}, infoset_root=<{}>, total_nodes={}",
                            total_blocks, block.category, block.length, root_tag, doc.total_nodes
                        );
                    }
                }
                Err(err) => {
                    validation_failures = validation_failures.saturating_add(1);
                    if validation_failures <= 5 {
                        eprintln!(
                            "  [FAIL] Block {}: Cat {}, len={}: {:?}",
                            total_blocks, block.category, block.length, err
                        );
                    }
                }
            }
        }
    }

    println!("\n=== ASTERIX Validation Summary ===");
    println!("Total PCAP packets processed : {}", packets.len());
    println!("Total ASTERIX data blocks    : {}", total_blocks);
    println!("  - Category 034 blocks      : {}", cat34_blocks);
    println!("  - Category 048 blocks      : {}", cat48_blocks);
    println!("Successfully validated blocks: {}", validated_blocks);
    println!("Validation failures          : {}", validation_failures);

    if validation_failures == 0 && validated_blocks > 0 {
        println!("\nRESULT: 100% of ASTERIX blocks PASSED schema validation!");
        Ok(())
    } else {
        Err(format!("Validation failed for {validation_failures} blocks").into())
    }
}
