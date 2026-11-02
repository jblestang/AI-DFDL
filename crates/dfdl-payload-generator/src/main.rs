//! `dfdl-pcap-gen` CLI: DFDL-driven PCAP capture generator.
//!
//! Command-line tool to synthesize valid and anomalous PCAP captures
//! from any compiled DFDL schema.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::env;
use std::fs::{self, File};
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::ExitCode;

use dfdl_example::compile_simple_schema;
use dfdl_payload_generator::{
    GenerationLimits, NetworkConfig, PayloadGenerator, SyntaxAnomaly, TransportProtocol,
};
use dfdl_schema::compiler::SchemaCompiler;

fn print_help() {
    println!(
        r#"dfdl-pcap-gen: DFDL-driven PCAP Capture Generator

USAGE:
    dfdl-pcap-gen [OPTIONS]

OPTIONS:
    --schema <PATH>            Path to DFDL schema XML file (.dfdl.xsd or .xsd).
                               If omitted, defaults to built-in NATO STANAG 5602 SIMPLE schema.
    --root <NAME>              Root element name in schema (e.g. SimplePacket, SimpleControlPayload).
    --output, -o <PATH>        Output PCAP capture file path (default: capture.pcap).
    --mode <MODE>              Payload generation mode:
                                 valid       - Exclusively valid packets (default)
                                 exhaustive  - Comprehensive suite (valid + syntax + semantic)
                                 syntax      - Syntactic anomalies only (truncation, length, bits)
                                 semantic    - Semantic anomalies only (facets, enums, keys)
    --count, -n <COUNT>        Number of packets to generate (default: 10 for 'valid',
                               full suite for 'exhaustive'/'syntax'/'semantic').
                               In 'exhaustive' mode, cycles the test matrix up to COUNT.
    --max-size <SIZE>          Strict maximum total generated PCAP file size in bytes (e.g. 102400, 1.5MB, 500KB).
    --max-payload-size <SIZE>  Maximum allowed size in bytes for an individual payload (e.g. 200, 1KB).
    --seed <SEED>              PRNG seed for deterministic reproducibility (default: 42).
    --transport <PROTO>        Transport encapsulation: 'udp' (default) or 'raw'.
    --port <PORT>              UDP source and destination port (default: 5602).
    --src-ip <IP>              Source IPv4 address (default: 192.168.1.10).
    --dst-ip <IP>              Destination IPv4 address (default: 192.168.1.20).
    --src-mac <MAC>            Source MAC address in hex (default: 00:1A:2B:3C:4D:5E).
    --dst-mac <MAC>            Destination MAC address in hex (default: 00:50:56:C0:00:01).
    --help, -h                 Print this help documentation.

EXAMPLES:
    # 1. Generate 10 valid UDP packets on port 5602
    dfdl-pcap-gen -o valid.pcap -n 10

    # 2. Generate an exhaustive anomaly test matrix
    dfdl-pcap-gen --mode exhaustive -o anomalies.pcap

    # 3. Enforce a strict max capture size of 100 KB
    dfdl-pcap-gen -o budgeted.pcap -n 50 --max-size 102400

    # 4. Generate 5000 exhaustive packets cycling under a budget
    dfdl-pcap-gen --mode exhaustive -o budgeted.pcap -n 5000 --max-size 500000

    # 5. Restrict individual payload size and total capture size
    dfdl-pcap-gen -o budgeted.pcap -n 5000 --max-payload-size 500 --max-size 50000

    # 6. Generate from a custom DFDL schema
    dfdl-pcap-gen --schema my_protocol.dfdl.xsd --root MyMessage -o custom.pcap
"#
    );
}

fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        mac[i] = u8::from_str_radix(p.trim(), 16).ok()?;
    }
    Some(mac)
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let ip: Ipv4Addr = s.parse().ok()?;
    Some(ip.octets())
}

/// Parses a human-readable size string (e.g. "1200", "500KB", "1.5MB", "2GB") into bytes.
fn parse_byte_size(s: &str) -> Option<usize> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if let Some(num_str) = lower.strip_suffix("kb").or_else(|| lower.strip_suffix('k')) {
        let val: f64 = num_str.trim().parse().ok()?;
        Some((val * 1024.0) as usize)
    } else if let Some(num_str) = lower.strip_suffix("mb").or_else(|| lower.strip_suffix('m')) {
        let val: f64 = num_str.trim().parse().ok()?;
        Some((val * 1024.0 * 1024.0) as usize)
    } else if let Some(num_str) = lower.strip_suffix("gb").or_else(|| lower.strip_suffix('g')) {
        let val: f64 = num_str.trim().parse().ok()?;
        Some((val * 1024.0 * 1024.0 * 1024.0) as usize)
    } else if let Some(num_str) = lower.strip_suffix('b') {
        num_str.trim().parse::<usize>().ok()
    } else {
        trimmed.parse::<usize>().ok()
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    let mut schema_path: Option<String> = None;
    let mut root_name: Option<String> = None;
    let mut output_path = String::from("capture.pcap");
    let mut mode = String::from("valid");
    let mut count: usize = 10;
    let mut count_explicitly_set = false;
    let mut max_total_size: Option<usize> = None;
    let mut max_payload_size: Option<usize> = None;
    let mut seed: u64 = 42;
    let mut transport_name = String::from("udp");
    let mut port: u16 = 5602;
    let mut src_ip = [192, 168, 1, 10];
    let mut dst_ip = [192, 168, 1, 20];
    let mut src_mac = [0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E];
    let mut dst_mac = [0x00, 0x50, 0x56, 0xC0, 0x00, 0x01];

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--schema" => {
                i += 1;
                if i < args.len() {
                    schema_path = Some(args[i].clone());
                }
            }
            "--root" => {
                i += 1;
                if i < args.len() {
                    root_name = Some(args[i].clone());
                }
            }
            "--output" | "-o" => {
                i += 1;
                if i < args.len() {
                    output_path = args[i].clone();
                }
            }
            "--mode" => {
                i += 1;
                if i < args.len() {
                    mode = args[i].to_lowercase();
                }
            }
            "--count" | "-n" => {
                i += 1;
                if i < args.len() {
                    if let Ok(c) = args[i].parse::<usize>() {
                        count = c;
                        count_explicitly_set = true;
                    }
                }
            }
            "--max-size" => {
                i += 1;
                if i < args.len() {
                    max_total_size = parse_byte_size(&args[i]);
                }
            }
            "--max-payload-size" => {
                i += 1;
                if i < args.len() {
                    max_payload_size = parse_byte_size(&args[i]);
                }
            }
            "--seed" => {
                i += 1;
                if i < args.len() {
                    if let Ok(s) = args[i].parse::<u64>() {
                        seed = s;
                    }
                }
            }
            "--transport" => {
                i += 1;
                if i < args.len() {
                    transport_name = args[i].to_lowercase();
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    if let Ok(p) = args[i].parse::<u16>() {
                        port = p;
                    }
                }
            }
            "--src-ip" => {
                i += 1;
                if i < args.len() {
                    if let Some(ip) = parse_ip(&args[i]) {
                        src_ip = ip;
                    }
                }
            }
            "--dst-ip" => {
                i += 1;
                if i < args.len() {
                    if let Some(ip) = parse_ip(&args[i]) {
                        dst_ip = ip;
                    }
                }
            }
            "--src-mac" => {
                i += 1;
                if i < args.len() {
                    if let Some(mac) = parse_mac(&args[i]) {
                        src_mac = mac;
                    }
                }
            }
            "--dst-mac" => {
                i += 1;
                if i < args.len() {
                    if let Some(mac) = parse_mac(&args[i]) {
                        dst_mac = mac;
                    }
                }
            }
            unknown => {
                eprintln!("Unknown argument: {unknown}");
                eprintln!("Run with --help for usage information.");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    println!("==================================================================");
    println!(" dfdl-pcap-gen: DFDL PCAP Capture Generator                       ");
    println!("==================================================================");

    // 1. Compile schema
    let compiled_schema = if let Some(ref path) = schema_path {
        println!("[1/4] Loading DFDL schema from '{}'...", path);
        let xml_content = match fs::read_to_string(Path::new(path)) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Error reading schema file '{}': {}", path, e);
                return ExitCode::FAILURE;
            }
        };
        let compiler = SchemaCompiler::new();
        match compiler.compile_str_with_root(&xml_content, root_name.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Schema compilation failed: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("[1/4] Using built-in NATO STANAG 5602 SIMPLE schema...");
        match compile_simple_schema(root_name.as_deref().or(Some("SimplePacket"))) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Built-in schema compilation failed: {:?}", e);
                return ExitCode::FAILURE;
            }
        }
    };

    // 2. Initialize generator with limits
    let mut limits = GenerationLimits::new()
        .max_total_size(max_total_size.unwrap_or(usize::MAX))
        .max_payload_size(max_payload_size.unwrap_or(usize::MAX));
    if max_total_size.is_none() {
        limits.max_total_size = None;
    }
    if max_payload_size.is_none() {
        limits.max_payload_size = None;
    }
    if count_explicitly_set || mode == "valid" {
        limits.max_messages = Some(count);
    }

    let mut generator = PayloadGenerator::new(&compiled_schema, seed)
        .with_limits(limits);

    println!(
        "[2/4] Schema analysis: discovered {} choice branches, {} enum fields, {} bounded fields",
        generator.analysis().choice_branches.len(),
        generator.analysis().enum_fields.len(),
        generator.analysis().bounded_fields.len()
    );

    // 3. Configure network framing
    let net_cfg = NetworkConfig {
        src_mac,
        dst_mac,
        src_ip,
        dst_ip,
    };
    let transport = match transport_name.as_str() {
        "raw" => TransportProtocol::Raw,
        _ => TransportProtocol::Udp {
            src_port: port,
            dst_port: port,
        },
    };

    // 4. Generate payloads and write PCAP
    let mut details = Vec::new();
    details.push(format!("mode: '{}'", mode));
    if count_explicitly_set {
        details.push(format!("requested count: {}", count));
    }
    if let Some(b) = max_total_size {
        details.push(format!("max total size: {} bytes", b));
    }
    if let Some(p) = max_payload_size {
        details.push(format!("max payload size: {} bytes", p));
    }
    println!("[3/4] Generating payloads ({})...", details.join(", "));

    let mut file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error creating output file '{}': {}", output_path, e);
            return ExitCode::FAILURE;
        }
    };

    let report_res = match mode.as_str() {
        "valid" => generator.write_valid_pcap(&mut file, count, &net_cfg, &transport),
        "exhaustive" => {
            let target_count = if count_explicitly_set { count } else { 0 };
            generator.write_exhaustive_pcap(&mut file, target_count, &net_cfg, &transport)
        }
        "syntax" => {
            let mut syntax_payloads = Vec::new();
            if let Ok(p) = generator.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 4 }) {
                syntax_payloads.push(p);
            }
            if let Ok(p) = generator.generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 8 }) {
                syntax_payloads.push(p);
            }
            let (offset, width) = if let Some(len) = generator.analysis().find_length_field() {
                (len.estimated_offset, len.estimated_width.max(1))
            } else {
                (0, 2)
            };
            if let Ok(p) = generator.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
                field_offset: offset,
                field_width: width,
                delta: 16,
            }) {
                syntax_payloads.push(p);
            }
            if let Ok(p) = generator.generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 }) {
                syntax_payloads.push(p);
            }
            for (discriminator_name, valid_keys) in generator.analysis().find_choice_discriminators() {
                let invalid_key = if valid_keys.iter().all(|s| s.chars().all(|c| c.is_ascii_digit())) {
                    let max_key = valid_keys
                        .iter()
                        .filter_map(|s| s.parse::<u64>().ok())
                        .max()
                        .unwrap_or(0);
                    dfdl_core::infoset::value::DfdlValue::UnsignedInt((max_key.saturating_add(99)) as u32)
                } else {
                    dfdl_core::infoset::value::DfdlValue::String("__INVALID_DISPATCH_KEY__".to_string())
                };
                if let Ok(p) = generator.generate_syntax_anomaly(SyntaxAnomaly::WrongDispatchKey {
                    discriminator_name,
                    invalid_key,
                }) {
                    syntax_payloads.push(p);
                }
            }
            let target_count = if count_explicitly_set { count } else { 0 };
            generator
                .write_cycled_pcap(&mut file, &syntax_payloads, target_count, &net_cfg, &transport)
                .map_err(|e| dfdl_core::error::DFDLError::new(dfdl_core::error::DFDLErrorKind::Unparse, &e.to_string()))
        }
        "semantic" => {
            let full_suite = match generator.generate_exhaustive_suite() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to generate semantic payloads: {:?}", e);
                    return ExitCode::FAILURE;
                }
            };
            let semantic_payloads: Vec<_> = full_suite
                .into_iter()
                .filter(|p| matches!(p.mode, dfdl_payload_generator::PayloadMode::SemanticAnomaly(_)))
                .collect();
            let target_count = if count_explicitly_set { count } else { 0 };
            generator
                .write_cycled_pcap(&mut file, &semantic_payloads, target_count, &net_cfg, &transport)
                .map_err(|e| dfdl_core::error::DFDLError::new(dfdl_core::error::DFDLErrorKind::Unparse, &e.to_string()))
        }
        unknown => {
            eprintln!("Unknown mode '{}'. Supported modes: valid, exhaustive, syntax, semantic", unknown);
            return ExitCode::FAILURE;
        }
    };

    let report = match report_res {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed writing PCAP capture: {:?}", e);
            return ExitCode::FAILURE;
        }
    };

    println!("[4/4] Output PCAP capture written to '{}'", output_path);
    println!("------------------------------------------------------------------");
    println!(" Generation Report Summary:");
    println!("   - Packets written:         {}", report.message_count);
    if let Some(limit_msg) = limits.max_messages {
        println!("   - Target message limit:    {}", limit_msg);
    }
    if let Some(budget) = max_total_size {
        println!("   - Total capture size:      {} bytes (budget: {} bytes)", report.total_bytes, budget);
    } else {
        println!("   - Total capture size:      {} bytes", report.total_bytes);
    }
    if let Some(p) = max_payload_size {
        println!("   - Max payload size:        {} bytes", p);
    }
    println!("   - Reached message limit:   {}", report.reached_message_limit);
    println!("   - Reached size limit:      {}", report.reached_size_limit);
    println!("------------------------------------------------------------------");

    if report.message_count == 0 && report.reached_size_limit {
        let budget = max_total_size.unwrap_or(0);
        eprintln!("[!] NOTE: 0 packets were written because the requested --max-size ({} bytes)", budget);
        eprintln!("    is smaller than the minimum packet size for this schema.");
        eprintln!("    To generate packets within this budget, either:");
        eprintln!("      1) Increase total capture size: --max-size <BYTES> (e.g. --max-size 100000)");
        eprintln!("      2) Or limit individual payload size: --max-payload-size 500 --max-size {}", budget);
    } else if report.reached_size_limit {
        if let Some(budget) = max_total_size {
            println!("[i] Info: Stopped generation upon reaching --max-size budget ({} bytes written, budget: {} bytes).", report.total_bytes, budget);
        } else {
            println!("[i] Info: Stopped generation upon reaching size limit ({} bytes written).", report.total_bytes);
        }
    } else if report.reached_message_limit {
        println!("[i] Info: Successfully reached requested message count ({} packets).", report.message_count);
    }

    println!("Capture is ready for inspection with Wireshark and DFDL dissectors.");

    ExitCode::SUCCESS
}
