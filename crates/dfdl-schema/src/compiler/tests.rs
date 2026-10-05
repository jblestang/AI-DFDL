use super::*;

#[test]
fn test_parse_default_value_expressions_and_literals() {
    assert_eq!(
        parse_default_value("42", DfdlSimpleType::Int, None),
        Some(DfdlValue::Int(42))
    );
    assert_eq!(
        parse_default_value("{ 42 }", DfdlSimpleType::Int, None),
        Some(DfdlValue::Int(42))
    );
    assert_eq!(
        parse_default_value("{ (42) }", DfdlSimpleType::Int, None),
        Some(DfdlValue::Int(42))
    );
    assert_eq!(
        parse_default_value("{ 10 + 32 }", DfdlSimpleType::Int, None),
        Some(DfdlValue::Int(42))
    );
    assert_eq!(
        parse_default_value("{ 'hello' }", DfdlSimpleType::String, None),
        Some(DfdlValue::String(String::from("hello")))
    );
    assert_eq!(
        parse_default_value("hello", DfdlSimpleType::String, None),
        Some(DfdlValue::String(String::from("hello")))
    );
    assert_eq!(
        parse_default_value("true", DfdlSimpleType::Boolean, None),
        Some(DfdlValue::Boolean(true))
    );
    assert_eq!(
        parse_default_value("{ true }", DfdlSimpleType::Boolean, None),
        Some(DfdlValue::Boolean(true))
    );

    let mut vmap = VariableMap::new();
    vmap.define_variable(
        dfdl_core::types::QName::local("base"),
        DfdlSimpleType::Int,
        Some(DfdlValue::Int(100)),
    );
    assert_eq!(
        parse_default_value("{ $base + 23 }", DfdlSimpleType::Int, Some(&vmap)),
        Some(DfdlValue::Int(123))
    );
}

#[test]
fn test_compile_schema_with_group_ref_and_root() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        xmlns:ex="http://example.com"
        targetNamespace="http://example.com">
  <group name="setVars">
    <sequence>
      <annotation><appinfo source="http://www.ogf.org/dfdl/">
        <dfdl:setVariable ref="requireLengthInWholeWords" value="yes"/>
      </appinfo></annotation>
    </sequence>
  </group>

  <element name="TwoByteSwapTest">
    <complexType>
      <sequence>
        <group ref="ex:setVars"/>
        <sequence>
          <element name="Block">
            <complexType>
              <sequence>
                <element name="Data" minOccurs="1" maxOccurs="30" type="xs:short"/>
              </sequence>
            </complexType>
          </element>
        </sequence>
      </sequence>
    </complexType>
  </element>
</schema>"#;

    let compiler = SchemaCompiler::new();
    let mut reader = XmlReader::with_limits(xml, compiler.limits);
    let mut resolver = |_loc: &str| -> Option<String> { None };
    let mut visited = Vec::new();
    let xsd_schema = compiler
        .parse_schema_document_internal(&mut reader, &mut resolver, &mut visited, Some(xml))
        .unwrap();
    assert_eq!(xsd_schema.top_level_elements.len(), 1);
    assert_eq!(
        xsd_schema
            .top_level_elements
            .first()
            .map(|e| e.name.local_name.as_str()),
        Some("TwoByteSwapTest")
    );
}

#[test]
fn test_choice_branch_key_duplicate_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root">
    <complexType>
      <choice dfdl:choiceDispatchKey="{ 'A' }">
        <element name="branch1" type="xs:int" dfdl:choiceBranchKey="A"/>
        <element name="branch2" type="xs:string" dfdl:choiceBranchKey="A"/>
      </choice>
    </complexType>
  </element>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err(), "Expected error on duplicate choiceBranchKey");
}

#[test]
fn test_zoned_text_number_rep_non_numeric_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root" type="xs:string" dfdl:lengthKind="explicit" dfdl:length="5" dfdl:representation="text" dfdl:textNumberRep="zoned"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_err(),
        "Expected error on textNumberRep='zoned' for xs:string"
    );
}

#[test]
fn test_invalid_parse_unparse_policy_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root" type="xs:int" dfdl:lengthKind="explicit" dfdl:length="4" dfdl:parseUnparsePolicy="invalidPolicy"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err(), "Expected error on invalid parseUnparsePolicy");
}

#[test]
fn test_invalid_fill_byte_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root" type="xs:string" dfdl:lengthKind="explicit" dfdl:length="4" dfdl:fillByte="tooLongString"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err(), "Expected error on multi-character fillByte");
}

#[test]
fn test_input_value_calc_on_global_element_allowed() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root" type="xs:int" dfdl:inputValueCalc="{ 42 }"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_ok(),
        "inputValueCalc is allowed on global element declarations per DFDL 1.0 §17.1"
    );
}

#[test]
fn test_input_value_calc_and_output_value_calc_both_specified_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="container">
    <complexType>
      <sequence>
        <element name="sub" type="xs:int" dfdl:inputValueCalc="{ 42 }" dfdl:outputValueCalc="{ 42 }"/>
      </sequence>
    </complexType>
  </element>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_err(),
        "Expected error on both inputValueCalc and outputValueCalc"
    );
}

#[test]
fn test_unsupported_layer_transform_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        xmlns:dfdlx="http://www.ogf.org/dfdl/dfdl-1.0/extensions">
  <element name="root" type="xs:string" dfdl:lengthKind="explicit" dfdl:length="4" dfdlx:layerTransform="unsupportedCustomLayer"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err(), "Expected error on unsupported layerTransform");
}

/// A 1-bit signed binary int compiles by default and is rejected when the tunable disallows it.
#[test]
fn test_signed_integer_length_1bit_tunable() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" representation="binary"/>
            <xs:element name="root" type="xs:int" dfdl:length="{ 1 }"/>
        </xs:schema>"#;
    let allowed = SchemaCompiler::new().compile_str(xml);
    assert!(allowed.is_ok(), "1-bit signed int allowed by default: {allowed:?}");
    let denied = SchemaCompiler::new()
        .with_allow_signed_integer_length_1bit(false)
        .compile_str(xml);
    assert!(denied.is_err(), "1-bit signed int rejected when tunable is false");
}

#[test]
fn test_invalid_total_digits_facet_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root">
    <simpleType>
      <restriction base="xs:int">
        <totalDigits value="0"/>
      </restriction>
    </simpleType>
  </element>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err(), "Expected error on totalDigits=0");
}

#[test]
fn test_implicit_length_unequal_min_max_length_facet_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <element name="root" dfdl:representation="text" dfdl:lengthKind="implicit">
    <simpleType>
      <restriction base="xs:string">
        <minLength value="5"/>
        <maxLength value="10"/>
      </restriction>
    </simpleType>
  </element>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_err(),
        "Expected error when lengthKind='implicit' and minLength != maxLength"
    );
}

#[test]
fn test_missing_representation_property_error() {
    let xml = r#"<schema xmlns="http://www.w3.org/2001/XMLSchema"
        xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <annotation>
    <appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" binaryNumberRep="binary" fillByte="%#r20;"/>
    </appinfo>
  </annotation>
  <element name="root" type="xs:int"/>
</schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_err(),
        "Expected error when representation is missing"
    );
    let err_msg = alloc::format!("{}", res.unwrap_err());
    assert!(err_msg.contains("Property representation is not defined"));
}

#[test]
fn test_escape_scheme_literal_whitespace_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:defineEscapeScheme name="table1">
      <dfdl:escapeScheme escapeKind="escapeBlock" escapeBlockStart="[ start ]" escapeBlockEnd="[ end ]"
        escapeEscapeCharacter="%%" extraEscapedCharacters="?" generateEscapeBlock="whenNeeded"/>
    </dfdl:defineEscapeScheme>
    <xs:element name="e1" type="xs:string" />
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(
        res.is_err(),
        "Expected error on literal whitespace in escapeScheme, got: {:?}",
        res
    );
}

#[test]
fn test_missing_text_standard_decimal_separator_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" binaryNumberRep="binary" fillByte="%#r20;" representation="text" textNumberRep="standard" textStandardExponentRep="E"/>
    <xs:element name="myFloat" type="xs:float"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("Property textStandardDecimalSeparator is not defined"));
}

#[test]
fn test_missing_text_standard_exponent_rep_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" binaryNumberRep="binary" fillByte="%#r20;" representation="text" textNumberRep="standard" textStandardDecimalSeparator="."/>
    <xs:element name="myDouble" type="xs:double"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("Property textStandardExponentRep is not defined."));
}

#[test]
fn test_initiated_content_zero_length_initiator_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" binaryNumberRep="binary" fillByte="%#r20;" representation="text"/>
    <xs:element name="root">
        <xs:complexType>
            <xs:sequence dfdl:initiatedContent="yes">
                <xs:element name="child" type="xs:string" dfdl:initiator="%ES;"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("initiatedContent"));
    assert!(err.contains("zero"));
}

#[test]
fn test_bcd_signed_integer_type_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="explicit" length="4" binaryNumberRep="bcd" representation="binary"/>
    <xs:element name="myInt" type="xs:int"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("not an allowed type for bcd"));
}

#[test]
fn test_packed_implicit_length_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="implicit" binaryNumberRep="bcd" representation="binary"/>
    <xs:element name="myUInt" type="xs:unsignedInt"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("lengthKind='implicit' is not allowed with packed binary formats"));
}

#[test]
fn test_packed_bit_length_multiple_of_four_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" length="7" binaryNumberRep="bcd" representation="binary"/>
    <xs:element name="myUInt" type="xs:unsignedInt"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("must be a multiple of 4"));
}

#[test]
fn test_calendar_pattern_fractional_seconds_excess_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myTime" type="xs:time" dfdl:calendarPattern="hh:mm:ss.SSSSSSSSSS"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains(
        "More than 9 fractional seconds unsupported in dfdl:calendarPattern for xs:time"
    ));
}

#[test]
fn test_both_discriminator_and_assert_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myElem" type="xs:string">
        <xs:annotation>
            <xs:appinfo source="http://www.ogf.org/dfdl/">
                <dfdl:assert test="{ fn:true() }"/>
                <dfdl:discriminator test="{ fn:true() }"/>
            </xs:appinfo>
        </xs:annotation>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("both"));
    assert!(err.contains("discriminator"));
    assert!(err.contains("assert"));
}

#[test]
fn test_multiple_discriminator_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myElem" type="xs:string">
        <xs:annotation>
            <xs:appinfo source="http://www.ogf.org/dfdl/">
                <dfdl:discriminator test="{ fn:true() }"/>
                <dfdl:discriminator test="{ fn:true() }"/>
            </xs:appinfo>
        </xs:annotation>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("more than one discriminator"));
}

#[test]
fn test_choice_branch_min_occurs_zero_rejected() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myChoice">
        <xs:complexType>
            <xs:choice>
                <xs:element name="branch1" type="xs:string" minOccurs="0"/>
                <xs:element name="branch2" type="xs:int"/>
            </xs:choice>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("must be non-optional"));
}

#[test]
fn test_occurs_count_kind_fixed_mismatch_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myElem" type="xs:string" minOccurs="1" maxOccurs="5" dfdl:occursCountKind="fixed"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("occursCountKind='fixed'"));
}

#[test]
fn test_nan_range_facet_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myFloat">
        <xs:simpleType>
            <xs:restriction base="xs:float">
                <xs:minInclusive value="NaN"/>
            </xs:restriction>
        </xs:simpleType>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("cannot be NaN"));
}

#[test]
fn test_integer_facet_overflow_error() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:element name="myByte">
        <xs:simpleType>
            <xs:restriction base="xs:byte">
                <xs:maxInclusive value="500"/>
            </xs:restriction>
        </xs:simpleType>
    </xs:element>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("does not fit in type"));
}

#[test]
fn test_custom_namespace_type_resolution() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:custom="http://example.com/custom">
    <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
    <xs:simpleType name="string">
        <xs:restriction base="xs:int"/>
    </xs:simpleType>
    <xs:element name="myNumber" type="custom:string"/>
</xs:schema>"#;
    let compiler = SchemaCompiler::new();
    let schema = compiler.compile_str(xml);
    assert!(matches!(
        schema.as_ref().ok().and_then(|s| s.root_term()),
        Some(term) if matches!(&term.kind, TermKind::Element(elem) if elem.type_ir == CompiledType::Simple(DfdlSimpleType::Int))
    ));
}

#[test]
fn test_choice_dispatch_key_compiler_validation() {
    let compiler = SchemaCompiler::new();

    // 1. Conflict with initiatedContent="yes"
    let xml_init = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="root">
                <xs:complexType>
                    <xs:choice dfdl:choiceDispatchKey="{ ./branch }" dfdl:initiatedContent="yes">
                        <xs:element name="a" type="xs:string" dfdl:choiceBranchKey="1" dfdl:initiator="a:"/>
                    </xs:choice>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err_init = alloc::format!("{}", compiler.compile_str(xml_init).unwrap_err());
    assert!(err_init.contains("choiceDispatchKey is defined with initiatedContent='yes'"));

    // 2. Missing branch key on one of the branches
    let xml_missing = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="root">
                <xs:complexType>
                    <xs:choice dfdl:choiceDispatchKey="{ ./branch }">
                        <xs:element name="a" type="xs:string" dfdl:choiceBranchKey="1"/>
                        <xs:element name="b" type="xs:string"/>
                    </xs:choice>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err_missing = alloc::format!("{}", compiler.compile_str(xml_missing).unwrap_err());
    assert!(err_missing.contains("choiceBranchKey or choiceBranchKeyRanges defined"));

    // 3. choiceBranchKey defined on global element
    let xml_global = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="globalElem" type="xs:string" dfdl:choiceBranchKey="1"/>
        </xs:schema>"#;
    let err_global = alloc::format!("{}", compiler.compile_str(xml_global).unwrap_err());
    assert!(err_global.contains("cannot be defined on a global element declaration"));

    // 4. choiceBranchKeyRanges odd number of values
    let xml_odd = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:dfdlx="http://www.ogf.org/dfdl/dfdl-1.0/extensions">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="root">
                <xs:complexType>
                    <xs:choice dfdl:choiceDispatchKey="{ ./branch }">
                        <xs:element name="a" type="xs:string" dfdlx:choiceBranchKeyRanges="1 10 20"/>
                    </xs:choice>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err_odd = alloc::format!("{}", compiler.compile_str(xml_odd).unwrap_err());
    assert!(err_odd.contains("even number of values"));

    // 5. choiceBranchKey conflict with choiceBranchKeyRanges
    let xml_conflict = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:dfdlx="http://www.ogf.org/dfdl/dfdl-1.0/extensions">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="root">
                <xs:complexType>
                    <xs:choice dfdl:choiceDispatchKey="{ ./branch }">
                        <xs:element name="a" type="xs:string" dfdl:choiceBranchKey="5"/>
                        <xs:element name="b" type="xs:string" dfdlx:choiceBranchKeyRanges="1 10"/>
                    </xs:choice>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err_conflict = alloc::format!("{}", compiler.compile_str(xml_conflict).unwrap_err());
    assert!(err_conflict.contains("conflicts with"));
}

#[test]
fn test_assert_and_discriminator_multiple_forms_rejection() {
    let compiler = SchemaCompiler::new();

    // 1. Both test attribute and body expression on assert
    let xml_both_test_and_body = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="e1" type="xs:int" dfdl:inputValueCalc="{ 42 }">
                <xs:annotation>
                    <xs:appinfo source="http://www.ogf.org/dfdl/">
                        <dfdl:assert test="{ xs:int(.) eq 42 }">{ xs:int(.) eq 42 }</dfdl:assert>
                    </xs:appinfo>
                </xs:annotation>
            </xs:element>
        </xs:schema>"#;
    let err1 = alloc::format!(
        "{}",
        compiler.compile_str(xml_both_test_and_body).unwrap_err()
    );
    assert!(err1.contains("You may not specify both test attribute and a body expression"));

    // 2. Both testPattern attribute and body expression on assert
    let xml_both_pattern_and_body = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="e2" type="xs:int" dfdl:inputValueCalc="{ 42 }">
                <xs:annotation>
                    <xs:appinfo source="http://www.ogf.org/dfdl/">
                        <dfdl:assert testKind="pattern" testPattern="\d\d">\d\d</dfdl:assert>
                    </xs:appinfo>
                </xs:annotation>
            </xs:element>
        </xs:schema>"#;
    let err2 = alloc::format!(
        "{}",
        compiler.compile_str(xml_both_pattern_and_body).unwrap_err()
    );
    assert!(err2.contains("You may not specify both testPattern attribute and a body expression"));

    // 3. Both test and testPattern attributes on assert
    let xml_both_test_and_pattern = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" lengthKind="delimited" representation="text"/>
            <xs:element name="e3" type="xs:int" dfdl:inputValueCalc="{ 42 }">
                <xs:annotation>
                    <xs:appinfo source="http://www.ogf.org/dfdl/">
                        <dfdl:assert test="{ xs:int(.) eq 42 }" testPattern="\d\d"/>
                    </xs:appinfo>
                </xs:annotation>
            </xs:element>
        </xs:schema>"#;
    let err3 = alloc::format!(
        "{}",
        compiler.compile_str(xml_both_test_and_pattern).unwrap_err()
    );
    assert!(err3.contains("You may not specify both test and testPattern attributes"));
}

#[test]
fn test_binary_integer_bit_length_bounds_rejection() {
    let compiler = SchemaCompiler::new();

    // 1. Unsigned binary integer length 0
    let xml_unsigned_zero = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" representation="binary"/>
            <xs:element name="spare1" type="xs:unsignedInt" dfdl:length="{ 0 }"/>
        </xs:schema>"#;
    let err1 = alloc::format!("{}", compiler.compile_str(xml_unsigned_zero).unwrap_err());
    assert!(err1.contains("unsigned binary integer"));
    assert!(err1.contains("1 bit(s)"));
    assert!(err1.contains("0 out of range"));

    // 2. Signed binary integer length 1
    let xml_signed_one = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" representation="binary"/>
            <xs:element name="spare3" type="xs:int" dfdl:length="{ 1 }"/>
        </xs:schema>"#;
    // With the allowSignedIntegerLength1Bit tunable disabled, 1-bit signed is rejected.
    let strict = SchemaCompiler::new().with_allow_signed_integer_length_1bit(false);
    let err2 = alloc::format!("{}", strict.compile_str(xml_signed_one).unwrap_err());
    assert!(err2.contains("signed binary integer"));
    assert!(err2.contains("2 bit(s)"));
    assert!(err2.contains("1 out of range"));

    // 3. Unsigned binary int length 64 (exceeds 32 bits)
    let xml_unsigned_64 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" representation="binary"/>
            <xs:element name="spare4" type="xs:unsignedInt" dfdl:length="64"/>
        </xs:schema>"#;
    let err3 = alloc::format!("{}", compiler.compile_str(xml_unsigned_64).unwrap_err());
    assert!(err3.contains("out of range"));
    assert!(err3.contains("between 1 and 32"));

    // 4. Binary float with dynamic length expression
    let xml_float_expr = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bits" lengthKind="explicit" representation="binary"/>
            <xs:element name="dynFloat" type="xs:float" dfdl:length="{ 32 }"/>
        </xs:schema>"#;
    let err4 = alloc::format!("{}", compiler.compile_str(xml_float_expr).unwrap_err());
    assert!(err4.contains("Floating point binary numbers may not have runtime-specified lengths"));
}

#[test]
fn test_hidden_group_schema_constraints() {
    let compiler = SchemaCompiler::new();

    // 1. Empty hiddenGroupRef
    let xml_empty_hgr = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format lengthKind="delimited"/>
            <xs:element name="e">
                <xs:complexType>
                    <xs:sequence>
                        <xs:sequence dfdl:hiddenGroupRef=""/>
                    </xs:sequence>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err1 = alloc::format!("{}", compiler.compile_str(xml_empty_hgr).unwrap_err());
    assert!(err1.contains("Cannot be empty string QName for hiddenGroupRef"));

    // 2. Sequence with hiddenGroupRef cannot have children
    let xml_hgr_children = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format lengthKind="delimited"/>
            <xs:group name="hg">
                <xs:sequence>
                    <xs:element name="f" type="xs:int" dfdl:outputValueCalc="{ 42 }"/>
                </xs:sequence>
            </xs:group>
            <xs:element name="e">
                <xs:complexType>
                    <xs:sequence>
                        <xs:sequence dfdl:hiddenGroupRef="hg">
                            <xs:element name="illegal" type="xs:int"/>
                        </xs:sequence>
                    </xs:sequence>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err2 = alloc::format!("{}", compiler.compile_str(xml_hgr_children).unwrap_err());
    assert!(err2.contains("A sequence with hiddenGroupRef cannot have children"));

    // 3. Complex type cannot have sequence with hiddenGroupRef as model group
    let xml_ct_hgr = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format lengthKind="delimited"/>
            <xs:group name="hg">
                <xs:sequence>
                    <xs:element name="f" type="xs:int" dfdl:outputValueCalc="{ 42 }"/>
                </xs:sequence>
            </xs:group>
            <xs:element name="e">
                <xs:complexType>
                    <xs:sequence dfdl:hiddenGroupRef="hg"/>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err3 = alloc::format!("{}", compiler.compile_str(xml_ct_hgr).unwrap_err());
    assert!(err3.contains(
        "A complex type cannot have a sequence with a hiddenGroupRef as its model group"
    ));

    // 4. Element in hidden group must be defaultable or have outputValueCalc
    let xml_no_ovc_hg = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format lengthKind="delimited"/>
            <xs:group name="hg">
                <xs:sequence>
                    <xs:element name="f" type="xs:int"/>
                </xs:sequence>
            </xs:group>
            <xs:element name="e">
                <xs:complexType>
                    <xs:sequence>
                        <xs:sequence dfdl:hiddenGroupRef="hg"/>
                        <xs:element name="g" type="xs:int"/>
                    </xs:sequence>
                </xs:complexType>
            </xs:element>
        </xs:schema>"#;
    let err4 = alloc::format!("{}", compiler.compile_str(xml_no_ovc_hg).unwrap_err());
    assert!(err4.contains(
        "Element 'f' in hidden group must be defaultable or define dfdl:outputValueCalc"
    ));
}

#[test]
fn test_query_style_paths_rejected() {
    let compiler = SchemaCompiler::new();

    // 1. Array path step without index predicate is rejected as Query-style path (§23.2)
    let xml_query_path = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="items" type="xs:string" minOccurs="1" maxOccurs="10"/>
                    <xs:element name="target" type="xs:string" dfdl:inputValueCalc="{ /root/items }"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let err = alloc::format!("{}", compiler.compile_str(xml_query_path).unwrap_err());
    assert!(
        err.contains("Query-style paths not supported"),
        "Expected query-style path error, got: {}",
        err
    );

    // 2. Array path step with index predicate is allowed
    let xml_indexed_path = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="items" type="xs:string" minOccurs="1" maxOccurs="10"/>
                    <xs:element name="target" type="xs:string" dfdl:inputValueCalc="{ /root/items[1] }"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_indexed_path).is_ok());

    // 3. Array path inside fn:count is allowed as an exception (§23.5)
    let xml_count_path = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:fn="http://www.w3.org/2005/xpath-functions">
        <dfdl:format lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="items" type="xs:string" minOccurs="1" maxOccurs="10"/>
                    <xs:element name="target" type="xs:int" dfdl:inputValueCalc="{ fn:count(/root/items) }"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_count_path).is_ok());
}

#[test]
fn test_prefix_length_type_bounds_rejected() {
    let compiler = SchemaCompiler::new();

    // prefixLengthType with 64 bits for xs:int must be rejected
    let xml_invalid = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bits"/>
        <xs:simpleType name="plt" dfdl:representation="binary" dfdl:lengthUnits="bits" dfdl:length="64">
            <xs:restriction base="xs:int"/>
        </xs:simpleType>
        <xs:element name="elem" type="xs:string" dfdl:lengthKind="prefixed" dfdl:prefixLengthType="plt"/>
    </xs:schema>"#;
    let err = alloc::format!("{}", compiler.compile_str(xml_invalid).unwrap_err());
    assert!(
        err.contains("Length in bits 64 out of range") && err.contains("between 1 and 32"),
        "Expected bit length error, got: {}",
        err
    );

    // Valid prefixLengthType with 16 bits for xs:int is allowed
    let xml_valid = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bits"/>
        <xs:simpleType name="pltValid" dfdl:representation="binary" dfdl:lengthUnits="bits" dfdl:length="16">
            <xs:restriction base="xs:int"/>
        </xs:simpleType>
        <xs:element name="elem" type="xs:string" dfdl:lengthKind="prefixed" dfdl:prefixLengthType="pltValid"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_valid).is_ok());
}

#[test]
fn test_text_standard_base_rejection() {
    let compiler = SchemaCompiler::new();

    // 1. xs:float with textStandardBase="16" must be rejected
    let xml_float_base16 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="elem" type="xs:float" dfdl:textStandardBase="16"/>
    </xs:schema>"#;
    let err = alloc::format!("{}", compiler.compile_str(xml_float_base16).unwrap_err());
    assert!(err.contains("dfdl:textStandardBase=\"16\" is not allowed for xs:float"), "got: {}", err);

    // 2. xs:double with textStandardBase="2" must be rejected
    let xml_double_base2 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="elem" type="xs:double" dfdl:textStandardBase="2"/>
    </xs:schema>"#;
    let err2 = alloc::format!("{}", compiler.compile_str(xml_double_base2).unwrap_err());
    assert!(err2.contains("dfdl:textStandardBase=\"2\" is not allowed for xs:double"), "got: {}", err2);

    // 3. xs:decimal with textStandardBase="8" must be rejected
    let xml_decimal_base8 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="elem" type="xs:decimal" dfdl:textStandardBase="8"/>
    </xs:schema>"#;
    let err3 = alloc::format!("{}", compiler.compile_str(xml_decimal_base8).unwrap_err());
    assert!(err3.contains("dfdl:textStandardBase=\"8\" is not allowed for xs:decimal"), "got: {}", err3);

    // 4. xs:integer and xs:nonNegativeInteger with textStandardBase="16" are allowed
    let xml_integer_base16 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="elem" type="xs:integer" dfdl:textStandardBase="16"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_integer_base16).is_ok());

    let xml_non_neg_base2 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="elem" type="xs:nonNegativeInteger" dfdl:textStandardBase="2"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_non_neg_base2).is_ok());
}

#[test]
fn test_binary_calendar_rep_validation() {
    let compiler = SchemaCompiler::new();

    // 1. binaryMilliseconds on xs:date is rejected
    let xml_date_millis = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bits"/>
        <xs:element name="elem" type="xs:date" dfdl:lengthKind="explicit" dfdl:length="64" dfdl:binaryCalendarRep="binaryMilliseconds"/>
    </xs:schema>"#;
    let err = alloc::format!("{}", compiler.compile_str(xml_date_millis).unwrap_err());
    assert!(err.contains("binaryCalendarRep='binaryMilliseconds' is not allowed"), "got: {}", err);

    // 2. binaryMilliseconds on xs:time is rejected
    let xml_time_millis = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bits"/>
        <xs:element name="elem" type="xs:time" dfdl:lengthKind="explicit" dfdl:length="64" dfdl:binaryCalendarRep="binaryMilliseconds"/>
    </xs:schema>"#;
    let err2 = alloc::format!("{}", compiler.compile_str(xml_time_millis).unwrap_err());
    assert!(err2.contains("binaryCalendarRep='binaryMilliseconds' is not allowed"), "got: {}", err2);

    // 3. BCD with length not multiple of 4 bits is rejected
    let xml_bcd_bad_len = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bits"/>
        <xs:element name="elem" type="xs:date" dfdl:lengthKind="explicit" dfdl:length="7" dfdl:lengthUnits="bits" dfdl:binaryCalendarRep="bcd" dfdl:calendarPattern="MMddyy" dfdl:calendarPatternKind="explicit"/>
    </xs:schema>"#;
    let err3 = alloc::format!("{}", compiler.compile_str(xml_bcd_bad_len).unwrap_err());
    assert!(err3.contains("must be a multiple of 4"), "got: {}", err3);

    // 4. BCD with '-' in calendarPattern is rejected
    let xml_bcd_bad_pat = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="binary" byteOrder="bigEndian" lengthUnits="bytes"/>
        <xs:element name="elem" type="xs:date" dfdl:lengthKind="explicit" dfdl:length="3" dfdl:lengthUnits="bytes" dfdl:binaryCalendarRep="bcd" dfdl:calendarPattern="MM-dd-yy" dfdl:calendarPatternKind="explicit"/>
    </xs:schema>"#;
    let err4 = alloc::format!("{}", compiler.compile_str(xml_bcd_bad_pat).unwrap_err());
    assert!(err4.contains("Character '-' not allowed in dfdl:calendarPattern"), "got: {}", err4);
}

#[test]
fn test_invalid_default_value_constraint() {
    let compiler = SchemaCompiler::new();

    // 1. Invalid boolean default "FALSE" (lexical space is true, false, 1, 0)
    let xml_bool_bad = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited" textBooleanTrueRep="true 1" textBooleanFalseRep="false 0"/>
        <xs:element name="b" type="xs:boolean" default="FALSE"/>
    </xs:schema>"#;
    let err1 = alloc::format!("{}", compiler.compile_str(xml_bool_bad).unwrap_err());
    assert!(err1.contains("Invalid value constraint value"), "got: {}", err1);

    // 2. Valid boolean defaults "1" and "false" succeed
    let xml_bool_ok = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited" textBooleanTrueRep="true 1" textBooleanFalseRep="false 0"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="b1" type="xs:boolean" default="1"/>
                    <xs:element name="b2" type="xs:boolean" default="false"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(compiler.compile_str(xml_bool_ok).is_ok());

    // 3. Invalid integer default "abc"
    let xml_int_bad = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="n" type="xs:int" default="abc"/>
    </xs:schema>"#;
    let err2 = alloc::format!("{}", compiler.compile_str(xml_int_bad).unwrap_err());
    assert!(err2.contains("Invalid value constraint value"), "got: {}", err2);
}

/// Unit test verifying `dfdl:defineEscapeScheme` definition, reference resolution,
/// and negative schema validation (colon in NCName, missing child escapeScheme).
#[test]
fn test_define_escape_scheme_resolution_and_validation() {
    let compiler = SchemaCompiler::new();

    // 1. Valid top-level defineEscapeScheme resolved via escapeSchemeRef
    let valid_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <dfdl:defineEscapeScheme name="es1">
            <dfdl:escapeScheme escapeKind="escapeCharacter" escapeCharacter="/" escapeEscapeCharacter="" extraEscapedCharacters=""/>
        </dfdl:defineEscapeScheme>
        <xs:element name="field" type="xs:string" dfdl:escapeSchemeRef="es1"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(valid_schema).is_ok());

    // 2. defineEscapeScheme missing child escapeScheme tag must be rejected
    let incomplete_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <dfdl:defineEscapeScheme name="incomplete"/>
        <xs:element name="field" type="xs:string"/>
    </xs:schema>"#;
    let res = compiler.compile_str(incomplete_schema);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("content of element 'dfdl:defineEscapeScheme' is not complete"), "got: {}", err);

    // 3. defineEscapeScheme with colon in name must be rejected
    let colon_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <dfdl:defineEscapeScheme name="foo:bar">
            <dfdl:escapeScheme escapeKind="escapeCharacter" escapeCharacter="/" escapeEscapeCharacter="" extraEscapedCharacters=""/>
        </dfdl:defineEscapeScheme>
        <xs:element name="field" type="xs:string"/>
    </xs:schema>"#;
    let res2 = compiler.compile_str(colon_schema);
    assert!(res2.is_err());
    let err2 = alloc::format!("{}", res2.unwrap_err());
    assert!(err2.contains("must be an NCName and cannot contain a colon"), "got: {}", err2);
}

/// Unit test verifying that scalar elements inheriting `occursCountKind="expression"`
/// from default format do NOT require `dfdl:occursCount`, per DFDL §16.1.
#[test]
fn test_scalar_element_inheriting_occurs_count_kind_expression_accepted() {
    let compiler = SchemaCompiler::new();

    // 1. Scalar element inherits occursCountKind="expression" without dfdl:occursCount
    let scalar_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited" occursCountKind="expression"/>
        <xs:element name="scalarElem" type="xs:int"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(scalar_schema).is_ok());

    // 2. Array element with occursCountKind="expression" must require dfdl:occursCount
    let array_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited" occursCountKind="expression"/>
        <xs:element name="arrElem" type="xs:int" minOccurs="1" maxOccurs="unbounded"/>
    </xs:schema>"#;
    let res = compiler.compile_str(array_schema);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("requires dfdl:occursCount to be defined"), "got: {}", err);
}

/// Unit test verifying hidden group validation recurses into complex child elements
/// and accepts leaf elements defining `outputValueCalc` or default values.
#[test]
fn test_hidden_group_complex_elements_with_ovc() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:group name="hg">
            <xs:sequence>
                <xs:element name="complexLeaf">
                    <xs:complexType>
                        <xs:sequence>
                            <xs:element name="val" type="xs:int" dfdl:outputValueCalc="{ 42 }"/>
                        </xs:sequence>
                    </xs:complexType>
                </xs:element>
            </xs:sequence>
        </xs:group>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:sequence dfdl:hiddenGroupRef="hg"/>
                    <xs:element name="data" type="xs:string"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(compiler.compile_str(schema_xml).is_ok());
}

/// Unit test verifying choice branches can specify both discrete `choiceBranchKey`
/// and continuous `choiceBranchKeyRanges` simultaneously on the same branch.
#[test]
fn test_choice_branch_both_key_and_ranges() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:dfdlx="http://www.ogf.org/dfdl/dfdl-1.0/extensions">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="tag" type="xs:int"/>
                    <xs:choice dfdl:choiceDispatchKey="{ xs:string(../tag) }">
                        <xs:element name="b1" type="xs:string" dfdl:choiceBranchKey="0" dfdlx:choiceBranchKeyRanges="10 20"/>
                        <xs:element name="b2" type="xs:string" dfdl:choiceBranchKey="1" dfdlx:choiceBranchKeyRanges="30 40"/>
                    </xs:choice>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(compiler.compile_str(schema_xml).is_ok());
}

/// Unit test verifying `textStandardExponentRep=""` is accepted as an explicit empty exponent rep.
#[test]
fn test_empty_text_standard_exponent_rep_allowed() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited" textStandardDecimalSeparator="." textStandardGroupingSeparator="," textStandardExponentRep=""/>
        <xs:element name="f" type="xs:float"/>
    </xs:schema>"#;
    assert!(compiler.compile_str(schema_xml).is_ok());
}

/// Unit test verifying that an `<xs:element>` declaration without either a `name`
/// or a `ref` attribute is rejected with a Schema Definition Error.
#[test]
fn test_element_missing_name_and_ref_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element type="xs:int"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("'name'"));
}

/// Unit test verifying that an `<xs:complexType>` with no sequence, choice, or group
/// child is rejected with a Schema Definition Error (DFDL §14).
#[test]
fn test_complex_type_without_model_group_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="emptyComplex">
            <xs:complexType>
                <xs:annotation><xs:documentation>No model group</xs:documentation></xs:annotation>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("A complex type must have exactly one model-group"));
}

/// Unit test verifying that an unordered sequence without members is rejected
/// as a Schema Definition Error (DFDL §14.3).
#[test]
fn test_unordered_sequence_empty_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="emptyUnordered">
            <xs:complexType>
                <xs:sequence dfdl:sequenceKind="unordered" dfdl:separator=","/>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("Unordered sequences must not be empty"));
}

/// Unit test verifying that duplicate element branch names in a choice are
/// rejected under Unique Particle Attribution (UPA) rules (DFDL §15).
#[test]
fn test_choice_duplicate_branch_names_upa_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format representation="text" byteOrder="bigEndian" lengthUnits="bytes" lengthKind="delimited"/>
        <xs:element name="upaRoot">
            <xs:complexType>
                <xs:choice>
                    <xs:element name="dup" type="xs:string"/>
                    <xs:element name="dup" type="xs:string"/>
                </xs:choice>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("Unique Particle Attribution"));
}

/// Unit test verifying that numeric elements with binary representation and
/// delimited lengthKind are rejected (DFDL-12-160R).
#[test]
fn test_binary_delimited_numeric_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes"/>
        <xs:element name="binInt" type="xs:int" dfdl:representation="binary" dfdl:lengthKind="delimited"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("Binary data elements cannot have lengthKind='delimited'"));
}

/// Unit test verifying that calendar elements with text representation cannot
/// use lengthKind="implicit" (DFDL-12-067R).
#[test]
fn test_calendar_text_implicit_length_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes"/>
        <xs:element name="calTime" type="xs:time" dfdl:representation="text" dfdl:lengthKind="implicit"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("cannot have lengthKind='implicit'"));
}

/// Unit test verifying that string elements with lengthKind="implicit" must
/// specify both minLength and maxLength facets and they must be equal (DFDL-5-063R).
#[test]
fn test_string_implicit_length_without_facets_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text"/>
        <xs:element name="strImp" type="xs:string" dfdl:lengthKind="implicit"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("both minLength and maxLength facets must be specified"));
}

/// Unit test verifying that lengthKind="endOfParent" is rejected as unsupported.
#[test]
fn test_end_of_parent_length_kind_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text"/>
        <xs:element name="eop" type="xs:string" dfdl:lengthKind="endOfParent"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("lengthKind='endOfParent' is not implemented"));
}

/// Unit test verifying that dfdlx:objectKind="bytes" requires explicit or prefixed lengthKind.
#[test]
fn test_bytes_object_kind_non_explicit_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:dfdlx="http://www.ogf.org/dfdl/dfdl-1.0/extensions">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="binary"/>
        <xs:element name="blobElem" type="xs:hexBinary" dfdlx:objectKind="bytes" dfdl:lengthKind="delimited"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("objectKind='bytes' must have dfdl:lengthKind='explicit'"));
}

/// Unit test verifying that dfdl:terminator on a delimited element cannot contain %ES;.
#[test]
fn test_delimited_terminator_containing_es_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text"/>
        <xs:element name="termElem" type="xs:string" dfdl:lengthKind="delimited" dfdl:terminator="%ES;"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("dfdl:terminator cannot contain own %ES; when lengthKind='delimited'"));
}

/// Unit test verifying that textNumberPattern with virtual decimal 'V' is forbidden on integer types.
#[test]
fn test_zoned_virtual_decimal_on_integer_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited" textNumberRep="zoned"/>
        <xs:element name="byteElem" type="xs:byte" dfdl:textNumberPattern="+99V99"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("virtual decimal point 'V' is only valid for types xs:decimal, xs:float, xs:double"));
}

/// Unit test verifying that dfdl:occursCount referencing parent complex element '..' is rejected.
#[test]
fn test_occurs_count_expression_parent_complex_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="arr" type="xs:string" minOccurs="1" maxOccurs="unbounded" dfdl:occursCountKind="expression" dfdl:occursCount="{ .. }"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("cannot be converted to xs:unsignedLong"));
}

/// Unit test verifying that dfdl:occursCount with downward relative path is rejected.
#[test]
fn test_occurs_count_expression_non_upward_relative_path_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="arr" type="xs:string" minOccurs="1" maxOccurs="unbounded" dfdl:occursCountKind="expression" dfdl:occursCount="{ down/step }"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("must be absolute or begin with an upward step"));
}

/// Unit test verifying that occursCountKind='implicit' with unbounded maxOccurs cannot precede required elements in a sequence (DFDL §16.1.2).
#[test]
fn test_occurs_count_kind_implicit_unbounded_not_last_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="arr" type="xs:string" minOccurs="1" maxOccurs="unbounded" dfdl:occursCountKind="implicit"/>
                    <xs:element name="required" type="xs:int" minOccurs="1"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("occursCountKind='implicit' with unbounded maxOccurs only allowed for last element"));
}

/// Unit test verifying that inputValueCalc with type mismatch requires manual cast (DFDL §17).
#[test]
fn test_input_value_calc_type_mismatch_rejected() {
    let compiler = SchemaCompiler::new();
    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:element name="intElem" type="xs:int" dfdl:inputValueCalc="{ 2.5 }"/>
    </xs:schema>"#;
    let res = compiler.compile_str(schema_xml);
    assert!(res.is_err());
    let err = alloc::format!("{}", res.unwrap_err());
    assert!(err.contains("must be manually cast to Int"));
}


/// Wraps global declarations and an annotated root group in a minimal DFDL schema and compiles it.
fn compile_var_schema(globals: &str, annotation: &str, component: &str) -> Result<(), String> {
    let xml = alloc::format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        {globals}
        <xs:element name="root">
          <xs:complexType>
            <xs:{component}>
              <xs:annotation><xs:appinfo source="http://www.ogf.org/dfdl/">{annotation}</xs:appinfo></xs:annotation>
              <xs:element name="a" type="xs:string"/>
            </xs:{component}>
          </xs:complexType>
        </xs:element>
    </xs:schema>"#
    );
    SchemaCompiler::new()
        .compile_str(&xml)
        .map(|_| ())
        .map_err(|e| alloc::format!("{}", e))
}

/// DFDL §7.2: a defineVariable default may not be given as both attribute and body.
#[test]
fn test_define_variable_default_attr_and_body_rejected() {
    let g = r#"<dfdl:defineVariable name="v" type="xs:string" defaultValue="x">y</dfdl:defineVariable>"#;
    let err = compile_var_schema(g, "", "sequence").unwrap_err();
    assert!(err.contains("supplied both as attribute and element value"));
    let ok = r#"<dfdl:defineVariable name="v" type="xs:string">y</dfdl:defineVariable>"#;
    assert!(compile_var_schema(ok, "", "sequence").is_ok());
}

/// DFDL §7.7: setVariable value may not be both attribute and body, and refs must be distinct.
#[test]
fn test_set_variable_value_and_distinctness() {
    let g = r#"<dfdl:defineVariable name="v" type="xs:string"/>"#;
    let both = r#"<dfdl:setVariable ref="v" value="1">2</dfdl:setVariable>"#;
    assert!(compile_var_schema(g, both, "sequence")
        .unwrap_err()
        .contains("Cannot have both a value attribute and an element value"));
    let dup = r#"<dfdl:setVariable ref="v" value="1"/><dfdl:setVariable ref="v" value="2"/>"#;
    assert!(compile_var_schema(g, dup, "sequence")
        .unwrap_err()
        .contains("must be distinct at the same location"));
    let one = r#"<dfdl:setVariable ref="v" value="1"/>"#;
    assert!(compile_var_schema(g, one, "sequence").is_ok());
}

/// DFDL §7.4: newVariableInstance refs are distinct and not allowed on elements.
#[test]
fn test_new_variable_instance_distinct_and_placement() {
    let g = r#"<dfdl:defineVariable name="v" type="xs:string"/>"#;
    let dup = r#"<dfdl:newVariableInstance ref="v"/><dfdl:newVariableInstance ref="v"/>"#;
    assert!(compile_var_schema(g, dup, "sequence")
        .unwrap_err()
        .contains("must all be distinct"));
    let one = r#"<dfdl:newVariableInstance ref="v" defaultValue="7"/>"#;
    assert!(compile_var_schema(g, one, "sequence").is_ok());
}

/// Compiles a root sequence with the given separator whose child uses an escape scheme built from the given attributes.
fn compile_escape_schema(scheme_attrs: &str, sep: &str) -> Result<(), String> {
    let xml = alloc::format!(
        r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <dfdl:defineEscapeScheme name="es"><dfdl:escapeScheme {scheme_attrs}/></dfdl:defineEscapeScheme>
        <xs:element name="root"><xs:complexType>
          <xs:sequence dfdl:separator="{sep}">
            <xs:element name="a" type="xs:string" dfdl:escapeSchemeRef="es"/>
          </xs:sequence></xs:complexType></xs:element>
    </xs:schema>"#
    );
    SchemaCompiler::new()
        .compile_str(&xml)
        .map(|_| ())
        .map_err(|e| alloc::format!("{}", e))
}

/// DFDL §13.2.1: separator may not begin with escapeCharacter or escapeEscapeCharacter.
#[test]
fn test_escape_character_conflicts_with_separator() {
    let ec = r##"escapeKind="escapeCharacter" escapeCharacter="#" escapeEscapeCharacter="" extraEscapedCharacters="" generateEscapeBlock="whenNeeded""##;
    assert!(compile_escape_schema(ec, "#").unwrap_err().contains("may not begin with"));
    assert!(compile_escape_schema(ec, ",").is_ok());
    let ee = r#"escapeKind="escapeCharacter" escapeCharacter="/" escapeEscapeCharacter="$" extraEscapedCharacters="" generateEscapeBlock="whenNeeded""#;
    assert!(compile_escape_schema(ee, "$;").unwrap_err().contains("escapeEscapeCharacter"));
    // escapeBlock schemes only conflict through escapeEscapeCharacter.
    let blk = r#"escapeKind="escapeBlock" escapeBlockStart="/*" escapeBlockEnd="*/" escapeEscapeCharacter="" extraEscapedCharacters="" generateEscapeBlock="whenNeeded""#;
    assert!(compile_escape_schema(blk, "#").is_ok());
}

/// DFDL §13.2.1: extraEscapedCharacters items are single characters; byte/class entities rejected.
#[test]
fn test_extra_escaped_characters_validation() {
    let mk = |v: &str| {
        alloc::format!(
            r#"escapeKind="escapeCharacter" escapeCharacter="/" escapeEscapeCharacter="\" extraEscapedCharacters="{v}" generateEscapeBlock="whenNeeded""#
        )
    };
    assert!(compile_escape_schema(&mk("A BB C"), ",").unwrap_err().contains("exactly 1 character"));
    assert!(compile_escape_schema(&mk("%#r0A; ,"), ",").unwrap_err().contains("Byte Entity"));
    assert!(compile_escape_schema(&mk("%#WSP*; ,"), ",").unwrap_err().contains("Invalid DFDL Entity"));
    assert!(compile_escape_schema(&mk("%SP; %#126; %#x21; , | ;"), ".").is_ok());
}

/// DFDL-12-039R: an element declaring lengthKind='explicit' without dfdl:length is an SDE,
/// while an inherited default-format lengthKind is not checked on the element.
#[test]
fn test_explicit_length_kind_requires_length() {
    let mk = |attrs: &str| {
        alloc::format!(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
            <xs:element name="a" type="xs:string" {attrs}/>
        </xs:schema>"#
        )
    };
    let err = SchemaCompiler::new()
        .compile_str(&mk(r#"dfdl:lengthKind="explicit""#))
        .unwrap_err();
    assert!(alloc::format!("{}", err).contains("Property length is not defined"));
    assert!(SchemaCompiler::new()
        .compile_str(&mk(r#"dfdl:lengthKind="explicit" dfdl:length="3""#))
        .is_ok());
    assert!(SchemaCompiler::new().compile_str(&mk("")).is_ok());
}

/// DFDL §12.3 Table 23: lengthKind='delimited' is allowed for packed, bcd, and ibm4690Packed
/// binary numbers, but disallowed for standard binary representation.
#[test]
fn test_binary_delimited_representation_rules() {
    let mk = |type_name: &str, num_rep: &str| {
        alloc::format!(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
            <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="binary" lengthKind="delimited" initiator="" terminator=";"/>
            <xs:element name="a" type="{type_name}" dfdl:binaryNumberRep="{num_rep}"/>
        </xs:schema>"#
        )
    };
    // Packed and IBM4690Packed are allowed with lengthKind="delimited" on xs:int
    assert!(SchemaCompiler::new().compile_str(&mk("xs:int", "packed")).is_ok());
    assert!(SchemaCompiler::new().compile_str(&mk("xs:int", "ibm4690Packed")).is_ok());

    // BCD is allowed with lengthKind="delimited" on unsigned types or decimal
    assert!(SchemaCompiler::new().compile_str(&mk("xs:unsignedInt", "bcd")).is_ok());

    // Standard binary representation is disallowed
    let err = SchemaCompiler::new().compile_str(&mk("xs:int", "binary")).unwrap_err();
    assert!(alloc::format!("{}", err).contains("Binary data elements cannot have lengthKind='delimited'"));
}

/// DFDL §14.4.1: A hidden choice is valid if at least one branch can legally produce output
/// without requiring infoset events (e.g. branch with outputValueCalc), while a hidden choice
/// where no branch can produce output without infoset is rejected.
#[test]
fn test_hidden_choice_validation() {
    let valid_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:group name="c1">
            <xs:choice>
                <xs:element name="x" type="xs:string" dfdl:inputValueCalc="{ 'hello' }"/>
                <xs:element name="y" type="xs:string" dfdl:outputValueCalc="{ 'world' }"/>
            </xs:choice>
        </xs:group>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:sequence dfdl:hiddenGroupRef="c1"/>
                    <xs:element name="body" type="xs:string"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    assert!(SchemaCompiler::new().compile_str(valid_schema).is_ok());

    let invalid_schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:group name="c2">
            <xs:choice>
                <xs:element name="x" type="xs:string" dfdl:inputValueCalc="{ 'hello' }"/>
                <xs:element name="y" type="xs:string"/>
            </xs:choice>
        </xs:group>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:sequence dfdl:hiddenGroupRef="c2"/>
                    <xs:element name="body" type="xs:string"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;
    let err = SchemaCompiler::new().compile_str(invalid_schema).unwrap_err();
    assert!(alloc::format!("{}", err).contains("hidden choice"));
}

/// Verifies that schemas with identical escapeScheme local names under different namespaces
/// are cleanly distinguished and resolved via QName prefix bindings.
#[test]
fn test_multi_namespace_escape_scheme_qname_resolution() {
    // Defines an imported schema with target namespace http://example.com/ns1
    // containing an escapeScheme named 'esc' using escapeCharacter '\'.
    let schema_ns1 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        targetNamespace="http://example.com/ns1">
        <xs:annotation>
            <xs:appinfo source="http://www.ogf.org/dfdl/">
                <dfdl:defineEscapeScheme name="esc">
                    <dfdl:escapeScheme escapeCharacter="\" escapeKind="escapeCharacter"
                        escapeEscapeCharacter="\" extraEscapedCharacters="?" generateEscapeBlock="whenNeeded"/>
                </dfdl:defineEscapeScheme>
            </xs:appinfo>
        </xs:annotation>
    </xs:schema>"#;

    // Defines the primary schema with target namespace http://example.com/ns2
    // importing ns1 and defining its own escapeScheme named 'esc' using escapeCharacter '/'.
    let schema_ns2 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        xmlns:ns1="http://example.com/ns1"
        xmlns:ns2="http://example.com/ns2"
        targetNamespace="http://example.com/ns2">
        <xs:import namespace="http://example.com/ns1" schemaLocation="schema_ns1.xsd"/>
        <xs:annotation>
            <xs:appinfo source="http://www.ogf.org/dfdl/">
                <dfdl:defineEscapeScheme name="esc">
                    <dfdl:escapeScheme escapeCharacter="/" escapeKind="escapeCharacter"
                        escapeEscapeCharacter="/" extraEscapedCharacters="?" generateEscapeBlock="whenNeeded"/>
                </dfdl:defineEscapeScheme>
            </xs:appinfo>
        </xs:annotation>
        <xs:element name="root">
            <xs:complexType>
                <xs:sequence>
                    <xs:element name="item" type="xs:string" dfdl:representation="text"
                        dfdl:lengthKind="delimited" dfdl:escapeSchemeRef="ns2:esc"/>
                </xs:sequence>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;

    // Provide resolver returning schema_ns1 for imported schemaLocation
    let compiler = SchemaCompiler::new();
    let resolver = |loc: &str| -> Option<String> {
        if loc == "schema_ns1.xsd" {
            Some(String::from(schema_ns1))
        } else {
            None
        }
    };

    // Both escapeSchemes must co-exist without "More than one definition" error
    let res = compiler.compile_str_with_resolver(schema_ns2, resolver);
    assert!(res.is_ok(), "Expected compilation to succeed: {:?}", res.err());
}

/// Verifies that top-level `<xs:element ref="..."/>` references an imported definition
/// without triggering a duplicate definition error.
#[test]
fn test_top_level_element_ref_deduplication() {
    // Imported schema defining global element 'base'
    let schema_a = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        targetNamespace="http://example.com/a">
        <xs:element name="base" type="xs:string" dfdl:representation="text" dfdl:lengthKind="delimited"/>
    </xs:schema>"#;

    // Primary schema importing schema_a and exposing <xs:element ref="a:base"/> at the top level
    let schema_base = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        xmlns:a="http://example.com/a"
        targetNamespace="http://example.com/base">
        <xs:import namespace="http://example.com/a" schemaLocation="schema_a.xsd"/>
        <xs:element ref="a:base"/>
    </xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let resolver = |loc: &str| -> Option<String> {
        if loc == "schema_a.xsd" {
            Some(String::from(schema_a))
        } else {
            None
        }
    };

    let res = compiler.compile_str_with_resolver(schema_base, resolver);
    assert!(res.is_ok(), "Expected compilation to succeed: {:?}", res.err());
}

/// Verifies that annotation syntax errors on individual components are recorded
/// and only raised when compiling the erroneous component, allowing valid sibling
/// components in the same schema document to compile and parse successfully.
#[test]
fn test_scoped_component_annotation_error_raised_on_compile() {
    // Schema containing both a valid element and an invalid element with misplaced dfdl:property
    let schema = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" lengthUnits="bytes" representation="text" lengthKind="delimited"/>
        <xs:element name="validElem" type="xs:int">
            <xs:annotation>
                <xs:appinfo source="http://www.ogf.org/dfdl/">
                    <dfdl:element lengthKind="delimited" textNumberRep="standard"/>
                </xs:appinfo>
            </xs:annotation>
        </xs:element>
        <xs:element name="invalidElem" type="xs:int">
            <xs:annotation>
                <xs:appinfo source="http://www.ogf.org/dfdl/">
                    <dfdl:property name="representation">text</dfdl:property>
                </xs:appinfo>
            </xs:annotation>
        </xs:element>
    </xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let dummy_resolver = |_: &str| -> Option<String> { None };

    // Compiling the schema with root="validElem" must succeed
    let compiled_valid = compiler.compile_str_with_resolver_and_root(schema, dummy_resolver, Some("validElem"));
    assert!(compiled_valid.is_ok(), "Expected validElem compilation to succeed");

    // Compiling the schema with root="invalidElem" must fail with the expected SDE
    let compiled_invalid = compiler.compile_str_with_resolver_and_root(schema, dummy_resolver, Some("invalidElem"));
    assert!(compiled_invalid.is_err(), "Expected invalidElem compilation to fail");
    let err_msg = alloc::format!("{}", compiled_invalid.unwrap_err());
    assert!(
        err_msg.contains("The dfdl:property annotation element is not allowed directly under xs:appinfo"),
        "Unexpected error message: {}",
        err_msg
    );
}

#[test]
fn test_debug_property_scoping_s3() {
    let s3 = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
        xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
        xmlns:tns="http://example.com"
        targetNamespace="http://example.com">
    <xs:include schemaLocation="/org/apache/daffodil/xsd/DFDLGeneralFormat.dfdl.xsd"/>
    <dfdl:format ref="tns:GeneralFormat" initiator=""
      terminator="" encoding="utf-8" binaryNumberRep="binary"
      initiatedContent="no" />

    <xs:element name="a">
      <xs:annotation>
        <xs:appinfo source="http://www.ogf.org/dfdl/">
          <dfdl:element representation="binary" />
        </xs:appinfo>
      </xs:annotation>
      <xs:simpleType>
        <xs:annotation>
          <xs:appinfo source="http://www.ogf.org/dfdl/">
            <dfdl:simpleType byteOrder="bigEndian" />
          </xs:appinfo>
        </xs:annotation>
        <xs:restriction base="xs:int" />
      </xs:simpleType>
    </xs:element>

    <xs:element name="aa" type="tns:c">
      <xs:annotation>
        <xs:appinfo source="http://www.ogf.org/dfdl/">
          <dfdl:element representation="binary" />
        </xs:appinfo>
      </xs:annotation>
    </xs:element>

    <xs:simpleType name="c">
      <xs:annotation>
        <xs:appinfo source="http://www.ogf.org/dfdl/">
          <dfdl:simpleType byteOrder="bigEndian" />
        </xs:appinfo>
      </xs:annotation>
      <xs:restriction base="xs:int" />
    </xs:simpleType>

    <xs:element name="aaa" dfdl:lengthKind="implicit">
      <xs:complexType>
        <xs:sequence dfdl:separator="">
          <xs:element ref="tns:aa" dfdl:occursCountKind="fixed"
            minOccurs="3" maxOccurs="3" />
        </xs:sequence>
      </xs:complexType>
    </xs:element>
  </xs:schema>"#;

    extern crate std;
    let gf_xml = std::fs::read_to_string("../dfdl-tests/tests/daffodil/xsd/DFDLGeneralFormat.dfdl.xsd").unwrap();
    let gfb_xml = std::fs::read_to_string("../dfdl-tests/tests/daffodil/xsd/DFDLGeneralFormatBase.dfdl.xsd").unwrap();

    let resolver = move |loc: &str| -> Option<String> {
        if loc.ends_with("DFDLGeneralFormat.dfdl.xsd") {
            Some(gf_xml.clone())
        } else if loc.ends_with("DFDLGeneralFormatBase.dfdl.xsd") {
            Some(gfb_xml.clone())
        } else {
            None
        }
    };

    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str_with_resolver_and_root(s3, resolver, Some("aaa"));
    assert!(res.is_ok(), "Expected s3 compilation to succeed: {:?}", res.err());
}

#[test]
fn test_element_ref_cannot_have_name_and_type() {
    let schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format representation="text" encoding="ASCII" lengthKind="delimited"/>
    </xs:appinfo>
  </xs:annotation>
  <xs:element name="root">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="invalidElem" type="xs:string" ref="targetElem" />
      </xs:sequence>
    </xs:complexType>
  </xs:element>
  <xs:element name="targetElem" type="xs:string" />
</xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(schema);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
    let msg = err.message.as_str();
    assert!(msg.contains("cannot appear"));
    assert!(msg.contains("name"));
    assert!(msg.contains("type"));
}

#[test]
fn test_restriction_unknown_xsd_base_type() {
    let schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format representation="text" encoding="ASCII" lengthKind="delimited"/>
    </xs:appinfo>
  </xs:annotation>
  <xs:simpleType name="badType">
    <xs:restriction base="xs:nonExistent" />
  </xs:simpleType>
  <xs:element name="root" type="badType" />
</xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(schema);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
    let msg = err.message.as_str();
    assert!(msg.contains("Unknown base type"));
    assert!(msg.contains("nonExistent"));
}

#[test]
fn test_empty_escape_scheme_ref_compiles() {
    let schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:format representation="text" encoding="ASCII" lengthKind="delimited" escapeSchemeRef=""/>
    </xs:appinfo>
  </xs:annotation>
  <xs:element name="root" type="xs:string" dfdl:escapeSchemeRef=""/>
</xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let res = compiler.compile_str(schema);
    assert!(res.is_ok(), "Expected empty escapeSchemeRef to compile: {:?}", res.err());
}

#[test]
fn test_prefixed_and_chameleon_format_ref_resolution() {
    let base_fmt_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:defineFormat name="GeneralFormat">
        <dfdl:format representation="text" encoding="ASCII" lengthKind="delimited" emptyElementParsePolicy="treatAsEmpty"/>
      </dfdl:defineFormat>
    </xs:appinfo>
  </xs:annotation>
</xs:schema>"#;

    let main_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
           xmlns:ex="http://example.com"
           targetNamespace="http://example.com">
  <xs:include schemaLocation="base_fmt.xsd"/>
  <xs:annotation>
    <xs:appinfo source="http://www.ogf.org/dfdl/">
      <dfdl:defineFormat name="textual1">
        <dfdl:format ref="ex:GeneralFormat" textStringJustification="left"/>
      </dfdl:defineFormat>
      <dfdl:format ref="ex:textual1"/>
    </xs:appinfo>
  </xs:annotation>
  <xs:element name="root" type="xs:string"/>
</xs:schema>"#;

    let compiler = SchemaCompiler::new();
    let resolver = |loc: &str| -> Option<String> {
        if loc == "base_fmt.xsd" {
            Some(String::from(base_fmt_xsd))
        } else {
            None
        }
    };

    let res = compiler.compile_str_with_resolver(main_xsd, resolver);
    assert!(res.is_ok(), "Expected format ref to resolve successfully: {:?}", res.err());
}

/// DFDL §12.3.1 Table 33: If dfdl:representation is 'binary', dfdl:lengthUnits can only be 'bytes' or 'bits'.
#[test]
fn test_binary_representation_with_characters_length_units_rejected() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
        <dfdl:format byteOrder="bigEndian" alignment="1" representation="binary" lengthKind="explicit" lengthUnits="characters" length="4"/>
        <xs:element name="root" type="xs:int"/>
    </xs:schema>"#;
    let res = SchemaCompiler::new().compile_str(xml);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        alloc::format!("{:?}", err).contains("cannot be 'characters' when dfdl:representation is 'binary'"),
        "Unexpected error: {:?}",
        err
    );
}

/// DFDL §12.3.4: Nested prefixLengthType with depth > 1 is rejected at compile time.
#[test]
fn test_nested_prefixed_depth_validation() {
    let xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/" xmlns:ex="http://example.com" targetNamespace="http://example.com">
        <dfdl:format byteOrder="bigEndian" alignment="1" representation="text" lengthUnits="bytes" encoding="utf-8"/>
        <xs:simpleType name="nest3" dfdl:lengthKind="explicit" dfdl:length="2">
            <xs:restriction base="xs:integer"/>
        </xs:simpleType>
        <xs:simpleType name="nest2" dfdl:lengthKind="prefixed" dfdl:prefixLengthType="ex:nest3">
            <xs:restriction base="xs:integer"/>
        </xs:simpleType>
        <xs:simpleType name="nest1" dfdl:lengthKind="prefixed" dfdl:prefixLengthType="ex:nest2">
            <xs:restriction base="xs:integer"/>
        </xs:simpleType>
        <xs:element name="root" type="xs:string" dfdl:lengthKind="prefixed" dfdl:prefixLengthType="ex:nest1"/>
    </xs:schema>"#;
    let res = SchemaCompiler::new().compile_str(xml);
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        alloc::format!("{:?}", err).contains("Nested dfdl:lengthKind")
            && alloc::format!("{:?}", err).contains("not supported"),
        "Unexpected error: {:?}",
        err
    );
}


