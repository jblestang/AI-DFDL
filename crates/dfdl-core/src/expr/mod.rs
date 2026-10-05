//! DFDL Property Resolution and Expression Engine.
//!
//! Follows DFDL 1.0 Specification §6, §7, §8, and §18.
//! Features Pest-based expression parsing, typed AST, static type checking, bounded evaluation,
//! and inheritance-based property resolution.

pub mod ast;
pub mod eval;
pub mod parser;
pub mod properties;
pub mod variables;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};

pub use ast::{BinaryOp, ExprAst, UnaryOp};
pub use eval::{eval_expr, ExprContext, MAX_EVAL_DEPTH};
pub use parser::parse_expr;
pub use properties::{PropertyBinding, PropertyStore};
pub use variables::{DfdlVariable, VariableMap};

/// Validates that all prefixes used in QNames, function calls, variables, and path steps
/// within a DFDL expression are bound in the in-scope XML namespaces (DFDL §23.1).
pub fn validate_expression_namespaces(
    raw_expr: &str,
    in_scope_namespaces: &[(alloc::string::String, alloc::string::String)],
) -> DFDLResult<()> {
    if in_scope_namespaces.is_empty() {
        return Ok(());
    }

    let trimmed = raw_expr.trim();
    let inner_expr = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
        trimmed[1..trimmed.len().saturating_sub(1)].trim()
    } else {
        trimmed
    };
    if inner_expr.is_empty() {
        return Ok(());
    }

    if let Ok(ast) = parse_expr(inner_expr) {
        validate_ast_namespaces(&ast, raw_expr, in_scope_namespaces)?;
    }
    Ok(())
}

fn validate_ast_namespaces(
    ast: &ExprAst,
    raw_expr: &str,
    in_scope_namespaces: &[(alloc::string::String, alloc::string::String)],
) -> DFDLResult<()> {
    match ast {
        ExprAst::Literal(_) => Ok(()),
        ExprAst::Variable(ref qname) => {
            if let Some(ref p) = qname.prefix {
                check_prefix(
                    p,
                    &alloc::format!("${}:{}", p, qname.local_name),
                    raw_expr,
                    in_scope_namespaces,
                )?;
            }
            Ok(())
        }
        ExprAst::FnCall { ref name, ref args } => {
            if name.prefix.as_deref() == Some("fn") && name.local_name == "trace" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Unsupported function: fn:trace",
                ));
            }
            if let Some(ref p) = name.prefix {
                check_prefix(
                    p,
                    &alloc::format!("{}:{}", p, name.local_name),
                    raw_expr,
                    in_scope_namespaces,
                )?;
            }
            for arg in args {
                validate_ast_namespaces(arg, raw_expr, in_scope_namespaces)?;
            }
            Ok(())
        }
        ExprAst::Path(ref path) => {
            for seg in path.segments() {
                let node_part = seg.split('[').next().unwrap_or(seg);
                let unparenthesized = if let Some(inner) = node_part.strip_prefix("..(") {
                    inner.strip_suffix(')').unwrap_or(inner)
                } else if let Some(inner) = node_part.strip_prefix(".(") {
                    inner.strip_suffix(')').unwrap_or(inner)
                } else {
                    node_part
                };
                if let Some((prefix, _local)) = unparenthesized.split_once(':') {
                    if !prefix.is_empty() && prefix != ".." && prefix != "." {
                        check_prefix(prefix, seg, raw_expr, in_scope_namespaces)?;
                    }
                }
                if let (Some(b_open), Some(b_close)) = (seg.find('['), seg.rfind(']')) {
                    if b_open < b_close {
                        let pred_inner = &seg[b_open.saturating_add(1)..b_close];
                        let _ = validate_expression_namespaces(pred_inner, in_scope_namespaces);
                    }
                }
            }
            Ok(())
        }
        ExprAst::IfThenElse {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_ast_namespaces(cond, raw_expr, in_scope_namespaces)?;
            validate_ast_namespaces(then_expr, raw_expr, in_scope_namespaces)?;
            validate_ast_namespaces(else_expr, raw_expr, in_scope_namespaces)?;
            Ok(())
        }
        ExprAst::Unary { expr, .. } => {
            validate_ast_namespaces(expr, raw_expr, in_scope_namespaces)
        }
        ExprAst::Binary { left, right, .. } => {
            validate_ast_namespaces(left, raw_expr, in_scope_namespaces)?;
            validate_ast_namespaces(right, raw_expr, in_scope_namespaces)
        }
    }
}

fn check_prefix(
    prefix: &str,
    target: &str,
    raw_expr: &str,
    in_scope_namespaces: &[(alloc::string::String, alloc::string::String)],
) -> DFDLResult<()> {
    if prefix.is_empty()
        || prefix == "xml"
        || prefix == "xmlns"
        || prefix == "xs"
        || prefix == "xsd"
    {
        return Ok(());
    }
    let is_declared = in_scope_namespaces.iter().any(|(p, _)| p == prefix);
    if !is_declared {
        let msg = alloc::format!(
            "Schema Definition Error: The prefix '{}' for '{}' in expression '{}' has no corresponding namespace declaration.",
            prefix,
            target,
            raw_expr
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use crate::error::DFDLErrorKind;
    use crate::infoset::events::{InfosetEvent, InfosetEventSink};
    use crate::infoset::tree::InfosetBuilder;
    use crate::infoset::value::DfdlValue;
    use crate::limits::WorkBudget;
    use crate::types::{InfosetPath, QName};

    #[test]
    fn test_pest_expression_parsing_literals() {
        let ast = parse_expr("{ 123 + 456 }").unwrap();
        assert_eq!(
            ast,
            ExprAst::Binary {
                op: BinaryOp::Add,
                left: alloc::boxed::Box::new(ExprAst::Literal(DfdlValue::Long(123))),
                right: alloc::boxed::Box::new(ExprAst::Literal(DfdlValue::Long(456))),
            }
        );
    }

    #[test]
    fn test_pest_expression_parsing_relational_and_logic() {
        let ast = parse_expr("{ ($x ge 10) and ($y lt 20) }").unwrap();
        match ast {
            ExprAst::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                assert!(matches!(
                    *left,
                    ExprAst::Binary {
                        op: BinaryOp::Ge,
                        ..
                    }
                ));
                assert!(matches!(
                    *right,
                    ExprAst::Binary {
                        op: BinaryOp::Lt,
                        ..
                    }
                ));
            }
            _ => panic!("Expected And binary ast node"),
        }
    }

    #[test]
    fn test_pest_expression_parsing_function_calls() {
        let ast = parse_expr("{ fn:concat('hello ', $name) }").unwrap();
        match ast {
            ExprAst::FnCall { name, args } => {
                assert_eq!(name.local_name, "concat");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected FnCall AST node"),
        }
    }

    #[test]
    fn test_pest_path_with_predicate() {
        let ast = parse_expr(r#"{ ../../E1Word[1]/contents eq "a" }"#).unwrap();
        match ast {
            ExprAst::Binary { ref left, .. } => {
                if let ExprAst::Path(ref p) = **left {
                    assert_eq!(p.segments(), &["..", "..", "E1Word[1]", "contents"]);
                } else {
                    panic!("Expected Path, got {:?}", left);
                }
            }
            _ => panic!("Expected Binary AST node"),
        }

        let mut builder = InfosetBuilder::new();
        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("msgA"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();

        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("E1Word"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::SimpleValue {
                    name: QName::local("contents"),
                    value: DfdlValue::String(alloc::string::String::from("a")),
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::EndElement {
                    name: QName::local("E1Word"),
                },
                false,
            )
            .unwrap();

        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("E1Word"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::SimpleValue {
                    name: QName::local("contents"),
                    value: DfdlValue::String(alloc::string::String::from("b")),
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::EndElement {
                    name: QName::local("E1Word"),
                },
                false,
            )
            .unwrap();

        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("C3Word"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("A"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();

        let doc = builder.active_doc();
        let curr_path = builder.current_path();
        let mut budget = WorkBudget::new(100);
        let mut ctx = ExprContext::new(Some(&doc), &curr_path, &[], &mut budget);

        let res = eval_expr(&ast, &mut ctx);
        assert_eq!(res.unwrap(), DfdlValue::Boolean(true));
    }

    #[test]
    fn test_expression_evaluation() {
        let ast = parse_expr("{ $a + ($b * 2) }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);
        let vars = [("a", DfdlValue::Long(10)), ("b", DfdlValue::Long(5))];

        let mut ctx = ExprContext::new(None, &path, &vars, &mut budget);
        let res = eval_expr(&ast, &mut ctx).unwrap();
        assert_eq!(res, DfdlValue::Long(20));
    }

    #[test]
    fn test_expression_fn_not() {
        let ast1 = parse_expr("{ fn:not(fn:true()) }").unwrap();
        let ast2 = parse_expr("{ fn:not(1 eq 2) }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        let mut ctx1 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(
            eval_expr(&ast1, &mut ctx1).unwrap(),
            DfdlValue::Boolean(false)
        );

        let mut budget2 = WorkBudget::new(100);
        let mut ctx2 = ExprContext::new(None, &path, &[], &mut budget2);
        assert_eq!(
            eval_expr(&ast2, &mut ctx2).unwrap(),
            DfdlValue::Boolean(true)
        );
    }

    #[test]
    fn test_expression_udfs_and_bitwise_functions() {
        let ast_replace = parse_expr("{ dfdl:replace('Hello World', 'World', 'DFDL') }").unwrap();
        let ast_bitand = parse_expr("{ dfdl:bitAnd(12, 10) }").unwrap();
        let ast_bitor = parse_expr("{ dfdl:bitOr(12, 10) }").unwrap();
        let ast_bitxor = parse_expr("{ dfdl:bitXor(12, 10) }").unwrap();
        let ast_add = parse_expr("{ sgiu:addBoxed(31, 11) }").unwrap();

        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);
        let mut ctx = ExprContext::new(None, &path, &[], &mut budget);

        assert_eq!(
            eval_expr(&ast_replace, &mut ctx).unwrap(),
            DfdlValue::String(alloc::string::String::from("Hello DFDL"))
        );
        assert_eq!(
            eval_expr(&ast_bitand, &mut ctx).unwrap(),
            DfdlValue::Long(8)
        );
        assert_eq!(
            eval_expr(&ast_bitor, &mut ctx).unwrap(),
            DfdlValue::Long(14)
        );
        assert_eq!(
            eval_expr(&ast_bitxor, &mut ctx).unwrap(),
            DfdlValue::Long(6)
        );
        assert_eq!(eval_expr(&ast_add, &mut ctx).unwrap(), DfdlValue::Int(42));
    }

    #[test]
    fn test_expression_evaluation_with_infoset_doc() {
        use crate::limits::ResourceLimits;
        let mut builder = InfosetBuilder::with_limits(ResourceLimits::default());
        builder.push_event(InfosetEvent::StartDocument).unwrap();
        builder
            .push_event(InfosetEvent::StartElement {
                name: QName::local("root"),
                is_nil: false,
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::SimpleValue {
                name: QName::local("len"),
                value: DfdlValue::Long(42),
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::EndElement {
                name: QName::local("root"),
            })
            .unwrap();
        builder.push_event(InfosetEvent::EndDocument).unwrap();
        let doc = builder.build().unwrap();

        let ast = parse_expr("{ /root/len * 2 }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);
        let vars = [];

        let mut ctx = ExprContext::new(Some(&doc), &path, &vars, &mut budget);
        let res = eval_expr(&ast, &mut ctx).unwrap();
        assert_eq!(res, DfdlValue::Long(84));
    }

    #[test]
    fn test_property_store_inheritance() {
        let mut parent = PropertyStore::new();
        parent.set_property("representation", "text").unwrap();
        parent.set_property("byteOrder", "bigEndian").unwrap();
        parent.set_property("alignment", "4").unwrap();

        let mut child = PropertyStore::new();
        child.set_property("alignment", "8").unwrap();

        assert_eq!(
            child.resolve_property(Some(&parent), "byteOrder", "littleEndian"),
            "bigEndian"
        );
        assert_eq!(child.resolve_property(Some(&parent), "alignment", "1"), "8");

        let resolved = child.to_resolved_properties(Some(&parent)).unwrap();
        assert_eq!(resolved.alignment, 8);
        assert_eq!(resolved.byte_order, crate::io::traits::ByteOrder::BigEndian);
    }

    #[test]
    fn test_expression_if_then_else_and_idiv() {
        let ast = parse_expr("{ if ($x gt 10) then ($x idiv 2) else ($x + 1) }").unwrap();
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        let vars_high = [("x", DfdlValue::Long(25))];
        let mut ctx_high = ExprContext::new(None, &path, &vars_high, &mut budget);
        assert_eq!(eval_expr(&ast, &mut ctx_high).unwrap(), DfdlValue::Long(12));

        let mut budget2 = WorkBudget::new(100);
        let vars_low = [("x", DfdlValue::Long(4))];
        let mut ctx_low = ExprContext::new(None, &path, &vars_low, &mut budget2);
        assert_eq!(eval_expr(&ast, &mut ctx_low).unwrap(), DfdlValue::Long(5));
    }

    #[test]
    fn test_xpath_numeric_coercion_and_string_operands() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        // DFDL §23 / XPath 2.0: arithmetic operators require numeric operands; strings are not implicitly coerced.
        let ast1 = parse_expr("{ $a * $b }").unwrap();
        let vars1 = [
            ("a", DfdlValue::String(alloc::string::String::from("7"))),
            ("b", DfdlValue::String(alloc::string::String::from("7"))),
        ];
        let mut ctx1 = ExprContext::new(None, &path, &vars1, &mut budget);
        let err1 = eval_expr(&ast1, &mut ctx1).unwrap_err();
        assert!(err1.message.to_string().contains("requires numeric operands"));

        // Mixed Int and String addition must also be rejected
        let ast2 = parse_expr("{ $x + 5 }").unwrap();
        let vars2 = [("x", DfdlValue::String(alloc::string::String::from("10")))];
        let mut ctx2 = ExprContext::new(None, &path, &vars2, &mut budget);
        let err2 = eval_expr(&ast2, &mut ctx2).unwrap_err();
        assert!(err2.message.to_string().contains("requires numeric operands"));
    }

    #[test]
    fn test_dfdl_length_functions() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        // dfdl:valueLength('hello', 'bytes') -> 5
        let ast1 = parse_expr("{ dfdl:valueLength('hello', 'bytes') }").unwrap();
        let mut ctx1 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(eval_expr(&ast1, &mut ctx1).unwrap(), DfdlValue::Long(5));

        // dfdl:valueLength('hello', 'bits') -> 40
        let ast2 = parse_expr("{ dfdl:valueLength('hello', 'bits') }").unwrap();
        let mut ctx2 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(eval_expr(&ast2, &mut ctx2).unwrap(), DfdlValue::Long(40));

        // dfdl:contentLength('abc') -> default bytes -> 3
        let ast3 = parse_expr("{ dfdl:contentLength('abc') }").unwrap();
        let mut ctx3 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(eval_expr(&ast3, &mut ctx3).unwrap(), DfdlValue::Long(3));
    }

    #[test]
    fn test_relative_path_navigation_nested() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        let mut root = InfosetElement::complex(QName::local("e2"));
        let mut header = InfosetElement::complex(QName::local("header"));
        let msg_size = InfosetElement::simple(
            QName::local("message_size"),
            ElementState::Value(DfdlValue::UnsignedInt(7)),
        );
        let _ = header.children.try_reserve(1);
        header.children.push(InfosetNode::Element(msg_size));
        let _ = root.children.try_reserve(1);
        root.children.push(InfosetNode::Element(header));

        let doc = InfosetDocument {
            root: Some(root),
            total_nodes: 3,
        };

        let mut current_path = InfosetPath::root();
        let _ = current_path.try_push("e2");
        let _ = current_path.try_push("message");

        let mut budget = WorkBudget::new(100);
        let ast = parse_expr("{ ../header/message_size - 4 }").unwrap();
        let mut ctx = ExprContext::new(Some(&doc), &current_path, &[], &mut budget);

        assert_eq!(eval_expr(&ast, &mut ctx).unwrap(), DfdlValue::Long(3));
    }

    #[test]
    fn test_xpath_string_functions_and_edge_cases() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        // Standard 1-based substring
        let ast1 = parse_expr("{ fn:substring('hello world', 1, 5) }").unwrap();
        let mut ctx1 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(
            eval_expr(&ast1, &mut ctx1).unwrap(),
            DfdlValue::String(alloc::string::String::from("hello"))
        );

        // Missing length parameter -> remainder of string
        let ast2 = parse_expr("{ fn:substring('hello world', 7) }").unwrap();
        let mut ctx2 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(
            eval_expr(&ast2, &mut ctx2).unwrap(),
            DfdlValue::String(alloc::string::String::from("world"))
        );

        // Out-of-bounds start index -> empty string
        let ast3 = parse_expr("{ fn:substring('hello world', 50, 5) }").unwrap();
        let mut ctx3 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(
            eval_expr(&ast3, &mut ctx3).unwrap(),
            DfdlValue::String(alloc::string::String::new())
        );

        // Length exceeding available string bounds -> truncated to available length
        let ast4 = parse_expr("{ fn:substring('hello world', 1, 100) }").unwrap();
        let mut ctx4 = ExprContext::new(None, &path, &[], &mut budget);
        assert_eq!(
            eval_expr(&ast4, &mut ctx4).unwrap(),
            DfdlValue::String(alloc::string::String::from("hello world"))
        );

        // contains, starts-with, ends-with, upper-case, lower-case, matches
        let ast5 = parse_expr("{ fn:contains('hello world', 'world') }").unwrap();
        assert_eq!(
            eval_expr(&ast5, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );

        let ast6 = parse_expr("{ fn:starts-with('hello world', 'hello') }").unwrap();
        assert_eq!(
            eval_expr(&ast6, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );

        let ast7 = parse_expr("{ fn:ends-with('hello world', 'world') }").unwrap();
        assert_eq!(
            eval_expr(&ast7, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );

        let ast8 = parse_expr("{ fn:upper-case('hello') }").unwrap();
        assert_eq!(
            eval_expr(&ast8, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::String(alloc::string::String::from("HELLO"))
        );

        let ast9 = parse_expr("{ fn:lower-case('WORLD') }").unwrap();
        assert_eq!(
            eval_expr(&ast9, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::String(alloc::string::String::from("world"))
        );

        let ast10 = parse_expr("{ fn:matches('foo123bar', '123') }").unwrap();
        assert_eq!(
            eval_expr(&ast10, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );
    }

    #[test]
    fn test_xpath_numeric_functions_and_edge_cases() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        // fn:abs
        let ast_abs = parse_expr("{ fn:abs(-42) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_abs,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Long(42)
        );

        // fn:ceiling
        let ast_ceil = parse_expr("{ fn:ceiling(12.34) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_ceil,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Double(13.0)
        );

        // fn:floor
        let ast_floor = parse_expr("{ fn:floor(12.78) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_floor,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Double(12.0)
        );

        // fn:round
        let ast_round = parse_expr("{ fn:round(12.6) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_round,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Double(13.0)
        );
    }

    #[test]
    fn test_dfdl_variables_and_expression_evaluation() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        let mut vmap = VariableMap::new();
        vmap.define_variable(
            QName::local("intVar"),
            crate::infoset::value::DfdlSimpleType::Int,
            Some(DfdlValue::Int(10)),
        );

        let ast = parse_expr("{ $intVar + 5 }").unwrap();

        // 1. Evaluate with default value (10 + 5 = 15)
        let mut ctx1 = ExprContext::with_variable_map(None, &path, &[], Some(&vmap), &mut budget);
        assert_eq!(eval_expr(&ast, &mut ctx1).unwrap(), DfdlValue::Long(15));

        // 2. Push a new variable instance with value 100 via newVariableInstance (§7.7) and re-evaluate (100 + 5 = 105)
        vmap.new_variable_instance(&QName::local("intVar"), Some(DfdlValue::Int(100)))
            .unwrap();
        let mut ctx2 = ExprContext::with_variable_map(None, &path, &[], Some(&vmap), &mut budget);
        assert_eq!(eval_expr(&ast, &mut ctx2).unwrap(), DfdlValue::Long(105));
    }

    #[test]
    fn test_layer_variable_uninitialized_defaults() {
        let mut vmap = VariableMap::new();
        vmap.define_variable(
            QName::local("uninitByte"),
            crate::infoset::value::DfdlSimpleType::Byte,
            None,
        );
        vmap.define_variable(
            QName::local("uninitBool"),
            crate::infoset::value::DfdlSimpleType::Boolean,
            None,
        );
        vmap.define_variable(
            QName::local("uninitString"),
            crate::infoset::value::DfdlSimpleType::String,
            None,
        );

        assert_eq!(vmap.get_variable("$uninitByte"), Some(&DfdlValue::Byte(0)));
        assert_eq!(
            vmap.get_variable("$uninitBool"),
            Some(&DfdlValue::Boolean(false))
        );
        assert_eq!(
            vmap.get_variable("$uninitString"),
            Some(&DfdlValue::String(alloc::string::String::new()))
        );
    }

    #[test]
    fn test_in_progress_infoset_active_doc() {
        use crate::infoset::{InfosetBuilder, InfosetEvent};
        let mut builder = InfosetBuilder::new();
        builder
            .push_event_with_hidden(
                InfosetEvent::StartElement {
                    name: QName::local("root"),
                    is_nil: false,
                },
                false,
            )
            .unwrap();
        builder
            .push_event_with_hidden(
                InfosetEvent::SimpleValue {
                    name: QName::local("count"),
                    value: DfdlValue::Int(42),
                },
                false,
            )
            .unwrap();

        let active_doc = builder.active_doc();
        let path = InfosetPath::parse("/root/count");
        let elem = active_doc.find_element(&path);
        assert!(elem.is_some());
        if let Some(e) = elem {
            assert_eq!(
                e.state,
                crate::infoset::ElementState::Value(DfdlValue::Int(42))
            );
        }
    }

    #[test]
    fn test_dfdl_functions_and_constructors() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(100);

        // checkConstraints
        let ast1 = parse_expr("{ dfdl:checkConstraints(.) }").unwrap();
        assert_eq!(
            eval_expr(&ast1, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );

        // type constructors: double, int, float, string
        let ast_dbl = parse_expr("{ xs:double('123.456') }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_dbl,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Double(123.456)
        );

        let ast_int = parse_expr("{ xs:int('123') }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_int,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Int(123)
        );

        let ast_flt = parse_expr("{ xs:float('2.5') }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_flt,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Float(2.5)
        );

        let ast_str = parse_expr("{ xs:string(999) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_str,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::String(alloc::string::String::from("999"))
        );

        // fn:compare
        let ast_cmp1 = parse_expr("{ fn:compare('abc', 'def') }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_cmp1,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Int(-1)
        );

        let ast_cmp2 = parse_expr("{ fn:compare('xyz', 'xyz') }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_cmp2,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Int(0)
        );

        // fn:true / fn:false
        let ast_t = parse_expr("{ fn:true() }").unwrap();
        assert_eq!(
            eval_expr(&ast_t, &mut ExprContext::new(None, &path, &[], &mut budget)).unwrap(),
            DfdlValue::Boolean(true)
        );

        // dfdl:lookAhead / dfdl:currentPosition
        let ast_la = parse_expr("{ dfdl:lookAhead(0, 8) }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_la,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::Long(0)
        );

        // $dfdl:encoding standard variable
        let ast_enc = parse_expr("{ $dfdl:encoding }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_enc,
                &mut ExprContext::new(None, &path, &[], &mut budget)
            )
            .unwrap(),
            DfdlValue::String(alloc::string::String::from("UTF-8"))
        );

        // dfdl:occursIndex with dynamic occurs_index context
        let ast_occ = parse_expr("{ dfdl:occursIndex() }").unwrap();
        assert_eq!(
            eval_expr(
                &ast_occ,
                &mut ExprContext::new(None, &path, &[], &mut budget).with_occurs_index(3)
            )
            .unwrap(),
            DfdlValue::Long(3)
        );

        // Wildcard step parsing (* step) must be rejected per DFDL §23
        let res_wild = parse_expr("{ /ex:e1/ex:nest/* }");
        assert!(res_wild.is_err());
        assert_eq!(res_wild.unwrap_err().kind, DFDLErrorKind::SchemaDefinition);

        // Predicate path parsing (password[1])
        let ast_pred = parse_expr("{ /e2/password[1] }").unwrap();
        assert!(matches!(ast_pred, ExprAst::Path(_)));
    }

    #[test]
    fn test_general_comparison_operators_rejected() {
        let general_ops = ["=", "!=", "<", "<=", ">", ">="];
        for op in general_ops {
            let expr_str = alloc::format!("{{ $x {} $y }}", op);
            let res = parse_expr(&expr_str);
            assert!(res.is_err(), "Expected error for general operator {}", op);
            let err = res.unwrap_err();
            assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
            let err_msg = err.to_string();
            assert!(
                err_msg.contains("Schema Definition Error"),
                "Message must state Schema Definition Error: {}",
                err_msg
            );
            assert!(
                err_msg.contains("Unsupported operation"),
                "Message must mention Unsupported operation: {}",
                err_msg
            );
        }
    }

    #[test]
    fn test_validate_expression_namespaces() {
        let namespaces = alloc::vec![
            (alloc::string::String::from("ex"), alloc::string::String::from("http://example.com")),
            (alloc::string::String::from("dfdl"), alloc::string::String::from("http://www.ogf.org/dfdl/dfdl-1.0/")),
        ];

        // Valid expressions using declared prefixes
        assert!(validate_expression_namespaces("{ /ex:root/ex:child }", &namespaces).is_ok());
        assert!(validate_expression_namespaces("{ dfdl:occursIndex() }", &namespaces).is_ok());
        assert!(validate_expression_namespaces("{ 1 + 2 }", &namespaces).is_ok());

        // Undeclared prefix in path step
        let err_path = validate_expression_namespaces("{ parent::ez:ivc_09/ex:num1 }", &namespaces).unwrap_err();
        assert_eq!(err_path.kind, DFDLErrorKind::SchemaDefinition);
        let msg_path = err_path.to_string();
        assert!(msg_path.contains("ez"));
        assert!(msg_path.contains("prefix"));
        assert!(msg_path.contains("no corresponding namespace"));

        // Undeclared prefix in function call
        let err_fn = validate_expression_namespaces("{ fn:abs(-9) }", &namespaces).unwrap_err();
        assert_eq!(err_fn.kind, DFDLErrorKind::SchemaDefinition);
        let msg_fn = err_fn.to_string();
        assert!(msg_fn.contains("fn"));
        assert!(msg_fn.contains("fn:abs"));

        // Undeclared prefix in variable
        let err_var = validate_expression_namespaces("{ $undeclared:myVar }", &namespaces).unwrap_err();
        assert_eq!(err_var.kind, DFDLErrorKind::SchemaDefinition);
        assert!(err_var.to_string().contains("undeclared"));
    }
}
