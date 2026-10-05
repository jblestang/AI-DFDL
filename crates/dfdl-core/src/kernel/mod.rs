//! Bidirectional DFDL execution kernel (parser and unparser engines).
//!
//! Aligned with DFDL 1.0 Specification §9, §10, §11, §12. Operates strictly under `#![no_std]` + `alloc`.

pub mod layer;
pub mod parser;
pub mod unparser;

pub use parser::{ParserEngine, ValidationMode};
pub use unparser::UnparserEngine;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infoset::{DfdlSimpleType, DfdlValue};
    use crate::io::bitstream::{BitReader, BitWriter};
    use crate::io::sink::VecByteSink;
    use crate::io::source::SliceByteSource;
    use crate::io::traits::{BitOrder, ByteOrder};
    use crate::limits::WorkBudget;
    use crate::schema::builder::SchemaBuilder;
    use crate::schema::ir::{CompiledElement, CompiledType, TermKind};
    use crate::types::QName;

    #[test]
    fn test_kernel_parser_and_unparser_binary_scalar() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("HeaderVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            byte_order: ByteOrder::BigEndian,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            alignment: 1,
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("HeaderVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Unparse DfdlValue::Int(0x12345678) to bitstream
        let doc = crate::infoset::tree::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                QName::local("HeaderVal"),
                crate::infoset::state::ElementState::Value(DfdlValue::Int(0x1234_5678)),
            ),
        );

        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser.unparse_document(&doc).unwrap();

        let bytes = writer.sink().as_slice();
        assert_eq!(bytes, &[0x12, 0x34, 0x56, 0x78]);

        // 2. Parse bitstream back to InfosetDocument
        let src = SliceByteSource::new(bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut parse_budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut parse_budget);
        let parsed_doc = parser.parse_document().unwrap();

        let root = parsed_doc.root.unwrap();
        assert_eq!(root.name.local_name, "HeaderVal");
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(0x1234_5678))
        );
    }

    #[test]
    fn test_kernel_occurs_count_fixed_array() {
        use crate::schema::ir::{CompiledSequence, OccursCountKind};

        let mut builder = SchemaBuilder::new();
        let item_elem = CompiledElement {
            name: QName::local("Item"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 3,
            max_occurs: Some(3),
            is_nillable: false,
            default_value: None,
        };

        let item_props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            occurs_count_kind: OccursCountKind::Fixed,
            ..Default::default()
        };

        let item_id = builder
            .add_term_with_props(
                QName::local("Item"),
                TermKind::Element(item_elem),
                item_props,
            )
            .unwrap();

        let seq_term = TermKind::Sequence(CompiledSequence {
            members: alloc::vec![item_id],
        });
        let seq_props = crate::schema::ir::ResolvedProperties {
            separator: Some(alloc::string::String::from(",")),
            separator_position: crate::schema::ir::SeparatorPosition::Infix,
            ..Default::default()
        };
        let seq_id = builder
            .add_term_with_props(QName::local("sequence"), seq_term, seq_props)
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("ArrayRecord"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let root_id = builder
            .add_term_with_props(
                QName::local("ArrayRecord"),
                TermKind::Element(root_elem),
                Default::default(),
            )
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parse text stream "10,20,30"
        let data = b"10,20,30";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 3);

        // 2. Unparse back to stream
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), data);
    }

    #[test]
    fn test_kernel_choice_branch_resolution() {
        use crate::schema::ir::CompiledChoice;

        let mut builder = SchemaBuilder::new();
        let branch_a = CompiledElement {
            name: QName::local("BranchA"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_a = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            initiator: Some(alloc::string::String::from("A:")),
            ..Default::default()
        };
        let id_a = builder
            .add_term_with_props(
                QName::local("BranchA"),
                TermKind::Element(branch_a),
                props_a,
            )
            .unwrap();

        let branch_b = CompiledElement {
            name: QName::local("BranchB"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_b = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            initiator: Some(alloc::string::String::from("B:")),
            ..Default::default()
        };
        let id_b = builder
            .add_term_with_props(
                QName::local("BranchB"),
                TermKind::Element(branch_b),
                props_b,
            )
            .unwrap();

        let choice_term = TermKind::Choice(CompiledChoice {
            branches: alloc::vec![id_a, id_b],
        });
        let choice_id = builder
            .add_term_with_props(QName::local("choice"), choice_term, Default::default())
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("ChoiceRecord"),
            type_ir: CompiledType::Complex(choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                QName::local("ChoiceRecord"),
                TermKind::Element(root_elem),
                Default::default(),
            )
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Stream starts with "B:42" -> Branch A fails ("A:"), rolls back and Branch B succeeds!
        let data = b"B:42";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        let crate::infoset::tree::InfosetNode::Element(ref child) = root.children.first().unwrap();
        assert_eq!(child.name.local_name, "BranchB");
        assert_eq!(
            child.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 2. Unparse back to stream "B:42"
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), data);
    }

    #[test]
    fn test_kernel_choice_discriminator_evaluation() {
        use crate::schema::ir::CompiledChoice;

        let mut builder = SchemaBuilder::new();
        let branch_a = CompiledElement {
            name: QName::local("OptA"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        // Branch A has discriminator that fails ({ false })
        let props_a = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            discriminator: Some(alloc::string::String::from("{ false }")),
            ..Default::default()
        };
        let id_a = builder
            .add_term_with_props(QName::local("OptA"), TermKind::Element(branch_a), props_a)
            .unwrap();

        let branch_b = CompiledElement {
            name: QName::local("OptB"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        // Branch B has discriminator that succeeds ({ true })
        let props_b = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            discriminator: Some(alloc::string::String::from("{ true }")),
            ..Default::default()
        };
        let id_b = builder
            .add_term_with_props(QName::local("OptB"), TermKind::Element(branch_b), props_b)
            .unwrap();

        let choice_term = TermKind::Choice(CompiledChoice {
            branches: alloc::vec![id_a, id_b],
        });
        let choice_id = builder
            .add_term_with_props(QName::local("choice"), choice_term, Default::default())
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("DiscRecord"),
            type_ir: CompiledType::Complex(choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                QName::local("DiscRecord"),
                TermKind::Element(root_elem),
                Default::default(),
            )
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Data "99" -> OptA parses "99" but discriminator { false } fails branch! Rollback -> OptB parses "99" and discriminator { true } succeeds!
        let data = b"99";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        let crate::infoset::tree::InfosetNode::Element(ref child) = root.children.first().unwrap();
        assert_eq!(child.name.local_name, "OptB");
    }

    #[test]
    fn test_kernel_nillable_element_literal_nil() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("NullableVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: true,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(3),
            nil_kind: crate::schema::ir::NilKind::LiteralValue,
            nil_value: Some(alloc::string::String::from("NIL")),
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("NullableVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parse bitstream "NIL"
        let data = b"NIL";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "NullableVal");
        assert_eq!(root.state, crate::infoset::state::ElementState::Nil);

        // 2. Unparse back to bitstream "NIL"
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), data);
    }

    #[test]
    fn test_kernel_nillable_element_non_nil_value() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("NullableVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: true,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(3),
            nil_kind: crate::schema::ir::NilKind::LiteralValue,
            nil_value: Some(alloc::string::String::from("NIL")),
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("NullableVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parse bitstream "123" (non-nil)
        let data = b"123";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "NullableVal");
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(123))
        );

        // 2. Unparse back to bitstream "123"
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), data);
    }

    #[test]
    fn test_kernel_nillable_unparse_missing_nil_value_error() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("NullableVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: true,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            nil_kind: crate::schema::ir::NilKind::LiteralValue,
            nil_value: None, // No nil value specified!
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("NullableVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let doc = crate::infoset::tree::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                QName::local("NullableVal"),
                crate::infoset::state::ElementState::Nil,
            ),
        );

        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        let res = unparser.unparse_document(&doc);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, crate::error::DFDLErrorKind::Unparse);
    }

    #[test]
    fn test_kernel_text_trimming_both_pad_char() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("PaddedVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(6),
            text_trim_kind: crate::schema::ir::TextTrimKind::Both,
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_pad_char: alloc::string::String::from(" "),
            text_number_justification: crate::schema::ir::TextJustification::Left,
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("PaddedVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parse text stream "  42  " -> trimmed to "42" -> Int(42)
        let data = b"  42  ";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "PaddedVal");
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 2. Unparse back to stream -> padded to length 6 "42    "
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), b"42    ");
    }

    #[test]
    fn test_kernel_text_trimming_head_and_tail() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("HeadTailVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(5),
            text_trim_kind: crate::schema::ir::TextTrimKind::Head,
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_pad_char: alloc::string::String::from("0"),
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("HeadTailVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Parse text stream "00123" -> trimmed head '0's -> String("123")
        let data = b"00123";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::String(
                alloc::string::String::from("123")
            ))
        );

        // Unparse back -> padded head '0's to length 5 "00123"
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), data);
    }

    #[test]
    fn test_kernel_input_value_calc() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("CalculatedVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            input_value_calc: Some(alloc::string::String::from("{ 10 + 32 }")),
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(
                QName::local("CalculatedVal"),
                TermKind::Element(elem),
                props,
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Parse stream with 0 bytes (no data read from bitstream)
        let data = b"";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "CalculatedVal");
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );
    }

    #[test]
    fn test_kernel_output_value_calc() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("CalculatedVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            output_value_calc: Some(alloc::string::String::from("{ 50 + 50 }")),
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(
                QName::local("CalculatedVal"),
                TermKind::Element(elem),
                props,
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let doc = crate::infoset::tree::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                QName::local("CalculatedVal"),
                crate::infoset::state::ElementState::Value(DfdlValue::Int(0)),
            ),
        );

        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), b"100");
    }

    #[test]
    fn test_kernel_assert_success() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("AssertVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            asserts: alloc::vec![crate::schema::ir::CompiledAssert {
                test_kind: crate::schema::ir::TestKind::Expression,
                test_expr: alloc::string::String::from("{ 10 gt 5 }"),
                message: Some(alloc::string::String::from("Value must be > 5")),
                failure_type: crate::schema::ir::FailureType::ProcessingError,
            }],
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("AssertVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let data = b"42";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(
            root.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );
    }

    #[test]
    fn test_kernel_assert_failure_validation_error() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("AssertVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            asserts: alloc::vec![crate::schema::ir::CompiledAssert {
                test_kind: crate::schema::ir::TestKind::Expression,
                test_expr: alloc::string::String::from("{ 10 lt 5 }"), // Fails!
                message: Some(alloc::string::String::from("Value must be > 5")),
                failure_type: crate::schema::ir::FailureType::ProcessingError,
            }],
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(QName::local("AssertVal"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let data = b"42";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.kind, crate::error::DFDLErrorKind::Validation);
        assert_eq!(err.message.as_str(), "Value must be > 5");
    }

    #[test]
    fn test_kernel_assert_multiple_rules_evaluation() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("MultiAssertVal"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            asserts: alloc::vec![
                crate::schema::ir::CompiledAssert {
                    test_kind: crate::schema::ir::TestKind::Expression,
                    test_expr: alloc::string::String::from("{ 10 gt 5 }"),
                    message: Some(alloc::string::String::from("Rule 1 ok")),
                    failure_type: crate::schema::ir::FailureType::ProcessingError,
                },
                crate::schema::ir::CompiledAssert {
                    test_kind: crate::schema::ir::TestKind::Expression,
                    test_expr: alloc::string::String::from("{ 5 gt 10 }"),
                    message: Some(alloc::string::String::from("Rule 2 failed")),
                    failure_type: crate::schema::ir::FailureType::ProcessingError,
                },
            ],
            ..Default::default()
        };

        let root_id = builder
            .add_term_with_props(
                QName::local("MultiAssertVal"),
                TermKind::Element(elem),
                props,
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let data = b"42";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.kind, crate::error::DFDLErrorKind::Validation);
        assert_eq!(err.message.as_str(), "Rule 2 failed");
    }

    #[test]
    fn test_kernel_hidden_group_element_suppression_and_unparsing() {
        use crate::io::sink::VecByteSink;
        use crate::kernel::UnparserEngine;
        use crate::schema::ir::{
            CompiledElement, CompiledSequence, CompiledType, LengthKind, Representation,
            ResolvedProperties, TermKind,
        };

        let mut builder = SchemaBuilder::new();

        let header_elem = CompiledElement {
            name: QName::local("HeaderSecret"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let header_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(2),
            is_hidden: true,
            output_value_calc: Some(alloc::string::String::from("{ 99 }")),
            ..Default::default()
        };
        let header_id = builder
            .add_term_with_props(
                QName::local("HeaderSecret"),
                TermKind::Element(header_elem),
                header_props,
            )
            .unwrap();

        let body_elem = CompiledElement {
            name: QName::local("BodyPayload"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let body_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(2),
            is_hidden: false,
            ..Default::default()
        };
        let body_id = builder
            .add_term_with_props(
                QName::local("BodyPayload"),
                TermKind::Element(body_elem),
                body_props,
            )
            .unwrap();

        let seq = CompiledSequence {
            members: alloc::vec![header_id, body_id],
        };
        let seq_id = builder
            .add_term(QName::local("Seq"), TermKind::Sequence(seq))
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("Record"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("Record"), TermKind::Element(root_elem))
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parse bitstream "9942"
        let data = b"9942";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let full_doc = parser.parse_document().unwrap();

        // Internal tree contains both HeaderSecret and BodyPayload
        let root = full_doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 2);

        // 2. Strip hidden elements -> public doc contains ONLY BodyPayload
        let public_doc = full_doc.strip_hidden();
        let pub_root = public_doc.root.as_ref().unwrap();
        assert_eq!(pub_root.children.len(), 1);
        let crate::infoset::tree::InfosetNode::Element(ref body_node) =
            pub_root.children.first().unwrap();
        assert_eq!(body_node.name.local_name, "BodyPayload");
        assert_eq!(
            body_node.state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 3. Unparse public_doc (missing HeaderSecret) -> output_value_calc for HeaderSecret emits "99" + "42" = b"9942"
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&public_doc).unwrap();

        let output_bytes = writer.into_sink().into_vec();
        assert_eq!(&output_bytes, b"9942");
    }

    #[test]
    fn test_kernel_soft_eof_binary_read_fails_gracefully() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("BinaryInt"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            byte_order: ByteOrder::BigEndian,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(QName::local("BinaryInt"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1-byte input stream (requires 4 bytes for binary Int)
        let short_bytes = b"\x01";
        let src = SliceByteSource::new(short_bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        let err = res.err().unwrap();
        assert_eq!(err.kind, crate::error::DFDLErrorKind::Parse);
        assert!(alloc::format!("{:?}", err.message).contains("Insufficient binary data"));
    }

    #[test]
    fn test_dynamic_non_byte_sized_encoding_rejected_in_unparser() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("Field"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            encoding: alloc::string::ToString::to_string("{ 'X-DFDL-US-ASCII-6-BIT-PACKED' }"),
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(QName::local("Field"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let doc = crate::infoset::tree::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                QName::local("Field"),
                crate::infoset::state::ElementState::Value(DfdlValue::String(alloc::string::ToString::to_string("AB"))),
            ),
        );

        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(sink, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let res = unparser.unparse_document(&doc);
        assert!(res.is_err());
        let err_msg = alloc::format!("{}", res.unwrap_err());
        assert!(
            err_msg.contains("Only encodings with byte-sized code units can be computed via expressions"),
            "Expected dynamic encoding error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_runtime_binary_integer_bit_length_bounds_rejected() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("Num"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length_units: crate::schema::ir::LengthUnits::Bits,
            length_expr: Some(alloc::string::ToString::to_string("64")),
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(QName::local("Num"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parser fails when dynamic length exceeds 32 bits for xs:int
        let src = SliceByteSource::new(&[0xFF; 8]);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res_parse = parser.parse_document();
        assert!(res_parse.is_err());
        let err_msg = alloc::format!("{}", res_parse.unwrap_err());
        assert!(
            err_msg.contains("Length in bits 64 out of range") && err_msg.contains("between 1 and 32"),
            "Expected runtime bit length bounds error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_non_base_10_leading_sign_and_unparse() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("Base16Num"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            text_standard_base: 16,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(2),
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(QName::local("Base16Num"), TermKind::Element(elem), props.clone())
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Parsing leading sign with base 16 is rejected
        let src = SliceByteSource::new(b"-0");
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        assert!(alloc::format!("{}", res.unwrap_err()).contains("Non-base 10 representation cannot contain leading sign"));

        // 2. Unparsing negative number with base 16 is rejected
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(sink, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let val = DfdlValue::Int(-1);
        let res_unparse = unparser.unparse_text_value(&val, &props);
        assert!(res_unparse.is_err());
        assert!(alloc::format!("{}", res_unparse.unwrap_err()).contains("Unable to unparse negative value"));
    }

    #[test]
    fn test_unaligned_charset_mandatory_alignment() {
        let mut builder = SchemaBuilder::new();
        let elem1 = CompiledElement {
            name: QName::local("fourBits"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let elem1_props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..Default::default()
        };
        let elem1_id = builder
            .add_term_with_props(QName::local("fourBits"), TermKind::Element(elem1), elem1_props)
            .unwrap();

        let elem2 = CompiledElement {
            name: QName::local("str"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        // Manual alignment: automatic alignment disabled, so string starts at bit 4 (unaligned for byte charset)
        let elem2_props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            encoding: alloc::string::String::from("US-ASCII"),
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(3),
            length_units: crate::schema::ir::LengthUnits::Bytes,
            alignment: 8,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            alignment_kind: crate::schema::ir::AlignmentKind::Manual,
            ..Default::default()
        };
        let elem2_id = builder
            .add_term_with_props(QName::local("str"), TermKind::Element(elem2), elem2_props)
            .unwrap();

        let seq_term = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![elem1_id, elem2_id],
        });
        let seq_id = builder
            .add_term_with_props(QName::local("seq"), seq_term, Default::default())
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("root"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(QName::local("root"), TermKind::Element(root_elem), Default::default())
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let src = SliceByteSource::new(&[0xF3, 0x34, 0x35, 0x36]);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        let err_msg = alloc::format!("{}", res.unwrap_err());
        assert!(err_msg.contains("charset not byte aligned"), "got: {}", err_msg);
        assert!(err_msg.contains("5"), "got: {}", err_msg);
    }

    #[test]
    fn test_separator_suppression_never_requires_all_max_occurs() {
        let mut builder = SchemaBuilder::new();
        let item_elem = CompiledElement {
            name: QName::local("Item"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 0,
            max_occurs: Some(3),
            is_nillable: false,
            default_value: None,
        };
        let item_props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Delimited,
            occurs_count_kind: crate::schema::ir::OccursCountKind::Implicit,
            ..Default::default()
        };
        let item_id = builder
            .add_term_with_props(QName::local("Item"), TermKind::Element(item_elem), item_props)
            .unwrap();

        let seq_term = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![item_id],
        });
        let seq_props = crate::schema::ir::ResolvedProperties {
            separator: Some(alloc::string::String::from(",")),
            separator_position: crate::schema::ir::SeparatorPosition::Infix,
            separator_suppression_policy: crate::schema::ir::SeparatorSuppressionPolicy::Never,
            ..Default::default()
        };
        let seq_id = builder
            .add_term_with_props(QName::local("seq"), seq_term, seq_props)
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("root"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(QName::local("root"), TermKind::Element(root_elem), Default::default())
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Only 2 elements provided with separator "a,b": fails because maxOccurs (3) required with 'never'
        let src1 = SliceByteSource::new(b"a,b");
        let mut reader1 = BitReader::new(src1, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget1 = WorkBudget::new(100);
        let mut parser1 = ParserEngine::new(&schema, &mut reader1, &mut budget1);
        let res1 = parser1.parse_document();
        assert!(res1.is_err());
        let err1 = alloc::format!("{}", res1.unwrap_err());
        assert!(err1.contains("separatorSuppressionPolicy is 'never'"));
        assert!(err1.contains("maxOccurs (3)"));

        // 2. All 3 elements provided "a,b,c": succeeds
        let src2 = SliceByteSource::new(b"a,b,c");
        let mut reader2 = BitReader::new(src2, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget2 = WorkBudget::new(100);
        let mut parser2 = ParserEngine::new(&schema, &mut reader2, &mut budget2);
        let res2 = parser2.parse_document();
        assert!(res2.is_ok());
    }

    #[test]
    fn test_bit_reader_limit_and_leftover_bits() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("e1"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let elem_props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(1),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(QName::local("e1"), TermKind::Element(elem), elem_props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Document has 2 significant bits (0xC0 has 2 bits 11 padded with zeros).
        // Parser consumes 1 bit. 1 bit remains -> Left over data error.
        let src = SliceByteSource::new(&[0xC0]);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        reader.set_bit_limit(Some(2));
        let mut budget = WorkBudget::new(100);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
        let err = alloc::format!("{}", res.unwrap_err());
        assert!(err.contains("Left over data remaining"), "got: {}", err);

        // Document has 1 significant bit. Parser consumes 1 bit -> success.
        let src2 = SliceByteSource::new(&[0x80]);
        let mut reader2 = BitReader::new(src2, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        reader2.set_bit_limit(Some(1));
        let mut budget2 = WorkBudget::new(100);
        let mut parser2 = ParserEngine::new(&schema, &mut reader2, &mut budget2);
        assert!(parser2.parse_document().is_ok());
    }
}
