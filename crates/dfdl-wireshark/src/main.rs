//! `dfdl-wireshark` CLI: Generic Wireshark dissector and PCAP decoder.
//!
//! Provides command-line tools to compile DFDL schemas, inspect schema metadata,
//! decode PCAP capture files, and generate Wireshark Lua plugin scripts.

use dfdl_wireshark::{
    decode_pcap_with_engine, generate_lua_dissector, DfdlDissectorEngine, LuaDissectorConfig,
    SchemaMetadataRegistry,
};
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn print_help() {
    println!(
        r#"dfdl-wireshark: Generic Wireshark Dissector and PCAP Decoder (DFDL Powered)

USAGE:
    dfdl-wireshark decode --schema <SCHEMA.xsd> --pcap <CAPTURE.pcap> [OPTIONS]
    dfdl-wireshark dissect-hex --schema <SCHEMA.xsd> <HEX_PAYLOAD>
    dfdl-wireshark inspect-schema --schema <SCHEMA.xsd>
    dfdl-wireshark generate-lua --schema <SCHEMA.xsd> [OPTIONS]

SUBCOMMANDS:
    decode          Decodes a PCAP capture file using a DFDL schema
    dissect-hex     Dissects a raw hexadecimal payload and prints JSON tree
    inspect-schema  Extracts and displays all metadata and units from a DFDL schema
    schema-fields   Extracts schema fields as JSON for dynamic Wireshark dissector
    generate-lua    Generates a generic Wireshark Lua dissector script

OPTIONS:
    --schema <PATH>   Path to DFDL XML Schema (.dfdl.xsd)
    --pcap <PATH>     Path to network packet capture file (.pcap)
    --root <NAME>     Optional target root element name in schema
    --limit <N>       Limit the maximum number of packets to decode
    --format <FMT>    Output format: ascii (default), pdml, or json
    --proto <NAME>    Protocol name for Lua generator (defaults to schema name)
    --port <PORT>     UDP port number to register in Wireshark (repeatable)
    --cli-bin <PATH>  Full path to dfdl-wireshark CLI binary (for Lua generator)
    --dylib <PATH>    Full path to libdfdl_wireshark library (for Lua generator)
    --help            Print this help message
"#
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_help();
        return ExitCode::FAILURE;
    }

    let subcmd = match args.get(1) {
        Some(s) => s.as_str(),
        None => return ExitCode::FAILURE,
    };

    let rest_args = args.get(2..).unwrap_or(&[]);

    match subcmd {
        "decode" => handle_decode(rest_args),
        "dissect-hex" => handle_dissect_hex(rest_args),
        "inspect-schema" => handle_inspect_schema(rest_args),
        "schema-fields" => handle_schema_fields(rest_args),
        "generate-lua" => handle_generate_lua(rest_args),
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        unknown => {
            eprintln!("Unknown subcommand: {unknown}");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn handle_decode(args: &[String]) -> ExitCode {
    let mut schema_path = None;
    let mut pcap_path = None;
    let mut root_elem = None;
    let mut limit = None;
    let mut format = "ascii".to_string();

    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str) {
            Some("--schema") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                schema_path = args.get(i).cloned();
            }
            Some("--pcap") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                pcap_path = args.get(i).cloned();
            }
            Some("--root") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                root_elem = args.get(i).cloned();
            }
            Some("--limit") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                limit = args.get(i).and_then(|s| s.parse::<usize>().ok());
            }
            Some("--format") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                if let Some(f) = args.get(i) {
                    format = f.clone();
                }
            }
            _ => {}
        }
        i = match i.checked_add(1) {
            Some(idx) => idx,
            None => break,
        };
    }

    let schema_file = match schema_path {
        Some(s) => s,
        None => {
            eprintln!("Error: --schema is required for decode");
            return ExitCode::FAILURE;
        }
    };

    let pcap_file = match pcap_path {
        Some(p) => p,
        None => {
            eprintln!("Error: --pcap is required for decode");
            return ExitCode::FAILURE;
        }
    };

    let engine = match DfdlDissectorEngine::from_schema_file(&schema_file, root_elem.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to load schema: {e}");
            return ExitCode::FAILURE;
        }
    };

    let pcap_bytes = match fs::read(Path::new(&pcap_file)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to read PCAP file {pcap_file}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let packets = match decode_pcap_with_engine(&pcap_bytes, &engine, limit) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("PCAP decoding failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "Successfully decoded {} packets using schema: {}",
        packets.len(),
        engine.protocol_name()
    );

    for pkt in &packets {
        match format.as_str() {
            "pdml" => print!("{}", pkt.tree.render_pdml(pkt.packet_index)),
            "json" => println!("{}", pkt.tree.to_json()),
            _ => {
                println!(
                    "\n=== Packet #{} ({} bytes, timestamp: {}.{:06}) ===",
                    pkt.packet_index, pkt.payload_len, pkt.timestamp_sec, pkt.timestamp_frac
                );
                print!("{}", pkt.tree.render_ascii());
            }
        }
    }

    ExitCode::SUCCESS
}

fn handle_dissect_hex(args: &[String]) -> ExitCode {
    let mut schema_path = None;
    let mut hex_data = None;

    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str) {
            Some("--schema") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                schema_path = args.get(i).cloned();
            }
            Some(arg) if !arg.starts_with("--") && hex_data.is_none() => {
                hex_data = Some(arg.to_string());
            }
            _ => {}
        }
        i = match i.checked_add(1) {
            Some(idx) => idx,
            None => break,
        };
    }

    let schema_file = match schema_path {
        Some(s) => s,
        None => {
            eprintln!("Error: --schema is required");
            return ExitCode::FAILURE;
        }
    };

    let hex_str = match hex_data {
        Some(h) => h,
        None => {
            eprintln!("Error: hex payload is required");
            return ExitCode::FAILURE;
        }
    };

    let bytes = match decode_hex(&hex_str) {
        Some(b) => b,
        None => {
            eprintln!("Error: invalid hex string");
            return ExitCode::FAILURE;
        }
    };

    let engine = match DfdlDissectorEngine::from_schema_file(&schema_file, None) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to load schema: {e}");
            return ExitCode::FAILURE;
        }
    };

    match engine.dissect_payload_to_json(&bytes) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Dissection failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn handle_inspect_schema(args: &[String]) -> ExitCode {
    let mut schema_path = None;

    let mut i = 0;
    while i < args.len() {
        if let Some("--schema") = args.get(i).map(String::as_str) {
            i = match i.checked_add(1) {
                Some(idx) => idx,
                None => break,
            };
            schema_path = args.get(i).cloned();
        }
        i = match i.checked_add(1) {
            Some(idx) => idx,
            None => break,
        };
    }

    let schema_file = match schema_path {
        Some(s) => s,
        None => {
            eprintln!("Error: --schema is required");
            return ExitCode::FAILURE;
        }
    };

    let content = match fs::read_to_string(&schema_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read schema file: {e}");
            return ExitCode::FAILURE;
        }
    };

    let registry = SchemaMetadataRegistry::from_schema_xml(&content);
    println!("Schema Metadata Discovery: {}", schema_file);
    println!("Discovered Protocol Name: {}", registry.protocol_name);
    if let Some(ref root) = registry.root_element {
        println!("Discovered Root Element: {}", root);
    }
    println!("Total Registered Fields: {}\n", registry.len());

    for (name, meta) in registry.iter() {
        let mut details = Vec::new();
        if !meta.unit.is_empty() {
            details.push(format!("Unit: {}", meta.unit));
        }
        if !meta.resolution_str.is_empty() {
            details.push(format!("Resolution: {}", meta.resolution_str));
        }
        if meta.scale_factor != 1.0 {
            details.push(format!("Scale: {}", meta.scale_factor));
        }
        if !meta.description.is_empty() {
            details.push(format!("Title: {}", meta.description));
        }

        let desc = if details.is_empty() {
            String::new()
        } else {
            format!(" [{}]", details.join(", "))
        };

        println!("- {}{}", name, desc);
        if !meta.documentation.is_empty() {
            println!("    Doc: {}", meta.documentation);
        }
    }

    ExitCode::SUCCESS
}

fn handle_schema_fields(args: &[String]) -> ExitCode {
    let mut schema_path = None;

    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str) {
            Some("--schema") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                schema_path = args.get(i).cloned();
            }
            Some(arg) if !arg.starts_with("--") && schema_path.is_none() => {
                schema_path = Some(arg.to_string());
            }
            _ => {}
        }
        i = match i.checked_add(1) {
            Some(idx) => idx,
            None => break,
        };
    }

    let schema_file = match schema_path {
        Some(s) => s,
        None => {
            eprintln!("Error: --schema is required");
            return ExitCode::FAILURE;
        }
    };

    let xml = match std::fs::read_to_string(&schema_file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read schema {schema_file}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let registry = SchemaMetadataRegistry::from_schema_xml(&xml);
    println!("{}", registry.to_fields_json());
    ExitCode::SUCCESS
}

fn handle_generate_lua(args: &[String]) -> ExitCode {
    let mut schema_path = None;
    let mut proto_name = None;
    let mut output_path = None;
    let mut udp_ports = Vec::new();
    let mut cli_bin_path = None;
    let mut dylib_path = None;

    let mut i = 0;
    while i < args.len() {
        match args.get(i).map(String::as_str) {
            Some("--schema") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                schema_path = args.get(i).cloned();
            }
            Some("--proto") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                proto_name = args.get(i).cloned();
            }
            Some("--output") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                output_path = args.get(i).cloned();
            }
            Some("--port") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                if let Some(p) = args.get(i).and_then(|s| s.parse::<u16>().ok()) {
                    udp_ports.push(p);
                }
            }
            Some("--cli-bin") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                cli_bin_path = args.get(i).cloned();
            }
            Some("--dylib") => {
                i = match i.checked_add(1) {
                    Some(idx) => idx,
                    None => break,
                };
                dylib_path = args.get(i).cloned();
            }
            _ => {}
        }
        i = match i.checked_add(1) {
            Some(idx) => idx,
            None => break,
        };
    }

    let raw_schema = schema_path.unwrap_or_else(|| {
        "crates/dfdl-example/schemas/asterix.dfdl.xsd".to_string()
    });

    let schema_file = match std::fs::canonicalize(&raw_schema) {
        Ok(abs) => abs.to_string_lossy().to_string(),
        Err(_) => raw_schema,
    };

    let p_name = proto_name.unwrap_or_else(|| {
        Path::new(&schema_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("DFDL_Protocol")
            .to_string()
    });

    let mut config = LuaDissectorConfig::new(&p_name, &schema_file);
    config.udp_ports = udp_ports;

    // Resolve CLI binary path if specified or check standard locations
    config.cli_bin_path = cli_bin_path.or_else(|| {
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(canon) = std::fs::canonicalize(&exe) {
                return Some(canon.to_string_lossy().to_string());
            }
        }
        if let Ok(canon) = std::fs::canonicalize("target/release/dfdl-wireshark") {
            return Some(canon.to_string_lossy().to_string());
        }
        None
    });

    // Resolve native dylib path if specified or check standard locations
    config.dylib_path = dylib_path.or_else(|| {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                for name in &["libdfdl_wireshark.dylib", "libdfdl_wireshark.so", "dfdl-wireshark.so"] {
                    let p = parent.join(name);
                    if p.exists() {
                        if let Ok(canon) = std::fs::canonicalize(&p) {
                            return Some(canon.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        for path in &[
            "target/release/libdfdl_wireshark.dylib",
            "target/release/libdfdl_wireshark.so",
        ] {
            if let Ok(canon) = std::fs::canonicalize(path) {
                return Some(canon.to_string_lossy().to_string());
            }
        }
        None
    });

    let lua_code = generate_lua_dissector(&config);
    if let Some(ref out_file) = output_path {
        if let Err(e) = std::fs::write(out_file, &lua_code) {
            eprintln!("Failed to write {out_file}: {e}");
            return ExitCode::FAILURE;
        }
    } else {
        println!("{lua_code}");
    }

    ExitCode::SUCCESS
}

/// Decodes an ASCII hex string into raw bytes.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    let clean = hex.trim();
    if !clean.len().is_multiple_of(2) {
        return None;
    }

    let mut bytes = Vec::with_capacity(clean.len() / 2);
    let chars: Vec<char> = clean.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        let hi = chars.get(i)?.to_digit(16)? as u8;
        let lo = chars.get(i.checked_add(1)?)?.to_digit(16)? as u8;
        bytes.push((hi << 4) | lo);
        i = i.checked_add(2)?;
    }

    Some(bytes)
}
