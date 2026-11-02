//! Unit tests for DFDL expression evaluation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use super::context::*;
use super::functions::*;
use super::helpers::*;
use super::navigation::*;
use super::operators::*;
use crate::expr::ast::{BinaryOp, ExprAst, UnaryOp};
use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use crate::limits::WorkBudget;
use crate::types::QName;
use alloc::boxed::Box;

    #[test]
    fn test_eval_new_functions() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let res_inc = eval_fn_call(
            &QName::local("checkRangeInclusive"),
            &[DfdlValue::Int(5), DfdlValue::Int(1), DfdlValue::Int(10)],
            &ctx,
        )
        .expect("checkRangeInclusive evaluation failed");
        assert_eq!(res_inc, DfdlValue::Boolean(true));

        let res_exc = eval_fn_call(
            &QName::local("checkRangeExclusive"),
            &[DfdlValue::Int(10), DfdlValue::Int(1), DfdlValue::Int(10)],
            &ctx,
        )
        .expect("checkRangeExclusive evaluation failed");
        assert_eq!(res_exc, DfdlValue::Boolean(false));

        let res_hex = eval_fn_call(
            &QName::local("hexBinary"),
            &[DfdlValue::String(alloc::string::String::from("A1B2"))],
            &ctx,
        )
        .expect("hexBinary evaluation failed");
        assert_eq!(res_hex, DfdlValue::HexBinary(alloc::vec![0xA1, 0xB2]));

        let res_round = eval_fn_call(
            &QName::local("round-half-to-even"),
            &[DfdlValue::Double(2.5)],
            &ctx,
        )
        .expect("round-half-to-even evaluation failed");
        assert_eq!(res_round, DfdlValue::Double(2.0));

        let res_trace = eval_fn_call(
            &QName::local("trace"),
            &[
                DfdlValue::Int(42),
                DfdlValue::String(alloc::string::String::from("label")),
            ],
            &ctx,
        )
        .expect("trace evaluation failed");
        assert_eq!(res_trace, DfdlValue::Int(42));
    }

    #[test]
    fn test_eval_division_by_zero_error() {
        let err_div = eval_div(DfdlValue::Int(10), DfdlValue::Int(0)).unwrap_err();
        assert!(err_div.message.to_string().contains("integer division"));
        assert!(err_div.message.to_string().contains("NaN"));

        let dbl_inf = eval_div(DfdlValue::Double(10.0), DfdlValue::Double(0.0)).unwrap();
        assert_eq!(dbl_inf, DfdlValue::Double(f64::INFINITY));
    }

    #[test]
    fn test_constructor_function_range_and_errors() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Valid conversions
        assert_eq!(
            eval_fn_call(
                &QName::local("short"),
                &[DfdlValue::String(alloc::string::String::from("32767"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Short(32767)
        );

        // Out of range errors
        assert!(eval_fn_call(
            &QName::local("short"),
            &[DfdlValue::String(alloc::string::String::from("-32769"))],
            &ctx
        )
        .is_err());

        assert!(eval_fn_call(
            &QName::local("byte"),
            &[DfdlValue::String(alloc::string::String::from("128"))],
            &ctx
        )
        .is_err());

        assert!(eval_fn_call(
            &QName::local("unsignedByte"),
            &[DfdlValue::String(alloc::string::String::from("-1"))],
            &ctx
        )
        .is_err());

        assert!(eval_fn_call(&QName::local("unsignedInt"), &[DfdlValue::Long(-5)], &ctx).is_err());

        assert!(eval_fn_call(
            &QName::local("int"),
            &[DfdlValue::Long(5_000_000_000)],
            &ctx
        )
        .is_err());

        // Invalid string formats
        assert!(eval_fn_call(
            &QName::local("int"),
            &[DfdlValue::String(alloc::string::String::from("abc"))],
            &ctx
        )
        .is_err());

        assert!(eval_fn_call(
            &QName::local("double"),
            &[DfdlValue::String(alloc::string::String::from(
                "not_a_number"
            ))],
            &ctx
        )
        .is_err());
    }

    #[test]
    fn test_bitwise_functions_validation() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Matching signed integers
        let or_res = eval_fn_call(
            &QName::local("bitOr"),
            &[DfdlValue::Byte(1), DfdlValue::Short(256)],
            &ctx,
        )
        .unwrap();
        assert_eq!(or_res, DfdlValue::Long(257));

        // Matching unsigned integers
        let and_res = eval_fn_call(
            &QName::local("bitAnd"),
            &[DfdlValue::UnsignedInt(0xF0), DfdlValue::UnsignedByte(0x33)],
            &ctx,
        )
        .unwrap();
        assert_eq!(and_res, DfdlValue::UnsignedLong(0x30));

        // Signedness mismatch error
        let err = eval_fn_call(
            &QName::local("bitOr"),
            &[DfdlValue::Long(1), DfdlValue::UnsignedLong(1)],
            &ctx,
        )
        .unwrap_err();
        assert!(err.to_string().contains("must match in signedness"));

        // Unbounded integer rejection
        let err2 = eval_fn_call(
            &QName::local("bitOr"),
            &[
                DfdlValue::Decimal(alloc::string::String::from("1")),
                DfdlValue::Long(1),
            ],
            &ctx,
        )
        .unwrap_err();
        assert!(err2.to_string().contains("xs:unsignedLong or xs:long"));
    }

    /// bitNot keeps the operand type, so unsigned/signed subtypes stay within their width.
    #[test]
    fn test_bit_not_preserves_operand_width() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);
        let not = |v: DfdlValue| eval_fn_call(&QName::local("bitNot"), &[v], &ctx).unwrap();
        assert_eq!(
            not(DfdlValue::UnsignedByte(1)),
            DfdlValue::UnsignedByte(254)
        );
        assert_eq!(
            not(DfdlValue::UnsignedShort(2)),
            DfdlValue::UnsignedShort(65533)
        );
        assert_eq!(
            not(DfdlValue::UnsignedInt(2)),
            DfdlValue::UnsignedInt(u32::MAX - 2)
        );
        assert_eq!(
            not(DfdlValue::UnsignedLong(0)),
            DfdlValue::UnsignedLong(u64::MAX)
        );
        assert_eq!(not(DfdlValue::Byte(1)), DfdlValue::Byte(-2));
        assert_eq!(not(DfdlValue::Int(3)), DfdlValue::Int(-4));
        assert_eq!(not(DfdlValue::Long(0)), DfdlValue::Long(-1));
    }

    #[test]
    fn test_path_past_root_error() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let doc = crate::infoset::InfosetDocument::new();
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // Attempting to evaluate path with '..' past root
        let ipath = crate::types::InfosetPath::parse("../foo");
        let res = resolve_path(&ipath, &ctx);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.to_string().contains("past root element"));
    }

    #[test]
    fn test_lookahead_negative_arguments_rejection() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let doc = crate::infoset::InfosetDocument::new();
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // 1. Negative offset
        let err1 = eval_fn_call(
            &QName::local("lookAhead"),
            &[DfdlValue::Int(-1), DfdlValue::Int(8)],
            &ctx,
        )
        .unwrap_err();
        assert!(err1.to_string().contains("out of range"));
        assert!(err1.to_string().contains("xs:unsignedInt"));

        // 2. Negative bitLength
        let err2 = eval_fn_call(
            &QName::local("lookAhead"),
            &[DfdlValue::Int(0), DfdlValue::Int(-1)],
            &ctx,
        )
        .unwrap_err();
        assert!(err2.to_string().contains("out of range"));
        assert!(err2.to_string().contains("xs:unsignedInt"));
    }

    #[test]
    fn test_nan_and_infinity_arithmetic() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let doc = crate::infoset::InfosetDocument::new();
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // fn:round(NaN), fn:floor(NaN), fn:ceiling(NaN) raise Parse Error on NaN in Daffodil
        let err_round =
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(f64::NAN)], &ctx).unwrap_err();
        assert!(err_round.to_string().contains("NaN"));

        let err_floor =
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(f32::NAN)], &ctx).unwrap_err();
        assert!(err_floor.to_string().contains("NaN"));

        let err_ceil = eval_fn_call(
            &QName::local("ceiling"),
            &[DfdlValue::Double(f64::NAN)],
            &ctx,
        )
        .unwrap_err();
        assert!(err_ceil.to_string().contains("NaN"));

        // idiv with NaN
        let err_idiv_nan =
            eval_idiv(DfdlValue::Double(f64::NAN), DfdlValue::Double(2.0)).unwrap_err();
        assert!(err_idiv_nan.to_string().contains("integer division"));
        assert!(err_idiv_nan.to_string().contains("NaN"));

        // idiv with Infinity
        let err_idiv_inf =
            eval_idiv(DfdlValue::Double(f64::INFINITY), DfdlValue::Double(2.0)).unwrap_err();
        assert!(err_idiv_inf.to_string().contains("integer division"));
        assert!(err_idiv_inf.to_string().contains("Infinity"));

        // idiv division by Infinity -> 0
        let res_idiv_zero =
            eval_idiv(DfdlValue::Double(5.0), DfdlValue::Double(f64::INFINITY)).unwrap();
        assert_eq!(res_idiv_zero, DfdlValue::Long(0));
    }

    #[test]
    fn test_substring_before_after_and_constructors() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // substring-before
        let res_sb = eval_fn_call(
            &QName::local("substring-before"),
            &[
                DfdlValue::String(String::from("192.168.1.1")),
                DfdlValue::String(String::from(".")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sb, DfdlValue::String(String::from("192")));

        // substring-before not found
        let res_sb_none = eval_fn_call(
            &QName::local("substring-before"),
            &[
                DfdlValue::String(String::from("hello")),
                DfdlValue::String(String::from("xyz")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sb_none, DfdlValue::String(String::new()));

        // substring-after
        let res_sa = eval_fn_call(
            &QName::local("substring-after"),
            &[
                DfdlValue::String(String::from("192.168.1.1")),
                DfdlValue::String(String::from(".")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sa, DfdlValue::String(String::from("168.1.1")));

        // date constructor
        let res_date = eval_fn_call(
            &QName::local("date"),
            &[DfdlValue::String(String::from("2026-10-03"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_date, DfdlValue::Date(String::from("2026-10-03")));

        // boolean constructor
        let res_b1 = eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(1)], &ctx).unwrap();
        assert_eq!(res_b1, DfdlValue::Boolean(true));
        let res_b0 = eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(0)], &ctx).unwrap();
        assert_eq!(res_b0, DfdlValue::Boolean(false));
    }

    #[test]
    fn test_calc_elem_value_length_text_numeric_coercion() {
        let elem = InfosetElement::simple(
            QName::local("z"),
            ElementState::Value(DfdlValue::String(String::from("5.0"))),
        );

        let props = crate::schema::ir::ResolvedProperties {
            representation: crate::schema::ir::Representation::Text,
            text_number_rep: crate::schema::ir::TextNumberRep::Standard,
            text_number_pattern: Some(String::from("'$'#0.0")),
            text_standard_decimal_separator: String::from("Z"),
            ..Default::default()
        };

        let term = crate::schema::ir::CompiledTerm {
            id: crate::schema::NodeId(0),
            name: QName::local("z"),
            kind: crate::schema::ir::TermKind::Element(crate::schema::ir::CompiledElement {
                name: QName::local("z"),
                type_ir: crate::schema::ir::CompiledType::Simple(
                    crate::infoset::DfdlSimpleType::Float,
                ),
                min_occurs: 1,
                max_occurs: Some(1),
                default_value: None,
                is_nillable: false,
            }),
            properties: props,
        };

        let schema = crate::schema::ir::CompiledSchema {
            root_element_id: crate::schema::NodeId(0),
            terms: alloc::vec![term],
            variable_map: Default::default(),
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
            max_skip_length_in_bytes: crate::limits::DEFAULT_MAX_SKIP_LENGTH_IN_BYTES,
            term_representation: Vec::new(),
        };

        let path = InfosetPath::parse("/z");
        let len = calc_elem_value_length(&elem, Some(&schema), &path, None);
        // "$5Z0" has 4 bytes
        assert_eq!(len, 4);
    }

    #[test]
    fn test_decode_and_encode_dfdl_entities_eval() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let decoded = eval_fn_call(
            &QName::local("decodeDFDLEntities"),
            &[DfdlValue::String("%NEL;%LF;".to_string())],
            &ctx,
        )
        .unwrap();
        assert_eq!(decoded, DfdlValue::String("\u{0085}\n".to_string()));

        let encoded = eval_fn_call(
            &QName::local("encodeDFDLEntities"),
            &[DfdlValue::String("\u{0085}\n".to_string())],
            &ctx,
        )
        .unwrap();
        assert_eq!(encoded, DfdlValue::String("%NEL;%LF;".to_string()));
    }

    #[test]
    fn test_left_and_right_shift_operations() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Int shifts
        let res_ls_int = eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::Int(4), DfdlValue::Int(2)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ls_int, DfdlValue::Int(16));

        let res_rs_int = eval_fn_call(
            &QName::local("rightShift"),
            &[DfdlValue::Int(16), DfdlValue::Int(2)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_rs_int, DfdlValue::Int(4));

        // UnsignedByte shifts
        let res_ls_ub = eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::UnsignedByte(4), DfdlValue::Int(2)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ls_ub, DfdlValue::UnsignedByte(16));

        // Out of bounds shift amount rejected with bit width in message
        let err_ls_byte = eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::Byte(2), DfdlValue::Int(8)],
            &ctx,
        )
        .unwrap_err();
        assert!(err_ls_byte.message.to_string().contains("8"));

        let err_rs_int = eval_fn_call(
            &QName::local("rightShift"),
            &[DfdlValue::Int(2), DfdlValue::Int(32)],
            &ctx,
        )
        .unwrap_err();
        assert!(err_rs_int.message.to_string().contains("32"));

        // Non-integer operand rejected
        let err_float = eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::Float(1.0), DfdlValue::Int(1)],
            &ctx,
        )
        .unwrap_err();
        assert!(err_float
            .message
            .to_string()
            .contains("xs:unsignedLong or xs:long"));
    }

    #[test]
    fn test_decimal_arithmetic_precision() {
        // Decimal mul
        let mul_res = eval_mul(
            DfdlValue::Decimal("9".to_string()),
            DfdlValue::Decimal("5".to_string()),
        )
        .unwrap();
        assert_eq!(mul_res, DfdlValue::Decimal("45".to_string()));

        // Decimal div
        let div_res =
            eval_div(DfdlValue::Decimal("45".to_string()), DfdlValue::Long(1600)).unwrap();
        assert_eq!(div_res, DfdlValue::Decimal("0.028125".to_string()));

        // Float division by zero produces IEEE 754 values, not errors
        let dbl_div_zero = eval_div(DfdlValue::Double(5.0), DfdlValue::Double(0.0)).unwrap();
        assert_eq!(dbl_div_zero, DfdlValue::Double(f64::INFINITY));

        let flt_div_zero = eval_div(DfdlValue::Float(5.0), DfdlValue::Float(0.0)).unwrap();
        assert_eq!(flt_div_zero, DfdlValue::Float(f32::INFINITY));
    }

    #[test]
    fn test_boolean_comparison_ordering() {
        assert_eq!(
            eval_cmp(
                "lt",
                DfdlValue::Boolean(false),
                DfdlValue::Boolean(true),
                |o| o.is_lt()
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_cmp(
                "lt",
                DfdlValue::Boolean(true),
                DfdlValue::Boolean(false),
                |o| o.is_lt()
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_cmp(
                "ge",
                DfdlValue::Boolean(true),
                DfdlValue::Boolean(true),
                |o| o.is_ge()
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_cmp(
                "le",
                DfdlValue::Boolean(false),
                DfdlValue::Boolean(false),
                |o| o.is_le()
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
    }

    #[test]
    fn test_date_time_extraction_functions() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let y_date = eval_fn_call(
            &QName::local("year-from-date"),
            &[DfdlValue::Date(String::from("2026-10-05"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(y_date, DfdlValue::Long(2026));

        let m_date = eval_fn_call(
            &QName::local("month-from-date"),
            &[DfdlValue::Date(String::from("2026-10-05"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(m_date, DfdlValue::Long(10));

        let d_date = eval_fn_call(
            &QName::local("day-from-date"),
            &[DfdlValue::Date(String::from("2026-10-05"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(d_date, DfdlValue::Long(5));

        let y_dt = eval_fn_call(
            &QName::local("year-from-dateTime"),
            &[DfdlValue::DateTime(String::from(
                "1999-05-31T13:20:05.18-05:00",
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(y_dt, DfdlValue::Long(1999));

        let sec_dt = eval_fn_call(
            &QName::local("seconds-from-dateTime"),
            &[DfdlValue::DateTime(String::from(
                "1999-05-31T13:20:05.18-05:00",
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(sec_dt, DfdlValue::Decimal(String::from("5.18")));

        let h_time = eval_fn_call(
            &QName::local("hours-from-time"),
            &[DfdlValue::Time(String::from("13:20:00Z"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(h_time, DfdlValue::Long(13));

        // Type misuse should return SchemaDefinition error
        let err_misuse = eval_fn_call(
            &QName::local("year-from-date"),
            &[DfdlValue::DateTime(String::from("1999-05-31T13:20:00"))],
            &ctx,
        )
        .unwrap_err();
        assert!(err_misuse
            .message
            .as_str()
            .contains("DateTime cannot be converted to Date"));
    }

    /// Verifies unary plus and unary minus operations across all numeric and non-numeric types.
    #[test]
    fn test_eval_unary_all_variants_and_errors() {
        // Unary plus on various types
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Int(10)).unwrap(),
            DfdlValue::Int(10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Long(20)).unwrap(),
            DfdlValue::Long(20)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Short(5)).unwrap(),
            DfdlValue::Short(5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Byte(2)).unwrap(),
            DfdlValue::Byte(2)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::UnsignedLong(100)).unwrap(),
            DfdlValue::UnsignedLong(100)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::UnsignedInt(50)).unwrap(),
            DfdlValue::UnsignedInt(50)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::UnsignedShort(25)).unwrap(),
            DfdlValue::UnsignedShort(25)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::UnsignedByte(10)).unwrap(),
            DfdlValue::UnsignedByte(10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Float(1.5)).unwrap(),
            DfdlValue::Float(1.5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Plus, DfdlValue::Decimal(String::from("42.5"))).unwrap(),
            DfdlValue::Decimal(String::from("42.5"))
        );

        // Invalid type for unary plus
        let err_plus =
            eval_unary(UnaryOp::Plus, DfdlValue::String(String::from("abc"))).unwrap_err();
        assert_eq!(err_plus.kind, DFDLErrorKind::TypeError);

        // Unary negate on scalar types
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Int(10)).unwrap(),
            DfdlValue::Int(-10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Long(20)).unwrap(),
            DfdlValue::Long(-20)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Short(5)).unwrap(),
            DfdlValue::Short(-5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Byte(2)).unwrap(),
            DfdlValue::Byte(-2)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedInt(50)).unwrap(),
            DfdlValue::Long(-50)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedShort(25)).unwrap(),
            DfdlValue::Int(-25)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedByte(10)).unwrap(),
            DfdlValue::Int(-10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Float(1.5)).unwrap(),
            DfdlValue::Float(-1.5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(-2.5)
        );

        // UnsignedLong negation edge cases: i64::MIN boundary, small u64, and large u64
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(100)).unwrap(),
            DfdlValue::Long(-100)
        );
        assert_eq!(
            eval_unary(
                UnaryOp::Negate,
                DfdlValue::UnsignedLong(9_223_372_036_854_775_808)
            )
            .unwrap(),
            DfdlValue::Long(i64::MIN)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(u64::MAX)).unwrap(),
            DfdlValue::Decimal(alloc::format!("-{}", u64::MAX))
        );

        // Decimal negation (both positive and negative values)
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Decimal(String::from("10.5"))).unwrap(),
            DfdlValue::Decimal(String::from("-10.5"))
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Decimal(String::from("-10.5"))).unwrap(),
            DfdlValue::Decimal(String::from("10.5"))
        );

        // Negation overflow errors
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Int(i32::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Long(i64::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Short(i16::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(i8::MIN)).is_err());

        // Invalid type for unary negate
        let err_neg = eval_unary(UnaryOp::Negate, DfdlValue::Boolean(true)).unwrap_err();
        assert_eq!(err_neg.kind, DFDLErrorKind::TypeError);

        // Unary not operator
        assert_eq!(
            eval_unary(UnaryOp::Not, DfdlValue::Boolean(true)).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_unary(UnaryOp::Not, DfdlValue::Boolean(false)).unwrap(),
            DfdlValue::Boolean(true)
        );
        let err_not = eval_unary(UnaryOp::Not, DfdlValue::Int(1)).unwrap_err();
        assert_eq!(err_not.kind, DFDLErrorKind::TypeError);
    }

    /// Verifies coercion of numeric representations to long or double.
    #[test]
    fn test_coerce_numeric_all_variants() {
        assert_eq!(coerce_numeric(DfdlValue::Int(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::Short(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::Byte(5)), DfdlValue::Long(5));
        assert_eq!(
            coerce_numeric(DfdlValue::UnsignedInt(5)),
            DfdlValue::Long(5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::UnsignedShort(5)),
            DfdlValue::Long(5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::UnsignedByte(5)),
            DfdlValue::Long(5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::UnsignedLong(5)),
            DfdlValue::Long(5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::Float(1.5)),
            DfdlValue::Double(1.5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::String(String::from("100"))),
            DfdlValue::Long(100)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::String(String::from("2.5"))),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            coerce_numeric(DfdlValue::String(String::from("not_num"))),
            DfdlValue::String(String::from("not_num"))
        );
    }

    /// Verifies binary arithmetic operations (add, sub, mul, idiv, mod) across numeric types.
    #[test]
    fn test_eval_arithmetic_combinations() {
        use crate::expr::ast::BinaryOp;

        // Additions
        assert_eq!(
            eval_add(DfdlValue::Int(5), DfdlValue::Int(10)).unwrap(),
            DfdlValue::Int(15)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(5), DfdlValue::Long(10)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_add(DfdlValue::Int(5), DfdlValue::Long(10)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(10), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_add(DfdlValue::Float(1.0), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(3.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Double(1.0), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(3.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(5), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Double(2.5), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(5), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(7.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Float(2.5), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Float(7.5)
        );
        assert_eq!(
            eval_add(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Decimal(String::from("5.25"))
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("15.75"))
        );
        assert_eq!(
            eval_add(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Decimal(String::from("15.5"))
        );
        assert_eq!(
            eval_add(DfdlValue::Long(5), DfdlValue::Decimal(String::from("10.5"))).unwrap(),
            DfdlValue::Decimal(String::from("15.5"))
        );
        assert_eq!(
            eval_add(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Double(2.0)
            )
            .unwrap(),
            DfdlValue::Double(12.5)
        );
        assert_eq!(
            eval_add(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Float(2.0)
            )
            .unwrap(),
            DfdlValue::Float(12.5)
        );
        assert_eq!(
            eval_add(
                DfdlValue::Double(2.0),
                DfdlValue::Decimal(String::from("10.5"))
            )
            .unwrap(),
            DfdlValue::Double(12.5)
        );
        assert_eq!(
            eval_add(
                DfdlValue::Float(2.0),
                DfdlValue::Decimal(String::from("10.5"))
            )
            .unwrap(),
            DfdlValue::Float(12.5)
        );
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Int(1)).is_err());
        assert!(eval_add(DfdlValue::Long(i64::MAX), DfdlValue::Long(1)).is_err());
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Long(i64::MAX)).is_err());
        assert!(eval_add(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Arithmetic via eval_binary which performs coerce_numeric on Short/Byte/Unsigned*
        assert_eq!(
            eval_binary(BinaryOp::Add, DfdlValue::Short(5), DfdlValue::Short(10)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_binary(BinaryOp::Add, DfdlValue::Byte(5), DfdlValue::Byte(10)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Add,
                DfdlValue::UnsignedLong(5),
                DfdlValue::UnsignedLong(10)
            )
            .unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Add,
                DfdlValue::UnsignedInt(5),
                DfdlValue::UnsignedInt(10)
            )
            .unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Add,
                DfdlValue::UnsignedShort(5),
                DfdlValue::UnsignedShort(10)
            )
            .unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Add,
                DfdlValue::UnsignedByte(5),
                DfdlValue::UnsignedByte(10)
            )
            .unwrap(),
            DfdlValue::Long(15)
        );

        // Subtractions
        assert_eq!(
            eval_sub(DfdlValue::Int(10), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Int(5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Long(5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Int(10), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Long(5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Long(5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Float(5.0), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(2.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Double(5.0), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(5), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Double(5.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(3.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(5), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(2.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Float(5.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Float(3.0)
        );
        assert_eq!(
            eval_sub(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Decimal(String::from("5.25"))
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("5.25"))
        );
        assert_eq!(
            eval_sub(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Decimal(String::from("5.5"))
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Decimal(String::from("5.5"))).unwrap(),
            DfdlValue::Decimal(String::from("4.5"))
        );
        assert_eq!(
            eval_sub(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Double(2.0)
            )
            .unwrap(),
            DfdlValue::Double(8.5)
        );
        assert_eq!(
            eval_sub(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Float(2.0)
            )
            .unwrap(),
            DfdlValue::Float(8.5)
        );
        assert_eq!(
            eval_sub(
                DfdlValue::Double(10.5),
                DfdlValue::Decimal(String::from("2.0"))
            )
            .unwrap(),
            DfdlValue::Double(8.5)
        );
        assert_eq!(
            eval_sub(
                DfdlValue::Float(10.5),
                DfdlValue::Decimal(String::from("2.0"))
            )
            .unwrap(),
            DfdlValue::Float(8.5)
        );
        assert!(eval_sub(DfdlValue::Int(i32::MIN), DfdlValue::Int(1)).is_err());
        assert!(eval_sub(DfdlValue::Long(i64::MIN), DfdlValue::Long(1)).is_err());
        assert!(eval_sub(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Multiplications
        assert_eq!(
            eval_mul(DfdlValue::Int(5), DfdlValue::Int(3)).unwrap(),
            DfdlValue::Int(15)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(5), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_mul(DfdlValue::Int(5), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(5), DfdlValue::Int(3)).unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_mul(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Double(2.0), DfdlValue::Double(3.0)).unwrap(),
            DfdlValue::Double(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(2), DfdlValue::Double(3.0)).unwrap(),
            DfdlValue::Double(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Double(2.0), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Double(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(2), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Float(2.0), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Float(6.0)
        );
        assert_eq!(
            eval_mul(
                DfdlValue::Decimal(String::from("2.5")),
                DfdlValue::Decimal(String::from("3.0"))
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("7.5"))
        );
        assert_eq!(
            eval_mul(DfdlValue::Decimal(String::from("2.5")), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Decimal(String::from("7.5"))
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(3), DfdlValue::Decimal(String::from("2.5"))).unwrap(),
            DfdlValue::Decimal(String::from("7.5"))
        );
        assert_eq!(
            eval_mul(
                DfdlValue::Decimal(String::from("2.5")),
                DfdlValue::Double(3.0)
            )
            .unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_mul(
                DfdlValue::Decimal(String::from("2.5")),
                DfdlValue::Float(3.0)
            )
            .unwrap(),
            DfdlValue::Float(7.5)
        );
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Int(2)).is_err());
        assert!(eval_mul(DfdlValue::Long(i64::MAX), DfdlValue::Long(2)).is_err());
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Long(i64::MAX)).is_err());
        assert!(eval_mul(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Regular division (div)
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(10), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(10), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(10), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(10), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(6), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(5), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(6), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Int(3)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(5), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(6), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(5), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(6), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(5), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Int(10), DfdlValue::Int(0)).is_err());
        assert!(eval_div(DfdlValue::Int(10), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Int(0)).is_err());
        assert!(eval_div(DfdlValue::Decimal(String::from("10")), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Decimal(String::from("0"))).is_err());
        assert_eq!(
            eval_div(
                DfdlValue::Decimal(String::from("10")),
                DfdlValue::Double(2.0)
            )
            .unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(
                DfdlValue::Decimal(String::from("10")),
                DfdlValue::Float(2.0)
            )
            .unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(
                DfdlValue::Double(10.0),
                DfdlValue::Decimal(String::from("2"))
            )
            .unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(
                DfdlValue::Float(10.0),
                DfdlValue::Decimal(String::from("2"))
            )
            .unwrap(),
            DfdlValue::Float(5.0)
        );
        assert!(eval_div(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Integer division (idiv)
        assert_eq!(
            eval_idiv(DfdlValue::Long(10), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_idiv(DfdlValue::Int(10), DfdlValue::Int(3)).unwrap(),
            DfdlValue::Int(3)
        );
        assert_eq!(
            eval_idiv(DfdlValue::Double(10.5), DfdlValue::Double(3.2)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_idiv(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Long(3)
        );
        assert_eq!(
            eval_idiv(DfdlValue::Long(10), DfdlValue::Decimal(String::from("3.0"))).unwrap(),
            DfdlValue::Long(3)
        );
        assert!(eval_idiv(DfdlValue::Double(f64::NAN), DfdlValue::Double(3.0)).is_err());
        assert!(eval_idiv(DfdlValue::Double(f64::INFINITY), DfdlValue::Double(3.0)).is_err());
        assert_eq!(
            eval_idiv(DfdlValue::Double(3.0), DfdlValue::Double(f64::INFINITY)).unwrap(),
            DfdlValue::Long(0)
        );
        assert!(eval_idiv(DfdlValue::Double(10.0), DfdlValue::Double(0.0)).is_err());
        assert!(eval_idiv(
            DfdlValue::Decimal(String::from("10.0")),
            DfdlValue::Decimal(String::from("0.0"))
        )
        .is_err());
        assert!(eval_idiv(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_idiv(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Modulo (mod)
        assert_eq!(
            eval_mod(DfdlValue::Long(10), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Long(1)
        );
        assert_eq!(
            eval_mod(DfdlValue::Int(10), DfdlValue::Int(3)).unwrap(),
            DfdlValue::Int(1)
        );
        assert_eq!(
            eval_mod(DfdlValue::Double(10.5), DfdlValue::Double(3.0)).unwrap(),
            DfdlValue::Double(1.5)
        );
        assert_eq!(
            eval_mod(DfdlValue::Float(10.5), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(1.5)
        );
        assert_eq!(
            eval_mod(
                DfdlValue::Decimal(String::from("10.5")),
                DfdlValue::Decimal(String::from("3.0"))
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("1.5"))
        );
        assert!(eval_mod(DfdlValue::Double(10.5), DfdlValue::Double(0.0)).is_err());
        assert!(eval_mod(DfdlValue::Float(10.5), DfdlValue::Float(0.0)).is_err());
        assert!(eval_mod(
            DfdlValue::Decimal(String::from("10.5")),
            DfdlValue::Decimal(String::from("0.0"))
        )
        .is_err());
        assert!(eval_mod(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_mod(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());
    }

    /// Verifies relational comparisons across scalar types (strings, decimals, booleans, floats).
    #[test]
    fn test_eval_relational_comparisons() {
        use crate::expr::ast::BinaryOp;

        // String ordering
        assert_eq!(
            eval_binary(
                BinaryOp::Lt,
                DfdlValue::String(String::from("apple")),
                DfdlValue::String(String::from("banana"))
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Gt,
                DfdlValue::String(String::from("apple")),
                DfdlValue::String(String::from("banana"))
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        // Float comparisons
        assert_eq!(
            eval_binary(BinaryOp::Lt, DfdlValue::Float(1.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Le, DfdlValue::Float(2.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Gt, DfdlValue::Float(3.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Ge, DfdlValue::Float(2.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );

        // Double comparisons
        assert_eq!(
            eval_binary(BinaryOp::Lt, DfdlValue::Double(1.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Le, DfdlValue::Double(2.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Gt, DfdlValue::Double(3.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(BinaryOp::Ge, DfdlValue::Double(2.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Boolean(true)
        );

        // Decimal comparisons
        assert_eq!(
            eval_binary(
                BinaryOp::Lt,
                DfdlValue::Decimal(String::from("1.5")),
                DfdlValue::Decimal(String::from("2.5"))
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Gt,
                DfdlValue::Decimal(String::from("3.5")),
                DfdlValue::Decimal(String::from("2.5"))
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );

        // Boolean comparisons
        assert_eq!(
            eval_binary(
                BinaryOp::Lt,
                DfdlValue::Boolean(false),
                DfdlValue::Boolean(true)
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_binary(
                BinaryOp::Gt,
                DfdlValue::Boolean(true),
                DfdlValue::Boolean(false)
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
    }

    /// Verifies boolean test evaluation (`value_to_bool`) on all DfdlValue variants.
    #[test]
    fn test_eval_boolean_test_all_variants() {
        assert!(!value_to_bool(&DfdlValue::Boolean(false)));
        assert!(value_to_bool(&DfdlValue::Boolean(true)));
        assert!(!value_to_bool(&DfdlValue::Int(0)));
        assert!(value_to_bool(&DfdlValue::Int(42)));
        assert!(!value_to_bool(&DfdlValue::Long(0)));
        assert!(value_to_bool(&DfdlValue::Long(100)));
        assert!(!value_to_bool(&DfdlValue::Short(0)));
        assert!(value_to_bool(&DfdlValue::Short(5)));
        assert!(!value_to_bool(&DfdlValue::Byte(0)));
        assert!(value_to_bool(&DfdlValue::Byte(1)));
        assert!(!value_to_bool(&DfdlValue::UnsignedLong(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedLong(5)));
        assert!(!value_to_bool(&DfdlValue::UnsignedInt(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedInt(5)));
        assert!(!value_to_bool(&DfdlValue::UnsignedShort(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedShort(5)));
        assert!(!value_to_bool(&DfdlValue::UnsignedByte(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedByte(5)));
        assert!(!value_to_bool(&DfdlValue::Float(0.0)));
        assert!(!value_to_bool(&DfdlValue::Float(f32::NAN)));
        assert!(value_to_bool(&DfdlValue::Float(1.5)));
        assert!(!value_to_bool(&DfdlValue::Double(0.0)));
        assert!(!value_to_bool(&DfdlValue::Double(f64::NAN)));
        assert!(value_to_bool(&DfdlValue::Double(2.5)));
        assert!(!value_to_bool(&DfdlValue::String(String::new())));
        assert!(value_to_bool(&DfdlValue::String(String::from("true"))));
        assert!(value_to_bool(&DfdlValue::String(String::from("1"))));
        assert!(!value_to_bool(&DfdlValue::String(String::from("false"))));
    }

    /// Verifies date/time string extraction and conversion rules via extract_date_str.
    #[test]
    fn test_extract_date_str_all_branches() {
        // Direct matching variants
        assert_eq!(
            extract_date_str(&DfdlValue::Date(String::from("2026-10-07")), "Date").unwrap(),
            "2026-10-07"
        );
        assert_eq!(
            extract_date_str(
                &DfdlValue::DateTime(String::from("2026-10-07T12:00:00")),
                "DateTime"
            )
            .unwrap(),
            "2026-10-07T12:00:00"
        );
        assert_eq!(
            extract_date_str(&DfdlValue::Time(String::from("12:00:00")), "Time").unwrap(),
            "12:00:00"
        );

        // String conversions
        assert_eq!(
            extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "Date").unwrap(),
            "2026-10-07"
        );
        assert_eq!(
            extract_date_str(&DfdlValue::String(String::from("12:00:00")), "Time").unwrap(),
            "12:00:00"
        );
        assert_eq!(
            extract_date_str(
                &DfdlValue::String(String::from("2026-10-07T12:00:00")),
                "DateTime"
            )
            .unwrap(),
            "2026-10-07T12:00:00"
        );

        // Errors: Date cannot be converted to Time / DateTime
        assert!(extract_date_str(&DfdlValue::Date(String::from("2026-10-07")), "Time").is_err());
        assert!(extract_date_str(
            &DfdlValue::DateTime(String::from("2026-10-07T12:00:00")),
            "Date"
        )
        .is_err());
        assert!(extract_date_str(&DfdlValue::Time(String::from("12:00:00")), "DateTime").is_err());

        // String mismatch errors
        assert!(extract_date_str(
            &DfdlValue::String(String::from("2026-10-07T12:00:00")),
            "Date"
        )
        .is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("12:00:00")), "Date").is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "Time").is_err());
        assert!(
            extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "DateTime").is_err()
        );
        assert!(
            extract_date_str(&DfdlValue::String(String::from("12:00:00")), "DateTime").is_err()
        );

        // Incompatible types
        assert!(extract_date_str(&DfdlValue::Int(42), "Date").is_err());
    }

    /// Verifies built-in constructor functions in eval_fn_call.
    #[test]
    fn test_eval_fn_call_constructors() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        assert_eq!(
            eval_fn_call(
                &QName::local("int"),
                &[DfdlValue::String(String::from("123"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(123)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("long"),
                &[DfdlValue::String(String::from("456"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(456)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("short"),
                &[DfdlValue::String(String::from("12"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Short(12)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("byte"),
                &[DfdlValue::String(String::from("7"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Byte(7)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("unsignedLong"),
                &[DfdlValue::String(String::from("100"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(100)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("unsignedInt"),
                &[DfdlValue::String(String::from("50"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedInt(50)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("unsignedShort"),
                &[DfdlValue::String(String::from("25"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedShort(25)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("unsignedByte"),
                &[DfdlValue::String(String::from("10"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedByte(10)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::String(String::from("123.456"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(123.456)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("float"),
                &[DfdlValue::String(String::from("1.5"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(1.5)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("decimal"),
                &[DfdlValue::String(String::from("99.9"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("99.9"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::String(String::from("true"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::String(String::from("false"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("string"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::String(String::from("42"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::String(String::from("ABCD"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0xAB, 0xCD])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("dateTime"),
                &[DfdlValue::String(String::from("2026-10-07T12:00:00"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::DateTime(String::from("2026-10-07T12:00:00"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("date"),
                &[DfdlValue::String(String::from("2026-10-07"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Date(String::from("2026-10-07"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("time"),
                &[DfdlValue::String(String::from("12:00:00"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Time(String::from("12:00:00"))
        );
    }

    /// Verifies built-in XPath/DFDL functions in eval_fn_call.
    #[test]
    fn test_eval_fn_call_builtins() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // String functions
        let res_len = eval_fn_call(
            &QName::local("string-length"),
            &[DfdlValue::String(String::from("hello"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_len, DfdlValue::Long(5));

        let res_upper = eval_fn_call(
            &QName::local("upper-case"),
            &[DfdlValue::String(String::from("dfdl"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_upper, DfdlValue::String(String::from("DFDL")));

        let res_lower = eval_fn_call(
            &QName::local("lower-case"),
            &[DfdlValue::String(String::from("DFDL"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_lower, DfdlValue::String(String::from("dfdl")));

        let res_sub = eval_fn_call(
            &QName::local("substring"),
            &[
                DfdlValue::String(String::from("motorcycle")),
                DfdlValue::Long(3),
                DfdlValue::Long(5),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sub, DfdlValue::String(String::from("torcy")));

        let res_sub_bef = eval_fn_call(
            &QName::local("substring-before"),
            &[
                DfdlValue::String(String::from("tattoo")),
                DfdlValue::String(String::from("atto")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sub_bef, DfdlValue::String(String::from("t")));

        let res_sub_aft = eval_fn_call(
            &QName::local("substring-after"),
            &[
                DfdlValue::String(String::from("tattoo")),
                DfdlValue::String(String::from("tat")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sub_aft, DfdlValue::String(String::from("too")));

        let res_cont = eval_fn_call(
            &QName::local("contains"),
            &[
                DfdlValue::String(String::from("banana")),
                DfdlValue::String(String::from("nan")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_cont, DfdlValue::Boolean(true));

        let res_sw = eval_fn_call(
            &QName::local("starts-with"),
            &[
                DfdlValue::String(String::from("banana")),
                DfdlValue::String(String::from("ban")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sw, DfdlValue::Boolean(true));

        let res_ew = eval_fn_call(
            &QName::local("ends-with"),
            &[
                DfdlValue::String(String::from("banana")),
                DfdlValue::String(String::from("na")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ew, DfdlValue::Boolean(true));

        // Math functions
        assert_eq!(
            eval_fn_call(&QName::local("abs"), &[DfdlValue::Long(-42)], &ctx).unwrap(),
            DfdlValue::Long(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap(),
            DfdlValue::Double(4.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap(),
            DfdlValue::Double(3.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.5)], &ctx).unwrap(),
            DfdlValue::Double(4.0)
        );

        // Logic & check functions
        assert_eq!(
            eval_fn_call(
                &QName::local("exists"),
                &[DfdlValue::String(String::from("nonexistent"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );

        // Matches
        let res_mat = eval_fn_call(
            &QName::local("matches"),
            &[
                DfdlValue::String(String::from("abracadabra")),
                DfdlValue::String(String::from("cadabra")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_mat, DfdlValue::Boolean(true));

        // Entity encoding and decoding
        let res_enc = eval_fn_call(
            &QName::local("encodeDFDLEntities"),
            &[DfdlValue::String(String::from("hello%world"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_enc, DfdlValue::String(String::from("hello%%world")));

        let res_dec = eval_fn_call(
            &QName::local("decodeDFDLEntities"),
            &[DfdlValue::String(String::from("hello%%world"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_dec, DfdlValue::String(String::from("hello%world")));
    }

    /// Verifies value length calculation on infoset elements.
    #[test]
    fn test_calc_elem_value_length_recursive() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::types::QName;

        let child1 = InfosetElement::simple(
            QName::local("child1"),
            ElementState::Value(DfdlValue::String(String::from("12345"))),
        );
        let child2 = InfosetElement::simple(
            QName::local("child2"),
            ElementState::Value(DfdlValue::Int(42)),
        );

        let mut parent = InfosetElement::complex(QName::local("parent"));
        parent.children.push(InfosetNode::Element(child1.clone()));
        parent.children.push(InfosetNode::Element(child2));

        let doc = InfosetDocument {
            root: Some(parent.clone()),
            total_nodes: 3,
        };

        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // Value length on child1
        let vl_child = calc_elem_value_length(&child1, None, &path, Some(&ctx));
        assert_eq!(vl_child, 5);

        // Value length on root parent element (sum of children: child1 (5) + child2 (4-byte int) = 9)
        let vl_parent = calc_elem_value_length(&parent, None, &path, Some(&ctx));
        assert_eq!(vl_parent, 9);
    }

    /// Verifies expression evaluation depth limits, variable lookups, and all if-then-else conditions.
    #[test]
    fn test_eval_expr_ast_all_variants_and_depth_limit() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let var_val = ("my_var", DfdlValue::Int(99));
        let vars = [var_val];
        let mut ctx = ExprContext::new(None, &path, &vars, &mut budget);

        // Depth limit exceeded error
        ctx.depth = MAX_EVAL_DEPTH;
        let depth_err = eval_expr(&ExprAst::Literal(DfdlValue::Int(1)), &mut ctx).unwrap_err();
        assert_eq!(depth_err.kind, DFDLErrorKind::ExpressionError);
        assert!(depth_err.message.as_str().contains("depth limit"));
        ctx.depth = 0;

        // Variable lookup success and failure
        let val_var = eval_expr(&ExprAst::Variable(QName::local("my_var")), &mut ctx).unwrap();
        assert_eq!(val_var, DfdlValue::Int(99));
        let err_var =
            eval_expr(&ExprAst::Variable(QName::local("unknown_var")), &mut ctx).unwrap_err();
        assert_eq!(err_var.kind, DFDLErrorKind::ExpressionError);

        // IfThenElse conditions across all scalar variants
        let then_lit = Box::new(ExprAst::Literal(DfdlValue::String(String::from("THEN"))));
        let else_lit = Box::new(ExprAst::Literal(DfdlValue::String(String::from("ELSE"))));

        let eval_cond = |cond_val: DfdlValue, c: &mut ExprContext| -> String {
            let ast = ExprAst::IfThenElse {
                cond: Box::new(ExprAst::Literal(cond_val)),
                then_expr: then_lit.clone(),
                else_expr: else_lit.clone(),
            };
            match eval_expr(&ast, c).unwrap() {
                DfdlValue::String(s) => s,
                _ => String::new(),
            }
        };

        assert_eq!(eval_cond(DfdlValue::Boolean(true), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Boolean(false), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Int(1), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Int(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Long(5), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Long(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Short(2), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Short(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Byte(3), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Byte(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::UnsignedLong(10), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::UnsignedLong(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::UnsignedInt(10), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::UnsignedInt(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::UnsignedShort(10), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::UnsignedShort(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::UnsignedByte(10), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::UnsignedByte(0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Float(1.5), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Float(0.0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Float(f32::NAN), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Double(2.5), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::Double(0.0), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::Double(f64::NAN), &mut ctx), "ELSE");
        assert_eq!(
            eval_cond(DfdlValue::String(String::from("true")), &mut ctx),
            "THEN"
        );
        assert_eq!(
            eval_cond(DfdlValue::String(String::from("1")), &mut ctx),
            "THEN"
        );
        assert_eq!(
            eval_cond(DfdlValue::String(String::from("false")), &mut ctx),
            "ELSE"
        );
        assert_eq!(
            eval_cond(DfdlValue::String(String::from("0")), &mut ctx),
            "ELSE"
        );
        assert_eq!(
            eval_cond(DfdlValue::String(String::new()), &mut ctx),
            "ELSE"
        );
        assert_eq!(
            eval_cond(DfdlValue::String(String::from("nonempty")), &mut ctx),
            "THEN"
        );

        // Invalid condition type
        let bad_ast = ExprAst::IfThenElse {
            cond: Box::new(ExprAst::Literal(DfdlValue::HexBinary(alloc::vec![0xAA]))),
            then_expr: then_lit,
            else_expr: else_lit,
        };
        let err_bad = eval_expr(&bad_ast, &mut ctx).unwrap_err();
        assert_eq!(err_bad.kind, DFDLErrorKind::TypeError);
    }

    /// Verifies builder pattern methods on ExprContext and lookup of elements.
    #[test]
    fn test_eval_context_builder_methods() {
        use crate::types::UnqualifiedPathStepPolicy;

        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget)
            .with_unqualified_path_step_policy(UnqualifiedPathStepPolicy::PreferDefaultNamespace)
            .with_occurs_index(3)
            .for_unparsing();

        assert_eq!(ctx.occurs_index, 3);
        assert!(!ctx.is_parsing);
        assert_eq!(
            ctx.unqualified_path_step_policy,
            UnqualifiedPathStepPolicy::PreferDefaultNamespace
        );
        assert!(ctx.find_element(&path).is_none());
    }

    /// Verifies primitive, boxed, and interoperability functions in eval_fn_call.
    #[test]
    fn test_eval_builtin_all_primitives_and_interop_funcs() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget).with_occurs_index(7);

        // Primitive and boxed integer functions
        assert_eq!(
            eval_fn_call(&QName::local("primByteFunc"), &[DfdlValue::Int(120)], &ctx).unwrap(),
            DfdlValue::Byte(120)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boxedByteFunc"), &[DfdlValue::Int(121)], &ctx).unwrap(),
            DfdlValue::Byte(121)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("primByteArrayFunc"),
                &[DfdlValue::String(String::from("data"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::String(String::from("data"))
        );
        assert_eq!(
            eval_fn_call(&QName::local("primShortFunc"), &[DfdlValue::Int(300)], &ctx).unwrap(),
            DfdlValue::Short(300)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boxedShortFunc"),
                &[DfdlValue::Int(301)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Short(301)
        );
        assert_eq!(
            eval_fn_call(&QName::local("primLongFunc"), &[DfdlValue::Int(500)], &ctx).unwrap(),
            DfdlValue::Long(500)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boxedLongFunc"), &[DfdlValue::Int(501)], &ctx).unwrap(),
            DfdlValue::Long(501)
        );

        // Floating point and boolean primitive functions
        assert_eq!(
            eval_fn_call(
                &QName::local("primDoubleFunc"),
                &[DfdlValue::String(String::from("98.765"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(98.765)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boxedDoubleFunc"),
                &[DfdlValue::Int(42)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(42.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("primFloatFunc"),
                &[DfdlValue::String(String::from("2.5"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(2.5)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boxedFloatFunc"), &[DfdlValue::Int(10)], &ctx).unwrap(),
            DfdlValue::Float(10.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("primBooleanFunc"), &[DfdlValue::Int(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boxedBooleanFunc"),
                &[DfdlValue::Int(0)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        // Java BigInteger / BigDecimal interop
        assert_eq!(
            eval_fn_call(
                &QName::local("javaBigDecimalFunc"),
                &[DfdlValue::String(String::from("123.456"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("123.456"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("javaBigIntegerFunc"),
                &[DfdlValue::String(String::from("999999"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("999999"))
        );

        // Greeting and arithmetic helper functions
        assert_eq!(
            eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap(),
            DfdlValue::String(String::from("Hello"))
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addBoxed"),
                &[DfdlValue::Int(10), DfdlValue::Int(20)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(30)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addPrimitive"),
                &[DfdlValue::String(String::from("15")), DfdlValue::Long(5)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(20)
        );
        assert!(eval_fn_call(
            &QName::local("addPrimitive"),
            &[DfdlValue::Boolean(true), DfdlValue::Int(5)],
            &ctx
        )
        .is_err());

        // String compare function
        assert_eq!(
            eval_fn_call(
                &QName::local("compare"),
                &[
                    DfdlValue::String(String::from("a")),
                    DfdlValue::String(String::from("b"))
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(-1)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("compare"),
                &[
                    DfdlValue::String(String::from("a")),
                    DfdlValue::String(String::from("a"))
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("compare"),
                &[
                    DfdlValue::String(String::from("b")),
                    DfdlValue::String(String::from("a"))
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(1)
        );

        // Constant functions
        assert_eq!(
            eval_fn_call(&QName::local("true"), &[], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("false"), &[], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );

        // Occurs index and position
        assert_eq!(
            eval_fn_call(&QName::local("occursIndex"), &[], &ctx).unwrap(),
            DfdlValue::Long(7)
        );
        assert_eq!(
            eval_fn_call(&QName::local("currentPosition"), &[], &ctx).unwrap(),
            DfdlValue::Long(0)
        );

        // Count function
        assert_eq!(
            eval_fn_call(
                &QName::local("count"),
                &[DfdlValue::String(String::new())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("count"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::Long(1)
        );
        assert_eq!(
            eval_fn_call(&QName::local("count"), &[], &ctx).unwrap(),
            DfdlValue::Long(0)
        );
    }

    /// Verifies bitwise operations (bitAnd, bitOr, bitXor, bitNot, shift) and signedness constraints.
    #[test]
    fn test_bitwise_all_variants() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // bitAnd matching signedness
        assert_eq!(
            eval_fn_call(
                &QName::local("bitAnd"),
                &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(0b1000)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitAnd"),
                &[
                    DfdlValue::UnsignedLong(0b1100),
                    DfdlValue::UnsignedLong(0b1010)
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(0b1000)
        );
        let err_and = eval_fn_call(
            &QName::local("bitAnd"),
            &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)],
            &ctx,
        )
        .unwrap_err();
        assert_eq!(err_and.kind, DFDLErrorKind::SchemaDefinition);

        // bitOr matching signedness
        assert_eq!(
            eval_fn_call(
                &QName::local("bitOr"),
                &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(0b1110)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitOr"),
                &[
                    DfdlValue::UnsignedLong(0b1100),
                    DfdlValue::UnsignedLong(0b1010)
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(0b1110)
        );
        let err_or = eval_fn_call(
            &QName::local("bitOr"),
            &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)],
            &ctx,
        )
        .unwrap_err();
        assert_eq!(err_or.kind, DFDLErrorKind::SchemaDefinition);

        // bitXor matching signedness
        assert_eq!(
            eval_fn_call(
                &QName::local("bitXor"),
                &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(0b0110)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitXor"),
                &[
                    DfdlValue::UnsignedLong(0b1100),
                    DfdlValue::UnsignedLong(0b1010)
                ],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(0b0110)
        );
        let err_xor = eval_fn_call(
            &QName::local("bitXor"),
            &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)],
            &ctx,
        )
        .unwrap_err();
        assert_eq!(err_xor.kind, DFDLErrorKind::SchemaDefinition);

        // bitNot on all numeric variants
        assert_eq!(
            eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Byte(0x0F)], &ctx).unwrap(),
            DfdlValue::Byte(-16)
        );
        assert_eq!(
            eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Short(0x00FF)], &ctx).unwrap(),
            DfdlValue::Short(!0x00FF)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::Int(0x0000_FFFF)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(!0x0000_FFFF)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::Long(0x0000_0000_FFFF_FFFF)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(!0x0000_0000_FFFF_FFFF)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::UnsignedByte(0x0F)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedByte(0xF0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::UnsignedShort(0x00FF)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedShort(0xFF00)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::UnsignedInt(0x0000_FFFF)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedInt(0xFFFF_0000)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("bitNot"),
                &[DfdlValue::UnsignedLong(0x0000_0000_FFFF_FFFF)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(0xFFFF_FFFF_0000_0000)
        );

        // leftShift and rightShift with Short, Byte, Long, Unsigned* variants
        assert_eq!(
            eval_fn_call(
                &QName::local("leftShift"),
                &[DfdlValue::Short(1), DfdlValue::Int(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Short(8)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("leftShift"),
                &[DfdlValue::UnsignedShort(1), DfdlValue::Int(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedShort(8)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("leftShift"),
                &[DfdlValue::UnsignedInt(1), DfdlValue::Int(4)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedInt(16)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("leftShift"),
                &[DfdlValue::UnsignedLong(1), DfdlValue::Int(5)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(32)
        );

        assert_eq!(
            eval_fn_call(
                &QName::local("rightShift"),
                &[DfdlValue::Short(16), DfdlValue::Int(2)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Short(4)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("rightShift"),
                &[DfdlValue::UnsignedShort(16), DfdlValue::Int(2)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedShort(4)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("rightShift"),
                &[DfdlValue::UnsignedInt(32), DfdlValue::Int(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedInt(4)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("rightShift"),
                &[DfdlValue::UnsignedLong(64), DfdlValue::Int(4)],
                &ctx
            )
            .unwrap(),
            DfdlValue::UnsignedLong(4)
        );

        // Shift count errors (negative count)
        assert!(eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::Int(1), DfdlValue::Int(-1)],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("rightShift"),
            &[DfdlValue::Int(1), DfdlValue::Int(-1)],
            &ctx
        )
        .is_err());
    }

    /// Verifies replace function arguments, namespaces, and error paths.
    #[test]
    fn test_replace_function_and_errors() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Valid replace with fn prefix
        let qn_fn = QName {
            prefix: Some(String::from("fn")),
            local_name: String::from("replace"),
            namespace: None,
        };
        let res_rep = eval_fn_call(
            &qn_fn,
            &[
                DfdlValue::String(String::from("banana")),
                DfdlValue::String(String::from("a")),
                DfdlValue::String(String::from("o")),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_rep, DfdlValue::String(String::from("bonono")));

        // Valid replace with dfdl prefix
        let qn_dfdl = QName {
            prefix: Some(String::from("dfdl")),
            local_name: String::from("replace"),
            namespace: None,
        };
        assert!(eval_fn_call(
            &qn_dfdl,
            &[
                DfdlValue::String(String::from("abc")),
                DfdlValue::String(String::from("b")),
                DfdlValue::String(String::from("z"))
            ],
            &ctx
        )
        .is_ok());

        // Invalid prefix error
        let qn_invalid = QName {
            prefix: Some(String::from("custom")),
            local_name: String::from("replace"),
            namespace: None,
        };
        let err_pfx =
            eval_fn_call(&qn_invalid, &[DfdlValue::String(String::from("abc"))], &ctx).unwrap_err();
        assert_eq!(err_pfx.kind, DFDLErrorKind::SchemaDefinition);

        // Insufficient argument count (< 3)
        let err_argc = eval_fn_call(
            &qn_fn,
            &[
                DfdlValue::String(String::from("abc")),
                DfdlValue::String(String::from("b")),
            ],
            &ctx,
        )
        .unwrap_err();
        assert_eq!(err_argc.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies xs:hexBinary constructors across binary formats, integers, and hex strings.
    #[test]
    fn test_hexbinary_constructor_variants() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // From HexBinary identity
        let orig_hex = DfdlValue::HexBinary(alloc::vec![0x12, 0x34]);
        assert_eq!(
            eval_fn_call(&QName::local("hexBinary"), &[orig_hex], &ctx).unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34])
        );

        // From integer subtypes
        assert_eq!(
            eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Byte(0x42)], &ctx).unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x42])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::UnsignedByte(0x99)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x99])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::Short(0x1234)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::UnsignedShort(0xABCD)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0xAB, 0xCD])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::Int(0x12345678)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78])
        );

        // From string with even hex digits
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::String(String::from("deadbeef"))],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0xDE, 0xAD, 0xBE, 0xEF])
        );

        // Errors: odd length and invalid hex characters (non-decimal)
        assert!(eval_fn_call(
            &QName::local("hexBinary"),
            &[DfdlValue::String(String::from("abc"))],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("hexBinary"),
            &[DfdlValue::String(String::from("abz"))],
            &ctx
        )
        .is_err());
    }

    /// Verifies range checking functions (checkRangeInclusive and checkRangeExclusive).
    #[test]
    fn test_check_range_inclusive_exclusive() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Inclusive range
        let qn_inc = QName::local("checkRangeInclusive");
        assert_eq!(
            eval_fn_call(
                &qn_inc,
                &[DfdlValue::Int(5), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &qn_inc,
                &[DfdlValue::Int(1), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &qn_inc,
                &[DfdlValue::Int(10), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &qn_inc,
                &[DfdlValue::Int(0), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        // Exclusive range
        let qn_exc = QName::local("checkRangeExclusive");
        assert_eq!(
            eval_fn_call(
                &qn_exc,
                &[DfdlValue::Int(5), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &qn_exc,
                &[DfdlValue::Int(1), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(
                &qn_exc,
                &[DfdlValue::Int(10), DfdlValue::Int(1), DfdlValue::Int(10)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        // Non-numeric operand rejected
        assert!(eval_fn_call(
            &qn_inc,
            &[
                DfdlValue::String(String::from("5")),
                DfdlValue::Int(1),
                DfdlValue::Int(10)
            ],
            &ctx
        )
        .is_err());
    }

    /// Verifies round-half-to-even behavior with various precisions and tie-breaking.
    #[test]
    fn test_round_half_to_even_precision_and_ties() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let qn = QName::local("round-half-to-even");

        // Ties round to even
        assert_eq!(
            eval_fn_call(&qn, &[DfdlValue::Double(2.5)], &ctx).unwrap(),
            DfdlValue::Double(2.0)
        );
        assert_eq!(
            eval_fn_call(&qn, &[DfdlValue::Double(3.5)], &ctx).unwrap(),
            DfdlValue::Double(4.0)
        );

        // Positive precision
        assert_eq!(
            eval_fn_call(&qn, &[DfdlValue::Double(2.555), DfdlValue::Int(2)], &ctx).unwrap(),
            DfdlValue::Double(2.56)
        );

        // Negative precision
        assert_eq!(
            eval_fn_call(&qn, &[DfdlValue::Double(35.0), DfdlValue::Int(-1)], &ctx).unwrap(),
            DfdlValue::Double(40.0)
        );

        // NaN rejected
        assert!(eval_fn_call(&qn, &[DfdlValue::Double(f64::NAN)], &ctx).is_err());
    }

    /// Verifies date and time component extraction functions and formatting error handling.
    #[test]
    fn test_date_time_components_parsing_and_extraction() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let dt_str = DfdlValue::DateTime(String::from("2026-10-07T14:30:45.50"));

        assert_eq!(
            eval_fn_call(
                &QName::local("day-from-dateTime"),
                core::slice::from_ref(&dt_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(7)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hours-from-dateTime"),
                core::slice::from_ref(&dt_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(14)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("minutes-from-dateTime"),
                core::slice::from_ref(&dt_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(30)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("seconds-from-dateTime"),
                core::slice::from_ref(&dt_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("45.50"))
        );

        let tm_str = DfdlValue::Time(String::from("08:15:30"));
        assert_eq!(
            eval_fn_call(
                &QName::local("minutes-from-time"),
                core::slice::from_ref(&tm_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Long(15)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("seconds-from-time"),
                core::slice::from_ref(&tm_str),
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal(String::from("30"))
        );

        // Invalid format strings
        assert!(eval_fn_call(
            &QName::local("year-from-date"),
            &[DfdlValue::Date(String::from("bad-date"))],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("hours-from-time"),
            &[DfdlValue::Time(String::from("bad-time"))],
            &ctx
        )
        .is_err());
    }

    /// Verifies lookAhead function with and without callbacks, boundary limits, and decimal results.
    #[test]
    fn test_lookahead_evaluation_branches() {
        let path = InfosetPath::root();
        let mut budget1 = WorkBudget::new(10000);
        let mut budget2 = WorkBudget::new(10000);
        let mut budget3 = WorkBudget::new(10000);

        // No lookahead callback defaults to 0
        let ctx_no_la = ExprContext::new(None, &path, &[], &mut budget1);
        assert_eq!(
            eval_fn_call(
                &QName::local("lookAhead"),
                &[DfdlValue::Int(0), DfdlValue::Int(8)],
                &ctx_no_la
            )
            .unwrap(),
            DfdlValue::Long(0)
        );

        // Lookahead callback returning small value <= u64::MAX
        let cb_small = |_off: usize, _bits: usize| -> DFDLResult<u128> { Ok(42) };
        let ctx_small =
            ExprContext::new(None, &path, &[], &mut budget2).with_lookahead(Some(&cb_small));
        assert_eq!(
            eval_fn_call(
                &QName::local("lookAhead"),
                &[DfdlValue::Int(0), DfdlValue::Int(8)],
                &ctx_small
            )
            .unwrap(),
            DfdlValue::UnsignedLong(42)
        );

        // Lookahead callback returning large value > u64::MAX
        let cb_large =
            |_off: usize, _bits: usize| -> DFDLResult<u128> { Ok((u64::MAX as u128) + 10) };
        let ctx_large =
            ExprContext::new(None, &path, &[], &mut budget3).with_lookahead(Some(&cb_large));
        match eval_fn_call(
            &QName::local("lookAhead"),
            &[DfdlValue::Int(0), DfdlValue::Int(8)],
            &ctx_large,
        )
        .unwrap()
        {
            DfdlValue::Decimal(s) => assert!(s.contains("18446744073709551625")),
            other => panic!("Expected Decimal, got {:?}", other),
        }

        // Distance exceeding 512 bits
        let err_dist = eval_fn_call(
            &QName::local("lookAhead"),
            &[DfdlValue::Int(500), DfdlValue::Int(20)],
            &ctx_small,
        )
        .unwrap_err();
        assert_eq!(err_dist.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies trace function behavior and rejection when prefixed with fn:trace.
    #[test]
    fn test_trace_and_unsupported_function() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // trace without prefix passes through
        let qn_trace = QName::local("trace");
        assert_eq!(
            eval_fn_call(&qn_trace, &[DfdlValue::Int(123)], &ctx).unwrap(),
            DfdlValue::Int(123)
        );

        // fn:trace is prohibited by specification
        let qn_fn_trace = QName {
            prefix: Some(String::from("fn")),
            local_name: String::from("trace"),
            namespace: None,
        };
        let err_trace = eval_fn_call(&qn_fn_trace, &[DfdlValue::Int(123)], &ctx).unwrap_err();
        assert_eq!(err_trace.kind, DFDLErrorKind::SchemaDefinition);

        // Non-existent function produces SchemaDefinition error
        let qn_unknown = QName::local("nonExistentFunction");
        let err_unknown = eval_fn_call(&qn_unknown, &[], &ctx).unwrap_err();
        assert_eq!(err_unknown.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies path resolution states (Value, Empty, NoValue, Absent, Complex, OVC map, and errors).
    #[test]
    fn test_resolve_path_all_states_and_errors() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};

        let path = InfosetPath::parse("/root/child");
        let mut budget = WorkBudget::new(10000);

        // 1. No document provided
        let ctx_nodoc = ExprContext::new(None, &path, &[], &mut budget);
        let err_nodoc = resolve_path(&path, &ctx_nodoc).unwrap_err();
        assert!(err_nodoc.message.as_str().contains("No Infoset document"));

        // Build document with root, child (Value), empty_child (Empty), noval_child (NoValue), and complex_child
        let simple_val = InfosetElement::simple(
            QName::local("child"),
            ElementState::Value(DfdlValue::Int(42)),
        );
        let empty_val = InfosetElement::simple(QName::local("empty_child"), ElementState::Empty);
        let noval_val = InfosetElement::simple(QName::local("noval_child"), ElementState::NoValue);
        let mut complex_el = InfosetElement::complex(QName::local("complex_child"));
        complex_el
            .children
            .push(InfosetNode::Element(InfosetElement::simple(
                QName::local("inner"),
                ElementState::Value(DfdlValue::String(String::from("inside"))),
            )));

        let mut root = InfosetElement::complex(QName::local("root"));
        root.children.push(InfosetNode::Element(simple_val));
        root.children.push(InfosetNode::Element(empty_val));
        root.children.push(InfosetNode::Element(noval_val));
        root.children.push(InfosetNode::Element(complex_el));

        let doc = InfosetDocument {
            root: Some(root),
            total_nodes: 5,
        };

        let curr_path = InfosetPath::parse("/root");
        let ctx = ExprContext::new(Some(&doc), &curr_path, &[], &mut budget);

        // 2. Resolve value
        assert_eq!(
            resolve_path(&InfosetPath::parse("/root/child"), &ctx).unwrap(),
            DfdlValue::Int(42)
        );

        // 3. Resolve empty element
        assert_eq!(
            resolve_path(&InfosetPath::parse("/root/empty_child"), &ctx).unwrap(),
            DfdlValue::String(String::new())
        );

        // 4. Resolve NoValue simple element
        let err_noval = resolve_path(&InfosetPath::parse("/root/noval_child"), &ctx).unwrap_err();
        assert!(err_noval.message.as_str().contains("does not have a value"));

        // 5. Resolve complex element (does not have a simple value)
        let err_complex =
            resolve_path(&InfosetPath::parse("/root/complex_child"), &ctx).unwrap_err();
        assert!(err_complex.message.as_str().contains("Complex element"));

        // 6. Path not found
        let err_notfound = resolve_path(&InfosetPath::parse("/root/missing"), &ctx).unwrap_err();
        assert_eq!(err_notfound.kind, DFDLErrorKind::SchemaDefinition);

        // 7. Self referencing without value
        let noval_path = InfosetPath::parse("/root/noval_child");
        let self_ctx = ExprContext::new(Some(&doc), &noval_path, &[], &mut budget);
        let err_self = resolve_path(&InfosetPath::parse("."), &self_ctx).unwrap_err();
        assert!(
            err_self.message.as_str().contains("Circular reference")
                || err_self.message.as_str().contains("Self referencing")
        );

        // 8. OVC map override
        let mut ovc_map = alloc::collections::BTreeMap::new();
        ovc_map.insert(String::from("/root/noval_child"), DfdlValue::Int(100));
        let ctx_ovc = ExprContext::new(Some(&doc), &curr_path, &[], &mut budget)
            .with_ovc_values(Some(&ovc_map));
        assert_eq!(
            resolve_path(&InfosetPath::parse("/root/noval_child"), &ctx_ovc).unwrap(),
            DfdlValue::Int(100)
        );
    }

    /// Verifies normalize_infoset_path error handling (navigating past root, self mismatches, invalid index).
    #[test]
    fn test_normalize_infoset_path_steps_and_errors() {
        use crate::infoset::tree::{InfosetDocument, InfosetElement};

        let root = InfosetElement::complex(QName::local("root"));
        let doc = InfosetDocument {
            root: Some(root),
            total_nodes: 1,
        };
        let path = InfosetPath::parse("/root");
        let mut budget = WorkBudget::new(1000);
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // Relative '..' past root error
        let rel_past_root =
            InfosetPath::from_parts(alloc::vec![String::from(".."), String::from("..")], false);
        let err_past_root = normalize_infoset_path(&rel_past_root, &ctx, &doc).unwrap_err();
        assert_eq!(err_past_root.kind, DFDLErrorKind::SchemaDefinition);

        // self::mismatch error: .(expected) when current is different
        let rel_self_mismatch =
            InfosetPath::from_parts(alloc::vec![String::from(".(wrong)")], false);
        let err_self_mismatch = normalize_infoset_path(&rel_self_mismatch, &ctx, &doc).unwrap_err();
        assert_eq!(err_self_mismatch.kind, DFDLErrorKind::SchemaDefinition);

        // Indexing on dot: .[1] error
        let rel_idx_dot = InfosetPath::from_parts(alloc::vec![String::from(".[1]")], false);
        let err_idx_dot = normalize_infoset_path(&rel_idx_dot, &ctx, &doc).unwrap_err();
        assert_eq!(err_idx_dot.kind, DFDLErrorKind::SchemaDefinition);

        // Indexing on dotdot: ..[1] error
        let rel_idx_dotdot = InfosetPath::from_parts(alloc::vec![String::from("..[1]")], false);
        let err_idx_dotdot = normalize_infoset_path(&rel_idx_dotdot, &ctx, &doc).unwrap_err();
        assert_eq!(err_idx_dotdot.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies built-in XPath path-based functions (count, exists, empty, trace, valueLength).
    #[test]
    fn test_eval_expr_fncall_path_forms() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};

        let child1 =
            InfosetElement::simple(QName::local("item"), ElementState::Value(DfdlValue::Int(1)));
        let child2 =
            InfosetElement::simple(QName::local("item"), ElementState::Value(DfdlValue::Int(2)));
        let mut parent = InfosetElement::complex(QName::local("root"));
        parent.children.push(InfosetNode::Element(child1));
        parent.children.push(InfosetNode::Element(child2));
        let mut subgroup = InfosetElement::complex(QName::local("subgroup"));
        subgroup
            .children
            .push(InfosetNode::Element(InfosetElement::simple(
                QName::local("sub_item"),
                ElementState::Value(DfdlValue::Int(9)),
            )));
        parent.children.push(InfosetNode::Element(subgroup));

        let doc = InfosetDocument {
            root: Some(parent),
            total_nodes: 4,
        };

        let path = InfosetPath::parse("/root");
        let mut budget = WorkBudget::new(10000);
        let mut ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // fn:count with wildcard path
        let ast_count_star = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(
                alloc::vec![String::from("root"), String::from("*")],
                true
            ))],
        };
        assert_eq!(
            eval_expr(&ast_count_star, &mut ctx).unwrap(),
            DfdlValue::Long(3)
        );

        // fn:count with specific child name
        let ast_count_item = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(
                alloc::vec![String::from("root"), String::from("item")],
                true
            ))],
        };
        assert_eq!(
            eval_expr(&ast_count_item, &mut ctx).unwrap(),
            DfdlValue::Long(2)
        );

        // fn:exists with existing path
        let ast_exists_true = ExprAst::FnCall {
            name: QName::local("exists"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(
            eval_expr(&ast_exists_true, &mut ctx).unwrap(),
            DfdlValue::Boolean(true)
        );

        // fn:exists with non-existing path
        let ast_exists_false = ExprAst::FnCall {
            name: QName::local("exists"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/nonexistent"))],
        };
        assert_eq!(
            eval_expr(&ast_exists_false, &mut ctx).unwrap(),
            DfdlValue::Boolean(false)
        );

        // fn:empty with existing path
        let ast_empty_false = ExprAst::FnCall {
            name: QName::local("empty"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(
            eval_expr(&ast_empty_false, &mut ctx).unwrap(),
            DfdlValue::Boolean(false)
        );

        // fn:empty with non-existing path
        let ast_empty_true = ExprAst::FnCall {
            name: QName::local("empty"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/nonexistent"))],
        };
        assert_eq!(
            eval_expr(&ast_empty_true, &mut ctx).unwrap(),
            DfdlValue::Boolean(true)
        );

        // trace on simple value
        let ast_trace_simple = ExprAst::FnCall {
            name: QName::local("trace"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(
            eval_expr(&ast_trace_simple, &mut ctx).unwrap(),
            DfdlValue::Int(1)
        );

        // trace on complex element returns element name string
        let ast_trace_complex = ExprAst::FnCall {
            name: QName::local("trace"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/subgroup"))],
        };
        assert_eq!(
            eval_expr(&ast_trace_complex, &mut ctx).unwrap(),
            DfdlValue::String(String::from("subgroup"))
        );

        // valueLength with enclosing_lengths
        let enc_lengths = [(
            String::from("root"),
            16usize,
            crate::schema::ir::LengthUnits::Bits,
            crate::encoding::EncodingKind::Ascii,
        )];
        let mut ctx_enc = ExprContext::new(Some(&doc), &path, &[], &mut budget)
            .with_enclosing_lengths(&enc_lengths);
        let ast_vl_bits = ExprAst::FnCall {
            name: QName::local("valueLength"),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::parse("/root")),
                ExprAst::Literal(DfdlValue::String(String::from("bits"))),
            ],
        };
        assert_eq!(
            eval_expr(&ast_vl_bits, &mut ctx_enc).unwrap(),
            DfdlValue::Long(16)
        );
        let ast_vl_bytes = ExprAst::FnCall {
            name: QName::local("valueLength"),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::parse("/root")),
                ExprAst::Literal(DfdlValue::String(String::from("bytes"))),
            ],
        };
        assert_eq!(
            eval_expr(&ast_vl_bytes, &mut ctx_enc).unwrap(),
            DfdlValue::Long(2)
        );
    }

    #[test]
    fn test_eval_builtin_functions_comprehensive() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // String functions
        let res_cat = eval_fn_call(
            &QName::local("concat"),
            &[
                DfdlValue::String("a".into()),
                DfdlValue::String("b".into()),
                DfdlValue::Int(1),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_cat, DfdlValue::String("ab1".into()));

        let res_len = eval_fn_call(
            &QName::local("string-length"),
            &[DfdlValue::String("hello".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_len, DfdlValue::Long(5));

        let res_sub = eval_fn_call(
            &QName::local("substring"),
            &[
                DfdlValue::String("hello world".into()),
                DfdlValue::Int(1),
                DfdlValue::Int(5),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sub, DfdlValue::String("hello".into()));

        let res_before = eval_fn_call(
            &QName::local("substring-before"),
            &[
                DfdlValue::String("abc-def".into()),
                DfdlValue::String("-".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_before, DfdlValue::String("abc".into()));

        let res_after = eval_fn_call(
            &QName::local("substring-after"),
            &[
                DfdlValue::String("abc-def".into()),
                DfdlValue::String("-".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_after, DfdlValue::String("def".into()));

        let res_has = eval_fn_call(
            &QName::local("contains"),
            &[
                DfdlValue::String("testing".into()),
                DfdlValue::String("est".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_has, DfdlValue::Boolean(true));

        let res_sw = eval_fn_call(
            &QName::local("starts-with"),
            &[
                DfdlValue::String("testing".into()),
                DfdlValue::String("test".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sw, DfdlValue::Boolean(true));

        let res_ew = eval_fn_call(
            &QName::local("ends-with"),
            &[
                DfdlValue::String("testing".into()),
                DfdlValue::String("ing".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ew, DfdlValue::Boolean(true));

        let res_up = eval_fn_call(
            &QName::local("upper-case"),
            &[DfdlValue::String("abc".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_up, DfdlValue::String("ABC".into()));

        let res_low = eval_fn_call(
            &QName::local("lower-case"),
            &[DfdlValue::String("ABC".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_low, DfdlValue::String("abc".into()));

        let res_rep = eval_fn_call(
            &QName::local("replace"),
            &[
                DfdlValue::String("a-b-c".into()),
                DfdlValue::String("-".into()),
                DfdlValue::String("_".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_rep, DfdlValue::String("a_b_c".into()));

        // Math functions
        let res_abs = eval_fn_call(&QName::local("abs"), &[DfdlValue::Int(-42)], &ctx).unwrap();
        assert_eq!(res_abs, DfdlValue::Int(42));

        let res_ceil =
            eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap();
        assert_eq!(res_ceil, DfdlValue::Double(4.0));

        let res_fl = eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap();
        assert_eq!(res_fl, DfdlValue::Double(3.0));

        let res_rnd =
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.5)], &ctx).unwrap();
        assert_eq!(res_rnd, DfdlValue::Double(4.0));

        // Logic & Compare
        let res_cmp = eval_fn_call(
            &QName::local("compare"),
            &[DfdlValue::String("a".into()), DfdlValue::String("b".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_cmp, DfdlValue::Int(-1));

        let res_not =
            eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap();
        assert_eq!(res_not, DfdlValue::Boolean(true));

        let res_bool = eval_fn_call(
            &QName::local("boolean"),
            &[DfdlValue::String("true".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_bool, DfdlValue::Boolean(true));

        // Calendar extraction
        let res_dt = eval_fn_call(
            &QName::local("year-from-date"),
            &[DfdlValue::Date("2026-10-08".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_dt, DfdlValue::Long(2026));

        let res_m = eval_fn_call(
            &QName::local("month-from-date"),
            &[DfdlValue::Date("2026-10-08".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_m, DfdlValue::Long(10));

        let res_d = eval_fn_call(
            &QName::local("day-from-date"),
            &[DfdlValue::Date("2026-10-08".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_d, DfdlValue::Long(8));

        let res_h = eval_fn_call(
            &QName::local("hours-from-time"),
            &[DfdlValue::Time("14:30:15".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_h, DfdlValue::Long(14));

        let res_min = eval_fn_call(
            &QName::local("minutes-from-time"),
            &[DfdlValue::Time("14:30:15".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_min, DfdlValue::Long(30));

        let res_sec = eval_fn_call(
            &QName::local("seconds-from-time"),
            &[DfdlValue::Time("14:30:15".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sec, DfdlValue::Decimal("15".into()));

        // Casts
        let res_i = eval_fn_call(
            &QName::local("int"),
            &[DfdlValue::String("123".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_i, DfdlValue::Int(123));

        let res_l = eval_fn_call(&QName::local("long"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_l, DfdlValue::Long(123));

        let res_sh = eval_fn_call(&QName::local("short"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_sh, DfdlValue::Short(123));

        let res_by = eval_fn_call(&QName::local("byte"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_by, DfdlValue::Byte(123));

        let res_ub =
            eval_fn_call(&QName::local("unsignedByte"), &[DfdlValue::Int(250)], &ctx).unwrap();
        assert_eq!(res_ub, DfdlValue::UnsignedByte(250));

        let res_us = eval_fn_call(
            &QName::local("unsignedShort"),
            &[DfdlValue::Int(60000)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_us, DfdlValue::UnsignedShort(60000));

        let res_ui = eval_fn_call(
            &QName::local("unsignedInt"),
            &[DfdlValue::Long(100000)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ui, DfdlValue::UnsignedInt(100000));

        let res_ul = eval_fn_call(
            &QName::local("unsignedLong"),
            &[DfdlValue::Long(100000)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ul, DfdlValue::UnsignedLong(100000));

        let res_flt = eval_fn_call(
            &QName::local("float"),
            &[DfdlValue::String("1.5".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_flt, DfdlValue::Float(1.5));

        let res_str = eval_fn_call(&QName::local("string"), &[DfdlValue::Int(99)], &ctx).unwrap();
        assert_eq!(res_str, DfdlValue::String("99".into()));

        // Bitwise operations
        let res_band = eval_fn_call(
            &QName::local("bitAnd"),
            &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_band, DfdlValue::Long(0b1000));

        let res_bor = eval_fn_call(
            &QName::local("bitOr"),
            &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_bor, DfdlValue::Long(0b1110));

        let res_bxor = eval_fn_call(
            &QName::local("bitXor"),
            &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_bxor, DfdlValue::Long(0b0110));

        let res_bnot = eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Byte(0)], &ctx).unwrap();
        assert_eq!(res_bnot, DfdlValue::Byte(-1));

        let res_shl = eval_fn_call(
            &QName::local("leftShift"),
            &[DfdlValue::Int(1), DfdlValue::Int(3)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_shl, DfdlValue::Int(8));

        let res_shr = eval_fn_call(
            &QName::local("rightShift"),
            &[DfdlValue::Int(8), DfdlValue::Int(3)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_shr, DfdlValue::Int(1));

        // Entities & Helpers
        let res_ent = eval_fn_call(
            &QName::local("decodeDFDLEntities"),
            &[DfdlValue::String("%SP;".into())],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_ent, DfdlValue::String(" ".into()));

        let res_hello = eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap();
        assert_eq!(res_hello, DfdlValue::String("Hello".into()));

        let res_add = eval_fn_call(
            &QName::local("addPrimitive"),
            &[DfdlValue::Int(10), DfdlValue::Int(20)],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_add, DfdlValue::Int(30));
    }

    /// Verifies mathematical rounding functions, type conversions, and logical error branches.
    #[test]
    fn test_eval_extended_functions_and_branches() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // 1. ceiling function across Float and Double with NaN, INF, and finite values
        assert_eq!(
            eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Float(3.2)], &ctx).unwrap(),
            DfdlValue::Float(4.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap(),
            DfdlValue::Double(4.0)
        );
        assert!(eval_fn_call(
            &QName::local("ceiling"),
            &[DfdlValue::Float(f32::NAN)],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("ceiling"),
            &[DfdlValue::Double(f64::NAN)],
            &ctx
        )
        .is_err());
        assert_eq!(
            eval_fn_call(
                &QName::local("ceiling"),
                &[DfdlValue::Float(f32::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(f32::INFINITY)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("ceiling"),
                &[DfdlValue::Double(f64::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(f64::INFINITY)
        );

        // 2. floor function across Float and Double with NaN, INF, and finite values
        assert_eq!(
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(3.8)], &ctx).unwrap(),
            DfdlValue::Float(3.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap(),
            DfdlValue::Double(3.0)
        );
        assert!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(f32::NAN)], &ctx).is_err());
        assert!(
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(f64::NAN)], &ctx).is_err()
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("floor"),
                &[DfdlValue::Float(f32::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(f32::INFINITY)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("floor"),
                &[DfdlValue::Double(f64::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(f64::INFINITY)
        );

        // 3. round function across Float and Double with NaN, INF, positive, and negative values
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Float(3.6)], &ctx).unwrap(),
            DfdlValue::Float(4.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.6)], &ctx).unwrap(),
            DfdlValue::Double(4.0)
        );
        // W3C XPath 2.0 §6.4.4: -3.6 rounds to -4.0 (closest integer, distance 0.4 vs 0.6)
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Float(-3.6)], &ctx).unwrap(),
            DfdlValue::Float(-4.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(-3.6)], &ctx).unwrap(),
            DfdlValue::Double(-4.0)
        );
        // XPath 2.0 tie-breaking: round half towards positive infinity
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(-1.5)], &ctx).unwrap(),
            DfdlValue::Double(-1.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(-2.5)], &ctx).unwrap(),
            DfdlValue::Double(-2.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(1.5)], &ctx).unwrap(),
            DfdlValue::Double(2.0)
        );
        // Preserves -0.0 for -0.5 <= x < 0.0
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(-0.5)], &ctx).unwrap(),
            DfdlValue::Double(-0.0)
        );
        // Large values >= 2^52 without i64 saturation
        assert_eq!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(1e20)], &ctx).unwrap(),
            DfdlValue::Double(1e20)
        );
        assert_eq!(
            eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(1e20)], &ctx).unwrap(),
            DfdlValue::Double(1e20)
        );
        assert_eq!(
            eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(1e20)], &ctx).unwrap(),
            DfdlValue::Double(1e20)
        );
        // xs:decimal rounding
        assert_eq!(
            eval_fn_call(
                &QName::local("round"),
                &[DfdlValue::Decimal("0.125".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal("0".into())
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("round"),
                &[DfdlValue::Decimal("0.5".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Decimal("1".into())
        );
        assert!(eval_fn_call(&QName::local("round"), &[DfdlValue::Float(f32::NAN)], &ctx).is_err());
        assert!(
            eval_fn_call(&QName::local("round"), &[DfdlValue::Double(f64::NAN)], &ctx).is_err()
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("round"),
                &[DfdlValue::Float(f32::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(f32::INFINITY)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("round"),
                &[DfdlValue::Double(f64::INFINITY)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(f64::INFINITY)
        );

        // 4. xs:double and xs:float type casting error branches (lowercase nan, inf, -inf)
        assert!(eval_fn_call(
            &QName::local("double"),
            &[DfdlValue::String("nan".into())],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("double"),
            &[DfdlValue::String("inf".into())],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("double"),
            &[DfdlValue::String("-inf".into())],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("float"),
            &[DfdlValue::String("nan".into())],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("float"),
            &[DfdlValue::String("inf".into())],
            &ctx
        )
        .is_err());
        assert!(eval_fn_call(
            &QName::local("float"),
            &[DfdlValue::String("-inf".into())],
            &ctx
        )
        .is_err());

        // Valid integer and boolean casts to double
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Short(10)], &ctx).unwrap(),
            DfdlValue::Double(10.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Byte(5)], &ctx).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedLong(100)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(100.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedShort(50)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(50.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedByte(25)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(25.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Double(1.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(false)], &ctx).unwrap(),
            DfdlValue::Double(0.0)
        );

        // Valid integer and boolean casts to float
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Double(12.5)], &ctx).unwrap(),
            DfdlValue::Float(12.5)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Int(10)], &ctx).unwrap(),
            DfdlValue::Float(10.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Long(20)], &ctx).unwrap(),
            DfdlValue::Float(20.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Short(15)], &ctx).unwrap(),
            DfdlValue::Float(15.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Byte(7)], &ctx).unwrap(),
            DfdlValue::Float(7.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedLong(30)], &ctx).unwrap(),
            DfdlValue::Float(30.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedInt(40)], &ctx).unwrap(),
            DfdlValue::Float(40.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("float"),
                &[DfdlValue::UnsignedShort(50)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(50.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedByte(60)], &ctx).unwrap(),
            DfdlValue::Float(60.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Float(1.0)
        );

        // 5. xs:boolean across diverse data types
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(0)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Long(100)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Short(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Byte(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedLong(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedShort(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedByte(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Float(1.0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Double(1.0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert!(eval_fn_call(
            &QName::local("boolean"),
            &[DfdlValue::String("not_a_bool".into())],
            &ctx
        )
        .is_err());

        // 6. fn:not function across diverse data types
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Int(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Long(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Short(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Byte(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedLong(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedInt(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedShort(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedByte(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Float(0.0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Double(0.0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("not"),
                &[DfdlValue::String("true".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        // 7. fn:replace function
        let rep_res = eval_fn_call(
            &QName::local("replace"),
            &[
                DfdlValue::String("banana".into()),
                DfdlValue::String("a".into()),
                DfdlValue::String("o".into()),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(rep_res, DfdlValue::String("bonono".into()));

        // 8. addBoxed helper
        let add_b = eval_fn_call(
            &QName::local("addBoxed"),
            &[DfdlValue::Int(10), DfdlValue::Int(25)],
            &ctx,
        )
        .unwrap();
        assert_eq!(add_b, DfdlValue::Int(35));

        // 9. Logical operators error handling on non-boolean operands
        let mut budget2 = crate::limits::WorkBudget::new(1000);
        let mut ctx2 = ExprContext::new(None, &path, &[], &mut budget2);
        let and_bad1 = ExprAst::Binary {
            op: BinaryOp::And,
            left: Box::new(ExprAst::Literal(DfdlValue::Boolean(true))),
            right: Box::new(ExprAst::Literal(DfdlValue::Int(42))),
        };
        assert!(eval_expr(&and_bad1, &mut ctx2).is_err());

        let and_bad2 = ExprAst::Binary {
            op: BinaryOp::And,
            left: Box::new(ExprAst::Literal(DfdlValue::Int(42))),
            right: Box::new(ExprAst::Literal(DfdlValue::Boolean(true))),
        };
        assert!(eval_expr(&and_bad2, &mut ctx2).is_err());

        let or_bad1 = ExprAst::Binary {
            op: BinaryOp::Or,
            left: Box::new(ExprAst::Literal(DfdlValue::Boolean(false))),
            right: Box::new(ExprAst::Literal(DfdlValue::Int(42))),
        };
        assert!(eval_expr(&or_bad1, &mut ctx2).is_err());

        let or_bad2 = ExprAst::Binary {
            op: BinaryOp::Or,
            left: Box::new(ExprAst::Literal(DfdlValue::Int(42))),
            right: Box::new(ExprAst::Literal(DfdlValue::Boolean(false))),
        };
        assert!(eval_expr(&or_bad2, &mut ctx2).is_err());

        // 10. ExprContext::find_element helper method (lines 182-193)
        let mut doc_find = InfosetDocument::new();
        let mut root_find = InfosetElement::complex(QName::local("root"));
        let child_find = InfosetElement::simple(
            QName::local("child"),
            ElementState::Value(DfdlValue::Int(99)),
        );
        root_find
            .try_add_child(InfosetNode::Element(child_find))
            .unwrap();
        doc_find.root = Some(root_find);

        let p_child = InfosetPath::from_parts(alloc::vec!["root".into(), "child".into()], true);
        let mut budget3 = crate::limits::WorkBudget::new(1000);
        let ctx_find = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget3);
        let found_elem = ctx_find.find_element(&p_child);
        assert!(found_elem.is_some());
        if let Some(el) = found_elem {
            assert_eq!(el.state, ElementState::Value(DfdlValue::Int(99)));
        }

        // 11. fn:trace error (lines 328-331)
        let ast_trace = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "trace",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Int(1))],
        };
        let err_trace = eval_expr(&ast_trace, &mut ctx2).unwrap_err();
        assert!(err_trace
            .message
            .as_str()
            .contains("Unsupported function: fn:trace"));

        // 12. dfdl:valueLength with bits units (lines 549-555, 573-576)
        let ast_vlen = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![
                ExprAst::Path(p_child.clone()),
                ExprAst::Literal(DfdlValue::String("bits".into())),
            ],
        };
        let mut budget4 = crate::limits::WorkBudget::new(1000);
        let mut ctx_vlen = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget4);
        let res_vlen = eval_expr(&ast_vlen, &mut ctx_vlen).unwrap();
        assert_eq!(res_vlen, DfdlValue::Long(32)); // 4 bytes * 8 bits

        // 13. dfdl:valueLength enclosing element error (lines 486-499)
        let mut budget5 = crate::limits::WorkBudget::new(1000);
        let mut ctx_enclosing = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget5);
        let p_parent = InfosetPath::from_parts(alloc::vec!["..".into()], false);
        let ast_vlen_enc = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![
                ExprAst::Path(p_parent),
                ExprAst::Literal(DfdlValue::String("bytes".into())),
            ],
        };
        let err_enc = eval_expr(&ast_vlen_enc, &mut ctx_enclosing).unwrap_err();
        assert!(err_enc
            .message
            .as_str()
            .contains("Value Length cannot be computed for enclosing element"));

        // 14. dfdl:valueLength with characters units and enclosing_lengths (lines 444-451)
        let mut budget6 = crate::limits::WorkBudget::new(1000);
        let mut ctx_chars = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget6);
        let enc_lengths = [(
            alloc::string::String::from("child"),
            16, // 16 bits
            crate::schema::ir::LengthUnits::Bits,
            crate::encoding::EncodingKind::Utf8,
        )];
        ctx_chars.enclosing_lengths = &enc_lengths;
        let ast_vlen_chars = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![
                ExprAst::Path(p_child.clone()),
                ExprAst::Literal(DfdlValue::String("characters".into())),
            ],
        };
        let res_chars = eval_expr(&ast_vlen_chars, &mut ctx_chars).unwrap();
        assert_eq!(res_chars, DfdlValue::Long(2)); // 16 bits / 8 bits per char = 2 chars

        // 15. dfdl:checkConstraints on absolute path, relative with .., and nil element (lines 583-608)
        let mut doc_nil = InfosetDocument::new();
        let nil_elem = InfosetElement::simple(QName::local("nil_elem"), ElementState::Nil);
        doc_nil.root = Some(nil_elem);
        let p_nil = InfosetPath::from_parts(alloc::vec!["nil_elem".into()], true); // absolute path /nil_elem
        let ast_cc_abs = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "checkConstraints",
                Some("dfdl"),
            ),
            args: alloc::vec![ExprAst::Path(p_nil)],
        };
        let mut budget7 = crate::limits::WorkBudget::new(1000);
        let p_curr = InfosetPath::root();
        let mut ctx_nil = ExprContext::new(Some(&doc_nil), &p_curr, &[], &mut budget7);
        let res_cc_nil = eval_expr(&ast_cc_abs, &mut ctx_nil).unwrap();
        assert_eq!(res_cc_nil, DfdlValue::Boolean(true));

        // 16. eval_prop_str_with_ctx with double curly braces (lines 662-663)
        let res_curlies = eval_prop_str_with_ctx("{{hello", Some(&ctx_nil), &p_curr);
        assert_eq!(res_curlies, Some("{hello".into()));

        // 17. fn:empty returning !exists (lines 343-351)
        let ast_empty = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "empty",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Path(p_child.clone())],
        };
        let mut budget8 = crate::limits::WorkBudget::new(1000);
        let mut ctx_empty = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget8);
        assert_eq!(
            eval_expr(&ast_empty, &mut ctx_empty).unwrap(),
            DfdlValue::Boolean(false)
        );

        // 18. fn:count with parent step .. (lines 367-378)
        let p_count_parent =
            InfosetPath::from_parts(alloc::vec!["..".into(), "child".into()], false);
        let ast_count = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "count",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Path(p_count_parent)],
        };
        let mut budget9 = crate::limits::WorkBudget::new(1000);
        let mut ctx_count = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget9);
        assert_eq!(
            eval_expr(&ast_count, &mut ctx_count).unwrap(),
            DfdlValue::Long(1)
        );

        // 19. dfdl:valueLength without units argument defaulting to bytes (lines 405-415)
        let ast_vlen_nounits = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![ExprAst::Path(p_child.clone())],
        };
        let mut budget10 = crate::limits::WorkBudget::new(1000);
        let mut ctx_vlen_nounits = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget10);
        assert_eq!(
            eval_expr(&ast_vlen_nounits, &mut ctx_vlen_nounits).unwrap(),
            DfdlValue::Long(4)
        );

        // 20. fn:string on non-existent element returns Err (line 339)
        let p_missing = InfosetPath::from_parts(alloc::vec!["nonexistent".into()], false);
        let ast_str_missing = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "string",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Path(p_missing)],
        };
        let mut budget11 = crate::limits::WorkBudget::new(1000);
        let mut ctx_str_missing = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget11);
        assert!(eval_expr(&ast_str_missing, &mut ctx_str_missing).is_err());

        // 21. fn:ceiling and fn:floor with NaN and Infinity (lines 2430-2485)
        let mut budget12 = crate::limits::WorkBudget::new(1000);
        let mut ctx_math = ExprContext::new(None, &p_curr, &[], &mut budget12);

        let ast_ceil_nan = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "ceiling",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Double(f64::NAN))],
        };
        assert!(eval_expr(&ast_ceil_nan, &mut ctx_math).is_err());

        let ast_ceil_inf = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "ceiling",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Double(f64::INFINITY))],
        };
        assert_eq!(
            eval_expr(&ast_ceil_inf, &mut ctx_math).unwrap(),
            DfdlValue::Double(f64::INFINITY)
        );

        let ast_floor_nan = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "floor",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Float(f32::NAN))],
        };
        assert!(eval_expr(&ast_floor_nan, &mut ctx_math).is_err());

        let ast_floor_inf = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.w3.org/2005/xpath-functions",
                "floor",
                Some("fn"),
            ),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Float(f32::NEG_INFINITY))],
        };
        assert_eq!(
            eval_expr(&ast_floor_inf, &mut ctx_math).unwrap(),
            DfdlValue::Float(f32::NEG_INFINITY)
        );

        // 22. Standard variables (lines 685-691)
        for (var_name, expected_val) in [
            ("encoding", "UTF-8"),
            ("byteOrder", "bigEndian"),
            ("binaryFloatRep", "ieee"),
            ("outputNewLine", "\n"),
        ] {
            let ast_var = ExprAst::Variable(QName::local(var_name));
            let val = eval_expr(&ast_var, &mut ctx_math).unwrap();
            assert_eq!(val, DfdlValue::String(expected_val.into()));
        }

        // 23. dfdl:valueLength circular reference error (lines 507-520)
        let mut b_circ = crate::schema::builder::SchemaBuilder::new();
        let circ_props = crate::schema::ResolvedProperties {
            truncate_specified_length_string: true,
            length_expr: Some("{ ../elemA }".into()),
            ..Default::default()
        };
        let circ_elem = crate::schema::ir::TermKind::Element(crate::schema::ir::CompiledElement {
            name: QName::local("elemB"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        });
        let circ_id = b_circ
            .add_term_with_props(QName::local("elemB"), circ_elem, circ_props)
            .unwrap();
        b_circ.set_root(circ_id);
        let schema_circ = b_circ.build().unwrap();

        let ast_vl_circ = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(
                alloc::vec!["elemB".into()],
                false
            ))],
        };
        let p_elema = InfosetPath::from_parts(alloc::vec!["elemA".into()], false);
        let mut budget13 = crate::limits::WorkBudget::new(1000);
        let mut ctx_circ =
            ExprContext::new(None, &p_elema, &[], &mut budget13).with_schema(&schema_circ);
        let err_circ = eval_expr(&ast_vl_circ, &mut ctx_circ).unwrap_err();
        assert!(err_circ
            .message
            .as_str()
            .contains("Circular reference / circular dependency"));

        // 24. dfdl:valueLength self length calculation error (lines 470-477)
        let ast_vl_self = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("."))],
        };
        let p_elemb = InfosetPath::from_parts(alloc::vec!["elemB".into()], false);
        let mut budget14 = crate::limits::WorkBudget::new(1000);
        let mut ctx_self =
            ExprContext::new(None, &p_elemb, &[], &mut budget14).with_schema(&schema_circ);
        let err_self = eval_expr(&ast_vl_self, &mut ctx_self).unwrap_err();
        assert!(err_self
            .message
            .as_str()
            .contains("Value length unknown: cannot evaluate dfdl:valueLength"));

        // 25. fn:count with parent path . and .. (lines 368-378)
        let ast_count_parent = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(
                alloc::vec!["..".into(), ".".into(), "item".into()],
                false
            ))],
        };
        let doc_count = crate::infoset::tree::InfosetDocument::new();
        let mut budget15 = crate::limits::WorkBudget::new(1000);
        let p_sub = InfosetPath::from_parts(alloc::vec!["root".into(), "sub".into()], false);
        let mut ctx_count = ExprContext::new(Some(&doc_count), &p_sub, &[], &mut budget15);
        assert_eq!(
            eval_expr(&ast_count_parent, &mut ctx_count).unwrap(),
            DfdlValue::Long(0)
        );

        // 26. dfdl:valueLength with characters units from enclosing_lengths (lines 440-449)
        let ast_vl_chars = ExprAst::FnCall {
            name: QName::with_namespace(
                "http://www.ogf.org/dfdl/dfdl-1.0/",
                "valueLength",
                Some("dfdl"),
            ),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::from_parts(alloc::vec!["target".into()], false)),
                ExprAst::Literal(DfdlValue::String("characters".into())),
            ],
        };
        let mut budget16 = crate::limits::WorkBudget::new(1000);
        let p_root = InfosetPath::root();
        let enc_lengths = [(
            "target".into(),
            10usize,
            crate::schema::ir::LengthUnits::Characters,
            "UTF-8".into(),
        )];
        let mut ctx_chars = ExprContext::new(None, &p_root, &[], &mut budget16)
            .with_enclosing_lengths(&enc_lengths);
        assert_eq!(
            eval_expr(&ast_vl_chars, &mut ctx_chars).unwrap(),
            DfdlValue::Long(10)
        );
    }

    #[test]
    fn test_expr_eval_broad_edge_cases() {
        // 1. Predicate indexing with typed integers (Byte, Short, Long, Unsigned*)
        // Validates occurrences indexing using numeric subtypes.
        let mut budget = WorkBudget::new(10_000);
        let mut doc = InfosetDocument::new();
        let mut root_elem = InfosetElement::complex(crate::types::QName::local("root"));
        let item1 = InfosetElement::simple(
            crate::types::QName::local("item"),
            ElementState::Value(DfdlValue::String("first".into())),
        );
        let item2 = InfosetElement::simple(
            crate::types::QName::local("item"),
            ElementState::Value(DfdlValue::String("second".into())),
        );
        root_elem.children.push(InfosetNode::Element(item1));
        root_elem.children.push(InfosetNode::Element(item2));
        doc.root = Some(root_elem);

        let p_root = InfosetPath::root();
        let mut ctx = ExprContext::new(Some(&doc), &p_root, &[], &mut budget);

        // Test path expressions with predicate evaluating to expressions
        for pred_expr in [
            "item[xs:byte(2)]",
            "item[xs:short(2)]",
            "item[xs:long(2)]",
            "item[xs:unsignedByte(2)]",
            "item[xs:unsignedShort(2)]",
            "item[xs:unsignedInt(2)]",
            "item[xs:unsignedLong(2)]",
        ] {
            let ast = crate::expr::parse_expr(pred_expr).unwrap();
            assert_eq!(
                eval_expr(&ast, &mut ctx).unwrap(),
                DfdlValue::String("second".into())
            );
        }

        // 2. Binary AND/OR type errors with non-boolean operands
        // DFDL expression spec strictly requires boolean types for logical conjunction.
        let ast_bad_and = crate::expr::parse_expr("1 and fn:true()").unwrap();
        assert!(eval_expr(&ast_bad_and, &mut ctx).is_err());
        let ast_bad_or = crate::expr::parse_expr("fn:false() or 'abc'").unwrap();
        assert!(eval_expr(&ast_bad_or, &mut ctx).is_err());

        // 3. String length and value length on scalar variants
        // Verifies byte and bit length conversions on HexBinary, Short, Long, Byte.
        let ast_str_len =
            crate::expr::parse_expr("dfdl:valueLength(xs:hexBinary('AABBCC'), 'bits')").unwrap();
        assert_eq!(
            eval_expr(&ast_str_len, &mut ctx).unwrap(),
            DfdlValue::Long(24)
        );

        let ast_str_short = crate::expr::parse_expr("dfdl:valueLength(xs:short(1234))").unwrap();
        assert_eq!(
            eval_expr(&ast_str_short, &mut ctx).unwrap(),
            DfdlValue::Long(4)
        );

        let ast_str_byte = crate::expr::parse_expr("dfdl:valueLength(xs:byte(42))").unwrap();
        assert_eq!(
            eval_expr(&ast_str_byte, &mut ctx).unwrap(),
            DfdlValue::Long(1)
        );

        let ast_str_long = crate::expr::parse_expr("dfdl:valueLength(xs:long(9999))").unwrap();
        assert_eq!(
            eval_expr(&ast_str_long, &mut ctx).unwrap(),
            DfdlValue::Long(8)
        );

        // 4. Substring with float special values (NaN, -INF, +INF)
        // XPath 2.0 substring semantics on infinite and NaN indices.
        let ast_sub_nan =
            crate::expr::parse_expr("fn:substring('hello', xs:float('NaN'))").unwrap();
        assert_eq!(
            eval_expr(&ast_sub_nan, &mut ctx).unwrap(),
            DfdlValue::String(String::new())
        );
        let ast_sub_posinf =
            crate::expr::parse_expr("fn:substring('hello', xs:float('INF'), 2.0)").unwrap();
        assert_eq!(
            eval_expr(&ast_sub_posinf, &mut ctx).unwrap(),
            DfdlValue::String(String::new())
        );
        let ast_sub_neginf =
            crate::expr::parse_expr("fn:substring('hello', xs:float('-INF'), 2.0)").unwrap();
        assert_eq!(
            eval_expr(&ast_sub_neginf, &mut ctx).unwrap(),
            DfdlValue::String("he".into())
        );

        // 5. checkRangeInclusive with various numeric types
        // Evaluates boundary validation across distinct integer representations.
        for val_expr in [
            "xs:short(5)",
            "xs:byte(5)",
            "xs:unsignedLong(5)",
            "xs:unsignedInt(5)",
            "xs:unsignedShort(5)",
            "xs:unsignedByte(5)",
        ] {
            let expr_str = format!(
                "dfdl:checkRangeInclusive({}, xs:short(1), xs:short(10))",
                val_expr
            );
            let ast_range = crate::expr::parse_expr(&expr_str).unwrap();
            assert_eq!(
                eval_expr(&ast_range, &mut ctx).unwrap(),
                DfdlValue::Boolean(true)
            );
        }

        // 6. Type constructor macro edge cases (xs:short, xs:byte, xs:unsigned*)
        // Verifies cross-conversion between signed and unsigned primitives.
        let ast_conv1 = crate::expr::parse_expr("xs:int(xs:short(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv1, &mut ctx).unwrap(), DfdlValue::Int(10));
        let ast_conv2 = crate::expr::parse_expr("xs:int(xs:byte(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv2, &mut ctx).unwrap(), DfdlValue::Int(10));
        let ast_conv3 = crate::expr::parse_expr("xs:int(xs:unsignedLong(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv3, &mut ctx).unwrap(), DfdlValue::Int(10));
        let ast_conv4 = crate::expr::parse_expr("xs:int(xs:unsignedInt(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv4, &mut ctx).unwrap(), DfdlValue::Int(10));
        let ast_conv5 = crate::expr::parse_expr("xs:int(xs:unsignedShort(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv5, &mut ctx).unwrap(), DfdlValue::Int(10));
        let ast_conv6 = crate::expr::parse_expr("xs:int(xs:unsignedByte(10))").unwrap();
        assert_eq!(eval_expr(&ast_conv6, &mut ctx).unwrap(), DfdlValue::Int(10));

        // 7. Value conversions in value_to_i64 and value_to_f64
        // Bit shifts and rounding operations accepting unsigned types and decimals.
        let ast_shift =
            crate::expr::parse_expr("dfdl:leftShift(xs:unsignedShort(10), xs:unsignedByte(2))")
                .unwrap();
        assert_eq!(
            eval_expr(&ast_shift, &mut ctx).unwrap(),
            DfdlValue::UnsignedShort(40)
        );

        let ast_floor = crate::expr::parse_expr("fn:floor(xs:unsignedInt(42))").unwrap();
        assert_eq!(
            eval_expr(&ast_floor, &mut ctx).unwrap(),
            DfdlValue::UnsignedInt(42)
        );

        let ast_ceil = crate::expr::parse_expr("fn:ceiling(xs:short(17))").unwrap();
        assert_eq!(
            eval_expr(&ast_ceil, &mut ctx).unwrap(),
            DfdlValue::Short(17)
        );

        // 8. Complex element value length with prefixed child terms
        // Tests recursive valueLength calculation across nested complex elements with prefix types.
        let mut comp_elem = InfosetElement::complex(crate::types::QName::local("complexRoot"));
        let sub_child1 = InfosetElement::simple(
            crate::types::QName::local("sub1"),
            ElementState::Value(DfdlValue::String("data".into())),
        );
        let sub_child2 = InfosetElement::simple(
            crate::types::QName::local("sub2"),
            ElementState::Value(DfdlValue::String("more".into())),
        );
        comp_elem.children.push(InfosetNode::Element(sub_child1));
        comp_elem.children.push(InfosetNode::Element(sub_child2));

        let child1_props = crate::schema::ir::ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Prefixed,
            prefix_length_type: Some(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc(
                "prefix:len:16:bits",
            )),
            initiator: Some("[".into()),
            terminator: Some("]".into()),
            ..Default::default()
        };

        let child2_props = crate::schema::ir::ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Prefixed,
            prefix_length_type: Some(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc(
                "xs:int",
            )),
            ..Default::default()
        };

        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let elem1 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sub1"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::value::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id1 = builder
            .add_term_with_props(
                crate::types::QName::local("sub1"),
                crate::schema::ir::TermKind::Element(elem1),
                child1_props,
            )
            .unwrap();

        let elem2 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sub2"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::value::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id2 = builder
            .add_term_with_props(
                crate::types::QName::local("sub2"),
                crate::schema::ir::TermKind::Element(elem2),
                child2_props,
            )
            .unwrap();

        let seq_id = builder
            .add_term(
                crate::types::QName::local("seq"),
                crate::schema::ir::TermKind::Sequence(crate::schema::ir::CompiledSequence {
                    members: alloc::vec![id1, id2],
                }),
            )
            .unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("complexRoot"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(
                crate::types::QName::local("complexRoot"),
                crate::schema::ir::TermKind::Element(root_elem),
            )
            .unwrap();
        builder.set_root(root_id);
        let sch = builder.build().unwrap();

        let mut doc2 = InfosetDocument::new();
        doc2.root = Some(comp_elem);
        let mut ctx2 = ExprContext::new(Some(&doc2), &p_root, &[], &mut budget).with_schema(&sch);

        let ast_vl_comp =
            crate::expr::parse_expr("dfdl:valueLength(complexRoot, 'bytes')").unwrap();
        let res_vl = eval_expr(&ast_vl_comp, &mut ctx2).unwrap();
        // sub1: 4 data + 2 prefix + 1 init + 1 term = 8
        // sub2: 4 data + 4 prefix = 8
        // total = 16
        assert_eq!(res_vl, DfdlValue::Long(16));

        // 9. checkConstraints self-referencing error
        // Self-referencing constraint verification without an element value throws an error.
        let mut p_empty = InfosetPath::root();
        p_empty.try_push("missingElem").unwrap();
        let mut ctx3 = ExprContext::new(None, &p_empty, &[], &mut budget);
        let ast_cc = crate::expr::parse_expr("dfdl:checkConstraints()").unwrap();
        assert!(eval_expr(&ast_cc, &mut ctx3).is_err());
    }

    /// Verifies unary, binary arithmetic, casts, and value length calculations across all DFDL value representations.
    #[test]
    fn test_expr_eval_extended_operations_and_casts() {
        // 1. Unary negation across numeric types and overflow paths
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Int(10)).unwrap(),
            DfdlValue::Int(-10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Long(100)).unwrap(),
            DfdlValue::Long(-100)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Short(5)).unwrap(),
            DfdlValue::Short(-5)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::Byte(3)).unwrap(),
            DfdlValue::Byte(-3)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(10)).unwrap(),
            DfdlValue::Long(-10)
        );
        assert_eq!(
            eval_unary(
                UnaryOp::Negate,
                DfdlValue::UnsignedLong(9_223_372_036_854_775_808)
            )
            .unwrap(),
            DfdlValue::Long(i64::MIN)
        );
        assert_eq!(
            eval_unary(
                UnaryOp::Negate,
                DfdlValue::UnsignedLong(10_000_000_000_000_000_000)
            )
            .unwrap(),
            DfdlValue::Decimal("-10000000000000000000".into())
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedInt(10)).unwrap(),
            DfdlValue::Long(-10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedShort(10)).unwrap(),
            DfdlValue::Int(-10)
        );
        assert_eq!(
            eval_unary(UnaryOp::Negate, DfdlValue::UnsignedByte(10)).unwrap(),
            DfdlValue::Int(-10)
        );
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Int(i32::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Long(i64::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Short(i16::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(i8::MIN)).is_err());

        // 2. Binary addition across mixed and concrete types
        assert_eq!(
            eval_add(DfdlValue::Long(10), DfdlValue::Long(20)).unwrap(),
            DfdlValue::Long(30)
        );
        assert_eq!(
            eval_add(DfdlValue::Int(5), DfdlValue::Int(15)).unwrap(),
            DfdlValue::Int(20)
        );
        assert_eq!(
            eval_add(DfdlValue::Int(5), DfdlValue::Long(25)).unwrap(),
            DfdlValue::Long(30)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(25), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Long(30)
        );
        assert_eq!(
            eval_add(DfdlValue::Double(2.5), DfdlValue::Double(1.5)).unwrap(),
            DfdlValue::Double(4.0)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(2), DfdlValue::Double(3.5)).unwrap(),
            DfdlValue::Double(5.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Double(3.5), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(5.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_add(DfdlValue::Long(2), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_add(DfdlValue::Float(3.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(12.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(12.5)
        );
        assert_eq!(
            eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Decimal("15".into())
        );
        assert!(eval_add(DfdlValue::Long(i64::MAX), DfdlValue::Long(1)).is_err());
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Int(1)).is_err());

        // 3. Binary subtraction across mixed and concrete types
        assert_eq!(
            eval_sub(DfdlValue::Long(30), DfdlValue::Long(10)).unwrap(),
            DfdlValue::Long(20)
        );
        assert_eq!(
            eval_sub(DfdlValue::Int(20), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Int(15)
        );
        assert_eq!(
            eval_sub(DfdlValue::Int(30), DfdlValue::Long(5)).unwrap(),
            DfdlValue::Long(25)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(30), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Long(25)
        );
        assert_eq!(
            eval_sub(DfdlValue::Double(5.5), DfdlValue::Double(1.5)).unwrap(),
            DfdlValue::Double(4.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Double(10.0), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Double(7.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Float(5.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(3.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(7.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Float(10.0), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Float(7.0)
        );
        assert_eq!(
            eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(7.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Long(3)).unwrap(),
            DfdlValue::Decimal("7".into())
        );
        assert_eq!(
            eval_sub(DfdlValue::Double(10.0), DfdlValue::Decimal("2.5".into())).unwrap(),
            DfdlValue::Double(7.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Float(10.0), DfdlValue::Decimal("2.5".into())).unwrap(),
            DfdlValue::Float(7.5)
        );
        assert_eq!(
            eval_sub(DfdlValue::Long(10), DfdlValue::Decimal("3".into())).unwrap(),
            DfdlValue::Decimal("7".into())
        );
        assert!(eval_sub(DfdlValue::Long(i64::MIN), DfdlValue::Long(1)).is_err());
        assert!(eval_sub(DfdlValue::Int(i32::MIN), DfdlValue::Int(1)).is_err());

        // 4. Binary multiplication across mixed and concrete types
        assert_eq!(
            eval_mul(DfdlValue::Long(10), DfdlValue::Long(20)).unwrap(),
            DfdlValue::Long(200)
        );
        assert_eq!(
            eval_mul(DfdlValue::Int(5), DfdlValue::Int(4)).unwrap(),
            DfdlValue::Int(20)
        );
        assert_eq!(
            eval_mul(DfdlValue::Int(5), DfdlValue::Long(4)).unwrap(),
            DfdlValue::Long(20)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(4), DfdlValue::Int(5)).unwrap(),
            DfdlValue::Long(20)
        );
        assert_eq!(
            eval_mul(DfdlValue::Double(2.5), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(4), DfdlValue::Double(2.5)).unwrap(),
            DfdlValue::Double(10.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Double(2.5), DfdlValue::Long(4)).unwrap(),
            DfdlValue::Double(10.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(),
            DfdlValue::Float(6.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Long(4), DfdlValue::Float(2.5)).unwrap(),
            DfdlValue::Float(10.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Float(2.5), DfdlValue::Long(4)).unwrap(),
            DfdlValue::Float(10.0)
        );
        assert_eq!(
            eval_mul(DfdlValue::Decimal("5".into()), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(10.0)
        );
        assert!(eval_mul(DfdlValue::Long(i64::MAX), DfdlValue::Long(2)).is_err());
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Int(2)).is_err());

        // 5. Binary division across mixed and concrete types
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(10), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(10), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Double(10.0), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Long(10), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Int(10), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Float(10.0), DfdlValue::Int(2)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.0)).unwrap(),
            DfdlValue::Double(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.0)).unwrap(),
            DfdlValue::Float(5.0)
        );
        assert_eq!(
            eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Long(2)).unwrap(),
            DfdlValue::Decimal("5".into())
        );
        assert!(eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Long(0)).is_err());

        // 6. value_to_bool conversions across all variants
        assert!(value_to_bool(&DfdlValue::Boolean(true)));
        assert!(!value_to_bool(&DfdlValue::Boolean(false)));
        assert!(value_to_bool(&DfdlValue::Int(1)));
        assert!(!value_to_bool(&DfdlValue::Int(0)));
        assert!(value_to_bool(&DfdlValue::Long(1)));
        assert!(!value_to_bool(&DfdlValue::Long(0)));
        assert!(value_to_bool(&DfdlValue::Short(1)));
        assert!(!value_to_bool(&DfdlValue::Short(0)));
        assert!(value_to_bool(&DfdlValue::Byte(1)));
        assert!(!value_to_bool(&DfdlValue::Byte(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedLong(1)));
        assert!(!value_to_bool(&DfdlValue::UnsignedLong(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedInt(1)));
        assert!(!value_to_bool(&DfdlValue::UnsignedInt(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedShort(1)));
        assert!(!value_to_bool(&DfdlValue::UnsignedShort(0)));
        assert!(value_to_bool(&DfdlValue::UnsignedByte(1)));
        assert!(!value_to_bool(&DfdlValue::UnsignedByte(0)));
        assert!(value_to_bool(&DfdlValue::Float(1.0)));
        assert!(!value_to_bool(&DfdlValue::Float(0.0)));
        assert!(value_to_bool(&DfdlValue::Double(1.0)));
        assert!(!value_to_bool(&DfdlValue::Double(0.0)));
        assert!(value_to_bool(&DfdlValue::String("true".into())));
        assert!(value_to_bool(&DfdlValue::String("1".into())));
        assert!(!value_to_bool(&DfdlValue::String("false".into())));
        assert!(!value_to_bool(&DfdlValue::HexBinary(alloc::vec![])));

        // 7. eval_fn_call for boolean, not, sayHello, addBoxed, addPrimitive
        let mut budget = WorkBudget::new(1000);
        let path = InfosetPath::root();
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Long(0)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Short(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Byte(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedLong(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedShort(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::UnsignedByte(1)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Float(1.0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("boolean"), &[DfdlValue::Double(0.0)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::String("true".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("boolean"),
                &[DfdlValue::String("false".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );
        assert!(eval_fn_call(
            &QName::local("boolean"),
            &[DfdlValue::String("invalid".into())],
            &ctx
        )
        .is_err());

        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Int(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Long(0)], &ctx).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Short(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Byte(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedLong(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedShort(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedByte(1)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Float(1.0)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(&QName::local("not"), &[DfdlValue::Double(1.0)], &ctx).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("not"),
                &[DfdlValue::String("true".into())],
                &ctx
            )
            .unwrap(),
            DfdlValue::Boolean(false)
        );

        assert_eq!(
            eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap(),
            DfdlValue::String("Hello".into())
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addBoxed"),
                &[DfdlValue::Int(3), DfdlValue::Long(4)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(7)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addPrimitive"),
                &[DfdlValue::Short(2), DfdlValue::Byte(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(5)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addPrimitive"),
                &[DfdlValue::UnsignedInt(2), DfdlValue::UnsignedLong(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(5)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addPrimitive"),
                &[DfdlValue::UnsignedShort(2), DfdlValue::UnsignedByte(3)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(5)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("addPrimitive"),
                &[DfdlValue::String("10".into()), DfdlValue::Int(5)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Int(15)
        );
        assert!(eval_fn_call(
            &QName::local("addPrimitive"),
            &[DfdlValue::String("nan".into()), DfdlValue::Int(5)],
            &ctx
        )
        .is_err());

        // 8. eval_fn_call hexBinary conversions
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::HexBinary(alloc::vec![0xAA])],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0xAA])
        );
        assert_eq!(
            eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Byte(0x12)], &ctx).unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::UnsignedByte(0x34)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x34])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::Short(0x1234)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::UnsignedShort(0x5678)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x56, 0x78])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::Int(0x12345678)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78])
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("hexBinary"),
                &[DfdlValue::Long(0x123456789ABCDEF0)],
                &ctx
            )
            .unwrap(),
            DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0])
        );

        // 9. eval_fn_call double and float conversions
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Double(3.25)], &ctx).unwrap(),
            DfdlValue::Double(3.25)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Float(2.5)], &ctx).unwrap(),
            DfdlValue::Double(2.5)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Int(10)], &ctx).unwrap(),
            DfdlValue::Double(10.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Long(20)], &ctx).unwrap(),
            DfdlValue::Double(20.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Short(30)], &ctx).unwrap(),
            DfdlValue::Double(30.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Byte(40)], &ctx).unwrap(),
            DfdlValue::Double(40.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedLong(50)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(50.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedInt(60)], &ctx).unwrap(),
            DfdlValue::Double(60.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedShort(70)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(70.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("double"),
                &[DfdlValue::UnsignedByte(80)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Double(80.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Double(1.0)
        );

        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Float(2.5)], &ctx).unwrap(),
            DfdlValue::Float(2.5)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Double(3.5)], &ctx).unwrap(),
            DfdlValue::Float(3.5)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Int(10)], &ctx).unwrap(),
            DfdlValue::Float(10.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Long(20)], &ctx).unwrap(),
            DfdlValue::Float(20.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Short(30)], &ctx).unwrap(),
            DfdlValue::Float(30.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Byte(40)], &ctx).unwrap(),
            DfdlValue::Float(40.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedLong(50)], &ctx).unwrap(),
            DfdlValue::Float(50.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedInt(60)], &ctx).unwrap(),
            DfdlValue::Float(60.0)
        );
        assert_eq!(
            eval_fn_call(
                &QName::local("float"),
                &[DfdlValue::UnsignedShort(70)],
                &ctx
            )
            .unwrap(),
            DfdlValue::Float(70.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedByte(80)], &ctx).unwrap(),
            DfdlValue::Float(80.0)
        );
        assert_eq!(
            eval_fn_call(&QName::local("float"), &[DfdlValue::Boolean(true)], &ctx).unwrap(),
            DfdlValue::Float(1.0)
        );

        // 10. eval_fn_call integer casting functions
        assert_eq!(
            eval_fn_call(&QName::local("int"), &[DfdlValue::Long(42)], &ctx).unwrap(),
            DfdlValue::Int(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("long"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::Long(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("short"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::Short(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("byte"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::Byte(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("unsignedByte"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::UnsignedByte(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("unsignedShort"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::UnsignedShort(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("unsignedInt"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::UnsignedInt(42)
        );
        assert_eq!(
            eval_fn_call(&QName::local("unsignedLong"), &[DfdlValue::Int(42)], &ctx).unwrap(),
            DfdlValue::UnsignedLong(42)
        );

        // 11. calc_elem_value_length fallbacks without schema (lines 866-877)
        let make_elem =
            |v: DfdlValue| InfosetElement::simple(QName::local("test"), ElementState::Value(v));
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::String("hello".into())),
                None,
                &path,
                None
            ),
            5
        );
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::HexBinary(alloc::vec![1, 2, 3])),
                None,
                &path,
                None
            ),
            3
        );
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::DateTime("2026-01-01".into())),
                None,
                &path,
                None
            ),
            10
        );
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::Date("2026-01-01".into())),
                None,
                &path,
                None
            ),
            10
        );
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::Time("12:00:00".into())),
                None,
                &path,
                None
            ),
            8
        );
        assert_eq!(
            calc_elem_value_length(
                &make_elem(DfdlValue::Decimal("123.45".into())),
                None,
                &path,
                None
            ),
            6
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Int(1)), None, &path, None),
            4
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::UnsignedInt(1)), None, &path, None),
            4
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Float(1.0)), None, &path, None),
            4
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Long(1)), None, &path, None),
            8
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::UnsignedLong(1)), None, &path, None),
            8
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Double(1.0)), None, &path, None),
            8
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Short(1)), None, &path, None),
            2
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::UnsignedShort(1)), None, &path, None),
            2
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Byte(1)), None, &path, None),
            1
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::UnsignedByte(1)), None, &path, None),
            1
        );
        assert_eq!(
            calc_elem_value_length(&make_elem(DfdlValue::Boolean(true)), None, &path, None),
            1
        );
    }
