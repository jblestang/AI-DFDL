//! Example: Dumping decoded ASTERIX data as standard XML Infoset documents.
//!
//! Demonstrates parsing binary ASTERIX records and emitting structured XML
//! representing the DFDL Infoset data items.

use dfdl_example::pcap;
use dfdl_example::{
    compile_asterix_schema, infoset_to_xml, parse_asterix_record, ExampleError,
    SAMPLE_CAT_034_048_PCAP,
};
use std::env;

fn main() -> Result<(), ExampleError> {
    let args: Vec<String> = env::args().collect();
    let dump_limit: usize = args
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(2);

    println!("=== ASTERIX XML Infoset Dump (Displaying first {dump_limit} blocks) ===");

    // Step 1: Parse PCAP
    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP)?;
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header)?;

    // Step 2: Compile DFDL schemas
    let schema_record = compile_asterix_schema(Some("AsterixRecord"))?;
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record"))?;
    let schema_cat48 = compile_asterix_schema(Some("Cat048Record"))?;

    let mut displayed_count = 0usize;

    for (pkt_idx, pkt) in packets.iter().enumerate() {
        if displayed_count >= dump_limit {
            break;
        }

        let udp_payload = match pcap::extract_udp_payload(pkt.data) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let blocks = match pcap::extract_asterix_blocks(udp_payload) {
            Ok(b) => b,
            Err(_) => continue,
        };

        for block in blocks {
            if displayed_count >= dump_limit {
                break;
            }

            let schema_to_use = match block.category {
                34 => &schema_cat34,
                48 => &schema_cat48,
                _ => &schema_record,
            };

            let doc = match parse_asterix_record(schema_to_use, block.raw_data) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Warning: Failed to parse block: {e:?}");
                    continue;
                }
            };

            displayed_count = displayed_count.saturating_add(1);
            println!(
                "\n<!-- ================================================================ -->"
            );
            println!(
                "<!-- Packet {}, Block {} | Category {}, Length {} bytes -->",
                pkt_idx.saturating_add(1),
                displayed_count,
                block.category,
                block.length
            );
            println!(
                "<!-- ================================================================ -->"
            );

            let xml_output = infoset_to_xml(&doc);
            print!("{xml_output}");
        }
    }

    println!("\nDump completed: emitted {displayed_count} XML Infoset documents.");
    Ok(())
}
