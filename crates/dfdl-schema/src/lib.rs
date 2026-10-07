//! `dfdl-schema`: XSD and DFDL Schema Compiler crate.
//!
//! Transforms XML Schema documents with DFDL annotations into executable [`dfdl_core::schema::ir::CompiledSchema`] IR.
//! Operates strictly under `#![no_std]` + `alloc`. Panic-free by design.

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

pub mod annotation;
pub mod compiler;
pub mod xsd_ast;

pub use compiler::{InvalidRestrictionPolicy, SchemaCompiler};
pub use xsd_ast::{XsdElement, XsdSchema, XsdSequence, XsdTerm, XsdType};

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use dfdl_core::io::traits::ByteOrder;

    #[test]
    fn test_schema_compiler_basic_xsd() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format byteOrder="bigEndian" alignment="1"/>
        </xs:appinfo>
    </xs:annotation>
    <xs:element name="Header" type="xs:int" dfdl:byteOrder="littleEndian" dfdl:lengthKind="explicit" dfdl:length="4"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Header");
        assert_eq!(root.properties.byte_order, ByteOrder::LittleEndian);
        assert_eq!(root.properties.length, Some(4));
    }

    #[test]
    fn test_schema_compiler_complex_sequence() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Packet">
        <xs:complexType>
            <xs:sequence dfdl:byteOrder="bigEndian">
                <xs:element name="Magic" type="xs:int" dfdl:length="4" dfdl:lengthKind="explicit" default="1234"/>
                <xs:element name="Payload" type="xs:string" dfdl:length="16" dfdl:lengthKind="explicit"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Packet");
        if let dfdl_core::schema::ir::TermKind::Element(elem) = &root.kind {
            if let dfdl_core::schema::ir::CompiledType::Complex(seq_id) = elem.type_ir {
                let seq_term = schema.get_term(seq_id).unwrap();
                assert_eq!(seq_term.name.local_name, "sequence");
                if let dfdl_core::schema::ir::TermKind::Sequence(seq) = &seq_term.kind {
                    let m0_id = *seq.members.first().unwrap();
                    let m0 = schema.get_term(m0_id).unwrap();
                    assert_eq!(m0.name.local_name, "Magic");
                    if let dfdl_core::schema::ir::TermKind::Element(el) = &m0.kind {
                        assert_eq!(
                            el.default_value,
                            Some(dfdl_core::infoset::value::DfdlValue::Int(1234))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_schema_compiler_named_group_resolution() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:group name="HeaderGroup">
        <xs:sequence dfdl:byteOrder="bigEndian">
            <xs:element name="Tag" type="xs:int" dfdl:length="2" dfdl:lengthKind="explicit"/>
        </xs:sequence>
    </xs:group>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence>
                <xs:group ref="HeaderGroup"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2" dfdl:lengthKind="explicit"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Record");
    }

    #[test]
    fn test_schema_compiler_multi_file_include() {
        let compiler = SchemaCompiler::new();
        let main_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:include schemaLocation="common.xsd"/>
    <xs:element name="MainElem">
        <xs:complexType>
            <xs:sequence>
                <xs:group ref="CommonGroup"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let common_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:group name="CommonGroup">
        <xs:sequence>
            <xs:element name="CommonChild" type="xs:int" dfdl:length="4" dfdl:lengthKind="explicit"/>
        </xs:sequence>
    </xs:group>
</xs:schema>"#;

        let schema = compiler
            .compile_str_with_resolver(main_xml, |location| {
                if location == "common.xsd" {
                    Some(alloc::string::String::from(common_xml))
                } else {
                    None
                }
            })
            .unwrap();

        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "MainElem");
    }

    #[test]
    fn test_schema_compiler_circular_include() {
        let compiler = SchemaCompiler::new();
        let main_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:include schemaLocation="b.xsd"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Child" type="xs:string" dfdl:length="4" dfdl:lengthKind="explicit"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let b_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:include schemaLocation="main.xsd"/>
</xs:schema>"#;

        // Verify compilation terminates without infinite stack recursion
        let schema = compiler
            .compile_str_with_resolver(main_xml, |location| match location {
                "b.xsd" => Some(alloc::string::String::from(b_xml)),
                "main.xsd" => Some(alloc::string::String::from(main_xml)),
                _ => None,
            })
            .unwrap();

        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Root");
    }

    #[test]
    fn test_schema_compiler_define_and_set_variables() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:defineVariable name="intVar" type="xs:int" defaultValue="42"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence>
                <dfdl:setVariable ref="intVar" value="100"/>
                <xs:element name="Field" type="xs:int" dfdl:length="4" dfdl:lengthKind="explicit"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        assert!(schema.variable_map.get_variable("intVar").is_some());
        assert_eq!(
            schema.variable_map.get_variable("intVar"),
            Some(&dfdl_core::infoset::value::DfdlValue::Int(42))
        );
    }

    #[test]
    fn test_schema_compiler_built_in_types() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format byteOrder="bigEndian" alignment="1" representation="text" encoding="UTF-8" lengthKind="explicit" textStandardDecimalSeparator="."/>
        </xs:appinfo>
    </xs:annotation>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Dec" type="xs:decimal" dfdl:length="4"/>
                <xs:element name="DT" type="xs:dateTime" dfdl:length="19"/>
                <xs:element name="D" type="xs:date" dfdl:length="10"/>
                <xs:element name="T" type="xs:time" dfdl:length="8"/>
                <xs:element name="NNI" type="xs:nonNegativeInteger" dfdl:length="2"/>
                <xs:element name="URI" type="xs:anyURI" dfdl:length="10"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Record");
    }

    #[test]
    fn test_schema_compiler_xs_documentation_handling() {
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:documentation>Sample DFDL Schema documentation text</xs:documentation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthKind="explicit"/>
        </xs:appinfo>
    </xs:annotation>
    <xs:element name="Item" type="xs:int" dfdl:length="4"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "Item");
    }

    #[test]
    fn test_schema_compiler_duplicate_format_merging() {
        let compiler = SchemaCompiler::new();
        let main_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthKind="explicit"/>
        </xs:appinfo>
    </xs:annotation>
    <xs:include schemaLocation="sub.xsd"/>
    <xs:element name="MainItem" type="xs:int" dfdl:length="4"/>
</xs:schema>"#;

        let sub_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format encoding="UTF-8"/>
        </xs:appinfo>
    </xs:annotation>
</xs:schema>"#;

        let schema = compiler
            .compile_str_with_resolver(main_xml, |location| {
                if location == "sub.xsd" {
                    Some(alloc::string::String::from(sub_xml))
                } else {
                    None
                }
            })
            .unwrap();

        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "MainItem");
    }
}
