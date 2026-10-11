//! Benchmarking and validation suite for AI-DFDL using FakeTDL military messaging assets.
//!
//! Assets sourced from:
//! - <https://github.com/DFDLSchemas/faketdl>
//! - <https://github.com/OpenDFDL/training-dfdl-using-fakeTDL>
//!
//! Provides automated latency measurement across parse, unparse, and roundtrip
//! execution phases, as well as strict infoset validation against reference XML items.

#![deny(missing_docs)]

use std::time::{Duration, Instant};

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::io::bitstream::{BitReader, BitWriter};
use dfdl_core::io::sink::VecByteSink;
use dfdl_core::io::source::SliceByteSource;
use dfdl_core::io::traits::{BitOrder, ByteOrder};
use dfdl_core::kernel::{ParserEngine, UnparserEngine};
use dfdl_core::limits::WorkBudget;
use dfdl_core::schema::ir::CompiledSchema;
use dfdl_core::types::QName;
use dfdl_schema::SchemaCompiler;
use dfdl_xml::{XmlEvent, XmlReader};

/// Hand-written native Rust encoder and decoder for FakeTDL messages.
pub mod native;
pub use native::*;

/// Embedded FakeTDL schemas.
pub mod schemas {
    /// Root fakeTDL schema defining `fakeTDL` and `fakeTDLFile` elements.
    pub const FAKETDL_XSD: &str = include_str!("../assets/schemas/fakeTDL.dfdl.xsd");
    /// Base format schema.
    pub const FAKETDL_BASE_XSD: &str = include_str!("../assets/schemas/fakeTDL-base.dfdl.xsd");
    /// Complex types for FakeTDL messages.
    pub const FAKETDL_TYPE_XSD: &str = include_str!("../assets/schemas/fakeTDLType.dfdl.xsd");
    /// Simple field types and constraints.
    pub const FAKETDL_FIELD_TYPES_XSD: &str =
        include_str!("../assets/schemas/fakeTDL-field-types.dfdl.xsd");
    /// Enumerations for entity type details.
    pub const FAKETDL_ENUM_XSD: &str =
        include_str!("../assets/schemas/fakeTDL-enum-entityTypeDetail.dfdl.xsd");
    /// Apache Daffodil general format definition.
    pub const GENERAL_FORMAT_XSD: &str =
        include_str!("../assets/schemas/DFDLGeneralFormat.dfdl.xsd");
    /// Apache Daffodil general format base definitions.
    pub const GENERAL_FORMAT_BASE_XSD: &str =
        include_str!("../assets/schemas/DFDLGeneralFormatBase.dfdl.xsd");
}

/// Embedded FakeTDL binary test data and reference XML infosets.
pub mod data {
    /// 3 FakeTDL messages (Track, Identity, Ack), 192 bytes.
    pub const TEST_FILE_01_DAT: &[u8] = include_bytes!("../assets/data/test_file_01.dat");
    /// Reference XML infoset for `test_file_01.dat`.
    pub const TEST_FILE_01_XML: &str = include_str!("../assets/data/test_file_01.xml");

    /// Single Track message, 64 bytes.
    pub const TEST_MSG_01_DAT: &[u8] = include_bytes!("../assets/data/test_msg_01.dat");
    /// Reference XML infoset for `test_msg_01.dat`.
    pub const TEST_MSG_01_XML: &str = include_str!("../assets/data/test_msg_01.xml");

    /// Single valid Track message from Lab 05, 64 bytes.
    pub const TEST_TRACK_GOOD_01_DAT: &[u8] =
        include_bytes!("../assets/data/test_track_good_01.dat");
    /// Reference XML infoset for `test_track_good_01.dat`.
    pub const TEST_TRACK_GOOD_01_XML: &str =
        include_str!("../assets/data/test_track_good_01.xml");
}

/// Resolves FakeTDL include/import schema locations against embedded schemas.
#[must_use]
pub fn resolve_faketdl_schema(location: &str) -> Option<String> {
    if location.ends_with("fakeTDL-base.dfdl.xsd") {
        Some(schemas::FAKETDL_BASE_XSD.to_string())
    } else if location.ends_with("fakeTDLType.dfdl.xsd") {
        Some(schemas::FAKETDL_TYPE_XSD.to_string())
    } else if location.ends_with("fakeTDL-field-types.dfdl.xsd") {
        Some(schemas::FAKETDL_FIELD_TYPES_XSD.to_string())
    } else if location.ends_with("fakeTDL-enum-entityTypeDetail.dfdl.xsd") {
        Some(schemas::FAKETDL_ENUM_XSD.to_string())
    } else if location.ends_with("DFDLGeneralFormat.dfdl.xsd") {
        Some(schemas::GENERAL_FORMAT_XSD.to_string())
    } else if location.ends_with("DFDLGeneralFormatBase.dfdl.xsd") {
        Some(schemas::GENERAL_FORMAT_BASE_XSD.to_string())
    } else if location.ends_with("fakeTDL.dfdl.xsd") {
        Some(schemas::FAKETDL_XSD.to_string())
    } else {
        None
    }
}

/// Compiles the FakeTDL schema for a given root element (`fakeTDL` or `fakeTDLFile`).
///
/// # Errors
/// Returns [`DFDLError`] if the schema compilation fails.
pub fn compile_faketdl_schema(root_element: &str) -> DFDLResult<CompiledSchema> {
    let compiler = SchemaCompiler::new();
    compiler.compile_str_with_resolver_and_root(
        schemas::FAKETDL_XSD,
        resolve_faketdl_schema,
        Some(root_element),
    )
}

/// Parses binary input using the compiled FakeTDL schema.
///
/// # Errors
/// Returns [`DFDLError`] if parsing fails.
pub fn parse_faketdl(schema: &CompiledSchema, data: &[u8]) -> DFDLResult<InfosetDocument> {
    let source = SliceByteSource::new(data);
    let mut reader = BitReader::new(
        source,
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut budget = WorkBudget::new(100_000);
    let mut parser = ParserEngine::new(schema, &mut reader, &mut budget);
    parser.parse_document()
}

/// Unparses an [`InfosetDocument`] into binary output using the compiled FakeTDL schema.
///
/// # Errors
/// Returns [`DFDLError`] if unparsing fails.
pub fn unparse_faketdl(schema: &CompiledSchema, doc: &InfosetDocument) -> DFDLResult<Vec<u8>> {
    let sink = VecByteSink::new();
    let mut writer = BitWriter::new(
        sink,
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut budget = WorkBudget::new(100_000);
    let mut unparser = UnparserEngine::new(schema, &mut writer, &mut budget);
    unparser.unparse_document(doc)?;
    let bytes = writer.into_sink().into_vec();
    Ok(bytes)
}

/// Performs a full parse and unparse roundtrip, returning the parsed document and resulting bytes.
///
/// # Errors
/// Returns [`DFDLError`] if either parse or unparse fails.
pub fn roundtrip_faketdl(
    schema: &CompiledSchema,
    data: &[u8],
) -> DFDLResult<(InfosetDocument, Vec<u8>)> {
    let doc = parse_faketdl(schema, data)?;
    let output = unparse_faketdl(schema, &doc)?;
    Ok((doc, output))
}

/// Parses an XML string into an [`InfosetDocument`] representation.
///
/// # Errors
/// Returns [`DFDLError`] if XML parsing fails or elements are malformed.
pub fn parse_xml_infoset(xml: &str) -> DFDLResult<InfosetDocument> {
    let mut reader = XmlReader::new(xml);
    let mut root_elem: Option<InfosetElement> = None;
    let mut elem_stack: Vec<InfosetElement> = Vec::new();

    while let Some(event) = reader.next_event()? {
        match event {
            XmlEvent::StartElement {
                name, attributes, ..
            } => {
                let mut nil_attr: Option<bool> = None;
                for attr in &attributes {
                    if attr.name.local_name == "nil" {
                        match attr.value.trim() {
                            "true" | "1" => nil_attr = Some(true),
                            "false" | "0" => nil_attr = Some(false),
                            _ => {}
                        }
                    }
                }
                let elem_name = if let Some(ref ns) = name.namespace {
                    QName::with_namespace(ns.as_str(), &name.local_name, name.prefix.as_deref())
                } else {
                    QName::local(&name.local_name)
                };
                let mut elem = InfosetElement::complex(elem_name);
                elem.nil_attribute = nil_attr;
                if nil_attr == Some(true) {
                    elem.state = ElementState::Nil;
                }
                elem_stack.push(elem);
            }
            XmlEvent::Text { ref content, .. } => {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    if let Some(top) = elem_stack.last_mut() {
                        let val = if trimmed == "true" {
                            DfdlValue::Boolean(true)
                        } else if trimmed == "false" {
                            DfdlValue::Boolean(false)
                        } else {
                            DfdlValue::String(trimmed.to_string())
                        };
                        top.state = ElementState::Value(val);
                    }
                }
            }
            XmlEvent::EndElement { name, .. } => {
                if let Some(finished) = elem_stack.pop() {
                    if finished.name.local_name == name.local_name {
                        if let Some(parent) = elem_stack.last_mut() {
                            parent.try_add_child(InfosetNode::Element(finished))?;
                        } else {
                            root_elem = Some(finished);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let root = root_elem.ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::Parse, "Empty or invalid XML infoset document")
    })?;

    Ok(InfosetDocument::with_root(root))
}

/// Recursively compares two infoset elements, returning `Ok(())` on match or a descriptive error.
///
/// # Errors
/// Returns an error message describing the exact point of difference.
pub fn compare_infoset_elements(
    actual: &InfosetElement,
    expected: &InfosetElement,
) -> Result<(), String> {
    if actual.name.local_name != expected.name.local_name {
        return Err(format!(
            "Element name mismatch: actual '{}', expected '{}'",
            actual.name.local_name, expected.name.local_name
        ));
    }

    let actual_is_nil = actual.state == ElementState::Nil || actual.nil_attribute == Some(true);
    let expected_is_nil =
        expected.state == ElementState::Nil || expected.nil_attribute == Some(true);
    if actual_is_nil != expected_is_nil {
        return Err(format!(
            "Element '{}' nil status mismatch: actual nil={}, expected nil={}",
            actual.name.local_name, actual_is_nil, expected_is_nil
        ));
    }

    match (&actual.state, &expected.state) {
        (ElementState::Value(a_val), ElementState::Value(e_val)) => {
            let a_str = a_val.to_string();
            let e_str = e_val.to_string();
            if a_str != e_str {
                // Check if float equivalence applies (e.g. 41.0 vs 41)
                let a_num = a_str.parse::<f64>();
                let e_num = e_str.parse::<f64>();
                match (a_num, e_num) {
                    (Ok(a_f), Ok(e_f)) => {
                        if (a_f - e_f).abs() > 1e-4 {
                            return Err(format!(
                                "Element '{}' numeric mismatch: actual '{}', expected '{}'",
                                actual.name.local_name, a_str, e_str
                            ));
                        }
                    }
                    _ => {
                        return Err(format!(
                            "Element '{}' value mismatch: actual '{}', expected '{}'",
                            actual.name.local_name, a_str, e_str
                        ));
                    }
                }
            }
        }
        (ElementState::Value(a_val), ElementState::NoValue) if !expected.children.is_empty() => {
            return Err(format!(
                "Element '{}' state mismatch: actual has scalar value '{}' while expected is complex",
                actual.name.local_name, a_val
            ));
        }
        (ElementState::NoValue, ElementState::Value(e_val)) if !actual.children.is_empty() => {
            return Err(format!(
                "Element '{}' state mismatch: actual is complex while expected has scalar value '{}'",
                actual.name.local_name, e_val
            ));
        }
        _ => {}
    }

    let actual_children: Vec<&InfosetElement> = actual
        .children
        .iter()
        .filter_map(|c| match c {
            InfosetNode::Element(e) if !e.is_hidden => Some(e),
            _ => None,
        })
        .collect();

    let expected_children: Vec<&InfosetElement> = expected
        .children
        .iter()
        .filter_map(|c| match c {
            InfosetNode::Element(e) if !e.is_hidden => Some(e),
            _ => None,
        })
        .collect();

    if actual_children.len() != expected_children.len() {
        return Err(format!(
            "Element '{}' child count mismatch: actual {} children, expected {} children",
            actual.name.local_name,
            actual_children.len(),
            expected_children.len()
        ));
    }

    for (i, (a_child, e_child)) in actual_children
        .iter()
        .zip(expected_children.iter())
        .enumerate()
    {
        compare_infoset_elements(a_child, e_child).map_err(|err| {
            format!(
                "Child #{} of '{}' differed -> {}",
                i, actual.name.local_name, err
            )
        })?;
    }

    Ok(())
}

/// Validates that an actual [`InfosetDocument`] matches the expected reference XML infoset.
///
/// # Errors
/// Returns an error message describing the validation discrepancy.
pub fn validate_infoset(actual_doc: &InfosetDocument, expected_xml: &str) -> Result<(), String> {
    let expected_doc = parse_xml_infoset(expected_xml)
        .map_err(|e| format!("Failed to parse expected XML infoset: {:?}", e))?;

    let actual_root = actual_doc
        .root
        .as_ref()
        .ok_or_else(|| "Actual infoset document has no root element".to_string())?;

    let expected_root = expected_doc
        .root
        .as_ref()
        .ok_or_else(|| "Expected infoset document has no root element".to_string())?;

    compare_infoset_elements(actual_root, expected_root)
}

/// Latency and throughput benchmark results for a DFDL operation.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Name or description of the benchmark operation.
    pub name: String,
    /// Number of iterations measured.
    pub iterations: usize,
    /// Total duration of all measured iterations.
    pub total_duration: Duration,
    /// Minimum iteration duration.
    pub min: Duration,
    /// Maximum iteration duration.
    pub max: Duration,
    /// Mean (average) iteration duration.
    pub mean: Duration,
    /// Median (50th percentile) iteration duration.
    pub median: Duration,
    /// 95th percentile iteration duration.
    pub p95: Duration,
    /// 99th percentile iteration duration.
    pub p99: Duration,
    /// Standard deviation of iteration duration.
    pub stddev: Duration,
    /// Throughput in operations per second.
    pub ops_per_sec: f64,
    /// Throughput in megabytes per second (based on bytes per operation).
    pub mb_per_sec: f64,
}

/// Benchmarks a single closure operation across warmup and measured iterations.
///
/// Records fine-grained nanosecond latencies, calculates statistical percentiles,
/// and computes ops/sec and bandwidth (MB/sec).
pub fn benchmark_op<F, R>(
    name: &str,
    bytes_per_op: usize,
    warmup_iters: usize,
    bench_iters: usize,
    mut op: F,
) -> BenchmarkResult
where
    F: FnMut() -> R,
{
    // Warmup phase
    for _ in 0..warmup_iters {
        let res = op();
        std::hint::black_box(res);
    }

    // Measurement phase
    let mut durations = Vec::with_capacity(bench_iters);
    let start_all = Instant::now();

    for _ in 0..bench_iters {
        let t0 = Instant::now();
        let res = op();
        let elapsed = t0.elapsed();
        std::hint::black_box(res);
        durations.push(elapsed);
    }

    let total_duration = start_all.elapsed();

    durations.sort_unstable();

    let min = *durations.first().unwrap_or(&Duration::ZERO);
    let max = *durations.last().unwrap_or(&Duration::ZERO);

    let sum_nanos: u128 = durations.iter().map(|d| d.as_nanos()).sum();
    let mean_nanos = (sum_nanos as f64) / (bench_iters as f64);
    let mean = Duration::from_nanos(mean_nanos as u64);

    let median_idx = bench_iters / 2;
    let median = durations.get(median_idx).copied().unwrap_or(Duration::ZERO);

    let p95_idx = ((bench_iters as f64) * 0.95) as usize;
    let p95 = durations
        .get(p95_idx.min(bench_iters.saturating_sub(1)))
        .copied()
        .unwrap_or(Duration::ZERO);

    let p99_idx = ((bench_iters as f64) * 0.99) as usize;
    let p99 = durations
        .get(p99_idx.min(bench_iters.saturating_sub(1)))
        .copied()
        .unwrap_or(Duration::ZERO);

    // Standard deviation
    let variance_nanos: f64 = durations
        .iter()
        .map(|d| {
            let diff = (d.as_nanos() as f64) - mean_nanos;
            diff * diff
        })
        .sum::<f64>()
        / (bench_iters as f64);
    let stddev = Duration::from_nanos(variance_nanos.sqrt() as u64);

    let total_secs = total_duration.as_secs_f64();
    let ops_per_sec = (bench_iters as f64) / total_secs;
    let total_bytes = (bytes_per_op as f64) * (bench_iters as f64);
    let mb_per_sec = (total_bytes / (1024.0 * 1024.0)) / total_secs;

    BenchmarkResult {
        name: name.to_string(),
        iterations: bench_iters,
        total_duration,
        min,
        max,
        mean,
        median,
        p95,
        p99,
        stddev,
        ops_per_sec,
        mb_per_sec,
    }
}

/// Formats a duration cleanly into nanoseconds, microseconds, or milliseconds.
#[must_use]
pub fn format_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1_000 {
        format!("{} ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.2} µs", (nanos as f64) / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2} ms", (nanos as f64) / 1_000_000.0)
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

/// Formats a slice of [`BenchmarkResult`] items into a structured ASCII table.
#[must_use]
pub fn format_benchmark_table(results: &[BenchmarkResult]) -> String {
    let mut out = String::new();
    let sep = "+----------------------------------+--------+-----------+-----------+-----------+-----------+-----------+--------------+------------+\n";
    out.push_str(sep);
    out.push_str("| Operation                        | Iters  | Mean      | Median    | Min       | p95       | p99       | Throughput   | Bandwidth  |\n");
    out.push_str(sep);

    for r in results {
        out.push_str(&format!(
            "| {:<32} | {:>6} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>10.0} o/s | {:>8.2} MB/s |\n",
            r.name,
            r.iterations,
            format_duration(r.mean),
            format_duration(r.median),
            format_duration(r.min),
            format_duration(r.p95),
            format_duration(r.p99),
            r.ops_per_sec,
            r.mb_per_sec,
        ));
    }

    out.push_str(sep);
    out
}

/// Formats a comparison between interpreted and native benchmark results into an ASCII table.
#[must_use]
pub fn format_comparison_table(
    pairs: &[(&str, &BenchmarkResult, &BenchmarkResult)],
) -> String {
    let mut out = String::new();
    let sep = "+----------------------------------+-------------------+-------------------+--------------+---------------------+\n";
    out.push_str(sep);
    out.push_str("| Operation                        | Interpreted Mean  | Native Mean       | Speedup      | Native Throughput   |\n");
    out.push_str(sep);

    for (label, interp, native) in pairs {
        let interp_secs = interp.mean.as_secs_f64().max(1e-12);
        let native_secs = native.mean.as_secs_f64().max(1e-12);
        let speedup = interp_secs / native_secs;

        out.push_str(&format!(
            "| {:<32} | {:>17} | {:>17} | {:>10.1}x | {:>15.0} o/s |\n",
            label,
            format_duration(interp.mean),
            format_duration(native.mean),
            speedup,
            native.ops_per_sec,
        ));
    }

    out.push_str(sep);
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_track_msg_01_roundtrip() {
        let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL");
        let (doc, bytes) =
            roundtrip_faketdl(&schema, data::TEST_MSG_01_DAT).expect("roundtrip msg_01");
        assert_eq!(bytes.as_slice(), data::TEST_MSG_01_DAT);
        assert_eq!(doc.total_nodes, 18);
    }

    #[test]
    fn test_track_msg_01_xml_validation() {
        let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL");
        let doc = parse_faketdl(&schema, data::TEST_MSG_01_DAT).expect("parse msg_01");
        validate_infoset(&doc, data::TEST_MSG_01_XML).expect("validate msg_01 against XML");
    }

    #[test]
    fn test_track_good_01_roundtrip() {
        let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL");
        let (doc, bytes) =
            roundtrip_faketdl(&schema, data::TEST_TRACK_GOOD_01_DAT).expect("roundtrip good_01");
        assert_eq!(bytes.as_slice(), data::TEST_TRACK_GOOD_01_DAT);
        assert_eq!(doc.total_nodes, 18);
    }

    #[test]
    fn test_track_good_01_xml_validation() {
        let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL");
        let doc = parse_faketdl(&schema, data::TEST_TRACK_GOOD_01_DAT).expect("parse good_01");
        validate_infoset(&doc, data::TEST_TRACK_GOOD_01_XML).expect("validate good_01 against XML");
    }

    #[test]
    fn test_file_01_roundtrip() {
        let schema = compile_faketdl_schema("fakeTDLFile").expect("compile fakeTDLFile");
        let (doc, bytes) =
            roundtrip_faketdl(&schema, data::TEST_FILE_01_DAT).expect("roundtrip file_01");
        assert_eq!(bytes.as_slice(), data::TEST_FILE_01_DAT);
        assert_eq!(doc.total_nodes, 39);
    }

    #[test]
    fn test_file_01_xml_validation() {
        let schema = compile_faketdl_schema("fakeTDLFile").expect("compile fakeTDLFile");
        let doc = parse_faketdl(&schema, data::TEST_FILE_01_DAT).expect("parse file_01");
        validate_infoset(&doc, data::TEST_FILE_01_XML).expect("validate file_01 against XML");
    }

    #[test]
    fn test_benchmark_stats() {
        let res = benchmark_op("dummy_test", 100, 10, 100, || {
            let mut x = 0u64;
            for i in 0..100 {
                x = x.wrapping_add(i);
            }
            x
        });
        assert_eq!(res.iterations, 100);
        assert!(res.min <= res.mean);
        assert!(res.mean <= res.max);
        assert!(res.ops_per_sec > 0.0);
    }

    #[test]
    fn test_native_track_msg_01_roundtrip() {
        let msg = FakeTdlMessage::decode(data::TEST_MSG_01_DAT).expect("native decode msg_01");
        if let FakeTdlMessage::Track(t) = &msg {
            assert_eq!(&t.source, b"AG123");
            assert_eq!(t.send_time, TimeOfDay::new(1, 2, 3));
            assert!(!t.must_ack);
            assert_eq!(t.message_id, 1);
            assert_eq!(&t.track_num, b"UU777");
            assert_eq!(t.point_type, TrackPointType::Waypoint);
            assert_eq!(t.quality, TrackPointQuality::Actual);
            assert_eq!(t.course, 75);
            assert_eq!(t.speed, 200);
        }
        assert!(matches!(msg, FakeTdlMessage::Track(_)));
        let encoded = msg.encode();
        assert_eq!(encoded.as_slice(), data::TEST_MSG_01_DAT);
    }

    #[test]
    fn test_native_track_good_01_roundtrip() {
        let msg = FakeTdlMessage::decode(data::TEST_TRACK_GOOD_01_DAT).expect("native decode good_01");
        let encoded = msg.encode();
        assert_eq!(encoded.as_slice(), data::TEST_TRACK_GOOD_01_DAT);
    }

    #[test]
    fn test_native_file_01_roundtrip() {
        let file = FakeTdlFile::decode(data::TEST_FILE_01_DAT).expect("native decode file_01");
        assert_eq!(file.messages.len(), 3);
        assert!(matches!(file.messages.first(), Some(FakeTdlMessage::Track(_))));
        assert!(matches!(file.messages.get(1), Some(FakeTdlMessage::Identity(_))));
        assert!(matches!(file.messages.get(2), Some(FakeTdlMessage::Ack(_))));

        let encoded = file.encode();
        assert_eq!(encoded.as_slice(), data::TEST_FILE_01_DAT);
    }
}
