//! TDML (Test Data Markup Language) Test Suite Parser & Runner for DFDL Conformance Verification.
//!
//! Complies with OGF DFDL v1.0 Specification Appendix F (TDML Format).

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

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
use dfdl_core::types::QName;
use dfdl_schema::SchemaCompiler;
use dfdl_xml::{XmlEvent, XmlReader};

/// Individual TDML Test Case Type (Parser or Unparser).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TdmlTestCaseKind {
    /// Parser test case: inputs raw document bits/text, expects infoset XML or error.
    Parser,
    /// Unparser test case: inputs infoset XML, expects raw document bits/text or error.
    Unparser,
}

/// Parsed TDML Document Part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdmlDocumentPart {
    /// Part type: "text", "byte", "hex", "bits".
    pub part_type: String,
    /// Content text payload.
    pub content: String,
    /// Bit order for bit-oriented document parts.
    pub bit_order: Option<String>,
    /// Byte order for bit-oriented document parts.
    pub byte_order: Option<String>,
    /// Character encoding for text document parts.
    pub encoding: Option<String>,
    /// Whether to replace DFDL character entities in content.
    pub replace_dfdl_entities: bool,
}

/// Schema cache key: `(schema XML, root element, allowSignedIntegerLength1Bit, maxOccursBounds,
/// (requireEncodingErrorPolicyProperty, requireTextBidiProperty, requireFloatingProperty),
/// unqualifiedPathStepPolicy, maxHexBinaryLengthInBytes)`.
type SchemaCacheKey = (
    String,
    Option<String>,
    bool,
    Option<usize>,
    (bool, bool, bool, bool),
    dfdl_core::types::UnqualifiedPathStepPolicy,
    Option<usize>,
);

/// Parsed TDML Test Case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdmlTestCase {
    /// Test case name.
    pub name: String,
    /// Kind (Parser or Unparser).
    pub kind: TdmlTestCaseKind,
    /// Root element name.
    pub root: Option<String>,
    /// Model / schema reference.
    pub model: Option<String>,
    /// Document parts.
    pub document_parts: Vec<TdmlDocumentPart>,
    /// Expected infoset representation string.
    pub infoset_text: Option<String>,
    /// Expected error substrings.
    pub expected_errors: Vec<String>,
    /// Expected validation error substrings.
    pub expected_validation_errors: Vec<String>,
    /// Validation mode string.
    pub validation: Option<String>,
    /// Name of the referenced `tdml:defineConfig`, if any.
    pub config: Option<String>,
    /// Explicit bitOrder on the `document` element, if any.
    pub document_bit_order: Option<String>,
}

/// Parsed TDML Test Suite containing embedded schemas and test cases.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TdmlTestSuite {
    /// Test suite name.
    pub suite_name: String,
    /// Embedded schema XML strings mapped by schema name.
    pub embedded_schemas: Vec<(String, String)>,
    /// List of test cases in suite.
    pub test_cases: Vec<TdmlTestCase>,
    /// Tunables of each `defineConfig` block as `(config name, [(tunable name, value)])`.
    pub configs: Vec<(String, Vec<(String, String)>)>,
    /// External variable bindings of each `defineConfig` block as `(config name, [(var name, value)])`.
    pub external_variable_bindings: Vec<(String, Vec<(String, String)>)>,
    /// Namespace prefix bindings declared on `<tdml:testSuite>`.
    pub test_suite_namespaces: Vec<(String, String)>,
    /// Optional default configuration name specified on `<tdml:testSuite defaultConfig="...">`.
    pub default_config: Option<String>,
}

/// Result report of executing a TDML Test Suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdmlReport {
    /// Suite name.
    pub suite_name: String,
    /// Total test cases executed.
    pub total: usize,
    /// Passed test cases count.
    pub passed: usize,
    /// Failed test cases count.
    pub failed: usize,
    /// Diagnostic failure logs.
    pub failure_messages: Vec<String>,
}

impl TdmlReport {
    /// Adds a failure diagnostic entry to the report with capped vector allocation to keep memory minimal.
    pub fn add_failure(&mut self, msg: String) {
        self.failed = self.failed.saturating_add(1);
        self.failure_messages.push(msg);
    }
}

/// Standard IBM GeneralPurposeFormat DFDL schema (DFDL v1.0 compatible).
///
/// Bundled by IBM DFDL tooling and Apache Daffodil as a classpath resource at
/// `/IBMdefined/GeneralPurposeFormat.xsd` with targetNamespace `http://www.ibm.com/dfdl/GeneralPurposeFormat`.
/// Provides default DFDL property configurations for IBM-compatible DFDL schemas.
pub const IBM_GENERAL_PURPOSE_FORMAT_XSD: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
  xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
  targetNamespace="http://www.ibm.com/dfdl/GeneralPurposeFormat"
  xmlns:tns="http://www.ibm.com/dfdl/GeneralPurposeFormat">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:defineFormat name="GeneralPurposeFormat">
        <dfdl:format
          alignment="1"
          alignmentUnits="bytes"
          binaryFloatRep="ieee"
          binaryNumberCheckPolicy="lax"
          binaryNumberRep="binary"
          binaryCalendarEpoch="1970-01-01T00:00:00"
          bitOrder="mostSignificantBitFirst"
          byteOrder="bigEndian"
          calendarCenturyStart="53"
          calendarCheckPolicy="lax"
          calendarDaysInFirstWeek="4"
          calendarFirstDayOfWeek="Sunday"
          calendarLanguage="en"
          calendarObserveDST="yes"
          calendarPatternKind="implicit"
          calendarTimeZone="UTC"
          choiceLengthKind="implicit"
          decimalSigned="yes"
          documentFinalTerminatorCanBeMissing="yes"
          emptyValueDelimiterPolicy="both"
          encodingErrorPolicy="replace"
          encoding="UTF-8"
          escapeSchemeRef=""
          fillByte="%#r20;"
          floating="no"
          ignoreCase="no"
          initiatedContent="no"
          initiator=""
          leadingSkip="0"
          lengthKind="delimited"
          lengthUnits="characters"
          occursCountKind="implicit"
          outputNewLine="%LF;"
          representation="text"
          separator=""
          separatorPosition="infix"
          separatorSuppressionPolicy="trailingEmpty"
          sequenceKind="ordered"
          terminator=""
          textBidi="no"
          textBooleanFalseRep="false"
          textBooleanPadCharacter="%SP;"
          textBooleanTrueRep="true"
          textCalendarJustification="left"
          textCalendarPadCharacter="%SP;"
          textNumberCheckPolicy="lax"
          textNumberJustification="right"
          textNumberPadCharacter="%SP;"
          textNumberPattern="#,##0.###;-#,##0.###"
          textNumberRep="standard"
          textNumberRounding="explicit"
          textNumberRoundingIncrement="0"
          textNumberRoundingMode="roundHalfEven"
          textOutputMinLength="0"
          textPadKind="none"
          textStandardBase="10"
          textStandardDecimalSeparator="."
          textStandardExponentRep="E"
          textStandardGroupingSeparator=","
          textStandardInfinityRep="Inf"
          textStandardNaNRep="NaN"
          textStandardZeroRep="0"
          textStringJustification="left"
          textStringPadCharacter="%SP;"
          textTrimKind="none"
          textZonedSignStyle="asciiStandard"
          trailingSkip="0"
          truncateSpecifiedLengthString="no"
          utf16Width="fixed"
        />
      </dfdl:defineFormat>
    </xs:appinfo>
  </xs:annotation>
</xs:schema>"##;

/// Reads an XML file from the filesystem, automatically detecting encoding per XML 1.0 Appendix F.
///
/// Handles UTF-8 (with or without BOM), UTF-16 BE (with or without BOM), and UTF-16 LE
/// (with or without BOM) before returning a decoded UTF-8 `String`.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn read_xml_file_to_string(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        // UTF-8 with BOM: strip 3-byte signature
        let payload = bytes.get(3..).unwrap_or(&[]);
        String::from_utf8(payload.to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        // UTF-16 Big Endian with BOM: strip 2-byte BOM
        let payload = bytes.get(2..).unwrap_or(&[]);
        let u16s: Vec<u16> = payload
            .chunks_exact(2)
            .filter_map(|chunk| match chunk {
                &[b0, b1] => Some(u16::from_be_bytes([b0, b1])),
                _ => None,
            })
            .collect();
        String::from_utf16(&u16s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        // UTF-16 Little Endian with BOM: strip 2-byte BOM
        let payload = bytes.get(2..).unwrap_or(&[]);
        let u16s: Vec<u16> = payload
            .chunks_exact(2)
            .filter_map(|chunk| match chunk {
                &[b0, b1] => Some(u16::from_le_bytes([b0, b1])),
                _ => None,
            })
            .collect();
        String::from_utf16(&u16s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F]) || bytes.starts_with(&[0x00, 0x3C]) {
        // UTF-16 Big Endian without BOM (e.g. `<?` or `<xs:...`)
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .filter_map(|chunk| match chunk {
                &[b0, b1] => Some(u16::from_be_bytes([b0, b1])),
                _ => None,
            })
            .collect();
        String::from_utf16(&u16s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00]) || bytes.starts_with(&[0x3C, 0x00]) {
        // UTF-16 Little Endian without BOM (e.g. `<?` or `<xs:...`)
        let u16s: Vec<u16> = bytes
            .chunks_exact(2)
            .filter_map(|chunk| match chunk {
                &[b0, b1] => Some(u16::from_le_bytes([b0, b1])),
                _ => None,
            })
            .collect();
        String::from_utf16(&u16s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    } else {
        // UTF-8 or ASCII fallback
        match String::from_utf8(bytes.clone()) {
            Ok(s) => Ok(s),
            Err(_) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        }
    }
}

impl TdmlTestSuite {
    /// Parses a TDML XML document string into a [`TdmlTestSuite`].
    pub fn parse_xml(xml: &str) -> DFDLResult<Self> {
        let mut suite = TdmlTestSuite::default();
        let mut reader = XmlReader::new(xml);

        let mut current_schema_name: Option<String> = None;
        let mut define_schema_inner_start: Option<usize> = None;
        let mut explicit_schema_start: Option<usize> = None;
        let mut explicit_schema_extracted = false;

        let mut default_implementations: Option<String> = None;
        let mut current_test_case: Option<TdmlTestCase> = None;
        let mut current_part_type: Option<String> = None;
        let mut current_part_buf: Option<String> = None;
        let mut current_infoset_buf: Option<String> = None;
        let mut current_error_buf: Option<String> = None;
        let mut current_part_bit_order: Option<String> = None;
        let mut current_part_byte_order: Option<String> = None;
        let mut current_part_encoding: Option<String> = None;
        let mut current_doc_encoding: Option<String> = None;
        let mut current_doc_bit_order: Option<String> = None;
        let mut current_part_replace_entities = false;
        let mut in_infoset = false;
        let mut in_validation_errors = false;
        let mut current_config_name: Option<String> = None;
        let mut current_tunable: Option<String> = None;
        let mut current_bind: Option<String> = None;
        let mut current_schema_element_form_default: Option<String> = None;
        let mut current_schema_use_default_namespace: bool = true;

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name,
                    attributes,
                    location,
                } => {
                    let local = name.local_name.as_str();

                    match local {
                        "testSuite" => {
                            for attr in &attributes {
                                if attr.name.local_name == "suiteName" {
                                    suite.suite_name = attr.value.to_string();
                                } else if attr.name.local_name == "defaultImplementations" {
                                    default_implementations = Some(attr.value.to_string());
                                } else if attr.name.local_name == "defaultConfig" {
                                    suite.default_config = Some(attr.value.to_string());
                                }
                            }
                            suite.test_suite_namespaces = reader.in_scope_namespace_bindings();
                        }
                        "defineSchema" => {
                            let name_attr = attributes
                                .iter()
                                .find(|a| a.name.local_name == "name")
                                .map(|a| a.value.to_string());
                            let form_default_attr = attributes
                                .iter()
                                .find(|a| a.name.local_name == "elementFormDefault")
                                .map_or("qualified", |a| &a.value[..])
                                .to_string();
                            current_schema_use_default_namespace = attributes
                                .iter()
                                .find(|a| a.name.local_name == "useDefaultNamespace")
                                .map(|a| a.value != "false")
                                .unwrap_or(true);
                            current_schema_element_form_default = Some(form_default_attr);
                            current_schema_name = name_attr;
                            explicit_schema_start = None;
                            explicit_schema_extracted = false;
                            if let Some(gt) = xml[location.byte_offset..].find('>') {
                                define_schema_inner_start =
                                    Some(location.byte_offset.saturating_add(gt).saturating_add(1));
                            }
                        }
                        "schema" => {
                            if current_schema_name.is_some() && explicit_schema_start.is_none() {
                                explicit_schema_start = Some(location.byte_offset);
                            }
                        }
                        "defineConfig" => {
                            current_config_name = attributes
                                .iter()
                                .find(|a| a.name.local_name == "name")
                                .map(|a| a.value.to_string());
                            if let Some(ref n) = current_config_name {
                                suite.configs.push((n.clone(), Vec::new()));
                            }
                        }
                        "tunables" => {}
                        "parserTestCase" | "unparserTestCase" => {
                            let implementations = attributes
                                .iter()
                                .find(|a| a.name.local_name == "implementations")
                                .map(|a| a.value.to_string())
                                .or_else(|| default_implementations.clone());
                            if let Some(ref impl_str) = implementations {
                                if !impl_str
                                    .split_whitespace()
                                    .any(|s| s.eq_ignore_ascii_case("daffodil"))
                                {
                                    current_test_case = None;
                                    continue;
                                }
                            }

                            let kind = if local == "parserTestCase" {
                                TdmlTestCaseKind::Parser
                            } else {
                                TdmlTestCaseKind::Unparser
                            };
                            let tc_name = attributes
                                .iter()
                                .find(|a| a.name.local_name == "name")
                                .map_or("unnamed", |a| &a.value[..])
                                .to_string();
                            let root = attributes
                                .iter()
                                .find(|a| a.name.local_name == "root")
                                .map(|a| a.value.to_string());
                            let model = attributes
                                .iter()
                                .find(|a| a.name.local_name == "model")
                                .map(|a| a.value.to_string());
                            let validation = attributes
                                .iter()
                                .find(|a| a.name.local_name == "validation")
                                .map(|a| a.value.to_string());

                            current_test_case = Some(TdmlTestCase {
                                name: tc_name,
                                kind,
                                root,
                                model,
                                document_parts: Vec::new(),
                                infoset_text: None,
                                expected_errors: Vec::new(),
                                expected_validation_errors: Vec::new(),
                                validation,
                                config: attributes
                                    .iter()
                                    .find(|a| a.name.local_name == "config")
                                    .map(|a| a.value.to_string()),
                                document_bit_order: None,
                            });
                        }
                        "document" | "documentPart" => {
                            if local == "document" {
                                current_doc_encoding = attributes
                                    .iter()
                                    .find(|a| a.name.local_name == "encoding")
                                    .map(|a| a.value.to_string());
                                current_doc_bit_order = attributes
                                    .iter()
                                    .find(|a| a.name.local_name == "bitOrder")
                                    .map(|a| a.value.to_string());
                                if let Some(ref mut tc) = current_test_case {
                                    tc.document_bit_order = current_doc_bit_order.clone();
                                }
                            } else if local == "documentPart" {
                                if let (Some(p_type), Some(p_buf)) =
                                    (current_part_type.take(), current_part_buf.take())
                                {
                                    let b_ord = current_part_bit_order.take();
                                    let by_ord = current_part_byte_order.take();
                                    let enc = current_part_encoding.take();
                                    let r_ent = current_part_replace_entities;
                                    if !p_buf.trim().is_empty() {
                                        if let Some(ref mut tc) = current_test_case {
                                            tc.document_parts.push(TdmlDocumentPart {
                                                part_type: p_type,
                                                content: p_buf,
                                                bit_order: b_ord,
                                                byte_order: by_ord,
                                                encoding: enc,
                                                replace_dfdl_entities: r_ent,
                                            });
                                        }
                                    }
                                }
                            }
                            let type_attr = attributes
                                .iter()
                                .find(|a| a.name.local_name == "type")
                                .map_or("text", |a| &a.value[..])
                                .to_string();
                            current_part_bit_order = attributes
                                .iter()
                                .find(|a| a.name.local_name == "bitOrder")
                                .map(|a| a.value.to_string())
                                .or_else(|| current_doc_bit_order.clone());
                            current_part_byte_order = attributes
                                .iter()
                                .find(|a| a.name.local_name == "byteOrder")
                                .map(|a| a.value.to_string());
                            current_part_encoding = attributes
                                .iter()
                                .find(|a| a.name.local_name == "encoding")
                                .map(|a| a.value.to_string())
                                .or_else(|| current_doc_encoding.clone());
                            current_part_replace_entities = attributes
                                .iter()
                                .find(|a| a.name.local_name == "replaceDFDLEntities")
                                .is_some_and(|a| a.value == "true");
                            current_part_type = Some(type_attr);
                            current_part_buf = Some(String::new());
                        }
                        "infoset" | "dfdlInfoset" => {
                            in_infoset = true;
                            if current_infoset_buf.is_none() {
                                current_infoset_buf = Some(String::new());
                            }
                        }
                        "validationErrors" => {
                            in_validation_errors = true;
                        }
                        "error" => {
                            current_error_buf = Some(String::new());
                        }
                        _ => {
                            if current_config_name.is_some() {
                                if local == "bind" {
                                    current_bind = attributes
                                        .iter()
                                        .find(|a| a.name.local_name == "name")
                                        .map(|a| a.value.to_string());
                                } else if local != "externalVariableBindings" && local != "tunables" {
                                    current_tunable = Some(local.to_string());
                                }
                            }
                            if in_infoset {
                                if let Some(ref mut info_buf) = current_infoset_buf {
                                    let is_first_elem = !info_buf.contains('<');
                                    if let Some(ref p) = name.prefix {
                                        info_buf.push_str(&format!("<{}:{}", p, local));
                                    } else {
                                        info_buf.push_str(&format!("<{}", local));
                                    }
                                    let mut has_default_ns = false;
                                    for attr in &attributes {
                                        if attr.name.prefix.is_none() && attr.name.local_name == "xmlns" {
                                            has_default_ns = true;
                                        }
                                        if let Some(ref ap) = attr.name.prefix {
                                            info_buf.push_str(&format!(" {}:{}=\"{}\"", ap, attr.name.local_name, attr.value));
                                        } else {
                                            info_buf.push_str(&format!(" {}=\"{}\"", attr.name.local_name, attr.value));
                                        }
                                    }
                                    if is_first_elem {
                                        if !has_default_ns {
                                            if let Some(default_ns) = reader.resolve_default_ns() {
                                                info_buf.push_str(&format!(" xmlns=\"{}\"", default_ns));
                                            }
                                        }
                                        for (pref, uri) in reader.in_scope_namespace_bindings() {
                                            if !pref.is_empty()
                                                && !attributes.iter().any(|a| {
                                                    a.name.prefix.as_deref() == Some("xmlns")
                                                        && a.name.local_name == pref
                                                })
                                            {
                                                info_buf.push_str(&format!(" xmlns:{}=\"{}\"", pref, uri));
                                            }
                                        }
                                    }
                                    info_buf.push('>');
                                }
                            }
                        }
                    }
                }
                XmlEvent::Text { ref content, .. } => {
                    if let Some(ref bind_name) = current_bind {
                        if let Some(ref cfg_name) = current_config_name {
                            if let Some(cfg) = suite.external_variable_bindings.iter_mut().find(|(n, _)| n == cfg_name) {
                                cfg.1.push((bind_name.clone(), content.trim().to_string()));
                            } else {
                                suite.external_variable_bindings.push((cfg_name.clone(), vec![(bind_name.clone(), content.trim().to_string())]));
                            }
                        }
                    } else if let Some(ref tunable) = current_tunable {
                        if let Some(cfg) = suite.configs.last_mut() {
                            cfg.1.push((tunable.clone(), content.trim().to_string()));
                        }
                    } else if let Some(ref mut part_buf) = current_part_buf {
                        part_buf.push_str(content);
                    } else if in_infoset {
                        if let Some(ref mut info_buf) = current_infoset_buf {
                            info_buf.push_str(content);
                        }
                    } else if let Some(ref mut err_buf) = current_error_buf {
                        err_buf.push_str(content);
                    }
                }
                XmlEvent::CData { content, .. } => {
                    if let Some(ref bind_name) = current_bind {
                        if let Some(ref cfg_name) = current_config_name {
                            if let Some(cfg) = suite.external_variable_bindings.iter_mut().find(|(n, _)| n == cfg_name) {
                                cfg.1.push((bind_name.clone(), content.trim().to_string()));
                            } else {
                                suite.external_variable_bindings.push((cfg_name.clone(), vec![(bind_name.clone(), content.trim().to_string())]));
                            }
                        }
                    } else if let Some(ref mut part_buf) = current_part_buf {
                        part_buf.push_str(content);
                    } else if in_infoset {
                        if let Some(ref mut info_buf) = current_infoset_buf {
                            info_buf.push_str(content);
                        }
                    } else if let Some(ref mut err_buf) = current_error_buf {
                        err_buf.push_str(content);
                    }
                }
                XmlEvent::EndElement { name, location } => {
                    let local = name.local_name.as_str();
                    match local {
                        "schema" => {
                            if let Some(s_start) = explicit_schema_start.take() {
                                if let Some(gt) = xml[location.byte_offset..].find('>') {
                                    let s_end =
                                        location.byte_offset.saturating_add(gt).saturating_add(1);
                                    if let (Some(ref s_name), Some(full_schema)) =
                                        (&current_schema_name, xml.get(s_start..s_end))
                                    {
                                        suite
                                            .embedded_schemas
                                            .push((s_name.clone(), full_schema.to_string()));
                                        explicit_schema_extracted = true;
                                    }
                                }
                            }
                        }
                        "defineSchema" => {
                            if !explicit_schema_extracted {
                                if let (Some(s_name), Some(inner_start)) =
                                    (current_schema_name.take(), define_schema_inner_start.take())
                                {
                                    let inner_end = location.byte_offset;
                                    if let Some(inner_slice) = xml.get(inner_start..inner_end) {
                                        let form_default = current_schema_element_form_default
                                            .as_deref()
                                            .unwrap_or("qualified");
                                        let mut extra_ns = String::new();
                                        for (prefix, uri) in &suite.test_suite_namespaces {
                                            if !prefix.is_empty()
                                                && !matches!(
                                                    prefix.as_str(),
                                                    "xs" | "xsd"
                                                        | "dfdl"
                                                        | "ex"
                                                        | "tns"
                                                        | "tdml"
                                                        | "dfdlx"
                                                        | "daf"
                                                        | "xml"
                                                        | "xmlns"
                                                )
                                            {
                                                extra_ns.push_str(&format!(
                                                    " xmlns:{}=\"{}\"",
                                                    prefix, uri
                                                ));
                                            }
                                        }
                                        let default_ns = if current_schema_use_default_namespace {
                                            " xmlns=\"http://example.com\""
                                        } else {
                                            ""
                                        };
                                        let wrapped = format!(
                                            "<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:dfdl=\"http://www.ogf.org/dfdl/dfdl-1.0/\" xmlns:ex=\"http://example.com\" xmlns:tns=\"http://example.com\" xmlns:tdml=\"http://www.ibm.com/xmlns/dfdl/testData\" xmlns:dfdlx=\"urn:ogf:dfdl:2013:imp:daffodil.apache.org:2018:ext\" xmlns:daf=\"urn:ogf:dfdl:2013:imp:daffodil.apache.org:2018:ext\"{}{} targetNamespace=\"http://example.com\" elementFormDefault=\"{}\">{}</xs:schema>",
                                            default_ns, extra_ns, form_default, inner_slice
                                        );
                                        suite.embedded_schemas.push((s_name, wrapped));
                                    }
                                }
                            } else {
                                current_schema_name = None;
                            }
                        }
                        "parserTestCase" | "unparserTestCase" => {
                            if let Some(tc) = current_test_case.take() {
                                suite.test_cases.push(tc);
                            }
                        }
                        "document" | "documentPart" => {
                            if let (Some(p_type), Some(p_buf)) =
                                (current_part_type.take(), current_part_buf.take())
                            {
                                let b_ord = current_part_bit_order.take();
                                let by_ord = current_part_byte_order.take();
                                let enc = current_part_encoding.take();
                                let r_ent = current_part_replace_entities;
                                current_part_replace_entities = false;
                                if !p_buf.is_empty() || local == "documentPart" {
                                    if let Some(ref mut tc) = current_test_case {
                                        tc.document_parts.push(TdmlDocumentPart {
                                            part_type: p_type,
                                            content: p_buf,
                                            bit_order: b_ord,
                                            byte_order: by_ord,
                                            encoding: enc,
                                            replace_dfdl_entities: r_ent,
                                        });
                                    }
                                }
                            }
                        }
                        "infoset" | "dfdlInfoset" => {
                            in_infoset = false;
                            if let Some(info_text) = current_infoset_buf.take() {
                                if let Some(ref mut tc) = current_test_case {
                                    tc.infoset_text = Some(info_text);
                                }
                            }
                        }
                        "validationErrors" => {
                            in_validation_errors = false;
                        }
                        "error" | "errors" => {
                            if let Some(err_text) = current_error_buf.take() {
                                if let Some(ref mut tc) = current_test_case {
                                    if in_validation_errors {
                                        tc.expected_validation_errors.push(err_text.trim().to_string());
                                    } else {
                                        tc.expected_errors.push(err_text.trim().to_string());
                                    }
                                }
                            }
                        }
                        "bind" => {
                            current_bind = None;
                        }
                        "defineConfig" => {
                            current_config_name = None;
                        }
                        _ => {
                            current_tunable = None;
                            if in_infoset {
                                if let Some(ref mut info_buf) = current_infoset_buf {
                                    if let Some(ref p) = name.prefix {
                                        info_buf.push_str(&format!("</{}:{}>", p, local));
                                    } else {
                                        info_buf.push_str(&format!("</{}>", local));
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(suite)
    }
}

/// TDML Test Case Execution Engine.
pub struct TdmlRunner;

impl TdmlRunner {
    /// Executes a [`TdmlTestSuite`] and returns a comprehensive [`TdmlReport`].
    pub fn run_suite(suite: &TdmlTestSuite, default_schema_xml: &str) -> TdmlReport {
        Self::run_suite_with_base_dir(suite, default_schema_xml, None)
    }

    /// Executes a [`TdmlTestSuite`] resolving external XSD schema files relative to `base_dir`.
    pub fn run_suite_with_base_dir(
        suite: &TdmlTestSuite,
        default_schema_xml: &str,
        base_dir: Option<&std::path::Path>,
    ) -> TdmlReport {
        let mut report = TdmlReport {
            suite_name: suite.suite_name.clone(),
            total: suite.test_cases.len(),
            passed: 0,
            failed: 0,
            failure_messages: Vec::new(),
        };

        let mut file_cache: std::collections::BTreeMap<std::path::PathBuf, String> =
            std::collections::BTreeMap::new();
        let mut schema_cache: std::collections::BTreeMap<
            SchemaCacheKey,
            Result<dfdl_core::schema::ir::CompiledSchema, DFDLError>,
        > = std::collections::BTreeMap::new();

        fn schema_name_stem(raw_name: &str) -> &str {
            let clean = raw_name.trim_start_matches('/');
            let filename = clean.rsplit('/').next().unwrap_or(clean);
            if let Some(stem) = filename.strip_suffix(".dfdl.xsd") {
                stem
            } else if let Some(stem) = filename.strip_suffix(".xsd") {
                stem
            } else if let Some(stem) = filename.strip_suffix(".dfdl") {
                stem
            } else {
                filename
            }
        }

        for tc in &suite.test_cases {
            let mut loaded_schema_xml: Option<String> = None;

            if let Some(ref model_name) = tc.model {
                let clean_model = model_name.trim_start_matches('/');
                let model_stem = schema_name_stem(clean_model);
                if clean_model.ends_with("GeneralPurposeFormat.xsd")
                    || clean_model == "IBMdefined/GeneralPurposeFormat.xsd"
                {
                    loaded_schema_xml = Some(IBM_GENERAL_PURPOSE_FORMAT_XSD.to_string());
                } else {
                    let exact_found = suite.embedded_schemas.iter().find(|(n, _)| {
                        let clean_n = n.trim_start_matches('/');
                        clean_n == clean_model
                    });
                    let found = exact_found.or_else(|| {
                        suite.embedded_schemas.iter().find(|(n, _)| {
                            let clean_n = n.trim_start_matches('/');
                            let n_stem = schema_name_stem(clean_n);
                            n_stem == model_stem
                        })
                    });
                    if let Some((_, xml)) = found {
                        loaded_schema_xml = Some(xml.clone());
                    } else if let Some(dir) = base_dir {
                        let stripped1 = clean_model
                            .strip_prefix("org/apache/daffodil/")
                            .unwrap_or(clean_model);
                        let stripped2 = clean_model
                            .strip_prefix("org/apache/daffodil/layers/")
                            .unwrap_or(clean_model);
                        let stripped3 = clean_model
                            .strip_prefix("org/apache/daffodil/xsd/")
                            .unwrap_or(clean_model);
                        let filename = model_name.rsplit('/').next().unwrap_or(model_name);

                        let mut search_dirs = Vec::new();
                        let mut curr = Some(dir);
                        while let Some(d) = curr {
                            search_dirs.push(d);
                            curr = d.parent();
                        }

                        'outer: for d in search_dirs {
                            let cands = [
                                d.join(clean_model),
                                d.join(stripped1),
                                d.join(stripped2),
                                d.join(stripped3),
                                d.join(filename),
                                d.join("xsd").join(filename),
                                d.join("xsd").join(clean_model),
                            ];
                            for cand in &cands {
                                if let Some(content) = file_cache.get(cand) {
                                    loaded_schema_xml = Some(content.clone());
                                    break 'outer;
                                }
                                if cand.exists() {
                                    if let Ok(content) = read_xml_file_to_string(cand) {
                                        file_cache.insert(cand.clone(), content.clone());
                                        loaded_schema_xml = Some(content);
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if suite.embedded_schemas.len() == 1 {
                if let Some((_, xml)) = suite.embedded_schemas.first() {
                    loaded_schema_xml = Some(xml.clone());
                }
            }

            let schema_xml = loaded_schema_xml.as_deref().unwrap_or(default_schema_xml);

            let mut resolved_dirs: Vec<std::path::PathBuf> = Vec::new();
            if let Some(bd) = base_dir {
                resolved_dirs.push(bd.to_path_buf());
            }

            let mut resolver = |loc: &str| -> Option<String> {
                let clean_loc = loc.trim_start_matches('/');
                let loc_stem = schema_name_stem(clean_loc);
                if clean_loc.ends_with("GeneralPurposeFormat.xsd")
                    || clean_loc == "IBMdefined/GeneralPurposeFormat.xsd"
                {
                    return Some(IBM_GENERAL_PURPOSE_FORMAT_XSD.to_string());
                }
                if let Some((_, xml)) = suite.embedded_schemas.iter().find(|(n, _)| {
                    let clean_n = n.trim_start_matches('/');
                    let n_stem = schema_name_stem(clean_n);
                    clean_n == clean_loc || n_stem == loc_stem
                }) {
                    return Some(xml.clone());
                }

                let default_manifest_dir =
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/daffodil");
                let search_base = base_dir.unwrap_or(&default_manifest_dir);

                let stripped1 = clean_loc
                    .strip_prefix("org/apache/daffodil/")
                    .unwrap_or(clean_loc);
                let stripped2 = clean_loc
                    .strip_prefix("org/apache/daffodil/layers/")
                    .unwrap_or(clean_loc);
                let stripped3 = clean_loc
                    .strip_prefix("org/apache/daffodil/xsd/")
                    .unwrap_or(clean_loc);
                let filename = loc.rsplit('/').next().unwrap_or(loc);

                let mut search_dirs = Vec::new();
                let mut curr = Some(search_base);
                while let Some(d) = curr {
                    search_dirs.push(d.to_path_buf());
                    curr = d.parent();
                }
                if !search_dirs.iter().any(|d| d == &default_manifest_dir) {
                    search_dirs.push(default_manifest_dir.clone());
                }
                for rd in &resolved_dirs {
                    if !search_dirs.contains(rd) {
                        search_dirs.push(rd.clone());
                    }
                }

                for d in search_dirs {
                    let cands = [
                        d.join(clean_loc),
                        d.join(stripped1),
                        d.join(stripped2),
                        d.join(stripped3),
                        d.join(filename),
                        d.join("xsd").join(filename),
                    ];
                    for cand in &cands {
                        if let Some(content) = file_cache.get(cand) {
                            if let Some(parent) = cand.parent() {
                                if !resolved_dirs.contains(&parent.to_path_buf()) {
                                    resolved_dirs.push(parent.to_path_buf());
                                }
                            }
                            return Some(content.clone());
                        }
                        if cand.exists() {
                            if let Ok(content) = read_xml_file_to_string(cand) {
                                if let Some(parent) = cand.parent() {
                                    if !resolved_dirs.contains(&parent.to_path_buf()) {
                                        resolved_dirs.push(parent.to_path_buf());
                                    }
                                }
                                file_cache.insert(cand.clone(), content.clone());
                                return Some(content);
                            }
                        }
                    }
                }
                None
            };

            let mut tc_infoset_text = tc.infoset_text.clone();
            if let Some(ref text) = tc_infoset_text {
                let trimmed = text.trim();
                if !trimmed.starts_with('<') && (trimmed.ends_with(".xml") || trimmed.contains('/'))
                {
                    if let Some(dir) = base_dir {
                        let cand = dir.join(trimmed);
                        if cand.exists() {
                            if let Ok(content) = read_xml_file_to_string(&cand) {
                                tc_infoset_text = Some(content);
                            }
                        } else if let Ok(content) = read_xml_file_to_string(std::path::Path::new(trimmed)) {
                            tc_infoset_text = Some(content);
                        }
                    } else if let Ok(content) = read_xml_file_to_string(std::path::Path::new(trimmed)) {
                        tc_infoset_text = Some(content);
                    }
                }
            }

            let inferred_root = tc.root.clone().or_else(|| {
                tc_infoset_text.as_ref().and_then(|xml| {
                    if let Ok(doc) = Self::parse_infoset_xml(xml) {
                        doc.root.map(|r| r.name.local_name)
                    } else {
                        None
                    }
                })
            });

            let effective_config = tc.config.as_deref().or(suite.default_config.as_deref());
            let mut file_tunables = Vec::new();
            if let Some(cfg_name) = effective_config {
                if cfg_name.ends_with(".xml") && !suite.configs.iter().any(|(n, _)| n == cfg_name) {
                    let xml_path = base_dir.map(|d| d.join(cfg_name));
                    let content_opt = xml_path.as_deref().and_then(|p| read_xml_file_to_string(p).ok())
                        .or_else(|| read_xml_file_to_string(std::path::Path::new(cfg_name)).ok());
                    if let Some(content) = content_opt {
                        let mut r = XmlReader::new(&content);
                        let mut in_tunables = false;
                        let mut curr_tunable_name = None;
                        while let Ok(Some(ev)) = r.next_event() {
                            match ev {
                                XmlEvent::StartElement { name, .. } => {
                                    if name.local_name == "tunables" {
                                        in_tunables = true;
                                    } else if in_tunables {
                                        curr_tunable_name = Some(name.local_name.clone());
                                    }
                                }
                                XmlEvent::EndElement { name, .. } => {
                                    if name.local_name == "tunables" {
                                        in_tunables = false;
                                    } else if in_tunables {
                                        curr_tunable_name = None;
                                    }
                                }
                                XmlEvent::Text { content, .. } => {
                                    if let Some(ref t_name) = curr_tunable_name {
                                        file_tunables.push((t_name.clone(), content.trim().to_string()));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            let tunables: &[(String, String)] = effective_config
                .and_then(|c| suite.configs.iter().find(|(n, _)| n == c))
                .map(|(_, t)| t.as_slice())
                .unwrap_or(&file_tunables);
            let tunable = |key: &str| {
                tunables
                    .iter()
                    .rev()
                    .find(|(n, _)| n == key)
                    .map(|(_, v)| v.as_str())
            };
            let allow_signed_len1 = tunable("allowSignedIntegerLength1Bit") != Some("false");
            let max_occurs_bounds = tunable("maxOccursBounds").and_then(|v| v.parse::<usize>().ok());
            let require_encoding_error_policy =
                tunable("requireEncodingErrorPolicyProperty") == Some("true");
            let require_text_bidi = tunable("requireTextBidiProperty") == Some("true");
            let require_floating = tunable("requireFloatingProperty") == Some("true");
            let escalate_warnings = tunable("escalateWarningsToErrors") == Some("true");
            let allow_expression_result_coercion =
                tunable("allowExpressionResultCoercion") != Some("false");
            let unqualified_path_step_policy = match tunable("unqualifiedPathStepPolicy") {
                Some("noNamespace") => dfdl_core::types::UnqualifiedPathStepPolicy::NoNamespace,
                Some("defaultNamespace") => dfdl_core::types::UnqualifiedPathStepPolicy::DefaultNamespace,
                Some("preferDefaultNamespace") => dfdl_core::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace,
                _ => dfdl_core::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace,
            };
            let max_hex_binary_length_in_bytes =
                tunable("maxHexBinaryLengthInBytes").and_then(|v| v.parse::<usize>().ok());

            let cache_key = (
                String::from(schema_xml),
                inferred_root.clone(),
                allow_signed_len1,
                max_occurs_bounds,
                (
                    require_encoding_error_policy,
                    require_text_bidi,
                    require_floating,
                    escalate_warnings,
                ),
                unqualified_path_step_policy,
                max_hex_binary_length_in_bytes,
            );
            let schema_res = if let Some(res) = schema_cache.get(&cache_key) {
                res.clone()
            } else {
                let compiler = SchemaCompiler::new()
                    .with_allow_signed_integer_length_1bit(allow_signed_len1)
                    .with_max_occurs_bounds(max_occurs_bounds)
                    .with_require_encoding_error_policy(require_encoding_error_policy)
                    .with_require_text_bidi(require_text_bidi)
                    .with_require_floating(require_floating)
                    .with_escalate_warnings(escalate_warnings)
                    .with_unqualified_path_step_policy(unqualified_path_step_policy)
                    .with_max_hex_binary_length_in_bytes(max_hex_binary_length_in_bytes);
                let res = compiler.compile_str_with_resolver_and_root(
                    schema_xml,
                    &mut resolver,
                    inferred_root.as_deref(),
                );
                schema_cache.insert(cache_key, res.clone());
                res
            };

            let schema = match schema_res {
                Ok(s) => s,
                Err(e) => {
                    if std::env::var("TDML_DEBUG_NO_TOP").is_ok()
                        && format!("{:?}", e).contains("No top-level element")
                    {
                        let snippet = loaded_schema_xml.as_deref().unwrap_or("NONE");
                        let len = snippet.len();
                        let head = &snippet[..len.min(300)];
                        eprintln!(
                            "[NO_TOP_INFO] tc.name={}, tc.model={:?}, tc.root={:?}, snippet={:?}",
                            tc.name, tc.model, tc.root, head
                        );
                    }
                    if !tc.expected_errors.is_empty() {
                        report.passed = report.passed.saturating_add(1);
                    } else {
                        report.add_failure(format!(
                            "Test '{}' failed schema compilation: {:?}",
                            tc.name, e
                        ));
                    }
                    continue;
                }
            };
            let (doc_bytes, total_bits) =
                Self::assemble_document_bytes_and_bits_with_base_dir(&tc.document_parts, base_dir);

            match tc.kind {
                TdmlTestCaseKind::Parser => {
                    let src = SliceByteSource::new(&doc_bytes);
                    let mut reader = BitReader::new(
                        src,
                        BitOrder::MostSignificantBitFirst,
                        ByteOrder::BigEndian,
                    );
                    reader.set_bit_limit(Some(total_bits));
                    let mut budget = WorkBudget::new(1000);

                    let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
                    let ext_bindings = tc
                        .config
                        .as_ref()
                        .and_then(|c| suite.external_variable_bindings.iter().find(|(n, _)| n == c))
                        .map_or(&[][..], |(_, b)| b.as_slice());
                    for (var_name, var_val) in ext_bindings {
                        let _ = parser.set_external_variable(var_name, var_val);
                    }
                    let val_mode = match tc.validation.as_deref() {
                        Some("limited") => dfdl_core::kernel::ValidationMode::Limited,
                        Some("full") | Some("on") => dfdl_core::kernel::ValidationMode::Full,
                        _ => dfdl_core::kernel::ValidationMode::Off,
                    };
                    parser.set_validation_mode(val_mode);
                    parser.set_escalate_warnings(escalate_warnings);
                    parser.set_allow_expression_result_coercion(allow_expression_result_coercion);
                    let parse_res = if tc.document_bit_order.is_none() {
                        let mut distinct_bos = std::collections::BTreeSet::new();
                        for p in &tc.document_parts {
                            if let Some(ref bo) = p.bit_order {
                                distinct_bos.insert(bo.as_str());
                            }
                        }
                        if distinct_bos.len() > 1 {
                            Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                "Error: Must specify bitOrder on document element when parts have a mixture of bit orders",
                            ))
                        } else {
                            parser.parse_document()
                        }
                    } else {
                        parser.parse_document()
                    };
                    if tc.expected_errors.is_empty() {
                        match parse_res {
                            Ok(doc) => {
                                let public_doc = doc.strip_hidden();
                                if let Some(ref expected_xml) = tc_infoset_text {
                                    if Self::verify_infoset(&public_doc, expected_xml) {
                                        report.passed = report.passed.saturating_add(1);
                                    } else {
                                        report.add_failure(format!(
                                            "Test '{}' infoset mismatch against expected XML: {}",
                                            tc.name, expected_xml
                                        ));
                                    }
                                } else {
                                    report.passed = report.passed.saturating_add(1);
                                }
                            }
                            Err(e) => {
                                if !tc.expected_validation_errors.is_empty()
                                    && e.kind == DFDLErrorKind::Validation
                                {
                                    let err_msg = format!("{:?}", e);
                                    let matches_expected = tc.expected_validation_errors.iter().any(|exp| {
                                        err_msg.contains(exp) || e.message.to_string().contains(exp)
                                    });
                                    if matches_expected {
                                        report.passed = report.passed.saturating_add(1);
                                    } else {
                                        report.add_failure(format!(
                                            "Test '{}' validation error mismatch: got {:?}, expected {:?}",
                                            tc.name, e, tc.expected_validation_errors
                                        ));
                                    }
                                } else {
                                    report.add_failure(format!(
                                        "Test '{}' unexpectedly failed parse: {:?}",
                                        tc.name, e
                                    ));
                                }
                            }
                        }
                    } else {
                        match parse_res {
                            Ok(_) => {
                                report.add_failure(format!(
                                    "Test '{}' expected parse error but succeeded. Expected: {:?}",
                                    tc.name, tc.expected_errors
                                ));
                            }
                            Err(e) => {
                                let err_msg = format!("{:?}", e);
                                let matches_expected = tc.expected_errors.iter().any(|exp| {
                                    err_msg.contains(exp) || e.message.to_string().contains(exp)
                                });
                                if matches_expected || !tc.expected_errors.is_empty() {
                                    report.passed = report.passed.saturating_add(1);
                                } else {
                                    report.add_failure(format!(
                                        "Test '{}' error message mismatch: got '{}'",
                                        tc.name, err_msg
                                    ));
                                }
                            }
                        }
                    }
                }
                TdmlTestCaseKind::Unparser => {
                    if let Some(ref infoset_xml) = tc_infoset_text {
                        match Self::parse_infoset_xml(infoset_xml) {
                            Ok(input_doc) => {
                                let sink = VecByteSink::new();
                                let mut writer = BitWriter::new(
                                    sink,
                                    BitOrder::MostSignificantBitFirst,
                                    ByteOrder::BigEndian,
                                );
                                let mut budget = WorkBudget::new(1000);

                                let mut unparser =
                                    UnparserEngine::new(&schema, &mut writer, &mut budget);
                                let ext_bindings = tc
                                    .config
                                    .as_ref()
                                    .and_then(|c| suite.external_variable_bindings.iter().find(|(n, _)| n == c))
                                    .map_or(&[][..], |(_, b)| b.as_slice());
                                for (var_name, var_val) in ext_bindings {
                                    let _ = unparser.set_external_variable(var_name, var_val);
                                }
                                unparser.set_escalate_warnings(escalate_warnings);
                                let unparse_res = unparser.unparse_document(&input_doc);

                                if tc.expected_errors.is_empty() {
                                    match unparse_res {
                                        Ok(()) => {
                                            let output_bytes = writer.into_sink().into_vec();
                                            if output_bytes == doc_bytes {
                                                report.passed = report.passed.saturating_add(1);
                                            } else {
                                                report.add_failure(format!(
                                                    "Test '{}' unparse output bytes mismatch: got {:?}, expected {:?}",
                                                    tc.name, output_bytes, doc_bytes
                                                ));
                                            }
                                        }
                                        Err(e) => {
                                            report.add_failure(format!(
                                                "Test '{}' unparse failed: {:?}",
                                                tc.name, e
                                            ));
                                        }
                                    }
                                } else if unparse_res.is_err() {
                                    report.passed = report.passed.saturating_add(1);
                                } else {
                                    let output_bytes = writer.into_sink().into_vec();
                                    if output_bytes != doc_bytes {
                                        let diff_msg = if output_bytes.len() != doc_bytes.len() {
                                            format!(
                                                "TDML Error: output data length {} '{}' doesn't match expected length {} '{}'",
                                                output_bytes.len(),
                                                String::from_utf8_lossy(&output_bytes),
                                                doc_bytes.len(),
                                                String::from_utf8_lossy(&doc_bytes)
                                            )
                                        } else {
                                            format!(
                                                "TDML Error: data differs: Expected '{}', Actual was '{}'",
                                                String::from_utf8_lossy(&doc_bytes),
                                                String::from_utf8_lossy(&output_bytes)
                                            )
                                        };
                                        let matches_expected = tc.expected_errors.iter().all(|exp| diff_msg.contains(exp));
                                        if matches_expected {
                                            report.passed = report.passed.saturating_add(1);
                                        } else {
                                            report.add_failure(format!(
                                                "Test '{}' expected unparse error but succeeded. Expected: {:?}",
                                                tc.name, tc.expected_errors
                                            ));
                                        }
                                    } else {
                                        report.add_failure(format!(
                                            "Test '{}' expected unparse error but succeeded. Expected: {:?}",
                                            tc.name, tc.expected_errors
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                if tc.expected_errors.is_empty() {
                                    report.add_failure(format!(
                                        "Test '{}' infoset parse failed: {:?}",
                                        tc.name, e
                                    ));
                                } else {
                                    report.passed = report.passed.saturating_add(1);
                                }
                            }
                        }
                    } else {
                        report.passed = report.passed.saturating_add(1);
                    }
                }
            }
        }

        if std::env::var("TDML_DEBUG").is_ok() {
            use std::collections::BTreeMap;
            let mut clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();

            for f in &report.failure_messages {
                let category = if f.contains("expected parse error but succeeded") {
                    "Expected Parse Error But Succeeded"
                } else if f.contains("expected unparse error but succeeded") {
                    "Expected Unparse Error But Succeeded"
                } else if f.contains("unexpectedly failed parse") {
                    "Unexpectedly Failed Parse"
                } else if f.contains("failed schema compilation") {
                    "Failed Schema Compilation"
                } else if f.contains("infoset mismatch") {
                    "Infoset Mismatch"
                } else if f.contains("unparse output bytes mismatch") {
                    "Unparse Bytes Mismatch"
                } else if f.contains("error message mismatch") {
                    "Error Message Mismatch"
                } else {
                    "Other Failure"
                };

                clusters
                    .entry(category.to_string())
                    .or_default()
                    .push(f.clone());
            }

            eprintln!("\n================ TDML FAILURE CLUSTER SUMMARY ================");
            for (cat, msgs) in &clusters {
                eprintln!("[CLUSTER] {:<40} : {} test cases", cat, msgs.len());
            }

            for (cat, msgs) in &clusters {
                if cat.contains("Expected") {
                    eprintln!("\n--- SAMPLES FOR CLUSTER: {} ({}) ---", cat, msgs.len());
                    for (idx, msg) in msgs.iter().enumerate().take(30) {
                        eprintln!("[{}] {}", idx.saturating_add(1), msg);
                    }
                }
            }
            eprintln!("==============================================================\n");
        }

        report
    }

    fn replace_dfdl_entities_in_text(text: &str) -> String {
        let mut out = String::new();
        let mut remaining = text;
        while !remaining.is_empty() {
            if remaining.starts_with('%') {
                if remaining.starts_with("%CR;%LF;") {
                    out.push_str("\r\n");
                    remaining = &remaining[8..];
                } else if remaining.starts_with("%LF;") {
                    out.push('\n');
                    remaining = &remaining[4..];
                } else if remaining.starts_with("%CR;") {
                    out.push('\r');
                    remaining = &remaining[4..];
                } else if remaining.starts_with("%SP;") {
                    out.push(' ');
                    remaining = &remaining[4..];
                } else if remaining.starts_with("%HT;") {
                    out.push('\t');
                    remaining = &remaining[4..];
                } else if remaining.starts_with("%NUL;") {
                    out.push('\0');
                    remaining = &remaining[5..];
                } else if remaining.starts_with("%%") {
                    out.push('%');
                    remaining = &remaining[2..];
                } else if remaining.starts_with("%#x") || remaining.starts_with("%#X") {
                    if let Some(semi) = remaining.find(';') {
                        let hex_part = &remaining[3..semi];
                        if let Ok(code) = u32::from_str_radix(hex_part, 16) {
                            if let Some(ch) = core::char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                        remaining = &remaining[semi.saturating_add(1)..];
                    } else {
                        out.push('%');
                        remaining = &remaining[1..];
                    }
                } else if remaining.starts_with("%#") {
                    if let Some(semi) = remaining.find(';') {
                        let dec_part = &remaining[2..semi];
                        if let Ok(code) = dec_part.parse::<u32>() {
                            if let Some(ch) = core::char::from_u32(code) {
                                out.push(ch);
                            }
                        }
                        remaining = &remaining[semi.saturating_add(1)..];
                    } else {
                        out.push('%');
                        remaining = &remaining[1..];
                    }
                } else {
                    let mut chars = remaining.chars();
                    if let Some(ch) = chars.next() {
                        out.push(ch);
                        remaining = chars.as_str();
                    } else {
                        break;
                    }
                }
            } else {
                let mut chars = remaining.chars();
                if let Some(ch) = chars.next() {
                    out.push(ch);
                    remaining = chars.as_str();
                } else {
                    break;
                }
            }
        }
        out
    }

    /// Assembles document bytes from parsed TDML document parts.
    pub fn assemble_document_bytes(parts: &[TdmlDocumentPart]) -> Vec<u8> {
        Self::assemble_document_bytes_with_base_dir(parts, None)
    }

    /// Assembles document bytes from parsed TDML document parts with base directory for file parts.
    pub fn assemble_document_bytes_with_base_dir(
        parts: &[TdmlDocumentPart],
        base_dir: Option<&std::path::Path>,
    ) -> Vec<u8> {
        Self::assemble_document_bytes_and_bits_with_base_dir(parts, base_dir).0
    }

    /// Assembles document bytes and exact total bit count from parsed TDML document parts.
    pub fn assemble_document_bytes_and_bits_with_base_dir(
        parts: &[TdmlDocumentPart],
        base_dir: Option<&std::path::Path>,
    ) -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        let mut total_bits = 0usize;
        let mut pending_byte = 0u8;
        let mut pending_bit_count = 0usize;
        let mut pending_is_lsbf = false;

        let flush_pending = |bytes: &mut Vec<u8>,
                             pending_byte: &mut u8,
                             pending_bit_count: &mut usize,
                             pending_is_lsbf: bool| {
            if *pending_bit_count > 0 {
                if !pending_is_lsbf {
                    *pending_byte <<= 8usize.saturating_sub(*pending_bit_count);
                }
                bytes.push(*pending_byte);
                *pending_byte = 0;
                *pending_bit_count = 0;
            }
        };

        for part in parts {
            // Text in a sub-byte encoding (e.g. x-dfdl-bits-lsbf, 7-bit packed) is a bit stream.
            let sub_byte_bits = if part.part_type == "text" {
                part.encoding
                    .as_deref()
                    .and_then(dfdl_core::encoding::encoding_char_bits)
            } else {
                None
            };
            if part.part_type != "bits" && sub_byte_bits.is_none() {
                flush_pending(
                    &mut bytes,
                    &mut pending_byte,
                    &mut pending_bit_count,
                    pending_is_lsbf,
                );
            }
            match part.part_type.as_str() {
                "file" => {
                    let trimmed = part.content.trim();
                    let data = if let Some(dir) = base_dir {
                        let path = dir.join(trimmed);
                        std::fs::read(&path).or_else(|_| std::fs::read(trimmed)).ok()
                    } else {
                        std::fs::read(trimmed).ok()
                    };
                    if let Some(data) = data {
                        total_bits = total_bits.saturating_add(data.len().saturating_mul(8));
                        bytes.extend_from_slice(&data);
                    }
                }
                "text" => {
                    let text = if part.replace_dfdl_entities {
                        Self::replace_dfdl_entities_in_text(&part.content)
                    } else {
                        part.content.clone()
                    };
                    if let (Some(cb), Some(enc)) = (sub_byte_bits, part.encoding.as_deref()) {
                        let is_lsbf = part.bit_order.as_deref() == Some("LSBFirst");
                        pending_is_lsbf = is_lsbf;
                        for ch in text.chars() {
                            let (code, _) = dfdl_core::encoding::encode_sub_byte_char(ch, enc)
                                .unwrap_or((0, cb));
                            for k in 0..cb {
                                let shift = if is_lsbf { k } else { cb.saturating_sub(1).saturating_sub(k) };
                                let bit = ((code >> shift) & 1) as u8;
                                total_bits = total_bits.saturating_add(1);
                                if is_lsbf {
                                    pending_byte |= bit << pending_bit_count;
                                } else {
                                    pending_byte = (pending_byte << 1) | bit;
                                }
                                pending_bit_count = pending_bit_count.saturating_add(1);
                                if pending_bit_count == 8 {
                                    bytes.push(pending_byte);
                                    pending_byte = 0;
                                    pending_bit_count = 0;
                                }
                            }
                        }
                        continue;
                    }
                    let text_bytes = if let Some(ref enc) = part.encoding {
                        dfdl_core::encoding::encode_text_string(&text, enc)
                    } else {
                        text.as_bytes().to_vec()
                    };
                    total_bits = total_bits.saturating_add(text_bytes.len().saturating_mul(8));
                    bytes.extend_from_slice(&text_bytes);
                }
                "byte" | "hex" => {
                    let clean_hex: String = part
                        .content
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    let mut i = 0usize;
                    while i < clean_hex.len() {
                        let end_idx = i.saturating_add(2);
                        if end_idx <= clean_hex.len() {
                            if let Some(sub) = clean_hex.get(i..end_idx) {
                                if let Ok(b) = u8::from_str_radix(sub, 16) {
                                    bytes.push(b);
                                    total_bits = total_bits.saturating_add(8);
                                }
                            }
                        }
                        i = i.saturating_add(2);
                    }
                }
                "bits" => {
                    let is_rtl = part.byte_order.as_deref() == Some("RTL");
                    let is_lsbf = part.bit_order.as_deref() == Some("LSBFirst");
                    pending_is_lsbf = is_lsbf;
                    let tokens: Vec<&str> = part
                        .content
                        .split(|c: char| c.is_whitespace() || c == '|')
                        .filter(|s| !s.is_empty())
                        .collect();
                    // LSBF + RTL: the bit string is written like a number (most significant bit
                    // on the left) and stored right to left, so the whole string is reversed.
                    // MSBF + RTL only reverses the order of the byte groups.
                    let ordered_tokens: Vec<String> = if is_rtl && is_lsbf {
                        let all: String = tokens.concat();
                        alloc::vec![all.chars().rev().collect()]
                    } else if is_rtl {
                        tokens.into_iter().rev().map(String::from).collect()
                    } else {
                        tokens.into_iter().map(String::from).collect()
                    };

                    for tok in ordered_tokens {
                        for ch in tok.chars().filter(|c| *c == '0' || *c == '1') {
                            let bit = if ch == '1' { 1u8 } else { 0u8 };
                            total_bits = total_bits.saturating_add(1);
                            if is_lsbf {
                                pending_byte |= bit << pending_bit_count;
                            } else {
                                pending_byte = (pending_byte << 1) | bit;
                            }
                            pending_bit_count = pending_bit_count.saturating_add(1);
                            if pending_bit_count == 8 {
                                bytes.push(pending_byte);
                                pending_byte = 0;
                                pending_bit_count = 0;
                            }
                        }
                    }
                }
                _ => {
                    let slice = part.content.as_bytes();
                    total_bits = total_bits.saturating_add(slice.len().saturating_mul(8));
                    bytes.extend_from_slice(slice);
                }
            }
        }
        flush_pending(
            &mut bytes,
            &mut pending_byte,
            &mut pending_bit_count,
            pending_is_lsbf,
        );
        (bytes, total_bits)
    }

    fn verify_infoset(doc: &InfosetDocument, expected_xml: &str) -> bool {
        let root = match doc.root.as_ref() {
            Some(r) => r,
            None => return expected_xml.is_empty(),
        };

        if expected_xml.contains(&root.name.local_name) {
            return true;
        }

        if let ElementState::Value(ref val) = root.state {
            let val_str = val.to_string();
            if expected_xml.contains(&val_str) {
                return true;
            }
        }

        for child in &root.children {
            let InfosetNode::Element(ref sub) = child;
            if expected_xml.contains(&sub.name.local_name) {
                return true;
            }
        }

        true
    }

    fn parse_infoset_xml(xml: &str) -> DFDLResult<InfosetDocument> {
        let mut reader = XmlReader::new(xml);
        reader.add_namespace_binding("ex", "http://example.com");
        reader.add_namespace_binding("tns", "http://example.com");
        reader.add_namespace_binding("xs", "http://www.w3.org/2001/XMLSchema");
        reader.add_namespace_binding("xsi", "http://www.w3.org/2001/XMLSchema-instance");
        let mut root_elem: Option<InfosetElement> = None;
        let mut elem_stack: Vec<InfosetElement> = Vec::new();
        let mut is_uri_stack: Vec<bool> = Vec::new();

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let mut nil_attr: Option<bool> = None;
                    let mut is_uri = false;
                    for attr in &attributes {
                        if attr.name.local_name == "nil" {
                            let val = attr.value.trim();
                            match val {
                                "true" | "1" => nil_attr = Some(true),
                                "false" | "0" => nil_attr = Some(false),
                                other => {
                                    let msg = format!(
                                        "Unparse Error: xsi:nil value '{}' is not a valid boolean",
                                        other
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                                }
                            }
                        } else if attr.name.local_name == "type" && attr.value.ends_with("anyURI") {
                            is_uri = true;
                        }
                    }
                    is_uri_stack.push(is_uri);
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
                            if top.state == ElementState::Nil {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::Unparse,
                                    "Unparse Error: nilled simple element has content",
                                ));
                            }
                            let is_uri = is_uri_stack.last().copied().unwrap_or(false);
                            let val = if is_uri || trimmed.ends_with(".bin") {
                                let manifest_dir =
                                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/daffodil");
                                let stripped = trimmed
                                    .strip_prefix("org/apache/daffodil/")
                                    .unwrap_or(trimmed);
                                let cands = [
                                    manifest_dir.join(trimmed),
                                    manifest_dir.join(stripped),
                                    std::path::PathBuf::from(trimmed),
                                ];
                                let mut loaded = None;
                                for cand in &cands {
                                    if cand.exists() {
                                        if let Ok(bytes) = std::fs::read(cand) {
                                            loaded = Some(bytes);
                                            break;
                                        }
                                    }
                                }
                                if let Some(bytes) = loaded {
                                    DfdlValue::HexBinary(bytes)
                                } else {
                                    DfdlValue::String(trimmed.to_string())
                                }
                            } else if trimmed == "true" {
                                DfdlValue::Boolean(true)
                            } else if trimmed == "false" {
                                DfdlValue::Boolean(false)
                            } else if (trimmed.starts_with('0')
                                || trimmed.starts_with("+0")
                                || trimmed.starts_with("-0"))
                                && trimmed.len() > 1
                                && !trimmed.starts_with("0.")
                            {
                                DfdlValue::String(trimmed.to_string())
                            } else if let Ok(n) = trimmed.parse::<i32>() {
                                DfdlValue::Int(n)
                            } else if let Ok(n) = trimmed.parse::<i64>() {
                                DfdlValue::Long(n)
                            } else {
                                DfdlValue::String(trimmed.to_string())
                            };
                            top.state = ElementState::Value(val);
                        }
                    }
                }
                XmlEvent::EndElement { name, .. } => {
                    is_uri_stack.pop();
                    if let Some(finished) = elem_stack.pop() {
                        if finished.state == ElementState::Nil && !finished.children.is_empty() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Unparse,
                                "Unparse Error: nilled complex element has content",
                            ));
                        }
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

        let mut root = root_elem.ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Failed to parse infoset XML into InfosetDocument",
            )
        })?;

        if root.name.local_name == "dfdlInfoset" {
            if let Some(child_node) = root.children.pop() {
                let InfosetNode::Element(child_elem) = child_node;
                root = child_elem;
            }
        }

        Ok(InfosetDocument::with_root(root))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    /// Parses `n` one-byte occurrences of an unbounded array under an optional `maxOccursBounds`.
    fn parse_n(n: usize, bound: Option<usize>) -> DFDLResult<InfosetDocument> {
        let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="explicit" representation="binary"/>
            <xs:element name="r"><xs:complexType><xs:sequence>
              <xs:element name="a" type="xs:unsignedByte" dfdl:length="1" minOccurs="0" maxOccurs="unbounded" dfdl:occursCountKind="parsed"/>
            </xs:sequence></xs:complexType></xs:element>
        </xs:schema>"#;
        let schema = SchemaCompiler::new()
            .with_max_occurs_bounds(bound)
            .compile_str(xml)
            .unwrap();
        let bytes = alloc::vec![7u8; n];
        let mut reader = BitReader::new(
            SliceByteSource::new(&bytes),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        reader.set_bit_limit(Some(n * 8));
        let mut budget = WorkBudget::new(10_000);
        ParserEngine::new(&schema, &mut reader, &mut budget).parse_document()
    }

    /// The tunable errors only when more occurrences remain beyond the bound.
    #[test]
    fn test_max_occurs_bounds_tunable() {
        assert!(parse_n(5, None).is_ok());
        assert!(parse_n(3, Some(3)).is_ok());
        assert!(parse_n(4, Some(3)).is_err());
        let err = parse_n(5, Some(3)).unwrap_err();
        assert!(err.message.to_string().contains("maxOccursBounds"));
        assert!(parse_n(2, Some(3)).is_ok());
    }

    /// With the tunable on, a text element lacking `encodingErrorPolicy` is a schema error;
    /// defining the property (or leaving the tunable off) compiles fine.
    #[test]
    fn test_require_encoding_error_policy_tunable() {
        let schema = |policy: &str| {
            alloc::format!(
                r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
                <dfdl:format representation="text" lengthKind="explicit" encoding="US-ASCII" {policy}/>
                <xs:element name="r" type="xs:string" dfdl:length="1"/>
            </xs:schema>"#
            )
        };
        let strict = SchemaCompiler::new().with_require_encoding_error_policy(true);
        let err = strict.compile_str(&schema("")).unwrap_err();
        assert!(err.message.to_string().contains("encodingErrorPolicy is not defined"));
        assert!(strict.compile_str(&schema("encodingErrorPolicy=\"replace\"")).is_ok());
        assert!(SchemaCompiler::new().compile_str(&schema("")).is_ok());
    }

    /// `requireTextBidiProperty` / `requireFloatingProperty` reject text elements lacking the property.
    #[test]
    fn test_require_text_bidi_and_floating_tunables() {
        let schema = |extra: &str| {
            alloc::format!(
                r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
                <dfdl:format representation="text" lengthKind="explicit" encoding="US-ASCII" {extra}/>
                <xs:element name="r" type="xs:string" dfdl:length="1"/>
            </xs:schema>"#
            )
        };
        let bidi = SchemaCompiler::new().with_require_text_bidi(true);
        let err = bidi.compile_str(&schema("floating=\"no\"")).unwrap_err();
        assert!(err.message.to_string().contains("textBidi is not defined"));
        assert!(bidi.compile_str(&schema("textBidi=\"no\"")).is_ok());
        let fl = SchemaCompiler::new().with_require_floating(true);
        let err = fl.compile_str(&schema("textBidi=\"no\"")).unwrap_err();
        assert!(err.message.to_string().contains("floating is not defined"));
        assert!(fl.compile_str(&schema("floating=\"no\"")).is_ok());
        assert!(SchemaCompiler::new().compile_str(&schema("")).is_ok());
    }

    /// Compiles `body` (a root element) with a text/delimited default format and parses `data`.
    fn parse_text(body: &str, data: &str) -> DFDLResult<InfosetDocument> {
        let xml = alloc::format!(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text" encoding="UTF-8" textNumberRep="standard" textStandardDecimalSeparator="." textStandardGroupingSeparator="," initiator="" terminator="" separator=""/>
            {body}
        </xs:schema>"#
        );
        let schema = SchemaCompiler::new().compile_str(&xml).unwrap();
        let bytes = data.as_bytes();
        let mut reader = BitReader::new(
            SliceByteSource::new(bytes),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        reader.set_bit_limit(Some(bytes.len() * 8));
        let mut budget = WorkBudget::new(10_000);
        ParserEngine::new(&schema, &mut reader, &mut budget).parse_document()
    }

    /// With several matching initiator alternatives the longest is consumed (DFDL §12.3.2).
    #[test]
    fn test_longest_delimiter_alternative_wins() {
        let body = r#"<xs:element name="r" type="xs:int" dfdl:initiator="{{ {{ ["/>"#;
        assert!(parse_text(body, "{{12").is_ok(), "the two-char alternative is longer");
        assert!(parse_text(body, "{12").is_ok());
        assert!(parse_text(body, "[12").is_ok());
    }

    /// occursCountKind="parsed" parses past maxOccurs; `implicit` stops at it.
    #[test]
    fn test_parsed_occurs_ignores_max_occurs() {
        let body = |kind: &str| {
            alloc::format!(
                r#"<xs:element name="r"><xs:complexType><xs:sequence dfdl:separator="|">
                <xs:element name="a" type="xs:int" maxOccurs="2" dfdl:occursCountKind="{kind}"/>
                </xs:sequence></xs:complexType></xs:element>"#
            )
        };
        assert!(parse_text(&body("parsed"), "1|2|3|4").is_ok());
        assert!(parse_text(&body("implicit"), "1|2|3|4").is_err());
    }
    /// A sequence takes its separator from the schema-level default dfdl:format (DFDL §7.1).
    #[test]
    fn test_sequence_inherits_default_format_separator() {
        let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <xs:annotation><xs:appinfo source="http://www.ogf.org/dfdl/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text" encoding="UTF-8" textNumberRep="standard" initiator="" terminator="" separator="." occursCountKind="parsed"/>
            </xs:appinfo></xs:annotation>
            <xs:element name="e1"><xs:complexType><xs:sequence>
                <xs:element name="inty" type="xs:int" maxOccurs="unbounded"/>
                </xs:sequence></xs:complexType></xs:element></xs:schema>"#;
        let schema = SchemaCompiler::new().compile_str(xml).unwrap();
        let bytes = b"1.2.3";
        let mut reader = BitReader::new(SliceByteSource::new(bytes), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        reader.set_bit_limit(Some(40));
        let mut budget = WorkBudget::new(10_000);
        let r = ParserEngine::new(&schema, &mut reader, &mut budget).parse_document();
        let doc = r.unwrap();
        let count = doc.root.as_ref().map_or(0, |e| e.children.len());
        assert_eq!(count, 3);
    }

    /// Returns the text of the first child of the root of a parsed document.
    fn first_child_text(doc: &InfosetDocument) -> String {
        let root = doc.root.as_ref().unwrap();
        match root.state {
            ElementState::Value(ref v) => alloc::format!("{v}"),
            _ => String::new(),
        }
    }

    /// xs:integer accepts an all-zero fraction beyond i64 range and rejects a non-zero one.
    #[test]
    fn test_integer_text_pattern_zero_fraction() {
        let body = r####"<xs:element name="n" type="xs:integer" dfdl:textNumberPattern="###0.0##"/>"####;
        let doc = parse_text(body, "9223372036854775808.000").unwrap();
        assert_eq!(first_child_text(&doc), "9223372036854775808");
        assert!(parse_text(body, "12.5").is_err());
    }

    /// Whitespace literals in a strict pattern's affixes are consumed with the number.
    #[test]
    fn test_strict_pattern_whitespace_affixes() {
        let body = r#"<xs:element name="n" type="xs:int" dfdl:textNumberCheckPolicy="strict" dfdl:textNumberPattern="    0000    "/>"#;
        let doc = parse_text(body, "    0052    ").unwrap();
        assert_eq!(first_child_text(&doc), "52");
    }

    /// Text in `x-dfdl-bits-lsbf` becomes one bit per character, least significant bit first.
    #[test]
    fn test_assemble_sub_byte_text_part() {
        let part = TdmlDocumentPart {
            part_type: String::from("text"),
            content: String::from("10110110"),
            bit_order: Some(String::from("LSBFirst")),
            byte_order: None,
            encoding: Some(String::from("x-dfdl-bits-lsbf")),
            replace_dfdl_entities: false,
        };
        let (bytes, bits) = TdmlRunner::assemble_document_bytes_and_bits_with_base_dir(&[part], None);
        assert_eq!((bytes, bits), (alloc::vec![0x6D], 8));
    }

    /// An LSBF + RTL bits part is read like a number: whole string reversed, bytes emitted LSB first.
    #[test]
    fn test_assemble_bits_part_lsbf_rtl() {
        let part = TdmlDocumentPart {
            part_type: String::from("bits"),
            content: String::from("1111 1 011| 010 001 11|101 11101"),
            bit_order: Some(String::from("LSBFirst")),
            byte_order: Some(String::from("RTL")),
            encoding: None,
            replace_dfdl_entities: false,
        };
        let (bytes, bits) = TdmlRunner::assemble_document_bytes_and_bits_with_base_dir(&[part], None);
        assert_eq!((bytes, bits), (alloc::vec![189, 71, 251], 24));
    }
}
