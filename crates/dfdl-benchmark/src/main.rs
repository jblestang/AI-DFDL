//! Command-line benchmark runner for AI-DFDL latency and throughput testing.
//!
//! Evaluates parse, unparse, and roundtrip latency on FakeTDL tactical data link assets,
//! validating byte accuracy and infoset conformance against reference XML datasets,
//! and comparing performance directly across Raw DFDL (interpreted engine), DFDL-VM (linear bytecode engine),
//! and hand-written native Rust codecs.

#![deny(missing_docs)]
#![allow(clippy::expect_used)]

use dfdl_benchmark::{
    benchmark_op, compile_faketdl_schema, data, format_benchmark_table, format_comparison_table,
    format_three_way_comparison_table, parse_faketdl, roundtrip_faketdl, unparse_faketdl,
    validate_infoset, FakeTdlFile, FakeTdlMessage,
};
use dfdl_core::error::DFDLResult;
use dfdl_core::infoset::tree::InfosetDocument;
use dfdl_core::io::bitstream::BitReader;
use dfdl_core::io::source::SliceByteSource;
use dfdl_core::io::traits::{BitOrder, ByteOrder};
use dfdl_vm::{
    ArenaEventSink, BytecodeCompiler, BytecodeProgram, DomBuilderSink, NullEventSink,
    VmInterpreter,
};

/// Parses binary input using the schema-compiled VM program into an infoset DOM document.
fn parse_vm_dom(prog: &BytecodeProgram, data: &[u8]) -> DFDLResult<InfosetDocument> {
    let mut reader = BitReader::new(
        SliceByteSource::new(data),
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut vm = VmInterpreter::new();
    let mut sink = DomBuilderSink::new(&prog.symbols);
    vm.execute(prog, &mut reader, &mut sink)?;
    sink.into_document()
}

/// Parses binary input using the schema-compiled VM program into a zero-allocation arena event sink.
fn parse_vm_arena(prog: &BytecodeProgram, data: &[u8]) -> DFDLResult<ArenaEventSink> {
    let mut reader = BitReader::new(
        SliceByteSource::new(data),
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut vm = VmInterpreter::new();
    let mut sink = ArenaEventSink::with_capacity(64);
    vm.execute(prog, &mut reader, &mut sink)?;
    Ok(sink)
}

/// Validates binary input using the schema-compiled VM program at wire speed with null event sink.
fn parse_vm_null(prog: &BytecodeProgram, data: &[u8]) -> DFDLResult<()> {
    let mut reader = BitReader::new(
        SliceByteSource::new(data),
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut vm = VmInterpreter::new();
    let mut sink = NullEventSink::new();
    vm.execute(prog, &mut reader, &mut sink)
}

fn main() {
    println!("================================================================================");
    println!("                   AI-DFDL Tactical Messaging Latency Benchmark                  ");
    println!("                   Assets: OpenDFDL & DFDLSchemas / fakeTDL                     ");
    println!("================================================================================\n");

    println!("[1/6] Compiling FakeTDL schemas & deducing DFDL-VM bytecode from schema...");
    let t0 = std::time::Instant::now();
    let msg_schema =
        compile_faketdl_schema("fakeTDL").expect("Failed to compile 'fakeTDL' schema");
    let file_schema =
        compile_faketdl_schema("fakeTDLFile").expect("Failed to compile 'fakeTDLFile' schema");
    let vm_msg_prog =
        BytecodeCompiler::compile(&msg_schema).expect("Failed to compile 'fakeTDL' schema to VM");
    let vm_file_prog =
        BytecodeCompiler::compile(&file_schema).expect("Failed to compile 'fakeTDLFile' schema to VM");
    println!(
        "      Compiled AST schemas and deduced VM bytecode from schema in {:.2} ms\n",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    println!("[2/6] Validating data roundtrips, VM execution, and reference XML infosets...");

    // Validate test_msg_01 (Raw DFDL)
    let (msg_doc, msg_bytes) =
        roundtrip_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("Roundtrip test_msg_01 failed");
    assert_eq!(
        msg_bytes.as_slice(),
        data::TEST_MSG_01_DAT,
        "test_msg_01 binary roundtrip mismatch"
    );
    validate_infoset(&msg_doc, data::TEST_MSG_01_XML)
        .expect("test_msg_01 XML infoset validation failed");
    println!("  [PASS] Raw DFDL: test_msg_01.dat (64B Track) roundtrip & XML verified");

    // Validate test_file_01 (Raw DFDL)
    let (file_doc, file_bytes) = roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT)
        .expect("Roundtrip test_file_01 failed");
    assert_eq!(
        file_bytes.as_slice(),
        data::TEST_FILE_01_DAT,
        "test_file_01 binary roundtrip mismatch"
    );
    validate_infoset(&file_doc, data::TEST_FILE_01_XML)
        .expect("test_file_01 XML infoset validation failed");
    println!("  [PASS] Raw DFDL: test_file_01.dat (192B Multi-msg) roundtrip & XML verified");

    // Validate DFDL-VM against XML infosets
    let vm_msg_doc = parse_vm_dom(&vm_msg_prog, data::TEST_MSG_01_DAT)
        .expect("DFDL-VM parse test_msg_01 failed");
    validate_infoset(&vm_msg_doc, data::TEST_MSG_01_XML)
        .expect("DFDL-VM test_msg_01 XML infoset validation failed");
    println!("  [PASS] DFDL-VM: test_msg_01.dat XML infoset verified against reference");

    let vm_file_doc = parse_vm_dom(&vm_file_prog, data::TEST_FILE_01_DAT)
        .expect("DFDL-VM parse test_file_01 failed");
    validate_infoset(&vm_file_doc, data::TEST_FILE_01_XML)
        .expect("DFDL-VM test_file_01 XML infoset validation failed");
    println!("  [PASS] DFDL-VM: test_file_01.dat XML infoset verified against reference");

    // Validate Native Rust codec
    let native_msg =
        FakeTdlMessage::decode(data::TEST_MSG_01_DAT).expect("Native decode test_msg_01 failed");
    assert_eq!(
        native_msg.encode().as_slice(),
        data::TEST_MSG_01_DAT,
        "native test_msg_01 binary mismatch"
    );

    let native_file =
        FakeTdlFile::decode(data::TEST_FILE_01_DAT).expect("Native decode test_file_01 failed");
    assert_eq!(
        native_file.encode().as_slice(),
        data::TEST_FILE_01_DAT,
        "native test_file_01 binary mismatch"
    );
    println!("  [PASS] Native Rust Codec: byte-identical verification on all test assets\n");

    let warmup = 1_000;
    let iterations = 10_000;

    println!("[3/6] Running Raw DFDL (Interpreted AST) benchmarks (1,000 warmup, 10,000 iters)...");
    let b_raw_msg_parse = benchmark_op("Raw DFDL parse (64B Track)", 64, warmup, iterations, || {
        parse_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("parse failed")
    });

    let b_raw_msg_unparse = benchmark_op("Raw DFDL unparse (64B Track)", 64, warmup, iterations, || {
        unparse_faketdl(&msg_schema, &msg_doc).expect("unparse failed")
    });

    let b_raw_msg_rt = benchmark_op("Raw DFDL roundtrip (64B Track)", 64, warmup, iterations, || {
        roundtrip_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("roundtrip failed")
    });

    let b_raw_file_parse = benchmark_op("Raw DFDLFile parse (192B Multi)", 192, warmup, iterations, || {
        parse_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file parse failed")
    });

    let b_raw_file_unparse = benchmark_op("Raw DFDLFile unparse (192B Multi)", 192, warmup, iterations, || {
        unparse_faketdl(&file_schema, &file_doc).expect("file unparse failed")
    });

    let b_raw_file_rt = benchmark_op("Raw DFDLFile roundtrip (192B Multi)", 192, warmup, iterations, || {
        roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file roundtrip failed")
    });

    println!("\n[4/6] Running DFDL-VM (Schema-compiled Bytecode) benchmarks (1,000 warmup, 10,000 iters)...");
    let b_vm_msg_dom = benchmark_op("DFDL-VM parse DOM (64B Track)", 64, warmup, iterations, || {
        parse_vm_dom(&vm_msg_prog, data::TEST_MSG_01_DAT).expect("vm dom failed")
    });

    let b_vm_msg_arena = benchmark_op("DFDL-VM parse Arena (64B Track)", 64, warmup, iterations, || {
        parse_vm_arena(&vm_msg_prog, data::TEST_MSG_01_DAT).expect("vm arena failed")
    });

    let b_vm_msg_null = benchmark_op("DFDL-VM parse Null (64B Track)", 64, warmup, iterations, || {
        parse_vm_null(&vm_msg_prog, data::TEST_MSG_01_DAT).expect("vm null failed")
    });

    let b_vm_file_dom = benchmark_op("DFDL-VM parse DOM (192B Multi)", 192, warmup, iterations, || {
        parse_vm_dom(&vm_file_prog, data::TEST_FILE_01_DAT).expect("vm file dom failed")
    });

    let b_vm_file_arena = benchmark_op("DFDL-VM parse Arena (192B Multi)", 192, warmup, iterations, || {
        parse_vm_arena(&vm_file_prog, data::TEST_FILE_01_DAT).expect("vm file arena failed")
    });

    let b_vm_file_null = benchmark_op("DFDL-VM parse Null (192B Multi)", 192, warmup, iterations, || {
        parse_vm_null(&vm_file_prog, data::TEST_FILE_01_DAT).expect("vm file null failed")
    });

    println!("\n[5/6] Running Native Rust Codec benchmarks (1,000 warmup, 10,000 iters)...");
    let b_native_msg_decode = benchmark_op("Native decode (64B Track)", 64, warmup, iterations, || {
        FakeTdlMessage::decode(data::TEST_MSG_01_DAT).expect("native decode failed")
    });

    let b_native_msg_encode = benchmark_op("Native encode (64B Track)", 64, warmup, iterations, || {
        native_msg.encode()
    });

    let b_native_msg_rt = benchmark_op("Native roundtrip (64B Track)", 64, warmup, iterations, || {
        let msg = FakeTdlMessage::decode(data::TEST_MSG_01_DAT).expect("native decode failed");
        msg.encode()
    });

    let b_native_file_decode = benchmark_op("NativeFile decode (192B Multi)", 192, warmup, iterations, || {
        FakeTdlFile::decode(data::TEST_FILE_01_DAT).expect("native file decode failed")
    });

    let b_native_file_encode = benchmark_op("NativeFile encode (192B Multi)", 192, warmup, iterations, || {
        native_file.encode()
    });

    let b_native_file_rt = benchmark_op("NativeFile roundtrip (192B Multi)", 192, warmup, iterations, || {
        let file = FakeTdlFile::decode(data::TEST_FILE_01_DAT).expect("native file decode failed");
        file.encode()
    });

    println!("\n[6/6] Benchmark Results & Performance Gap Summary:\n");

    println!("--- Raw DFDL (Interpreted AST Engine) Performance ---");
    let raw_results = [
        b_raw_msg_parse.clone(),
        b_raw_msg_unparse.clone(),
        b_raw_msg_rt.clone(),
        b_raw_file_parse.clone(),
        b_raw_file_unparse.clone(),
        b_raw_file_rt.clone(),
    ];
    print!("{}", format_benchmark_table(&raw_results));

    println!("\n--- DFDL-VM (Linear Bytecode Engine compiled from Schema in no_std) Performance ---");
    let vm_results = [
        b_vm_msg_dom.clone(),
        b_vm_msg_arena.clone(),
        b_vm_msg_null.clone(),
        b_vm_file_dom.clone(),
        b_vm_file_arena.clone(),
        b_vm_file_null.clone(),
    ];
    print!("{}", format_benchmark_table(&vm_results));

    println!("\n--- Native Rust Codec Performance ---");
    let native_results = [
        b_native_msg_decode.clone(),
        b_native_msg_encode.clone(),
        b_native_msg_rt.clone(),
        b_native_file_decode.clone(),
        b_native_file_encode.clone(),
        b_native_file_rt.clone(),
    ];
    print!("{}", format_benchmark_table(&native_results));

    println!("\n--- 3-Way Head-to-Head Comparison: Raw DFDL vs DFDL-VM vs Native Rust ---");
    let three_way_rows = [
        (
            "Track Msg Parse (DOM)",
            &b_raw_msg_parse,
            &b_vm_msg_dom,
            &b_native_msg_decode,
        ),
        (
            "Track Msg Parse (Arena zero-alloc)",
            &b_raw_msg_parse,
            &b_vm_msg_arena,
            &b_native_msg_decode,
        ),
        (
            "Track Msg Parse (Null wire-speed)",
            &b_raw_msg_parse,
            &b_vm_msg_null,
            &b_native_msg_decode,
        ),
        (
            "File Parse (DOM 192B Multi)",
            &b_raw_file_parse,
            &b_vm_file_dom,
            &b_native_file_decode,
        ),
        (
            "File Parse (Arena zero-alloc 192B)",
            &b_raw_file_parse,
            &b_vm_file_arena,
            &b_native_file_decode,
        ),
        (
            "File Parse (Null wire-speed 192B)",
            &b_raw_file_parse,
            &b_vm_file_null,
            &b_native_file_decode,
        ),
    ];
    print!("{}", format_three_way_comparison_table(&three_way_rows));

    println!("\n--- Performance Gap Comparison (Raw DFDL vs Native Rust) ---");
    let comparison_pairs = [
        ("Parse / Decode (64B Track)", &b_raw_msg_parse, &b_native_msg_decode),
        ("Unparse / Encode (64B Track)", &b_raw_msg_unparse, &b_native_msg_encode),
        ("Roundtrip (64B Track)", &b_raw_msg_rt, &b_native_msg_rt),
        ("File Parse / Decode (192B)", &b_raw_file_parse, &b_native_file_decode),
        ("File Unparse / Encode (192B)", &b_raw_file_unparse, &b_native_file_encode),
        ("File Roundtrip (192B)", &b_raw_file_rt, &b_native_file_rt),
    ];
    print!("{}", format_comparison_table(&comparison_pairs));

    println!("\nLatency testing, 3-way comparison, and data validation completed successfully.");
}
