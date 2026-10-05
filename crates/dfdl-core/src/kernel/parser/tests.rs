#![allow(clippy::unwrap_used, clippy::expect_used, clippy::field_reassign_with_default)]

    use super::*;
    use super::binary::*;
    use super::delimiters::*;
    use super::element::*;
    use super::numbers::*;
    use crate::infoset::{DfdlSimpleType, DfdlValue};
    use crate::io::source::SliceByteSource;
    use crate::io::traits::{BitOrder, ByteOrder};
    use crate::schema::builder::SchemaBuilder;
    use crate::schema::ir::{CompiledElement, Representation, ResolvedProperties};
    use alloc::string::ToString;
    use alloc::vec;

    fn dummy_schema() -> CompiledSchema {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let _ = builder
            .add_term(crate::types::QName::local("root"), TermKind::Element(elem))
            .unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn test_sign_extend_helper() {
        assert_eq!(sign_extend(0b1111, 4), -1);
        assert_eq!(sign_extend(0b0011, 4), 3);
        assert_eq!(sign_extend(0x80, 8), -128);
        assert_eq!(sign_extend(0x7F, 8), 127);
    }

    #[test]
    fn test_binary_value_little_endian() {
        let bytes = [0x78, 0x56, 0x34, 0x12];
        let src = SliceByteSource::new(&bytes);
        let mut reader = BitReader::new(
            src,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::LittleEndian,
        );
        let mut budget = WorkBudget::new(100);
        let schema = dummy_schema();
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut props = ResolvedProperties::default();
        props.byte_order = ByteOrder::LittleEndian;
        props.representation = Representation::Binary;

        let val = engine
            .parse_binary_value(DfdlSimpleType::Int, &props, None)
            .unwrap();
        assert_eq!(val, DfdlValue::Int(0x12345678));
    }

    #[test]
    fn test_unaligned_bitreader_rollback() {
        let bytes = [0xAA, 0xBB, 0xCC];
        let src = SliceByteSource::new(&bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        let _ = reader.read_bits(5).unwrap();
        let cp = reader.checkpoint();
        assert_eq!(cp.bit_position.0, 5);

        let val1 = reader.read_bits(8).unwrap();
        assert!(reader.rollback(cp).is_ok());
        let val2 = reader.read_bits(8).unwrap();
        assert_eq!(val1, val2);
    }

    #[test]
    fn test_text_value_bits_length_units() {
        let bytes = b"HELLO WORLD";
        let src = SliceByteSource::new(bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let schema = dummy_schema();
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut props = ResolvedProperties::default();
        props.representation = Representation::Text;
        props.length_kind = crate::schema::ir::LengthKind::Explicit;
        props.length_units = crate::schema::ir::LengthUnits::Bits;
        props.length = Some(40); // 40 bits = 5 bytes ("HELLO")

        let builder = InfosetBuilder::new();
        let dynamic_len = engine
            .evaluate_length_property(&props, None, &builder)
            .unwrap();
        let val = engine
            .parse_text_value(DfdlSimpleType::String, &props, dynamic_len, &builder, None)
            .unwrap();
        assert_eq!(val, DfdlValue::String(String::from("HELLO")));
    }

    #[test]
    fn test_binary_implicit_length_ignores_props_length() {
        let bytes = [0x7F, 0xAA, 0xBB, 0xCC];
        let src = SliceByteSource::new(&bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let schema = dummy_schema();
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut props = ResolvedProperties::default();
        props.representation = Representation::Binary;
        props.length_kind = crate::schema::ir::LengthKind::Implicit;
        props.length = Some(4);

        let dynamic_len = engine
            .evaluate_length_property(&props, None, &InfosetBuilder::new())
            .unwrap();
        assert_eq!(dynamic_len, None);

        let val = engine
            .parse_binary_value(DfdlSimpleType::Byte, &props, dynamic_len)
            .unwrap();
        assert_eq!(val, DfdlValue::Byte(127));
    }

    #[test]
    fn test_text_delimited_ignores_explicit_length() {
        let bytes = b"123,456";
        let src = SliceByteSource::new(bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let schema = dummy_schema();
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut props = ResolvedProperties::default();
        props.representation = Representation::Text;
        props.length_kind = crate::schema::ir::LengthKind::Delimited;
        props.separator = Some(alloc::string::String::from(","));
        props.length = Some(100);

        let builder = InfosetBuilder::new();
        let dynamic_len = engine
            .evaluate_length_property(&props, None, &builder)
            .unwrap();
        assert_eq!(dynamic_len, None);

        let val = engine
            .parse_text_value(DfdlSimpleType::Int, &props, dynamic_len, &builder, None)
            .unwrap();
        assert_eq!(val, DfdlValue::Int(123));
    }

    #[test]
    fn test_flexible_scalar_parsing_helpers() {
        assert_eq!(parse_flexible_int_i64("+123"), Some(123));
        assert_eq!(parse_flexible_int_i64("00123"), Some(123));
        assert_eq!(parse_flexible_int_i64("1,234"), Some(1234));
        assert_eq!(parse_flexible_int_i64("$50"), Some(50));
        assert_eq!(parse_flexible_int_i64("0x10"), Some(16));
        assert_eq!(parse_flexible_int_i64("42.00"), Some(42));
        assert_eq!(parse_flexible_uint_u64("+100"), Some(100));
        assert_eq!(parse_flexible_bool("YES"), Some(true));
        assert_eq!(parse_flexible_bool("0"), Some(false));

        let strict_pattern = Some("#,##0.00;(#,##0.00)");
        assert_eq!(
            parse_strict_int_i64("1,234.00", strict_pattern, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            Some(1234)
        );
        assert_eq!(
            parse_strict_int_i64("(1,234.00)", strict_pattern, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            Some(-1234)
        );
        assert_eq!(
            parse_strict_int_i64("-1,234.00", strict_pattern, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            None
        );
        assert_eq!(
            parse_strict_int_i64("12,34.00", strict_pattern, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            None
        );
        assert_eq!(parse_strict_int_i64("0x12", None, ".", ",", None, crate::schema::ir::TextTrimKind::None), None);
    }

    #[test]
    fn test_other_parse_errors_cluster_resolution() {
        assert!(parse_flexible_f64("NaN").unwrap().is_nan());
        assert_eq!(parse_flexible_f64("INF"), Some(f64::INFINITY));
        assert_eq!(parse_flexible_f64("+Infinity"), Some(f64::INFINITY));
        assert_eq!(parse_flexible_f64("-INF"), Some(f64::NEG_INFINITY));
        assert_eq!(parse_flexible_f64("-Infinity"), Some(f64::NEG_INFINITY));

        assert!(parse_strict_f64("NaN", None, ".", ",", None, None, None, false, None, crate::schema::ir::TextTrimKind::None).unwrap().is_nan());
        assert_eq!(
            parse_strict_f64("Infinity", None, ".", ",", None, None, None, false, None, crate::schema::ir::TextTrimKind::None),
            Some(f64::INFINITY)
        );
        assert_eq!(
            parse_strict_f64("-INF", None, ".", ",", None, None, None, false, None, crate::schema::ir::TextTrimKind::None),
            Some(f64::NEG_INFINITY)
        );
        assert_eq!(parse_strict_f64("1.234,56", None, ",", ".", None, None, None, false, None, crate::schema::ir::TextTrimKind::None), Some(1234.56));
    }

    #[test]
    fn test_expected_error_not_raised_cluster_resolution() {
        // 1. Invalid DFDL percent entity syntax raises SchemaDefinition error
        let err1 = parse_single_delim_tokens("%INVALID;");
        assert!(err1.is_err());
        assert_eq!(err1.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);

        // 2. Control characters in delimiter property raise SchemaDefinition error
        let mut props = crate::expr::properties::PropertyStore::new();
        props.set_property("initiator", "\x01BAD").unwrap();
        let res = props.validate_property_entities();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);

        // 3. Valid DFDL entity percent sequences parse correctly
        let tokens = parse_single_delim_tokens("%SP;").unwrap();
        assert_eq!(tokens, vec![DelimToken::SP]);
    }

    #[test]
    fn test_ivc_value_range_coercion_and_signedness() {
        let def_props = ResolvedProperties::default();
        let res_ok = coerce_and_validate_ivc_value(&DfdlValue::Long(42), &DfdlSimpleType::Int, &def_props);
        assert_eq!(res_ok.unwrap(), DfdlValue::Int(42));

        let res_err =
            coerce_and_validate_ivc_value(&DfdlValue::Long(-1), &DfdlSimpleType::UnsignedInt, &def_props);
        assert!(res_err.is_err());
        assert!(res_err
            .unwrap_err()
            .message
            .to_string()
            .contains("must match in signedness"));

        let res_overflow =
            coerce_and_validate_ivc_value(&DfdlValue::Long(32768), &DfdlSimpleType::Short, &def_props);
        assert!(res_overflow.is_err());
        assert!(res_overflow
            .unwrap_err()
            .message
            .to_string()
            .contains("out of range"));

        let res_byte_overflow =
            coerce_and_validate_ivc_value(&DfdlValue::Long(128), &DfdlSimpleType::Byte, &def_props);
        assert!(res_byte_overflow.is_err());

        let res_ubyte_neg = coerce_and_validate_ivc_value(
            &DfdlValue::String(alloc::string::String::from("-1")),
            &DfdlSimpleType::UnsignedByte,
            &def_props,
        );
        assert!(res_ubyte_neg.is_err());
        assert!(res_ubyte_neg
            .unwrap_err()
            .message
            .to_string()
            .contains("must match in signedness"));

        let res_ushort_overflow =
            coerce_and_validate_ivc_value(&DfdlValue::Long(65537), &DfdlSimpleType::UnsignedShort, &def_props);
        assert!(res_ushort_overflow.is_err());

        // Negative value for xs:nonNegativeInteger is rejected
        let mut non_neg_props = ResolvedProperties::default();
        non_neg_props.facets.min_inclusive = Some(alloc::string::String::from("0"));
        let res_non_neg = coerce_and_validate_ivc_value(&DfdlValue::Long(-30), &DfdlSimpleType::Decimal, &non_neg_props);
        assert!(res_non_neg.is_err());
        assert!(alloc::format!("{}", res_non_neg.unwrap_err()).contains("Cannot convert"));

        // Invalid date/time strings rejected
        let res_bad_date = coerce_and_validate_ivc_value(
            &DfdlValue::String(alloc::string::String::from("Wday, July 10, 1996")),
            &DfdlSimpleType::Date,
            &def_props,
        );
        assert!(res_bad_date.is_err());
        assert!(alloc::format!("{}", res_bad_date.unwrap_err()).contains("Failed to parse xs:date"));
    }

    #[test]
    fn test_trailing_empty_strict_rejection() {
        let mut builder = SchemaBuilder::new();
        let mut elem_props = ResolvedProperties::default();
        elem_props.representation = Representation::Text;
        elem_props.length_kind = crate::schema::ir::LengthKind::Delimited;

        let elem = CompiledElement {
            name: crate::types::QName::local("val"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(3),
            is_nillable: false,
            default_value: None,
        };
        let elem_id = builder
            .add_term_with_props(
                crate::types::QName::local("val"),
                TermKind::Element(elem),
                elem_props,
            )
            .unwrap();

        let mut seq_props = ResolvedProperties::default();
        seq_props.separator = Some(alloc::string::String::from("/"));
        seq_props.separator_position = crate::schema::ir::SeparatorPosition::Infix;
        seq_props.separator_suppression_policy =
            crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict;
        seq_props.representation = Representation::Text;

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![elem_id],
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("seq"),
                TermKind::Sequence(seq),
                seq_props,
            )
            .unwrap();

        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let bytes = b"a/b/";
        let src = SliceByteSource::new(bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut infoset_builder = InfosetBuilder::new();
        let res = engine.parse_term(root_id, &mut infoset_builder);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().message.to_string();
        assert!(err_msg.contains("trailingEmptyStrict"));
    }

    #[test]
    fn test_choice_direct_dispatch_and_errors() {
        let build_schema = |dispatch_key: &str| -> (CompiledSchema, NodeId) {
            let mut builder = SchemaBuilder::new();
            let mut inty_props = ResolvedProperties::default();
            inty_props.representation = Representation::Text;
            inty_props.length_kind = crate::schema::ir::LengthKind::Delimited;
            inty_props.choice_branch_key = Some(alloc::string::String::from("1"));
            let inty_elem = CompiledElement {
                name: crate::types::QName::local("inty"),
                type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Int),
                min_occurs: 1,
                max_occurs: Some(1),
                is_nillable: false,
                default_value: None,
            };
            let inty_id = builder
                .add_term_with_props(
                    crate::types::QName::local("inty"),
                    TermKind::Element(inty_elem),
                    inty_props,
                )
                .unwrap();

            let mut stringy_props = ResolvedProperties::default();
            stringy_props.representation = Representation::Text;
            stringy_props.length_kind = crate::schema::ir::LengthKind::Delimited;
            stringy_props.choice_branch_key = Some(alloc::string::String::from("2"));
            let stringy_elem = CompiledElement {
                name: crate::types::QName::local("stringy"),
                type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::String),
                min_occurs: 1,
                max_occurs: Some(1),
                is_nillable: false,
                default_value: None,
            };
            let stringy_id = builder
                .add_term_with_props(
                    crate::types::QName::local("stringy"),
                    TermKind::Element(stringy_elem),
                    stringy_props,
                )
                .unwrap();

            let mut choice_props = ResolvedProperties::default();
            choice_props.choice_dispatch_key = Some(alloc::string::String::from(dispatch_key));
            let choice = crate::schema::ir::CompiledChoice {
                branches: alloc::vec![inty_id, stringy_id],
            };
            let choice_id = builder
                .add_term_with_props(
                    crate::types::QName::local("choice"),
                    TermKind::Choice(choice),
                    choice_props,
                )
                .unwrap();

            builder.set_root(choice_id);
            (builder.build().unwrap(), choice_id)
        };

        // 1. Successful dispatch
        let (schema, choice_id) = build_schema("{ 1 }");
        let bytes = b"42";
        let src = SliceByteSource::new(bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);
        let mut infoset_builder = InfosetBuilder::new();
        let res = engine.parse_term(choice_id, &mut infoset_builder);
        assert!(res.is_ok());

        // 2. Choice dispatch key mismatch error
        let (schema2, choice_id2) = build_schema("{ 99 }");
        let src2 = SliceByteSource::new(bytes);
        let mut reader2 = BitReader::new(
            src2,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget2 = WorkBudget::new(100);
        let mut engine2 = ParserEngine::new(&schema2, &mut reader2, &mut budget2);
        let mut infoset_builder2 = InfosetBuilder::new();
        let err2 = engine2
            .parse_term(choice_id2, &mut infoset_builder2)
            .unwrap_err();
        assert!(err2.to_string().contains("failed to match"));

        // 3. Choice dispatch key empty string error
        let (schema3, choice_id3) = build_schema("{ '' }");
        let src3 = SliceByteSource::new(bytes);
        let mut reader3 = BitReader::new(
            src3,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget3 = WorkBudget::new(100);
        let mut engine3 = ParserEngine::new(&schema3, &mut reader3, &mut budget3);
        let mut infoset_builder3 = InfosetBuilder::new();
        let err3 = engine3
            .parse_term(choice_id3, &mut infoset_builder3)
            .unwrap_err();
        assert!(err3.to_string().contains("Non-empty string required"));
    }

    #[test]
    fn test_binary_calendar_milliseconds_and_bounds() {
        let mut props = ResolvedProperties::default();
        props.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds;
        props.binary_calendar_epoch = Some(alloc::string::String::from("2000-06-15T03:25:19"));
        props.byte_order = ByteOrder::BigEndian;

        // 252,438,276,689,639 ms = 9999-12-31T23:59:59.999000
        let bytes: [u8; 8] = [0x00, 0x00, 0xE5, 0x98, 0x0F, 0xB4, 0x8E, 0xE7];
        let res = ParserEngine::<SliceByteSource<'_>>::parse_binary_seconds_or_millis(
            &bytes,
            &props,
            DfdlSimpleType::DateTime,
        )
        .unwrap();
        assert_eq!(res, "9999-12-31T23:59:59.999000");

        // Exceeding maxValidYear: 284,000,000,000,000 ms
        let bytes_year_limit: [u8; 8] = [0x00, 0x01, 0x02, 0x4E, 0xD1, 0xA7, 0x08, 0x00];
        let err_limit = ParserEngine::<SliceByteSource<'_>>::parse_binary_seconds_or_millis(
            &bytes_year_limit,
            &props,
            DfdlSimpleType::DateTime,
        )
        .unwrap_err();
        assert!(err_limit
            .to_string()
            .contains("Tunable Limit Exceeded Error"));

        // Upper bound millis exceeded (0x028D46FBFCAE62E9)
        let bytes_upper_bound: [u8; 8] = [0x02, 0x8D, 0x46, 0xFB, 0xFC, 0xAE, 0x62, 0xE9];
        let err_upper = ParserEngine::<SliceByteSource<'_>>::parse_binary_seconds_or_millis(
            &bytes_upper_bound,
            &props,
            DfdlSimpleType::DateTime,
        )
        .unwrap_err();
        assert!(err_upper
            .to_string()
            .contains("millis value greater than upper bounds"));
    }

    #[test]
    fn test_unordered_sequence_parsing_and_reordering() {
        let mut builder = SchemaBuilder::new();

        let elem_a = CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_a = ResolvedProperties {
            initiator: Some(String::from("a:")),
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let a_id = builder
            .add_term_with_props(
                crate::types::QName::local("a"),
                TermKind::Element(elem_a),
                props_a,
            )
            .unwrap();

        let elem_b = CompiledElement {
            name: crate::types::QName::local("b"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_b = ResolvedProperties {
            initiator: Some(String::from("b:")),
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let b_id = builder
            .add_term_with_props(
                crate::types::QName::local("b"),
                TermKind::Element(elem_b),
                props_b,
            )
            .unwrap();

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![a_id, b_id],
        };
        let seq_props = ResolvedProperties {
            sequence_kind: crate::schema::ir::SequenceKind::Unordered,
            separator: Some(String::from(",")),
            separator_position: crate::schema::ir::SeparatorPosition::Infix,
            ..Default::default()
        };
        let seq_id = builder
            .add_term_with_props(
                crate::types::QName::local("seq"),
                TermKind::Sequence(seq),
                seq_props,
            )
            .unwrap();

        let root_elem = CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Data has b first, then a:
        let data = b"b:second,a:first";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = crate::limits::WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = engine.parse_document().unwrap();
        let root = doc.root.unwrap();
        assert_eq!(root.name.local_name, "root");
        // Verify Infoset reordering: 'a' must come before 'b' in infoset!
        assert_eq!(root.children.len(), 2);
        let c0_is_a = matches!(root.children.first(), Some(crate::infoset::tree::InfosetNode::Element(c)) if c.name.local_name == "a");
        let c1_is_b = matches!(root.children.get(1), Some(crate::infoset::tree::InfosetNode::Element(c)) if c.name.local_name == "b");
        assert!(c0_is_a);
        assert!(c1_is_b);
    }

    #[test]
    fn test_input_value_calc_bypasses_sequence_separators() {
        let mut builder = SchemaBuilder::new();

        let elem_a = CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_a = ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let a_id = builder
            .add_term_with_props(
                crate::types::QName::local("a"),
                TermKind::Element(elem_a),
                props_a,
            )
            .unwrap();

        let elem_calc = CompiledElement {
            name: crate::types::QName::local("calc"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_calc = ResolvedProperties {
            input_value_calc: Some(String::from("../a * 2")),
            ..Default::default()
        };
        let calc_id = builder
            .add_term_with_props(
                crate::types::QName::local("calc"),
                TermKind::Element(elem_calc),
                props_calc,
            )
            .unwrap();

        let elem_b = CompiledElement {
            name: crate::types::QName::local("b"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props_b = ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Delimited,
            ..Default::default()
        };
        let b_id = builder
            .add_term_with_props(
                crate::types::QName::local("b"),
                TermKind::Element(elem_b),
                props_b,
            )
            .unwrap();

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![a_id, calc_id, b_id],
        };
        let seq_props = ResolvedProperties {
            separator: Some(String::from(",")),
            separator_position: crate::schema::ir::SeparatorPosition::Infix,
            ..Default::default()
        };
        let seq_id = builder
            .add_term_with_props(
                crate::types::QName::local("seq"),
                TermKind::Sequence(seq),
                seq_props,
            )
            .unwrap();

        let root_elem = CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // Data contains ONLY a and b separated by a single comma; calc has no representation!
        let data = b"21,50";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = crate::limits::WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = engine.parse_document().unwrap();
        let root = doc.root.unwrap();
        assert_eq!(root.children.len(), 3);
        let c0_is_a = matches!(root.children.first(), Some(crate::infoset::tree::InfosetNode::Element(c)) if c.name.local_name == "a");
        let c1_is_calc = matches!(root.children.get(1), Some(crate::infoset::tree::InfosetNode::Element(c)) if c.name.local_name == "calc" && c.state == crate::infoset::state::ElementState::Value(DfdlValue::Int(42)));
        let c2_is_b = matches!(root.children.get(2), Some(crate::infoset::tree::InfosetNode::Element(c)) if c.name.local_name == "b");
        assert!(c0_is_a);
        assert!(c1_is_calc);
        assert!(c2_is_b);
    }

    #[test]
    fn test_bit_order_byte_boundary_enforcement() {
        use crate::io::source::SliceByteSource;
        use crate::schema::builder::SchemaBuilder;
        use crate::schema::ir::{CompiledElement, CompiledSequence, TermKind};

        let mut builder = SchemaBuilder::new();

        let mut bit_props = ResolvedProperties::default();
        bit_props.representation = crate::schema::ir::Representation::Binary;
        bit_props.length_kind = crate::schema::ir::LengthKind::Explicit;
        bit_props.length_units = crate::schema::ir::LengthUnits::Bits;
        bit_props.length = Some(1);
        bit_props.bit_order = BitOrder::MostSignificantBitFirst;

        let bit_elem = CompiledElement {
            name: crate::types::QName::local("bit"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let mut s_props = ResolvedProperties::default();
        s_props.representation = crate::schema::ir::Representation::Binary;
        s_props.length_kind = crate::schema::ir::LengthKind::Explicit;
        s_props.length_units = crate::schema::ir::LengthUnits::Bits;
        s_props.length = Some(7);
        s_props.bit_order = BitOrder::LeastSignificantBitFirst;

        let s_elem = CompiledElement {
            name: crate::types::QName::local("s"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let bit_id = builder
            .add_term_with_props(
                crate::types::QName::local("bit"),
                TermKind::Element(bit_elem),
                bit_props,
            )
            .unwrap();

        let s_id = builder
            .add_term_with_props(
                crate::types::QName::local("s"),
                TermKind::Element(s_elem),
                s_props,
            )
            .unwrap();

        let seq = CompiledSequence {
            members: alloc::vec![bit_id, s_id],
        };
        let seq_id = builder
            .add_term_with_props(
                crate::types::QName::local("seq"),
                TermKind::Sequence(seq),
                ResolvedProperties::default(),
            )
            .unwrap();

        let root_elem = CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);

        let schema = builder.build().unwrap();

        let data = [0b10101010];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = crate::limits::WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);
        let err = engine.parse_document().unwrap_err();
        assert!(err.to_string().contains("bitOrder on a byte boundary"));
    }

    #[test]
    fn test_conformance_expected_error_clusters() {
        use crate::kernel::parser::calendar::parse_calendar_from_text;
        use crate::kernel::parser::numbers::{parse_strict_f64, parse_strict_int_i64, parse_strict_uint_u64};

        // 1. Packed decimal sign nibble validation per DFDL §13.7.1
        // Low nibble 0x04 is a digit, not a valid packed sign nibble (must be 0x0A..=0x0F)
        let invalid_sign_bytes = [0x12, 0x34];
        let res_sign = crate::util::decode_packed_decimal(&invalid_sign_bytes);
        assert!(res_sign.is_err());
        assert!(res_sign.unwrap_err().to_string().contains("Invalid sign nibble"));

        // Valid packed decimal sign nibbles: 0x0C (positive), 0x0D (negative)
        let valid_pos = [0x12, 0x3C];
        assert_eq!(crate::util::decode_packed_decimal(&valid_pos).unwrap(), 123);
        let valid_neg = [0x12, 0x3D];
        assert_eq!(crate::util::decode_packed_decimal(&valid_neg).unwrap(), -123);

        // 2. Strict number policy unsolicited whitespace rejection per DFDL §13.6
        // Leading or trailing spaces without explicit pattern padding must be rejected in strict mode
        assert_eq!(
            parse_strict_int_i64("    52    ", Some("##"), ".", ",", None, crate::schema::ir::TextTrimKind::None),
            None
        );
        assert_eq!(
            parse_strict_uint_u64("999 ", None, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            None
        );
        assert_eq!(
            parse_strict_uint_u64(" 999", None, ".", ",", None, crate::schema::ir::TextTrimKind::None),
            None
        );

        // Valid strict integer without whitespace
        assert_eq!(
            parse_strict_int_i64("52", Some("##"), ".", ",", None, crate::schema::ir::TextTrimKind::None),
            Some(52)
        );

        // 3. Negative subpattern grouping inheritance per ICU DecimalFormat and DFDL §13.7.1
        // The negative subpattern (#,#,0.00) grouping is ignored; grouping size 3 is inherited from positive (#,##0.00)
        // Therefore, (4,3,0.00) with group size 1 must fail grouping validation
        let float_fail = parse_strict_f64(
            "(4,3,0.00)",
            Some("#,##0.00;(#,#,0.00)"),
            ".",
            ",",
            None,
            None,
            None,
            false,
            None,
            crate::schema::ir::TextTrimKind::None,
        );
        assert_eq!(float_fail, None);

        // Valid grouping following positive subpattern grouping size 3
        let float_pass = parse_strict_f64(
            "(4,300.00)",
            Some("#,##0.00;(#,#,0.00)"),
            ".",
            ",",
            None,
            None,
            None,
            false,
            None,
            crate::schema::ir::TextTrimKind::None,
        );
        assert_eq!(float_pass, Some(-4300.0));

        // 4. Calendar hour pattern validation
        // KK denotes hour in am/pm (0~11); hour 12 is out of range
        let kk_res = parse_calendar_from_text(
            "12:30AM",
            Some("KK:mmaa"),
            None,
            crate::schema::ir::TextTrimKind::None,
            DfdlSimpleType::Time,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
        );
        assert!(kk_res.is_err());

        // hh denotes hour in am/pm (1~12); hour 0 is out of range
        let hh_res = parse_calendar_from_text(
            "00:30AM",
            Some("hh:mmaa"),
            None,
            crate::schema::ir::TextTrimKind::None,
            DfdlSimpleType::Time,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
        );
        assert!(hh_res.is_err());

        // Valid hours
        let kk_valid = parse_calendar_from_text(
            "11:30AM",
            Some("KK:mmaa"),
            None,
            crate::schema::ir::TextTrimKind::None,
            DfdlSimpleType::Time,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
        );
        assert_eq!(kk_valid.unwrap(), "11:30:00");

        // 5. XML Schema §3.2.7 timezone -00:00 prohibition
        let tz_res = parse_calendar_from_text(
            "2013-03-24T03:45:30-00:00",
            None,
            None,
            crate::schema::ir::TextTrimKind::None,
            DfdlSimpleType::DateTime,
            crate::schema::ir::CalendarCheckPolicy::Strict,
            crate::schema::ir::CalendarFirstDayOfWeek::Sunday,
        );
        assert!(tz_res.is_err());
        assert!(tz_res.unwrap_err().to_string().contains("-00:00"));
    }

    #[test]
    fn test_unsupported_types_and_formatters_cluster() {
        use crate::kernel::parser::calendar::parse_calendar_from_text;
        use crate::kernel::parser::numbers::parse_zoned_number;
        use crate::schema::ir::{
            CalendarCheckPolicy, CalendarFirstDayOfWeek, TextTrimKind,
            TextZonedSignStyle,
        };

        // 1. Zoned decimal and integer tests across all 4 styles (DFDL §13.7.2)
        // asciiStandard: 'A' -> 1 (+), 'J' -> 1 (-), '{' -> 0 (+), '}' -> 0 (-)
        let num_str = parse_zoned_number(
            "12A",
            Some("00+"),
            TextZonedSignStyle::AsciiStandard,
            false,
            true,
        )
        .expect("asciiStandard trailing positive");
        assert_eq!(num_str, "121");

        let num_str = parse_zoned_number(
            "12J",
            Some("00+"),
            TextZonedSignStyle::AsciiStandard,
            false,
            true,
        )
        .expect("asciiStandard trailing negative");
        assert_eq!(num_str, "-121");

        // asciiCaRealiaModified: '0'..'9' (+) with negative overpunch ' ' (0), '!'..=')' (1..9)
        let num_str = parse_zoned_number(
            "12!",
            Some("00+"),
            TextZonedSignStyle::AsciiCaRealiaModified,
            false,
            true,
        )
        .expect("asciiCaRealiaModified trailing negative");
        assert_eq!(num_str, "-121");

        // asciiTandemModified: negative overpunch '\u{80}' (0), '\u{81}'..='\u{89}' (1..9)
        let num_str = parse_zoned_number(
            "12\u{81}",
            Some("00+"),
            TextZonedSignStyle::AsciiTandemModified,
            false,
            true,
        )
        .expect("asciiTandemModified trailing negative");
        assert_eq!(num_str, "-121");

        // Virtual decimal point 'V': scale calculation
        // e.g. pattern "000V00+" -> 2 fractional digits
        let num_str = parse_zoned_number(
            "1234A",
            Some("000V00+"),
            TextZonedSignStyle::AsciiStandard,
            false,
            true,
        )
        .expect("zoned with virtual decimal point V");
        assert_eq!(num_str, "123.41");

        // EBCDIC decoding for zoned numbers
        let ebcdic_bytes = [0xF1, 0xF2, 0xD3]; // "12" followed by 0xD3 (L = 3 negative in EBCDIC)
        let ebcdic_str = crate::encoding::decode_text_bytes(&ebcdic_bytes, "ebcdic-cp-us").expect("ebcdic decode");
        let num_str = parse_zoned_number(
            &ebcdic_str,
            Some("00+"),
            TextZonedSignStyle::AsciiStandard,
            true,
            true,
        )
        .expect("ebcdic zoned parsing");
        assert_eq!(num_str, "-123");

        // 2. Lax calendar rollover tests (DFDL §13.12, calendarCheckPolicy="lax")
        // Date rollover: 2000-12-45 -> rolls over 14 days into Jan 2001
        let date_lax = parse_calendar_from_text(
            "2000-12-45",
            Some("yyyy-MM-dd"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Date,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Sunday,
        )
        .expect("lax date rollover");
        assert_eq!(date_lax, "2001-01-14");

        // Time rollover: 27:30:30 -> 03:30:30 (next day, hour rolls 27 % 24 = 3)
        let time_lax = parse_calendar_from_text(
            "27:30:30",
            Some("HH:mm:ss"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Time,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Sunday,
        )
        .expect("lax time rollover");
        assert_eq!(time_lax, "03:30:30");

        // Time minute rollover: 04:61:44 PM -> 17:01:44
        let time_min_lax = parse_calendar_from_text(
            "04:61:44 PM",
            Some("hh:mm:ss aa"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Time,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Sunday,
        )
        .expect("lax minute rollover");
        assert_eq!(time_min_lax, "17:01:44");

        // 3. Calendar day of week parsing ('e' / 'E')
        let dow_sun = parse_calendar_from_text(
            "1970-01-1",
            Some("yyyy-MM-e"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Date,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Sunday,
        )
        .expect("day of week with Sunday first");
        // In Jan 1970, Jan 1 was Thursday (day 5 when Sun=1). First Sunday is Jan 4.
        assert_eq!(dow_sun, "1970-01-04");

        let dow_mon = parse_calendar_from_text(
            "1970-01-1",
            Some("yyyy-MM-e"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Date,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Monday,
        )
        .expect("day of week with Monday first");
        // First Monday of Jan 1970 is Jan 5.
        assert_eq!(dow_mon, "1970-01-05");

        // 4. Number padding character trimming
        use crate::kernel::parser::numbers::parse_strict_int_i64;
        let padded_int = parse_strict_int_i64(
            "**42**",
            Some("##"),
            ".",
            ",",
            Some("*"),
            TextTrimKind::Both,
        )
        .expect("padded int trimmed");
        assert_eq!(padded_int, 42);

        // 5. Calendar language test with Spanish full month name and day of week (dateCalendarLanguage2)
        let date_es = parse_calendar_from_text(
            "Lunes Noviembre 2013",
            Some("EEEE MMM yyyy"),
            None,
            TextTrimKind::None,
            DfdlSimpleType::Date,
            CalendarCheckPolicy::Lax,
            CalendarFirstDayOfWeek::Sunday,
        )
        .expect("Spanish month and weekday parsing");
        assert_eq!(date_es, "2013-11-04");
    }

    /// Verifies that text boolean parsing correctly supports multi-character representations
    /// with `ignoreCase="yes"`, ensures entity-decoded pad characters (such as `%SP;`) do not
    /// improperly strip letters from representations, and validates that scalar versus array
    /// element minOccurs constraints conform to DFDL 1.0 §16.1.
    #[test]
    fn test_text_boolean_case_insensitivity_and_array_conformance() {
        use crate::io::{BitReader, SliceByteSource};
        use crate::kernel::ParserEngine;
        use crate::schema::builder::SchemaBuilder;
        use crate::schema::ir::{
            CompiledElement, CompiledSequence, CompiledType, LengthKind, OccursCountKind,
            Representation, ResolvedProperties, SeparatorPosition, TermKind,
        };
        use crate::types::QName;

        // Construct schema with sequence having delimiter ',' and boolean element 'x'
        let mut builder = SchemaBuilder::new();

        let mut x_props = ResolvedProperties::default();
        x_props.representation = Representation::Text;
        x_props.length_kind = LengthKind::Delimited;
        x_props.text_boolean_true_rep = Some(alloc::string::ToString::to_string("yes Y 1"));
        x_props.text_boolean_false_rep = Some(alloc::string::ToString::to_string("no N 0"));
        x_props.text_boolean_pad_character = Some(alloc::string::ToString::to_string(" "));
        x_props.ignore_case = true;
        x_props.occurs_count_kind = OccursCountKind::Parsed;

        let x_elem = CompiledElement {
            name: QName::local("x"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Boolean),
            min_occurs: 1,
            max_occurs: None, // unbounded array
            is_nillable: false,
            default_value: None,
        };
        let x_id = builder
            .add_term_with_props(QName::local("x"), TermKind::Element(x_elem), x_props)
            .expect("add x term");

        let mut seq_props = ResolvedProperties::default();
        seq_props.separator = Some(alloc::string::ToString::to_string(","));
        seq_props.terminator = Some(alloc::string::ToString::to_string(";"));
        seq_props.separator_position = SeparatorPosition::Infix;
        seq_props.separator_suppression_policy =
            crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty;

        let seq = CompiledSequence {
            members: alloc::vec![x_id],
        };
        let seq_id = builder
            .add_term_with_props(QName::local("seq"), TermKind::Sequence(seq), seq_props)
            .expect("add seq term");

        let root_elem = CompiledElement {
            name: QName::local("root"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("root"), TermKind::Element(root_elem))
            .expect("add root term");
        builder.set_root(root_id);

        let schema = builder.build().expect("build schema");

        // Input data contains mixed casing: '1,y,YES,0,NO,n;'
        let input = b"1,y,YES,0,NO,n;";
        let src = SliceByteSource::new(input);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = crate::limits::WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);

        let doc = parser.parse_document().expect("parse document successful");
        let root = doc.root.expect("root element present");
        assert_eq!(root.children.len(), 6);

        // Verify values: true, true, true, false, false, false
        let expected = [true, true, true, false, false, false];
        for (child, &exp) in root.children.iter().zip(&expected) {
            let crate::infoset::InfosetNode::Element(e) = child;
            assert_eq!(
                e.state,
                crate::infoset::ElementState::Value(DfdlValue::Boolean(exp))
            );
        }
    }

    #[test]
    fn test_dynamic_set_variable_and_byte_order() {
        use crate::schema::ir::{CompiledElement, CompiledSequence, CompiledType, TermKind};
        use crate::types::QName;

        let mut builder = SchemaBuilder::new();
        builder.define_variable(QName::local("bom"), DfdlSimpleType::String, None);

        let mut bom_props = ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            ..Default::default()
        };
        bom_props.set_variables.push((
            QName::local("bom"),
            "{\n  if (xs:unsignedShort(.) eq 65534) then 'littleEndian'\n  else 'bigEndian'\n}".to_string(),
        ));

        let bom_elem = CompiledElement {
            name: QName::local("bom"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedShort),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let bom_id = builder
            .add_term_with_props(QName::local("bom"), TermKind::Element(bom_elem), bom_props)
            .unwrap();

        let num_props = ResolvedProperties {
            representation: crate::schema::ir::Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            byte_order_expr: Some(alloc::string::String::from("{ $bom }")),
            ..Default::default()
        };
        let num_elem = CompiledElement {
            name: QName::local("num"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let num_id = builder
            .add_term_with_props(QName::local("num"), TermKind::Element(num_elem), num_props)
            .unwrap();

        let seq = CompiledSequence {
            members: alloc::vec![bom_id, num_id],
        };
        let seq_id = builder
            .add_term(QName::local("seq"), TermKind::Sequence(seq))
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("e4"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("e4"), TermKind::Element(root_elem))
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let data = [0xFF, 0xFE, 0x00, 0x01, 0x00, 0x01];
        let src = SliceByteSource::new(&data);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);

        let doc = parser.parse_document().unwrap();
        let root = doc.root.unwrap();
        assert_eq!(root.children.len(), 2);
    }
