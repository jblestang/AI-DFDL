//! Unit tests for DFDL Property Resolution, Scoping, and Entity Management.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

extern crate alloc;
use alloc::string::{String, ToString};

use crate::error::DFDLErrorKind;
use crate::io::traits::ByteOrder;
use crate::schema::ir::{LengthKind, ParseUnparsePolicy};

use super::entities::*;
use super::keys::*;
use super::resolution::*;
use super::scope::*;
use super::store::*;

    /// Literal and computed byteOrder values share one validator (DFDL §13.1).
    #[test]
    fn test_parse_byte_order_accepts_known_and_rejects_unknown() {
        assert_eq!(parse_byte_order("bigEndian").unwrap(), ByteOrder::BigEndian);
        assert_eq!(parse_byte_order(" LE ").unwrap(), ByteOrder::LittleEndian);
        let err = parse_byte_order("middleEndian").unwrap_err();
        assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
        assert!(err.to_string().contains("Unknown value for byteOrder property: middleEndian"));
    }

    #[test]
    fn test_static_byte_order_literal_is_validated() {
        let mut store = PropertyStore::new();
        store.set_property("byteOrder", "fatEndian").unwrap();
        let err = store.to_resolved_properties(None).unwrap_err();
        assert!(err.to_string().contains("fatEndian"));
    }

    #[test]
    fn test_property_forbids_literal_whitespace() {
        let mut store = PropertyStore::new();
        // escapeCharacter with raw space must fail
        store.set_property("escapeCharacter", " ").unwrap();
        let err = store.validate_property_entities().unwrap_err();
        assert!(err.to_string().contains("must not contain any whitespace"));
        assert!(err.to_string().contains("Use DFDL Entities"));

        // escapeBlockStart with raw space must fail
        let mut store2 = PropertyStore::new();
        store2
            .set_property("escapeBlockStart", "[ start ]")
            .unwrap();
        let err2 = store2.validate_property_entities().unwrap_err();
        assert!(err2.to_string().contains("must not contain any whitespace"));

        // textStringPadCharacter with raw space must fail
        let mut store3 = PropertyStore::new();
        store3.set_property("textStringPadCharacter", " ").unwrap();
        let err3 = store3.validate_property_entities().unwrap_err();
        assert!(err3.to_string().contains("must not contain any whitespace"));

        // Entity %SP; must succeed
        let mut store4 = PropertyStore::new();
        assert!(store4.set_property("escapeCharacter", "%SP;").is_ok());
        assert!(store4.validate_property_entities().is_ok());
        let mut store5 = PropertyStore::new();
        assert!(store5
            .set_property("escapeBlockStart", "[%SP;start%SP;]")
            .is_ok());
        assert!(store5.validate_property_entities().is_ok());
    }

    #[test]
    fn test_byte_entity_prohibition_and_separator_validation() {
        let mut store = PropertyStore::new();
        // Byte entity in textStandardDecimalSeparator must fail
        store
            .set_property("textStandardDecimalSeparator", "%#r2E;")
            .unwrap();
        let err = store.validate_property_entities().unwrap_err();
        assert!(err.to_string().contains("DFDL Byte Entity"));

        // Byte entity in fillByte must succeed
        let mut store2 = PropertyStore::new();
        assert!(store2.set_property("fillByte", "%#r20;").is_ok());
        assert!(store2.validate_property_entities().is_ok());

        // More than one char in textStandardDecimalSeparator must fail in resolution
        let mut store2 = PropertyStore::new();
        assert!(store2
            .set_property("textStandardDecimalSeparator", ". , *")
            .is_ok());
        let res = store2.to_resolved_properties(None);
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("more than one character"));

        // Duplicate / non-distinct values must fail in resolution
        let mut store3 = PropertyStore::new();
        assert!(store3
            .set_property("textStandardDecimalSeparator", ".")
            .is_ok());
        assert!(store3
            .set_property("textStandardGroupingSeparator", ".")
            .is_ok());
        let res3 = store3.to_resolved_properties(None);
        assert!(res3.is_err());
        assert!(res3
            .unwrap_err()
            .to_string()
            .contains("Non-distinct property"));
    }

    #[test]
    fn test_single_character_property_validation() {
        // Multi-character literal pad character
        let mut store1 = PropertyStore::new();
        store1.set_property("textStringPadCharacter", "o0").unwrap();
        let err1 = store1.validate_property_entities().unwrap_err();
        assert!(err1
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Multi-entity pad character
        let mut store2 = PropertyStore::new();
        store2
            .set_property("textStringPadCharacter", "%HT;%CR;")
            .unwrap();
        let err2 = store2.validate_property_entities().unwrap_err();
        assert!(err2
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Character class %NL; in single character property
        let mut store3 = PropertyStore::new();
        store3
            .set_property("textStandardGroupingSeparator", "%NL;")
            .unwrap();
        let err3 = store3.validate_property_entities().unwrap_err();
        assert!(err3.to_string().contains("disallowed character class"));

        // Empty string in textStandardGroupingSeparator
        let mut store4 = PropertyStore::new();
        store4
            .set_property("textStandardGroupingSeparator", "")
            .unwrap();
        let err4 = store4.to_resolved_properties(None).unwrap_err();
        assert!(err4
            .to_string()
            .contains("Length of string must be exactly 1 character"));

        // Valid single character entity %SP;
        let mut store5 = PropertyStore::new();
        store5
            .set_property("textStringPadCharacter", "%SP;")
            .unwrap();
        assert!(store5.validate_property_entities().is_ok());
    }

    #[test]
    fn test_parse_unparse_policy_validation() {
        // Valid policies
        let mut store1 = PropertyStore::new();
        store1.set_property("parseUnparsePolicy", "parseOnly").unwrap();
        let resolved = store1.to_resolved_properties(None).unwrap();
        assert_eq!(resolved.parse_unparse_policy, ParseUnparsePolicy::ParseOnly);

        let mut store2 = PropertyStore::new();
        store2.set_property("parseUnparsePolicy", "unparseOnly").unwrap();
        let resolved2 = store2.to_resolved_properties(None).unwrap();
        assert_eq!(resolved2.parse_unparse_policy, ParseUnparsePolicy::UnparseOnly);

        // Invalid policy
        let mut store3 = PropertyStore::new();
        store3.set_property("parseUnparsePolicy", "invalid").unwrap();
        let err3 = store3.to_resolved_properties(None).unwrap_err();
        assert!(err3.to_string().contains("Invalid parseUnparsePolicy"));

        // Incompatible parent parseOnly and child unparseOnly
        let mut parent_store = PropertyStore::new();
        parent_store.set_property("parseUnparsePolicy", "parseOnly").unwrap();
        let mut child_store = PropertyStore::new();
        child_store.set_property("parseUnparsePolicy", "unparseOnly").unwrap();
        let err4 = child_store.to_resolved_properties(Some(&parent_store)).unwrap_err();
        assert!(err4.to_string().contains("Incompatible parseUnparsePolicy"));
    }
    /// fillByte presence is tracked so the unparser can reject padding without it.
    #[test]
    fn test_fill_byte_defined_flag() {
        let mut store = PropertyStore::new();
        assert!(!store.to_resolved_properties(None).unwrap().fill_byte_defined);
        store.set_property("fillByte", "%NUL;").unwrap();
        assert!(store.to_resolved_properties(None).unwrap().fill_byte_defined);
    }

    /// Verifies that extend_excluding copies properties except those in the exclude store.
    #[test]
    fn test_extend_excluding() {
        let mut target = PropertyStore::new();
        target.set_property("representation", "text").unwrap();

        let mut st_props = PropertyStore::new();
        st_props.set_property("length", "10").unwrap();
        st_props.set_property("encoding", "UTF-8").unwrap();

        let mut direct_props = PropertyStore::new();
        direct_props.set_property("length", "{ 2 }").unwrap();

        target.extend_excluding(&st_props, &direct_props);

        // "length" was excluded because it exists in direct_props
        assert_eq!(target.get_property("length"), None);
        // "encoding" was inherited from st_props
        assert_eq!(target.get_property("encoding"), Some("UTF-8"));
        // "representation" was preserved
        assert_eq!(target.get_property("representation"), Some("text"));
    }

    #[test]
    fn test_dfdl_entities_encode_decode_roundtrip() {
        assert_eq!(decode_dfdl_character_entities("%NEL;"), "\u{0085}");
        assert_eq!(decode_dfdl_character_entities("%LF;"), "\n");
        assert_eq!(decode_dfdl_character_entities("%CR;"), "\r");
        assert_eq!(decode_dfdl_character_entities("%CR;%LF;"), "\r\n");
        assert_eq!(decode_dfdl_character_entities("%%"), "%");

        assert_eq!(encode_dfdl_character_entities("\u{0085}"), "%NEL;");
        assert_eq!(encode_dfdl_character_entities("\n"), "%LF;");
        assert_eq!(encode_dfdl_character_entities("\r"), "%CR;");
        assert_eq!(encode_dfdl_character_entities("\r\n"), "%CR;%LF;");
        assert_eq!(encode_dfdl_character_entities("%"), "%%");
        assert_eq!(encode_dfdl_character_entities("hello\nworld%"), "hello%LF;world%%");
    }

    #[test]
    fn test_property_store_remove_property() {
        let mut store = PropertyStore::new();
        store.set_property("ref", "myFormat").unwrap();
        store.set_property("dfdl:lengthKind", "explicit").unwrap();
        assert_eq!(store.get_property("ref"), Some("myFormat"));
        assert_eq!(store.get_property("lengthKind"), Some("explicit"));

        store.remove_property("ref");
        assert_eq!(store.get_property("ref"), None);

        store.remove_property("dfdl:lengthKind");
        assert_eq!(store.get_property("lengthKind"), None);
    }

    #[test]
    fn test_property_store_update_namespaces_and_inheritance() {
        let mut store = PropertyStore::new();
        store.add_namespaces(&[
            (String::new(), alloc::string::String::from("urn:default")),
            (alloc::string::String::from("ex1"), alloc::string::String::from("urn:old")),
        ]);

        // Update namespaces simulates an inner element shadowing a prefix
        store.update_namespaces(&[
            (alloc::string::String::from("ex1"), alloc::string::String::from("http://example.com/new")),
            (alloc::string::String::from("ex2"), alloc::string::String::from("http://example.com/2")),
        ]);

        let ns = store.in_scope_namespaces();
        assert_eq!(
            ns.iter().find(|(p, _)| p == "ex1").map(|(_, u)| u.as_str()),
            Some("http://example.com/new")
        );
        assert_eq!(
            ns.iter().find(|(p, _)| p == "ex2").map(|(_, u)| u.as_str()),
            Some("http://example.com/2")
        );
        assert_eq!(
            ns.iter().find(|(p, _)| p.is_empty()).map(|(_, u)| u.as_str()),
            Some("urn:default")
        );

        // Test extending another store preserves target's existing prefix
        let mut child_store = PropertyStore::new();
        child_store.add_namespaces(&[
            (alloc::string::String::from("ex1"), alloc::string::String::from("http://example.com/child")),
        ]);
        child_store.extend(&store);
        let child_ns = child_store.in_scope_namespaces();
        assert_eq!(
            child_ns.iter().find(|(p, _)| p == "ex1").map(|(_, u)| u.as_str()),
            Some("http://example.com/child")
        );
        assert_eq!(
            child_ns.iter().find(|(p, _)| p == "ex2").map(|(_, u)| u.as_str()),
            Some("http://example.com/2")
        );
    }

    /// Verifies xs:dateTime validation coverage across all branches and binaryCalendarEpoch resolution.
    #[test]
    fn test_is_valid_xs_date_time_and_binary_calendar_epoch() {
        // Valid date times
        assert!(is_valid_xs_date_time("2026-10-07T23:59:59Z"));
        assert!(is_valid_xs_date_time("1970-01-01T00:00:00+02:00"));
        assert!(is_valid_xs_date_time("-0044-03-15T12:30:00-05:00"));
        assert!(is_valid_xs_date_time("2020-02-29T23:59:59.123456Z"));
        assert!(is_valid_xs_date_time("2020-01-01T24:00:00Z"));
        assert!(is_valid_xs_date_time("2020-01-01T12:00:00"));

        // Invalid date times
        assert!(!is_valid_xs_date_time("notADateTime"));
        assert!(!is_valid_xs_date_time("2020-01T12:00:00"));
        assert!(!is_valid_xs_date_time("20-01-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-13-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-00-01T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-32T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-00T12:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:00+0200"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:00."));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T25:00:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:60:00"));
        assert!(!is_valid_xs_date_time("2020-01-01T12:00:61"));
        assert!(!is_valid_xs_date_time("2020-01-01T24:01:00"));

        // binaryCalendarEpoch property validation
        let mut store_valid = PropertyStore::new();
        store_valid.set_property("binaryCalendarEpoch", "1970-01-01T00:00:00Z").unwrap();
        assert!(store_valid.to_resolved_properties(None).is_ok());

        let mut store_invalid = PropertyStore::new();
        store_invalid.set_property("binaryCalendarEpoch", "invalid-epoch").unwrap();
        assert!(store_invalid.to_resolved_properties(None).is_err());
    }

    #[test]
    fn test_property_store_methods_and_character_entities() {
        // 1. All character entity codes in encode and decode
        let all_ctrls = "\u{2028}\0\t\x7F\x1B\x07\x08\x0C\x0B\x18\x06\x15\x05\x04\x03\x02\x01\x0E\x0F\x16\x17\x19\x1A\x1C\x1D\x1E\x1F\x10\x11\x12\x13\x14";
        let encoded = encode_dfdl_character_entities(all_ctrls);
        assert!(encoded.contains("%LS;"));
        assert!(encoded.contains("%NUL;"));
        assert!(encoded.contains("%HT;"));
        assert!(encoded.contains("%DEL;"));
        assert!(encoded.contains("%ESC;"));
        assert!(encoded.contains("%BEL;"));
        assert!(encoded.contains("%BS;"));
        assert!(encoded.contains("%FF;"));
        assert!(encoded.contains("%VT;"));
        assert!(encoded.contains("%CAN;"));
        assert!(encoded.contains("%ACK;"));
        assert!(encoded.contains("%NAK;"));
        assert!(encoded.contains("%ENQ;"));
        assert!(encoded.contains("%EOT;"));
        assert!(encoded.contains("%ETX;"));
        assert!(encoded.contains("%STX;"));
        assert!(encoded.contains("%SOH;"));
        assert!(encoded.contains("%SO;"));
        assert!(encoded.contains("%SI;"));
        assert!(encoded.contains("%SYN;"));
        assert!(encoded.contains("%ETB;"));
        assert!(encoded.contains("%EM;"));
        assert!(encoded.contains("%SUB;"));
        assert!(encoded.contains("%FS;"));
        assert!(encoded.contains("%GS;"));
        assert!(encoded.contains("%RS;"));
        assert!(encoded.contains("%US;"));
        assert!(encoded.contains("%DLE;"));
        assert!(encoded.contains("%DC1;"));
        assert!(encoded.contains("%DC2;"));
        assert!(encoded.contains("%DC3;"));
        assert!(encoded.contains("%DC4;"));

        let decoded = decode_dfdl_character_entities(&encoded);
        assert_eq!(decoded, all_ctrls);

        // 2. is_empty, add_set_variable, add_new_variable_instance, add_assert_error
        let mut s = PropertyStore::new();
        assert!(s.is_empty());
        s.add_set_variable("tns:myVar", "10");
        assert!(!s.is_empty());
        assert_eq!(s.set_variables().len(), 1);
        assert_eq!(s.set_variables()[0].0.local_name, "myVar");

        s.add_new_variable_instance("ex:newVar", Some("42"));
        assert_eq!(s.new_variable_instances().len(), 1);

        s.add_assert_error("some error");
        assert_eq!(s.assert_errors(), &["some error"]);

        // 3. override_with
        let mut other = PropertyStore::new();
        other.set_property("byteOrder", "littleEndian").unwrap();
        other.add_set_variable("otherVar", "99");
        other.add_new_variable_instance("otherInst", None);
        other.add_assert_error("other error");
        s.override_with(&other);
        assert_eq!(s.get_property("byteOrder"), Some("littleEndian"));
        assert_eq!(s.set_variables().len(), 2);
        assert_eq!(s.new_variable_instances().len(), 2);
        assert_eq!(s.assert_errors().len(), 2);

        // 4. merge_parent non-inheritable property exclusions
        let mut parent = PropertyStore::new();
        parent.set_property("inputValueCalc", "{42}").unwrap();
        parent.set_property("outputValueCalc", "{42}").unwrap();
        parent.set_property("initiator", "%SP;").unwrap();
        parent.set_property("terminator", "%NL;").unwrap();
        parent.set_property("separator", ",").unwrap();
        parent.set_property("sequenceKind", "ordered").unwrap();
        parent.set_property("initiatedContent", "no").unwrap();
        parent.set_property("choiceDispatchKey", "{.}").unwrap();
        parent.set_property("choiceBranchKey", "1").unwrap();
        parent.set_property("hiddenGroupRef", "tns:g").unwrap();
        parent.set_property("length", "10").unwrap();
        parent.set_property("lengthKind", "explicit").unwrap();
        parent.set_property("occursCount", "5").unwrap();
        parent.set_property("occursCountKind", "fixed").unwrap();
        parent.set_property("fillByte", "0").unwrap();
        parent.set_property("prefixLengthType", "tns:len").unwrap();
        parent.set_property("representation", "binary").unwrap();
        parent.set_property("repType", "tns:rep").unwrap();
        parent.set_property("repValues", "1 2").unwrap();
        parent.set_property("encoding", "UTF-8").unwrap(); // inheritable!

        let mut child = PropertyStore::new();
        child.merge_parent(&parent);
        assert_eq!(child.get_property("encoding"), Some("UTF-8"));
        assert_eq!(child.get_property("inputValueCalc"), None);
        assert_eq!(child.get_property("outputValueCalc"), None);
        assert_eq!(child.get_property("initiator"), None);
        assert_eq!(child.get_property("terminator"), None);
        assert_eq!(child.get_property("separator"), None);
        assert_eq!(child.get_property("choiceDispatchKey"), None);
        assert_eq!(child.get_property("length"), None);

        // 5. Property syntax validation errors
        assert!(validate_dfdl_property_entities("initiator", "bad\x01val").is_err());
        assert!(validate_dfdl_property_entities("escapeCharacter", "%WSP;").is_err());
        assert!(validate_dfdl_property_entities("textStandardZeroRep", "%NL;").is_err());

        // 6. %ES; empty entity decoding and CRLF/control encoding
        assert_eq!(decode_dfdl_character_entities("a%ES;b"), "ab");
        let enc_ctrl = encode_dfdl_character_entities("a\r\nb\x01\u{0080}c");
        assert!(enc_ctrl.contains("%CR;%LF;"));
        assert!(enc_ctrl.contains("%SOH;"));
        assert!(enc_ctrl.contains("%#x80;"));

        // 7. extend_excluding with variables, repValue, and enumeration
        let mut s_src = PropertyStore::new();
        s_src.add_set_variable("var1", "{1}");
        s_src.add_new_variable_instance("var2", Some("{2}"));
        s_src.set_property("repValue:1", "one").unwrap();
        s_src.set_property("enumeration", "val").unwrap();

        let mut s_dest = PropertyStore::new();
        let s_excl = PropertyStore::new();
        s_dest.extend_excluding(&s_src, &s_excl);
        assert_eq!(s_dest.set_variables().len(), 1);
        assert_eq!(s_dest.new_variable_instances().len(), 1);
        assert_eq!(s_dest.get_property("repValue:1"), Some("one"));
        assert_eq!(s_dest.get_property("enumeration"), Some("val"));

        // 8. leadingSkip and trailingSkip invalid property validation
        let mut s_skips = PropertyStore::new();
        s_skips.set_property("leadingSkip", "-5").unwrap();
        assert!(s_skips.to_resolved_properties(None).is_err());
        s_skips.set_property("leadingSkip", "not_a_number").unwrap();
        assert!(s_skips.to_resolved_properties(None).is_err());

        let mut s_tskip = PropertyStore::new();
        s_tskip.set_property("trailingSkip", "-1").unwrap();
        assert!(s_tskip.to_resolved_properties(None).is_err());
        s_tskip.set_property("trailingSkip", "invalid").unwrap();
        assert!(s_tskip.to_resolved_properties(None).is_err());

        // 9. fillByte validation error branches
        let mut s_fb1 = PropertyStore::new();
        s_fb1.set_property("fillByte", "").unwrap();
        assert!(s_fb1.to_resolved_properties(None).is_err());

        let mut s_fb2 = PropertyStore::new();
        s_fb2.set_property("fillByte", "%#d999;").unwrap();
        assert!(s_fb2.to_resolved_properties(None).is_err());

        let mut s_fb3 = PropertyStore::new();
        s_fb3.set_property("encoding", "X-DFDL-6-BIT-DFI-264.2").unwrap();
        s_fb3.set_property("fillByte", "A").unwrap();
        assert!(s_fb3.to_resolved_properties(None).is_err());

        let mut s_fb4 = PropertyStore::new();
        s_fb4.set_property("encoding", "UTF-8").unwrap();
        s_fb4.set_property("fillByte", "\u{00E9}").unwrap();
        assert!(s_fb4.to_resolved_properties(None).is_err());

        let mut s_fb5 = PropertyStore::new();
        s_fb5.set_property("fillByte", "ABC").unwrap();
        assert!(s_fb5.to_resolved_properties(None).is_err());
    }

    /// Verifies PropertyStore namespace retention, assertion builder helpers, and enum validation error branches.
    #[test]
    fn test_property_store_additional_coverage() {
        // 1. In-scope namespaces setter and getter roundtrip
        let mut store = PropertyStore::new();
        let ns = alloc::vec![("ns1".into(), "http://example.com/1".into())];
        store.set_in_scope_namespaces(ns);
        assert_eq!(store.in_scope_namespaces().len(), 1);

        // 2. Assertion builder methods
        store.add_assert("true()", Some("Assertion message"));
        store.add_assert_with_kind(crate::schema::ir::TestKind::Pattern, "[a-z]+", None);
        assert_eq!(store.asserts.len(), 2);

        // 3. Invalid bitOrder property rejection
        let mut s_bo = PropertyStore::new();
        s_bo.set_property("bitOrder", "unknownBitOrder").unwrap();
        assert!(s_bo.to_resolved_properties(None).is_err());

        // 4. lengthKind endOfParent and invalid lengthKind
        let mut s_eop = PropertyStore::new();
        s_eop.set_property("lengthKind", "endOfParent").unwrap();
        let props_eop = s_eop.to_resolved_properties(None).unwrap();
        assert_eq!(props_eop.length_kind, LengthKind::EndOfParent);

        let mut s_lk_bad = PropertyStore::new();
        s_lk_bad.set_property("lengthKind", "badLengthKind").unwrap();
        assert!(s_lk_bad.to_resolved_properties(None).is_err());

        // 5. Invalid separatorPosition and separatorSuppressionPolicy rejection
        let mut s_sp_bad = PropertyStore::new();
        s_sp_bad.set_property("separatorPosition", "badPos").unwrap();
        assert!(s_sp_bad.to_resolved_properties(None).is_err());

        let mut s_ssp_bad = PropertyStore::new();
        s_ssp_bad.set_property("separatorSuppressionPolicy", "badSSP").unwrap();
        assert!(s_ssp_bad.to_resolved_properties(None).is_err());
    }

    /// Verifies PropertyKey parsing, Display, typed getters, and hierarchical PropertyScope delegation (RUST-SCH-02, RUST-SCH-04).
    #[test]
    fn test_property_key_and_scope() {
        // 1. PropertyKey parsing & roundtrip
        assert_eq!(PropertyKey::parse("byteOrder"), Some(PropertyKey::ByteOrder));
        assert_eq!(PropertyKey::parse("dfdl:byteOrder"), Some(PropertyKey::ByteOrder));
        assert_eq!(PropertyKey::parse("dfdlx:repType"), Some(PropertyKey::RepType));
        assert_eq!(PropertyKey::parse("daf:layerTransform"), Some(PropertyKey::LayerTransform));
        assert_eq!(PropertyKey::parse("unknownNonStandardProperty"), None);

        assert_eq!(PropertyKey::ByteOrder.as_str(), "byteOrder");
        assert!(PropertyKey::ByteOrder.is_inheritable());
        assert!(!PropertyKey::Initiator.is_inheritable());

        // 2. Typed set and get in PropertyStore
        let mut store = PropertyStore::new();
        store.set_property_key(PropertyKey::ByteOrder, "bigEndian").unwrap();
        store.set_property("dfdl:encoding", "UTF-8").unwrap();
        store.set_property("customProp", "customVal").unwrap();

        assert_eq!(store.get_property_key(PropertyKey::ByteOrder), Some("bigEndian"));
        assert_eq!(store.get_property("byteOrder"), Some("bigEndian"));
        assert_eq!(store.get_property_key(PropertyKey::Encoding), Some("UTF-8"));
        assert_eq!(store.get_property("customProp"), Some("customVal"));

        store.remove_property_key(PropertyKey::Encoding);
        assert_eq!(store.get_property_key(PropertyKey::Encoding), None);

        // 3. PropertyScope hierarchical resolution (RUST-SCH-02)
        let mut parent_store = PropertyStore::new();
        parent_store.set_property_key(PropertyKey::ByteOrder, "littleEndian").unwrap();
        parent_store.set_property_key(PropertyKey::Initiator, "[").unwrap();

        let mut child_store = PropertyStore::new();
        child_store.set_property_key(PropertyKey::Encoding, "ASCII").unwrap();

        let parent_scope = PropertyScope::root(&parent_store);
        let child_scope = parent_scope.child(&child_store);

        // Direct child property
        assert_eq!(child_scope.get_property_key(PropertyKey::Encoding), Some("ASCII"));
        // Inherited parent property (byteOrder is inheritable)
        assert_eq!(child_scope.get_property_key(PropertyKey::ByteOrder), Some("littleEndian"));
        // Non-inheritable property does NOT leak to child scope (§13)
        assert_eq!(child_scope.get_property_key(PropertyKey::Initiator), None);
        // But parent scope still has its non-inheritable property
        assert_eq!(parent_scope.get_property_key(PropertyKey::Initiator), Some("["));

        // Materialize child scope into a merged PropertyStore
        let materialized = child_scope.materialize();
        assert_eq!(materialized.get_property_key(PropertyKey::Encoding), Some("ASCII"));
        assert_eq!(materialized.get_property_key(PropertyKey::ByteOrder), Some("littleEndian"));
        assert_eq!(materialized.get_property_key(PropertyKey::Initiator), None);
    }
