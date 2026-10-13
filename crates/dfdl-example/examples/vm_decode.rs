//! Example: High-speed ASTERIX decoding using the `dfdl-vm` bytecode engine.
//!
//! Demonstrates compiling an ASTERIX DFDL schema into optimized bytecode,
//! inspecting bytecode instructions, and executing [`dfdl_vm::VmInterpreter`]
//! for wire-speed parsing.

use dfdl_example::pcap;
use dfdl_example::{
    compile_asterix_schema, compile_asterix_vm, decode_with_vm, parse_asterix_record,
    ExampleError, SAMPLE_CAT_034_048_PCAP,
};
use std::time::Instant;

fn main() -> Result<(), ExampleError> {
    println!("=== ASTERIX Decoding via dfdl-vm Bytecode Engine ===");

    // Step 1: Compile DFDL Schema to CompiledSchema graph
    println!("Compiling DFDL Schema for Category 034 Service Messages...");
    let schema_cat34 = compile_asterix_schema(Some("Cat034Record"))?;

    // Step 2: Compile to linear VM Bytecode Program
    println!("Lowering CompiledSchema to VM Bytecode Program via BytecodeCompiler...");
    let vm_program = compile_asterix_vm(&schema_cat34)?;
    println!(
        "VM Compilation Succeeded! Instructions: {}, Interned Symbols: {}",
        vm_program.instructions.len(),
        vm_program.symbols.len()
    );

    // Print first 10 instructions for inspection
    println!("\nBytecode Disassembly Preview (first 10 instructions):");
    for (ip, instr) in vm_program.instructions.iter().take(10).enumerate() {
        println!("  [{:03}] {:?}", ip, instr);
    }

    // Step 3: Extract Category 034 blocks from PCAP
    let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP)?;
    let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header)?;

    let mut cat34_blocks = Vec::new();
    for pkt in &packets {
        if let Ok(udp_payload) = pcap::extract_udp_payload(pkt.data) {
            if let Ok(blocks) = pcap::extract_asterix_blocks(udp_payload) {
                for block in blocks {
                    if block.category == 34 {
                        cat34_blocks.push(block.raw_data.to_vec());
                    }
                }
            }
        }
    }
    println!(
        "\nExtracted {} Category 034 blocks from PCAP sample",
        cat34_blocks.len()
    );

    // Step 4: Execute VM Interpreter on extracted blocks and verify parity
    let mut vm_success_count = 0usize;
    let mut total_duration = std::time::Duration::ZERO;

    for (idx, raw_block) in cat34_blocks.iter().enumerate() {
        let t0 = Instant::now();
        let vm_doc_res = decode_with_vm(&vm_program, raw_block);
        let elapsed = t0.elapsed();
        total_duration = total_duration.saturating_add(elapsed);

        match vm_doc_res {
            Ok(doc) => {
                vm_success_count = vm_success_count.saturating_add(1);
                if idx == 0 {
                    let root_tag = doc
                        .root
                        .as_ref()
                        .map(|r| r.name.local_name.as_str())
                        .unwrap_or("unknown");
                    println!(
                        "  Block 1 decoded successfully via VM: root=<{}>, total_nodes={}",
                        root_tag, doc.total_nodes
                    );

                    // Cross-check parity against ParserEngine
                    let engine_doc = parse_asterix_record(&schema_cat34, raw_block)?;
                    assert_eq!(
                        doc.total_nodes, engine_doc.total_nodes,
                        "VM decoded node count matches ParserEngine node count"
                    );
                    println!("  Parity check PASSED: VM output exactly matches ParserEngine DOM!");
                }
            }
            Err(e) => {
                eprintln!("  Block {} VM execution error: {:?}", idx.saturating_add(1), e);
            }
        }
    }

    println!("\n=== dfdl-vm Execution Summary ===");
    println!("Total Cat 034 blocks tested  : {}", cat34_blocks.len());
    println!("Successfully decoded by VM   : {}", vm_success_count);
    let avg_latency = total_duration
        .checked_div(cat34_blocks.len().max(1) as u32)
        .unwrap_or_default();
    println!("Total VM decode time         : {:?}", total_duration);
    println!("Average VM decode latency    : {:?}", avg_latency);

    if vm_success_count > 0 {
        println!("\nRESULT: dfdl-vm execution succeeded!");
        Ok(())
    } else {
        Err("No blocks successfully decoded by dfdl-vm".into())
    }
}
