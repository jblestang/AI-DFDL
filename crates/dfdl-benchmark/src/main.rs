//! Command-line benchmark runner for AI-DFDL latency and throughput testing.
//!
//! Evaluates parse, unparse, and roundtrip latency on FakeTDL tactical data link assets,
//! validating byte accuracy and infoset conformance against reference XML datasets,
//! and comparing performance directly against a hand-written native Rust codec.

#![deny(missing_docs)]
#![allow(clippy::expect_used)]

use dfdl_benchmark::{
    benchmark_op, compile_faketdl_schema, data, format_benchmark_table, format_comparison_table,
    parse_faketdl, roundtrip_faketdl, unparse_faketdl, validate_infoset, FakeTdlFile,
    FakeTdlMessage,
};

fn main() {
    println!("================================================================================");
    println!("                   AI-DFDL Tactical Messaging Latency Benchmark                  ");
    println!("                   Assets: OpenDFDL & DFDLSchemas / fakeTDL                     ");
    println!("================================================================================\n");

    println!("[1/5] Compiling FakeTDL schemas...");
    let t0 = std::time::Instant::now();
    let msg_schema =
        compile_faketdl_schema("fakeTDL").expect("Failed to compile 'fakeTDL' schema");
    let file_schema =
        compile_faketdl_schema("fakeTDLFile").expect("Failed to compile 'fakeTDLFile' schema");
    println!("      Compiled schemas in {:.2} ms\n", t0.elapsed().as_secs_f64() * 1000.0);

    println!("[2/5] Validating data roundtrips and reference XML infosets...");

    // Validate test_msg_01 (Interpreted)
    let (msg_doc, msg_bytes) =
        roundtrip_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("Roundtrip test_msg_01 failed");
    assert_eq!(
        msg_bytes.as_slice(),
        data::TEST_MSG_01_DAT,
        "test_msg_01 binary roundtrip mismatch"
    );
    validate_infoset(&msg_doc, data::TEST_MSG_01_XML)
        .expect("test_msg_01 XML infoset validation failed");
    println!("  [PASS] test_msg_01.dat (64B Track): byte-identical roundtrip & XML infoset verified");

    // Validate test_track_good_01 (Interpreted)
    let (track_doc, track_bytes) = roundtrip_faketdl(&msg_schema, data::TEST_TRACK_GOOD_01_DAT)
        .expect("Roundtrip test_track_good_01 failed");
    assert_eq!(
        track_bytes.as_slice(),
        data::TEST_TRACK_GOOD_01_DAT,
        "test_track_good_01 binary roundtrip mismatch"
    );
    validate_infoset(&track_doc, data::TEST_TRACK_GOOD_01_XML)
        .expect("test_track_good_01 XML infoset validation failed");
    println!("  [PASS] test_track_good_01.dat (64B Track): byte-identical roundtrip & XML infoset verified");

    // Validate test_file_01 (Interpreted)
    let (file_doc, file_bytes) = roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT)
        .expect("Roundtrip test_file_01 failed");
    assert_eq!(
        file_bytes.as_slice(),
        data::TEST_FILE_01_DAT,
        "test_file_01 binary roundtrip mismatch"
    );
    validate_infoset(&file_doc, data::TEST_FILE_01_XML)
        .expect("test_file_01 XML infoset validation failed");
    println!("  [PASS] test_file_01.dat (192B Multi-msg): byte-identical roundtrip & XML infoset verified");

    // Validate Native Rust codec
    let native_msg =
        FakeTdlMessage::decode(data::TEST_MSG_01_DAT).expect("Native decode test_msg_01 failed");
    assert_eq!(
        native_msg.encode().as_slice(),
        data::TEST_MSG_01_DAT,
        "native test_msg_01 binary mismatch"
    );

    let native_track_good =
        FakeTdlMessage::decode(data::TEST_TRACK_GOOD_01_DAT).expect("Native decode test_track_good_01 failed");
    assert_eq!(
        native_track_good.encode().as_slice(),
        data::TEST_TRACK_GOOD_01_DAT,
        "native test_track_good_01 binary mismatch"
    );

    let native_file =
        FakeTdlFile::decode(data::TEST_FILE_01_DAT).expect("Native decode test_file_01 failed");
    assert_eq!(
        native_file.encode().as_slice(),
        data::TEST_FILE_01_DAT,
        "native test_file_01 binary mismatch"
    );
    println!("  [PASS] Native Rust Codec: byte-identical verification on all test assets\n");

    println!("[3/5] Running Interpreted DFDL benchmarks (1,000 warmup, 10,000 iterations)...");
    let warmup = 1_000;
    let iterations = 10_000;

    let b_interp_msg_parse = benchmark_op("DFDL parse (64B Track)", 64, warmup, iterations, || {
        parse_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("parse failed")
    });

    let b_interp_msg_unparse = benchmark_op("DFDL unparse (64B Track)", 64, warmup, iterations, || {
        unparse_faketdl(&msg_schema, &msg_doc).expect("unparse failed")
    });

    let b_interp_msg_rt = benchmark_op("DFDL roundtrip (64B Track)", 64, warmup, iterations, || {
        roundtrip_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("roundtrip failed")
    });

    let b_interp_file_parse = benchmark_op("DFDLFile parse (192B Multi)", 192, warmup, iterations, || {
        parse_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file parse failed")
    });

    let b_interp_file_unparse = benchmark_op("DFDLFile unparse (192B Multi)", 192, warmup, iterations, || {
        unparse_faketdl(&file_schema, &file_doc).expect("file unparse failed")
    });

    let b_interp_file_rt = benchmark_op("DFDLFile roundtrip (192B Multi)", 192, warmup, iterations, || {
        roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file roundtrip failed")
    });

    println!("\n[4/5] Running Native Rust Codec benchmarks (1,000 warmup, 10,000 iterations)...");

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

    println!("\n[5/5] Benchmark Results & Performance Gap Summary:\n");

    println!("--- Interpreted DFDL Engine Performance ---");
    let interp_results = [
        b_interp_msg_parse.clone(),
        b_interp_msg_unparse.clone(),
        b_interp_msg_rt.clone(),
        b_interp_file_parse.clone(),
        b_interp_file_unparse.clone(),
        b_interp_file_rt.clone(),
    ];
    print!("{}", format_benchmark_table(&interp_results));

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

    println!("\n--- Performance Gap Comparison (Interpreted vs Native Rust) ---");
    let comparison_pairs = [
        ("Parse / Decode (64B Track)", &b_interp_msg_parse, &b_native_msg_decode),
        ("Unparse / Encode (64B Track)", &b_interp_msg_unparse, &b_native_msg_encode),
        ("Roundtrip (64B Track)", &b_interp_msg_rt, &b_native_msg_rt),
        ("File Parse / Decode (192B)", &b_interp_file_parse, &b_native_file_decode),
        ("File Unparse / Encode (192B)", &b_interp_file_unparse, &b_native_file_encode),
        ("File Roundtrip (192B)", &b_interp_file_rt, &b_native_file_rt),
    ];
    print!("{}", format_comparison_table(&comparison_pairs));

    println!("\nLatency testing, native comparison, and data validation completed successfully.");
}
