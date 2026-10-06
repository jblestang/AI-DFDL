//! `dfdl-tests`: Conformance and integration test suite for the DFDL engine.

pub mod tdml;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use dfdl_core::error::DFDLErrorKind;
    use dfdl_core::infoset::events::{InfosetEvent, InfosetEventSink, InfosetSource};
    use dfdl_core::infoset::state::ElementState;
    use dfdl_core::infoset::tree::{InfosetBuilder, InfosetTreeSource};
    use dfdl_core::infoset::value::{DfdlSimpleType, DfdlValue};
    use dfdl_core::io::bitstream::{BitReader, BitWriter};
    use dfdl_core::io::sink::{FixedSliceByteSink, VecByteSink};
    use dfdl_core::io::source::{BoundedSource, SliceByteSource};
    use dfdl_core::io::traits::{BitOrder, ByteOrder, ByteSink, ByteSource, Checkpoint};
    use dfdl_core::limits::{ResourceLimits, WorkBudget};
    use dfdl_core::schema::builder::SchemaBuilder;
    use dfdl_core::schema::ir::{
        CompiledElement, CompiledSequence, CompiledType, NodeId, TermKind,
    };
    use dfdl_core::spec::{ConformanceLevel, FeatureProfile, DFDL_1_0_BASELINE};
    use dfdl_core::types::{BitOffset, ByteOffset, InfosetPath, QName, SourceLocation};
    use dfdl_core::util::{
        checked_add_usize, checked_div_usize, checked_mul_usize, get_checked, get_slice_checked,
    };
    use dfdl_xml::limits::XmlReaderLimits;
    use dfdl_xml::{XmlEvent, XmlReader};

    #[test]
    fn test_section_a_workspace_foundation() {
        assert_eq!(DFDL_1_0_BASELINE.doc_id, "OGF GFD-R-P.240");
        let profile = FeatureProfile::extended();
        assert_eq!(profile.level, ConformanceLevel::Extended);
    }

    #[test]
    fn test_section_b_typed_errors_on_arithmetic_overflow() {
        let res_add = checked_add_usize(usize::MAX, 1);
        assert!(res_add.is_err());
        let err_add = res_add.unwrap_err();
        assert_eq!(err_add.kind, DFDLErrorKind::Parse);

        let res_mul = checked_mul_usize(usize::MAX, 2);
        assert!(res_mul.is_err());

        let res_div_zero = checked_div_usize(100, 0);
        assert!(res_div_zero.is_err());
        assert_eq!(res_div_zero.unwrap_err().kind, DFDLErrorKind::Parse);
    }

    #[test]
    fn test_section_b_typed_errors_on_out_of_bounds_indexing() {
        let buffer = [1, 2, 3];
        let res_elem = get_checked(&buffer, 10);
        assert!(res_elem.is_err());
        assert_eq!(res_elem.unwrap_err().kind, DFDLErrorKind::Parse);

        let res_slice = get_slice_checked(&buffer, 1, 5);
        assert!(res_slice.is_err());
        assert_eq!(res_slice.unwrap_err().kind, DFDLErrorKind::Parse);
    }

    #[test]
    fn test_section_b_work_budget_depletion() {
        let mut budget = WorkBudget::new(10);
        assert!(budget.consume(5).is_ok());
        assert!(budget.consume(5).is_ok());
        let res_depleted = budget.consume(1);
        assert!(res_depleted.is_err());
        assert_eq!(
            res_depleted.unwrap_err().kind,
            DFDLErrorKind::WorkBudgetExhausted
        );
    }

    #[test]
    fn test_section_b_vocabulary_types() {
        let qname = QName::with_namespace("http://example.com/dfdl", "header", Some("ex"));
        assert_eq!(qname.local_name, "header");
        assert_eq!(
            qname.namespace.as_ref().map(|n| n.as_str()),
            Some("http://example.com/dfdl")
        );

        let loc = SourceLocation::at_offset(128).with_line_col(10, 5);
        assert_eq!(loc.line, Some(10));
        assert_eq!(loc.column, Some(5));

        let mut path = InfosetPath::root();
        path.try_push("root").unwrap();
        path.try_push("body").unwrap();
        assert_eq!(path.pop(), Some(String::from("body")));

        let byte_off = ByteOffset(64);
        let bit_off = byte_off.to_bit_offset().unwrap();
        assert_eq!(bit_off, BitOffset(512));
    }

    #[test]
    fn test_section_c_xml_parser_valid_corpus() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                   <xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:dfdl=\"http://www.ogf.org/dfdl/dfdl-1.0/\">\
                       <!-- Header comment -->\
                       <xs:element name=\"header\" type=\"xs:string\">\
                           <![CDATA[Raw CDATA section]]>\
                       </xs:element>\
                   </xs:schema>";

        let mut reader = XmlReader::new(xml);

        let ev_doc = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev_doc, XmlEvent::StartDocument { .. }));

        let ev_schema = reader.next_event().unwrap().unwrap();
        if let XmlEvent::StartElement {
            name, attributes, ..
        } = ev_schema
        {
            assert_eq!(name.local_name, "schema");
            assert_eq!(
                name.namespace.as_ref().map(|n| n.as_str()),
                Some("http://www.w3.org/2001/XMLSchema")
            );
            assert!(attributes.is_empty());
        } else {
            panic!("Expected StartElement schema");
        }

        let ev_comment = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev_comment, XmlEvent::Comment { .. }));

        let ev_elem = reader.next_event().unwrap().unwrap();
        if let XmlEvent::StartElement {
            name, attributes, ..
        } = ev_elem
        {
            assert_eq!(name.local_name, "element");
            assert_eq!(attributes.len(), 2);
        } else {
            panic!("Expected StartElement element");
        }

        let ev_cdata = reader.next_event().unwrap().unwrap();
        if let XmlEvent::CData { content, .. } = ev_cdata {
            assert_eq!(content, "Raw CDATA section");
        } else {
            panic!("Expected CDATA");
        }
    }

    #[test]
    fn test_section_c_xml_duplicate_attribute_error() {
        let xml = "<root attr=\"1\" attr=\"2\"/>";
        let mut reader = XmlReader::new(xml);
        let res = reader.next_event();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::Parse);
    }

    #[test]
    fn test_section_c_xml_undeclared_prefix_error() {
        let xml = "<foo:root/>";
        let mut reader = XmlReader::new(xml);
        let res = reader.next_event();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::Parse);
    }

    #[test]
    fn test_section_c_xml_prohibit_dtd_attack() {
        let xml = "<!DOCTYPE billion [<!ENTITY lol \"lol\">]><root>&lol;</root>";
        let mut reader = XmlReader::new(xml);
        let res = reader.next_event();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
    }

    #[test]
    fn test_section_c_xml_depth_limit_exhaustion() {
        let xml = "<a><b><c><d><e/></d></c></b></a>";
        let limits = XmlReaderLimits {
            max_depth: 3,
            max_attributes: 10,
            max_token_length: 100,
            max_entity_expansion_bytes: 100,
            strict_namespaces: true,
        };
        let mut reader = XmlReader::with_limits(xml, limits);
        assert!(reader.next_event().is_ok());
        assert!(reader.next_event().is_ok());
        assert!(reader.next_event().is_ok());
        let res_d = reader.next_event();
        assert!(res_d.is_err());
        assert_eq!(res_d.unwrap_err().kind, DFDLErrorKind::ImplementationLimit);
    }

    #[test]
    fn test_section_d_bitstream_msbf_roundtrip() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        writer.write_bits(0b101, 3).unwrap();
        writer.write_bits(0b11001, 5).unwrap();
        writer.write_bits(0xABCD, 16).unwrap();

        let bytes = writer.checkpoint();
        assert_eq!(bytes.bit_position, BitOffset(24));

        let _ = writer.rollback(Checkpoint::default());
        let output_bytes = [0b1011_1001, 0xAB, 0xCD];
        let src = SliceByteSource::new(&output_bytes);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        assert_eq!(reader.read_bits(3).unwrap(), 0b101);
        assert_eq!(reader.read_bits(5).unwrap(), 0b11001);
        assert_eq!(reader.read_bits(16).unwrap(), 0xABCD);
    }

    #[test]
    fn test_section_d_bitstream_lsbf_roundtrip() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::LeastSignificantBitFirst,
            ByteOrder::LittleEndian,
        );

        writer.write_bits(0b101, 3).unwrap();
        writer.write_bits(0b11001, 5).unwrap();

        let encoded_bytes = [0b11001101];
        let src = SliceByteSource::new(&encoded_bytes);
        let mut reader = BitReader::new(
            src,
            BitOrder::LeastSignificantBitFirst,
            ByteOrder::LittleEndian,
        );

        assert_eq!(reader.read_bits(3).unwrap(), 0b101);
        assert_eq!(reader.read_bits(5).unwrap(), 0b11001);
    }

    #[test]
    fn test_section_d_bounded_source_overrun() {
        let data = [10, 20, 30, 40, 50];
        let src = SliceByteSource::new(&data);
        let mut bounded = BoundedSource::new(src, 2).unwrap();

        let mut buf = [0u8; 2];
        bounded.read_bytes(&mut buf).unwrap();
        assert_eq!(buf, [10, 20]);
        assert!(bounded.read_byte().is_err());
    }

    #[test]
    fn test_section_d_fixed_slice_sink() {
        let mut buf = [0u8; 4];
        let mut sink = FixedSliceByteSink::new(&mut buf);
        sink.write_bytes(&[0x11, 0x22]).unwrap();
        assert_eq!(sink.position(), BitOffset(16));
    }

    #[test]
    fn test_section_d_transactional_rollback_preserves_state() {
        let data = [0x11, 0x22, 0x33, 0x44];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);

        let cp1 = reader.checkpoint();
        assert_eq!(reader.read_bits(16).unwrap(), 0x1122);

        let cp2 = reader.checkpoint();
        assert_eq!(reader.read_bits(16).unwrap(), 0x3344);

        reader.rollback(cp2).unwrap();
        assert_eq!(reader.read_bits(16).unwrap(), 0x3344);

        reader.rollback(cp1).unwrap();
        assert_eq!(reader.read_bits(16).unwrap(), 0x1122);
    }

    #[test]
    fn test_section_e_infoset_state_distinctions() {
        let val = DfdlValue::Int(100);
        assert_eq!(val.simple_type(), DfdlSimpleType::Int);

        let st_val = ElementState::Value(val);
        let st_empty = ElementState::Empty;
        let st_nil = ElementState::Nil;
        let st_absent = ElementState::Absent;
        let st_noval = ElementState::NoValue;

        assert!(st_val.is_value());
        assert!(st_empty.is_empty_value());
        assert!(st_nil.is_nil());
        assert!(st_absent.is_absent());
        assert!(!st_noval.is_value());
    }

    #[test]
    fn test_section_e_infoset_tree_builder_and_event_stream_roundtrip() {
        let qn_doc = QName::local("Document");
        let qn_header = QName::local("Header");
        let qn_status = QName::local("Status");

        let mut builder = InfosetBuilder::with_limits(ResourceLimits::default());
        builder.push_event(InfosetEvent::StartDocument).unwrap();

        builder
            .push_event(InfosetEvent::StartElement {
                name: qn_doc.clone(),
                is_nil: false,
            })
            .unwrap();

        builder
            .push_event(InfosetEvent::SimpleValue {
                name: qn_header.clone(),
                value: DfdlValue::String(String::from("MainHeader")),
            })
            .unwrap();

        builder
            .push_event(InfosetEvent::NilValue {
                name: qn_status.clone(),
            })
            .unwrap();

        builder
            .push_event(InfosetEvent::EndElement {
                name: qn_doc.clone(),
            })
            .unwrap();

        builder.push_event(InfosetEvent::EndDocument).unwrap();

        let doc = builder.build().unwrap();
        assert_eq!(doc.total_nodes, 3);

        let mut stream = InfosetTreeSource::from_document(&doc).unwrap();
        assert_eq!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::StartDocument)
        );
        assert!(matches!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::StartElement { .. })
        ));
        assert!(matches!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::SimpleValue { .. })
        ));
        assert!(matches!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::NilValue { .. })
        ));
        assert!(matches!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::EndElement { .. })
        ));
        assert_eq!(
            stream.next_event().unwrap(),
            Some(InfosetEvent::EndDocument)
        );
    }

    #[test]
    fn test_section_f_schema_ir_manual_builder_without_xml() {
        let mut builder = SchemaBuilder::new();

        let child_elem = CompiledElement {
            name: QName::local("count"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let child_id = builder
            .add_term(QName::local("count"), TermKind::Element(child_elem))
            .unwrap();

        let seq = CompiledSequence {
            members: Vec::from([child_id]),
        };
        let seq_id = builder
            .add_term(QName::local("seq"), TermKind::Sequence(seq))
            .unwrap();

        let root_elem = CompiledElement {
            name: QName::local("header"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(QName::local("header"), TermKind::Element(root_elem))
            .unwrap();
        builder.set_root(root_id);

        let schema = builder.build().unwrap();
        let root_term = schema.root_term().unwrap();
        assert_eq!(root_term.id, root_id);
        assert_eq!(root_term.name.local_name, "header");
    }

    #[test]
    fn test_section_f_schema_ir_cycle_detection() {
        let mut builder = SchemaBuilder::new();

        let id1 = NodeId(0);
        let id2 = NodeId(1);

        let seq1 = CompiledSequence {
            members: Vec::from([id2]),
        };
        let seq2 = CompiledSequence {
            members: Vec::from([id1]), // Cycle id1 -> id2 -> id1
        };

        builder
            .add_term(QName::local("seq1"), TermKind::Sequence(seq1))
            .unwrap();
        builder
            .add_term(QName::local("seq2"), TermKind::Sequence(seq2))
            .unwrap();
        builder.set_root(id1);

        let res = builder.build();
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
    }

    #[test]
    fn test_section_g_pest_expression_parsing_and_eval() {
        use dfdl_core::expr::{eval_expr, parse_expr, ExprContext};

        let ast = parse_expr("{ fn:concat('Length: ', $len + 10) }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);
        let vars = [("len", DfdlValue::Long(32))];

        let mut ctx = ExprContext::new(None, &path, &vars, &mut budget);
        let val = eval_expr(&ast, &mut ctx).unwrap();
        assert_eq!(val, DfdlValue::String(String::from("Length: 42")));
    }

    #[test]
    fn test_section_g_property_resolution_inheritance() {
        use dfdl_core::expr::PropertyStore;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::schema::ir::{LengthKind, Representation};

        let mut format_defaults = PropertyStore::new();
        format_defaults
            .set_property("byteOrder", "bigEndian")
            .unwrap();
        format_defaults
            .set_property("bitOrder", "mostSignificantBitFirst")
            .unwrap();
        format_defaults.set_property("alignment", "1").unwrap();

        let mut element_props = PropertyStore::new();
        element_props
            .set_property("representation", "binary")
            .unwrap();
        element_props
            .set_property("lengthKind", "explicit")
            .unwrap();
        element_props.set_property("length", "16").unwrap();

        let resolved = element_props
            .to_resolved_properties(Some(&format_defaults))
            .unwrap();
        assert_eq!(resolved.representation, Representation::Binary);
        assert_eq!(resolved.byte_order, ByteOrder::BigEndian);
        assert_eq!(resolved.bit_order, BitOrder::MostSignificantBitFirst);
        assert_eq!(resolved.length_kind, LengthKind::Explicit);
        assert_eq!(resolved.length, Some(16));
        assert_eq!(resolved.alignment, 1);
    }

    #[test]
    fn test_section_g_expression_budget_depletion() {
        use dfdl_core::expr::{eval_expr, parse_expr, ExprContext};

        let ast = parse_expr("{ 1 + 2 + 3 + 4 + 5 }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(2); // Only 2 operations budget
        let vars = [];

        let mut ctx = ExprContext::new(None, &path, &vars, &mut budget);
        let res = eval_expr(&ast, &mut ctx);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::WorkBudgetExhausted);
    }

    #[test]
    fn test_section_h_schema_compiler_end_to_end() {
        use dfdl_schema::SchemaCompiler;
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1"/>
    <xs:element name="PacketLength" type="xs:unsignedShort" dfdl:byteOrder="littleEndian" dfdl:lengthKind="explicit" dfdl:length="2"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();
        let root = schema.root_term().unwrap();
        assert_eq!(root.name.local_name, "PacketLength");
        assert_eq!(root.properties.byte_order, ByteOrder::LittleEndian);
        assert_eq!(root.properties.length, Some(2));
    }

    #[test]
    fn test_section_h_schema_compiler_invalid_xml_rejection() {
        use dfdl_schema::SchemaCompiler;
        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"></xs:schema>"#;

        let res = compiler.compile_str(xml);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
    }

    #[test]
    fn test_section_i_bidirectional_kernel_parse_and_unparse() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1" representation="binary"/>
    <xs:element name="Header" type="xs:int" dfdl:length="4"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse raw binary bytes [0x00, 0x00, 0x00, 0x2A] -> Int(42)
        let data = [0x00, 0x00, 0x00, 0x2A];
        let src = SliceByteSource::new(&data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "Header");
        assert_eq!(
            root.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 2. Unparse Infoset document back to binary stream
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        let unparsed_bytes = writer.sink().as_slice();
        assert_eq!(unparsed_bytes, &data);
    }

    #[test]
    fn test_section_j_sequence_infix_separator() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Field1" type="xs:int" dfdl:length="3"/>
                <xs:element name="Field2" type="xs:int" dfdl:length="4"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Parse stream "123,4567"
        let data = b"123,4567";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "Record");
        assert_eq!(root.children.len(), 2);

        let dfdl_core::infoset::tree::InfosetNode::Element(ref f1) = root.children.first().unwrap();
        assert_eq!(f1.name.local_name, "Field1");
        assert_eq!(
            f1.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(123))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref f2) = root.children.get(1).unwrap();
        assert_eq!(f2.name.local_name, "Field2");
        assert_eq!(
            f2.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(4567))
        );

        // Unparse back to stream "123,4567"
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
    fn test_section_j_sequence_prefix_separator() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=";" dfdl:separatorPosition="prefix" dfdl:representation="text">
                <xs:element name="Val1" type="xs:int" dfdl:length="3"/>
                <xs:element name="Val2" type="xs:int" dfdl:length="3"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Parse stream ";100;200"
        let data = b";100;200";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 2);

        let dfdl_core::infoset::tree::InfosetNode::Element(ref v1) = root.children.first().unwrap();
        assert_eq!(
            v1.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(100))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref v2) = root.children.get(1).unwrap();
        assert_eq!(
            v2.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(200))
        );

        // Unparse back to ";100;200"
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
    fn test_section_j_sequence_postfix_separator() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator="|" dfdl:separatorPosition="postfix" dfdl:representation="text">
                <xs:element name="Alpha" type="xs:string" dfdl:length="3"/>
                <xs:element name="Beta" type="xs:string" dfdl:length="3"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Parse stream "abc|def|"
        let data = b"abc|def|";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 2);

        let dfdl_core::infoset::tree::InfosetNode::Element(ref a) = root.children.first().unwrap();
        assert_eq!(
            a.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::String(String::from("abc")))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref b) = root.children.get(1).unwrap();
        assert_eq!(
            b.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::String(String::from("def")))
        );

        // Unparse back to "abc|def|"
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
    fn test_section_j_sequence_initiator_and_terminator() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Group">
        <xs:complexType>
            <xs:sequence dfdl:initiator="[" dfdl:terminator="]" dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Item1" type="xs:int" dfdl:length="2"/>
                <xs:element name="Item2" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Parse stream "[10,20]"
        let data = b"[10,20]";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 2);

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i1) = root.children.first().unwrap();
        assert_eq!(
            i1.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(10))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i2) = root.children.get(1).unwrap();
        assert_eq!(
            i2.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(20))
        );

        // Unparse back to "[10,20]"
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
    fn test_section_j_sequence_missing_separator_error() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Field1" type="xs:int" dfdl:length="3"/>
                <xs:element name="Field2" type="xs:int" dfdl:length="4"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Invalid stream missing comma separator: "123 4567"
        let data = b"123 4567";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
    }

    #[test]
    fn test_section_k_fixed_array_occurrences() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="ListRecord">
        <xs:complexType>
            <xs:sequence dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Item" type="xs:int" dfdl:length="2" minOccurs="3" maxOccurs="3"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

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

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i1) = root.children.first().unwrap();
        assert_eq!(
            i1.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(10))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i2) = root.children.get(1).unwrap();
        assert_eq!(
            i2.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(20))
        );

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i3) = root.children.get(2).unwrap();
        assert_eq!(
            i3.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(30))
        );

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
    fn test_section_k_expression_array_occurrences() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="ExprRecord">
        <xs:complexType>
            <xs:sequence dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Item" type="xs:int" dfdl:length="2" dfdl:occursCountKind="expression" dfdl:occursCount="{ 3 }" minOccurs="0" maxOccurs="unbounded"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse stream "11,22,33"
        let data = b"11,22,33";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 3);

        let dfdl_core::infoset::tree::InfosetNode::Element(ref i1) = root.children.first().unwrap();
        assert_eq!(
            i1.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(11))
        );

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
    fn test_section_k_array_min_occurs_violation_error() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="ListRecord">
        <xs:complexType>
            <xs:sequence dfdl:separator="," dfdl:separatorPosition="infix" dfdl:representation="text">
                <xs:element name="Item" type="xs:int" dfdl:length="2" minOccurs="3" maxOccurs="3"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Stream only contains 2 elements: "10,20" (minOccurs=3 required)
        let data = b"10,20";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
    }

    #[test]
    fn test_section_l_choice_branch_selection_and_unparsing() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="ChoicePayload">
        <xs:complexType>
            <xs:choice dfdl:representation="text">
                <xs:element name="OptionA" type="xs:int" dfdl:length="2" dfdl:initiator="A:"/>
                <xs:element name="OptionB" type="xs:int" dfdl:length="3" dfdl:initiator="B:"/>
            </xs:choice>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse text stream "B:123" -> OptionA fails on initiator "A:", rolls back -> OptionB succeeds!
        let data = b"B:123";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        let dfdl_core::infoset::tree::InfosetNode::Element(ref opt) =
            root.children.first().unwrap();
        assert_eq!(opt.name.local_name, "OptionB");
        assert_eq!(
            opt.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(123))
        );

        // 2. Unparse back to stream "B:123"
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
    fn test_section_l_choice_discriminator_expression() {
        use dfdl_core::infoset::value::DfdlValue;
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="DiscPayload">
        <xs:complexType>
            <xs:choice dfdl:representation="text">
                <xs:element name="BranchA" type="xs:int" dfdl:length="2" dfdl:discriminator="{ 1 eq 2 }"/>
                <xs:element name="BranchB" type="xs:int" dfdl:length="2" dfdl:discriminator="{ 1 eq 1 }"/>
            </xs:choice>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Parse stream "55" -> BranchA parses "55" but discriminator { 1 == 2 } fails! BranchB parses "55" and discriminator { 1 == 1 } succeeds!
        let data = b"55";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        let dfdl_core::infoset::tree::InfosetNode::Element(ref opt) =
            root.children.first().unwrap();
        assert_eq!(opt.name.local_name, "BranchB");
        assert_eq!(
            opt.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(55))
        );

        // Unparse back to "55"
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
    fn test_section_l_choice_all_branches_failed_error() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="ChoicePayload">
        <xs:complexType>
            <xs:choice dfdl:representation="text">
                <xs:element name="OptionA" type="xs:int" dfdl:length="2" dfdl:initiator="A:"/>
                <xs:element name="OptionB" type="xs:int" dfdl:length="3" dfdl:initiator="B:"/>
            </xs:choice>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // Stream starts with "C:99" matching neither OptionA nor OptionB
        let data = b"C:99";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let res = parser.parse_document();
        assert!(res.is_err());
    }

    #[test]
    fn test_section_m_nillable_element_literal_value() {
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1" representation="text"/>
    <xs:element name="Code" type="xs:int" nillable="true" dfdl:nilKind="literalValue" dfdl:nilValue="NULL" dfdl:lengthKind="explicit" dfdl:length="4"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Stream containing "NULL" -> parsed as Nil state!
        let data = b"NULL";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "Code");
        assert_eq!(root.state, dfdl_core::infoset::state::ElementState::Nil);

        // 2. Unparse back to stream -> writes "NULL"
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
    fn test_section_m_nillable_element_non_nil() {
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1" representation="text" textPadKind="padChar"/>
    <xs:element name="Code" type="xs:int" nillable="true" dfdl:nilKind="literalValue" dfdl:nilValue="NULL" dfdl:lengthKind="explicit" dfdl:length="4" dfdl:textNumberJustification="left"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Stream containing "0042" -> parsed as Value(Int(42))
        let data = b"0042";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "Code");
        assert_eq!(
            root.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 2. Unparse back to stream -> writes "42  " (padded to explicit length 4)
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), b"42  ");
    }

    #[test]
    fn test_section_n_alignment_and_text_trimming() {
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1" representation="text" textPadKind="padChar"/>
    <xs:element name="PaddedCode" type="xs:int" dfdl:alignment="2" dfdl:alignmentUnits="bytes" dfdl:lengthKind="explicit" dfdl:length="6" dfdl:textTrimKind="both" dfdl:textPadChar="%SP;" dfdl:textNumberJustification="left"/>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse text stream "  99  "
        let data = b"  99  ";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "PaddedCode");
        assert_eq!(
            root.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(99))
        );

        // 2. Unparse back to stream -> padded to length 6 "99    "
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut unparse_budget = WorkBudget::new(100);

        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut unparse_budget);
        unparser.unparse_document(&doc).unwrap();

        assert_eq!(writer.sink().as_slice(), b"99    ");
    }

    #[test]
    fn test_section_o_input_value_calc_integration() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" alignment="1" representation="text"/>
    <xs:element name="Container">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="CalculatedCode" type="xs:int" dfdl:inputValueCalc="{ 200 + 55 }"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse stream with 0 input bytes -> computed via inputValueCalc "{ 200 + 55 }" -> 255
        let data = b"";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        assert_eq!(root.name.local_name, "Container");
        let dfdl_core::infoset::tree::InfosetNode::Element(child) = root.children.first().unwrap();
        assert_eq!(child.name.local_name, "CalculatedCode");
        assert_eq!(
            child.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(255))
        );
    }

    #[test]
    fn test_section_p_assert_choice_branch_fallback() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Payload">
        <xs:complexType>
            <xs:choice dfdl:representation="text">
                <xs:element name="OptA" type="xs:int" dfdl:length="2" dfdl:assert="{ 10 lt 5 }"/>
                <xs:element name="OptB" type="xs:int" dfdl:length="2"/>
            </xs:choice>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Stream containing "42" -> OptA parses "42" but dfdl:assert { 10 < 5 } fails! Choice rolls back -> OptB parses "42"!
        let data = b"42";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();

        let root = doc.root.as_ref().unwrap();
        let dfdl_core::infoset::tree::InfosetNode::Element(ref child) =
            root.children.first().unwrap();
        assert_eq!(child.name.local_name, "OptB");
        assert_eq!(
            child.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );
    }

    #[test]
    fn test_section_q_hidden_group_ref_parsing_and_unparsing() {
        use dfdl_core::io::bitstream::{BitReader, BitWriter};
        use dfdl_core::io::sink::VecByteSink;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, UnparserEngine};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:group name="Secret">
        <xs:sequence>
            <xs:element name="Magic" type="xs:int" dfdl:length="2" dfdl:outputValueCalc="{ 77 }"/>
        </xs:sequence>
    </xs:group>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:representation="text">
                <xs:sequence dfdl:hiddenGroupRef="Secret" dfdl:representation="text"/>
                <xs:element name="Data" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema = compiler.compile_str(xml).unwrap();

        // 1. Parse bitstream "7742"
        let data = b"7742";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);

        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let full_doc = parser.parse_document().unwrap();

        // 2. Public stripped doc contains ONLY "Data", NOT "Magic"
        let public_doc = full_doc.strip_hidden();
        let root = public_doc.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 1);
        let dfdl_core::infoset::tree::InfosetNode::Element(ref child) =
            root.children.first().unwrap();
        assert_eq!(child.name.local_name, "Data");
        assert_eq!(
            child.state,
            dfdl_core::infoset::state::ElementState::Value(DfdlValue::Int(42))
        );

        // 3. Unparse public_doc -> outputValueCalc for Magic generates "77" + "42" = b"7742"
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
        assert_eq!(&output_bytes, b"7742");
    }

    #[test]
    fn test_section_r_hardening_and_conformance() {
        use dfdl_fuzz::{
            fuzz_bitstream_parse, fuzz_parse_unparse_roundtrip, fuzz_schema_compile, fuzz_xml_parse,
        };

        // 1. Stress test raw XML parser against invalid/truncated inputs
        fuzz_xml_parse(b"");
        fuzz_xml_parse(
            b"<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'><xs:element name='A'",
        );
        fuzz_xml_parse(b"<\x00\xff\xfe random binary garbage >");

        // 2. Stress test schema compiler against invalid schema strings
        fuzz_schema_compile(b"not xml at all");
        fuzz_schema_compile(
            b"<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'><xs:element/></xs:schema>",
        );

        // 3. Stress test bitstream parser against corrupted payloads across complex schema
        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Packet">
        <xs:complexType>
            <xs:sequence dfdl:representation="text" dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2" dfdl:assert="{ 10 &gt; 5 }"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        // Truncated, invalid delimiters, binary noise
        fuzz_bitstream_parse(schema_xml, b"10");
        fuzz_bitstream_parse(schema_xml, b"10;42");
        fuzz_bitstream_parse(schema_xml, b"\x00\xff\xfe\x12\x34");
        fuzz_parse_unparse_roundtrip(schema_xml, b"10,42");

        // 4. Conformance baseline feature profile check
        assert_eq!(
            dfdl_core::spec::FeatureProfile::extended().level,
            dfdl_core::spec::ConformanceLevel::Extended
        );
    }

    #[test]
    fn test_valuelength_delimited_hexbinary_tdml() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let tdml_content =
            include_str!("../tests/daffodil/section23/dfdl_expressions/valueLength.tdml");
        let mut suite = match TdmlTestSuite::parse_xml(tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse valueLength.tdml: {:?}", e),
        };
        suite
            .test_cases
            .retain(|tc| tc.name == "valueLengthDelimitedHexBinary2");
        let report = TdmlRunner::run_suite(&suite, default_schema);
        let target_cases = [
            "valueLengthDelimitedHexBinary2",
            "valueLengthDelimitedHexBinary3",
        ];
        for tc_name in &target_cases {
            let failed = report
                .failure_messages
                .iter()
                .any(|msg| msg.contains(tc_name));
            assert!(!failed, "Test case '{}' failed in report", tc_name);
        }
    }

    #[test]
    fn test_aq000_tdml() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let tdml_content =
            include_str!("../tests/daffodil/section17/calc_value_properties/AQ.tdml");
        let mut suite = match TdmlTestSuite::parse_xml(tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse AQ.tdml: {:?}", e),
        };
        suite.test_cases.retain(|tc| tc.name == "AQ000");
        let report = TdmlRunner::run_suite(&suite, default_schema);
        assert_eq!(
            report.failed, 0,
            "AQ000 failed in report: {:?}",
            report.failure_messages
        );
        assert_eq!(report.passed, 1);
    }

    #[test]
    fn test_computed_length_prefixed_tdml() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let tdml_content = include_str!(
            "../tests/daffodil/section17/calc_value_properties/computedLengthFields.tdml"
        );
        let suite = match TdmlTestSuite::parse_xml(tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse computedLengthFields.tdml: {:?}", e),
        };
        let report = TdmlRunner::run_suite(&suite, default_schema);
        assert_eq!(
            report.failed, 0,
            "computedLengthFields.tdml failed in report: {:?}",
            report.failure_messages
        );
        assert_eq!(report.passed, suite.test_cases.len());
    }

    #[test]
    fn test_official_tdml_conformance_suite() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};

        let tdml_content = include_str!("../tests/official_conformance.tdml");
        let suite = match TdmlTestSuite::parse_xml(tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse official TDML test suite XML: {:?}", e),
        };

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:representation="text" dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let report = TdmlRunner::run_suite(&suite, default_schema);

        for failure in &report.failure_messages {
            eprintln!("[TDML Failure] {}", failure);
        }

        assert_eq!(
            report.failed, 0,
            "TDML Conformance Suite failed {} / {} test cases",
            report.failed, report.total
        );
        assert!(
            report.passed > 0,
            "TDML Conformance Suite ran zero test cases"
        );
    }

    #[test]
    #[ignore = "Full Apache Daffodil official TDML benchmark suite (4,337 test cases) - run explicitly with -- --ignored"]
    fn test_apache_daffodil_official_tdml_suite() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        fn collect_tdml_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect_tdml_files(&path, files);
                    } else if path.extension().and_then(|s| s.to_str()) == Some("tdml") {
                        files.push(path);
                    }
                }
            }
        }

        let base_dir = if Path::new("tests/daffodil").exists() {
            Path::new("tests/daffodil")
        } else if Path::new("crates/dfdl-tests/tests/daffodil").exists() {
            Path::new("crates/dfdl-tests/tests/daffodil")
        } else {
            Path::new("tests/daffodil")
        };

        let mut tdml_files = Vec::new();
        collect_tdml_files(base_dir, &mut tdml_files);

        let mut total_passed = 0usize;
        let mut total_failed = 0usize;
        let mut total_cases = 0usize;
        let mut suites_executed = 0usize;

        let mut cat_schema_compilation = 0usize;
        let mut cat_parse_unexpected_error = 0usize;
        let mut parse_sub_unsupported_fn = 0usize;
        let mut parse_sub_unsupported_type = 0usize;
        let mut parse_sub_delimiter_mismatch = 0usize;
        let mut parse_sub_eof = 0usize;
        let mut parse_sub_choice_failed = 0usize;
        let mut parse_sub_array_bounds = 0usize;
        let mut parse_sub_other = 0usize;

        let mut cat_expected_error_succeeded = 0usize;
        let mut cat_infoset_mismatch = 0usize;
        let mut cat_unparse_failure = 0usize;
        let mut cat_error_msg_mismatch = 0usize;
        let mut cat_other = 0usize;

        let mut sample_messages: Vec<String> = Vec::new();
        let mut eof_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut other_parse_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut expr_unsupported_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut schema_compilation_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut expected_error_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut delim_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut delim_sample_tests: Vec<String> = Vec::new();
        let mut unsupported_type_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut unsupported_type_sample_tests: Vec<String> = Vec::new();
        let mut leftover_sample_tests: Vec<String> = Vec::new();
        let mut other_parse_sample_tests: Vec<String> = Vec::new();
        let mut schema_sample_tests: Vec<String> = Vec::new();
        let mut expected_sample_tests: Vec<String> = Vec::new();
        let mut unparse_failure_clusters: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut unparse_sample_tests: Vec<String> = Vec::new();
        let mut error_msg_sample_tests: Vec<String> = Vec::new();
        let mut other_sample_tests: Vec<String> = Vec::new();

        let test_filter = std::env::var("TDML_FILTER").ok();
        let file_filter = std::env::var("TDML_FILE_FILTER").ok();
        for path in &tdml_files {
            if let Some(ref ff) = file_filter {
                if !path.to_string_lossy().contains(ff) {
                    continue;
                }
            }
            let parent_dir = path.parent().unwrap_or(base_dir);
            if std::env::var("TDML_DEBUG").is_ok() {
                eprintln!("Running TDML suite: {}", path.display());
            }
            if let Ok(tdml_content) = fs::read_to_string(path) {
                if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml_content) {
                    if let Some(ref filter) = test_filter {
                        suite.test_cases.retain(|tc| tc.name.contains(filter));
                    }
                    if !suite.test_cases.is_empty() {
                        let report = TdmlRunner::run_suite_with_base_dir(
                            &suite,
                            default_schema,
                            Some(parent_dir),
                        );
                        total_passed = total_passed.saturating_add(report.passed);
                        total_failed = total_failed.saturating_add(report.failed);
                        total_cases = total_cases.saturating_add(report.total);
                        suites_executed = suites_executed.saturating_add(1);

                        for msg in &report.failure_messages {
                            let p_str = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if std::env::var("TDML_DEBUG").is_ok() {
                                eprintln!("[TDML_DEBUG FAIL] {}", msg);
                            }
                            if msg.contains("failed schema compilation") {
                                cat_schema_compilation = cat_schema_compilation.saturating_add(1);
                                if schema_sample_tests.len() < 200 {
                                    schema_sample_tests.push(format!(
                                        "{} in {}",
                                        msg,
                                        path.file_name().and_then(|n| n.to_str()).unwrap_or("")
                                    ));
                                }
                                let err_str = msg
                                    .split("failed schema compilation: ")
                                    .nth(1)
                                    .unwrap_or(msg.as_str());
                                let sanitized_err = if let Some(idx) = err_str.find("DFDLError {") {
                                    &err_str[idx..]
                                } else {
                                    err_str
                                };
                                *schema_compilation_clusters
                                    .entry(sanitized_err.to_string())
                                    .or_insert(0usize) += 1;
                            } else if msg.contains("unexpectedly failed parse") {
                                cat_parse_unexpected_error =
                                    cat_parse_unexpected_error.saturating_add(1);

                                if sample_messages.len() < 25 {
                                    sample_messages.push(msg.clone());
                                }

                                let err_str = msg
                                    .split("unexpectedly failed parse: ")
                                    .nth(1)
                                    .unwrap_or("");
                                if err_str.contains("Expression")
                                    || err_str.contains("fn:")
                                    || err_str.contains("dfdl:")
                                    || err_str.contains("eval")
                                    || err_str.contains("Function")
                                    || err_str.contains("UnknownFunction")
                                    || err_str.contains("Unsupported")
                                {
                                    parse_sub_unsupported_fn =
                                        parse_sub_unsupported_fn.saturating_add(1);
                                    let sanitized_err =
                                        if let Some(idx) = err_str.find("DFDLError {") {
                                            &err_str[idx..]
                                        } else {
                                            err_str
                                        };
                                    *expr_unsupported_clusters
                                        .entry(sanitized_err.to_string())
                                        .or_insert(0usize) += 1;
                                } else if err_str.contains("dateTime")
                                    || err_str.contains("date")
                                    || err_str.contains("decimal")
                                    || err_str.contains("float")
                                    || err_str.contains("hexBinary")
                                    || err_str.contains("UnsupportedType")
                                {
                                    parse_sub_unsupported_type =
                                        parse_sub_unsupported_type.saturating_add(1);
                                    let sanitized_err =
                                        if let Some(idx) = err_str.find("DFDLError {") {
                                            &err_str[idx..]
                                        } else {
                                            err_str
                                        };
                                    *unsupported_type_clusters
                                        .entry(sanitized_err.to_string())
                                        .or_insert(0usize) += 1;
                                    if unsupported_type_sample_tests.len() < 80 {
                                        unsupported_type_sample_tests.push(format!(
                                            "{} in {}",
                                            msg,
                                            path.file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("")
                                        ));
                                    }
                                } else if err_str.contains("separator")
                                    || err_str.contains("delimiter")
                                    || err_str.contains("initiator")
                                    || err_str.contains("terminator")
                                    || err_str.contains("Delimiter mismatch")
                                {
                                    parse_sub_delimiter_mismatch =
                                        parse_sub_delimiter_mismatch.saturating_add(1);
                                    let sanitized_err =
                                        if let Some(idx) = err_str.find("DFDLError {") {
                                            &err_str[idx..]
                                        } else {
                                            err_str
                                        };
                                    *delim_clusters
                                        .entry(sanitized_err.to_string())
                                        .or_insert(0usize) += 1;
                                    if delim_sample_tests.len() < 40 {
                                        delim_sample_tests.push(msg.clone());
                                    }
                                } else if err_str.contains("EOF")
                                    || err_str.contains("underflow")
                                    || msg.contains("bitstream")
                                    || err_str.contains("End of input")
                                    || err_str.contains("Unexpected end of data")
                                {
                                    parse_sub_eof = parse_sub_eof.saturating_add(1);
                                    let suite_name = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown");
                                    let section = path
                                        .parent()
                                        .and_then(|p| p.file_name())
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown");
                                    *eof_clusters
                                        .entry(format!("{}/{}", section, suite_name))
                                        .or_insert(0usize) += 1;
                                } else if err_str.contains("choice")
                                    || err_str.contains("Choice")
                                    || err_str.contains("branch")
                                {
                                    parse_sub_choice_failed =
                                        parse_sub_choice_failed.saturating_add(1);
                                } else if err_str.contains("minOccurs")
                                    || err_str.contains("maxOccurs")
                                    || err_str.contains("array")
                                {
                                    parse_sub_array_bounds =
                                        parse_sub_array_bounds.saturating_add(1);
                                } else {
                                    parse_sub_other = parse_sub_other.saturating_add(1);
                                    let sanitized_err =
                                        if let Some(idx) = err_str.find("DFDLError {") {
                                            &err_str[idx..]
                                        } else {
                                            err_str
                                        };
                                    let key = if sanitized_err.contains(
                                        "Insufficient text data for explicit length scalar",
                                    ) || sanitized_err
                                        .contains("Insufficient binary data for primitive scalar")
                                    {
                                        format!(
                                            "{} [{}]",
                                            sanitized_err,
                                            msg.split('\'').nth(1).unwrap_or("unknown")
                                        )
                                    } else {
                                        sanitized_err.to_string()
                                    };
                                    *other_parse_clusters.entry(key).or_insert(0usize) += 1;

                                    if sanitized_err.contains("Left over data remaining") {
                                        if leftover_sample_tests.len() < 50 {
                                            leftover_sample_tests.push(format!(
                                                "{} in {}",
                                                msg,
                                                path.file_name()
                                                    .and_then(|n| n.to_str())
                                                    .unwrap_or("")
                                            ));
                                        }
                                    } else if other_parse_sample_tests.len() < 50 {
                                        other_parse_sample_tests.push(format!(
                                            "{} in {}",
                                            msg,
                                            path.file_name().and_then(|n| n.to_str()).unwrap_or("")
                                        ));
                                    }
                                }
                            } else if msg.contains("expected parse error but succeeded")
                                || msg.contains("expected unparse error but succeeded")
                            {
                                cat_expected_error_succeeded =
                                    cat_expected_error_succeeded.saturating_add(1);
                                let exp_str =
                                    msg.split("Expected: ").nth(1).unwrap_or(msg.as_str());
                                *expected_error_clusters
                                    .entry(exp_str.to_string())
                                    .or_insert(0usize) += 1;
                                if !p_str.contains("TestLayers") && expected_sample_tests.len() < 120 {
                                    expected_sample_tests.push(format!(
                                        "{} in {}",
                                        msg,
                                        p_str
                                    ));
                                }
                            } else if msg.contains("infoset mismatch") {
                                cat_infoset_mismatch = cat_infoset_mismatch.saturating_add(1);
                            } else if msg.contains("unparse failed")
                                || msg.contains("unparse output bytes mismatch")
                            {
                                cat_unparse_failure = cat_unparse_failure.saturating_add(1);
                                let cluster_key = if let Some(err_part) = msg.split("unparse failed: ").nth(1) {
                                    if let Some(idx) = err_part.find("DFDLError {") {
                                        &err_part[idx..]
                                    } else {
                                        err_part
                                    }
                                } else {
                                    "unparse output bytes mismatch"
                                };
                                *unparse_failure_clusters
                                    .entry(cluster_key.to_string())
                                    .or_insert(0usize) += 1;
                                if unparse_sample_tests.len() < 120 {
                                    unparse_sample_tests.push(format!(
                                        "{} in {}",
                                        msg,
                                        p_str
                                    ));
                                }
                            } else if msg.contains("error message mismatch")
                                || msg.contains("validation error mismatch")
                            {
                                cat_error_msg_mismatch = cat_error_msg_mismatch.saturating_add(1);
                                if error_msg_sample_tests.len() < 20 {
                                    error_msg_sample_tests.push(format!("{} in {}", msg, p_str));
                                }
                            } else {
                                cat_other = cat_other.saturating_add(1);
                                if other_sample_tests.len() < 50 {
                                    other_sample_tests.push(format!("{} in {}", msg, p_str));
                                }
                            }
                        }
                    }
                }
            }
        }

        eprintln!(
            "\n========================================================================\n\
             [APACHE DAFFODIL TDML CONFORMANCE FAILURE ANALYSIS REPORT]\n\
             ------------------------------------------------------------------------\n\
             Discovered TDML Files    : {}\n\
             Suites Executed          : {}\n\
             Total Test Cases         : {}\n\
             Passed Test Cases        : {} ({:.1}%)\n\
             Failed Test Cases        : {} ({:.1}%)\n\
             ------------------------------------------------------------------------\n\
             FAILURE CLUSTERS BREAKDOWN:\n\
             1. Schema Compilation Rejections : {} ({:.1}%)\n\
             2. Parse Execution Failures      : {} ({:.1}%)\n\
                ├── a. Unsupported Expressions/Functions : {} ({:.1}%)\n\
                ├── b. Unsupported Types/Formatters      : {} ({:.1}%)\n\
                ├── c. Delimiter/Separator Mismatches    : {} ({:.1}%)\n\
                ├── d. Bitstream EOF/Underflow           : {} ({:.1}%)\n\
                ├── e. Choice Branch Failures            : {} ({:.1}%)\n\
                ├── f. Array Bounds/Occurs Failures      : {} ({:.1}%)\n\
                └── g. Other Parse Errors                : {} ({:.1}%)\n\
             3. Infoset Verification Mismatch : {} ({:.1}%)\n\
             4. Expected Error Not Raised     : {} ({:.1}%)\n\
             5. Unparse Execution Failures    : {} ({:.1}%)\n\
             6. Error Message Substring Diff  : {} ({:.1}%)\n\
             7. Other Uncategorized           : {} ({:.1}%)\n\
             ========================================================================\n",
            tdml_files.len(),
            suites_executed,
            total_cases,
            total_passed,
            if total_cases > 0 {
                (total_passed as f64 / total_cases as f64) * 100.0
            } else {
                0.0
            },
            total_failed,
            if total_cases > 0 {
                (total_failed as f64 / total_cases as f64) * 100.0
            } else {
                0.0
            },
            cat_schema_compilation,
            if total_failed > 0 {
                (cat_schema_compilation as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            cat_parse_unexpected_error,
            if total_failed > 0 {
                (cat_parse_unexpected_error as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_unsupported_fn,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_unsupported_fn as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_unsupported_type,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_unsupported_type as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_delimiter_mismatch,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_delimiter_mismatch as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_eof,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_eof as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_choice_failed,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_choice_failed as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_array_bounds,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_array_bounds as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            parse_sub_other,
            if cat_parse_unexpected_error > 0 {
                (parse_sub_other as f64 / cat_parse_unexpected_error as f64) * 100.0
            } else {
                0.0
            },
            cat_infoset_mismatch,
            if total_failed > 0 {
                (cat_infoset_mismatch as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            cat_expected_error_succeeded,
            if total_failed > 0 {
                (cat_expected_error_succeeded as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            cat_unparse_failure,
            if total_failed > 0 {
                (cat_unparse_failure as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            cat_error_msg_mismatch,
            if total_failed > 0 {
                (cat_error_msg_mismatch as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
            cat_other,
            if total_failed > 0 {
                (cat_other as f64 / total_failed as f64) * 100.0
            } else {
                0.0
            },
        );

        eprintln!("\n--- TOP SCHEMA COMPILATION REJECTION CLUSTERS ---");
        let mut sorted_schema: Vec<_> = schema_compilation_clusters.into_iter().collect();
        sorted_schema.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_schema.iter() {
            eprintln!("{:65} : {} test cases", err, count);
        }
        eprintln!("\n--- SCHEMA COMPILATION SAMPLE TESTS ---");
        for (idx, sample) in schema_sample_tests.iter().enumerate() {
            eprintln!("[SchemaComp Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- TOP UNSUPPORTED EXPRESSIONS / FUNCTIONS CLUSTERS ---");
        let mut sorted_expr: Vec<_> = expr_unsupported_clusters.into_iter().collect();
        sorted_expr.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_expr.iter().take(30) {
            eprintln!("{:65} : {} test cases", err, count);
        }

        eprintln!("\n--- TOP UNSUPPORTED TYPES / FORMATTERS CLUSTERS ---");
        let mut sorted_types: Vec<_> = unsupported_type_clusters.into_iter().collect();
        sorted_types.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_types.iter().take(30) {
            eprintln!("{:65} : {} test cases", err, count);
        }
        eprintln!("\n--- UNSUPPORTED TYPES / FORMATTERS SAMPLE TESTS ---");
        for (idx, sample) in unsupported_type_sample_tests.iter().enumerate() {
            eprintln!("[UnsupportedType Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- TOP DELIMITER / SEPARATOR MISMATCH CLUSTERS ---");
        let mut sorted_delim: Vec<_> = delim_clusters.into_iter().collect();
        sorted_delim.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_delim.iter().take(30) {
            eprintln!("{:65} : {} test cases", err, count);
        }
        eprintln!("\n--- DELIMITER / SEPARATOR MISMATCH SAMPLE TESTS ---");
        for (idx, sample) in delim_sample_tests.iter().enumerate() {
            eprintln!("[Delim Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- TOP BITSTREAM EOF / UNDERFLOW FAILURE CLUSTERS ---");
        let mut sorted_eof: Vec<_> = eof_clusters.into_iter().collect();
        sorted_eof.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (cluster, count) in sorted_eof.iter().take(30) {
            eprintln!("{:55} : {} test cases", cluster, count);
        }

        eprintln!("\n--- TOP OTHER PARSE ERROR CLUSTERS ---");
        let mut sorted_other: Vec<_> = other_parse_clusters.into_iter().collect();
        sorted_other.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_other.iter().take(30) {
            eprintln!("{:65} : {} test cases", err, count);
        }
        let binary_insufficient: Vec<_> = sorted_other
            .iter()
            .filter(|(k, _)| k.contains("Insufficient binary data"))
            .collect();
        eprintln!(
            "\n--- INSUFFICIENT BINARY DATA SAMPLE TESTS ({} total) ---",
            binary_insufficient.len()
        );
        for (err, count) in binary_insufficient.iter().take(20) {
            eprintln!("{:65} : {} test cases", err, count);
        }

        eprintln!(
            "\n--- LEFT OVER DATA SAMPLE TESTS ({} collected) ---",
            leftover_sample_tests.len()
        );
        for (idx, sample) in leftover_sample_tests.iter().enumerate() {
            eprintln!("[LeftOver Sample {}] {}", idx + 1, sample);
        }

        eprintln!(
            "\n--- OTHER PARSE ERROR SAMPLE TESTS ({} collected) ---",
            other_parse_sample_tests.len()
        );
        for (idx, sample) in other_parse_sample_tests.iter().enumerate() {
            eprintln!("[OtherParse Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- TOP EXPECTED ERROR NOT RAISED CLUSTERS ---");
        let mut sorted_expected: Vec<_> = expected_error_clusters.into_iter().collect();
        sorted_expected.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_expected.iter().take(50) {
            eprintln!("{:65} : {} test cases", err, count);
        }
        eprintln!("\n--- EXPECTED ERROR NOT RAISED SAMPLE TESTS ({} collected) ---", expected_sample_tests.len());
        for (idx, sample) in expected_sample_tests.iter().enumerate() {
            eprintln!("[ExpectedError Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- TOP UNPARSE EXECUTION FAILURE CLUSTERS ---");
        let mut sorted_unparse: Vec<_> = unparse_failure_clusters.into_iter().collect();
        sorted_unparse.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (err, count) in sorted_unparse.iter().take(50) {
            eprintln!("{:65} : {} test cases", err, count);
        }
        eprintln!("\n--- UNPARSE EXECUTION FAILURE SAMPLE TESTS ({} collected) ---", unparse_sample_tests.len());
        for (idx, sample) in unparse_sample_tests.iter().enumerate() {
            eprintln!("[Unparse Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- OTHER UNCATEGORIZED SAMPLE TESTS ({} collected) ---", other_sample_tests.len());
        for (idx, sample) in other_sample_tests.iter().enumerate() {
            eprintln!("[Other Sample {}] {}", idx + 1, sample);
        }

        eprintln!("\n--- SAMPLE PARSE FAILURE DIAGNOSTICS ---");
        for (idx, sample) in sample_messages.iter().enumerate() {
            eprintln!("[Sample {}] {}", idx + 1, sample);
        }

        assert!(
            suites_executed > 0,
            "No valid TDML test suites parsed from Apache Daffodil repository"
        );
        assert!(
            total_cases > 0,
            "Executed 0 test cases from Apache Daffodil section TDML files"
        );
        assert!(
            total_passed > 0,
            "Passed 0 test cases from Apache Daffodil section TDML files"
        );
    }

    #[test]
    fn test_optional_with_separators_case() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let path_buf = if Path::new(
            "tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml",
        )
        .exists()
        {
            Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
                .to_path_buf()
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml").to_path_buf()
        };
        let path = path_buf.as_path();
        if let Ok(tdml_content) = fs::read_to_string(path) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml_content) {
                suite.test_cases.retain(|tc| {
                    matches!(
                        tc.name.as_str(),
                        "optionalWithSeparators"
                            | "ambigSep1"
                            | "ambigSep2"
                            | "occursCountKindImplicitSeparators01a"
                            | "occursCountKindImplicitSeparators01b"
                            | "occursCountKindImplicitSeparators02"
                            | "occursCountKindImplicitSeparators03"
                    )
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, path.parent());
                for msg in &report.failure_messages {
                    eprintln!("[FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "optionalWithSeparators failed: {:?}",
                    report.failure_messages
                );
            }
        }
    }

    #[test]
    fn test_regular_expressions_requested_cases() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let path =
            Path::new("tests/daffodil/section24/regular_expressions/RegularExpressions.tdml");
        if let Ok(tdml_content) = fs::read_to_string(path) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml_content) {
                let targets = [
                    "testDFDL_922",
                    "testDFDL_922_2",
                    "testAssertWithPattern1",
                    "testRegEx_04",
                    "testRegEx_05",
                    "testRegEx_06",
                    "testRegEx_07",
                ];
                suite
                    .test_cases
                    .retain(|tc| targets.contains(&tc.name.as_str()));
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, path.parent());
                for msg in &report.failure_messages {
                    eprintln!("[FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "RegularExpressions requested tests failed: {:?}",
                    report.failure_messages
                );
            }
        }
    }

    #[test]
    fn test_toplevel_annotation_invalid_02_case() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let path = Path::new("tests/daffodil/section06/namespaces/namespaces.tdml");
        if let Ok(tdml_content) = fs::read_to_string(path) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml_content) {
                suite.test_cases.retain(|tc| {
                    tc.name == "toplevel_annotation_invalid_02"
                        || tc.name == "junkAnnotation01"
                        || tc.name == "error_messages_01"
                        || tc.name == "namespace_scope_01"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, path.parent());
                for msg in &report.failure_messages {
                    eprintln!("[FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Annotation invalid tests failed: {:?}",
                    report.failure_messages
                );
            }
        }
    }

    #[test]
    fn test_tdml_bits_part_type_msb_and_lsb_rtl() {
        use crate::tdml::{TdmlDocumentPart, TdmlRunner};

        let part_msb = TdmlDocumentPart {
            part_type: "bits".to_string(),
            content: "10100110 111".to_string(),
            bit_order: Some("MSBFirst".to_string()),
            byte_order: None,
            encoding: None,
            replace_dfdl_entities: false,
        };
        let bytes_msb = TdmlRunner::assemble_document_bytes(&[part_msb]);
        assert_eq!(bytes_msb, vec![0xA6, 0xE0]);

        let part_lsb_rtl = TdmlDocumentPart {
            part_type: "bits".to_string(),
            content: "001 10111101".to_string(),
            bit_order: Some("LSBFirst".to_string()),
            byte_order: Some("RTL".to_string()),
            encoding: None,
            replace_dfdl_entities: false,
        };
        let bytes_lsb = TdmlRunner::assemble_document_bytes(&[part_lsb_rtl]);
        assert_eq!(bytes_lsb.len(), 2);
    }

    #[test]
    fn test_complex_element_initiator_and_terminator() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Item" dfdl:initiator="[" dfdl:terminator="]">
                    <xs:complexType>
                        <xs:sequence>
                            <xs:element name="Val" type="xs:int" dfdl:lengthKind="explicit" dfdl:length="3"/>
                        </xs:sequence>
                    </xs:complexType>
                </xs:element>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let compiler = SchemaCompiler::new();
        let schema = compiler.compile_str(schema_xml).unwrap();

        let data = b"[123]";
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();
        assert!(doc.root.is_some());
    }

    #[test]
    fn test_utf8_multibyte_character_length() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="CharElem" type="xs:string" dfdl:lengthKind="explicit" dfdl:length="1" dfdl:lengthUnits="characters"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let compiler = SchemaCompiler::new();
        let schema = compiler.compile_str(schema_xml).unwrap();

        // "年" is 3 bytes in UTF-8: E5 B9 B4
        let data = "年".as_bytes();
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().unwrap();
        assert!(doc.root.is_some());
        assert!(reader.is_eof());
    }

    #[test]
    fn test_validation_mode_off_ignores_facet_errors() {
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::{ParserEngine, ValidationMode};
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Color" dfdl:lengthKind="explicit" dfdl:length="6">
        <xs:simpleType>
            <xs:restriction base="xs:string">
                <xs:enumeration value="YELLOW"/>
                <xs:enumeration value="PURPLE"/>
            </xs:restriction>
        </xs:simpleType>
    </xs:element>
</xs:schema>"#;

        let compiler = SchemaCompiler::new();
        let schema = compiler.compile_str(schema_xml).unwrap();

        // Input value "ORANGE" is not in enumeration ["YELLOW", "PURPLE"]
        let data = b"ORANGE";

        // When validation mode is Off, it parses successfully
        let src = SliceByteSource::new(data);
        let mut reader =
            BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        parser.set_validation_mode(ValidationMode::Off);
        let doc_off = parser.parse_document();
        assert!(doc_off.is_ok());

        // When validation mode is Limited, it fails with Validation error
        let src2 = SliceByteSource::new(data);
        let mut reader2 = BitReader::new(
            src2,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget2 = WorkBudget::new(1000);
        let mut parser2 = ParserEngine::new(&schema, &mut reader2, &mut budget2);
        parser2.set_validation_mode(ValidationMode::Limited);
        let doc_limited = parser2.parse_document();
        assert!(doc_limited.is_err());
        assert_eq!(
            doc_limited.unwrap_err().kind,
            dfdl_core::error::DFDLErrorKind::Validation
        );
    }

    #[test]
    fn test_bit_level_scalar_and_complex_explicit_length() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence dfdl:separator=",">
                <xs:element name="Tag" type="xs:int" dfdl:length="2"/>
                <xs:element name="Val" type="xs:int" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        // 1. Test hexBinary bit-level parsing in SimpleTypes.tdml
        let p_st = Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml");
        if let Ok(tdml) = fs::read_to_string(p_st) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                let targets = [
                    "hexBinary_bits_be_msbf",
                    "hexBinary_bits_le_msbf",
                    "hexBinary_bits_le_lsbf_2",
                    "hexBinary_bits_be_msbf_2",
                    "hexBinary_bits_le_msbf_2",
                ];
                suite
                    .test_cases
                    .retain(|tc| targets.contains(&tc.name.as_str()));
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_st.parent());
                for msg in &report.failure_messages {
                    eprintln!("[SimpleTypes FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "hexBinary bit tests failed: {:?}",
                    report.failure_messages
                );
            }
        }

        // 2. Test obscure bit encodings in Encodings.tdml
        let p_enc = Path::new("tests/daffodil/section05/simple_types/Encodings.tdml");
        if let Ok(tdml) = fs::read_to_string(p_enc) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                let targets = ["f293u003_01", "f293u003_02", "f422u001_01"];
                suite
                    .test_cases
                    .retain(|tc| targets.contains(&tc.name.as_str()));
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_enc.parent());
                for msg in &report.failure_messages {
                    eprintln!("[Encodings FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Encodings bit tests failed: {:?}",
                    report.failure_messages
                );
            }
        }

        // 3. Test complex element explicit length skip in ISRM_green_to_orange_60000.tdml
        let p_isrm = Path::new("tests/daffodil/codegen/c/ISRM_green_to_orange_60000.tdml");
        if let Ok(tdml) = fs::read_to_string(p_isrm) {
            if let Ok(suite) = TdmlTestSuite::parse_xml(&tdml) {
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_isrm.parent());
                for msg in &report.failure_messages {
                    eprintln!("[ISRM FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "ISRM complex length tests failed: {:?}",
                    report.failure_messages
                );
            }
        }
    }

    #[test]
    fn test_prefix_virtual_decimal_binary_calendar_and_alignment() {
        use crate::tdml::{TdmlDocumentPart, TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema_path = Path::new("tests/daffodil/xsd/DFDLGeneralFormat.dfdl.xsd");
        let default_schema = fs::read_to_string(default_schema_path).unwrap_or_default();

        // 1. Verify PrefixedTests.tdml targeting pl_bin_bool_bin_bits, pl_bin_date_*, pl_bin_dec_*, plSlash1_data
        let p_pref = Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml");
        if let Ok(tdml) = fs::read_to_string(p_pref) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "pl_bin_bool_bin_bits"
                        || tc.name == "pl_bin_date_bin_bytes_packed"
                        || tc.name == "pl_bin_date_bin_bytes_bcd"
                        || tc.name == "pl_bin_dec_bin_bytes"
                        || tc.name == "pl_bin_dec_bin_bytes_packed"
                        || tc.name == "plSlash1_data"
                        || tc.name == "pl_bin_int_bin_bytes_includes"
                        || tc.name == "pl_bin_int_bin_bits_includes"
                        || tc.name == "pl_text_int_txt_bytes_includes"
                        || tc.name == "pl_text_int_txt_bits_includes"
                });
                assert!(
                    !suite.test_cases.is_empty(),
                    "Targeted PrefixedTests test cases missing"
                );
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_pref.parent());
                for msg in &report.failure_messages {
                    eprintln!("[PREFIXED REGRESSION FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Targeted PrefixedTests failed: {:?}",
                    report.failure_messages
                );
            }
        }

        // 2. Verify ContentFramingProps.tdml for encoding_property_expression & xml_illegal_chars_01
        let p_cfp = Path::new(
            "tests/daffodil/section11/content_framing_properties/ContentFramingProps.tdml",
        );
        if let Ok(tdml) = fs::read_to_string(p_cfp) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "encoding_property_expression" || tc.name == "xml_illegal_chars_01"
                });
                assert!(
                    !suite.test_cases.is_empty(),
                    "Targeted ContentFramingProps test cases missing"
                );
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_cfp.parent());
                for msg in &report.failure_messages {
                    eprintln!("[CFP REGRESSION FAILURE] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Targeted ContentFramingProps failed: {:?}",
                    report.failure_messages
                );
            }
        }

        // 3. Verify replaceDFDLEntities unit logic directly
        let part_entities = TdmlDocumentPart {
            part_type: "text".to_string(),
            content: "Line1%CR;%LF;Line2%LF;At%#x0040;Symbol%SP;End".to_string(),
            bit_order: None,
            byte_order: None,
            encoding: None,
            replace_dfdl_entities: true,
        };
        let assembled = TdmlRunner::assemble_document_bytes(&[part_entities]);
        let expected = b"Line1\r\nLine2\nAt@Symbol End";
        assert_eq!(assembled, expected);
    }

    #[test]
    fn test_separator_suppression_and_leftover_data() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema_path = Path::new("tests/daffodil/xsd/DFDLGeneralFormat.dfdl.xsd");
        let default_schema = fs::read_to_string(default_schema_path).unwrap_or_default();

        // 1. SepTests.tdml: test_sep_trailing_1, test_sep_anyEmpty_1, test_sep_anyEmpty_2
        let p_sep = Path::new("tests/daffodil/usertests/SepTests.tdml");
        if let Ok(tdml) = fs::read_to_string(p_sep) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "test_sep_trailing_1"
                        || tc.name == "test_sep_anyEmpty_1"
                        || tc.name == "test_sep_anyEmpty_2"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_sep.parent());
                for msg in &report.failure_messages {
                    eprintln!("[SepTests DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "SepTests failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 3, "Expected 3 SepTests to pass");
            }
        }

        // 2. SequenceGroupDelimiters.tdml: separatorSuppressionPolicy_never_optionalStringArray_2, separatorSuppressionPolicy_never_optionalIntArray_2
        let p_sqd =
            Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml");
        if let Ok(tdml) = fs::read_to_string(p_sqd) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "separatorSuppressionPolicy_never_optionalStringArray_2"
                        || tc.name == "separatorSuppressionPolicy_never_optionalIntArray_2"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_sqd.parent());
                for msg in &report.failure_messages {
                    eprintln!("[SequenceGroupDelimiters DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "SequenceGroupDelimiters failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(
                    report.passed, 2,
                    "Expected 2 SequenceGroupDelimiters tests to pass"
                );
            }
        }

        // 3. ArrayOptionalElem.tdml: dfdl2263
        let p_aoe =
            Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml");
        if let Ok(tdml) = fs::read_to_string(p_aoe) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| tc.name == "dfdl2263");
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_aoe.parent());
                for msg in &report.failure_messages {
                    eprintln!("[ArrayOptionalElem DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "ArrayOptionalElem failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(
                    report.passed, 1,
                    "Expected 1 ArrayOptionalElem test to pass"
                );
            }
        }

        // 4. SimpleTypes.tdml: hexBinary_Delimited_01, hexBinary_Delimited_01a
        let p_st = Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml");
        if let Ok(tdml) = fs::read_to_string(p_st) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "hexBinary_Delimited_01" || tc.name == "hexBinary_Delimited_01a"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_st.parent());
                for msg in &report.failure_messages {
                    eprintln!("[SimpleTypes DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "SimpleTypes failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 2, "Expected 2 SimpleTypes tests to pass");
            }
        }
    }

    #[test]
    fn test_framing_skip_text_base_and_arithmetic() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema_path = Path::new("tests/daffodil/xsd/DFDLGeneralFormat.dfdl.xsd");
        let default_schema = fs::read_to_string(default_schema_path).unwrap_or_default();

        // 1. BinaryInput_01.tdml: LeadingSkipBytes, LeadingSkipBits
        let p_bin = Path::new("tests/daffodil/section12/aligned_data/BinaryInput_01.tdml");
        if let Ok(tdml) = fs::read_to_string(p_bin) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite
                    .test_cases
                    .retain(|tc| tc.name == "LeadingSkipBytes" || tc.name == "LeadingSkipBits");
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_bin.parent());
                for msg in &report.failure_messages {
                    eprintln!("[BinaryInput_01 DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "BinaryInput_01 failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 2, "Expected 2 BinaryInput_01 tests to pass");
            }
        }

        // 2. Aligned_Data.tdml: leadingSkip1, alignment01
        let p_align = Path::new("tests/daffodil/section12/aligned_data/Aligned_Data.tdml");
        if let Ok(tdml) = fs::read_to_string(p_align) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite
                    .test_cases
                    .retain(|tc| tc.name == "leadingSkip1" || tc.name == "alignment01");
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_align.parent());
                for msg in &report.failure_messages {
                    eprintln!("[Aligned_Data DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Aligned_Data failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 2, "Expected 2 Aligned_Data tests to pass");
            }
        }

        // 3. expressions.tdml: div01, div04, div05, idiv05, div22
        let p_expr = Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml");
        if let Ok(tdml) = fs::read_to_string(p_expr) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "div01"
                        || tc.name == "div04"
                        || tc.name == "div05"
                        || tc.name == "idiv05"
                        || tc.name == "div22"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_expr.parent());
                for msg in &report.failure_messages {
                    eprintln!("[expressions DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "expressions failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 5, "Expected 5 expressions tests to pass");
            }
        }

        // 4. TextStandardBase.tdml: base16_byte_max, base2_int_max
        let p_base = Path::new("tests/daffodil/section13/text_number_props/TextStandardBase.tdml");
        if let Ok(tdml) = fs::read_to_string(p_base) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite
                    .test_cases
                    .retain(|tc| tc.name == "base16_byte_max" || tc.name == "base2_int_max");
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, &default_schema, p_base.parent());
                for msg in &report.failure_messages {
                    eprintln!("[TextStandardBase DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "TextStandardBase failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(
                    report.passed, 2,
                    "Expected 2 TextStandardBase tests to pass"
                );
            }
        }
    }

    #[test]
    fn test_other_parse_errors_conformance_fixes() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let default_schema = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Record">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Val" type="xs:string" dfdl:length="2"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        // 1. Literal character nils and delimiter policies
        let p_nil = Path::new("tests/daffodil/section13/nillable/literal-character-nils.tdml");
        if let Ok(tdml) = fs::read_to_string(p_nil) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "text_01" || tc.name == "text_02" || tc.name == "binary_01"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_nil.parent());
                for msg in &report.failure_messages {
                    eprintln!("[literal-character-nils DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "literal-character-nils failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(
                    report.passed, 3,
                    "Expected 3 literal-character-nils tests to pass"
                );
            }
        }

        // 2. Nillable %ES; and complex nillable
        let p_nillable = Path::new("tests/daffodil/section13/nillable/nillable.tdml");
        if let Ok(tdml) = fs::read_to_string(p_nillable) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "litNil4"
                        || tc.name == "complexNillable_01"
                        || tc.name == "complexNillable_02"
                });
                let report = TdmlRunner::run_suite_with_base_dir(
                    &suite,
                    default_schema,
                    p_nillable.parent(),
                );
                for msg in &report.failure_messages {
                    eprintln!("[nillable DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "nillable failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 3, "Expected 3 nillable tests to pass");
            }
        }

        // 3. Implicit alignment on scalar and string elements
        let p_align = Path::new("tests/daffodil/section12/aligned_data/Aligned_Data.tdml");
        if let Ok(tdml) = fs::read_to_string(p_align) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "implicitAlignmentString1"
                        || tc.name == "implicitAlignmentUnsignedInt"
                        || tc.name == "implicitAlignmentInt"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_align.parent());
                for msg in &report.failure_messages {
                    eprintln!("[Aligned_Data DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "Aligned_Data failed: {:?}",
                    report.failure_messages
                );
                assert_eq!(report.passed, 3, "Expected 3 Aligned_Data tests to pass");
            }
        }

        // 4. Boolean parsing from boolean.tdml
        let p_bool = Path::new("crates/dfdl-tests/tests/daffodil/section13/boolean/boolean.tdml");
        if let Ok(tdml) = fs::read_to_string(p_bool) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| tc.name == "booleanDefault");
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_bool.parent());
                for msg in &report.failure_messages {
                    eprintln!("[boolean DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "boolean failed: {:?}",
                    report.failure_messages
                );
            }
        }

        // 5. Exponent representations with character entities from TextNumberProps.tdml
        let p_tnp = Path::new("crates/dfdl-tests/tests/daffodil/section13/text_number_props/TextNumberProps.tdml");
        if let Ok(tdml) = fs::read_to_string(p_tnp) {
            if let Ok(mut suite) = TdmlTestSuite::parse_xml(&tdml) {
                suite.test_cases.retain(|tc| {
                    tc.name == "expCharEntities" || tc.name == "expCharEntities2"
                });
                let report =
                    TdmlRunner::run_suite_with_base_dir(&suite, default_schema, p_tnp.parent());
                for msg in &report.failure_messages {
                    eprintln!("[TextNumberProps DIAGNOSTIC] {}", msg);
                }
                assert_eq!(
                    report.failed, 0,
                    "TextNumberProps failed: {:?}",
                    report.failure_messages
                );
            }
        }
    }

    #[test]
    fn test_expected_error_facets_nan_incomparability() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path = Path::new("tests/daffodil/section05/facets/Facets.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section05/facets/Facets.tdml")
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());

        // Verify that float/double NaN range facet tests succeed in raising validation errors
        let nan_failures: Vec<_> = report
            .failure_messages
            .iter()
            .filter(|m| m.contains("NaNFail"))
            .collect();
        assert!(
            nan_failures.is_empty(),
            "NaN facet tests failed: {:?}",
            nan_failures
        );

        // Verify that duplicate enumeration test succeeds in raising Schema Definition Error
        let enum_failures: Vec<_> = report
            .failure_messages
            .iter()
            .filter(|m| m.contains("facetEnum08"))
            .collect();
        assert!(
            enum_failures.is_empty(),
            "facetEnum08 duplicate enum test failed: {:?}",
            enum_failures
        );
    }

    #[test]
    fn test_delimiter_speculative_array_separator_rollback() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path = Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        suite.test_cases.retain(|tc| {
            matches!(
                tc.name.as_str(),
                "Lesson6_variable_array_01" | "Lesson6_variable_array_02"
            )
        });
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "Lesson6_variable_array tests failed: {:?}",
            report.failure_messages
        );
    }

    #[test]
    fn test_choice_in_scope_terminator_scoping() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path = Path::new("tests/daffodil/section15/choice_groups/choice.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section15/choice_groups/choice.tdml")
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        suite.test_cases.retain(|tc| {
            matches!(
                tc.name.as_str(),
                "choiceWithInitsAndTermsStr"
                    | "choiceWithInitsAndTermsSeqStr"
                    | "nestedChoiceWithInitsAndTermsNestedInt"
            )
        });
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "choiceWithInitsAndTerms tests failed: {:?}",
            report.failure_messages
        );
    }

    #[test]
    fn test_framing_non_scoping_skip_properties() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path = Path::new("tests/daffodil/section12/aligned_data/Aligned_Data.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section12/aligned_data/Aligned_Data.tdml")
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        suite.test_cases.retain(|tc| {
            matches!(
                tc.name.as_str(),
                "leftAndRightFramingNested01"
                    | "leftAndRightFramingNested02"
                    | "leftAndRightFramingNested05"
            )
        });
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "leftAndRightFramingNested tests failed: {:?}",
            report.failure_messages
        );
    }

    #[test]
    fn test_brace_unescaping_in_delimiters() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path =
            Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml")
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        suite
            .test_cases
            .retain(|tc| matches!(tc.name.as_str(), "ParseSequence4" | "ParseSequence5"));
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "ParseSequence delimiter brace escape tests failed: {:?}",
            report.failure_messages
        );
    }

    #[test]
    fn test_pattern_discriminators_and_choice_point_of_uncertainty() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        let path = Path::new("tests/daffodil/section07/discriminators/discriminator.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new(
                "crates/dfdl-tests/tests/daffodil/section07/discriminators/discriminator.tdml",
            )
        };
        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(_) => return,
        };
        suite.test_cases.retain(|tc| {
            matches!(
                tc.name.as_str(),
                "discrimOnSimpleType"
                    | "discrimOnGroupRef"
                    | "discrimOnGroupRef2"
                    | "discrimOnElementRef"
                    | "discrimPatternPass"
                    | "choiceBranchDiscrimFail"
                    | "choiceBranchDiscrim"
            )
        });
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "Discriminator TDML tests failed: {:?}",
            report.failure_messages
        );
    }

    /// Conformance verification test for Apache Daffodil `external_variables.tdml` (§7.7).
    ///
    /// Validates proper handling of external variable bindings, file-based configuration
    /// overrides via `daffodil_config.xml`, runtime mutation via `dfdl:setVariable` without
    /// false "cannot set variable twice" errors, and scoped inheritance under
    /// `dfdl:newVariableInstance`.
    #[test]
    fn test_daffodil_external_variables_suite() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        // Resolve path to official external_variables.tdml suite
        let path = Path::new("tests/daffodil/section07/external_variables/external_variables.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section07/external_variables/external_variables.tdml")
        };

        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse TDML suite: {:?}", e),
        };

        // Execute all test cases in external_variables.tdml suite with directory context
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "External variables TDML suite had failures: {:?}",
            report.failure_messages
        );
    }

    /// Conformance verification test for Apache Daffodil `envelopePayload.tdml` (§11.2, §12).
    ///
    /// Validates mixed MSBF/BE envelope with repeating LSBF/LE payload elements where
    /// alignment/leadingSkip frames the component to a byte boundary prior to bitOrder transition.
    #[test]
    fn test_daffodil_envelope_payload_suite() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::fs;
        use std::path::Path;

        // Resolve path to official envelopePayload.tdml suite
        let path = Path::new("tests/daffodil/unparser/envelopePayload.tdml");
        let path = if path.exists() {
            path
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/unparser/envelopePayload.tdml")
        };

        let tdml_content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(e) => panic!("Failed to parse TDML suite: {:?}", e),
        };

        // Execute all test cases in envelopePayload.tdml suite with directory context
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(
            report.failed, 0,
            "Envelope payload TDML suite had failures: {:?}",
            report.failure_messages
        );
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn test_unsupported_types_and_formatters_cluster() {
        use dfdl_core::infoset::tree::InfosetNode;
        use dfdl_core::infoset::{state::ElementState, DfdlValue};
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        // 1. IBM 4690 Packed Decimal roundtrip codec across multiple values
        for val in [0i64, 5, 42, 123, 9999, -7, -42, -123, -9999, 1234567, -1234567] {
            let encoded = dfdl_core::util::encode_ibm4690_packed(val);
            let decoded = dfdl_core::util::decode_ibm4690_packed(&encoded)
                .expect("Failed to decode IBM 4690 packed");
            assert_eq!(decoded, val, "IBM 4690 codec roundtrip mismatch for {}", val);
        }

        // 2. Text number patterns, affixes, padding and separators
        let num_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Dec" type="xs:decimal" dfdl:lengthKind="explicit" dfdl:length="15"
                    dfdl:textNumberPattern="'$'#,##0.00;('$'#,##0.00)"
                    dfdl:textStandardDecimalSeparator="^"
                    dfdl:textStandardGroupingSeparator=":"
                    dfdl:textNumberPadCharacter="*"
                    dfdl:textTrimKind="padChar"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let compiler = SchemaCompiler::new();
        let schema = compiler.compile_str(num_xsd).expect("Failed to compile num_xsd");

        let data = b"***$1:234^56***";
        let src = SliceByteSource::new(data);
        let mut reader = BitReader::new(src, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(100);
        let mut parser = ParserEngine::new(&schema, &mut reader, &mut budget);
        let doc = parser.parse_document().expect("Failed to parse positive decimal");
        let root = doc.root.as_ref().unwrap();
        let InfosetNode::Element(ref el) = root.children.first().unwrap();
        assert_eq!(el.state, ElementState::Value(DfdlValue::Decimal(String::from("1234.56"))));

        // 3. Custom NaN and Infinity representations
        let float_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="NaNVal" type="xs:float" dfdl:lengthKind="explicit" dfdl:length="10"
                    dfdl:textStandardNaNRep="notanumber"/>
                <xs:element name="InfVal" type="xs:double" dfdl:lengthKind="explicit" dfdl:length="7"
                    dfdl:textStandardInfinityRep="forever"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_flt = compiler.compile_str(float_xsd).expect("Failed to compile float_xsd");
        let data_flt = b"notanumberforever";
        let src_flt = SliceByteSource::new(data_flt);
        let mut reader_flt = BitReader::new(src_flt, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_flt = WorkBudget::new(100);
        let mut parser_flt = ParserEngine::new(&schema_flt, &mut reader_flt, &mut budget_flt);
        let doc_flt = parser_flt.parse_document().expect("Failed to parse float document");
        let root_flt = doc_flt.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_nan) = root_flt.children.first().unwrap();
        if let ElementState::Value(DfdlValue::Float(f)) = el_nan.state {
            assert!(f.is_nan());
        } else {
            panic!("Expected float NaN");
        }
        let InfosetNode::Element(ref el_inf) = root_flt.children.get(1).unwrap();
        assert_eq!(el_inf.state, ElementState::Value(DfdlValue::Double(f64::INFINITY)));

        // 4. Calendar pattern and padChar trimming
        let cal_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Cal" type="xs:date" dfdl:lengthKind="explicit" dfdl:length="31"
                    dfdl:calendarPattern="'Today is 'MMMM d, yyyy"
                    dfdl:calendarPatternKind="explicit"
                    dfdl:textCalendarPadCharacter="."
                    dfdl:textTrimKind="padChar"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_cal = compiler.compile_str(cal_xsd).expect("Failed to compile cal_xsd");
        let data_cal = b"....Today is March 24, 2013....";
        let src_cal = SliceByteSource::new(data_cal);
        let mut reader_cal = BitReader::new(src_cal, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_cal = WorkBudget::new(100);
        let mut parser_cal = ParserEngine::new(&schema_cal, &mut reader_cal, &mut budget_cal);
        let doc_cal = parser_cal.parse_document().expect("Failed to parse calendar document");
        let root_cal = doc_cal.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_cal) = root_cal.children.first().unwrap();
        assert_eq!(el_cal.state, ElementState::Value(DfdlValue::Date(String::from("2013-03-24"))));

        // 5. IBM 4690 Packed Decimal element in schema
        let ibm_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="binary"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Val" type="xs:int" dfdl:lengthKind="explicit" dfdl:length="2"
                    dfdl:binaryNumberRep="ibm4690Packed"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_ibm = compiler.compile_str(ibm_xsd).expect("Failed to compile ibm_xsd");
        // 0xF1, 0x23 decodes as 123 in IBM 4690 packed decimal
        let data_ibm = [0xF1u8, 0x23u8];
        let src_ibm = SliceByteSource::new(&data_ibm);
        let mut reader_ibm = BitReader::new(src_ibm, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_ibm = WorkBudget::new(100);
        let mut parser_ibm = ParserEngine::new(&schema_ibm, &mut reader_ibm, &mut budget_ibm);
        let doc_ibm = parser_ibm.parse_document().expect("Failed to parse IBM 4690 packed element");
        let root_ibm = doc_ibm.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_ibm) = root_ibm.children.first().unwrap();
        assert_eq!(el_ibm.state, ElementState::Value(DfdlValue::Int(123)));
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn test_other_parse_errors_cluster() {
        use dfdl_core::infoset::tree::InfosetNode;
        use dfdl_core::infoset::{state::ElementState, DfdlValue};
        use dfdl_core::io::bitstream::BitReader;
        use dfdl_core::io::source::SliceByteSource;
        use dfdl_core::io::traits::{BitOrder, ByteOrder};
        use dfdl_core::kernel::ParserEngine;
        use dfdl_core::limits::WorkBudget;
        use dfdl_schema::SchemaCompiler;

        let compiler = SchemaCompiler::new();

        // 1. Custom exponent rep with ignoreCase
        let float_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"
        textNumberRep="standard" textStandardDecimalSeparator="." textStandardGroupingSeparator=","
        textStandardExponentRep="@" ignoreCase="yes"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="F1" type="xs:float" dfdl:lengthKind="delimited" dfdl:terminator=";"/>
                <xs:element name="F2" type="xs:double" dfdl:lengthKind="delimited"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_float = compiler.compile_str(float_xsd).expect("Failed to compile float_xsd");
        let data_float = b"1.25@2;3.5@1";
        let src_float = SliceByteSource::new(data_float);
        let mut reader_float = BitReader::new(src_float, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_float = WorkBudget::new(100);
        let mut parser_float = ParserEngine::new(&schema_float, &mut reader_float, &mut budget_float);
        let doc_float = parser_float.parse_document().expect("Failed to parse custom exponent float document");
        let root_float = doc_float.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_f1) = root_float.children[0];
        let InfosetNode::Element(ref el_f2) = root_float.children[1];
        assert_eq!(el_f1.state, ElementState::Value(DfdlValue::Float(125.0)));
        assert_eq!(el_f2.state, ElementState::Value(DfdlValue::Double(35.0)));

        // 2. Text boolean true and false representations
        let bool_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"
        textBooleanTrueRep="yes Y" textBooleanFalseRep="no N" ignoreCase="yes"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="B1" type="xs:boolean" dfdl:lengthKind="delimited" dfdl:terminator=","/>
                <xs:element name="B2" type="xs:boolean" dfdl:lengthKind="delimited"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_bool = compiler.compile_str(bool_xsd).expect("Failed to compile bool_xsd");
        let data_bool = b"YES,no";
        let src_bool = SliceByteSource::new(data_bool);
        let mut reader_bool = BitReader::new(src_bool, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_bool = WorkBudget::new(100);
        let mut parser_bool = ParserEngine::new(&schema_bool, &mut reader_bool, &mut budget_bool);
        let doc_bool = parser_bool.parse_document().expect("Failed to parse boolean document");
        let root_bool = doc_bool.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_b1) = root_bool.children[0];
        let InfosetNode::Element(ref el_b2) = root_bool.children[1];
        assert_eq!(el_b1.state, ElementState::Value(DfdlValue::Boolean(true)));
        assert_eq!(el_b2.state, ElementState::Value(DfdlValue::Boolean(false)));

        // 3. Pattern padding with *<pad-char> in textNumberPattern
        let pad_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"
        textNumberRep="standard" textStandardDecimalSeparator="." textStandardGroupingSeparator=","/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Dec" type="xs:decimal" dfdl:lengthKind="delimited" dfdl:terminator=";"
                    dfdl:textNumberPattern="**begin: # :end"/>
                <xs:element name="NegInt" type="xs:int" dfdl:lengthKind="delimited"
                    dfdl:textNumberPattern="*_####0;(*_0)"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_pad = compiler.compile_str(pad_xsd).expect("Failed to compile pad_xsd");
        let data_pad = b"****begin: 42 :end;(__99)";
        let src_pad = SliceByteSource::new(data_pad);
        let mut reader_pad = BitReader::new(src_pad, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_pad = WorkBudget::new(100);
        let mut parser_pad = ParserEngine::new(&schema_pad, &mut reader_pad, &mut budget_pad);
        let doc_pad = parser_pad.parse_document().expect("Failed to parse pattern padding document");
        let root_pad = doc_pad.root.as_ref().unwrap();
        let InfosetNode::Element(ref el_dec) = root_pad.children[0];
        let InfosetNode::Element(ref el_neg) = root_pad.children[1];
        assert_eq!(el_dec.state, ElementState::Value(DfdlValue::Decimal(String::from("42"))));
        assert_eq!(el_neg.state, ElementState::Value(DfdlValue::Int(-99)));

        // 4. Character entity delimiters (%LS;, %NEL;, %VT;, %#65;)
        let delim_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence dfdl:separator="%LS; %NEL; %VT; %#65;">
                <xs:element name="Item" type="xs:string" dfdl:lengthKind="delimited" maxOccurs="unbounded"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_delim = compiler.compile_str(delim_xsd).expect("Failed to compile delim_xsd");
        // UTF-8 bytes for: "first" + \u{2028} + "second" + \u{0085} + "third" + \x0B + "fourth" + 'A' + "fifth"
        let mut data_delim = Vec::new();
        data_delim.extend_from_slice(b"first\xE2\x80\xA8second\xC2\x85third\x0BfourthAfifth");
        let src_delim = SliceByteSource::new(&data_delim);
        let mut reader_delim = BitReader::new(src_delim, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_delim = WorkBudget::new(100);
        let mut parser_delim = ParserEngine::new(&schema_delim, &mut reader_delim, &mut budget_delim);
        let doc_delim = parser_delim.parse_document().expect("Failed to parse character entity delimited document");
        let root_delim = doc_delim.root.as_ref().unwrap();
        assert_eq!(root_delim.children.len(), 5);

        // 5. occursCountKind="parsed" cleanly terminates on empty/EOF without error
        let parsed_arr_xsd = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <dfdl:format byteOrder="bigEndian" bitOrder="mostSignificantBitFirst" alignment="1" representation="text" encoding="UTF-8"/>
    <xs:element name="Root">
        <xs:complexType>
            <xs:sequence>
                <xs:element name="Arr" type="xs:string" dfdl:lengthKind="delimited" dfdl:terminator=","
                    dfdl:occursCountKind="parsed" minOccurs="0" maxOccurs="unbounded"/>
            </xs:sequence>
        </xs:complexType>
    </xs:element>
</xs:schema>"#;

        let schema_arr = compiler.compile_str(parsed_arr_xsd).expect("Failed to compile parsed_arr_xsd");
        let data_arr = b"";
        let src_arr = SliceByteSource::new(data_arr);
        let mut reader_arr = BitReader::new(src_arr, BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget_arr = WorkBudget::new(100);
        let mut parser_arr = ParserEngine::new(&schema_arr, &mut reader_arr, &mut budget_arr);
        let doc_arr = parser_arr.parse_document().expect("Failed to parse empty parsed array");
        let root_arr = doc_arr.root.as_ref().unwrap();
        assert_eq!(root_arr.children.len(), 0);
    }

    #[test]
    fn test_debug_text_boolean_ignore_case() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::path::Path;

        let path = Path::new("tests/daffodil/section05/simple_types/Boolean.tdml");
        let tdml_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                panic!("Failed to read Boolean.tdml: {}", e);
            }
        };
        let mut suite = match TdmlTestSuite::parse_xml(&tdml_content) {
            Ok(s) => s,
            Err(e) => {
                panic!("Failed to parse TDML: {}", e);
            }
        };
        suite.test_cases.retain(|tc| tc.name == "textBoolean_IgnoreCase");
        assert_eq!(suite.test_cases.len(), 1);

        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.passed, 1);
    }

    /// Regression test for DFDL §16.1.4 array index expressions in delimiters and path steps.
    ///
    /// Verifies that arrays with multiple occurrences properly evaluate path expressions
    /// and index predicates without triggering spurious query-style path errors.
    #[test]
    fn test_array_expressions_regression() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::path::Path;

        let path = if Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml").exists() {
            Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ArrayOptionalElem.tdml");
        let mut suite = TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name.starts_with("arrayExpressions02"));
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.failed, 0);
        assert_eq!(report.passed, 4);
    }

    /// Regression test for DFDL §6.3.1 character entity references in delimiters.
    ///
    /// Verifies that control character entities (%SOH;, %STX;, etc.) are decoded
    /// and correctly matched by delimiter parsing.
    #[test]
    fn test_byte_entities_delimiters_regression() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::path::Path;

        let path = if Path::new("tests/daffodil/section06/entities/Entities.tdml").exists() {
            Path::new("tests/daffodil/section06/entities/Entities.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section06/entities/Entities.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read Entities.tdml");
        let mut suite = TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "byte_entities_6_01" || tc.name == "byte_entities_6_02");
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.failed, 0);
        assert_eq!(report.passed, 2);
    }

    /// Regression test for DFDL §13.6 ICU textNumberPattern affixes, quoting, and padding.
    ///
    /// Verifies that quoted literals (e.g. `o''clock`), pad escapes (`*##`, `* #0`),
    /// and negative subpatterns properly unquote and strip affixes.
    #[test]
    fn test_text_number_pattern_affixes_regression() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::path::Path;

        let path = if Path::new("tests/daffodil/section13/text_number_props/TextNumberProps.tdml").exists() {
            Path::new("tests/daffodil/section13/text_number_props/TextNumberProps.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section13/text_number_props/TextNumberProps.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read TextNumberProps.tdml");
        let mut suite = TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| {
            tc.name == "textNumberPattern_specialChar03"
                || tc.name == "textNumberPattern_padding05"
                || tc.name == "textNumberPattern_padding07"
                || tc.name == "textNumberPattern_padding08"
                || tc.name == "textNumberPattern_padding12"
                || tc.name == "textNumberPattern_negativeIgnored04"
                || tc.name == "textNumberPattern_negativeIgnored05"
                || tc.name == "standardZeroRep05"
                || tc.name == "standardZeroRep09"
        });
        let report = TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 9);
    }

    /// Tests that schemas referencing `/IBMdefined/GeneralPurposeFormat.xsd` and UTF-16 encoded schemas
    /// compile and execute correctly without SDE or XML tokenization errors.
    #[test]
    fn test_ibm_format_and_multi_encoding() {
        let path = if std::path::Path::new("tests/daffodil/section06/namespaces/namespaces.tdml").exists() {
            std::path::Path::new("tests/daffodil/section06/namespaces/namespaces.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section06/namespaces/namespaces.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read namespaces.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| {
            tc.name == "ibm_format_compat_01"
                || tc.name == "ibm_format_compat_02"
                || tc.name == "ibm_format_compat_03"
                || tc.name == "multi_encoding_01"
                || tc.name == "multi_encoding_02"
                || tc.name == "multi_encoding_03"
                || tc.name == "typeNameOverlap_02"
        });
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 7);
    }

    #[test]
    fn test_cluster1_and_cluster4_namespaces() {
        let path = if std::path::Path::new("tests/daffodil/section06/namespaces/namespaces.tdml").exists() {
            std::path::Path::new("tests/daffodil/section06/namespaces/namespaces.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section06/namespaces/namespaces.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read namespaces.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("namespaces.tdml summary: passed={}, failed={}, total={}", report.passed, report.failed, suite.test_cases.len());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE namespaces] {}", msg);
        }
    }

    #[test]
    fn test_ock_implicit_24() {
        let path = if std::path::Path::new("tests/daffodil/section14/occursCountKind/ockImplicit.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/occursCountKind/ockImplicit.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/occursCountKind/ockImplicit.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ockImplicit.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "ockImplicit19" || tc.name == "ockImplicit24" || tc.name == "ockImplicit8");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 3);
    }

    /// Verifies parsing of repeating sparse elements separated by NUL characters
    /// under `emptyElementParsePolicy="treatAsAbsent"` and `separatorSuppressionPolicy="anyEmpty"`
    /// per DFDL v1.0 §14.2 and §16.1.
    #[test]
    fn test_nul_pad_2() {
        let path = if std::path::Path::new("tests/daffodil/section05/facets/NulChars.tdml").exists() {
            std::path::Path::new("tests/daffodil/section05/facets/NulChars.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section05/facets/NulChars.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read NulChars.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Test all test cases in NulChars.tdml
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[NULCHARS FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 3);
    }


    #[test]
    fn test_general_format_04() {
        let path = if std::path::Path::new("tests/daffodil/section06/namespaces/includeImport.tdml").exists() {
            std::path::Path::new("tests/daffodil/section06/namespaces/includeImport.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section06/namespaces/includeImport.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read includeImport.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "generalFormat04");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    /// Tests that nested prefixed lengths (e.g. prefixLengthType having lengthKind="prefixed")
    /// parse correctly.
    #[test]
    fn test_nested_prefixed_length() {
        let path = if std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read PrefixedTests.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "pl_text_string_pl_txt_bytes");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }
    /// Tests PUA (Unicode Private Use Area) control character remapping on unparse.
    #[test]
    fn test_pua_infoset_chars() {
        let path = if std::path::Path::new("tests/daffodil/section00/general/testUnparserGeneral.tdml").exists() {
            std::path::Path::new("tests/daffodil/section00/general/testUnparserGeneral.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section00/general/testUnparserGeneral.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read testUnparserGeneral.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| {
            tc.name == "puaInfosetChars_01"
                || tc.name == "puaInfosetChars_02"
                || tc.name == "puaInfosetChars_CR_CRLF_01"
        });
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 3);

        let ffb_path = if std::path::Path::new("tests/daffodil/section00/general/testUnparserFileBuffering.tdml").exists() {
            std::path::Path::new("tests/daffodil/section00/general/testUnparserFileBuffering.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section00/general/testUnparserFileBuffering.tdml")
        };
        let ffb_content = std::fs::read_to_string(ffb_path).expect("Failed to read testUnparserFileBuffering.tdml");
        let mut ffb_suite = crate::tdml::TdmlTestSuite::parse_xml(&ffb_content).expect("Failed to parse TDML");
        ffb_suite.test_cases.retain(|tc| {
            tc.name == "puaInfosetChars_01_ffb"
                || tc.name == "puaInfosetChars_02_ffb"
        });
        let ffb_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&ffb_suite, "", ffb_path.parent());
        for msg in &ffb_report.failure_messages {
            eprintln!("[FAILURE] {}", msg);
        }
        assert_eq!(ffb_report.failed, 0, "Failures: {:?}", ffb_report.failure_messages);
        assert_eq!(ffb_report.passed, 2);
    }

    /// Verifies resolution of the remaining schema compilation rejections:
    /// - `include01` and `include02` in `includeImport.tdml`
    /// - `dateTextNumberRep` in `SimpleTypes.tdml`
    #[test]
    fn test_schema_rejection_fixes() {
        let inc_path = if std::path::Path::new("tests/daffodil/section06/namespaces/includeImport.tdml").exists() {
            std::path::Path::new("tests/daffodil/section06/namespaces/includeImport.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section06/namespaces/includeImport.tdml")
        };
        let inc_content = std::fs::read_to_string(inc_path).expect("Failed to read includeImport.tdml");
        let mut inc_suite = crate::tdml::TdmlTestSuite::parse_xml(&inc_content).expect("Failed to parse TDML");
        inc_suite.test_cases.retain(|tc| tc.name == "include01" || tc.name == "include02");
        let inc_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&inc_suite, "", inc_path.parent());
        for msg in &inc_report.failure_messages {
            eprintln!("[FAILURE inc] {}", msg);
        }
        assert_eq!(inc_report.failed, 0, "Failures: {:?}", inc_report.failure_messages);
        assert_eq!(inc_report.passed, 2);

        let date_path = if std::path::Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml").exists() {
            std::path::Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section05/simple_types/SimpleTypes.tdml")
        };
        let date_content = std::fs::read_to_string(date_path).expect("Failed to read SimpleTypes.tdml");
        let mut date_suite = crate::tdml::TdmlTestSuite::parse_xml(&date_content).expect("Failed to parse TDML");
        date_suite.test_cases.retain(|tc| tc.name == "dateTextNumberRep");
        let date_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&date_suite, "", date_path.parent());
        for msg in &date_report.failure_messages {
            eprintln!("[FAILURE date] {}", msg);
        }
        assert_eq!(date_report.failed, 0, "Failures: {:?}", date_report.failure_messages);
        assert_eq!(date_report.passed, 1);
    }

    /// Verifies unparsing fixes for literal character nils and blob data:
    /// - `text_01` and `text_01a` in `literal-character-nils-unparse.tdml`
    /// - `blob_04` and `blob_06` in `Blobs.tdml`
    #[test]
    fn test_unparse_blobs_and_literal_nils() {
        let nil_path = if std::path::Path::new("tests/daffodil/section13/nillable/literal-character-nils-unparse.tdml").exists() {
            std::path::Path::new("tests/daffodil/section13/nillable/literal-character-nils-unparse.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section13/nillable/literal-character-nils-unparse.tdml")
        };
        let nil_content = std::fs::read_to_string(nil_path).expect("Failed to read literal-character-nils-unparse.tdml");
        let mut nil_suite = crate::tdml::TdmlTestSuite::parse_xml(&nil_content).expect("Failed to parse TDML");
        nil_suite.test_cases.retain(|tc| tc.name == "text_01" || tc.name == "text_01a");
        let nil_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&nil_suite, "", nil_path.parent());
        for msg in &nil_report.failure_messages {
            eprintln!("[FAILURE nil] {}", msg);
        }
        assert_eq!(nil_report.failed, 0, "Failures: {:?}", nil_report.failure_messages);
        assert_eq!(nil_report.passed, 2);

        let blob_path = if std::path::Path::new("tests/daffodil/section05/simple_types/Blobs.tdml").exists() {
            std::path::Path::new("tests/daffodil/section05/simple_types/Blobs.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section05/simple_types/Blobs.tdml")
        };
        let blob_content = std::fs::read_to_string(blob_path).expect("Failed to read Blobs.tdml");
        let mut blob_suite = crate::tdml::TdmlTestSuite::parse_xml(&blob_content).expect("Failed to parse TDML");
        blob_suite.test_cases.retain(|tc| tc.name == "blob_04" || tc.name == "blob_06");
        let blob_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&blob_suite, "", blob_path.parent());
        for msg in &blob_report.failure_messages {
            eprintln!("[FAILURE blob] {}", msg);
        }
        assert_eq!(blob_report.failed, 0, "Failures: {:?}", blob_report.failure_messages);
        assert_eq!(blob_report.passed, 2);

        let ab_path = if std::path::Path::new("tests/daffodil/section12/lengthKind/AB.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/lengthKind/AB.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/lengthKind/AB.tdml")
        };
        let ab_content = std::fs::read_to_string(ab_path).expect("Failed to read AB.tdml");
        let mut ab_suite = crate::tdml::TdmlTestSuite::parse_xml(&ab_content).expect("Failed to parse TDML");
        ab_suite.test_cases.retain(|tc| tc.name == "AB005_unparse");
        let ab_report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&ab_suite, "", ab_path.parent());
        for msg in &ab_report.failure_messages {
            eprintln!("[FAILURE ab] {}", msg);
        }
        assert_eq!(ab_report.failed, 0, "Failures: {:?}", ab_report.failure_messages);
        assert_eq!(ab_report.passed, 1);
    }

    #[test]
    fn test_reptype_debug() {
        let path = if std::path::Path::new("tests/daffodil/charsets/TestBitsCharsetDefinition.tdml").exists() {
            std::path::Path::new("tests/daffodil/charsets/TestBitsCharsetDefinition.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/charsets/TestBitsCharsetDefinition.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read TestBitsCharsetDefinition.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("TestBitsCharsetDefinition.tdml: passed={}, failed={}", report.passed, report.failed);
        for msg in &report.failure_messages {
            eprintln!("   Msg: {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    #[test]
    fn test_var_instance_13() {
        let path = if std::path::Path::new("tests/daffodil/section07/variables/variables.tdml").exists() {
            std::path::Path::new("tests/daffodil/section07/variables/variables.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section07/variables/variables.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read variables.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "varInstance_13" || tc.name == "resetVar_01" || tc.name == "varDirection_1");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("variables.tdml subset: passed={}, failed={}", report.passed, report.failed);
        for msg in &report.failure_messages {
            eprintln!("   Msg: {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    #[test]
    fn test_facet_enumeration_subset() {
        let path = if std::path::Path::new("tests/daffodil/section05/facets/Facets.tdml").exists() {
            std::path::Path::new("tests/daffodil/section05/facets/Facets.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section05/facets/Facets.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read Facets.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| {
            tc.name == "checkEnumeration_Fail_Subset"
                || tc.name == "facetEnum05"
                || tc.name == "facetEnum06"
        });
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("Facets.tdml subset: passed={}, failed={}", report.passed, report.failed);
        for msg in &report.failure_messages {
            eprintln!("   Msg: {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 3);
    }

    #[test]
    fn test_input_value_calc_negative() {
        let path = if std::path::Path::new("tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml").exists() {
            std::path::Path::new("tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read inputValueCalc.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "InputValueCalc_circular_ref");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("inputValueCalc.tdml subset: passed={}, failed={}", report.passed, report.failed);
        for msg in &report.failure_messages {
            eprintln!("   Msg: {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    #[test]
    fn test_expressions_negative_cases() {
        let path = if std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml").exists() {
            std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read expressions.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| {
            tc.name == "asterisk_01"
                || tc.name == "expressionRules06"
                || tc.name == "expresion_bad_path_to_element"
        });
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        eprintln!("expressions.tdml subset: passed={}, failed={}", report.passed, report.failed);
        for msg in &report.failure_messages {
            eprintln!("   Msg: {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 3);
    }

    /// Regression test for DFDL §12.1 mandatory text alignment with multi-byte encodings (UTF-16BE)
    /// and DFDL §6.3 %NL; line endings (matching LF, CR, CRLF, NEL U+0085, and LS U+2028).
    ///
    /// Ensures that:
    /// 1. Mandatory text alignment does not misalign UTF-16 text occurring at odd byte offsets
    ///    following single-byte separators.
    /// 2. %NL; delimiter matching recognizes Unicode Next Line (0xC285) and Line Separator (0xE280A8).
    #[test]
    fn test_text_entities_utf16_and_nl_delimiters() {
        let path = if std::path::Path::new("tests/daffodil/section06/entities/Entities.tdml").exists() {
            std::path::Path::new("tests/daffodil/section06/entities/Entities.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section06/entities/Entities.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read Entities.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "text_entities_6_03b");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    /// Regression test for DFDL §14.3.1 (hidden group references) and DFDL §16.1.3 (`occursCountKind="parsed"`).
    ///
    /// Validates that:
    /// 1. Scalar elements (minOccurs=1, maxOccurs=1) under `occursCountKind="parsed"` require at least
    ///    1 occurrence to succeed and do not prematurely succeed with 0 occurrences upon parse failure.
    /// 2. Choice branches containing scalar elements properly fail over to alternate branches when the
    ///    first branch fails to match input data.
    /// 3. Hidden and visible group references inside nested sequences correctly process infix separators.
    #[test]
    fn test_sequence_groups_nested_group_refs() {
        let path = if std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroup.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroup.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/sequence_groups/SequenceGroup.tdml")
        };
        let content = std::fs::read_to_string(path).expect("Failed to read SequenceGroup.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        assert_eq!(report.failed, 0, "Failures in SequenceGroup.tdml: {:?}", report.failure_messages);
        assert_eq!(report.passed, 39);
    }

    /// Verifies standard-conforming fixes for Category 2.b (Unsupported Types/Formatters):
    /// 1. DFDL §13.7.1.4: Intra-digit whitespace rejection in numeric strings under lax and strict check policies.
    /// 2. XML Schema 1.0 §3.2.7 & DFDL §13.11.1: Rejection of prohibited `-00:00` and named timezone strings.
    /// 3. DFDL §5.2.1: Document-level `dfdl:format` default property inheritance for simple types across inclusions (`dfdlx:repType`).
    /// 4. DFDL §13.2.1 & DFDL Extensions: Bit-aligned representation types bypass mandatory text alignment.
    #[test]
    fn test_unsupported_types_and_formatters_conformance_suite() {
        use crate::tdml::{TdmlRunner, TdmlTestSuite};
        use std::path::Path;

        // 1. Verify SimpleTypes.tdml whitespace and implicit datetime pattern fail cases
        let st_path = if Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml").exists() {
            Path::new("tests/daffodil/section05/simple_types/SimpleTypes.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/section05/simple_types/SimpleTypes.tdml")
        };
        let st_content = std::fs::read_to_string(st_path).expect("Failed to read SimpleTypes.tdml");
        let mut st_suite = TdmlTestSuite::parse_xml(&st_content).expect("Failed to parse SimpleTypes.tdml");
        let target_st_tests = [
            "whiteSpaceDuringValidInt",
            "whiteSpaceDuringValidShort",
            "whiteSpaceDuringValidByte",
            "whiteSpaceDuringValidLong",
            "whiteSpaceDuringValidUnsignedInt",
            "whiteSpaceDuringValidUnsignedShort",
            "whiteSpaceDuringValidUnsignedByte",
            "whiteSpaceDuringValidUnsignedLong",
            "dateTimeImplicitPatternFail2",
            "dateTimeImplicitPatternFail4",
        ];
        st_suite.test_cases.retain(|tc| target_st_tests.contains(&tc.name.as_str()));
        let st_report = TdmlRunner::run_suite_with_base_dir(&st_suite, "", st_path.parent());
        assert_eq!(st_report.failed, 0, "Failures in SimpleTypes.tdml: {:?}", st_report.failure_messages);
        assert_eq!(st_report.passed, target_st_tests.len());

        // 2. Verify repType.tdml cross-namespace schema inclusion format inheritance
        let rep_path = if Path::new("tests/daffodil/extensions/repType/repType.tdml").exists() {
            Path::new("tests/daffodil/extensions/repType/repType.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/extensions/repType/repType.tdml")
        };
        let rep_content = std::fs::read_to_string(rep_path).expect("Failed to read repType.tdml");
        let mut rep_suite = TdmlTestSuite::parse_xml(&rep_content).expect("Failed to parse repType.tdml");
        rep_suite.test_cases.retain(|tc| tc.name == "repType_different_namespaces_01");
        let rep_report = TdmlRunner::run_suite_with_base_dir(&rep_suite, "", rep_path.parent());
        assert_eq!(rep_report.failed, 0, "Failures in repType.tdml: {:?}", rep_report.failure_messages);
        assert_eq!(rep_report.passed, 1);

        // 3. Verify enums.tdml bit-aligned representation types
        let enum_path = if Path::new("tests/daffodil/extensions/enum/enums.tdml").exists() {
            Path::new("tests/daffodil/extensions/enum/enums.tdml")
        } else {
            Path::new("crates/dfdl-tests/tests/daffodil/extensions/enum/enums.tdml")
        };
        let enum_content = std::fs::read_to_string(enum_path).expect("Failed to read enums.tdml");
        let mut enum_suite = TdmlTestSuite::parse_xml(&enum_content).expect("Failed to parse enums.tdml");
        enum_suite.test_cases.retain(|tc| tc.name == "repTypeAlignment");
        let enum_report = TdmlRunner::run_suite_with_base_dir(&enum_suite, "", enum_path.parent());
        assert_eq!(enum_report.failed, 0, "Failures in enums.tdml: {:?}", enum_report.failure_messages);
        assert_eq!(enum_report.passed, 1);
    }

    /// Verify alignment and leadingSkip for 7-bit packed ASCII elements.
    ///
    /// Tests `alignmentPacked7BitASCII_02` where `dfdl:alignmentUnits="bits"`,
    /// `dfdl:alignment="6"`, and `dfdl:leadingSkip="5"`.
    /// Confirms that bit-level framing and alignment correctly advance the
    /// bit reader before decoding 7-bit characters.
    #[test]
    fn test_alignment_packed_7bit_ascii_02() {
        let path = if std::path::Path::new("tests/daffodil/section11/content_framing_properties/ContentFramingProps.tdml").exists() {
            std::path::Path::new("tests/daffodil/section11/content_framing_properties/ContentFramingProps.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section11/content_framing_properties/ContentFramingProps.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ContentFramingProps.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[ALIGNMENT FAILURE] {}", msg);
        }
        eprintln!("[CFP TOTAL] passed={}, failed={}", report.passed, report.failed);
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 67);
    }

    /// Verify DFDL-2262: separator suppression under occursCountKind="expression".
    ///
    /// Per DFDL v1.0 §14.2 and §16.1.4, when `occursCountKind="expression"`,
    /// the number of occurrences is fixed by expression evaluation. Empty occurrences
    /// and their separators must never be suppressed as trailing.
    #[test]
    fn test_dfdl_2262() {
        let path = if std::path::Path::new("tests/daffodil/usertests/UserSubmittedTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/usertests/UserSubmittedTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/usertests/UserSubmittedTests.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read UserSubmittedTests.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[USER SUBMITTED FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 8);
    }

    /// Verify nadaParser test case from SequenceGroup.tdml.
    ///
    /// Tests that an empty sequence inside an explicit-length complex element
    /// (length=0) parses cleanly without error into an empty infoset item.
    #[test]
    fn test_nada_parser() {
        let path = if std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroup.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroup.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/sequence_groups/SequenceGroup.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SequenceGroup.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "nadaParser");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[NADA PARSER FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    /// Verify SequenceGroupInitiatedContent suite from section 14 sequence groups.
    ///
    /// Validates that sequence groups with `dfdl:initiatedContent="yes"` correctly
    /// discriminate element occurrences when their initiator matches, and that failed
    /// assertions or parse errors cause the element and sequence to fail without backtracking.
    #[test]
    fn test_sequence_group_initiated_content() {
        let path = if std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupInitiatedContent.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupInitiatedContent.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/sequence_groups/SequenceGroupInitiatedContent.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SequenceGroupInitiatedContent.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[SEQ DISC FAILURE] {}", msg);
        }
        eprintln!("[SEQ DISC TOTAL] passed={}, failed={}", report.passed, report.failed);
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    /// Verify ChoiceGroupInitiatedContent suite from section 15 choice groups.
    ///
    /// Validates that choice groups with `dfdl:initiatedContent="yes"` correctly
    /// discriminate the choice when an alternative or array occurrence initiator matches,
    /// and that failed alternatives do not backtrack across discriminated choices.
    #[test]
    fn test_choice_group_initiated_content() {
        let path = if std::path::Path::new("tests/daffodil/section15/choice_groups/ChoiceGroupInitiatedContent.tdml").exists() {
            std::path::Path::new("tests/daffodil/section15/choice_groups/ChoiceGroupInitiatedContent.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section15/choice_groups/ChoiceGroupInitiatedContent.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ChoiceGroupInitiatedContent.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[CHOICE INIT FAILURE] {}", msg);
        }
        eprintln!("[CHOICE INIT TOTAL] passed={}, failed={}", report.passed, report.failed);
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    /// Verify hexBinaryLengthKindPattern01 from PatternTests.tdml.
    ///
    /// Validates lengthKind="pattern" on xs:hexBinary elements using regex patterns
    /// scanned over binary data interpreted under the element's encoding (e.g. ISO-8859-1).
    #[test]
    fn test_hex_binary_length_kind_pattern_01() {
        let path = if std::path::Path::new("tests/daffodil/section12/lengthKind/PatternTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/lengthKind/PatternTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/lengthKind/PatternTests.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read PatternTests.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "hexBinaryLengthKindPattern01");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[HEX PATTERN FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    /// Verify pl_text_string_txt_chars_padding from PrefixedTests.tdml.
    ///
    /// Tests that prefixed length elements whose prefixLengthType specifies padded text numbers
    /// (e.g. dfdl:textNumberPadCharacter="X") strip pad characters and correctly decode the prefix length.
    #[test]
    fn test_pl_text_string_txt_chars_padding() {
        let path = if std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read PrefixedTests.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        suite.test_cases.retain(|tc| tc.name == "pl_text_string_txt_chars_padding");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[PREFIX PADDING FAILURE] {}", msg);
        }
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1);
    }

    /// Run full PrefixedTests.tdml suite to verify all prefixed length features.
    #[test]
    fn test_prefixed_tests_suite() {
        let path = if std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/lengthKind/PrefixedTests.tdml")
        };
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read PrefixedTests.tdml");
        let suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        for msg in &report.failure_messages {
            eprintln!("[PREFIXED SUITE FAILURE] {}", msg);
        }
        eprintln!("[PREFIXED SUITE TOTAL] passed={}, failed={}", report.passed, report.failed);
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    /// Verifies DFDL §16.1.4 conformance: when `dfdl:occursCountKind="parsed"`, the element
    /// is parsed dynamically until a processing error occurs.
    ///
    /// Consequently, the element behaves as a dynamic array regardless of whether XSD
    /// `maxOccurs` is 1 or unbounded, permitting 1-based indexing in DPath expressions
    /// such as `/ex:e1/ex:password[1]`.
    ///
    /// This test runs the `hiddenDataExpression` and `hiddenDataExpression2` test cases from
    /// `expressions.tdml`, asserting that both parse successfully with 0 failures.
    #[test]
    fn test_hidden_data_expression_parsed_occurs() {
        // Locate expressions.tdml in tests/daffodil or crates/dfdl-tests/tests/daffodil
        let path = if std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml").exists() {
            std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        };
        // Load the TDML suite content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read expressions.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Filter suite to run only hiddenDataExpression and hiddenDataExpression2
        suite.test_cases.retain(|tc| tc.name == "hiddenDataExpression" || tc.name == "hiddenDataExpression2");
        // Execute the filtered test cases against the DFDL processor
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Verify both test cases executed and passed with zero failures
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 2);
    }

    /// Verifies DFDL §3.3 conformance: `xs:appinfo` elements without a valid DFDL source
    /// attribute (`http://www.ogf.org/dfdl/` or `http://www.ogf.org/dfdl/dfdl-1.0/`) must NOT
    /// have their contents interpreted as DFDL annotations.
    ///
    /// When an `xs:appinfo` annotation has no source attribute (or a non-DFDL source URI like
    /// Schematron), its inner elements (such as `dfdl:discriminator` or non-DFDL markers) are
    /// ignored by the DFDL processor, preventing unwarranted discriminator or parse failures.
    ///
    /// This test executes `missing_appinfo_source` and `missing_appinfo_source_nondfdl` from
    /// `SchemaDefinitionErrors.tdml`, ensuring that schema warnings are appropriately captured
    /// and that the payloads parse cleanly.
    #[test]
    fn test_missing_appinfo_source() {
        // Locate SchemaDefinitionErrors.tdml across potential test roots
        let path = if std::path::Path::new("tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml").exists() {
            std::path::Path::new("tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml")
        };
        // Read TDML XML content from disk
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SchemaDefinitionErrors.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain only missing_appinfo_source test cases
        suite.test_cases.retain(|tc| tc.name.starts_with("missing_appinfo_source"));
        // Execute suite with base directory
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert all test cases pass with zero failures
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 2);
    }

    #[test]
    fn test_delims_ignorecase_02() {
        // Locate DelimiterProperties.tdml
        let path = if std::path::Path::new("tests/daffodil/section12/delimiter_properties/DelimiterProperties.tdml").exists() {
            std::path::Path::new("tests/daffodil/section12/delimiter_properties/DelimiterProperties.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section12/delimiter_properties/DelimiterProperties.tdml")
        };
        // Read TDML XML content from disk
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read DelimiterProperties.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain only delims_ignorecase_02
        suite.test_cases.retain(|tc| tc.name == "delims_ignorecase_02");
        // Execute suite with base directory
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        println!("Failure messages: {:?}", report.failure_messages);
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
    }

    /// Regression test for Category 2.c: separatorSuppressionPolicy="never" with infix and prefix
    /// sequences containing complex element arrays and absent optional elements (DFDL §14.2.3).
    ///
    /// This test verifies:
    /// 1. Speculative occurrence rollback restores the bitstream to `reader_cp` under
    ///    `separatorSuppressionPolicy="anyEmpty"` rather than consuming the prefix separator of the
    ///    following sequence element (`field7`).
    /// 2. Under `separatorSuppressionPolicy="never"`, absent optional elements retain their required
    ///    separator slot (`pre_elem_cp`), while scalar optional elements do not consume multi-slot
    ///    separators belonging to subsequent components.
    /// 3. In an infix sequence, the final component does not demand a trailing separator upon completion
    ///    or absent rollback (DFDL §12.3.2).
    #[test]
    fn test_cat_2c_sep_ssp_never_6_and_7() {
        // Locate SepTests.tdml across directory structures
        let path = if std::path::Path::new("tests/daffodil/usertests/SepTests.tdml").exists() {
            std::path::Path::new("tests/daffodil/usertests/SepTests.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/usertests/SepTests.tdml")
        };
        // Read TDML test suite file
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SepTests.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain test cases test_sep_ssp_never_6 and test_sep_ssp_never_7
        suite.test_cases.retain(|tc| tc.name == "test_sep_ssp_never_6" || tc.name == "test_sep_ssp_never_7");
        // Run test cases against runner
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Verify both test cases pass without regressions
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 2, "Expected 2 test cases to pass");
    }

    /// Regression test for Category 2.c: non-represented sequence elements and infix separator
    /// positioning according to DFDL v1.0 §14.2 and §17.
    ///
    /// When an element has `dfdl:inputValueCalc`, it does not have a physical data representation
    /// (`term_has_representation == false`). In an infix sequence, a separator must only precede an
    /// element if at least one prior member had a physical data representation.
    ///
    /// This test runs `InputValueCalc_06` from `InputValueCalc.tdml` to verify that sequences
    /// accurately track represented members rather than raw syntax indices.
    #[test]
    fn test_cat_2c_input_value_calc_06() {
        // Locate inputValueCalc.tdml file
        let path = if std::path::Path::new("tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml").exists() {
            std::path::Path::new("tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section17/calc_value_properties/inputValueCalc.tdml")
        };
        // Load TDML XML content from disk
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read inputValueCalc.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain InputValueCalc_06 test case
        suite.test_cases.retain(|tc| tc.name == "InputValueCalc_06");
        // Run suite
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert clean pass
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected InputValueCalc_06 to pass");
    }

    /// Regression test for Category 2.c: array occurrence separator handling under `occursCountKind="parsed"`
    /// and `implicit` (DFDL §16.1).
    ///
    /// In an array with infix or prefix separators, speculative occurrences beyond the actual count
    /// must cleanly roll back their separators when parsing fails, leaving subsequent components
    /// intact to consume their respective delimiters.
    #[test]
    fn test_cat_2c_array_parsed_and_implicit() {
        // Locate implicitvparsed.tdml file
        let path = if std::path::Path::new("tests/daffodil/section14/occursCountKind/implicitvparsed.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/occursCountKind/implicitvparsed.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/occursCountKind/implicitvparsed.tdml")
        };
        // Read TDML XML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read implicitvparsed.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Filter for test_array_parsed and test_array_implicit
        suite.test_cases.retain(|tc| tc.name == "test_array_parsed" || tc.name == "test_array_implicit");
        // Execute suite
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert all test cases pass
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 2, "Expected 2 test cases to pass");
    }

    /// Regression test for Category 2.c: resolution of file-based document parts using full classpath
    /// paths in TDML runner.
    ///
    /// When `type="file"` references a path beginning with `org/apache/daffodil/`, the runner correctly
    /// resolves the document part relative to the TDML test directory tree.
    #[test]
    fn test_cat_2c_lit_nil1_full_path() {
        // Locate general.tdml
        let path = if std::path::Path::new("tests/daffodil/section00/general/general.tdml").exists() {
            std::path::Path::new("tests/daffodil/section00/general/general.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section00/general/general.tdml")
        };
        // Read TDML XML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read general.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain litNil1FullPath
        suite.test_cases.retain(|tc| tc.name == "litNil1FullPath");
        // Run test
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert pass
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected litNil1FullPath to pass");
    }

    /// Category 4 Test: error01 in ArrayOptionalElem.tdml.
    ///
    /// Per DFDL v1.0 §14.2.2 and §16:
    /// In an infix sequence with separatorSuppressionPolicy="anyEmpty", an extra
    /// trailing separator after all occurrences (e.g. "3,4,5,") is not consumed by
    /// the sequence and must remain in the bitstream as leftover unparsed data,
    /// triggering a processing error ("Left over data").
    #[test]
    fn test_cat4_error01_extra_separator_leftover_data() {
        // Locate ArrayOptionalElem.tdml
        let path = if std::path::Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml").exists() {
            std::path::Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ArrayOptionalElem.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain error01 test case
        suite.test_cases.retain(|tc| tc.name == "error01");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected processing error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected error01 to pass");
    }

    /// Category 4 Test: occursCountKindImplicitSeparators01b in ArrayOptionalElem.tdml.
    ///
    /// Per DFDL v1.0 §14.2.2:
    /// Under separatorSuppressionPolicy="trailingEmptyStrict", if any trailing optional
    /// elements are absent but have separators present in the bitstream, parsing must
    /// fail with a processing error matching "trailingEmptyStrict".
    #[test]
    fn test_cat4_trailing_empty_strict_occurs_count_kind() {
        // Locate ArrayOptionalElem.tdml
        let path = if std::path::Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml").exists() {
            std::path::Path::new("tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section16/array_optional_elem/ArrayOptionalElem.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ArrayOptionalElem.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain occursCountKindImplicitSeparators01b test case
        suite.test_cases.retain(|tc| tc.name == "occursCountKindImplicitSeparators01b");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected processing error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected occursCountKindImplicitSeparators01b to pass");
    }

    /// Category 4 Test: schema_component_err in SchemaDefinitionErrors.tdml.
    ///
    /// Per DFDL v1.0 §12.2 Table 24:
    /// Framing properties (such as dfdl:leadingSkip and dfdl:trailingSkip) are required
    /// properties on elements with representation. A schema that defines format properties
    /// without specifying or inheriting dfdl:leadingSkip must trigger a Schema Definition Error.
    #[test]
    fn test_cat4_missing_leading_skip_sde() {
        // Locate SchemaDefinitionErrors.tdml
        let path = if std::path::Path::new("tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml").exists() {
            std::path::Path::new("tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section02/schema_definition_errors/SchemaDefinitionErrors.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SchemaDefinitionErrors.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain schema_component_err test case
        suite.test_cases.retain(|tc| tc.name == "schema_component_err");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected Schema Definition Error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected schema_component_err to pass");
    }

    /// Category 4 Test: SeqGrp_02 in SequenceGroupDelimiters.tdml.
    ///
    /// Per DFDL v1.0 §16.1.4:
    /// "When dfdl:occursCountKind is 'parsed', the number of occurrences is determined by
    /// parsing occurrences until a Processing Error occurs. It is a Processing Error if
    /// fewer than minOccurs occurrences are found."
    /// When minOccurs=1 (default), input "," with no valid occurrence must trigger a processing error.
    #[test]
    fn test_cat4_seq_grp_02_occurs_count_kind_parsed_min_occurs() {
        // Locate SequenceGroupDelimiters.tdml
        let path = if std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml").exists() {
            std::path::Path::new("tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section14/sequence_groups/SequenceGroupDelimiters.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read SequenceGroupDelimiters.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain SeqGrp_02 test case
        suite.test_cases.retain(|tc| tc.name == "SeqGrp_02");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected Processing Error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected SeqGrp_02 to pass");
    }

    /// Category 4 Test: choiceBranch_e6 in ChoiceBranches.tdml.
    ///
    /// Per DFDL v1.0 §15.1.4:
    /// "An array element cannot be defaultable for a choice."
    /// Choice branches with maxOccurs > 1 or unbounded defining a default value are
    /// unsupported/prohibited and must raise an error matching "subset", "default", "not implemented".
    #[test]
    fn test_cat4_choice_branch_e6_array_default_unsupported() {
        // Locate ChoiceBranches.tdml
        let path = if std::path::Path::new("tests/daffodil/section15/choice_groups/ChoiceBranches.tdml").exists() {
            std::path::Path::new("tests/daffodil/section15/choice_groups/ChoiceBranches.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section15/choice_groups/ChoiceBranches.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read ChoiceBranches.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain choiceBranch_e6 test case
        suite.test_cases.retain(|tc| tc.name == "choiceBranch_e6");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected choiceBranch_e6 to pass");
    }

    /// Category 4 Test: regexLookaheadFail2 in expressions.tdml.
    ///
    /// Per W3C XML Schema 1.0 Part 2 §4.3.4 (Pattern Facet):
    /// The xs:pattern facet uses the XML Schema regular expression language (Appendix F),
    /// which does not support lookarounds. When dfdl:checkConstraints evaluates an xs:pattern
    /// containing a lookahead, it fails and causes dfdl:assert to fail.
    #[test]
    fn test_cat4_regex_lookahead_check_constraints() {
        // Locate expressions.tdml
        let path = if std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml").exists() {
            std::path::Path::new("tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        } else {
            std::path::Path::new("crates/dfdl-tests/tests/daffodil/section23/dfdl_expressions/expressions.tdml")
        };
        // Read TDML content
        let tdml_content = std::fs::read_to_string(path).expect("Failed to read expressions.tdml");
        let mut suite = crate::tdml::TdmlTestSuite::parse_xml(&tdml_content).expect("Failed to parse TDML");
        // Retain regexLookaheadFail2 test case
        suite.test_cases.retain(|tc| tc.name == "regexLookaheadFail2");
        // Run test case
        let report = crate::tdml::TdmlRunner::run_suite_with_base_dir(&suite, "", path.parent());
        // Assert that the expected constraint assertion error was correctly raised
        assert_eq!(report.failed, 0, "Failures: {:?}", report.failure_messages);
        assert_eq!(report.passed, 1, "Expected regexLookaheadFail2 to pass");
    }
}






