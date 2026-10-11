//! Command-line benchmark runner for AI-DFDL latency and throughput testing.
//!
//! Evaluates parse, unparse, and roundtrip latency on FakeTDL tactical data link assets,
//! validating byte accuracy and infoset conformance against reference XML datasets.

#![deny(missing_docs)]
#![allow(clippy::expect_used)]

use dfdl_benchmark::{
    benchmark_op, compile_faketdl_schema, data, format_benchmark_table, parse_faketdl,
    roundtrip_faketdl, unparse_faketdl, validate_infoset, BenchmarkResult,
};

fn main() {
    println!("================================================================================");
    println!("                   AI-DFDL Tactical Messaging Latency Benchmark                  ");
    println!("                   Assets: OpenDFDL & DFDLSchemas / fakeTDL                     ");
    println!("================================================================================\n");

    println!("[1/4] Compiling FakeTDL schemas...");
    let t0 = std::time::Instant::now();
    let msg_schema =
        compile_faketdl_schema("fakeTDL").expect("Failed to compile 'fakeTDL' schema");
    let file_schema =
        compile_faketdl_schema("fakeTDLFile").expect("Failed to compile 'fakeTDLFile' schema");
    println!("      Compiled schemas in {:.2} ms\n", t0.elapsed().as_secs_f64() * 1000.0);

    println!("[2/4] Validating data roundtrips and reference XML infosets...");

    // Validate test_msg_01
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

    // Validate test_track_good_01
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

    // Validate test_file_01
    let (file_doc, file_bytes) = roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT)
        .expect("Roundtrip test_file_01 failed");
    assert_eq!(
        file_bytes.as_slice(),
        data::TEST_FILE_01_DAT,
        "test_file_01 binary roundtrip mismatch"
    );
    validate_infoset(&file_doc, data::TEST_FILE_01_XML)
        .expect("test_file_01 XML infoset validation failed");
    println!("  [PASS] test_file_01.dat (192B Multi-msg): byte-identical roundtrip & XML infoset verified\n");

    println!("[3/4] Running latency and throughput benchmarks (1,000 warmup, 10,000 iterations)...");

    let warmup = 1_000;
    let iterations = 10_000;
    let mut results: Vec<BenchmarkResult> = Vec::new();

    // 1. Single message parse
    let b_msg_parse = benchmark_op("fakeTDL parse (64B Track)", 64, warmup, iterations, || {
        parse_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("parse failed")
    });
    results.push(b_msg_parse);

    // 2. Single message unparse
    let b_msg_unparse =
        benchmark_op("fakeTDL unparse (64B Track)", 64, warmup, iterations, || {
            unparse_faketdl(&msg_schema, &msg_doc).expect("unparse failed")
        });
    results.push(b_msg_unparse);

    // 3. Single message roundtrip
    let b_msg_rt = benchmark_op("fakeTDL roundtrip (64B)", 64, warmup, iterations, || {
        roundtrip_faketdl(&msg_schema, data::TEST_MSG_01_DAT).expect("roundtrip failed")
    });
    results.push(b_msg_rt);

    // 4. Multi-message file parse
    let b_file_parse = benchmark_op(
        "fakeTDLFile parse (192B Multi)",
        192,
        warmup,
        iterations,
        || parse_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file parse failed"),
    );
    results.push(b_file_parse);

    // 5. Multi-message file unparse
    let b_file_unparse = benchmark_op(
        "fakeTDLFile unparse (192B Multi)",
        192,
        warmup,
        iterations,
        || unparse_faketdl(&file_schema, &file_doc).expect("file unparse failed"),
    );
    results.push(b_file_unparse);

    // 6. Multi-message file roundtrip
    let b_file_rt = benchmark_op(
        "fakeTDLFile roundtrip (192B)",
        192,
        warmup,
        iterations,
        || roundtrip_faketdl(&file_schema, data::TEST_FILE_01_DAT).expect("file roundtrip failed"),
    );
    results.push(b_file_rt);

    println!("\n[4/4] Benchmark Results Summary:\n");
    print!("{}", format_benchmark_table(&results));
    println!("\nLatency testing and data validation completed successfully.");
}
