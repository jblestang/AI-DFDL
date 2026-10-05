//! `dfdl-fuzz`: Fuzz testing targets and property verification for DFDL engine components.

use dfdl_core::io::bitstream::{BitReader, BitWriter};
use dfdl_core::io::sink::VecByteSink;
use dfdl_core::io::source::SliceByteSource;
use dfdl_core::io::traits::{BitOrder, ByteOrder};
use dfdl_core::kernel::{ParserEngine, UnparserEngine};
use dfdl_core::limits::WorkBudget;
use dfdl_schema::SchemaCompiler;
use dfdl_xml::XmlReader;

/// Fuzz target entrypoint for raw XML stream parsing.
///
/// Ensures that arbitrary byte sequences never produce panics or memory safety violations in `XmlReader`.
pub fn fuzz_xml_parse(data: &[u8]) {
    if let Ok(xml_str) = core::str::from_utf8(data) {
        let mut reader = XmlReader::new(xml_str);
        while let Ok(Some(_)) = reader.next_event() {}
    }
}

/// Fuzz target entrypoint for schema compilation.
///
/// Ensures that arbitrary schema strings never cause compilation panics or unexpected crashes.
pub fn fuzz_schema_compile(data: &[u8]) {
    if let Ok(xml_str) = core::str::from_utf8(data) {
        let compiler = SchemaCompiler::new();
        let _ = compiler.compile_str(xml_str);
    }
}

/// Fuzz target entrypoint for bitstream parsing against a valid schema.
///
/// Ensures that malformed input data under valid schemas yields graceful `Err(DFDLError)` instead of panicking.
pub fn fuzz_bitstream_parse(schema_xml: &str, data: &[u8]) {
    let compiler = SchemaCompiler::new();
    if let Ok(schema) = compiler.compile_str(schema_xml) {
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(500);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let _ = parser.parse_document();
    }
}

/// Fuzz target entrypoint for parse -> strip_hidden -> unparse roundtripping.
///
/// Validates invariant that parsing and unparsing arbitrary inputs under valid schemas never panics.
pub fn fuzz_parse_unparse_roundtrip(schema_xml: &str, data: &[u8]) {
    let compiler = SchemaCompiler::new();
    if let Ok(schema) = compiler.compile_str(schema_xml) {
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(500);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        if let Ok(doc) = parser.parse_document() {
            let public_doc = doc.strip_hidden();
            let sink = VecByteSink::new();
            let mut writer = BitWriter::new(
                sink,
                BitOrder::MostSignificantBitFirst,
                ByteOrder::BigEndian,
            );
            let mut unparse_budget = WorkBudget::new(500);

            let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
            let _ = unparser.unparse_document(&public_doc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzz_targets_smoke() {
        fuzz_xml_parse(b"<invalid xml <<>>");
        fuzz_schema_compile(b"<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'/>");

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Val" type="xs:int" dfdl:representation="text" dfdl:length="2"/>
</xs:schema>"#;

        fuzz_bitstream_parse(schema_xml, b"42");
        fuzz_bitstream_parse(schema_xml, b"garbage data!!!");
        fuzz_parse_unparse_roundtrip(schema_xml, b"42");
    }
}
