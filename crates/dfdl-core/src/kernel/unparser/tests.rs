//! Unit tests for DFDL unparser engine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use super::element::*;
use super::framing::*;
use crate::infoset::DfdlValue;
use crate::io::bitstream::BitWriter;
use crate::io::sink::VecByteSink;
use crate::io::traits::{BitOrder, ByteOrder};
use crate::limits::WorkBudget;
use crate::schema::builder::SchemaBuilder;
use crate::schema::ir::{LengthKind, Representation, ResolvedProperties, TermKind};

    /// Unparses `val` as binary with the given byte order and returns the emitted bytes.
    fn unparse_binary_bytes(val: &DfdlValue, order: ByteOrder) -> alloc::vec::Vec<u8> {
        unparse_binary_bytes_len(val, order, None)
    }

    /// Like [`unparse_binary_bytes`] with an optional explicit length in bits.
    fn unparse_binary_bytes_len(
        val: &DfdlValue,
        order: ByteOrder,
        bits: Option<usize>,
    ) -> alloc::vec::Vec<u8> {
        let mut writer = BitWriter::new(
            VecByteSink::new(),
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Float),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        {
            let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
            let props = ResolvedProperties {
                representation: Representation::Binary,
                binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
                byte_order: order,
                length: bits,
                length_units: crate::schema::ir::LengthUnits::Bits,
                ..Default::default()
            };
            unparser.unparse_binary_value(val, &props).unwrap();
        }
        writer.flush().unwrap();
        writer.into_sink().into_vec()
    }

    /// Floats and doubles must be byte-swapped for little-endian output.
    #[test]
    fn test_unparse_binary_float_double_byte_order() {
        let f = DfdlValue::Float(1.0);
        assert_eq!(unparse_binary_bytes(&f, ByteOrder::BigEndian), [0x3F, 0x80, 0, 0]);
        assert_eq!(unparse_binary_bytes(&f, ByteOrder::LittleEndian), [0, 0, 0x80, 0x3F]);
        let d = DfdlValue::Double(1.0);
        assert_eq!(
            unparse_binary_bytes(&d, ByteOrder::LittleEndian),
            [0, 0, 0, 0, 0, 0, 0xF0, 0x3F]
        );
    }

    /// A narrow little-endian xs:int keeps its value; only 32-bit fields are byte-swapped.
    #[test]
    fn test_unparse_int_little_endian_narrow_not_swapped() {
        let one = DfdlValue::Int(1);
        assert_eq!(unparse_binary_bytes_len(&one, ByteOrder::LittleEndian, Some(8)), [1]);
        assert_eq!(
            unparse_binary_bytes_len(&one, ByteOrder::LittleEndian, Some(32)),
            [1, 0, 0, 0]
        );
    }

    #[test]
    fn test_unparse_number_truncation_prohibited_error() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::Double,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(4),
            truncate_specified_length_string: false,
            ..Default::default()
        };
        let val = DfdlValue::Double(0.555);
        let res = unparser.unparse_text_value(&val, &props);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.message.as_str().contains("cannot truncate"));
    }

    /// Runs `unparse_text_value` on a bare schema and returns the bytes (or the error message).
    fn unparse_text_bytes(
        val: &DfdlValue,
        props: &ResolvedProperties,
        order: crate::io::traits::BitOrder,
    ) -> Result<Vec<u8>, String> {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(sink, order, crate::io::traits::ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        {
            let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
            unparser
                .unparse_text_value(val, props)
                .map_err(|e| String::from(e.message.as_str()))?;
        }
        writer.flush().unwrap();
        Ok(writer.into_sink().into_vec())
    }

    /// Sub-byte encodings write `cb`-bit groups: "XX" in 5-bit LSBF is 29,29 packed from bit 0.
    #[test]
    fn test_unparse_sub_byte_text_packs_codes() {
        let enc = crate::encoding::EncodingKind::parse_name("X-DFDL-5-BIT-PACKED-LSBF");
        let props = ResolvedProperties {
            representation: Representation::Text,
            encoding: enc.clone(),
            encoding_char_bits: enc.char_bits(),
            encoding_unit_bits: enc.unit_bits(),
            length_kind: LengthKind::Explicit,
            length: Some(10),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..Default::default()
        };
        let lsb = crate::io::traits::BitOrder::LeastSignificantBitFirst;
        let out = unparse_text_bytes(&DfdlValue::String(String::from("XX")), &props, lsb).unwrap();
        assert_eq!(out, [0xBD, 0x03]);
        // Too long for the 2-character budget and truncation is off: error.
        let err = unparse_text_bytes(&DfdlValue::String(String::from("XXX")), &props, lsb);
        assert!(err.unwrap_err().contains("cannot truncate"));
    }

    /// Octal text is 3 bits per digit.
    #[test]
    fn test_unparse_octal_text_three_bits_per_digit() {
        let enc = crate::encoding::EncodingKind::Octal;
        let props = ResolvedProperties {
            representation: Representation::Text,
            encoding: enc.clone(),
            encoding_char_bits: enc.char_bits(),
            encoding_unit_bits: enc.unit_bits(),
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        // 101 010 11(pad): "52" then "3" -> 101 010 011 -> 0xA9, 0x80
        let out = unparse_text_bytes(&DfdlValue::String(String::from("523")), &props, msb).unwrap();
        assert_eq!(out, [0xA9, 0x80]);
    }

    /// Unmappable characters: `replace` writes the replacement code, `error` fails the unparse.
    #[test]
    fn test_unparse_sub_byte_encoding_error_policy() {
        let enc = crate::encoding::EncodingKind::Packed7Bit;
        let mut props = ResolvedProperties {
            representation: Representation::Text,
            encoding: enc.clone(),
            encoding_char_bits: enc.char_bits(),
            encoding_unit_bits: enc.unit_bits(),
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        let pound = DfdlValue::String(String::from("\u{A3}"));
        // '?' = 0111111 (7 bits) then flushed with zero padding.
        let out = unparse_text_bytes(&pound, &props, msb).unwrap();
        assert_eq!(out, [0b0111_1110]);
        props.encoding_error_policy_error = true;
        let err = unparse_text_bytes(&pound, &props, msb).unwrap_err();
        assert!(err.contains("UnmappableCharacterException"));
    }

    /// Byte encodings honour encodingErrorPolicy too: `replace` writes '?', `error` fails.
    #[test]
    fn test_unparse_byte_encoding_error_policy() {
        let mut props = ResolvedProperties {
            representation: Representation::Text,
            encoding: crate::encoding::EncodingKind::Ascii,
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        let pound = DfdlValue::String(String::from("\u{A3}"));
        assert_eq!(unparse_text_bytes(&pound, &props, msb).unwrap(), [b'?']);
        props.encoding_error_policy_error = true;
        let err = unparse_text_bytes(&pound, &props, msb).unwrap_err();
        assert!(err.contains("UnmappableCharacterException"));
    }

    #[test]
    fn test_unparse_nil_attribute_validation() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Element with nil_attribute: Some(true) on non-nillable element
        let mut unparser1 = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let doc1 = crate::infoset::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                crate::types::QName::local("root"),
                crate::infoset::state::ElementState::Nil,
            )
            .with_nil_attribute(Some(true)),
        );
        let err1 = unparser1.unparse_document(&doc1).unwrap_err();
        assert!(err1
            .message
            .as_str()
            .contains("defines nil property but is not nillable"));

        // 2. Element with nil_attribute: Some(false) on non-nillable element
        let mut unparser2 = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let doc2 = crate::infoset::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                crate::types::QName::local("root"),
                crate::infoset::state::ElementState::Value(DfdlValue::String(
                    alloc::string::String::from("val"),
                )),
            )
            .with_nil_attribute(Some(false)),
        );
        let err2 = unparser2.unparse_document(&doc2).unwrap_err();
        assert!(err2
            .message
            .as_str()
            .contains("defines nil property but is not nillable"));
    }

    #[test]
    fn test_unparse_unexpected_child_and_simple_element_with_children() {
        let mut budget = WorkBudget::new(1000);

        // 1. Simple element with child elements
        let sink1 = VecByteSink::new();
        let mut writer1 = BitWriter::new(
            sink1,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut builder = SchemaBuilder::new();
        let simple_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("simple_root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("simple_root"),
                TermKind::Element(simple_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema1 = builder.build().unwrap();

        let mut unparser1 = UnparserEngine::new(&schema1, &mut writer1, &mut budget);
        let mut bad_simple_elem = crate::infoset::tree::InfosetElement::complex(
            crate::types::QName::local("simple_root"),
        );
        bad_simple_elem
            .try_add_child(crate::infoset::tree::InfosetNode::Element(
                crate::infoset::tree::InfosetElement::simple(
                    crate::types::QName::local("child"),
                    crate::infoset::state::ElementState::Value(DfdlValue::Int(1)),
                ),
            ))
            .unwrap();
        let doc1 = crate::infoset::InfosetDocument::with_root(bad_simple_elem);
        let err1 = unparser1.unparse_document(&doc1).unwrap_err();
        assert!(err1.message.as_str().contains("Illegal content"));
        assert!(err1.message.as_str().contains("simple element"));

        // 2. Choice with unexpected child
        let sink2 = VecByteSink::new();
        let mut writer2 = BitWriter::new(
            sink2,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut builder2 = SchemaBuilder::new();
        let c1_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("c1"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let c1_id = builder2
            .add_term_with_props(
                crate::types::QName::local("c1"),
                TermKind::Element(c1_elem),
                ResolvedProperties::default(),
            )
            .unwrap();

        let c2_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("c2"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let c2_id = builder2
            .add_term_with_props(
                crate::types::QName::local("c2"),
                TermKind::Element(c2_elem),
                ResolvedProperties::default(),
            )
            .unwrap();

        let choice_id = builder2
            .add_term_with_props(
                crate::types::QName::local("choice"),
                TermKind::Choice(crate::schema::ir::CompiledChoice {
                    branches: alloc::vec![c1_id, c2_id],
                }),
                ResolvedProperties::default(),
            )
            .unwrap();

        let choice_root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root_choice"),
            type_ir: crate::schema::ir::CompiledType::Complex(choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let choice_root_id = builder2
            .add_term_with_props(
                crate::types::QName::local("root_choice"),
                TermKind::Element(choice_root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder2.set_root(choice_root_id);
        let schema2 = builder2.build().unwrap();

        let mut unparser2 = UnparserEngine::new(&schema2, &mut writer2, &mut budget);
        let mut bad_choice_elem = crate::infoset::tree::InfosetElement::complex(
            crate::types::QName::local("root_choice"),
        );
        bad_choice_elem
            .try_add_child(crate::infoset::tree::InfosetNode::Element(
                crate::infoset::tree::InfosetElement::simple(
                    crate::types::QName::local("unexpected_branch"),
                    crate::infoset::state::ElementState::Value(DfdlValue::Int(1)),
                ),
            ))
            .unwrap();
        let doc2 = crate::infoset::InfosetDocument::with_root(bad_choice_elem);
        let err2 = unparser2.unparse_document(&doc2).unwrap_err();
        assert!(err2.message.as_str().contains("Found next element"));
        assert!(err2.message.as_str().contains("expected one of"));
        assert!(err2.message.as_str().contains("c1"));
        assert!(err2.message.as_str().contains("c2"));
    }
    /// Padding with an undefined fillByte is a Schema Definition Error (DFDL 12.3.3 / Table 3).
    #[test]
    fn test_fill_byte_value_requires_definition() {
        let mut props = ResolvedProperties {
            fill_byte: 0x20,
            ..ResolvedProperties::default()
        };
        assert_eq!(fill_byte_value(&props).unwrap(), 0x20);
        props.fill_byte_defined = false;
        let err = fill_byte_value(&props).unwrap_err();
        assert!(err.message.as_str().contains("Property fillByte is not defined"));
    }

    /// Verifies that string truncation and padding respect textStringJustification (§13.2).
    #[test]
    fn test_unparse_string_justification_and_truncation() {
        let mut props = ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(5),
            truncate_specified_length_string: true,
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_pad_char: String::from("#"),
            ..ResolvedProperties::default()
        };

        // Right justification: truncate left, pad left
        props.text_string_justification = crate::schema::ir::TextJustification::Right;
        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut budget = WorkBudget::new(10_000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser
            .unparse_simple_value(&DfdlValue::String(String::from("12345678")), &props)
            .unwrap();
        writer.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer.into_sink().as_slice()).unwrap(), "45678");

        // Padding with Right justification
        let mut writer_pad = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser_pad = UnparserEngine::new(&schema, &mut writer_pad, &mut budget);
        unparser_pad
            .unparse_simple_value(&DfdlValue::String(String::from("12")), &props)
            .unwrap();
        writer_pad.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_pad.into_sink().as_slice()).unwrap(), "###12");

        // Center justification: truncation is prohibited by DFDL §13.7.1.3
        props.text_string_justification = crate::schema::ir::TextJustification::Center;
        let mut writer_ctr = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser_ctr = UnparserEngine::new(&schema, &mut writer_ctr, &mut budget);
        let err = unparser_ctr
            .unparse_simple_value(&DfdlValue::String(String::from("1234567")), &props)
            .unwrap_err();
        assert!(err.message.as_str().contains("textStringJustification is 'center'"));
        assert!(err.message.as_str().contains("truncateSpecifiedLengthString is 'yes'"));
    }

    /// Verifies that number padding respects textNumberJustification and textNumberPadCharacter (§13.2.1).
    #[test]
    fn test_unparse_text_number_justification_and_padding() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let num_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("num"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("num"),
                TermKind::Element(num_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let base_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_number_pad_character: Some(String::from("x")),
            ..ResolvedProperties::default()
        };

        // 1. Right justification: "xx20"
        let mut props_right = base_props.clone();
        props_right.text_number_justification = crate::schema::ir::TextJustification::Right;
        let mut writer_r = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_r = UnparserEngine::new(&schema, &mut writer_r, &mut budget);
        unparser_r.unparse_simple_value(&DfdlValue::Int(20), &props_right).unwrap();
        writer_r.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_r.into_sink().as_slice()).unwrap(), "xx20");

        // 2. Left justification: "20xx"
        let mut props_left = base_props.clone();
        props_left.text_number_justification = crate::schema::ir::TextJustification::Left;
        let mut writer_l = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_l = UnparserEngine::new(&schema, &mut writer_l, &mut budget);
        unparser_l.unparse_simple_value(&DfdlValue::Int(20), &props_left).unwrap();
        writer_l.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_l.into_sink().as_slice()).unwrap(), "20xx");

        // 3. Center justification: "x20x"
        let mut props_ctr = base_props;
        props_ctr.text_number_justification = crate::schema::ir::TextJustification::Center;
        let mut writer_c = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_c = UnparserEngine::new(&schema, &mut writer_c, &mut budget);
        unparser_c.unparse_simple_value(&DfdlValue::Int(20), &props_ctr).unwrap();
        writer_c.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_c.into_sink().as_slice()).unwrap(), "x20x");
    }

    /// Verifies measure_element_content_bits accurately measures complex element children bit length with alignment (§17).
    #[test]
    fn test_measure_element_content_bits_complex_alignment() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        // Child 1: binary 4 bits
        let sublen_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sublength"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p1 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p1.fill_byte_defined = true;
        let id1 = builder.add_term_with_props(crate::types::QName::local("sublength"), TermKind::Element(sublen_elem), p1).unwrap();

        // Child 2: binary 5 bits
        let sub1_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("subfield1"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p2 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(5),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p2.fill_byte_defined = true;
        let id2 = builder.add_term_with_props(crate::types::QName::local("subfield1"), TermKind::Element(sub1_elem), p2).unwrap();

        // Child 3: binary 4 bits, alignment 8 bits
        let sub2_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("subfield2"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p3 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 8,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p3.fill_byte_defined = true;
        let id3 = builder.add_term_with_props(crate::types::QName::local("subfield2"), TermKind::Element(sub2_elem), p3).unwrap();

        let seq_id = builder.add_term_with_props(
            crate::types::QName::local("seq"),
            TermKind::Sequence(crate::schema::ir::CompiledSequence { members: alloc::vec![id1, id2, id3] }),
            ResolvedProperties::default(),
        ).unwrap();

        let payload_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("payload"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let payload_id = builder.add_term_with_props(crate::types::QName::local("payload"), TermKind::Element(payload_elem), ResolvedProperties::default()).unwrap();
        builder.set_root(payload_id);
        let schema = builder.build().unwrap();

        let mut payload_node = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("payload"));
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("sublength"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(5)))
        )).unwrap();
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("subfield1"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(21)))
        )).unwrap();
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("subfield2"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(10)))
        )).unwrap();

        let path = crate::types::InfosetPath::root();
        let bits = measure_element_content_bits(&schema, None, &payload_node, &path, None).unwrap();
        // 4 bits (sublength) + 5 bits (subfield1) + 7 bits (alignment padding to 8) + 4 bits (subfield2) = 20 bits
        assert_eq!(bits, 20);
    }

    /// Verifies that textNumberRoundingMode="roundUnnecessary" errors when rounding is needed (§13.7.1.4).
    #[test]
    fn test_unparse_number_rounding_unnecessary() {
        let props = ResolvedProperties {
            text_number_pattern: Some(String::from("##.##;")),
            text_number_rounding_mode: crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary,
            ..ResolvedProperties::default()
        };
        // 0.125 needs rounding to fit 2 fractional digits -> error
        let err = validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.125")), &props).unwrap_err();
        assert!(err.message.as_str().contains("rounding is required"));
        assert!(err.message.as_str().contains("roundUnnecessary"));

        // 0.12 needs no rounding -> ok
        assert!(validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.12")), &props).is_ok());
        // 0.1200 trailing zeros -> ok
        assert!(validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.1200")), &props).is_ok());
    }

    /// Verifies root element namespace validation during unparsing.
    #[test]
    fn test_unparse_root_element_namespace_mismatch() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace(
                "http://example.com",
                "root",
                None,
            ),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(10_000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Document has unqualified root {}root instead of {http://example.com}root
        let unqual_elem = crate::infoset::tree::InfosetElement::simple(
            crate::types::QName::local("root"),
            crate::infoset::state::ElementState::Value(DfdlValue::String(String::from("ok"))),
        );
        let doc = crate::infoset::InfosetDocument::with_root(unqual_elem);
        let err = unparser.unparse_document(&doc).unwrap_err();
        assert!(err.message.as_str().contains("expected element start '{http://example.com}root'"));
        assert!(err.message.as_str().contains("received '{}root'"));
    }

    /// Verifies binary decimal unparsing, virtual scale point, and minimum length bounds (§13.7).
    #[test]
    fn test_unparse_binary_decimal_signedness_and_bounds() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Decimal),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Unsigned binary decimal with 0 bits length -> error
        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget1 = WorkBudget::new(1000);
        let mut unparser1 = UnparserEngine::new(&schema, &mut writer1, &mut budget1);
        let props_u = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: false,
            length: Some(0),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..ResolvedProperties::default()
        };
        let err1 = unparser1.unparse_simple_value(&DfdlValue::Decimal(String::from("1")), &props_u).unwrap_err();
        assert!(err1.message.as_str().contains("unsigned binary number: minimum length is 1 bit(s), but 0 out of range"));

        // 2. Signed binary decimal with 1 bit when disallow_signed_integer_length_1bit is true -> error
        let mut schema_disallow = schema.clone();
        schema_disallow.disallow_signed_integer_length_1bit = true;
        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget2 = WorkBudget::new(1000);
        let mut unparser2 = UnparserEngine::new(&schema_disallow, &mut writer2, &mut budget2);
        let props_s = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: true,
            length: Some(1),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..ResolvedProperties::default()
        };
        let err2 = unparser2.unparse_simple_value(&DfdlValue::Decimal(String::from("1")), &props_s).unwrap_err();
        assert!(err2.message.as_str().contains("signed binary number: minimum length is 2 bit(s), but 1 out of range"));

        // 3. Binary decimal unparsing with virtual decimal point
        let mut writer3 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget3 = WorkBudget::new(1000);
        let mut unparser3 = UnparserEngine::new(&schema, &mut writer3, &mut budget3);
        let props_scale = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: false,
            length: Some(16),
            length_units: crate::schema::ir::LengthUnits::Bits,
            binary_decimal_virtual_point: 2,
            ..ResolvedProperties::default()
        };
        // "12.34" * 10^2 = 1234 (0x04D2)
        unparser3.unparse_simple_value(&DfdlValue::Decimal(String::from("12.34")), &props_scale).unwrap();
        writer3.flush().unwrap();
        assert_eq!(writer3.into_sink().into_vec().as_slice(), &[0x04, 0xD2]);
    }

    #[test]
    fn test_unparser_unexpected_child_element_clark_notation() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::schema::ir::CompiledSequence;

        let mut builder = SchemaBuilder::new();
        let child_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace("http://example.com", "expected_child", None),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let child_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Characters,
            ..Default::default()
        };
        let child_id = builder
            .add_term_with_props(
                crate::types::QName::with_namespace("http://example.com", "expected_child", None),
                TermKind::Element(child_elem),
                child_props,
            )
            .unwrap();

        let seq_id = builder
            .add_term(
                crate::types::QName::local("sequence"),
                TermKind::Sequence(CompiledSequence {
                    members: alloc::vec![child_id],
                }),
            )
            .unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace("http://example.com", "root", None),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(
                crate::types::QName::with_namespace("http://example.com", "root", None),
                TermKind::Element(root_elem),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut root_infoset = InfosetElement::complex(crate::types::QName::with_namespace("http://example.com", "root", None));
        let expected_child = InfosetElement::simple(
            crate::types::QName::with_namespace("http://example.com", "expected_child", None),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("data"))),
        );
        let unexpected_child = InfosetElement::simple(
            crate::types::QName::local("extra"),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("boom"))),
        );
        root_infoset.children.push(InfosetNode::Element(expected_child));
        root_infoset.children.push(InfosetNode::Element(unexpected_child));
        let doc = InfosetDocument::with_root(root_infoset);

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        let err = unparser.unparse_document(&doc).unwrap_err();
        let err_msg = err.message.as_str();
        assert!(err_msg.contains("expected element end for '{http://example.com}root'"), "msg: {}", err_msg);
        assert!(err_msg.contains("received element start '{}extra'"), "msg: {}", err_msg);
    }

    /// Verifies escape scheme unparsing for both EscapeCharacter and EscapeBlock.
    #[test]
    fn test_escape_scheme_unparsing() {
        use crate::schema::ir::{CompiledEscapeScheme, EscapeKind, GenerateEscapeBlock};

        let mut builder = SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(crate::types::QName::local("root"), TermKind::Element(root_elem))
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser.active_delimiters.push(alloc::string::String::from(","));

        // EscapeCharacter scheme with '/' as escape char and '//' escaping itself
        let char_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeCharacter,
            escape_character: Some(alloc::string::String::from("/")),
            escape_escape_character: Some(alloc::string::String::from("/")),
            escape_block_start: None,
            escape_block_end: None,
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };

        assert_eq!(unparser.apply_escape_scheme("plain", &char_scheme), "plain");
        assert_eq!(unparser.apply_escape_scheme("a,b", &char_scheme), "a/,b");
        assert_eq!(unparser.apply_escape_scheme("a/b", &char_scheme), "a//b");
        assert_eq!(unparser.apply_escape_scheme("a/,b", &char_scheme), "a///,b");

        // EscapeBlock scheme with quote delimiters and '#' as escape char
        let block_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: Some(alloc::string::String::from("#")),
            escape_block_start: Some(alloc::string::String::from("'")),
            escape_block_end: Some(alloc::string::String::from("'")),
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };

        assert_eq!(unparser.apply_escape_scheme("plain", &block_scheme), "plain");
        assert_eq!(unparser.apply_escape_scheme("a,b", &block_scheme), "'a,b'");
        assert_eq!(unparser.apply_escape_scheme("a'b", &block_scheme), "'a#'b'");
    }

    #[test]
    fn test_unparse_lsbf_little_endian_float() {
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("f"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Float),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = ResolvedProperties {
            byte_order: ByteOrder::LittleEndian,
            bit_order: BitOrder::LeastSignificantBitFirst,
            representation: crate::schema::ir::Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Implicit,
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("f"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::LeastSignificantBitFirst, ByteOrder::LittleEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // 1.25f32 has IEEE 754 bits 0x3fa00000. LittleEndian byte representation is [0x00, 0x00, 0xa0, 0x3f].
        let elem_info = InfosetElement::simple(crate::types::QName::local("f"), ElementState::Value(DfdlValue::Float(1.25)));
        unparser.unparse_element(root_id, &elem_info).unwrap();
        writer.flush().unwrap();

        assert_eq!(writer.into_sink().into_vec(), alloc::vec![0x00, 0x00, 0xa0, 0x3f]);
    }

    #[test]
    fn test_hidden_group_unparse_preserves_infoset_elements() {
        let mut builder = SchemaBuilder::new();
        // Hidden element with OVC "{ 1 }"
        let hidden_props = ResolvedProperties {
            is_hidden: true,
            output_value_calc: Some(alloc::string::String::from("1")),
            ..Default::default()
        };
        let hidden_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let hidden_id = builder
            .add_term_with_props(crate::types::QName::local("a"), TermKind::Element(hidden_elem), hidden_props)
            .unwrap();

        // Regular element
        let regular_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let regular_id = builder
            .add_term_with_props(crate::types::QName::local("a"), TermKind::Element(regular_elem), ResolvedProperties::default())
            .unwrap();

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![hidden_id, regular_id],
        };
        let seq_id = builder
            .add_term_with_props(crate::types::QName::local("seq"), TermKind::Sequence(seq), ResolvedProperties::default())
            .unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("root"), TermKind::Element(root_elem), ResolvedProperties::default())
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Infoset has ONLY ONE child <a> with value 10 (the regular element)
        let mut root_info = InfosetElement::complex(crate::types::QName::local("root"));
        root_info.children.push(InfosetNode::Element(InfosetElement::simple(
            crate::types::QName::local("a"),
            ElementState::Value(DfdlValue::Int(10)),
        )));

        // Unparse root: hidden <a> must be synthesized with value 1, and regular <a> must consume the infoset's value 10
        unparser.unparse_element(root_id, &root_info).unwrap();
        writer.flush().unwrap();

        let bytes = writer.into_sink().into_vec();
        let text = core::str::from_utf8(&bytes).unwrap();
        assert_eq!(text, "110");
    }

    /// Verifies delimiter entity substitution during unparsing per DFDL §6.3.1.
    #[test]
    fn test_decode_unparse_delimiter_all_entities() {
        // Escaped percent and comma
        assert_eq!(decode_unparse_delimiter("%% %,", "\n"), "% ,");

        // Newline substitution with specific CRLF target
        assert_eq!(decode_unparse_delimiter("header%NL;body", "\r\n"), "header\r\nbody");

        // Control entities
        let input = "%CR;%LF;%NEL;%LS;%FF;%VT;%SP;%HT;%NUL;%ES;";
        let expected = "\r\n\u{0085}\u{2028}\x0C\x0B \t\0";
        assert_eq!(decode_unparse_delimiter(input, "\n"), expected);

        // Whitespace entities
        assert_eq!(decode_unparse_delimiter("%WSP;%WSP+;%WSP*;", "\n"), "  ");

        // Character references in hex and decimal
        let char_refs = "%#x41;%#r42;%#d67;%#68;";
        assert_eq!(decode_unparse_delimiter(char_refs, "\n"), "ABCD");

        // Unrecognized %# fallback
        assert_eq!(decode_unparse_delimiter("%#INVALID;", "\n"), "%#INVALID;");
    }

    /// Verifies that constant and entity-free delimiters return `Cow::Borrowed` with zero allocations.
    #[test]
    fn test_resolve_delimiter_string_cow_zero_allocation() {
        use alloc::borrow::Cow;
        let schema = CompiledSchema::default();
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(sink, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut engine = UnparserEngine::new(&schema, &mut writer, &mut budget);

        let props = ResolvedProperties {
            separator: Some(alloc::string::ToString::to_string(",")),
            separator_prop: Some(crate::schema::ir::DfdlProp::Constant(alloc::string::ToString::to_string(","))),
            ..Default::default()
        };

        let res = engine.resolve_delimiter_string(",", &props).unwrap();
        assert!(matches!(res, Cow::Borrowed(_)));
        assert_eq!(res.as_ref(), ",");
    }

    /// Verifies fill byte extraction and bitstream padding across byte and sub-byte boundaries.
    #[test]
    fn test_fill_byte_value_and_padding_logic() {
        // When fillByte is not defined, fill_byte_value must return SchemaDefinition error
        let props_undefined = ResolvedProperties {
            fill_byte_defined: false,
            ..Default::default()
        };
        assert!(fill_byte_value(&props_undefined).is_err());

        // When defined, returns the byte value
        let props_defined = ResolvedProperties {
            fill_byte_defined: true,
            fill_byte: 0xAA,
            ..Default::default()
        };
        assert_eq!(fill_byte_value(&props_defined).unwrap(), 0xAA);

        // 0-bit padding is a no-op
        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        assert!(write_fill_padding(&mut writer1, 0xFF, 0).is_ok());
        writer1.flush().unwrap();
        assert!(writer1.into_sink().into_vec().is_empty());

        // Full 2 bytes padding with MSBF
        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        assert!(write_fill_padding(&mut writer2, 0x5A, 16).is_ok());
        writer2.flush().unwrap();
        assert_eq!(writer2.into_sink().into_vec(), alloc::vec![0x5A, 0x5A]);

        // Sub-byte padding (3 bits) with MSBF: 0x80 >> (8 - 3) = 0x80 >> 5 = 4
        let mut writer3 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        assert!(write_fill_padding(&mut writer3, 0b1000_0000, 3).is_ok());
        // Remaining 5 bits to round to full byte
        writer3.write_bits(0, 5).unwrap();
        writer3.flush().unwrap();
        assert_eq!(writer3.into_sink().into_vec(), alloc::vec![0b1000_0000]);

        // Sub-byte padding (3 bits) with LSBF: 0x05 & 0b111 = 5
        let mut writer4 = BitWriter::new(VecByteSink::new(), BitOrder::LeastSignificantBitFirst, ByteOrder::BigEndian);
        assert!(write_fill_padding(&mut writer4, 0b0000_0101, 3).is_ok());
        writer4.write_bits(0, 5).unwrap();
        writer4.flush().unwrap();
        assert_eq!(writer4.into_sink().into_vec(), alloc::vec![0b0000_0101]);
    }

    /// Verifies that BCD unparsing rejects negative integers, and IBM 4690 rejects invalid non-numeric types.
    #[test]
    fn test_unparse_bcd_and_ibm4690_error_branches() {
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("val"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let bcd_props = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Bcd,
            length_kind: crate::schema::ir::LengthKind::Implicit,
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("val"), TermKind::Element(elem), bcd_props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // BCD negative int value returns an unparse error
        let neg_info = InfosetElement::simple(crate::types::QName::local("val"), ElementState::Value(DfdlValue::Int(-42)));
        let err = unparser.unparse_element(root_id, &neg_info).unwrap_err();
        assert!(err.message.as_str().contains("bcd only positive, cannot be negative"));

        // Setup IBM 4690 schema with Boolean type to reach packed decimal type mismatch error
        let mut builder2 = SchemaBuilder::new();
        let elem2 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("ibm"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Boolean),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let ibm_props = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Ibm4690Packed,
            length_kind: crate::schema::ir::LengthKind::Implicit,
            ..Default::default()
        };
        let root_id2 = builder2
            .add_term_with_props(crate::types::QName::local("ibm"), TermKind::Element(elem2), ibm_props)
            .unwrap();
        builder2.set_root(root_id2);
        let schema2 = builder2.build().unwrap();

        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget2 = WorkBudget::new(1000);
        let mut unparser2 = UnparserEngine::new(&schema2, &mut writer2, &mut budget2);

        // IBM4690 non-numeric value returns an unparse error
        let non_num_info = InfosetElement::simple(crate::types::QName::local("ibm"), ElementState::Value(DfdlValue::Boolean(true)));
        let err2 = unparser2.unparse_element(root_id2, &non_num_info).unwrap_err();
        assert!(err2.message.as_str().contains("Cannot unparse Boolean(true) as ibm4690 packed decimal"));
    }

    /// Verifies xs:hexBinary string facet validation errors (odd length and non-hex characters).
    #[test]
    fn test_unparse_hex_binary_facet_validation_errors() {
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("hexElem"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::HexBinary),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("hexElem"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Odd length hex string
        let odd_info = InfosetElement::simple(
            crate::types::QName::local("hexElem"),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("ABC"))),
        );
        let err_odd = unparser.unparse_element(root_id, &odd_info).unwrap_err();
        assert!(err_odd.message.as_str().contains("even number of characters required"));

        // Invalid hex characters
        let invalid_info = InfosetElement::simple(
            crate::types::QName::local("hexElem"),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("ABZZ"))),
        );
        let err_inv = unparser.unparse_element(root_id, &invalid_info).unwrap_err();
        assert!(err_inv.message.as_str().contains("invalid hex digits"));
    }

    /// Verifies unparsing error paths when document root is missing or has a mismatched QName.
    #[test]
    fn test_unparse_document_empty_and_mismatch_errors() {
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("expectedRoot"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("expectedRoot"), TermKind::Element(elem), ResolvedProperties::default())
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Empty document with no root element
        let empty_doc = InfosetDocument { root: None, total_nodes: 0 };
        let err_empty = unparser.unparse_document(&empty_doc).unwrap_err();
        assert!(err_empty.message.as_str().contains("Cannot unparse an empty InfosetDocument with no root element"));

        // Mismatched root element name
        let wrong_root = InfosetElement::simple(
            crate::types::QName::local("actualRoot"),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("val"))),
        );
        let mismatched_doc = InfosetDocument::with_root(wrong_root);
        let err_mismatch = unparser.unparse_document(&mismatched_doc).unwrap_err();
        assert!(err_mismatch.message.as_str().contains("expected element start '{}expectedRoot', received '{}actualRoot'"));

        // External variable setting and warning escalation
        assert!(unparser.set_external_variable("var1", "100").is_ok());
        unparser.set_escalate_warnings(true);
        assert!(unparser.variable_map().escalate_warnings);
    }

    /// Verifies unparser framing leading skip, bitOrder change byte alignment, and array min/maxOccurs bounds enforcement.
    #[test]
    fn test_unparser_framing_and_occurrence_errors() {
        dfdl_layers::init();
        // 1. BitOrder change on non-byte boundary error (lines 235-246)
        let mut builder_bo = SchemaBuilder::new();
        let elem_bo = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root_bo"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_lsbf = ResolvedProperties {
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            bit_order: BitOrder::LeastSignificantBitFirst,
            ..Default::default()
        };
        let root_bo_id = builder_bo.add_term_with_props(crate::types::QName::local("root_bo"), TermKind::Element(elem_bo), props_lsbf).unwrap();
        builder_bo.set_root(root_bo_id);
        let schema_bo = builder_bo.build().unwrap();

        let mut writer_bo = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        // Write unaligned 3 bits
        writer_bo.write_bits(0b101, 3).unwrap();
        let mut budget_bo = WorkBudget::new(1000);
        let mut unparser_bo = UnparserEngine::new(&schema_bo, &mut writer_bo, &mut budget_bo);
        let elem_info = InfosetElement::simple(crate::types::QName::local("root_bo"), ElementState::Value(DfdlValue::Byte(1)));
        let doc_bo = InfosetDocument::with_root(elem_info);
        let res_bo = unparser_bo.unparse_document(&doc_bo);
        assert!(res_bo.is_err());
        assert!(res_bo.unwrap_err().message.as_str().contains("Can only change bitOrder on a byte boundary"));

        // 2. Element occurrences below minOccurs and above maxOccurs (lines 303-326)
        let mut builder_occ = SchemaBuilder::new();
        let child_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("item"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 2,
            max_occurs: Some(3),
            is_nillable: false,
            default_value: None,
        };
        let child_id = builder_occ.add_term_with_props(crate::types::QName::local("item"), TermKind::Element(child_elem), ResolvedProperties::default()).unwrap();
        let seq = crate::schema::ir::CompiledSequence { members: alloc::vec![child_id] };
        let seq_id = builder_occ.add_term_with_props(crate::types::QName::local("seq"), TermKind::Sequence(seq), ResolvedProperties::default()).unwrap();
        let parent_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("container"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let parent_id = builder_occ.add_term_with_props(crate::types::QName::local("container"), TermKind::Element(parent_elem), ResolvedProperties::default()).unwrap();
        builder_occ.set_root(parent_id);
        let schema_occ = builder_occ.build().unwrap();

        // Too few occurrences (1 found, minOccurs is 2)
        let mut elem_too_few = InfosetElement::complex(crate::types::QName::local("container"));
        elem_too_few.try_add_child(InfosetNode::Element(InfosetElement::simple(crate::types::QName::local("item"), ElementState::Value(DfdlValue::Int(1))))).unwrap();
        let doc_too_few = InfosetDocument::with_root(elem_too_few.clone());

        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget1 = WorkBudget::new(1000);
        let mut unparser1 = UnparserEngine::new(&schema_occ, &mut writer1, &mut budget1);
        let err_too_few = unparser1.unparse_document(&doc_too_few).unwrap_err();
        assert!(err_too_few.message.as_str().contains("at least 2 required"));

        // Direct unparse_term for element occurrence lower bound
        let mut writer_direct1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_direct1 = WorkBudget::new(1000);
        let mut unparser_direct1 = UnparserEngine::new(&schema_occ, &mut writer_direct1, &mut budget_direct1);
        let err_direct1 = unparser_direct1.unparse_term(child_id, &elem_too_few).unwrap_err();
        assert!(err_direct1.message.as_str().contains("Expected at least 2 occurrence(s)"));

        // Too many occurrences (4 found, maxOccurs is 3)
        let mut elem_too_many = InfosetElement::complex(crate::types::QName::local("container"));
        for i in 0..4 {
            elem_too_many.try_add_child(InfosetNode::Element(InfosetElement::simple(crate::types::QName::local("item"), ElementState::Value(DfdlValue::Int(i))))).unwrap();
        }
        let doc_too_many = InfosetDocument::with_root(elem_too_many.clone());

        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget2 = WorkBudget::new(1000);
        let mut unparser2 = UnparserEngine::new(&schema_occ, &mut writer2, &mut budget2);
        let err_too_many = unparser2.unparse_document(&doc_too_many).unwrap_err();
        assert!(err_too_many.message.as_str().contains("exceeding maxOccurs 3"));

        // Direct unparse_term for element occurrence upper bound
        let mut writer_direct2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_direct2 = WorkBudget::new(1000);
        let mut unparser_direct2 = UnparserEngine::new(&schema_occ, &mut writer_direct2, &mut budget_direct2);
        let err_direct2 = unparser_direct2.unparse_term(child_id, &elem_too_many).unwrap_err();
        assert!(err_direct2.message.as_str().contains("exceeding maxOccurs 3"));

        // 3. Non-element sequence leading_skip during unparsing (lines 223-230)
        let mut builder_skip = SchemaBuilder::new();
        let skip_props = ResolvedProperties {
            leading_skip: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bytes,
            ..Default::default()
        };
        let empty_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let skip_seq_id = builder_skip.add_term_with_props(crate::types::QName::local("s"), TermKind::Sequence(empty_seq), skip_props).unwrap();
        builder_skip.set_root(skip_seq_id);
        let schema_skip = builder_skip.build().unwrap();

        let mut writer_skip = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_skip = WorkBudget::new(1000);
        let mut unparser_skip = UnparserEngine::new(&schema_skip, &mut writer_skip, &mut budget_skip);
        let elem_dummy = InfosetElement::complex(crate::types::QName::local("s"));
        assert!(unparser_skip.unparse_term(skip_seq_id, &elem_dummy).is_ok());

        // 4. Non-element sequence bitOrder change on non-byte boundary (lines 236-246)
        let mut builder_bo_seq = SchemaBuilder::new();
        let bo_props = ResolvedProperties {
            bit_order: BitOrder::LeastSignificantBitFirst,
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..Default::default()
        };
        let empty_bo_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let bo_seq_id = builder_bo_seq.add_term_with_props(crate::types::QName::local("bo_seq"), TermKind::Sequence(empty_bo_seq), bo_props).unwrap();
        builder_bo_seq.set_root(bo_seq_id);
        let schema_bo_seq = builder_bo_seq.build().unwrap();

        let mut writer_bo_seq = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        writer_bo_seq.write_bits(0b11, 2).unwrap(); // non-byte boundary
        let mut budget_bo_seq = WorkBudget::new(1000);
        let mut unparser_bo_seq = UnparserEngine::new(&schema_bo_seq, &mut writer_bo_seq, &mut budget_bo_seq);
        let err_bo_seq = unparser_bo_seq.unparse_term(bo_seq_id, &elem_dummy).unwrap_err();
        assert!(err_bo_seq.message.as_str().contains("Can only change bitOrder on a byte boundary"));

        // 5. Missing root Term NodeId from compiled schema graph in unparse_document (lines 140-145)
        let mut schema_missing_root = schema_skip.clone();
        schema_missing_root.root_element_id = NodeId(999999);
        let mut writer_miss = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_miss = WorkBudget::new(1000);
        let mut unparser_miss = UnparserEngine::new(&schema_missing_root, &mut writer_miss, &mut budget_miss);
        let doc_dummy = InfosetDocument::with_root(elem_dummy.clone());
        let err_miss = unparser_miss.unparse_document(&doc_dummy).unwrap_err();
        assert!(err_miss.message.as_str().contains("Term NodeId missing from compiled schema graph"));

        // 6. Missing child Term NodeId in unparse_term (lines 183-188)
        let err_miss_child = unparser_skip.unparse_term(NodeId(888888), &elem_dummy).unwrap_err();
        assert!(err_miss_child.message.as_str().contains("Term NodeId missing from compiled schema graph"));

        // 7. Non-element sequence leading_skip with Bits units (line 227)
        let mut builder_skip_bits = SchemaBuilder::new();
        let skip_bits_props = ResolvedProperties {
            leading_skip: 3,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..Default::default()
        };
        let skip_bits_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let skip_bits_id = builder_skip_bits.add_term_with_props(crate::types::QName::local("sb"), TermKind::Sequence(skip_bits_seq), skip_bits_props).unwrap();
        builder_skip_bits.set_root(skip_bits_id);
        let schema_skip_bits = builder_skip_bits.build().unwrap();

        let mut writer_sb = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_sb = WorkBudget::new(1000);
        let mut unparser_sb = UnparserEngine::new(&schema_skip_bits, &mut writer_sb, &mut budget_sb);
        assert!(unparser_sb.unparse_term(skip_bits_id, &elem_dummy).is_ok());
        assert_eq!(writer_sb.position().0, 3);

        // 8. twoByteSwap layer odd length error in unparser (lines 441-446)
        let mut builder_swap = SchemaBuilder::new();
        let swap_props = ResolvedProperties {
            layer: Some("twoByteSwap".into()),
            ..Default::default()
        };
        let elem_child = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("b"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::value::DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let b_props = ResolvedProperties {
            representation: Representation::Binary,
            ..Default::default()
        };
        let b_id = builder_swap.add_term_with_props(crate::types::QName::local("b"), TermKind::Element(elem_child), b_props).unwrap();
        let swap_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![b_id] };
        let swap_id = builder_swap.add_term_with_props(crate::types::QName::local("sw"), TermKind::Sequence(swap_seq), swap_props).unwrap();
        builder_swap.set_root(swap_id);
        let schema_swap = builder_swap.build().unwrap();

        let mut writer_swap = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_swap = WorkBudget::new(1000);
        let mut unparser_swap = UnparserEngine::new(&schema_swap, &mut writer_swap, &mut budget_swap);
        unparser_swap.variable_map_mut().define_variable(
            crate::types::QName::local("requireLengthInWholeWords"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("yes".into())),
        );
        let mut elem_complex = InfosetElement::complex(crate::types::QName::local("sw"));
        elem_complex.children.push(crate::infoset::InfosetNode::Element(
            InfosetElement::simple(crate::types::QName::local("b"), ElementState::Value(DfdlValue::Byte(42)))
        ));
        let err_swap = unparser_swap.unparse_term(swap_id, &elem_complex).unwrap_err();
        assert!(err_swap.message.as_str().contains("not a multiple of 2 for twoByteSwap layer"));
    }

    #[test]
    fn test_unparser_layers_extended() {
        use crate::infoset::value::DfdlSimpleType;
        use crate::schema::builder::SchemaBuilder;

        // 1. stlBombOutLayer in unparser
        let mut b_bomb = SchemaBuilder::new();
        let bomb_props = ResolvedProperties {
            layer: Some("stlBombOutLayer".into()),
            ..Default::default()
        };
        let bomb_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let bomb_id = b_bomb.add_term_with_props(crate::types::QName::local("bomb"), TermKind::Sequence(bomb_seq), bomb_props).unwrap();
        b_bomb.set_root(bomb_id);
        let schema_bomb = b_bomb.build().unwrap();

        // PE bomb
        let mut w1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b1 = WorkBudget::new(1000);
        let mut u1 = UnparserEngine::new(&schema_bomb, &mut w1, &mut b1);
        u1.variable_map_mut().define_variable(
            crate::types::QName::local("bombWhere"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("write".into())),
        );
        u1.variable_map_mut().define_variable(
            crate::types::QName::local("bombHow"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("PE".into())),
        );
        let elem_c = InfosetElement::complex(crate::types::QName::local("bomb"));
        let err1 = u1.unparse_term(bomb_id, &elem_c).unwrap_err();
        assert!(err1.message.as_str().contains("Unparse Error: Bombed out at write"));

        // RSDE bomb
        let mut w2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b2 = WorkBudget::new(1000);
        let mut u2 = UnparserEngine::new(&schema_bomb, &mut w2, &mut b2);
        u2.variable_map_mut().define_variable(
            crate::types::QName::local("bombWhere"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("write".into())),
        );
        u2.variable_map_mut().define_variable(
            crate::types::QName::local("bombHow"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("RSDE".into())),
        );
        let err2 = u2.unparse_term(bomb_id, &elem_c).unwrap_err();
        assert!(err2.message.as_str().contains("Runtime Schema Definition Error: Bombed out at write"));

        // 2. boundaryMark layer in unparser
        let mut b_bm = SchemaBuilder::new();
        let bm_props = ResolvedProperties {
            layer: Some("boundaryMark".into()),
            ..Default::default()
        };
        let bm_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let bm_id = b_bm.add_term_with_props(crate::types::QName::local("bm"), TermKind::Sequence(bm_seq), bm_props).unwrap();
        b_bm.set_root(bm_id);
        let schema_bm = b_bm.build().unwrap();

        let mut w_bm = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_bm_work = WorkBudget::new(1000);
        let mut u_bm = UnparserEngine::new(&schema_bm, &mut w_bm, &mut b_bm_work);
        u_bm.variable_map_mut().define_variable(
            crate::types::QName::local("boundaryMark"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("##".into())),
        );
        assert!(u_bm.unparse_term(bm_id, &elem_c).is_ok());
        w_bm.flush().unwrap();
        assert_eq!(w_bm.into_sink().into_vec(), b"##");

        // 3. gzip and base64_mime layers in unparser
        let mut b_gz = SchemaBuilder::new();
        let gz_props = ResolvedProperties {
            layer: Some("gzip".into()),
            ..Default::default()
        };
        let gz_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let gz_id = b_gz.add_term_with_props(crate::types::QName::local("gz"), TermKind::Sequence(gz_seq), gz_props).unwrap();
        b_gz.set_root(gz_id);
        let schema_gz = b_gz.build().unwrap();

        let mut w_gz = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_gz_work = WorkBudget::new(1000);
        let mut u_gz = UnparserEngine::new(&schema_gz, &mut w_gz, &mut b_gz_work);
        u_gz.variable_map_mut().define_variable(
            crate::types::QName::local("compressionLevel"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(9)),
        );
        assert!(u_gz.unparse_term(gz_id, &elem_c).is_ok());

        let mut b_b64 = SchemaBuilder::new();
        let b64_props = ResolvedProperties {
            layer: Some("base64_mime".into()),
            ..Default::default()
        };
        let b64_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let b64_id = b_b64.add_term_with_props(crate::types::QName::local("b64"), TermKind::Sequence(b64_seq), b64_props).unwrap();
        b_b64.set_root(b64_id);
        let schema_b64 = b_b64.build().unwrap();

        let mut w_b64 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_b64_work = WorkBudget::new(1000);
        let mut u_b64 = UnparserEngine::new(&schema_b64, &mut w_b64, &mut b_b64_work);
        assert!(u_b64.unparse_term(b64_id, &elem_c).is_ok());

        // 4. stlBombOutLayer PE and RSDE errors during unparsing (lines 360-390)
        let mut b_bomb = SchemaBuilder::new();
        let bomb_props = ResolvedProperties {
            layer: Some("stlBombOutLayer".into()),
            ..Default::default()
        };
        let bomb_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![] };
        let bomb_id = b_bomb.add_term_with_props(crate::types::QName::local("bomb"), TermKind::Sequence(bomb_seq), bomb_props).unwrap();
        b_bomb.set_root(bomb_id);
        let schema_bomb = b_bomb.build().unwrap();

        let mut w_b1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_b1_work = WorkBudget::new(1000);
        let mut u_b1 = UnparserEngine::new(&schema_bomb, &mut w_b1, &mut b_b1_work);
        u_b1.variable_map_mut().define_variable(
            crate::types::QName::local("bombWhere"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("write".into())),
        );
        u_b1.variable_map_mut().define_variable(
            crate::types::QName::local("bombHow"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("PE".into())),
        );
        let err_b1 = u_b1.unparse_term(bomb_id, &elem_c).unwrap_err();
        assert!(err_b1.message.as_str().contains("Unparse Error: Bombed out at write"));

        let mut w_b2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_b2_work = WorkBudget::new(1000);
        let mut u_b2 = UnparserEngine::new(&schema_bomb, &mut w_b2, &mut b_b2_work);
        u_b2.variable_map_mut().define_variable(
            crate::types::QName::local("bombWhere"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("write".into())),
        );
        u_b2.variable_map_mut().define_variable(
            crate::types::QName::local("bombHow"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("RSDE".into())),
        );
        let err_b2 = u_b2.unparse_term(bomb_id, &elem_c).unwrap_err();
        assert!(err_b2.message.as_str().contains("Runtime Schema Definition Error: Bombed out at write"));

        // 5. twoByteSwap layer requires whole words with odd bytes during unparsing (lines 441-446)
        let mut b_tbs = SchemaBuilder::new();
        let tbs_props = ResolvedProperties {
            layer: Some("twoByteSwap".into()),
            ..Default::default()
        };
        let odd_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("odd_byte"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let odd_id = b_tbs.add_term_with_props(crate::types::QName::local("odd_byte"), TermKind::Element(odd_elem), ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            length_kind: crate::schema::ir::LengthKind::Implicit,
            ..Default::default()
        }).unwrap();
        let tbs_seq = crate::schema::ir::CompiledSequence { members: alloc::vec![odd_id] };
        let tbs_id = b_tbs.add_term_with_props(crate::types::QName::local("tbs"), TermKind::Sequence(tbs_seq), tbs_props).unwrap();
        b_tbs.set_root(tbs_id);
        let schema_tbs = b_tbs.build().unwrap();

        let mut doc_tbs = crate::infoset::InfosetDocument::new();
        let mut root_elem_tbs = crate::infoset::InfosetElement::complex(crate::types::QName::local("root"));
        let odd_child = crate::infoset::InfosetElement::simple(
            crate::types::QName::local("odd_byte"),
            ElementState::Value(DfdlValue::Byte(42)),
        );
        root_elem_tbs.children.push(crate::infoset::tree::InfosetNode::Element(odd_child));
        doc_tbs.root = Some(root_elem_tbs.clone());

        let mut w_tbs = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_tbs_work = WorkBudget::new(1000);
        let mut u_tbs = UnparserEngine::new(&schema_tbs, &mut w_tbs, &mut b_tbs_work);
        u_tbs.doc = Some(&doc_tbs);
        u_tbs.variable_map_mut().define_variable(
            crate::types::QName::local("requireLengthInWholeWords"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("yes".into())),
        );
        let err_tbs = u_tbs.unparse_term(tbs_id, &root_elem_tbs).unwrap_err();
        assert!(err_tbs.message.as_str().contains("not a multiple of 2 for twoByteSwap layer"));
    }

    #[test]
    fn test_unparser_validation_and_scalar_edge_cases() {
        use crate::infoset::DfdlSimpleType;
        let default_props = ResolvedProperties::default();

        // 1. validate_unparse_value negative when decimalSigned is no (lines 3388-3403)
        let mut props_unsigned = ResolvedProperties {
            decimal_signed: false,
            ..Default::default()
        };
        assert!(validate_unparse_value(&DfdlValue::Int(-5), &DfdlSimpleType::Int, &props_unsigned, None).is_err());
        props_unsigned.binary_number_rep = crate::schema::ir::BinaryNumberRep::Packed;
        assert!(validate_unparse_value(&DfdlValue::Int(-5), &DfdlSimpleType::Int, &props_unsigned, None).is_err());

        // 2. validate_unparse_value negative when textStandardBase != 10 (lines 3405-3417)
        let props_base16 = ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            text_standard_base: 16,
            ..Default::default()
        };
        assert!(validate_unparse_value(&DfdlValue::Int(-5), &DfdlSimpleType::Int, &props_base16, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::Int(15), &DfdlSimpleType::Int, &props_base16, None).is_ok());

        // 3. validate_unparse_value integer type bounds checks (lines 3441-3580)
        // Long
        let val_huge = DfdlValue::String("9999999999999999999999999999".into());
        assert!(validate_unparse_value(&val_huge, &DfdlSimpleType::Long, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::Long(42), &DfdlSimpleType::Long, &default_props, None).is_ok());

        // Short
        let val_bad_short = DfdlValue::String("999999".into());
        assert!(validate_unparse_value(&val_bad_short, &DfdlSimpleType::Short, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::Short(42), &DfdlSimpleType::Short, &default_props, None).is_ok());

        // Byte
        let val_bad_byte = DfdlValue::String("999".into());
        assert!(validate_unparse_value(&val_bad_byte, &DfdlSimpleType::Byte, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::Byte(42), &DfdlSimpleType::Byte, &default_props, None).is_ok());

        // UnsignedLong
        let val_neg = DfdlValue::String("-1".into());
        assert!(validate_unparse_value(&val_neg, &DfdlSimpleType::UnsignedLong, &default_props, None).is_err());
        assert!(validate_unparse_value(&val_huge, &DfdlSimpleType::UnsignedLong, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::UnsignedLong(42), &DfdlSimpleType::UnsignedLong, &default_props, None).is_ok());

        // UnsignedInt
        assert!(validate_unparse_value(&val_neg, &DfdlSimpleType::UnsignedInt, &default_props, None).is_err());
        assert!(validate_unparse_value(&val_huge, &DfdlSimpleType::UnsignedInt, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::UnsignedInt(42), &DfdlSimpleType::UnsignedInt, &default_props, None).is_ok());

        // UnsignedShort
        assert!(validate_unparse_value(&val_neg, &DfdlSimpleType::UnsignedShort, &default_props, None).is_err());
        assert!(validate_unparse_value(&val_bad_short, &DfdlSimpleType::UnsignedShort, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::UnsignedShort(42), &DfdlSimpleType::UnsignedShort, &default_props, None).is_ok());

        // UnsignedByte
        assert!(validate_unparse_value(&val_neg, &DfdlSimpleType::UnsignedByte, &default_props, None).is_err());
        assert!(validate_unparse_value(&val_bad_byte, &DfdlSimpleType::UnsignedByte, &default_props, None).is_err());
        assert!(validate_unparse_value(&DfdlValue::UnsignedByte(42), &DfdlSimpleType::UnsignedByte, &default_props, None).is_ok());

        // 4. unparse_binary_value String branches (hex, integer, raw) (lines 2638-2679)
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder.add_term_with_props(crate::types::QName::local("root"), TermKind::Element(elem), ResolvedProperties::default()).unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut budget = WorkBudget::new(100);

        // Hex string unparsing
        let mut w_hex = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_hex = UnparserEngine::new(&schema, &mut w_hex, &mut budget);
            let mut hex_props = default_props.clone();
            hex_props.representation = Representation::Binary;
            u_hex.unparse_binary_value(&DfdlValue::String("ABCD".into()), &hex_props).unwrap();
        }
        assert_eq!(w_hex.into_sink().into_vec(), alloc::vec![0xAB, 0xCD]);

        // Integer string binary unparsing
        let mut w_int = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_int = UnparserEngine::new(&schema, &mut w_int, &mut budget);
            let mut int_props = default_props.clone();
            int_props.representation = Representation::Binary;
            int_props.length = Some(32);
            int_props.length_units = crate::schema::ir::LengthUnits::Bits;
            u_int.unparse_binary_value(&DfdlValue::String("305419896".into()), &int_props).unwrap();
        }
        assert_eq!(w_int.into_sink().into_vec(), alloc::vec![0x12, 0x34, 0x56, 0x78]);

        // Raw string binary unparsing
        let mut w_raw = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_raw = UnparserEngine::new(&schema, &mut w_raw, &mut budget);
            let mut raw_props = default_props.clone();
            raw_props.representation = Representation::Binary;
            u_raw.unparse_binary_value(&DfdlValue::String("xyz".into()), &raw_props).unwrap();
        }
        assert_eq!(w_raw.into_sink().into_vec(), b"xyz");

        // 5. Delimited text padding with text_output_min_length (lines 2970-3012)
        let props_pad_r = ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_pad_char: "#".into(),
            text_output_min_length: 5,
            text_string_justification: crate::schema::ir::TextJustification::Right,
            ..Default::default()
        };
        let mut w_pr = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_pr = UnparserEngine::new(&schema, &mut w_pr, &mut budget);
            u_pr.unparse_text_value(&DfdlValue::String("hi".into()), &props_pad_r).unwrap();
        }
        assert_eq!(w_pr.into_sink().into_vec(), b"###hi");

        let mut props_pad_c = props_pad_r.clone();
        props_pad_c.text_string_justification = crate::schema::ir::TextJustification::Center;
        let mut w_pc = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_pc = UnparserEngine::new(&schema, &mut w_pc, &mut budget);
            u_pc.unparse_text_value(&DfdlValue::String("hi".into()), &props_pad_c).unwrap();
        }
        assert_eq!(w_pc.into_sink().into_vec(), b"##hi#");

        let mut props_pad_l = props_pad_r.clone();
        props_pad_l.text_string_justification = crate::schema::ir::TextJustification::Left;
        props_pad_l.text_trim_kind = crate::schema::ir::TextTrimKind::Head;
        let mut w_pl = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        {
            let mut u_pl = UnparserEngine::new(&schema, &mut w_pl, &mut budget);
            u_pl.unparse_text_value(&DfdlValue::String("hi".into()), &props_pad_l).unwrap();
        }
        assert_eq!(w_pl.into_sink().into_vec(), b"###hi");
    }

    #[test]
    fn test_unparse_document_sequence_delimiters_and_suppression() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let e1 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("f1"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let e2 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("f2"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 0,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let e3 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("f3"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 0,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let p_txt = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let id1 = builder.add_term_with_props(e1.name.clone(), TermKind::Element(e1), p_txt.clone()).unwrap();
        let id2 = builder.add_term_with_props(e2.name.clone(), TermKind::Element(e2), p_txt.clone()).unwrap();
        let id3 = builder.add_term_with_props(e3.name.clone(), TermKind::Element(e3), p_txt.clone()).unwrap();

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![id1, id2, id3],
        };
        let p_seq = ResolvedProperties {
            initiator: Some("[".into()),
            separator: Some(",".into()),
            separator_position: crate::schema::ir::SeparatorPosition::Infix,
            terminator: Some("]".into()),
            separator_suppression_policy: crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty,
            ..Default::default()
        };
        let seq_id = builder.add_term_with_props(crate::types::QName::local("seq"), TermKind::Sequence(seq), p_seq).unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder.add_term_with_props(root_elem.name.clone(), TermKind::Element(root_elem), ResolvedProperties::default()).unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. All elements present
        let mut root_val = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("root"));
        root_val.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("f1"), crate::infoset::state::ElementState::Value(DfdlValue::String("hello".into())))
        )).unwrap();
        root_val.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("f2"), crate::infoset::state::ElementState::Value(DfdlValue::Int(42)))
        )).unwrap();
        root_val.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("f3"), crate::infoset::state::ElementState::Value(DfdlValue::String("world".into())))
        )).unwrap();
        let doc1 = crate::infoset::InfosetDocument::with_root(root_val);

        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(10_000);
        let mut unparser1 = UnparserEngine::new(&schema, &mut writer1, &mut budget);
        unparser1.unparse_document(&doc1).unwrap();
        writer1.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer1.into_sink().as_slice()).unwrap(), "[hello,42,world]");

        // 2. Trailing elements omitted -> suppressed by TrailingEmpty
        let mut root_val2 = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("root"));
        root_val2.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("f1"), crate::infoset::state::ElementState::Value(DfdlValue::String("hello".into())))
        )).unwrap();
        let doc2 = crate::infoset::InfosetDocument::with_root(root_val2);

        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser2 = UnparserEngine::new(&schema, &mut writer2, &mut budget);
        unparser2.unparse_document(&doc2).unwrap();
        writer2.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer2.into_sink().as_slice()).unwrap(), "[hello]");
    }

    #[test]
    fn test_unparse_document_layers_and_nils() {
        crate::kernel::layer::init_test_layers();
        // 1. TwoByteSwap layer
        let mut b1 = crate::schema::builder::SchemaBuilder::new();
        let el = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("body"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let p_el = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            ..Default::default()
        };
        let id_el = b1.add_term_with_props(el.name.clone(), TermKind::Element(el), p_el).unwrap();
        let seq = crate::schema::ir::CompiledSequence { members: alloc::vec![id_el] };
        let p_seq = ResolvedProperties {
            layer: Some("twoByteSwap".into()),
            ..Default::default()
        };
        let id_seq = b1.add_term_with_props(crate::types::QName::local("seq"), TermKind::Sequence(seq), p_seq).unwrap();
        let root = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(id_seq),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = b1.add_term_with_props(root.name.clone(), TermKind::Element(root), ResolvedProperties::default()).unwrap();
        b1.set_root(root_id);
        let schema1 = b1.build().unwrap();

        let mut root_elem = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("root"));
        root_elem.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("body"), crate::infoset::state::ElementState::Value(DfdlValue::String("ABCD".into())))
        )).unwrap();
        let doc1 = crate::infoset::InfosetDocument::with_root(root_elem);

        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(10_000);
        let mut unparser1 = UnparserEngine::new(&schema1, &mut writer1, &mut budget);
        unparser1.unparse_document(&doc1).unwrap();
        writer1.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer1.into_sink().as_slice()).unwrap(), "BADC");

        // 2. Nillable element with literalValue
        let mut b2 = crate::schema::builder::SchemaBuilder::new();
        let nil_el = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("item"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: true,
            default_value: None,
        };
        let mut p_nil = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            nil_kind: crate::schema::ir::NilKind::LiteralValue,
            ..Default::default()
        };
        p_nil.nil_value = Some("NIL".into());
        let id_nil = b2.add_term_with_props(nil_el.name.clone(), TermKind::Element(nil_el), p_nil).unwrap();
        b2.set_root(id_nil);
        let schema2 = b2.build().unwrap();

        let mut nil_item = crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("item"), crate::infoset::state::ElementState::Nil);
        nil_item.nil_attribute = Some(true);
        let doc2 = crate::infoset::InfosetDocument::with_root(nil_item);

        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser2 = UnparserEngine::new(&schema2, &mut writer2, &mut budget);
        unparser2.unparse_document(&doc2).unwrap();
        writer2.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer2.into_sink().as_slice()).unwrap(), "NIL");
    }

    #[test]
    fn test_unparse_document_choice_branches() {
        let mut b = crate::schema::builder::SchemaBuilder::new();
        let b1 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("optA"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let b2 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("optB"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let p = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let id1 = b.add_term_with_props(b1.name.clone(), TermKind::Element(b1), p.clone()).unwrap();
        let id2 = b.add_term_with_props(b2.name.clone(), TermKind::Element(b2), p).unwrap();

        let choice = crate::schema::ir::CompiledChoice {
            branches: alloc::vec![id1, id2],
        };
        let ch_id = b.add_term_with_props(crate::types::QName::local("ch"), TermKind::Choice(choice), ResolvedProperties::default()).unwrap();
        let root = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(ch_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = b.add_term_with_props(root.name.clone(), TermKind::Element(root), ResolvedProperties::default()).unwrap();
        b.set_root(root_id);
        let schema = b.build().unwrap();

        // Select branch optB
        let mut root_elem = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("root"));
        root_elem.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("optB"), crate::infoset::state::ElementState::Value(DfdlValue::Int(99)))
        )).unwrap();
        let doc = crate::infoset::InfosetDocument::with_root(root_elem);

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(10_000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser.unparse_document(&doc).unwrap();
        writer.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer.into_sink().as_slice()).unwrap(), "99");
    }
