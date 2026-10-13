//! `dfdl-example`: Comprehensive examples and validation harness for Eurocontrol ASTERIX protocols.
//!
//! Demonstrates parsing, validation, XML infoset dumping, and zero-allocation VM execution
//! across Eurocontrol ASTERIX Categories:
//! - Category 034: Transmission of Monoradar Service Messages
//! - Category 048: Transmission of Monoradar Target Reports
//! - Category 001: Transmission of Monoradar Data (Plots and Tracks)
//! - Category 002: Transmission of Monoradar Service Messages
//!
//! # Architecture
//! Provides integration with:
//! - [`dfdl_schema::SchemaCompiler`]: Compiles DFDL XML Schemas into in-memory schema graphs.
//! - [`dfdl_core::ParserEngine`]: High-assurance DFDL parser with full validation diagnostics.
//! - [`dfdl_vm::BytecodeCompiler`]: Lowers DFDL schema graphs into optimized linear bytecode.
//! - [`dfdl_vm::VmInterpreter`]: High-throughput wire-speed virtual machine interpreter.

pub mod pcap;

use dfdl_core::error::DFDLResult;
use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use dfdl_core::io::bitstream::BitReader;
use dfdl_core::io::source::SliceByteSource;
use dfdl_core::io::traits::{BitOrder, ByteOrder};
use dfdl_core::kernel::ParserEngine;
use dfdl_core::limits::WorkBudget;
use dfdl_core::schema::ir::CompiledSchema;
use dfdl_schema::SchemaCompiler;
use dfdl_vm::isa::BytecodeProgram;
use dfdl_vm::{BytecodeCompiler, DomBuilderSink, VmInterpreter};

/// Standard error type for dfdl-example operations and CLI examples.
#[derive(Debug)]
pub enum ExampleError {
    /// DFDL Engine processing or schema compilation error.
    Dfdl(dfdl_core::error::DFDLError),
    /// PCAP extraction or framing error.
    Pcap(pcap::PcapError),
    /// Standard I/O error.
    Io(std::io::Error),
    /// High-level formatted message error.
    Message(String),
}

impl core::fmt::Display for ExampleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Dfdl(e) => write!(f, "DFDL Error: {e}"),
            Self::Pcap(e) => write!(f, "PCAP Error: {e}"),
            Self::Io(e) => write!(f, "I/O Error: {e}"),
            Self::Message(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ExampleError {}

impl From<dfdl_core::error::DFDLError> for ExampleError {
    fn from(err: dfdl_core::error::DFDLError) -> Self {
        Self::Dfdl(err)
    }
}

impl From<pcap::PcapError> for ExampleError {
    fn from(err: pcap::PcapError) -> Self {
        Self::Pcap(err)
    }
}

impl From<std::io::Error> for ExampleError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<String> for ExampleError {
    fn from(msg: String) -> Self {
        Self::Message(msg)
    }
}

impl From<&str> for ExampleError {
    fn from(msg: &str) -> Self {
        Self::Message(msg.to_string())
    }
}

/// Embedded ASTERIX DFDL schema XML source string.
pub const ASTERIX_DFDL_XSD: &str = include_str!("../schemas/asterix.dfdl.xsd");

/// Authentic sample PCAP capture bytes containing Category 034 and Category 048 data blocks.
pub const SAMPLE_CAT_034_048_PCAP: &[u8] = include_bytes!("../data/cat_034_048.pcap");

/// Compiles the embedded ASTERIX DFDL schema with an optional distinguished root element.
///
/// If `root_element` is `None`, defaults to `AsterixRecord` (dynamic category dispatch).
/// Other available roots include `AsterixPacket`, `AsterixDatagram`, `Cat034Record`, `Cat048Record`, `Cat001Record`, `Cat002Record`.
///
/// # Errors
/// Returns [`DFDLError`] if the XML schema is malformed or violates DFDL rules.
pub fn compile_asterix_schema(root_element: Option<&str>) -> DFDLResult<CompiledSchema> {
    let compiler = SchemaCompiler::new();
    let root = root_element.unwrap_or("AsterixRecord");
    compiler.compile_str_with_resolver_and_root(ASTERIX_DFDL_XSD, |_| None, Some(root))
}

/// Compiles a [`CompiledSchema`] graph into an optimized linear [`BytecodeProgram`] for `dfdl-vm`.
///
/// # Errors
/// Returns [`DFDLError`] if the schema contains constructs unsupported by the VM compiler.
pub fn compile_asterix_vm(schema: &CompiledSchema) -> DFDLResult<BytecodeProgram> {
    BytecodeCompiler::compile(schema)
}

/// Parses an ASTERIX raw byte slice using [`ParserEngine`] and returns an [`InfosetDocument`].
///
/// # Errors
/// Returns [`DFDLError`] if the binary input violates schema constraints or unexpected EOF occurs.
pub fn parse_asterix_record(schema: &CompiledSchema, data: &[u8]) -> DFDLResult<InfosetDocument> {
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

/// Decodes an ASTERIX raw byte slice using [`VmInterpreter`] and returns an [`InfosetDocument`].
///
/// # Errors
/// Returns [`DFDLError`] if bytecode execution encounters an assertion or data mismatch.
pub fn decode_with_vm(program: &BytecodeProgram, data: &[u8]) -> DFDLResult<InfosetDocument> {
    let source = SliceByteSource::new(data);
    let mut reader = BitReader::new(
        source,
        BitOrder::MostSignificantBitFirst,
        ByteOrder::BigEndian,
    );
    let mut vm = VmInterpreter::new();
    let mut sink = DomBuilderSink::new(&program.symbols);
    vm.execute(program, &mut reader, &mut sink)?;
    sink.into_document()
}

/// Serializes an [`InfosetDocument`] into an indented XML string.
///
/// Emits standard XML with clean hierarchy, suitable for logging, inspection, and verification.
#[must_use]
pub fn infoset_to_xml(doc: &InfosetDocument) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if let Some(ref root) = doc.root {
        format_element_xml(root, 0, &mut xml);
    }
    xml
}

/// Formats a single [`InfosetElement`] recursively with standard 2-space indentation.
fn format_element_xml(elem: &InfosetElement, indent: usize, out: &mut String) {
    let tag = &elem.name.local_name;
    for _ in 0..indent {
        out.push_str("  ");
    }

    if elem.children.is_empty() {
        match &elem.state {
            ElementState::Value(v) => {
                out.push('<');
                out.push_str(tag);
                out.push('>');
                let val_str = format!("{}", v);
                escape_xml_content(&val_str, out);
                out.push_str("</");
                out.push_str(tag);
                out.push_str(">\n");
            }
            ElementState::Empty => {
                out.push('<');
                out.push_str(tag);
                out.push_str("/>\n");
            }
            ElementState::Nil => {
                out.push('<');
                out.push_str(tag);
                out.push_str(" xsi:nil=\"true\"/>\n");
            }
            _ => {
                out.push('<');
                out.push_str(tag);
                out.push_str("/>\n");
            }
        }
    } else {
        out.push('<');
        out.push_str(tag);
        out.push_str(">\n");

        for child in &elem.children {
            match child {
                InfosetNode::Element(ref child_elem) => {
                    format_element_xml(child_elem, indent.saturating_add(1), out);
                }
            }
        }

        for _ in 0..indent {
            out.push_str("  ");
        }
        out.push_str("</");
        out.push_str(tag);
        out.push_str(">\n");
    }
}

/// Escapes XML special characters (`<`, `>`, `&`, `"`, `'`) for safe infoset dumping.
fn escape_xml_content(input: &str, out: &mut String) {
    for c in input.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_asterix_schema_all_roots() {
        let root_names = [
            "AsterixPacket",
            "AsterixDatagram",
            "AsterixRecord",
            "Cat034Record",
            "Cat048Record",
            "Cat001Record",
            "Cat002Record",
        ];
        for name in root_names {
            let schema = compile_asterix_schema(Some(name))
                .unwrap_or_else(|e| panic!("Failed to compile root '{}': {:?}", name, e));
            assert!(schema.root_element_id.0 > 0);
        }
    }

    #[test]
    fn test_compile_asterix_vm_roots() {
        let roots = ["Cat034Record", "Cat001Record", "Cat002Record"];
        for name in roots {
            let schema = compile_asterix_schema(Some(name)).unwrap();
            let vm_prog = compile_asterix_vm(&schema);
            assert!(vm_prog.is_ok(), "Failed to compile VM for root {}", name);
        }
    }

    #[test]
    fn test_asterix_numerical_facets() {
        let schema = compile_asterix_schema(Some("Cat034Record")).unwrap();
        let mut checked_fields = 0;
        for term in &schema.terms {
            match term.name.local_name.as_str() {
                "category" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("255"));
                    checked_fields += 1;
                }
                "length" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("3"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_fields += 1;
                }
                "sac" | "sic" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("255"));
                    checked_fields += 1;
                }
                "timeOfDay" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("16777215"));
                    checked_fields += 1;
                }
                "antennaRotationPeriod" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_fields += 1;
                }
                "height" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("-32768"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("32767"));
                    checked_fields += 1;
                }
                "latitude" | "longitude" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("-8388608"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("8388607"));
                    checked_fields += 1;
                }
                _ => {}
            }
        }
        assert!(checked_fields >= 7, "Expected at least 7 checked numerical facet fields, got {}", checked_fields);
    }

    #[test]
    fn test_asterix_quantum_and_documentation_metadata() {
        // 1. Verify that custom ASTERIX annotations and quanta are declared in the schema
        assert!(ASTERIX_DFDL_XSD.contains("xmlns:ast=\"urn:eurocontrol:asterix\""));
        assert!(ASTERIX_DFDL_XSD.contains("<xs:appinfo source=\"urn:eurocontrol:asterix\">"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>1/128</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>1/256</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>360/65536</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>1/4</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>1/64</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>1/16384</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:resolution>360/8192</ast:resolution>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>seconds</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>NM</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>degrees</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>FL</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>feet</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>dBm</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:unit>replies</ast:unit>"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:item>I048/040-RHO"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:item>I048/040-THETA"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:item>I048/090"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:item>I048/130-SRL"));
        assert!(ASTERIX_DFDL_XSD.contains("<ast:item>I048/110"));

        // 2. Compile Cat048Record schema and verify new simpleTypes and facets
        let schema48 = compile_asterix_schema(Some("Cat048Record")).unwrap();
        let mut checked_cat48_facets = 0;
        for term in &schema48.terms {
            match term.name.local_name.as_str() {
                "rho" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_cat48_facets += 1;
                }
                "theta" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_cat48_facets += 1;
                }
                "flightLevel" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_cat48_facets += 1;
                }
                "srl" | "srr" | "sam" | "prl" | "pam" | "rpd" | "apd" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("255"));
                    checked_cat48_facets += 1;
                }
                "height3d" => {
                    assert_eq!(term.properties.facets.min_inclusive.as_deref(), Some("0"));
                    assert_eq!(term.properties.facets.max_inclusive.as_deref(), Some("65535"));
                    checked_cat48_facets += 1;
                }
                _ => {}
            }
        }
        assert!(checked_cat48_facets >= 10, "Expected at least 10 checked Cat048 facets, got {}", checked_cat48_facets);
    }

    #[test]
    fn test_pcap_extraction() {
        let (header, _) = pcap::parse_pcap_header(SAMPLE_CAT_034_048_PCAP).unwrap();
        assert_eq!(header.magic, 0xa1b2c3d4);
        assert!(header.is_little_endian);

        let packets = pcap::parse_pcap_packets(SAMPLE_CAT_034_048_PCAP, &header).unwrap();
        assert_eq!(packets.len(), 100);

        let mut cat34_count = 0;
        let mut cat48_count = 0;
        for pkt in &packets {
            let udp_payload = pcap::extract_udp_payload(pkt.data).unwrap();
            let blocks = pcap::extract_asterix_blocks(udp_payload).unwrap();
            for block in blocks {
                if block.category == 34 {
                    cat34_count += 1;
                } else if block.category == 48 {
                    cat48_count += 1;
                }
            }
        }
        assert_eq!(cat34_count, 34);
        assert_eq!(cat48_count, 86);
    }

    #[test]
    fn test_occurs_count_fspec() {
        let xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format representation="binary" byteOrder="bigEndian" binaryNumberRep="binary"
                   lengthUnits="bytes" alignment="1" alignmentUnits="bytes" lengthKind="implicit"
                   leadingSkip="0" trailingSkip="0" initiator="" terminator="" separator=""
                   encoding="ISO-8859-1" textBidi="no" floating="no" fillByte="%#r00;" occursCountKind="implicit"/>
    </xs:appinfo>
  </xs:annotation>
  <xs:element name="Record">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="fspec" type="xs:unsignedByte"/>
        <xs:element name="fieldA" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
                    dfdl:occursCountKind="expression"
                    dfdl:occursCount="{ (../fspec idiv 128) mod 2 }"/>
        <xs:element name="fieldB" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
                    dfdl:occursCountKind="expression"
                    dfdl:occursCount="{ (../fspec idiv 64) mod 2 }"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

        let compiler = dfdl_schema::SchemaCompiler::new();
        let schema = compiler.compile_str(xsd).unwrap();

        // 1. Both present: fspec = 0xc0 (192 = 128 + 64), fieldA = 10, fieldB = 20
        let data1 = [0xc0, 10, 20];
        let doc1 = parse_asterix_record(&schema, &data1).unwrap();
        let xml1 = infoset_to_xml(&doc1);
        assert!(xml1.contains("<fieldA>10</fieldA>"));
        assert!(xml1.contains("<fieldB>20</fieldB>"));

        // 2. Only fieldA: fspec = 0x80 (128), fieldA = 10
        let data2 = [0x80, 10];
        let doc2 = parse_asterix_record(&schema, &data2).unwrap();
        let xml2 = infoset_to_xml(&doc2);
        assert!(xml2.contains("<fieldA>10</fieldA>"));
        assert!(!xml2.contains("<fieldB>"));

        // 3. Only fieldB: fspec = 0x40 (64), fieldB = 20
        let data3 = [0x40, 20];
        let doc3 = parse_asterix_record(&schema, &data3).unwrap();
        let xml3 = infoset_to_xml(&doc3);
        assert!(!xml3.contains("<fieldA>"));
        assert!(xml3.contains("<fieldB>20</fieldB>"));
    }

    #[test]
    fn test_cascading_fspec_extension() {
        let xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format representation="binary" byteOrder="bigEndian" binaryNumberRep="binary"
                   lengthUnits="bytes" alignment="1" alignmentUnits="bytes" lengthKind="implicit"
                   leadingSkip="0" trailingSkip="0" initiator="" terminator="" separator=""
                   encoding="ISO-8859-1" textBidi="no" floating="no" fillByte="%#r00;" occursCountKind="implicit"/>
    </xs:appinfo>
  </xs:annotation>
  <xs:element name="Record">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="fspec1" type="xs:unsignedByte"/>
        <xs:element name="fspec2" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
                    dfdl:occursCountKind="expression"
                    dfdl:occursCount="{ ../fspec1 mod 2 }"/>
        <!-- Field 1 in fspec1 (bit 7 = 128) -->
        <xs:element name="item010" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
                    dfdl:occursCountKind="expression"
                    dfdl:occursCount="{ (../fspec1 idiv 128) mod 2 }"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

        let compiler = dfdl_schema::SchemaCompiler::new();
        let schema = compiler.compile_str(xsd).unwrap();

        // Case A: fspec1 = 0x81 (bit 7=1 -> item010 present; bit 0=1 -> fspec2 present)
        let data_a = [0x81, 0x55, 42];
        let doc_a = parse_asterix_record(&schema, &data_a).unwrap();
        let xml_a = infoset_to_xml(&doc_a);
        assert!(xml_a.contains("<fspec1>129</fspec1>"));
        assert!(xml_a.contains("<fspec2>85</fspec2>"));
        assert!(xml_a.contains("<item010>42</item010>"));

        // Case B: fspec1 = 0x80 (bit 7=1 -> item010 present; bit 0=0 -> fspec2 absent)
        let data_b = [0x80, 42];
        let doc_b = parse_asterix_record(&schema, &data_b).unwrap();
        let xml_b = infoset_to_xml(&doc_b);
        assert!(xml_b.contains("<fspec1>128</fspec1>"));
        assert!(!xml_b.contains("<fspec2>"));
        assert!(xml_b.contains("<item010>42</item010>"));

        // Case C: fspec1 = 0x00 (bit 7=0 -> item010 absent; bit 0=0 -> fspec2 absent)
        let data_c = [0x00];
        let doc_c = parse_asterix_record(&schema, &data_c).unwrap();
        let xml_c = infoset_to_xml(&doc_c);
        assert!(xml_c.contains("<fspec1>0</fspec1>"));
        assert!(!xml_c.contains("<fspec2>"));
        assert!(!xml_c.contains("<item010>"));
    }
}
