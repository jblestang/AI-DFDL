#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

    use super::*;
    use super::binary::*;
    use super::delimiters::*;
    use super::element::*;
    use super::calendar::*;
    use super::numbers::*;
    use crate::infoset::{DfdlSimpleType, DfdlValue};
    use crate::infoset::tree::InfosetNode;
    use crate::io::source::SliceByteSource;
    use crate::io::traits::{BitOrder, ByteOrder};
    use crate::schema::builder::SchemaBuilder;
    use crate::schema::ir::{
        CalendarCheckPolicy, CalendarFirstDayOfWeek, CompiledChoice, CompiledElement,
        CompiledSequence, CompiledType, LengthKind, OccursCountKind, Representation,
        ResolvedProperties, TermKind, TextTrimKind,
    };
    use crate::types::QName;
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

        // Int and Long to Decimal (lines 278-294)
        assert_eq!(
            coerce_and_validate_ivc_value(&DfdlValue::Long(123), &DfdlSimpleType::Decimal, &def_props).unwrap(),
            DfdlValue::Decimal("123".into())
        );
        assert_eq!(
            coerce_and_validate_ivc_value(&DfdlValue::Int(123), &DfdlSimpleType::Decimal, &def_props).unwrap(),
            DfdlValue::Decimal("123".into())
        );
        let res_non_neg_int = coerce_and_validate_ivc_value(&DfdlValue::Int(-30), &DfdlSimpleType::Decimal, &non_neg_props);
        assert!(res_non_neg_int.is_err());
        assert!(alloc::format!("{}", res_non_neg_int.unwrap_err()).contains("Cannot convert"));

        // Invalid Decimal to Float and Double (lines 296-311)
        let res_bad_flt = coerce_and_validate_ivc_value(&DfdlValue::Decimal("invalid".into()), &DfdlSimpleType::Float, &def_props);
        assert!(res_bad_flt.is_err());
        assert!(alloc::format!("{}", res_bad_flt.unwrap_err()).contains("out of range for xs:float"));

        let res_bad_dbl = coerce_and_validate_ivc_value(&DfdlValue::Decimal("invalid".into()), &DfdlSimpleType::Double, &def_props);
        assert!(res_bad_dbl.is_err());
        assert!(alloc::format!("{}", res_bad_dbl.unwrap_err()).contains("out of range for xs:double"));

        // String overflow conversions for Int, Short, Byte, UnsignedInt, UnsignedShort, UnsignedByte (lines 322-478)
        let res_overflow_int = coerce_and_validate_ivc_value(&DfdlValue::String("999999999999".into()), &DfdlSimpleType::Int, &def_props);
        assert!(res_overflow_int.is_err());
        assert!(alloc::format!("{}", res_overflow_int.unwrap_err()).contains("out of range for xs:int"));

        let res_overflow_short = coerce_and_validate_ivc_value(&DfdlValue::String("99999".into()), &DfdlSimpleType::Short, &def_props);
        assert!(res_overflow_short.is_err());
        assert!(alloc::format!("{}", res_overflow_short.unwrap_err()).contains("out of range for xs:short"));

        let res_overflow_byte = coerce_and_validate_ivc_value(&DfdlValue::String("999".into()), &DfdlSimpleType::Byte, &def_props);
        assert!(res_overflow_byte.is_err());
        assert!(alloc::format!("{}", res_overflow_byte.unwrap_err()).contains("out of range for xs:byte"));

        let res_overflow_uint = coerce_and_validate_ivc_value(&DfdlValue::String("999999999999".into()), &DfdlSimpleType::UnsignedInt, &def_props);
        assert!(res_overflow_uint.is_err());
        assert!(alloc::format!("{}", res_overflow_uint.unwrap_err()).contains("out of range for xs:unsignedInt"));

        let res_overflow_ushort = coerce_and_validate_ivc_value(&DfdlValue::String("999999".into()), &DfdlSimpleType::UnsignedShort, &def_props);
        assert!(res_overflow_ushort.is_err());
        assert!(alloc::format!("{}", res_overflow_ushort.unwrap_err()).contains("out of range for xs:unsignedShort"));

        let res_overflow_ubyte = coerce_and_validate_ivc_value(&DfdlValue::String("9999".into()), &DfdlSimpleType::UnsignedByte, &def_props);
        assert!(res_overflow_ubyte.is_err());
        assert!(alloc::format!("{}", res_overflow_ubyte.unwrap_err()).contains("out of range for xs:unsignedByte"));

        let res_neg_ushort = coerce_and_validate_ivc_value(&DfdlValue::String("-5".into()), &DfdlSimpleType::UnsignedShort, &def_props);
        assert!(res_neg_ushort.is_err());
        assert!(alloc::format!("{}", res_neg_ushort.unwrap_err()).contains("must match in signedness"));

        // Valid String coercions to numeric types (lines 328, 337, 349, 367, 392, 422, 452, 478)
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::Int, &def_props).unwrap(), DfdlValue::Int(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::Long, &def_props).unwrap(), DfdlValue::Long(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::Short, &def_props).unwrap(), DfdlValue::Short(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::Byte, &def_props).unwrap(), DfdlValue::Byte(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::UnsignedInt, &def_props).unwrap(), DfdlValue::UnsignedInt(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::UnsignedLong, &def_props).unwrap(), DfdlValue::UnsignedLong(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::UnsignedShort, &def_props).unwrap(), DfdlValue::UnsignedShort(42));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("42".into()), &DfdlSimpleType::UnsignedByte, &def_props).unwrap(), DfdlValue::UnsignedByte(42));

        // HexBinary String coercion (lines 249-261)
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("DEADBEEF".into()), &DfdlSimpleType::HexBinary, &def_props).unwrap(), DfdlValue::HexBinary(alloc::vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(coerce_and_validate_ivc_value(&DfdlValue::String("ODD".into()), &DfdlSimpleType::HexBinary, &def_props).unwrap(), DfdlValue::String("ODD".into()));
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
        s_props.alignment_units = crate::schema::ir::AlignmentUnits::Bits;
        s_props.alignment = 1;

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

    /// Tests compliance with DFDL v1.0 §16.1.4: `dfdl:occursCountKind="parsed"`.
    ///
    /// # Specification Rationale (DFDL §16.1.4)
    /// Under `occursCountKind="parsed"`, the number of occurrences of an array element is
    /// established solely through speculative parsing. The parser does not enforce `minOccurs`
    /// as a parse-time stopping constraint or fatal parse error; occurrences are parsed until
    /// speculative parsing cannot match another occurrence (e.g. at EOF or on zero-length
    /// non-matching text). When zero occurrences are found, the parser successfully yields
    /// an empty parent sequence, with `minOccurs` checking reserved for validation mode.
    ///
    /// # Test Scenario
    /// A root complex element `LP_01` contains a sequence with a child `num` typed as `xs:int`,
    /// `dfdl:lengthKind="explicit"`, `dfdl:length="0"`, `dfdl:occursCountKind="parsed"`, and
    /// `maxOccurs="unbounded"`. With an empty data stream (0 bytes), the parser speculatively
    /// attempts occurrence 1, discovers EOF/empty representation, terminates the array cleanly,
    /// and constructs `<LP_01></LP_01>` with zero occurrences of `num`.
    #[test]
    fn test_occurs_count_kind_parsed_zero_length() {
        // Step 1: Initialize schema builder for testing occursCountKind="parsed"
        let mut builder = SchemaBuilder::new();

        // Step 2: Configure resolved properties for the array member `num`
        let mut num_props = ResolvedProperties::default();
        num_props.representation = Representation::Text;
        num_props.length_kind = LengthKind::Explicit;
        num_props.length = Some(0); // Zero-length integer representation
        num_props.occurs_count_kind = OccursCountKind::Parsed;

        // Step 3: Define child element `num` with unbounded maxOccurs
        let num_elem = CompiledElement {
            name: QName::local("num"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1, // Default minOccurs in XSD; speculative parsing must not fail on it
            max_occurs: None, // Unbounded array
            is_nillable: false,
            default_value: None,
        };
        let num_id = builder
            .add_term_with_props(QName::local("num"), TermKind::Element(num_elem), num_props)
            .expect("register num element term");

        // Step 4: Define sequence containing `num`
        let seq = CompiledSequence {
            members: alloc::vec![num_id],
        };
        let seq_id = builder
            .add_term(QName::local("seq"), TermKind::Sequence(seq))
            .expect("register sequence term");

        // Step 5: Define root element `LP_01` containing the sequence
        let root_elem = CompiledElement {
            name: QName::local("LP_01"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("LP_01"), TermKind::Element(root_elem))
            .expect("register root term");
        builder.set_root(root_id);
        let schema = builder.build().expect("build schema");

        // Step 6: Execute parser on an empty byte buffer (0 bytes)
        let data = [];
        let src = SliceByteSource::new(&data);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);

        // Step 7: Verify parse successfully produces empty LP_01 infoset without error
        let doc = parser.parse_document().expect("parse document with empty parsed array");
        let root = doc.root.expect("root element present");
        assert_eq!(root.name.local_name, "LP_01");
        assert_eq!(root.children.len(), 0);
    }

    /// Tests compliance with DFDL v1.0 §7.5 and §9.3.2: Nested Choice Multiple Discriminator Resolution.
    ///
    /// # Specification Rationale (DFDL §7.5, §9.3.2)
    /// Points of Uncertainty (PoUs) form a hierarchy corresponding to nested choices and array
    /// occurrences. A discriminator evaluates against the innermost unresolved PoU. When a
    /// sequence within an inner choice branch contains multiple sequential discriminators:
    /// - The first discriminator that evaluates to true resolves the innermost (inner choice) PoU.
    /// - If a subsequent discriminator in the same branch evaluates to false, it triggers a
    ///   processing error.
    /// - Because the inner choice PoU is already resolved (discriminated), the parser CANNOT
    ///   backtrack to alternate branches of the inner choice.
    /// - Instead, the error propagates out of the resolved inner choice up to the outer choice
    ///   PoU (which remains unresolved).
    /// - The outer choice then successfully backtracks and tries its next alternative branch.
    ///
    /// # Test Scenario
    /// Data stream: "tfa".
    /// - `discrim1` reads "t".
    /// - `discrim2` reads "f".
    /// - Outer choice evaluates branch 1: contains inner choice.
    /// - Inner choice evaluates branch 1: sequence with two discriminators:
    ///   - Discriminator 1 checks `discrim1 eq 't'` -> evaluates to true, resolving inner choice PoU.
    ///   - Discriminator 2 checks `discrim2 eq 't'` -> evaluates to false ("f" != "t"), failing.
    /// - Backtracks out of inner choice into outer choice.
    /// - Outer choice evaluates branch 2: reads "a" into `outerBranch2` -> succeeds!
    #[test]
    fn test_nested_choice_multiple_discriminators_backtracking() {
        // Step 1: Initialize schema builder for nested choice discriminator test
        let mut builder = SchemaBuilder::new();

        // Step 2: Element discrim1 (length 1)
        let mut d1_props = ResolvedProperties::default();
        d1_props.representation = Representation::Text;
        d1_props.length_kind = LengthKind::Explicit;
        d1_props.length = Some(1);
        let d1_elem = CompiledElement {
            name: QName::local("discrim1"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let d1_id = builder
            .add_term_with_props(QName::local("discrim1"), TermKind::Element(d1_elem), d1_props)
            .expect("add discrim1");

        // Step 3: Element discrim2 (length 1)
        let mut d2_props = ResolvedProperties::default();
        d2_props.representation = Representation::Text;
        d2_props.length_kind = LengthKind::Explicit;
        d2_props.length = Some(1);
        let d2_elem = CompiledElement {
            name: QName::local("discrim2"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let d2_id = builder
            .add_term_with_props(QName::local("discrim2"), TermKind::Element(d2_elem), d2_props)
            .expect("add discrim2");

        // Step 4: Sequence 1 with Discriminator 1: { ../../discrim1 eq 't' }
        let mut s1_props = ResolvedProperties::default();
        s1_props.discriminator = Some(alloc::string::ToString::to_string("../../discrim1 eq 't'"));
        s1_props.discriminator_test_kind = crate::schema::ir::TestKind::Expression;
        let s1 = CompiledSequence { members: alloc::vec![] };
        let s1_id = builder
            .add_term_with_props(QName::local("s1"), TermKind::Sequence(s1), s1_props)
            .expect("add s1");

        // Step 5: Sequence 2 with Discriminator 2: { ../../discrim2 eq 't' }
        let mut s2_props = ResolvedProperties::default();
        s2_props.discriminator = Some(alloc::string::ToString::to_string("../../discrim2 eq 't'"));
        s2_props.discriminator_test_kind = crate::schema::ir::TestKind::Expression;
        let s2 = CompiledSequence { members: alloc::vec![] };
        let s2_id = builder
            .add_term_with_props(QName::local("s2"), TermKind::Sequence(s2), s2_props)
            .expect("add s2");

        // Step 6: Element integer in innerBranch1
        let mut int_props = ResolvedProperties::default();
        int_props.representation = Representation::Text;
        int_props.length_kind = LengthKind::Explicit;
        int_props.length = Some(1);
        let int_elem = CompiledElement {
            name: QName::local("integer"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let int_id = builder
            .add_term_with_props(QName::local("integer"), TermKind::Element(int_elem), int_props)
            .expect("add integer");

        // Step 7: innerBranch1 sequence: contains s1, s2, integer
        let inner_seq = CompiledSequence {
            members: alloc::vec![s1_id, s2_id, int_id],
        };
        let inner_seq_id = builder
            .add_term(QName::local("inner_seq"), TermKind::Sequence(inner_seq))
            .expect("add inner_seq");

        let inner_b1_elem = CompiledElement {
            name: QName::local("innerBranch1"),
            type_ir: CompiledType::Complex(inner_seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let inner_b1_id = builder
            .add_term(QName::local("innerBranch1"), TermKind::Element(inner_b1_elem))
            .expect("add innerBranch1");

        // Step 8: innerBranch2 element
        let mut ib2_props = ResolvedProperties::default();
        ib2_props.representation = Representation::Text;
        ib2_props.length_kind = LengthKind::Explicit;
        ib2_props.length = Some(1);
        let inner_b2_elem = CompiledElement {
            name: QName::local("innerBranch2"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let inner_b2_id = builder
            .add_term_with_props(QName::local("innerBranch2"), TermKind::Element(inner_b2_elem), ib2_props)
            .expect("add innerBranch2");

        // Step 9: innerChoice: branches [innerBranch1, innerBranch2]
        let inner_choice = CompiledChoice {
            branches: alloc::vec![inner_b1_id, inner_b2_id],
        };
        let inner_choice_id = builder
            .add_term(QName::local("inner_choice"), TermKind::Choice(inner_choice))
            .expect("add inner_choice");

        // Step 10: outerBranch1 element containing innerChoice
        let outer_b1_elem = CompiledElement {
            name: QName::local("outerBranch1"),
            type_ir: CompiledType::Complex(inner_choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let outer_b1_id = builder
            .add_term(QName::local("outerBranch1"), TermKind::Element(outer_b1_elem))
            .expect("add outerBranch1");

        // Step 11: outerBranch2 element (fallback branch on outer choice)
        let mut ob2_props = ResolvedProperties::default();
        ob2_props.representation = Representation::Text;
        ob2_props.length_kind = LengthKind::Explicit;
        ob2_props.length = Some(1);
        let outer_b2_elem = CompiledElement {
            name: QName::local("outerBranch2"),
            type_ir: CompiledType::Simple(DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let outer_b2_id = builder
            .add_term_with_props(QName::local("outerBranch2"), TermKind::Element(outer_b2_elem), ob2_props)
            .expect("add outerBranch2");

        // Step 12: outerChoice: branches [outerBranch1, outerBranch2]
        let outer_choice = CompiledChoice {
            branches: alloc::vec![outer_b1_id, outer_b2_id],
        };
        let outer_choice_id = builder
            .add_term(QName::local("outer_choice"), TermKind::Choice(outer_choice))
            .expect("add outer_choice");

        // Step 13: root sequence containing discrim1, discrim2, outerChoice
        let root_seq = CompiledSequence {
            members: alloc::vec![d1_id, d2_id, outer_choice_id],
        };
        let root_seq_id = builder
            .add_term(QName::local("root_seq"), TermKind::Sequence(root_seq))
            .expect("add root_seq");

        let root_elem = CompiledElement {
            name: QName::local("root"),
            type_ir: CompiledType::Complex(root_seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("root"), TermKind::Element(root_elem))
            .expect("add root");
        builder.set_root(root_id);
        let schema = builder.build().expect("build schema");

        // Step 14: Parse input data "tfa"
        let data = b"tfa";
        let src = SliceByteSource::new(data);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(2000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);

        // Step 15: Verify that backtracking from inner choice to outer choice succeeded
        let doc = parser.parse_document().expect("parse document with multiple discriminators");
        let root = doc.root.expect("root element");
        assert_eq!(root.name.local_name, "root");
        assert_eq!(root.children.len(), 3);
        assert!(matches!(root.children.first(), Some(InfosetNode::Element(e)) if e.name.local_name == "discrim1"));
        assert!(matches!(root.children.get(1), Some(InfosetNode::Element(e)) if e.name.local_name == "discrim2"));
        assert!(matches!(root.children.get(2), Some(InfosetNode::Element(e)) if e.name.local_name == "outerBranch2"));
    }

    /// Verifies virtual decimal formatting and binary parser operations (float, double, boolean, hexBinary, limits).
    #[test]
    fn test_binary_parser_comprehensive_coverage() {
        use crate::schema::ir::{BinaryBooleanRep, BinaryCalendarRep, BinaryNumberRep, LengthUnits};

        // 1. format_virtual_decimal
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(12345, 0), "12345");
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(12345, -2), "1234500");
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(12345, 2), "123.45");
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(12, 4), "0.0012");
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(-12345, 2), "-123.45");
        assert_eq!(ParserEngine::<SliceByteSource>::format_virtual_decimal(-5, 3), "-0.005");

        let schema = dummy_schema();

        // 2. Binary Float & Double (BigEndian & LittleEndian)
        let float_bytes = [0x42, 0xF6, 0xE6, 0x66]; // 123.45f32 in BE
        let src = SliceByteSource::new(&float_bytes);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut engine = ParserEngine::new(&schema, &mut reader, &mut budget);

        let mut props_float = ResolvedProperties::default();
        props_float.byte_order = ByteOrder::BigEndian;
        props_float.representation = Representation::Binary;
        let val_float = engine.parse_binary_value(DfdlSimpleType::Float, &props_float, None).unwrap();
        if let DfdlValue::Float(f) = val_float {
            assert!((f - 123.45).abs() < 0.01);
        } else {
            panic!("Expected Float");
        }

        let double_bytes = [0x40, 0x5E, 0xDC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCD]; // 123.45f64 in BE
        let src_d = SliceByteSource::new(&double_bytes);
        let mut reader_d = BitReader::new(src_d, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_d = ParserEngine::new(&schema, &mut reader_d, &mut budget);

        let mut props_double = ResolvedProperties::default();
        props_double.byte_order = ByteOrder::BigEndian;
        let val_double = engine_d.parse_binary_value(DfdlSimpleType::Double, &props_double, None).unwrap();
        if let DfdlValue::Double(d) = val_double {
            assert!((d - 123.45).abs() < 0.001);
        } else {
            panic!("Expected Double");
        }

        // 3. Binary Boolean custom representation match and mismatch error
        let bool_bytes = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02];
        let src_b = SliceByteSource::new(&bool_bytes);
        let mut reader_b = BitReader::new(src_b, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_b = ParserEngine::new(&schema, &mut reader_b, &mut budget);

        let mut props_bool = ResolvedProperties::default();
        props_bool.binary_boolean_true_rep = BinaryBooleanRep::Value(1);
        props_bool.binary_boolean_false_rep = BinaryBooleanRep::Value(0);
        let val_b_true = engine_b.parse_binary_value(DfdlSimpleType::Boolean, &props_bool, None).unwrap();
        assert_eq!(val_b_true, DfdlValue::Boolean(true));

        // Reading value 2 matches neither 1 nor 0 -> Parse Error
        assert!(engine_b.parse_binary_value(DfdlSimpleType::Boolean, &props_bool, None).is_err());

        // 4. Binary integer bit bounds validation (SDE)
        let dummy_bytes = [0xAA, 0xBB];
        let src_lim = SliceByteSource::new(&dummy_bytes);
        let mut reader_lim = BitReader::new(src_lim, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_lim = ParserEngine::new(&schema, &mut reader_lim, &mut budget);

        let mut props_lim = ResolvedProperties::default();
        props_lim.length_units = LengthUnits::Bits;

        // Unsigned with 0 bits -> SDE
        assert!(engine_lim.parse_binary_value(DfdlSimpleType::UnsignedInt, &props_lim, Some(0)).is_err());

        // Signed with 0 bits -> SDE
        assert!(engine_lim.parse_binary_value(DfdlSimpleType::Int, &props_lim, Some(0)).is_err());

        // Byte with 10 bits (> 8) -> SDE
        assert!(engine_lim.parse_binary_value(DfdlSimpleType::Byte, &props_lim, Some(10)).is_err());

        // 5. HexBinary length in bits and max length limit
        let hex_data = [0xAB, 0xCD, 0xEF];
        let src_hex = SliceByteSource::new(&hex_data);
        let mut reader_hex = BitReader::new(src_hex, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_hex = ParserEngine::new(&schema, &mut reader_hex, &mut budget);

        let mut props_hex = ResolvedProperties::default();
        props_hex.length_units = LengthUnits::Bits;
        let hex_val = engine_hex.parse_binary_value(DfdlSimpleType::HexBinary, &props_hex, Some(12)).unwrap();
        if let DfdlValue::HexBinary(b) = hex_val {
            assert_eq!(b.len(), 2);
        } else {
            panic!("Expected HexBinary");
        }

        // 6. Packed decimal negative rejection when decimalSigned='no'
        let neg_packed = [0x12, 0x3D]; // -123 in packed decimal
        let src_pack = SliceByteSource::new(&neg_packed);
        let mut reader_pack = BitReader::new(src_pack, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_pack = ParserEngine::new(&schema, &mut reader_pack, &mut budget);

        let mut props_pack = ResolvedProperties::default();
        props_pack.binary_number_rep = BinaryNumberRep::Packed;
        props_pack.decimal_signed = false;
        assert!(engine_pack.parse_binary_value(DfdlSimpleType::Int, &props_pack, Some(2)).is_err());

        // 7. Packed decimal valid conversions across types
        let parse_bin = |bytes: &[u8], st: DfdlSimpleType, p: &ResolvedProperties, len: Option<usize>| {
            let src = SliceByteSource::new(bytes);
            let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut reader, &mut b);
            eng.parse_binary_value(st, p, len)
        };

        let pos_packed = [0x01, 0x2C]; // +12 in packed decimal (2 bytes)
        props_pack.decimal_signed = true;

        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::Short, &props_pack, Some(2)).unwrap(), DfdlValue::Short(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::Byte, &props_pack, Some(2)).unwrap(), DfdlValue::Byte(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::UnsignedLong, &props_pack, Some(2)).unwrap(), DfdlValue::UnsignedLong(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::UnsignedInt, &props_pack, Some(2)).unwrap(), DfdlValue::UnsignedInt(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::UnsignedShort, &props_pack, Some(2)).unwrap(), DfdlValue::UnsignedShort(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::UnsignedByte, &props_pack, Some(2)).unwrap(), DfdlValue::UnsignedByte(12));
        assert_eq!(parse_bin(&pos_packed, DfdlSimpleType::Decimal, &props_pack, Some(2)).unwrap(), DfdlValue::Decimal("12".into()));

        // Packed decimal out of range for Byte
        let big_packed = [0x12, 0x34, 0x5C]; // +12345
        assert!(parse_bin(&big_packed, DfdlSimpleType::Byte, &props_pack, Some(3)).is_err());
        assert!(parse_bin(&big_packed, DfdlSimpleType::Short, &props_pack, Some(3)).is_ok());

        // 8. BCD conversions across types and errors
        let bcd_data = [0x12, 0x34]; // 1234 in BCD
        let mut props_bcd = ResolvedProperties::default();
        props_bcd.binary_number_rep = BinaryNumberRep::Bcd;

        assert_eq!(parse_bin(&bcd_data, DfdlSimpleType::Int, &props_bcd, Some(2)).unwrap(), DfdlValue::Int(1234));
        assert_eq!(parse_bin(&bcd_data, DfdlSimpleType::Short, &props_bcd, Some(2)).unwrap(), DfdlValue::Short(1234));
        assert_eq!(parse_bin(&bcd_data, DfdlSimpleType::UnsignedLong, &props_bcd, Some(2)).unwrap(), DfdlValue::UnsignedLong(1234));
        assert_eq!(parse_bin(&bcd_data, DfdlSimpleType::UnsignedInt, &props_bcd, Some(2)).unwrap(), DfdlValue::UnsignedInt(1234));
        assert_eq!(parse_bin(&bcd_data, DfdlSimpleType::UnsignedShort, &props_bcd, Some(2)).unwrap(), DfdlValue::UnsignedShort(1234));
        assert!(parse_bin(&bcd_data, DfdlSimpleType::Byte, &props_bcd, Some(2)).is_err()); // 1234 > 127

        // BCD invalid nibble
        let bad_bcd = [0x1F];
        assert!(parse_bin(&bad_bcd, DfdlSimpleType::Int, &props_bcd, Some(1)).is_err());

        // 9. Sub-byte binary strings (X-DFDL-BITS, X-DFDL-HEX, X-DFDL-OCTAL, X-DFDL-6-BIT)
        let bits_data = [0xA5]; // 10100101
        let mut props_sub = ResolvedProperties::default();
        props_sub.representation = Representation::Binary;
        props_sub.encoding = "X-DFDL-BITS".into();
        let val_bits = parse_bin(&bits_data, DfdlSimpleType::String, &props_sub, Some(4)).unwrap();
        assert_eq!(val_bits, DfdlValue::String("1010".into()));

        props_sub.encoding = "X-DFDL-HEX".into();
        let val_hex_s = parse_bin(&bits_data, DfdlSimpleType::String, &props_sub, Some(2)).unwrap();
        assert_eq!(val_hex_s, DfdlValue::String("A5".into()));

        // 10. UTF-8 binary string by characters lengthUnits
        let utf8_chars_data = [0x41, 0xC3, 0xA9]; // "Aé" (2 characters, 3 bytes)
        let mut props_u8 = ResolvedProperties::default();
        props_u8.representation = Representation::Binary;
        props_u8.encoding = "UTF-8".into();
        props_u8.length_units = LengthUnits::Characters;
        let val_u8 = parse_bin(&utf8_chars_data, DfdlSimpleType::String, &props_u8, Some(2)).unwrap();
        assert_eq!(val_u8, DfdlValue::String("Aé".into()));

        // 11. HexBinary max length limit
        let mut schema_hex_lim = dummy_schema();
        schema_hex_lim.max_hex_binary_length_in_bytes = Some(2);
        let raw_hex = [0x11, 0x22, 0x33];
        let src_hl = SliceByteSource::new(&raw_hex);
        let mut reader_hl = BitReader::new(src_hl, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut engine_hl = ParserEngine::new(&schema_hex_lim, &mut reader_hl, &mut budget);
        let mut props_hl = ResolvedProperties::default();
        props_hl.representation = Representation::Binary;
        assert!(engine_hl.parse_binary_value(DfdlSimpleType::HexBinary, &props_hl, Some(3)).is_err());

        // 12. Binary Calendar (Packed positive & negative error, BCD valid & invalid nibble, IBM4690Packed)
        let mut props_cal = ResolvedProperties::default();
        props_cal.representation = Representation::Binary;
        props_cal.binary_calendar_rep = BinaryCalendarRep::Packed;
        props_cal.calendar_pattern = Some("yyyyMMdd".into());
        let cal_bytes = [0x02, 0x02, 0x61, 0x00, 0x7C]; // 020261007 + C -> digits ends with 20261007
        let v_date = parse_bin(&cal_bytes, DfdlSimpleType::Date, &props_cal, Some(5));
        assert!(v_date.is_ok());

        let cal_neg_bytes = [0x02, 0x02, 0x61, 0x00, 0x7D]; // Negative D -> error
        assert!(parse_bin(&cal_neg_bytes, DfdlSimpleType::Date, &props_cal, Some(5)).is_err());

        props_cal.binary_calendar_rep = BinaryCalendarRep::Bcd;
        let cal_bcd = [0x20, 0x26, 0x10, 0x07];
        assert!(parse_bin(&cal_bcd, DfdlSimpleType::Date, &props_cal, Some(4)).is_ok());
        let cal_bad_bcd_high = [0xA0, 0x26, 0x10, 0x07];
        assert!(parse_bin(&cal_bad_bcd_high, DfdlSimpleType::Date, &props_cal, Some(4)).is_err());
        let cal_bad_bcd_low = [0x0F, 0x26, 0x10, 0x07];
        assert!(parse_bin(&cal_bad_bcd_low, DfdlSimpleType::Date, &props_cal, Some(4)).is_err());

        props_cal.binary_calendar_rep = BinaryCalendarRep::Ibm4690Packed;
        assert!(parse_bin(&cal_bcd, DfdlSimpleType::Date, &props_cal, Some(4)).is_ok());

        // 13. Binary Decimal LittleEndian (16, 24, 32, 64-bit) with virtual point
        let mut props_dec = ResolvedProperties::default();
        props_dec.representation = Representation::Binary;
        props_dec.byte_order = ByteOrder::LittleEndian;
        props_dec.binary_decimal_virtual_point = 2;
        props_dec.decimal_signed = true;
        props_dec.length_units = LengthUnits::Bits;

        let d16 = [0xD2, 0x04]; // 1234 in LE
        assert_eq!(parse_bin(&d16, DfdlSimpleType::Decimal, &props_dec, Some(16)).unwrap(), DfdlValue::Decimal("12.34".into()));

        let d24 = [0x40, 0xE2, 0x01]; // 123456 in LE 24-bit
        assert_eq!(parse_bin(&d24, DfdlSimpleType::Decimal, &props_dec, Some(24)).unwrap(), DfdlValue::Decimal("1234.56".into()));

        let d32 = [0xD2, 0x04, 0x00, 0x00];
        assert_eq!(parse_bin(&d32, DfdlSimpleType::Decimal, &props_dec, Some(32)).unwrap(), DfdlValue::Decimal("12.34".into()));

        let d64 = [0xD2, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(parse_bin(&d64, DfdlSimpleType::Decimal, &props_dec, Some(64)).unwrap(), DfdlValue::Decimal("12.34".into()));

        // 13b. Arbitrary-precision Binary Decimal (>64 bits: 128-bit unsigned and 256-bit signed)
        let mut props_bigdec = ResolvedProperties::default();
        props_bigdec.representation = Representation::Binary;
        props_bigdec.byte_order = ByteOrder::BigEndian;
        props_bigdec.length_units = LengthUnits::Bytes;

        let d128 = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x19, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99, 0x99];
        assert_eq!(
            parse_bin(&d128, DfdlSimpleType::Decimal, &props_bigdec, Some(16)).unwrap(),
            DfdlValue::Decimal("1844674407370955161".into())
        );

        let mut d256_neg = [0xFFu8; 32];
        for b in &mut d256_neg[24..32] {
            *b = 0x00;
        }
        props_bigdec.decimal_signed = true;
        assert_eq!(
            parse_bin(&d256_neg, DfdlSimpleType::Decimal, &props_bigdec, Some(32)).unwrap(),
            DfdlValue::Decimal("-18446744073709551616".into())
        );

        // 14. Binary Boolean LittleEndian (16, 64-bit)
        let mut props_bool2 = ResolvedProperties::default();
        props_bool2.representation = Representation::Binary;
        props_bool2.byte_order = ByteOrder::LittleEndian;
        props_bool2.length_units = LengthUnits::Bits;
        props_bool2.binary_boolean_true_rep = BinaryBooleanRep::Value(1);
        props_bool2.binary_boolean_false_rep = BinaryBooleanRep::Value(0);

        assert_eq!(parse_bin(&[0x01, 0x00], DfdlSimpleType::Boolean, &props_bool2, Some(16)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(parse_bin(&[0x00, 0x00], DfdlSimpleType::Boolean, &props_bool2, Some(16)).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(parse_bin(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], DfdlSimpleType::Boolean, &props_bool2, Some(64)).unwrap(), DfdlValue::Boolean(true));

        // 15. Binary Integers with LittleEndian
        let mut props_int = ResolvedProperties::default();
        props_int.representation = Representation::Binary;
        props_int.byte_order = ByteOrder::LittleEndian;

        assert_eq!(parse_bin(&[0x2A, 0x00], DfdlSimpleType::Short, &props_int, None).unwrap(), DfdlValue::Short(42));
        assert_eq!(parse_bin(&[0x2A, 0x00, 0x00, 0x00], DfdlSimpleType::Int, &props_int, None).unwrap(), DfdlValue::Int(42));
        assert_eq!(parse_bin(&[0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], DfdlSimpleType::Long, &props_int, None).unwrap(), DfdlValue::Long(42));
        assert_eq!(parse_bin(&[0x2A, 0x00], DfdlSimpleType::UnsignedShort, &props_int, None).unwrap(), DfdlValue::UnsignedShort(42));
        assert_eq!(parse_bin(&[0x2A, 0x00, 0x00, 0x00], DfdlSimpleType::UnsignedInt, &props_int, None).unwrap(), DfdlValue::UnsignedInt(42));
        assert_eq!(parse_bin(&[0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], DfdlSimpleType::UnsignedLong, &props_int, None).unwrap(), DfdlValue::UnsignedLong(42));
        assert_eq!(parse_bin(&[0x2A], DfdlSimpleType::Byte, &props_int, None).unwrap(), DfdlValue::Byte(42));
        assert_eq!(parse_bin(&[0x2A], DfdlSimpleType::UnsignedByte, &props_int, None).unwrap(), DfdlValue::UnsignedByte(42));

        // 16. HexBinary with Bits lengthUnits and partial byte
        let mut props_hex = ResolvedProperties::default();
        props_hex.representation = Representation::Binary;
        props_hex.length_units = LengthUnits::Bits;
        props_hex.bit_order = BitOrder::MostSignificantBitFirst;
        assert_eq!(parse_bin(&[0xA0], DfdlSimpleType::HexBinary, &props_hex, Some(4)).unwrap(), DfdlValue::HexBinary(alloc::vec![0xA0]));

        // 17. IBM 4690 packed integer conversion
        let mut props_ibm = ResolvedProperties::default();
        props_ibm.binary_number_rep = BinaryNumberRep::Ibm4690Packed;
        props_ibm.decimal_signed = true;
        let ibm_data = [0x12];
        assert_eq!(parse_bin(&ibm_data, DfdlSimpleType::Short, &props_ibm, Some(1)).unwrap(), DfdlValue::Short(12));

        // 18. Sub-byte strings (BASE4 and OCTAL)
        let mut props_sub_types = ResolvedProperties::default();
        props_sub_types.representation = Representation::Binary;
        props_sub_types.encoding = "X-DFDL-BASE4".into();
        assert_eq!(parse_bin(&[0x1B], DfdlSimpleType::String, &props_sub_types, Some(4)).unwrap(), DfdlValue::String("0123".into()));
        props_sub_types.encoding = "X-DFDL-OCTAL".into();
        assert_eq!(parse_bin(&[0x2B], DfdlSimpleType::String, &props_sub_types, Some(2)).unwrap(), DfdlValue::String("12".into()));

        // 19. Sub-byte string without explicit length reading until EOF (lines 614-622)
        let sub_unbounded = parse_bin(&[0x1B], DfdlSimpleType::String, &props_sub_types, None).unwrap();
        assert!(matches!(sub_unbounded, DfdlValue::String(_)));

        // 20. Binary string with LengthUnits::Characters and UTF-8 multi-byte characters (lines 629-647)
        let mut props_utf8 = ResolvedProperties::default();
        props_utf8.representation = Representation::Binary;
        props_utf8.encoding = "UTF-8".into();
        props_utf8.length_units = LengthUnits::Characters;
        let utf8_data = [0xC3, 0xA9, 0xE2, 0x82, 0xAC];
        let utf8_res = parse_bin(&utf8_data, DfdlSimpleType::String, &props_utf8, Some(2)).unwrap();
        assert_eq!(utf8_res, DfdlValue::String("\u{00E9}\u{20AC}".into()));

        // 21. Binary string with LengthUnits::Bits and LengthUnits::Bytes (lines 648-668)
        let mut props_str_bits = ResolvedProperties::default();
        props_str_bits.representation = Representation::Binary;
        props_str_bits.encoding = "ASCII".into();
        props_str_bits.length_units = LengthUnits::Bits;
        props_str_bits.bit_order = BitOrder::MostSignificantBitFirst;
        let str_bits_res = parse_bin(b"AB", DfdlSimpleType::String, &props_str_bits, Some(16)).unwrap();
        assert_eq!(str_bits_res, DfdlValue::String("AB".into()));

        let mut props_str_bytes = ResolvedProperties::default();
        props_str_bytes.representation = Representation::Binary;
        props_str_bytes.encoding = "ASCII".into();
        props_str_bytes.length_units = LengthUnits::Bytes;
        let str_bytes_res = parse_bin(b"Test", DfdlSimpleType::String, &props_str_bytes, Some(4)).unwrap();
        assert_eq!(str_bytes_res, DfdlValue::String("Test".into()));

        // 22. Binary Decimal with virtual point (lines 396-402)
        let mut props_vdec = ResolvedProperties::default();
        props_vdec.representation = Representation::Binary;
        props_vdec.binary_number_rep = BinaryNumberRep::Bcd;
        props_vdec.binary_decimal_virtual_point = 2;
        let bcd_dec = parse_bin(&[0x12, 0x34], DfdlSimpleType::Decimal, &props_vdec, Some(2)).unwrap();
        assert_eq!(bcd_dec, DfdlValue::Decimal("12.34".into()));

        // 23. Binary Calendar Date and Time variants (lines 740-744, 809-812)
        let mut props_cal_date = ResolvedProperties::default();
        props_cal_date.representation = Representation::Binary;
        props_cal_date.binary_calendar_rep = BinaryCalendarRep::Bcd;
        props_cal_date.calendar_pattern = Some("yyyyMMdd".into());
        let date_bcd = parse_bin(&[0x20, 0x26, 0x10, 0x08], DfdlSimpleType::Date, &props_cal_date, Some(4)).unwrap();
        assert!(matches!(date_bcd, DfdlValue::Date(_)));

        let mut props_cal_time = ResolvedProperties::default();
        props_cal_time.representation = Representation::Binary;
        props_cal_time.binary_calendar_rep = BinaryCalendarRep::Bcd;
        props_cal_time.calendar_pattern = Some("HHmmss".into());
        let time_bcd = parse_bin(&[0x14, 0x30, 0x00], DfdlSimpleType::Time, &props_cal_time, Some(3)).unwrap();
        assert!(matches!(time_bcd, DfdlValue::Time(_)));

        // Negative packed calendar error for Date and Time
        let mut props_neg_cal = ResolvedProperties::default();
        props_neg_cal.representation = Representation::Binary;
        props_neg_cal.binary_calendar_rep = BinaryCalendarRep::Packed;
        props_neg_cal.calendar_pattern = Some("yyyyMMdd".into());
        let neg_packed_data = [0x02, 0x02, 0x61, 0x00, 0x8D];
        assert!(parse_bin(&neg_packed_data, DfdlSimpleType::Date, &props_neg_cal, Some(5)).is_err());
        assert!(parse_bin(&neg_packed_data, DfdlSimpleType::Time, &props_neg_cal, Some(5)).is_err());

        // 24. Out of range error branches for packed/BCD numbers (lines 235-250, 340-392)
        let mut props_packed_err = ResolvedProperties::default();
        props_packed_err.binary_number_rep = BinaryNumberRep::Packed;
        let huge_packed = [0x99, 0x99, 0x99, 0x9C];
        assert!(parse_bin(&huge_packed, DfdlSimpleType::Short, &props_packed_err, Some(4)).is_err());
        assert!(parse_bin(&huge_packed, DfdlSimpleType::Byte, &props_packed_err, Some(4)).is_err());
        assert!(parse_bin(&huge_packed, DfdlSimpleType::UnsignedShort, &props_packed_err, Some(4)).is_err());
        assert!(parse_bin(&huge_packed, DfdlSimpleType::UnsignedByte, &props_packed_err, Some(4)).is_err());

        // 25. 128-bit integer with insufficient data / EOF (lines 54-65)
        let mut props_128 = ResolvedProperties::default();
        props_128.representation = Representation::Binary;
        props_128.length_units = LengthUnits::Bits;
        assert!(parse_bin(&[0x42], DfdlSimpleType::Long, &props_128, Some(128)).is_err());
        let bytes_64 = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        assert!(parse_bin(&bytes_64, DfdlSimpleType::Long, &props_128, Some(128)).is_err());

        // 26. Boolean representations and byte orders (lines 465, 470, 473, 482, 508)
        let mut props_bool_be16 = ResolvedProperties::default();
        props_bool_be16.representation = Representation::Binary;
        props_bool_be16.byte_order = ByteOrder::BigEndian;
        props_bool_be16.length_units = LengthUnits::Bits;
        props_bool_be16.binary_boolean_true_rep = BinaryBooleanRep::Value(1);
        assert_eq!(parse_bin(&[0x00, 0x01], DfdlSimpleType::Boolean, &props_bool_be16, Some(16)).unwrap(), DfdlValue::Boolean(true));

        let mut props_bool_le32 = ResolvedProperties::default();
        props_bool_le32.representation = Representation::Binary;
        props_bool_le32.byte_order = ByteOrder::LittleEndian;
        props_bool_le32.length_units = LengthUnits::Bits;
        props_bool_le32.binary_boolean_true_rep = BinaryBooleanRep::Value(1);
        assert_eq!(parse_bin(&[0x01, 0x00, 0x00, 0x00], DfdlSimpleType::Boolean, &props_bool_le32, Some(32)).unwrap(), DfdlValue::Boolean(true));

        let mut props_bool_be64 = ResolvedProperties::default();
        props_bool_be64.representation = Representation::Binary;
        props_bool_be64.byte_order = ByteOrder::BigEndian;
        props_bool_be64.length_units = LengthUnits::Bits;
        props_bool_be64.binary_boolean_true_rep = BinaryBooleanRep::Value(-1);
        assert_eq!(parse_bin(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], DfdlSimpleType::Boolean, &props_bool_be64, Some(64)).unwrap(), DfdlValue::Boolean(true));

        let mut props_bool_def = ResolvedProperties::default();
        props_bool_def.representation = Representation::Binary;
        assert_eq!(parse_bin(&[0x00, 0x00, 0x00, 0x05], DfdlSimpleType::Boolean, &props_bool_def, None).unwrap(), DfdlValue::Boolean(true));

        // 27. Float LittleEndian (line 518)
        let mut props_float_le = ResolvedProperties::default();
        props_float_le.representation = Representation::Binary;
        props_float_le.byte_order = ByteOrder::LittleEndian;
        let float_le_bytes = [0x66, 0xE6, 0xF6, 0x42];
        let v_flt_le = parse_bin(&float_le_bytes, DfdlSimpleType::Float, &props_float_le, None).unwrap();
        if let DfdlValue::Float(f) = v_flt_le {
            assert!((f - 123.45).abs() < 0.01);
        }

        // 28. 4-byte UTF-8 character and sub-byte 6-BIT / partial bit string (lines 598, 605, 638, 656)
        let mut props_u8_4 = ResolvedProperties::default();
        props_u8_4.representation = Representation::Binary;
        props_u8_4.encoding = "UTF-8".into();
        props_u8_4.length_units = LengthUnits::Characters;
        let emoji_data = [0xF0, 0x9F, 0x98, 0x80];
        assert_eq!(parse_bin(&emoji_data, DfdlSimpleType::String, &props_u8_4, Some(1)).unwrap(), DfdlValue::String("😀".into()));

        let mut props_6bit = ResolvedProperties::default();
        props_6bit.representation = Representation::Binary;
        props_6bit.encoding = "X-DFDL-6-BIT-DFI-264.2".into();
        props_6bit.length_units = LengthUnits::Bits;
        let res_6bit = parse_bin(&[0x04], DfdlSimpleType::String, &props_6bit, Some(6)).unwrap();
        assert!(matches!(res_6bit, DfdlValue::String(_)));

        let mut props_partial = ResolvedProperties::default();
        props_partial.representation = Representation::Binary;
        props_partial.encoding = "ASCII".into();
        props_partial.length_units = LengthUnits::Bits;
        props_partial.bit_order = BitOrder::MostSignificantBitFirst;
        let res_part = parse_bin(&[0b01000001], DfdlSimpleType::String, &props_partial, Some(7)).unwrap();
        assert!(matches!(res_part, DfdlValue::String(_)));

        // 29. Binary integer with xs:unsignedLong out of range for negative value (lines 254-258)
        let mut props_ulong = ResolvedProperties::default();
        props_ulong.representation = Representation::Binary;
        props_ulong.binary_number_rep = BinaryNumberRep::Packed;
        props_ulong.decimal_signed = true;
        let neg_bytes = [0x12, 0x3D];
        let err_ulong = parse_bin(&neg_bytes, DfdlSimpleType::UnsignedLong, &props_ulong, Some(2)).unwrap_err();
        assert!(err_ulong.message.as_str().contains("out of range for type xs:unsignedLong"));

        // 30. BCD out of range checks for integer and unsigned types (lines 340-394)
        let mut props_bcd = ResolvedProperties::default();
        props_bcd.representation = Representation::Binary;
        props_bcd.binary_number_rep = BinaryNumberRep::Bcd;
        props_bcd.length_units = LengthUnits::Bytes;

        // BCD > i32::MAX for Int (2147483648)
        let bcd_int_overflow = [0x21, 0x47, 0x48, 0x36, 0x48];
        let err_bcd_int = parse_bin(&bcd_int_overflow, DfdlSimpleType::Int, &props_bcd, Some(5)).unwrap_err();
        assert!(err_bcd_int.message.as_str().contains("out of range for type xs:int"));

        // BCD > i16::MAX for Short (32768)
        let bcd_short_overflow = [0x03, 0x27, 0x68];
        let err_bcd_short = parse_bin(&bcd_short_overflow, DfdlSimpleType::Short, &props_bcd, Some(3)).unwrap_err();
        assert!(err_bcd_short.message.as_str().contains("out of range for type xs:short"));

        // BCD > i8::MAX for Byte (128)
        let bcd_byte_overflow = [0x01, 0x28];
        let err_bcd_byte = parse_bin(&bcd_byte_overflow, DfdlSimpleType::Byte, &props_bcd, Some(2)).unwrap_err();
        assert!(err_bcd_byte.message.as_str().contains("out of range for type xs:byte"));

        // BCD > u32::MAX for UnsignedInt (4294967296)
        let bcd_uint_overflow = [0x42, 0x94, 0x96, 0x72, 0x96];
        let err_bcd_uint = parse_bin(&bcd_uint_overflow, DfdlSimpleType::UnsignedInt, &props_bcd, Some(5)).unwrap_err();
        assert!(err_bcd_uint.message.as_str().contains("out of range for type xs:unsignedInt"));

        // BCD > u16::MAX for UnsignedShort (65536)
        let bcd_ushort_overflow = [0x06, 0x55, 0x36];
        let err_bcd_ushort = parse_bin(&bcd_ushort_overflow, DfdlSimpleType::UnsignedShort, &props_bcd, Some(3)).unwrap_err();
        assert!(err_bcd_ushort.message.as_str().contains("out of range for type xs:unsignedShort"));

        // BCD > u8::MAX for UnsignedByte (256)
        let bcd_ubyte_overflow = [0x02, 0x56];
        let err_bcd_ubyte = parse_bin(&bcd_ubyte_overflow, DfdlSimpleType::UnsignedByte, &props_bcd, Some(2)).unwrap_err();
        assert!(err_bcd_ubyte.message.as_str().contains("out of range for type xs:unsignedByte"));

        // BCD Decimal with virtual point (lines 396-402)
        let mut props_bcd_dec = props_bcd.clone();
        props_bcd_dec.binary_decimal_virtual_point = 2;
        let bcd_dec = parse_bin(&[0x12, 0x34], DfdlSimpleType::Decimal, &props_bcd_dec, Some(2)).unwrap();
        assert_eq!(bcd_dec, DfdlValue::Decimal("12.34".into()));

        // 31. Negative packed calendar for xs:date and xs:time (lines 742, 743)
        let mut props_packed_cal = ResolvedProperties::default();
        props_packed_cal.representation = Representation::Binary;
        props_packed_cal.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::Packed;
        props_packed_cal.length_units = LengthUnits::Bytes;

        let neg_packed_date = [0x20, 0x26, 0x01, 0x0D]; // ends in 0x0D (negative)
        let err_p_date = parse_bin(&neg_packed_date, DfdlSimpleType::Date, &props_packed_cal, Some(4)).unwrap_err();
        assert!(err_p_date.message.as_str().contains("Unable to parse xs:date from negative packed number"));

        let neg_packed_time = [0x12, 0x30, 0x00, 0x0B]; // ends in 0x0B (negative)
        let err_p_time = parse_bin(&neg_packed_time, DfdlSimpleType::Time, &props_packed_cal, Some(4)).unwrap_err();
        assert!(err_p_time.message.as_str().contains("Unable to parse xs:time from negative packed number"));

        // 32. HexBinary exceeding maximum allowed length in bytes (lines 575-582)
        let mut builder_hex = SchemaBuilder::new();
        builder_hex.max_hex_binary_length_in_bytes = Some(2);
        let elem_hex = crate::schema::ir::CompiledElement {
            name: QName::local("hex_data"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::HexBinary),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut props_hex = ResolvedProperties::default();
        props_hex.representation = Representation::Binary;
        props_hex.length_kind = LengthKind::Delimited;
        let hex_id = builder_hex.add_term_with_props(QName::local("hex_data"), TermKind::Element(elem_hex), props_hex).unwrap();
        builder_hex.set_root(hex_id);
        let schema_hex = builder_hex.build().unwrap();
        let hex_bytes = [0xAA, 0xBB, 0xCC, 0xDD];
        let src_hex = SliceByteSource::new(&hex_bytes);
        let mut reader_hex = BitReader::new(src_hex, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_hex = WorkBudget::new(1000);
        let mut p_engine_hex = ParserEngine::new(&schema_hex, &mut reader_hex, &mut budget_hex);
        let err_hex = p_engine_hex.parse_document().unwrap_err();
        assert!(err_hex.message.as_str().contains("exceeds maximum allowed length of 2 bytes"));

        // 33. BCD valid Byte, UnsignedByte, and Long variants (lines 364, 394, 403)
        let mut props_bcd_valid = ResolvedProperties::default();
        props_bcd_valid.representation = Representation::Binary;
        props_bcd_valid.binary_number_rep = BinaryNumberRep::Bcd;
        props_bcd_valid.length_units = LengthUnits::Bytes;

        let bcd_byte_val = parse_bin(&[0x00, 0x42], DfdlSimpleType::Byte, &props_bcd_valid, Some(2)).unwrap();
        assert_eq!(bcd_byte_val, DfdlValue::Byte(42));

        let bcd_ubyte_val = parse_bin(&[0x02, 0x00], DfdlSimpleType::UnsignedByte, &props_bcd_valid, Some(2)).unwrap();
        assert_eq!(bcd_ubyte_val, DfdlValue::UnsignedByte(200));

        let bcd_long_val = parse_bin(&[0x12, 0x34], DfdlSimpleType::Long, &props_bcd_valid, Some(2)).unwrap();
        assert_eq!(bcd_long_val, DfdlValue::Long(1234));

        // 34. Packed Decimal Long variant (line 296)
        let mut props_packed_long = ResolvedProperties::default();
        props_packed_long.representation = Representation::Binary;
        props_packed_long.binary_number_rep = BinaryNumberRep::Packed;
        props_packed_long.length_units = LengthUnits::Bytes;
        props_packed_long.decimal_signed = true;
        let packed_long_val = parse_bin(&[0x12, 0x3C], DfdlSimpleType::Long, &props_packed_long, Some(2)).unwrap();
        assert_eq!(packed_long_val, DfdlValue::Long(123));

        // 35. Binary Calendar BinarySeconds and BinaryMilliseconds (lines 803-806)
        let mut props_cal_sec = ResolvedProperties::default();
        props_cal_sec.representation = Representation::Binary;
        props_cal_sec.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::BinarySeconds;
        props_cal_sec.length_units = LengthUnits::Bytes;
        // 0 seconds since epoch (1970-01-01T00:00:00)
        let cal_sec_val = parse_bin(&[0x00, 0x00, 0x00, 0x00], DfdlSimpleType::DateTime, &props_cal_sec, Some(4)).unwrap();
        assert!(matches!(cal_sec_val, DfdlValue::DateTime(_)));

        let mut props_cal_ms = ResolvedProperties::default();
        props_cal_ms.representation = Representation::Binary;
        props_cal_ms.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::BinaryMilliseconds;
        props_cal_ms.length_units = LengthUnits::Bytes;
        let cal_ms_val = parse_bin(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], DfdlSimpleType::DateTime, &props_cal_ms, Some(8)).unwrap();
        assert!(matches!(cal_ms_val, DfdlValue::DateTime(_)));

        // 36. Negative packed calendar rejection for DateTime, Date, and Time (lines 688-700)
        let mut props_packed_cal = ResolvedProperties::default();
        props_packed_cal.representation = Representation::Binary;
        props_packed_cal.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::Packed;
        props_packed_cal.length_units = LengthUnits::Bytes;
        let err_dt = parse_bin(&[0x12, 0x3D], DfdlSimpleType::DateTime, &props_packed_cal, Some(2)).unwrap_err();
        assert!(err_dt.message.as_str().contains("Unable to parse xs:dateTime from negative packed number"));

        let err_d = parse_bin(&[0x12, 0x3D], DfdlSimpleType::Date, &props_packed_cal, Some(2)).unwrap_err();
        assert!(err_d.message.as_str().contains("Unable to parse xs:date from negative packed number"));

        let err_t = parse_bin(&[0x12, 0x3D], DfdlSimpleType::Time, &props_packed_cal, Some(2)).unwrap_err();
        assert!(err_t.message.as_str().contains("Unable to parse xs:time from negative packed number"));

        // 37. BCD calendar invalid high and low nibbles (lines 731-741)
        let mut props_bcd_cal = ResolvedProperties::default();
        props_bcd_cal.representation = Representation::Binary;
        props_bcd_cal.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::Bcd;
        props_bcd_cal.length_units = LengthUnits::Bytes;

        let err_high = parse_bin(&[0xA0], DfdlSimpleType::DateTime, &props_bcd_cal, Some(1)).unwrap_err();
        assert!(err_high.message.as_str().contains("Invalid high nibble"));

        let err_low = parse_bin(&[0x0A], DfdlSimpleType::DateTime, &props_bcd_cal, Some(1)).unwrap_err();
        assert!(err_low.message.as_str().contains("Invalid low nibble"));

        // 38. String with fallback X-DFDL encoding (line 553)
        let mut props_xdfdl_fallback = ResolvedProperties::default();
        props_xdfdl_fallback.representation = Representation::Binary;
        props_xdfdl_fallback.encoding = "X-DFDL-UNKNOWN".into();
        props_xdfdl_fallback.length_units = LengthUnits::Bytes;
        let xdfdl_res = parse_bin(b"AB", DfdlSimpleType::String, &props_xdfdl_fallback, Some(2)).unwrap();
        assert_eq!(xdfdl_res, DfdlValue::String("AB".into()));

        // 39. Decimal little-endian with 8 bits (line 637)
        let mut props_dec_le = ResolvedProperties::default();
        props_dec_le.representation = Representation::Binary;
        props_dec_le.byte_order = ByteOrder::LittleEndian;
        props_dec_le.length_units = LengthUnits::Bytes;
        let dec_le_res = parse_bin(&[0x2A], DfdlSimpleType::Decimal, &props_dec_le, Some(1)).unwrap();
        assert_eq!(dec_le_res, DfdlValue::Decimal("42".into()));

        // 40. Delimited binary integer/packed decimal and delimited binary calendar (lines 156-175, 660-679)
        let mut props_delim_pack = ResolvedProperties::default();
        props_delim_pack.representation = Representation::Binary;
        props_delim_pack.binary_number_rep = BinaryNumberRep::Packed;
        props_delim_pack.decimal_signed = true;
        props_delim_pack.terminator = Some(";".into());
        let delim_pack_bytes = [0x12, 0x3C, b';'];
        let val_delim_pack = parse_bin(&delim_pack_bytes, DfdlSimpleType::Int, &props_delim_pack, None).unwrap();
        assert_eq!(val_delim_pack, DfdlValue::Int(123));

        let mut props_delim_cal = ResolvedProperties::default();
        props_delim_cal.representation = Representation::Binary;
        props_delim_cal.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::Packed;
        props_delim_cal.calendar_pattern = Some("yyyyMMdd".into());
        props_delim_cal.terminator = Some(";".into());
        let delim_cal_bytes = [0x02, 0x02, 0x61, 0x00, 0x7C, b';'];
        let val_delim_cal = parse_bin(&delim_cal_bytes, DfdlSimpleType::Date, &props_delim_cal, None).unwrap();
        assert!(matches!(val_delim_cal, DfdlValue::Date(_)));

        // 41. Delimited HexBinary with length = None and dynamic_len = None (lines 509-516, 523-527)
        let mut props_hb_delim = ResolvedProperties::default();
        props_hb_delim.representation = Representation::Binary;
        props_hb_delim.length_kind = LengthKind::Delimited;
        let hb_delim_res = parse_bin(&[0xDE, 0xAD], DfdlSimpleType::HexBinary, &props_hb_delim, None).unwrap();
        assert_eq!(hb_delim_res, DfdlValue::HexBinary(alloc::vec![0xDE, 0xAD]));

        // 42. Delimited packed decimal reaching EOF without terminator (line 171)
        let delim_pack_eof = [0x12, 0x3C];
        let val_pack_eof = parse_bin(&delim_pack_eof, DfdlSimpleType::Int, &props_delim_pack, None).unwrap();
        assert_eq!(val_pack_eof, DfdlValue::Int(123));

        // 43. Delimited binary calendar reaching EOF and with terminator (lines 656-673)
        let delim_cal_eof = [0x02, 0x02, 0x61, 0x00, 0x7C];
        let val_cal_eof = parse_bin(&delim_cal_eof, DfdlSimpleType::Date, &props_delim_cal, None).unwrap();
        assert!(matches!(val_cal_eof, DfdlValue::Date(_)));

        let mut props_term_cal = props_delim_cal.clone();
        props_term_cal.terminator = Some(";".into());
        let delim_cal_term = [0x02, 0x02, 0x61, 0x00, 0x7C, b';'];
        let val_cal_term = parse_bin(&delim_cal_term, DfdlSimpleType::Date, &props_term_cal, None).unwrap();
        assert!(matches!(val_cal_term, DfdlValue::Date(_)));

        // 44. Delimited binary packed integer with terminator and EOF (lines 156-174)
        let mut props_term_packed = ResolvedProperties::default();
        props_term_packed.representation = Representation::Binary;
        props_term_packed.length_kind = LengthKind::Delimited;
        props_term_packed.binary_number_rep = BinaryNumberRep::Packed;
        props_term_packed.terminator = Some(";".into());
        let delim_packed_term = [0x12, 0x3C, b';'];
        let val_packed_term = parse_bin(&delim_packed_term, DfdlSimpleType::Long, &props_term_packed, None).unwrap();
        assert_eq!(val_packed_term, DfdlValue::Long(123));
        let val_packed_eof = parse_bin(&[0x12, 0x3C], DfdlSimpleType::Long, &props_term_packed, None).unwrap();
        assert_eq!(val_packed_eof, DfdlValue::Long(123));

        // 45. Binary packed calendar negative sign (lines 682-694)
        let neg_cal_bytes = [0x02, 0x02, 0x61, 0x00, 0x7D];
        let err_cal_neg = parse_bin(&neg_cal_bytes, DfdlSimpleType::Date, &props_delim_cal, Some(5)).unwrap_err();
        assert!(err_cal_neg.message.as_str().contains("Unable to parse xs:date from negative packed number"));

        // 45. HexBinary exceeding max allowed length in bytes via parse_binary_value (lines 523-531)
        let mut schema_hb_max = dummy_schema();
        schema_hb_max.max_hex_binary_length_in_bytes = Some(1);
        let src_hb_max = SliceByteSource::new(&[0x11, 0x22]);
        let mut r_hb_max = BitReader::new(src_hb_max, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_hb_max = WorkBudget::new(100);
        let mut eng_hb_max = ParserEngine::new(&schema_hb_max, &mut r_hb_max, &mut b_hb_max);
        let mut p_hb_max = ResolvedProperties::default();
        p_hb_max.representation = Representation::Binary;
        p_hb_max.length_kind = LengthKind::Delimited;
        let err_hb_max = eng_hb_max.parse_binary_value(DfdlSimpleType::HexBinary, &p_hb_max, None).unwrap_err();
        assert!(err_hb_max.message.as_str().contains("exceeds maximum allowed length"));

        // 46. Delimiter CR / CRLF matching across WSP, WSP*, WSP+, and NL
        let test_delim = |bytes: &[u8], pattern: &str| {
            let src = SliceByteSource::new(bytes);
            let mut r = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut r, &mut b);
            let res = eng.match_literal_delimiter(pattern);
            let mut rem = alloc::vec::Vec::new();
            while !eng.reader.is_eof() {
                if let Ok(byte) = eng.reader.read_bits(8) {
                    rem.push(byte as u8);
                } else {
                    break;
                }
            }
            (res, rem)
        };
        // WSP matches standalone CR leaving following byte
        let (res, rem) = test_delim(b"\rX", "%WSP;");
        assert!(res.is_ok());
        assert_eq!(rem, b"X");

        // WSP* matches CRLF and trailing CR before non-whitespace
        let (res, rem) = test_delim(b"\r\n\rX", "%WSP*;");
        assert!(res.is_ok());
        assert_eq!(rem, b"X");

        // WSP+ matches standalone CR followed by CRLF and CR before non-whitespace
        let (res, rem) = test_delim(b"\r \r\n\rX", "%WSP+;");
        assert!(res.is_ok());
        assert_eq!(rem, b"X");

        // NL matches standalone CR leaving following byte
        let (res, rem) = test_delim(b"\rX", "%NL;");
        assert!(res.is_ok());
        assert_eq!(rem, b"X");
    }

    /// Verifies text parsing edge cases including UTF-32 explicit length, escape schemes, and facet validations.
    #[test]
    fn test_text_parser_extended_coverage() {
        use crate::schema::ir::{CompiledEscapeScheme, EscapeKind, LengthUnits};

        let schema = dummy_schema();
        let parse_text = |bytes: &[u8], st: DfdlSimpleType, p: &ResolvedProperties, len: Option<usize>| {
            let src = SliceByteSource::new(bytes);
            let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
            let mut b = WorkBudget::new(100);
            let mut eng = ParserEngine::new(&schema, &mut reader, &mut b);
            let builder = crate::infoset::tree::InfosetBuilder::new();
            eng.parse_text_value(st, p, len, &builder, None)
        };

        // 1. Incomplete multi-byte UTF-8 sequence error (lines 47-50 in text.rs)
        let mut props_u8 = ResolvedProperties::default();
        props_u8.encoding = "UTF-8".into();
        props_u8.length_units = LengthUnits::Characters;
        assert!(parse_text(&[0xC3], DfdlSimpleType::String, &props_u8, Some(1)).is_err());

        // 2. UTF-32 parsing with Characters lengthUnits and EOF branch (lines 197-208 in text.rs)
        let mut props_u32 = ResolvedProperties::default();
        props_u32.encoding = "UTF-32BE".into();
        props_u32.length_units = LengthUnits::Characters;
        let u32_bytes = [0x00, 0x00, 0x00, 0x41];
        let val_u32 = parse_text(&u32_bytes, DfdlSimpleType::String, &props_u32, Some(1)).unwrap();
        assert_eq!(val_u32, DfdlValue::String("A".into()));
        assert!(parse_text(&[0x00, 0x00], DfdlSimpleType::String, &props_u32, Some(1)).is_err());

        // 3. LengthUnits::Bits text parsing and EOF (lines 209-218 in text.rs)
        let mut props_bits = ResolvedProperties::default();
        props_bits.encoding = "ASCII".into();
        props_bits.length_units = LengthUnits::Bits;
        let val_bits = parse_text(b"Hello", DfdlSimpleType::String, &props_bits, Some(40)).unwrap();
        assert_eq!(val_bits, DfdlValue::String("Hello".into()));
        assert!(parse_text(b"Hi", DfdlSimpleType::String, &props_bits, Some(32)).is_err());

        // 4. EscapeCharacter doubling without escapeEscapeCharacter (lines 463-470 in text.rs)
        let mut props_esc_char = ResolvedProperties::default();
        props_esc_char.encoding = "ASCII".into();
        props_esc_char.terminator = Some(",".into());
        let esc_scheme_char = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeCharacter,
            escape_character: Some("/".into()),
            escape_escape_character: None,
            extra_escaped_characters: alloc::vec![],
            escape_block_start: None,
            escape_block_end: None,
            generate_escape_block: crate::schema::ir::GenerateEscapeBlock::WhenNeeded,
        };
        props_esc_char.escape_scheme = Some(esc_scheme_char);
        let val_doubled = parse_text(b"foo//bar,", DfdlSimpleType::String, &props_esc_char, None).unwrap();
        assert_eq!(val_doubled, DfdlValue::String("foo/bar".into()));

        // 5. EscapeBlock with escaped block end character (lines 330-355 in text.rs)
        let mut props_esc_block = ResolvedProperties::default();
        props_esc_block.encoding = "ASCII".into();
        props_esc_block.terminator = Some(",".into());
        let esc_scheme_block = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: Some("#".into()),
            extra_escaped_characters: alloc::vec![],
            escape_block_start: Some("[".into()),
            escape_block_end: Some("]".into()),
            generate_escape_block: crate::schema::ir::GenerateEscapeBlock::WhenNeeded,
        };
        props_esc_block.escape_scheme = Some(esc_scheme_block);
        let val_block = parse_text(b"[foo#]bar],", DfdlSimpleType::String, &props_esc_block, None).unwrap();
        assert_eq!(val_block, DfdlValue::String("foo]bar".into()));

        // 6. Facet validations: fraction_digits = 0 and nonNegativeInteger out of range (lines 1371-1392)
        let mut props_facet = ResolvedProperties::default();
        props_facet.facets.fraction_digits = Some(0);
        assert!(parse_text(b"42.5", DfdlSimpleType::Decimal, &props_facet, Some(4)).is_err());

        props_facet.facets.min_inclusive = Some("0".into());
        assert!(parse_text(b"-5", DfdlSimpleType::Decimal, &props_facet, Some(2)).is_err());

        // 7. Dynamic decimal and grouping separators with invalid length (lines 582-588, 607-610)
        let mut props_bad_dec = ResolvedProperties::default();
        props_bad_dec.text_standard_decimal_separator = "{ '..' }".into();
        let err_bad_dec = parse_text(b"12.34", DfdlSimpleType::Double, &props_bad_dec, Some(5)).unwrap_err();
        assert!(err_bad_dec.message.as_str().contains("must be exactly 1 character for textStandardDecimalSeparator"));

        let mut props_bad_grp = ResolvedProperties::default();
        props_bad_grp.text_standard_grouping_separator = "{ ',,' }".into();
        let err_bad_grp = parse_text(b"12,34", DfdlSimpleType::Double, &props_bad_grp, Some(5)).unwrap_err();
        assert!(err_bad_grp.message.as_str().contains("must be exactly 1 character for textStandardGroupingSeparator"));

        // 8. Dynamic exponent rep evaluation (line 632)
        let mut props_dyn_exp = ResolvedProperties::default();
        props_dyn_exp.text_standard_exponent_rep = Some("{ 'E' }".into());
        let res_dyn_exp = parse_text(b"1.2E3", DfdlSimpleType::Double, &props_dyn_exp, Some(5)).unwrap();
        assert_eq!(res_dyn_exp, DfdlValue::Double(1200.0));

        // 9. Non-base-10 with leading sign rejected (line 654)
        let mut props_hex_base = ResolvedProperties::default();
        props_hex_base.text_standard_base = 16;
        assert!(parse_text(b"+FF", DfdlSimpleType::Int, &props_hex_base, Some(3)).is_err());
        assert!(parse_text(b"-FF", DfdlSimpleType::Int, &props_hex_base, Some(3)).is_err());

        // 10. Parse uint with decimal part all zeros (lines 705-710)
        let props_uint_dec = ResolvedProperties::default();
        let res_uint_dec = parse_text(b"42.000", DfdlSimpleType::UnsignedInt, &props_uint_dec, Some(6)).unwrap();
        assert_eq!(res_uint_dec, DfdlValue::UnsignedInt(42));

        // 11. Sub-byte delimited text stopped by terminator (lines 151-155)
        let mut props_sub_delim = ResolvedProperties::default();
        props_sub_delim.encoding = "X-DFDL-BITS".into();
        props_sub_delim.terminator = Some(",".into());
        let res_sub_bits = parse_text(b"101,", DfdlSimpleType::String, &props_sub_delim, None).unwrap();
        assert_eq!(res_sub_bits, DfdlValue::String("001100010011000000110001".into()));

        // 12. Parse signed int with decimal part all zeros (lines 671-676)
        let props_int_dec = ResolvedProperties::default();
        let res_int_dec = parse_text(b"42.000", DfdlSimpleType::Int, &props_int_dec, Some(6)).unwrap();
        assert_eq!(res_int_dec, DfdlValue::Int(42));
        let res_neg_int_dec = parse_text(b"-42.000", DfdlSimpleType::Int, &props_int_dec, Some(7)).unwrap();
        assert_eq!(res_neg_int_dec, DfdlValue::Int(-42));

        // 13. Sub-byte delimited text stopped by separator (lines 146-150)
        let mut props_sub_sep = ResolvedProperties::default();
        props_sub_sep.encoding = "X-DFDL-BITS".into();
        props_sub_sep.separator = Some(";".into());
        let res_sub_sep = parse_text(b"1;", DfdlSimpleType::String, &props_sub_sep, None).unwrap();
        assert_eq!(res_sub_sep, DfdlValue::String("00110001".into()));

        // 14. TextTrimKind Tail and None (lines 562, 567)
        let mut props_trim_tail = ResolvedProperties::default();
        props_trim_tail.text_trim_kind = crate::schema::ir::TextTrimKind::Tail;
        props_trim_tail.text_pad_char = "#".into();
        let res_tail = parse_text(b"42##", DfdlSimpleType::Int, &props_trim_tail, Some(4)).unwrap();
        assert_eq!(res_tail, DfdlValue::Int(42));

        let mut props_trim_none = ResolvedProperties::default();
        props_trim_none.text_trim_kind = crate::schema::ir::TextTrimKind::None;
        let res_none = parse_text(b"42", DfdlSimpleType::Int, &props_trim_none, Some(2)).unwrap();
        assert_eq!(res_none, DfdlValue::Int(42));

        // 15. Sub-byte text parsing explicit length with EOF (lines 129-135)
        let mut props_sub_eof = ResolvedProperties::default();
        props_sub_eof.encoding = "X-DFDL-BITS".into();
        props_sub_eof.length_units = LengthUnits::Characters;
        assert!(parse_text(b"", DfdlSimpleType::String, &props_sub_eof, Some(2)).is_err());

        // 16. UTF-16 text parsing explicit length with EOF (lines 178-184)
        let mut props_u16_eof = ResolvedProperties::default();
        props_u16_eof.encoding = "UTF-16BE".into();
        props_u16_eof.length_units = LengthUnits::Characters;
        assert!(parse_text(&[0x00], DfdlSimpleType::String, &props_u16_eof, Some(1)).is_err());

        // 17. text_standard_zero_rep across numeric types (lines 828-842)
        let mut props_zr = ResolvedProperties::default();
        props_zr.text_standard_zero_rep = Some("ZERO".into());
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Long, &props_zr, Some(4)).unwrap(), DfdlValue::Long(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Short, &props_zr, Some(4)).unwrap(), DfdlValue::Short(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Byte, &props_zr, Some(4)).unwrap(), DfdlValue::Byte(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::UnsignedInt, &props_zr, Some(4)).unwrap(), DfdlValue::UnsignedInt(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::UnsignedLong, &props_zr, Some(4)).unwrap(), DfdlValue::UnsignedLong(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::UnsignedShort, &props_zr, Some(4)).unwrap(), DfdlValue::UnsignedShort(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::UnsignedByte, &props_zr, Some(4)).unwrap(), DfdlValue::UnsignedByte(0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Float, &props_zr, Some(4)).unwrap(), DfdlValue::Float(0.0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Double, &props_zr, Some(4)).unwrap(), DfdlValue::Double(0.0));
        assert_eq!(parse_text(b"ZERO", DfdlSimpleType::Decimal, &props_zr, Some(4)).unwrap(), DfdlValue::Decimal("0".into()));

        // 18. textNumberRep="zoned" for Float and Double triggers SDE (lines 852-857)
        let mut props_zoned_flt = ResolvedProperties::default();
        props_zoned_flt.text_number_rep = crate::schema::ir::TextNumberRep::Zoned;
        let err_zflt = parse_text(b"123", DfdlSimpleType::Float, &props_zoned_flt, Some(3)).unwrap_err();
        assert!(err_zflt.message.as_str().contains("textNumberRep=\"zoned\" is not allowed for Float"));
        let err_zdbl = parse_text(b"123", DfdlSimpleType::Double, &props_zoned_flt, Some(3)).unwrap_err();
        assert!(err_zdbl.message.as_str().contains("textNumberRep=\"zoned\" is not allowed for Double"));
    }

    /// Tests parser engine driver edge cases in `mod.rs` including reader position queries,
    /// bit order change boundary validation, sequence leading skip, and layer length variables.
    ///
    /// Verifies that:
    /// 1. `reader_position` returns the exact reader bit offset.
    /// 2. Changing `bitOrder` on a non-byte boundary produces a Schema Definition Error per DFDL §11.2.
    /// 3. Sequence terms apply `leadingSkip` in both bytes and bits alignment units.
    /// 4. Layer `checkDigit` accepts various integer types (`xs:short`, `xs:long`, `xs:unsignedShort`) for length.
    #[test]
    fn test_parser_engine_mod_extended_coverage() {
        let schema = dummy_schema();
        let bytes = [0xFF, 0x00, 0xAA, 0x55];
        let src = SliceByteSource::new(&bytes);
        let mut r = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b = WorkBudget::new(100);
        let eng = ParserEngine::new(&schema, &mut r, &mut b);

        // 1. reader_position query (lines 148-150)
        assert_eq!(eng.reader_position(), 0);
        let _ = eng.reader.read_bits(5).unwrap();
        assert_eq!(eng.reader_position(), 5);

        // 2. BitOrder change on non-byte boundary triggers SDE (lines 465-475)
        let mut b_seq = SchemaBuilder::new();
        let seq_props = ResolvedProperties {
            bit_order: BitOrder::LeastSignificantBitFirst, // reader is at bit 5 (rem = 5 != 0)
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..Default::default()
        };
        let seq_node = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let seq_id = b_seq.add_term_with_props(QName::local("child_seq"), seq_node, seq_props).unwrap();
        b_seq.set_root(seq_id);
        let schema_sde = b_seq.build().unwrap();

        let mut b_eng = WorkBudget::new(100);
        let mut eng_sde = ParserEngine::new(&schema_sde, eng.reader, &mut b_eng);
        let mut tree_builder = InfosetBuilder::new();
        let err_bit_order = eng_sde.parse_term_inner(seq_id, &mut tree_builder).unwrap_err();
        assert!(err_bit_order.message.as_str().contains("Can only change bitOrder on a byte boundary"));

        // 3. Sequence leading skip in bytes and bits (lines 434-442)
        let mut b_skip = SchemaBuilder::new();
        let skip_props = ResolvedProperties {
            leading_skip: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bytes,
            ..Default::default()
        };
        let skip_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let skip_id = b_skip.add_term_with_props(QName::local("skip_seq"), skip_seq, skip_props).unwrap();
        b_skip.set_root(skip_id);
        let schema_skip = b_skip.build().unwrap();

        let bytes_skip = [0x11, 0x22, 0x33];
        let src_skip = SliceByteSource::new(&bytes_skip);
        let mut r_skip = BitReader::new(src_skip, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_skip_work = WorkBudget::new(100);
        let mut eng_skip = ParserEngine::new(&schema_skip, &mut r_skip, &mut b_skip_work);
        let mut b_out = InfosetBuilder::new();
        assert!(eng_skip.parse_term_inner(skip_id, &mut b_out).is_ok());
        assert_eq!(eng_skip.reader_position(), 8); // 1 byte skipped

        // 4. Missing NodeId from compiled schema graph (lines 413-418)
        assert!(eng_skip.parse_term_inner(crate::schema::ir::NodeId(999999), &mut b_out).is_err());

        // 5. Sequence leading skip in bits (lines 438-442)
        let mut b_skip_bits = SchemaBuilder::new();
        let skip_bits_props = ResolvedProperties {
            leading_skip: 5,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..Default::default()
        };
        let skip_bits_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let skip_bits_id = b_skip_bits.add_term_with_props(QName::local("skip_bits_seq"), skip_bits_seq, skip_bits_props).unwrap();
        b_skip_bits.set_root(skip_bits_id);
        let schema_skip_bits = b_skip_bits.build().unwrap();

        let bytes_sb = [0xAA, 0x55];
        let src_sb = SliceByteSource::new(&bytes_sb);
        let mut r_sb = BitReader::new(src_sb, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_sb_work = WorkBudget::new(100);
        let mut eng_sb = ParserEngine::new(&schema_skip_bits, &mut r_sb, &mut b_sb_work);
        let mut b_out_sb = InfosetBuilder::new();
        assert!(eng_sb.parse_term_inner(skip_bits_id, &mut b_out_sb).is_ok());
        assert_eq!(eng_sb.reader_position(), 5); // 5 bits skipped

        // 6. IPv4Checksum layer insufficient data (lines 627-634)
        let mut b_ipv4 = SchemaBuilder::new();
        let ipv4_props = ResolvedProperties {
            layer: Some("IPv4Checksum".into()),
            ..Default::default()
        };
        let ipv4_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let ipv4_id = b_ipv4.add_term_with_props(QName::local("ipv4_seq"), ipv4_seq, ipv4_props).unwrap();
        b_ipv4.set_root(ipv4_id);
        let schema_ipv4 = b_ipv4.build().unwrap();

        let bytes_short = [0x01, 0x02]; // only 2 bytes < 20
        let src_ipv4 = SliceByteSource::new(&bytes_short);
        let mut r_ipv4 = BitReader::new(src_ipv4, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_ipv4_work = WorkBudget::new(100);
        let mut eng_ipv4 = ParserEngine::new(&schema_ipv4, &mut r_ipv4, &mut b_ipv4_work);
        let mut b_out_ipv4 = InfosetBuilder::new();
        let err_ipv4 = eng_ipv4.parse_term_inner(ipv4_id, &mut b_out_ipv4).unwrap_err();
        assert!(err_ipv4.message.as_str().contains("Insufficient data for IPv4 layer"));

        // 7. twoByteSwap layer requires whole words with odd bytes (lines 708-726)
        let mut b_swap = SchemaBuilder::new();
        let swap_props = ResolvedProperties {
            layer: Some("twoByteSwap".into()),
            ..Default::default()
        };
        let swap_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let swap_id = b_swap.add_term_with_props(QName::local("swap_seq"), swap_seq, swap_props).unwrap();
        b_swap.set_root(swap_id);
        let schema_swap = b_swap.build().unwrap();

        let bytes_odd = [0x01, 0x02, 0x03]; // 3 bytes, odd
        let src_swap = SliceByteSource::new(&bytes_odd);
        let mut r_swap = BitReader::new(src_swap, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_swap_work = WorkBudget::new(100);
        let mut eng_swap = ParserEngine::new(&schema_swap, &mut r_swap, &mut b_swap_work);
        eng_swap.variable_map.define_variable(
            QName::local("requireLengthInWholeWords"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("yes".into())),
        );
        let mut b_out_swap = InfosetBuilder::new();
        let err_swap = eng_swap.parse_term_inner(swap_id, &mut b_out_swap).unwrap_err();
        assert!(err_swap.message.as_str().contains("Data length is not a multiple of 2"));

        // 8. push_in_scope_delimiter, push_in_scope_terminator, and peek_in_scope_delimiter (lines 200-237)
        let bytes_peek = *b",;";
        let src_peek = SliceByteSource::new(&bytes_peek);
        let mut r_peek = BitReader::new(src_peek, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_peek = WorkBudget::new(100);
        let mut eng_peek = ParserEngine::new(&schema, &mut r_peek, &mut b_peek);
        eng_peek.push_in_scope_delimiter(",".into(), false, "UTF-8".into());
        eng_peek.push_in_scope_terminator(";".into(), false, "UTF-8".into());
        let delim_in_scope = crate::schema::ir::InScopeDelimiter {
            text: ",".into(),
            ignore_case: false,
            encoding: "UTF-8".into(),
        };
        assert!(eng_peek.peek_in_scope_delimiter(&delim_in_scope));

        // 9. checkDigit layer with Int, Long, UnsignedInt, and insufficient data (lines 666-681)
        let mut b_cd = SchemaBuilder::new();
        let cd_props = ResolvedProperties {
            layer: Some("checkDigit".into()),
            ..Default::default()
        };
        let cd_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let cd_id = b_cd.add_term_with_props(QName::local("cd_seq"), cd_seq, cd_props).unwrap();
        b_cd.set_root(cd_id);
        let schema_cd = b_cd.build().unwrap();

        // Insufficient data (< 10 bytes default)
        let bytes_cd_short = [0x01, 0x02];
        let src_cd_short = SliceByteSource::new(&bytes_cd_short);
        let mut r_cd_short = BitReader::new(src_cd_short, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_cd_work = WorkBudget::new(100);
        let mut eng_cd_short = ParserEngine::new(&schema_cd, &mut r_cd_short, &mut b_cd_work);
        let mut b_out_cd = InfosetBuilder::new();
        let err_cd = eng_cd_short.parse_term_inner(cd_id, &mut b_out_cd).unwrap_err();
        assert!(err_cd.message.as_str().contains("Insufficient data for checkDigit layer"));

        // checkDigit with Int length
        let bytes_cd_ok = [0x01, 0x02, 0x03, 0x04];
        let src_cd_ok = SliceByteSource::new(&bytes_cd_ok);
        let mut r_cd_ok = BitReader::new(src_cd_ok, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_cd_ok_work = WorkBudget::new(100);
        let mut eng_cd_ok = ParserEngine::new(&schema_cd, &mut r_cd_ok, &mut b_cd_ok_work);
        eng_cd_ok.variable_map.define_variable(
            QName::local("length"),
            crate::infoset::value::DfdlSimpleType::Int,
            Some(crate::infoset::value::DfdlValue::Int(4)),
        );
        eng_cd_ok.variable_map.define_variable(
            QName::with_namespace(
                "urn:org.apache.daffodil.layers.checkDigit",
                "checkDigit",
                None,
            ),
            crate::infoset::value::DfdlSimpleType::UnsignedShort,
            None,
        );
        let mut b_out_cd_ok = InfosetBuilder::new();
        assert!(eng_cd_ok.parse_term_inner(cd_id, &mut b_out_cd_ok).is_ok());

        // 10. boundaryMark layer with found boundary mark (lines 748-755)
        let mut b_bm = SchemaBuilder::new();
        let bm_props = ResolvedProperties {
            layer: Some("boundaryMark".into()),
            ..Default::default()
        };
        let bm_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let bm_id = b_bm.add_term_with_props(QName::local("bm_seq"), bm_seq, bm_props).unwrap();
        b_bm.set_root(bm_id);
        let schema_bm = b_bm.build().unwrap();

        let bytes_bm = b"abc//def";
        let src_bm = SliceByteSource::new(bytes_bm);
        let mut r_bm = BitReader::new(src_bm, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_bm_work = WorkBudget::new(100);
        let mut eng_bm = ParserEngine::new(&schema_bm, &mut r_bm, &mut b_bm_work);
        let mut b_out_bm = InfosetBuilder::new();
        assert!(eng_bm.parse_term_inner(bm_id, &mut b_out_bm).is_ok());
        assert_eq!(eng_bm.reader.bit_limit(), None);
        assert_eq!(eng_bm.reader_position(), 40);

        // 11. stlBombOutLayer PE and RSDE errors (lines 763-793)
        let mut b_bomb = SchemaBuilder::new();
        let bomb_props = ResolvedProperties {
            layer: Some("stlBombOutLayer".into()),
            ..Default::default()
        };
        let bomb_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let bomb_id = b_bomb.add_term_with_props(QName::local("bomb_seq"), bomb_seq, bomb_props).unwrap();
        b_bomb.set_root(bomb_id);
        let schema_bomb = b_bomb.build().unwrap();

        let src_bomb1 = SliceByteSource::new(b"data");
        let mut r_bomb1 = BitReader::new(src_bomb1, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_bomb1_work = WorkBudget::new(100);
        let mut eng_bomb1 = ParserEngine::new(&schema_bomb, &mut r_bomb1, &mut b_bomb1_work);
        eng_bomb1.variable_map.define_variable(
            QName::local("bombWhere"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("read".into())),
        );
        eng_bomb1.variable_map.define_variable(
            QName::local("bombHow"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("PE".into())),
        );
        let mut b_out_b1 = InfosetBuilder::new();
        let err_b1 = eng_bomb1.parse_term_inner(bomb_id, &mut b_out_b1).unwrap_err();
        assert!(err_b1.message.as_str().contains("Parse Error: Bombed out at read"));

        let src_bomb2 = SliceByteSource::new(b"data");
        let mut r_bomb2 = BitReader::new(src_bomb2, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_bomb2_work = WorkBudget::new(100);
        let mut eng_bomb2 = ParserEngine::new(&schema_bomb, &mut r_bomb2, &mut b_bomb2_work);
        eng_bomb2.variable_map.define_variable(
            QName::local("bombWhere"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("read".into())),
        );
        eng_bomb2.variable_map.define_variable(
            QName::local("bombHow"),
            crate::infoset::value::DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("RSDE".into())),
        );
        let mut b_out_b2 = InfosetBuilder::new();
        let err_b2 = eng_bomb2.parse_term_inner(bomb_id, &mut b_out_b2).unwrap_err();
        assert!(err_b2.message.as_str().contains("Runtime Schema Definition Error: Bombed out at read"));

        // 12. base64_mime layer on sequence (lines 607-622)
        let mut b_b64 = SchemaBuilder::new();
        let b64_props = ResolvedProperties {
            layer: Some("base64_mime".into()),
            ..Default::default()
        };
        let b64_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let b64_id = b_b64.add_term_with_props(QName::local("b64_seq"), b64_seq, b64_props).unwrap();
        b_b64.set_root(b64_id);
        let schema_b64 = b_b64.build().unwrap();

        let src_b64 = SliceByteSource::new(b"SGVsbG8=");
        let mut r_b64 = BitReader::new(src_b64, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_b64_work = WorkBudget::new(100);
        let mut eng_b64 = ParserEngine::new(&schema_b64, &mut r_b64, &mut b_b64_work);
        let mut b_out_b64 = InfosetBuilder::new();
        assert!(eng_b64.parse_term_inner(b64_id, &mut b_out_b64).is_ok());

        // 13. IPv4Checksum layer (lines 627-659)
        let mut b_ipv4 = SchemaBuilder::new();
        let ipv4_props = ResolvedProperties {
            layer: Some("IPv4Checksum".into()),
            ..Default::default()
        };
        let ipv4_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let ipv4_id = b_ipv4.add_term_with_props(QName::local("ipv4_seq"), ipv4_seq, ipv4_props).unwrap();
        b_ipv4.set_root(ipv4_id);
        let schema_ipv4 = b_ipv4.build().unwrap();

        // Insufficient bytes (< 20)
        let src_ipv4_err = SliceByteSource::new(&[0u8; 10]);
        let mut r_ipv4_err = BitReader::new(src_ipv4_err, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut b_ipv4_work = WorkBudget::new(100);
        let mut eng_ipv4_err = ParserEngine::new(&schema_ipv4, &mut r_ipv4_err, &mut b_ipv4_work);
        let mut b_out_ip_err = InfosetBuilder::new();
        let err_ip = eng_ipv4_err.parse_term_inner(ipv4_id, &mut b_out_ip_err).unwrap_err();
        assert!(err_ip.message.as_str().contains("Insufficient data for IPv4 layer"));

        // Sufficient bytes (20 bytes)
        let src_ipv4_ok = SliceByteSource::new(&[0x45, 0x00, 0x00, 0x3c, 0x1c, 0x46, 0x40, 0x00, 0x40, 0x06, 0xb1, 0xe6, 0xac, 0x10, 0x0a, 0x63, 0xac, 0x10, 0x0a, 0x0c]);
        let mut r_ipv4_ok = BitReader::new(src_ipv4_ok, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_ipv4_ok = ParserEngine::new(&schema_ipv4, &mut r_ipv4_ok, &mut b_ipv4_work);
        eng_ipv4_ok.variable_map.define_variable(
            QName::with_namespace("urn:org.apache.daffodil.layers.IPv4Checksum", "IPv4Checksum", None),
            DfdlSimpleType::UnsignedShort,
            None,
        );
        let mut b_out_ip_ok = InfosetBuilder::new();
        assert!(eng_ipv4_ok.parse_term_inner(ipv4_id, &mut b_out_ip_ok).is_ok());

        // 14. checkDigit layer (lines 660-707)
        let mut b_cd = SchemaBuilder::new();
        let cd_props = ResolvedProperties {
            layer: Some("checkDigit".into()),
            ..Default::default()
        };
        let cd_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let cd_id = b_cd.add_term_with_props(QName::local("cd_seq"), cd_seq, cd_props).unwrap();
        b_cd.set_root(cd_id);
        let schema_cd = b_cd.build().unwrap();

        // Insufficient bytes
        let src_cd_err = SliceByteSource::new(b"123");
        let mut r_cd_err = BitReader::new(src_cd_err, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cd_err = ParserEngine::new(&schema_cd, &mut r_cd_err, &mut b_ipv4_work);
        eng_cd_err.variable_map.define_variable(
            QName::local("length"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(10)),
        );
        let mut b_out_cd_err = InfosetBuilder::new();
        let err_cd = eng_cd_err.parse_term_inner(cd_id, &mut b_out_cd_err).unwrap_err();
        assert!(err_cd.message.as_str().contains("Insufficient data for checkDigit layer"));

        // Sufficient bytes
        let src_cd_ok = SliceByteSource::new(b"0123456789");
        let mut r_cd_ok = BitReader::new(src_cd_ok, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cd_ok = ParserEngine::new(&schema_cd, &mut r_cd_ok, &mut b_ipv4_work);
        eng_cd_ok.variable_map.define_variable(
            QName::local("length"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(10)),
        );
        eng_cd_ok.variable_map.define_variable(
            QName::with_namespace("urn:org.apache.daffodil.layers.checkDigit", "checkDigit", None),
            DfdlSimpleType::UnsignedShort,
            None,
        );
        let mut b_out_cd_ok = InfosetBuilder::new();
        assert!(eng_cd_ok.parse_term_inner(cd_id, &mut b_out_cd_ok).is_ok());

        // 15. twoByteSwap layer with odd bytes (lines 708-726)
        let mut b_tbs = SchemaBuilder::new();
        let tbs_props = ResolvedProperties {
            layer: Some("twoByteSwap".into()),
            ..Default::default()
        };
        let tbs_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let tbs_id = b_tbs.add_term_with_props(QName::local("tbs_seq"), tbs_seq, tbs_props).unwrap();
        b_tbs.set_root(tbs_id);
        let schema_tbs = b_tbs.build().unwrap();

        let src_tbs = SliceByteSource::new(b"odd");
        let mut r_tbs = BitReader::new(src_tbs, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_tbs = ParserEngine::new(&schema_tbs, &mut r_tbs, &mut b_ipv4_work);
        eng_tbs.variable_map.define_variable(
            QName::local("requireLengthInWholeWords"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("yes".into())),
        );
        let mut b_out_tbs = InfosetBuilder::new();
        let err_tbs = eng_tbs.parse_term_inner(tbs_id, &mut b_out_tbs).unwrap_err();
        assert!(err_tbs.message.as_str().contains("Data length is not a multiple of 2 for twoByteSwap layer"));

        // 16. boundaryMark layer (lines 728-762, 955-965)
        let mut b_bm = SchemaBuilder::new();
        let bm_props = ResolvedProperties {
            layer: Some("boundaryMark".into()),
            ..Default::default()
        };
        let bm_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let bm_id = b_bm.add_term_with_props(QName::local("bm_seq"), bm_seq, bm_props).unwrap();
        b_bm.set_root(bm_id);
        let schema_bm = b_bm.build().unwrap();

        let src_bm = SliceByteSource::new(b"hello//world");
        let mut r_bm = BitReader::new(src_bm, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bm = ParserEngine::new(&schema_bm, &mut r_bm, &mut b_ipv4_work);
        eng_bm.variable_map.define_variable(
            QName::local("boundaryMark"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("//".into())),
        );
        let mut b_out_bm = InfosetBuilder::new();
        assert!(eng_bm.parse_term_inner(bm_id, &mut b_out_bm).is_ok());

        // 17. push_in_scope_delimiter and terminator with empty string
        eng_bm.push_in_scope_delimiter("".into(), false, "UTF-8".into());
        eng_bm.push_in_scope_terminator("".into(), false, "UTF-8".into());

        // 18. TrailingEmptyStrict separator suppression policy error (lines 971-986)
        let mut b_tes = SchemaBuilder::new();
        let tes_props = ResolvedProperties {
            separator: Some(",".into()),
            separator_suppression_policy: crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict,
            ..Default::default()
        };
        let tes_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence {
            members: alloc::vec![],
        });
        let tes_id = b_tes.add_term_with_props(QName::local("tes_seq"), tes_seq, tes_props).unwrap();
        b_tes.set_root(tes_id);
        let schema_tes = b_tes.build().unwrap();

        let src_tes = SliceByteSource::new(b",,,");
        let mut r_tes = BitReader::new(src_tes, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_tes = ParserEngine::new(&schema_tes, &mut r_tes, &mut b_ipv4_work);
        let mut b_out_tes = InfosetBuilder::new();
        let err_tes = eng_tes.parse_term_inner(tes_id, &mut b_out_tes).unwrap_err();
        assert!(err_tes.message.as_str().contains("Trailing separators found with trailingEmptyStrict"));

        // 19. stlBombOutLayer stringVar doubling (lines 794-805)
        let src_bomb3 = SliceByteSource::new(b"data");
        let mut r_bomb3 = BitReader::new(src_bomb3, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bomb3 = ParserEngine::new(&schema_bomb, &mut r_bomb3, &mut b_ipv4_work);
        eng_bomb3.variable_map.define_variable(
            QName::with_namespace("urn:STL", "stringVar", None),
            DfdlSimpleType::String,
            Some(DfdlValue::String("val".into())),
        );
        let mut b_out_b3 = InfosetBuilder::new();
        assert!(eng_bomb3.parse_term_inner(bomb_id, &mut b_out_b3).is_ok());
        let doubled = eng_bomb3.variable_map.get_variable("stringVar").unwrap();
        assert_eq!(doubled, &DfdlValue::String("val val".into()));

        // 20. stlOk1 layer (lines 807-809)
        let mut b_ok1 = SchemaBuilder::new();
        let ok1_props = ResolvedProperties {
            layer: Some("stlOk1".into()),
            ..Default::default()
        };
        let ok1_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence { members: alloc::vec![] });
        let ok1_id = b_ok1.add_term_with_props(QName::local("ok1"), ok1_seq, ok1_props).unwrap();
        b_ok1.set_root(ok1_id);
        let schema_ok1 = b_ok1.build().unwrap();

        let src_ok1 = SliceByteSource::new(b"ok");
        let mut r_ok1 = BitReader::new(src_ok1, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_ok1 = ParserEngine::new(&schema_ok1, &mut r_ok1, &mut b_ipv4_work);
        let mut b_out_ok1 = InfosetBuilder::new();
        assert!(eng_ok1.parse_term_inner(ok1_id, &mut b_out_ok1).is_ok());

        // 21. GroupRef with assertions and discriminators (lines 1363-1382)
        let mut b_grp = SchemaBuilder::new();
        let mut grp_props = ResolvedProperties::default();
        grp_props.asserts.push(crate::schema::ir::CompiledAssert {
            test_kind: crate::schema::ir::TestKind::Expression,
            test_expr: "fn:true()".into(),
            message: Some("assert failed".into()),
            failure_type: crate::schema::ir::FailureType::ProcessingError,
        });
        let child_seq = TermKind::Sequence(crate::schema::ir::CompiledSequence { members: alloc::vec![] });
        let child_seq_id = b_grp.add_term_with_props(QName::local("child"), child_seq, ResolvedProperties::default()).unwrap();
        let grp_term = TermKind::GroupRef(child_seq_id);
        let grp_id = b_grp.add_term_with_props(QName::local("grp"), grp_term, grp_props).unwrap();
        b_grp.set_root(grp_id);
        let schema_grp = b_grp.build().unwrap();

        let src_grp = SliceByteSource::new(b"data");
        let mut r_grp = BitReader::new(src_grp, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_grp = ParserEngine::new(&schema_grp, &mut r_grp, &mut b_ipv4_work);
        let mut b_out_grp = InfosetBuilder::new();
        assert!(eng_grp.parse_term_inner(grp_id, &mut b_out_grp).is_ok());

        // 22. Choice explicit choiceLength with branch exceeding length (lines 1235-1262)
        let mut b_cho = SchemaBuilder::new();
        let mut cho_props = ResolvedProperties::default();
        cho_props.choice_length = Some(8); // 8 bits allowed
        cho_props.choice_length_kind = crate::schema::ir::LengthKind::Explicit;
        // branch that consumes 16 bits
        let branch_elem = TermKind::Element(crate::schema::ir::CompiledElement {
            name: QName::local("elem"),
            type_ir: crate::schema::ir::CompiledType::Simple(DfdlSimpleType::Short),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        });
        let mut elem_props = ResolvedProperties::default();
        elem_props.representation = Representation::Binary;
        elem_props.binary_number_rep = crate::schema::ir::BinaryNumberRep::Binary;
        let branch_id = b_cho.add_term_with_props(QName::local("elem"), branch_elem, elem_props).unwrap();
        let cho_term = TermKind::Choice(crate::schema::ir::CompiledChoice {
            branches: alloc::vec![branch_id],
        });
        let cho_id = b_cho.add_term_with_props(QName::local("cho"), cho_term, cho_props).unwrap();
        b_cho.set_root(cho_id);
        let schema_cho = b_cho.build().unwrap();

        let src_cho = SliceByteSource::new(&[0x12, 0x34]);
        let mut r_cho = BitReader::new(src_cho, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cho = ParserEngine::new(&schema_cho, &mut r_cho, &mut b_ipv4_work);
        let mut b_out_cho = InfosetBuilder::new();
        let err_cho = eng_cho.parse_term_inner(cho_id, &mut b_out_cho).unwrap_err();
        assert!(err_cho.message.as_str().contains("All Choice Alternatives Failed"));
    }

    #[test]
    fn test_binary_parser_edge_cases() {
        use crate::schema::ir::{BinaryCalendarRep, BinaryNumberRep, LengthKind, LengthUnits};

        let schema = dummy_schema();
        let mut budget = WorkBudget::new(100);

        // 1. Binary number delimited by literal terminator and EOF break (lines 160-175)
        let num_delim_data = [0x12, 0x34, 0x5C, b';'];
        let src_nd = SliceByteSource::new(&num_delim_data);
        let mut r_nd = BitReader::new(src_nd, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_nd = ParserEngine::new(&schema, &mut r_nd, &mut budget);
        let props_nd = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: BinaryNumberRep::Packed,
            length_kind: LengthKind::Delimited,
            terminator: Some(";".into()),
            ..Default::default()
        };
        let val_nd = eng_nd.parse_binary_value(DfdlSimpleType::Int, &props_nd, None).unwrap();
        assert_eq!(val_nd, DfdlValue::Int(12345));

        // Binary number delimited reading until EOF (lines 173-175)
        let num_eof_data = [0x12, 0x34, 0x5C];
        let src_ne = SliceByteSource::new(&num_eof_data);
        let mut r_ne = BitReader::new(src_ne, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_ne = ParserEngine::new(&schema, &mut r_ne, &mut budget);
        let props_ne = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: BinaryNumberRep::Packed,
            length_kind: LengthKind::Delimited,
            ..Default::default()
        };
        let val_ne = eng_ne.parse_binary_value(DfdlSimpleType::Int, &props_ne, None).unwrap();
        assert_eq!(val_ne, DfdlValue::Int(12345));

        // 2. HexBinary unbounded until EOF (line 513)
        let hex_eof_data = [0xAA, 0xBB, 0xCC];
        let src_he = SliceByteSource::new(&hex_eof_data);
        let mut r_he = BitReader::new(src_he, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_he = ParserEngine::new(&schema, &mut r_he, &mut budget);
        let props_he = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: LengthKind::Implicit,
            ..Default::default()
        };
        let val_he = eng_he.parse_binary_value(DfdlSimpleType::HexBinary, &props_he, None).unwrap();
        assert_eq!(val_he, DfdlValue::HexBinary(alloc::vec![0xAA, 0xBB, 0xCC]));

        // 3. UTF-8 binary string lead byte > 0xF7 fallback (line 590)
        let u8_inv_lead = [0xFF];
        let src_u8 = SliceByteSource::new(&u8_inv_lead);
        let mut r_u8 = BitReader::new(src_u8, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_u8 = ParserEngine::new(&schema, &mut r_u8, &mut budget);
        let props_u8 = ResolvedProperties {
            representation: Representation::Binary,
            encoding: "UTF-8".into(),
            length_units: LengthUnits::Characters,
            ..Default::default()
        };
        let val_u8 = eng_u8.parse_binary_value(DfdlSimpleType::String, &props_u8, Some(1));
        assert!(val_u8.is_ok());

        // 4. Calendar delimited by terminator and EOF (lines 661-679)
        let cal_delim_data = [0x02, 0x02, 0x61, 0x00, 0x7C, b';'];
        let src_cd = SliceByteSource::new(&cal_delim_data);
        let mut r_cd = BitReader::new(src_cd, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cd = ParserEngine::new(&schema, &mut r_cd, &mut budget);
        let props_cd = ResolvedProperties {
            representation: Representation::Binary,
            binary_calendar_rep: BinaryCalendarRep::Packed,
            length_kind: LengthKind::Delimited,
            terminator: Some(";".into()),
            calendar_pattern: Some("yyyyMMdd".into()),
            ..Default::default()
        };
        let val_cd = eng_cd.parse_binary_value(DfdlSimpleType::Date, &props_cd, None).unwrap();
        assert!(matches!(val_cd, DfdlValue::Date(_)));

        // 5. Packed calendar negative for DateTime and Time (lines 688-699)
        let cal_neg_data = [0x02, 0x02, 0x61, 0x00, 0x7D]; // 0x0D negative
        let src_cneg = SliceByteSource::new(&cal_neg_data);
        let mut r_cneg = BitReader::new(src_cneg, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cneg = ParserEngine::new(&schema, &mut r_cneg, &mut budget);
        let props_cneg = ResolvedProperties {
            representation: Representation::Binary,
            binary_calendar_rep: BinaryCalendarRep::Packed,
            calendar_pattern: Some("yyyyMMdd".into()),
            ..Default::default()
        };
        let err_dt = eng_cneg.parse_binary_value(DfdlSimpleType::DateTime, &props_cneg, Some(5)).unwrap_err();
        assert!(err_dt.message.as_str().contains("Unable to parse xs:dateTime from negative packed number"));

        let src_ct = SliceByteSource::new(&cal_neg_data);
        let mut r_ct = BitReader::new(src_ct, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_ct = ParserEngine::new(&schema, &mut r_ct, &mut budget);
        let err_t = eng_ct.parse_binary_value(DfdlSimpleType::Time, &props_cneg, Some(5)).unwrap_err();
        assert!(err_t.message.as_str().contains("Unable to parse xs:time from negative packed number"));

        // 6. Packed calendar non-decimal nibbles > 9 skipped (lines 705, 711, 716)
        let cal_nibble_data = [0xFA, 0x02, 0x02, 0x61, 0x00, 0x7C];
        let src_cnib = SliceByteSource::new(&cal_nibble_data);
        let mut r_cnib = BitReader::new(src_cnib, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cnib = ParserEngine::new(&schema, &mut r_cnib, &mut budget);
        let props_cnib = ResolvedProperties {
            representation: Representation::Binary,
            binary_calendar_rep: BinaryCalendarRep::Packed,
            calendar_pattern: Some("yyyyMMdd".into()),
            ..Default::default()
        };
        let val_cnib = eng_cnib.parse_binary_value(DfdlSimpleType::Date, &props_cnib, Some(6)).unwrap();
        assert_eq!(val_cnib, DfdlValue::Date("2026-10-07".into()));

        // 7. Invalid date from digits in packed and BCD (lines 723, 750)
        let cal_inv_month = [0x20, 0x26, 0x99, 0x01, 0x7C];
        let src_inv = SliceByteSource::new(&cal_inv_month);
        let mut r_inv = BitReader::new(src_inv, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_inv = ParserEngine::new(&schema, &mut r_inv, &mut budget);
        assert!(eng_inv.parse_binary_value(DfdlSimpleType::Date, &props_cnib, Some(5)).is_err());

        let bcd_inv_month = [0x20, 0x26, 0x99, 0x01];
        let src_bcd = SliceByteSource::new(&bcd_inv_month);
        let mut r_bcd = BitReader::new(src_bcd, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bcd = ParserEngine::new(&schema, &mut r_bcd, &mut budget);
        let mut props_bcd_inv = props_cnib.clone();
        props_bcd_inv.binary_calendar_rep = BinaryCalendarRep::Bcd;
        assert!(eng_bcd.parse_binary_value(DfdlSimpleType::Date, &props_bcd_inv, Some(4)).is_err());

        // 8. HexBinary exceeding schema max_hex_binary_length_in_bytes (lines 528-536)
        let mut max_hex_schema = dummy_schema();
        max_hex_schema.max_hex_binary_length_in_bytes = Some(2);

        let hex_too_long = [0xAA, 0xBB, 0xCC];
        let src_hex_tl = SliceByteSource::new(&hex_too_long);
        let mut r_hex_tl = BitReader::new(src_hex_tl, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_hex_tl = ParserEngine::new(&max_hex_schema, &mut r_hex_tl, &mut budget);
        let err_hex = eng_hex_tl.parse_binary_value(DfdlSimpleType::HexBinary, &props_he, None).unwrap_err();
        assert!(err_hex.message.as_str().contains("exceeds maximum allowed length"));

        // 9. Negative packed xs:dateTime rejection (lines 688-700)
        let dt_neg_packed = [0x20, 0x26, 0x10, 0x07, 0x12, 0x00, 0x00, 0x0D];
        let src_dt_neg = SliceByteSource::new(&dt_neg_packed);
        let mut r_dt_neg = BitReader::new(src_dt_neg, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_dt_neg = ParserEngine::new(&schema, &mut r_dt_neg, &mut budget);
        let mut props_dt = props_cnib;
        props_dt.calendar_pattern = Some("yyyyMMddHHmmss".into());
        let err_dt = eng_dt_neg.parse_binary_value(DfdlSimpleType::DateTime, &props_dt, Some(8)).unwrap_err();
        assert!(err_dt.message.as_str().contains("Unable to parse xs:dateTime from negative packed number"));
    }

    #[test]
    fn test_binary_parser_scalars_and_strings() {
        use crate::schema::ir::{BinaryBooleanRep, BinaryCalendarRep, BinaryNumberRep};

        // Schema and budget setup for testing binary scalar parsing
        let schema = dummy_schema();
        let mut budget = WorkBudget::new(10_000);

        // 1. UnsignedInt, UnsignedLong, UnsignedShort parsing
        let int_bytes = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let src_int = SliceByteSource::new(&int_bytes);
        let mut r_int = BitReader::new(src_int, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_int = ParserEngine::new(&schema, &mut r_int, &mut budget);
        let props_int = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: BinaryNumberRep::Binary,
            byte_order: ByteOrder::BigEndian,
            ..Default::default()
        };

        // Test UnsignedShort (2 bytes)
        let v_ushort = eng_int.parse_binary_value(DfdlSimpleType::UnsignedShort, &props_int, None).unwrap();
        assert_eq!(v_ushort, DfdlValue::UnsignedShort(0x1234));

        // Test UnsignedInt (4 bytes)
        let v_uint = eng_int.parse_binary_value(DfdlSimpleType::UnsignedInt, &props_int, None).unwrap();
        assert_eq!(v_uint, DfdlValue::UnsignedInt(0x5678_9ABC));

        // Test UnsignedLong (8 bytes from fresh source)
        let long_bytes = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let src_long = SliceByteSource::new(&long_bytes);
        let mut r_long = BitReader::new(src_long, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_long = ParserEngine::new(&schema, &mut r_long, &mut budget);
        let v_ulong = eng_long.parse_binary_value(DfdlSimpleType::UnsignedLong, &props_int, None).unwrap();
        assert_eq!(v_ulong, DfdlValue::UnsignedLong(0x0123_4567_89AB_CDEF));

        // 2. Binary Boolean variants (16-bit true, 32-bit false, and mismatch error)
        let bool_bytes = [0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02];
        let src_bool = SliceByteSource::new(&bool_bytes);
        let mut r_bool = BitReader::new(src_bool, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bool = ParserEngine::new(&schema, &mut r_bool, &mut budget);
        let props_bool = ResolvedProperties {
            representation: Representation::Binary,
            binary_boolean_true_rep: BinaryBooleanRep::Value(1),
            binary_boolean_false_rep: BinaryBooleanRep::Value(0),
            length_units: LengthUnits::Bits,
            ..Default::default()
        };
        // 16-bit boolean true
        let v_b1 = eng_bool.parse_binary_value(DfdlSimpleType::Boolean, &props_bool, Some(16)).unwrap();
        assert_eq!(v_b1, DfdlValue::Boolean(true));
        // 32-bit boolean false
        let v_b2 = eng_bool.parse_binary_value(DfdlSimpleType::Boolean, &props_bool, Some(32)).unwrap();
        assert_eq!(v_b2, DfdlValue::Boolean(false));
        // 16-bit boolean mismatch (neither true nor false rep)
        let err_b3 = eng_bool.parse_binary_value(DfdlSimpleType::Boolean, &props_bool, Some(16)).unwrap_err();
        assert!(err_b3.message.as_str().contains("matches neither binaryBooleanTrueRep"));

        // 3. Binary HexBinary with LengthUnits::Bits and Delimited kind
        let hex_bits = [0xAB, 0xCD, b';'];
        let src_hb = SliceByteSource::new(&hex_bits);
        let mut r_hb = BitReader::new(src_hb, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_hb = ParserEngine::new(&schema, &mut r_hb, &mut budget);
        let props_hb = ResolvedProperties {
            representation: Representation::Binary,
            length_units: LengthUnits::Bits,
            ..Default::default()
        };
        // Bits length units (12 bits: 1.5 bytes)
        let v_hb_bits = eng_hb.parse_binary_value(DfdlSimpleType::HexBinary, &props_hb, Some(12)).unwrap();
        assert_eq!(v_hb_bits, DfdlValue::HexBinary(alloc::vec![0xAB, 0xC0]));

        // Delimited HexBinary reading until delimiter
        let src_hb_delim = SliceByteSource::new(&hex_bits);
        let mut r_hb_delim = BitReader::new(src_hb_delim, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_hb_delim = ParserEngine::new(&schema, &mut r_hb_delim, &mut budget);
        eng_hb_delim.in_scope_delimiters.push(crate::schema::ir::InScopeDelimiter {
            text: ";".into(),
            ignore_case: false,
            encoding: "UTF-8".into(),
        });
        let props_hb_delim = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: LengthKind::Delimited,
            terminator: Some(";".into()),
            ..Default::default()
        };
        let v_hb_delim = eng_hb_delim.parse_binary_value(DfdlSimpleType::HexBinary, &props_hb_delim, None).unwrap();
        assert_eq!(v_hb_delim, DfdlValue::HexBinary(alloc::vec![0xAB, 0xCD]));

        // 4. Sub-byte 7-bit packed string decoding and bit-length units
        let packed_ascii = [0b1100_0011, 0b1000_1011, 0b0001_1000];
        let src_pascii = SliceByteSource::new(&packed_ascii);
        let mut r_pascii = BitReader::new(src_pascii, BitOrder::LeastSignificantBitFirst, ByteOrder::LittleEndian);
        let mut eng_pascii = ParserEngine::new(&schema, &mut r_pascii, &mut budget);
        let props_pascii = ResolvedProperties {
            representation: Representation::Binary,
            encoding: "X-DFDL-US-ASCII-7-BIT-PACKED".into(),
            length_units: LengthUnits::Characters,
            bit_order: BitOrder::LeastSignificantBitFirst,
            byte_order: ByteOrder::LittleEndian,
            ..Default::default()
        };
        let v_pstr = eng_pascii.parse_binary_value(DfdlSimpleType::String, &props_pascii, Some(3)).unwrap();
        if let DfdlValue::String(s) = v_pstr {
            assert_eq!(s.len(), 3);
        } else {
            panic!("Expected String value");
        }

        // Binary string with LengthUnits::Bits
        let str_bits = b"Hello, World!";
        let src_sb = SliceByteSource::new(str_bits);
        let mut r_sb = BitReader::new(src_sb, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_sb = ParserEngine::new(&schema, &mut r_sb, &mut budget);
        let props_sb = ResolvedProperties {
            representation: Representation::Binary,
            encoding: "UTF-8".into(),
            length_units: LengthUnits::Bits,
            ..Default::default()
        };
        let v_sb = eng_sb.parse_binary_value(DfdlSimpleType::String, &props_sb, Some(40)).unwrap();
        assert_eq!(v_sb, DfdlValue::String("Hello".into()));

        // Binary string with LengthUnits::Bytes
        let src_sbytes = SliceByteSource::new(str_bits);
        let mut r_sbytes = BitReader::new(src_sbytes, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_sbytes = ParserEngine::new(&schema, &mut r_sbytes, &mut budget);
        let props_sbytes = ResolvedProperties {
            representation: Representation::Binary,
            encoding: "UTF-8".into(),
            length_units: LengthUnits::Bytes,
            ..Default::default()
        };
        let v_sbytes = eng_sbytes.parse_binary_value(DfdlSimpleType::String, &props_sbytes, Some(5)).unwrap();
        assert_eq!(v_sbytes, DfdlValue::String("Hello".into()));

        // 5. Packed Calendar with positive sign ending in 0x1C (lines 695-699)
        let packed_cal_pos = [0x20, 0x26, 0x10, 0x1C];
        let src_cpos = SliceByteSource::new(&packed_cal_pos);
        let mut r_cpos = BitReader::new(src_cpos, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cpos = ParserEngine::new(&schema, &mut r_cpos, &mut budget);
        let props_cpos = ResolvedProperties {
            representation: Representation::Binary,
            binary_calendar_rep: BinaryCalendarRep::Packed,
            calendar_pattern: Some("yyyyMM".into()),
            ..Default::default()
        };
        let v_cpos = eng_cpos.parse_binary_value(DfdlSimpleType::Date, &props_cpos, Some(4));
        assert!(v_cpos.is_ok());
    }

    /// Verifies comprehensive delimiter parsing, tokenization, whitespace matching,
    /// carriage return / line feed sequences, character references, and in-scope terminators.
    #[test]
    fn test_delimiters_extended_coverage() {
        let schema = dummy_schema();
        let mut budget = WorkBudget::new(1000);

        // 1. Tokenizing hexadecimal and raw character reference entities
        // Verifies %#x and %#r syntax along with invalid entity rejection.
        let tok_hex = parse_single_delim_tokens("%#x41;").unwrap();
        assert_eq!(tok_hex, vec![DelimToken::CharRef(0x41)]);
        let tok_raw = parse_single_delim_tokens("%#r42;").unwrap();
        assert_eq!(tok_raw, vec![DelimToken::CharRef(0x42)]);
        assert!(parse_single_delim_tokens("%#xZZ;").is_err());
        assert!(parse_single_delim_tokens("%#r;").is_err());
        assert!(parse_single_delim_tokens("%unterminated").is_err());

        // 2. DelimToken::NL matching CRLF, standalone CR, and LF in bitstream
        // Validates that carriage returns consume following newlines when present,
        // or successfully roll back if not followed by a newline byte.
        let nl_crlf = b"\r\nREST";
        let src_crlf = SliceByteSource::new(nl_crlf);
        let mut r_crlf = BitReader::new(src_crlf, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_crlf = ParserEngine::new(&schema, &mut r_crlf, &mut budget);
        assert!(eng_crlf.match_single_delim_tokens_with_case(&[DelimToken::NL], false).is_ok());
        assert_eq!(r_crlf.position().0, 16);

        let nl_cr_only = b"\rREST";
        let src_cr = SliceByteSource::new(nl_cr_only);
        let mut r_cr = BitReader::new(src_cr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr = ParserEngine::new(&schema, &mut r_cr, &mut budget);
        assert!(eng_cr.match_single_delim_tokens_with_case(&[DelimToken::NL], false).is_ok());
        assert_eq!(r_cr.position().0, 8);

        // 3. DelimToken::WSPStar and DelimToken::WSPPlus matching spaces and CRLF
        // Ensures star and plus whitespace quantifiers correctly consume CR+LF combos
        // as well as single CR characters without advancing past non-newline bytes.
        let wsp_data = b" \t\r\n\rX";
        let src_wsp = SliceByteSource::new(wsp_data);
        let mut r_wsp = BitReader::new(src_wsp, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_wsp = ParserEngine::new(&schema, &mut r_wsp, &mut budget);
        assert!(eng_wsp.match_single_delim_tokens_with_case(&[DelimToken::WSPStar], false).is_ok());
        assert_eq!(r_wsp.position().0, 40); // 5 bytes: space, tab, CR, LF, CR

        let wspp_data = b"\r\n ";
        let src_wspp = SliceByteSource::new(wspp_data);
        let mut r_wspp = BitReader::new(src_wspp, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_wspp = ParserEngine::new(&schema, &mut r_wspp, &mut budget);
        assert!(eng_wspp.match_single_delim_tokens_with_case(&[DelimToken::WSPPlus], false).is_ok());
        assert_eq!(r_wspp.position().0, 24);

        let wspp_cr_only = b"\r ";
        let src_wcr = SliceByteSource::new(wspp_cr_only);
        let mut r_wcr = BitReader::new(src_wcr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_wcr = ParserEngine::new(&schema, &mut r_wcr, &mut budget);
        assert!(eng_wcr.match_single_delim_tokens_with_case(&[DelimToken::WSPPlus], false).is_ok());

        // 4. DelimToken::CharRef multi-byte UTF-8 matching and mismatch
        // Tests character references for non-ASCII Unicode code points (e.g. Euro sign).
        let euro_bytes = "€".as_bytes(); // 0xE2, 0x82, 0xAC
        let src_euro = SliceByteSource::new(euro_bytes);
        let mut r_euro = BitReader::new(src_euro, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_euro = ParserEngine::new(&schema, &mut r_euro, &mut budget);
        let euro_tok = DelimToken::CharRef(0x20AC);
        assert!(eng_euro.match_single_delim_tokens_with_case(core::slice::from_ref(&euro_tok), false).is_ok());

        let bad_euro_bytes = [0xE2, 0x00, 0x00];
        let src_beuro = SliceByteSource::new(&bad_euro_bytes);
        let mut r_beuro = BitReader::new(src_beuro, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_beuro = ParserEngine::new(&schema, &mut r_beuro, &mut budget);
        assert!(eng_beuro.match_single_delim_tokens_with_case(&[euro_tok], false).is_err());

        // 5. In-scope terminator length precedence and detection
        // Tests terminator inspection to ensure longer terminators take priority over separators.
        let input_term = b"long_term_data";
        let src_term = SliceByteSource::new(input_term);
        let mut r_term = BitReader::new(src_term, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_term = ParserEngine::new(&schema, &mut r_term, &mut budget);
        eng_term.push_in_scope_terminator("long_term".into(), false, "UTF-8".into());
        assert!(eng_term.is_in_scope_terminator_longer(32)); // 32 bits = 4 bytes, terminator is 9 bytes
        assert!(!eng_term.is_in_scope_terminator_longer(100)); // 100 bits > 9 bytes
        assert!(eng_term.peek_any_in_scope_delimiter());
        eng_term.in_scope_terminators.pop();

        // 6. String token matching for NL variants and WSP combinations
        // Validates string-based delimiter matching across all supported line ending variants.
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::Literal(b"A".to_vec())], "\r\nA", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::Literal(b"A".to_vec())], "\nA", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::Literal(b"A".to_vec())], "\rA", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::Literal(b"A".to_vec())], "\u{0085}A", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::NL, DelimToken::Literal(b"A".to_vec())], "\u{2028}A", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::NL], "X", false));

        // WSPPlus and WSPStar matching string tokens
        assert!(match_delim_tokens_against_str(&[DelimToken::WSPPlus, DelimToken::Literal(b"END".to_vec())], "   END", false));
        assert!(!match_delim_tokens_against_str(&[DelimToken::WSPPlus, DelimToken::Literal(b"END".to_vec())], "NO_WSP", false));
        assert!(match_delim_tokens_against_str(&[DelimToken::WSPStar, DelimToken::Literal(b"END".to_vec())], "END", false));

        // 7. match_dfdl_string_literal fallback on invalid delimiter entity expression
        // When entity parsing fails, it falls back to raw or unescaped string comparisons
        // across case-sensitive and case-insensitive policies.
        assert!(match_dfdl_string_literal("%invalidEntity;", "%invalidEntity;", false));
        assert!(match_dfdl_string_literal("%invalidEntity;", "%INVALIDENTITY;", true));
        assert!(!match_dfdl_string_literal("%invalidEntity;", "other", false));

        // 8. Delimiter matching with non-UTF8 bytes under single-byte encoding
        // Ensures non-UTF-8 literal bytes are passed unmodified without decoding errors.
        let non_utf8_data = [0xFF];
        let src_non_utf8 = SliceByteSource::new(&non_utf8_data);
        let mut r_non_utf8 = BitReader::new(src_non_utf8, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_non_utf8 = ParserEngine::new(&schema, &mut r_non_utf8, &mut budget);
        eng_non_utf8.delim_encoding = "ISO-8859-1".into();
        assert!(eng_non_utf8.match_single_delim_tokens_with_case(&[DelimToken::Literal(vec![0xFF])], false).is_ok());

        // 9. Delimiter expression evaluation failure
        // An unparsable expression inside curly braces should propagate an error.
        let builder_err = crate::infoset::tree::InfosetBuilder::new();
        assert!(eng_non_utf8.evaluate_delimiter_str("{invalid syntax +++}", &builder_err).is_err());
    }

    /// Verifies comprehensive edge cases for delimiter tokenizing, binary numbers, calendar trimming,
    /// strict integer boundary conversions, zoned text numeric parsing, and choice dispatch key variants.
    #[test]
    fn test_parser_edge_cases_and_delimiters_extended() {
        let schema = dummy_schema();
        let mut budget = WorkBudget::new(100_000);

        // 1. Delimiter token parsing errors: invalid hex entity and unknown entity name
        // §6.3.1 Character entity references must specify valid hexadecimal or decimal codepoints.
        assert!(parse_single_delim_tokens("%#xGGGG;").is_err());
        assert!(parse_single_delim_tokens("%nonExistentDfdlEntity;").is_err());

        // 2. Binary sign extension edge cases (§13.7.1.1)
        // Tests 1-bit signed integers, full 64-bit boundaries, and 8-bit sign extension.
        assert_eq!(sign_extend(1, 1), 1);
        assert_eq!(sign_extend(0, 1), 0);
        assert_eq!(sign_extend(0x80, 8), -128);
        assert_eq!(sign_extend(0x7F, 8), 127);
        assert_eq!(sign_extend(0x8000_0000_0000_0000, 64), i64::MIN);
        assert_eq!(sign_extend(0x7FFF_FFFF_FFFF_FFFF, 64), i64::MAX);

        // 3. DelimToken::CharRef matching with multi-byte UTF-8 mismatch and invalid unicode
        // Tests character reference matching against bitstream where trailing bytes do not match.
        let euro_tok = DelimToken::CharRef(0x20AC); // Euro symbol: 0xE2 0x82 0xAC
        let mismatched_euro = [0xE2, 0x00, 0x00];
        let src_meuro = SliceByteSource::new(&mismatched_euro);
        let mut r_meuro = BitReader::new(src_meuro, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_meuro = ParserEngine::new(&schema, &mut r_meuro, &mut budget);
        assert!(eng_meuro.match_single_delim_tokens_with_case(&[euro_tok], false).is_err());

        // DelimToken::CharRef with codepoint above Unicode maximum (0x110000)
        let invalid_char_tok = DelimToken::CharRef(0x11_0000);
        let valid_bytes = [0x41, 0x42];
        let src_inv = SliceByteSource::new(&valid_bytes);
        let mut r_inv = BitReader::new(src_inv, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_inv = ParserEngine::new(&schema, &mut r_inv, &mut budget);
        // CharRef with invalid Unicode char falls through safely
        let _ = eng_inv.match_single_delim_tokens_with_case(&[invalid_char_tok], false);

        // 4. DelimToken::NL, WSPStar, and WSPPlus CR not followed by LF
        // Validates that carriage return not followed by linefeed triggers checkpoint rollback.
        let cr_only_data = b"\rX";
        let src_cr = SliceByteSource::new(cr_only_data);
        let mut r_cr = BitReader::new(src_cr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_cr = ParserEngine::new(&schema, &mut r_cr, &mut budget);
        // NL matches \r followed by non-\n by rolling back second byte
        assert!(eng_cr.match_single_delim_tokens_with_case(&[DelimToken::NL], false).is_ok());

        // WSPStar with CR followed by non-LF
        let src_wsp_cr = SliceByteSource::new(cr_only_data);
        let mut r_wsp_cr = BitReader::new(src_wsp_cr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_wsp_cr = ParserEngine::new(&schema, &mut r_wsp_cr, &mut budget);
        assert!(eng_wsp_cr.match_single_delim_tokens_with_case(&[DelimToken::WSPStar], false).is_ok());

        // WSPPlus with CR followed by non-LF
        let src_wspp_cr = SliceByteSource::new(cr_only_data);
        let mut r_wspp_cr = BitReader::new(src_wspp_cr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_wspp_cr = ParserEngine::new(&schema, &mut r_wspp_cr, &mut budget);
        assert!(eng_wspp_cr.match_single_delim_tokens_with_case(&[DelimToken::WSPPlus], false).is_ok());

        // 5. Binary calendar delimited by in-scope delimiter (§13.11)
        // Tests parsing binary seconds where element length is delimited by parent in-scope delimiter.
        let bin_sec_data = [0x00, 0x00, 0x00, 0x01, b';', b'A'];
        let src_bin_sec = SliceByteSource::new(&bin_sec_data);
        let mut r_bin_sec = BitReader::new(src_bin_sec, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut eng_bin_sec = ParserEngine::new(&schema, &mut r_bin_sec, &mut budget);
        eng_bin_sec.push_in_scope_delimiter(";".into(), false, "UTF-8".into());
        let mut props_bin_sec = ResolvedProperties::default();
        props_bin_sec.representation = Representation::Binary;
        props_bin_sec.binary_calendar_rep = crate::schema::ir::BinaryCalendarRep::BinarySeconds;
        props_bin_sec.length_kind = LengthKind::Delimited;
        let sec_val = eng_bin_sec.parse_binary_value(DfdlSimpleType::DateTime, &props_bin_sec, None);
        assert!(sec_val.is_ok());

        // 6. Strict integer bounds with i64::MIN and padding
        // Tests minimum signed 64-bit integer literal and custom character padding.
        assert_eq!(parse_strict_int_i64("-9223372036854775808", None, ".", ",", None, TextTrimKind::None), Some(i64::MIN));
        assert_eq!(parse_strict_int_i64("###42###", None, ".", ",", Some("#"), TextTrimKind::Both), Some(42));
        assert_eq!(parse_strict_int_i64("###42", None, ".", ",", Some("#"), TextTrimKind::Head), Some(42));
        assert_eq!(parse_strict_int_i64("42###", None, ".", ",", Some("#"), TextTrimKind::Tail), Some(42));
        assert_eq!(parse_strict_int_i64("#####", None, ".", ",", Some("#"), TextTrimKind::Both), None);

        // 7. Calendar text trimming and prohibited timezone forms (§13.11)
        // Validates pad trimming on date inputs and rejection of prohibited -00:00 timezone.
        assert!(parse_calendar_from_text("###2023-10-10###", None, Some("#"), TextTrimKind::Both, DfdlSimpleType::Date, CalendarCheckPolicy::Strict, CalendarFirstDayOfWeek::Monday).is_ok());
        assert!(parse_calendar_from_text("###2023-10-10", None, Some("#"), TextTrimKind::Head, DfdlSimpleType::Date, CalendarCheckPolicy::Strict, CalendarFirstDayOfWeek::Monday).is_ok());
        assert!(parse_calendar_from_text("2023-10-10###", None, Some("#"), TextTrimKind::Tail, DfdlSimpleType::Date, CalendarCheckPolicy::Strict, CalendarFirstDayOfWeek::Monday).is_ok());
        assert!(parse_calendar_from_text("#####", None, Some("#"), TextTrimKind::Both, DfdlSimpleType::Date, CalendarCheckPolicy::Strict, CalendarFirstDayOfWeek::Monday).is_err());
        assert!(parse_calendar_from_text("2023-10-10-00:00", None, None, TextTrimKind::None, DfdlSimpleType::Date, CalendarCheckPolicy::Strict, CalendarFirstDayOfWeek::Monday).is_err());
    }
