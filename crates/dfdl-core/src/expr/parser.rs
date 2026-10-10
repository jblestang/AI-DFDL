//! Pest-based parser for DFDL Expression Language.
//!
//! Conforms to DFDL 1.0 Specification §18. Implements non-panic Pest grammar parsing
//! to construct [`ExprAst`].

#![allow(missing_docs)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::str::FromStr;

use pest::Parser;
use pest_derive::Parser;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::expr::ast::{BinaryOp, ExprAst, UnaryOp};
use crate::infoset::value::DfdlValue;
use crate::types::{InfosetPath, PathAxis, PathStep, QName, StepTarget};
use crate::util::try_push;

#[allow(missing_docs)]
#[derive(Parser)]
#[grammar = "expr/dfdl_expression.pest"]
struct DfdlPestParser;

/// Compiles a DFDL expression string into an [`ExprAst`].
///
/// Rejects syntax errors with a structured [`DFDLErrorKind::ExpressionError`].
pub fn parse_expr(input: &str) -> DFDLResult<ExprAst> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Empty DFDL expression body",
        ));
    }

    let mut pairs = DfdlPestParser::parse(Rule::expression, trimmed).map_err(|e| {
        let msg = alloc::format!("Pest parse error: {}", e);
        DFDLError::new(DFDLErrorKind::ExpressionError, &msg)
    })?;

    let inner = pairs
        .next()
        .and_then(|p| p.into_inner().next())
        .ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Failed to extract root expression AST node",
            )
        })?;

    build_expr(inner)
}

fn build_expr(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    match pair.as_rule() {
        Rule::expression | Rule::expr => {
            let Some(inner) = pair.into_inner().next() else {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "Empty expression node",
                ));
            };
            build_expr(inner)
        }
        Rule::if_expr => build_if_expr(pair),
        Rule::logic_or => build_binary_chain(pair, |_| Ok(BinaryOp::Or)),
        Rule::logic_and => build_binary_chain(pair, |_| Ok(BinaryOp::And)),
        Rule::equality => build_binary_chain(
            pair,
            |op_str| match op_str {
                "=" | "!=" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('eq' or 'ne') instead",
                        op_str
                    ),
                )),
                "ne" => Ok(BinaryOp::Ne),
                _ => Ok(BinaryOp::Eq),
            },
        ),
        Rule::relational => build_binary_chain(
            pair,
            |op_str| match op_str {
                "<=" | ">=" | "<" | ">" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('lt', 'le', 'gt', 'ge') instead",
                        op_str
                    ),
                )),
                "lt" => Ok(BinaryOp::Lt),
                "le" => Ok(BinaryOp::Le),
                "ge" => Ok(BinaryOp::Ge),
                "gt" => Ok(BinaryOp::Gt),
                _ => Ok(BinaryOp::Lt),
            },
        ),
        Rule::additive => build_binary_chain(
            pair,
            |op_str| match op_str {
                "-" => Ok(BinaryOp::Sub),
                _ => Ok(BinaryOp::Add),
            },
        ),
        Rule::multiplicative => build_binary_chain(
            pair,
            |op_str| match op_str {
                "div" => Ok(BinaryOp::Div),
                "idiv" => Ok(BinaryOp::IDiv),
                "mod" => Ok(BinaryOp::Mod),
                _ => Ok(BinaryOp::Mul),
            },
        ),
        Rule::unary => build_unary(pair),
        Rule::primary => build_primary(pair),
        Rule::literal => build_literal(pair),
        Rule::var_ref => build_var_ref(pair),
        Rule::fn_call => build_fn_call(pair),
        Rule::path => build_path(pair),
        _ => {
            let msg = alloc::format!(
                "Unexpected rule in expression: {:?} ({})",
                pair.as_rule(),
                pair.as_str()
            );
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
    }
}

fn build_if_expr(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let mut inner = pair.into_inner();
    let (Some(cond_pair), Some(then_pair), Some(else_pair)) =
        (inner.next(), inner.next(), inner.next())
    else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Incomplete if-then-else expression",
        ));
    };

    let cond = build_expr(cond_pair)?;
    let then_expr = build_expr(then_pair)?;
    let else_expr = build_expr(else_pair)?;

    Ok(ExprAst::IfThenElse {
        cond: Box::new(cond),
        then_expr: Box::new(then_expr),
        else_expr: Box::new(else_expr),
    })
}

fn build_binary_chain<FMapOp>(
    pair: pest::iterators::Pair<Rule>,
    map_op: FMapOp,
) -> DFDLResult<ExprAst>
where
    FMapOp: Fn(&str) -> DFDLResult<BinaryOp>,
{
    let mut inner = pair.into_inner();
    let Some(first_pair) = inner.next() else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Missing left operand in binary expression",
        ));
    };
    let mut current = build_expr(first_pair)?;

    while let Some(op_pair) = inner.next() {
        let op = map_op(op_pair.as_str())?;
        let Some(right_pair) = inner.next() else {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Missing right operand in binary expression",
            ));
        };
        let right = build_expr(right_pair)?;
        current = ExprAst::Binary {
            op,
            left: Box::new(current),
            right: Box::new(right),
        };
    }

    Ok(current)
}

fn build_unary(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let mut inner = pair.into_inner();
    let Some(first) = inner.next() else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Empty unary expression pair",
        ));
    };

    if first.as_rule() == Rule::unary_op {
        let op = match first.as_str() {
            "+" => UnaryOp::Plus,
            "not" => UnaryOp::Not,
            _ => UnaryOp::Negate,
        };
        let Some(next_pair) = inner.next() else {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Missing operand for unary operator",
            ));
        };
        let sub_expr = build_expr(next_pair)?;
        Ok(ExprAst::Unary {
            op,
            expr: Box::new(sub_expr),
        })
    } else {
        build_expr(first)
    }
}

fn build_primary(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let Some(inner_pair) = pair.into_inner().next() else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Empty primary node",
        ));
    };

    build_expr(inner_pair)
}

fn build_literal(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let Some(child) = pair.into_inner().next() else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Empty literal node",
        ));
    };

    match child.as_rule() {
        Rule::hex_literal => {
            let s = child
                .as_str()
                .trim_start_matches("0x")
                .trim_start_matches("0X");
            if let Ok(val) = i64::from_str_radix(s, 16) {
                Ok(ExprAst::Literal(DfdlValue::Long(val)))
            } else if let Ok(uval) = u64::from_str_radix(s, 16) {
                Ok(ExprAst::Literal(DfdlValue::UnsignedLong(uval)))
            } else {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "Invalid hex literal",
                ))
            }
        }
        Rule::int_literal => {
            let s = child.as_str();
            if let Ok(val) = i64::from_str(s) {
                Ok(ExprAst::Literal(DfdlValue::Long(val)))
            } else if let Ok(uval) = u64::from_str(s) {
                Ok(ExprAst::Literal(DfdlValue::UnsignedLong(uval)))
            } else {
                Ok(ExprAst::Literal(DfdlValue::Decimal(
                    alloc::string::ToString::to_string(s),
                )))
            }
        }
        Rule::float_literal => {
            let val = f64::from_str(child.as_str()).unwrap_or(0.0);
            Ok(ExprAst::Literal(DfdlValue::Double(val)))
        }
        Rule::bool_literal => {
            let val = child.as_str() == "true";
            Ok(ExprAst::Literal(DfdlValue::Boolean(val)))
        }
        Rule::string_literal => {
            let raw = child.as_str();
            let content = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
            Ok(ExprAst::Literal(DfdlValue::String(String::from(content))))
        }
        _ => {
            let msg = alloc::format!("Unknown literal rule: {}", child.as_str());
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
    }
}

fn build_var_ref(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let s = pair.as_str().trim_start_matches('$');
    let qname = QName::parse_element_name(s);
    Ok(ExprAst::Variable(qname))
}

fn build_fn_call(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let mut inner = pair.into_inner();
    let Some(fn_name_pair) = inner.next() else {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Missing function name",
        ));
    };
    let qname = QName::parse_element_name(fn_name_pair.as_str());
    let mut args = Vec::new();

    for arg_pair in inner {
        let arg_ast = build_expr(arg_pair)?;
        try_push(&mut args, arg_ast)?;
    }

    Ok(ExprAst::FnCall { name: qname, args })
}

/// Parses a path string via Pest grammar into an [`InfosetPath`].
pub fn parse_path(path_str: &str) -> DFDLResult<InfosetPath> {
    let normalized = path_str.replace('\\', "/");
    let trimmed = normalized.trim();
    let mut pairs = DfdlPestParser::parse(Rule::path, trimmed).map_err(|e| {
        DFDLError::new(
            DFDLErrorKind::ExpressionError,
            &alloc::format!("Pest path parse error: {}", e),
        )
    })?;
    let pair = pairs
        .next()
        .ok_or_else(|| DFDLError::new_static(DFDLErrorKind::ExpressionError, "Empty path pair"))?;
    parse_path_from_pair(pair)
}

/// Constructs an [`InfosetPath`] from a Pest `Rule::path` Pair.
pub fn parse_path_from_pair(pair: pest::iterators::Pair<Rule>) -> DFDLResult<InfosetPath> {
    let raw_str = pair.as_str();
    let is_absolute = raw_str.starts_with('/');
    let mut steps = Vec::new();

    for step in pair.into_inner() {
        if step.as_rule() == Rule::path_step {
            let mut axis = PathAxis::Child;
            let mut target = StepTarget::Unprefixed(String::new());
            let mut step_node_empty = true;
            let mut index_predicate = None;
            let mut predicate_expr = None;

            for sub in step.into_inner() {
                match sub.as_rule() {
                    Rule::axis_specifier => match sub.as_str() {
                        "parent::" => axis = PathAxis::Parent,
                        "self::" => axis = PathAxis::SelfAxis,
                        _ => axis = PathAxis::Child,
                    },
                    Rule::step_node => {
                        step_node_empty = false;
                        for node_inner in sub.into_inner() {
                            match node_inner.as_rule() {
                                Rule::parent_step => target = StepTarget::ParentNode,
                                Rule::self_step => target = StepTarget::SelfNode,
                                Rule::wildcard_step => {
                                    let msg = "Schema Definition Error: Wildcard '*' is unsupported in DFDL Expression Syntax.";
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, msg));
                                }
                                Rule::qname_step => {
                                    let mut prefix = None;
                                    let mut local = String::new();
                                    let raw = alloc::string::ToString::to_string(node_inner.as_str());
                                    for qpart in node_inner.into_inner() {
                                        match qpart.as_rule() {
                                            Rule::prefix => prefix = Some(alloc::string::ToString::to_string(qpart.as_str())),
                                            Rule::local_name => local = alloc::string::ToString::to_string(qpart.as_str()),
                                            _ => {}
                                        }
                                    }
                                    if let Some(pfx) = prefix {
                                        target = StepTarget::Prefixed { prefix: pfx, local, raw };
                                    } else {
                                        target = StepTarget::Unprefixed(local);
                                    }
                                }
                                _ => {
                                    target = StepTarget::parse(node_inner.as_str());
                                }
                            }
                        }
                    }
                    Rule::predicate => {
                        let pred_str = sub.as_str();
                        let inner = if pred_str.starts_with('[') && pred_str.ends_with(']') {
                            &pred_str[1..pred_str.len().saturating_sub(1)]
                        } else {
                            pred_str
                        };
                        let trimmed = inner.trim();
                        if let Ok(idx) = trimmed.parse::<usize>() {
                            index_predicate = Some(idx);
                        }
                        predicate_expr = Some(alloc::string::ToString::to_string(trimmed));
                    }
                    _ => {}
                }
            }

            if axis == PathAxis::Parent && (step_node_empty || target == StepTarget::ParentNode || target == StepTarget::SelfNode) {
                target = StepTarget::ParentNode;
            } else if axis == PathAxis::SelfAxis && (step_node_empty || target == StepTarget::SelfNode) {
                target = StepTarget::SelfNode;
            }

            if !step_node_empty || axis != PathAxis::Child {
                let path_step = PathStep {
                    axis,
                    target,
                    index_predicate,
                    predicate_expr,
                };
                try_push(&mut steps, path_step)?;
            }
        }
    }

    Ok(InfosetPath::from_steps(steps, is_absolute))
}

fn build_path(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let path = parse_path_from_pair(pair)?;
    Ok(ExprAst::Path(path))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::expr::ast::{BinaryOp, ExprAst, UnaryOp};
    use crate::infoset::value::DfdlValue;

    /// Verifies that empty and whitespace-only expression strings are rejected cleanly.
    #[test]
    fn test_parse_empty_or_whitespace_expression() {
        let err_empty = parse_expr("").unwrap_err();
        assert!(err_empty.message.as_str().contains("Empty DFDL expression"));

        let err_ws = parse_expr("   \t\n  ").unwrap_err();
        assert!(err_ws.message.as_str().contains("Empty DFDL expression"));
    }

    /// Verifies parsing of conditional if-then-else expressions.
    #[test]
    fn test_parse_if_then_else() {
        let ast = parse_expr("if (1) then 'yes' else 'no'").unwrap();
        assert!(matches!(
            ast,
            ExprAst::IfThenElse { ref cond, ref then_expr, ref else_expr }
                if matches!(**cond, ExprAst::Literal(DfdlValue::Long(1)))
                    && matches!(**then_expr, ExprAst::Literal(DfdlValue::String(ref s)) if s == "yes")
                    && matches!(**else_expr, ExprAst::Literal(DfdlValue::String(ref s)) if s == "no")
        ));
    }

    /// Verifies parsing of unary operations (+, -, not).
    #[test]
    fn test_parse_unary_operators() {
        let ast_pos = parse_expr("+42").unwrap();
        assert!(matches!(ast_pos, ExprAst::Unary { op: UnaryOp::Plus, .. }));

        let ast_neg = parse_expr("-100").unwrap();
        assert!(matches!(ast_neg, ExprAst::Unary { op: UnaryOp::Negate, .. }));

        let ast_not = parse_expr("not($flag)").unwrap();
        assert!(matches!(ast_not, ExprAst::FnCall { .. } | ExprAst::Unary { op: UnaryOp::Not, .. }));
    }

    /// Verifies parsing of integer, hex, float, and string literals.
    #[test]
    fn test_parse_literals() {
        // Hexadecimal literals with 0x and 0X prefixes
        let ast_hex1 = parse_expr("0x1A2B").unwrap();
        assert_eq!(ast_hex1, ExprAst::Literal(DfdlValue::Long(0x1A2B)));

        let ast_hex2 = parse_expr("0X7F").unwrap();
        assert_eq!(ast_hex2, ExprAst::Literal(DfdlValue::Long(0x7F)));

        // Unsigned 64-bit hex literal exceeding i64::MAX
        let ast_uhex = parse_expr("0x8000000000000000").unwrap();
        assert_eq!(ast_uhex, ExprAst::Literal(DfdlValue::UnsignedLong(0x8000000000000000)));

        // String literals: empty and non-empty
        let ast_empty_str = parse_expr("''").unwrap();
        assert_eq!(ast_empty_str, ExprAst::Literal(DfdlValue::String(alloc::string::String::new())));

        let ast_str = parse_expr("'Hello DFDL'").unwrap();
        assert_eq!(ast_str, ExprAst::Literal(DfdlValue::String(alloc::string::String::from("Hello DFDL"))));

        // Float literals
        let ast_flt = parse_expr("12.34").unwrap();
        assert!(matches!(ast_flt, ExprAst::Literal(DfdlValue::Double(d)) if (d - 12.34).abs() < 1e-6));
    }

    /// Verifies parsing of binary operations with comparison, boolean logic, and arithmetic.
    #[test]
    fn test_parse_binary_expressions() {
        let ast_add = parse_expr("1 + 2 * 3").unwrap();
        assert!(matches!(
            ast_add,
            ExprAst::Binary { op: BinaryOp::Add, ref left, ref right }
                if matches!(**left, ExprAst::Literal(DfdlValue::Long(1)))
                    && matches!(**right, ExprAst::Binary { op: BinaryOp::Mul, .. })
        ));

        let ast_comp = parse_expr("10 gt 5 and 2 lt 3").unwrap();
        assert!(matches!(ast_comp, ExprAst::Binary { op: BinaryOp::And, .. }));

        let ast_eq = parse_expr("$a eq $b or $c ne $d").unwrap();
        assert!(matches!(ast_eq, ExprAst::Binary { op: BinaryOp::Or, .. }));

        // General comparisons must be rejected per DFDL specification
        let err_gen = parse_expr("10 > 5").unwrap_err();
        assert_eq!(err_gen.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies parsing of function calls with zero or multiple arguments.
    #[test]
    fn test_parse_function_calls() {
        let ast_zero_arg = parse_expr("fn:true()").unwrap();
        assert!(matches!(ast_zero_arg, ExprAst::FnCall { ref name, ref args } if name.local_name == "true" && args.is_empty()));

        let ast_multi_arg = parse_expr("fn:concat('a', 'b', 'c')").unwrap();
        assert!(matches!(ast_multi_arg, ExprAst::FnCall { ref name, ref args } if name.local_name == "concat" && args.len() == 3));
    }

    /// Verifies parsing of XPath paths with axes, predicates, self, parent, and relative steps.
    #[test]
    fn test_parse_paths() {
        // Absolute path
        let p_abs = parse_path("/root/header/length").unwrap();
        assert!(p_abs.is_absolute());
        assert_eq!(p_abs.segments(), &["root", "header", "length"]);

        // Relative path
        let p_rel = parse_path("items/item[1]").unwrap();
        assert!(!p_rel.is_absolute());
        assert_eq!(p_rel.segments(), &["items", "item[1]"]);

        // Self axis steps
        let p_self_dot = parse_path("self::.").unwrap();
        assert_eq!(p_self_dot.segments(), &["."]);

        let p_self_elem = parse_path("self::child").unwrap();
        assert_eq!(p_self_elem.segments(), &[".(child)"]);

        let p_self_pred = parse_path("self::child[2]").unwrap();
        assert_eq!(p_self_pred.segments(), &[".(child)[2]"]);

        // Parent axis steps
        let p_parent_dot = parse_path("..").unwrap();
        assert_eq!(p_parent_dot.segments(), &[".."]);

        let p_parent_axis = parse_path("parent::node()").unwrap();
        assert_eq!(p_parent_axis.segments(), &["..(node)"]);

        let p_parent_pred = parse_path("parent::item[1]").unwrap();
        assert_eq!(p_parent_pred.segments(), &["..(item)[1]"]);

        // Path syntax error rejection (invalid leading character)
        let err_path = parse_path("@invalid").unwrap_err();
        assert_eq!(err_path.kind, DFDLErrorKind::ExpressionError);

        // Wildcard step rejection
        let err_wild = parse_path("items/*").unwrap_err();
        assert_eq!(err_wild.kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies syntax error reporting for invalid or incomplete expressions.
    #[test]
    fn test_parse_syntax_errors() {
        let err_incomplete = parse_expr("1 +").unwrap_err();
        assert_eq!(err_incomplete.kind, DFDLErrorKind::ExpressionError);

        let err_bad_paren = parse_expr("(1 + 2").unwrap_err();
        assert_eq!(err_bad_paren.kind, DFDLErrorKind::ExpressionError);

        // Empty expression body
        let err_empty = parse_expr("   ").unwrap_err();
        assert_eq!(err_empty.kind, DFDLErrorKind::ExpressionError);

        // General comparison operators must be rejected with SchemaDefinition error
        assert_eq!(parse_expr("5 = 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(parse_expr("5 != 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(parse_expr("5 < 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(parse_expr("5 <= 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(parse_expr("5 > 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(parse_expr("5 >= 5").unwrap_err().kind, DFDLErrorKind::SchemaDefinition);
    }

    /// Verifies additional binary and unary operators (idiv, mod, unary plus).
    #[test]
    fn test_parse_arithmetic_and_literal_extensions() {
        // Multiplicative idiv and mod
        let ast_idiv = parse_expr("10 idiv 2").unwrap();
        assert_eq!(
            ast_idiv,
            ExprAst::Binary {
                op: BinaryOp::IDiv,
                left: Box::new(ExprAst::Literal(DfdlValue::Long(10))),
                right: Box::new(ExprAst::Literal(DfdlValue::Long(2))),
            }
        );

        let ast_mod = parse_expr("10 mod 3").unwrap();
        assert_eq!(
            ast_mod,
            ExprAst::Binary {
                op: BinaryOp::Mod,
                left: Box::new(ExprAst::Literal(DfdlValue::Long(10))),
                right: Box::new(ExprAst::Literal(DfdlValue::Long(3))),
            }
        );

        // Unary plus
        let ast_plus = parse_expr("+42").unwrap();
        assert_eq!(
            ast_plus,
            ExprAst::Unary {
                op: UnaryOp::Plus,
                expr: Box::new(ExprAst::Literal(DfdlValue::Long(42))),
            }
        );

        // Unsigned 64-bit hex literal (MSB set)
        let ast_u64_hex = parse_expr("0xFFFFFFFFFFFFFFFF").unwrap();
        assert_eq!(ast_u64_hex, ExprAst::Literal(DfdlValue::UnsignedLong(u64::MAX)));

        // Unsigned 64-bit int literal (greater than i64::MAX)
        let ast_u64_dec = parse_expr("18446744073709551615").unwrap();
        assert_eq!(ast_u64_dec, ExprAst::Literal(DfdlValue::UnsignedLong(u64::MAX)));

        // Arbitrary precision decimal literal beyond u64::MAX
        let ast_big_dec = parse_expr("999999999999999999999999999999").unwrap();
        assert_eq!(
            ast_big_dec,
            ExprAst::Literal(DfdlValue::Decimal(String::from("999999999999999999999999999999")))
        );

        // Paths with backslashes normalized to slashes
        let p_bs = parse_path(r"root\child\subchild").unwrap();
        assert_eq!(p_bs.segments(), &["root", "child", "subchild"]);

        // Function call with multiple arguments
        let ast_fn = parse_expr("concat('a', 'b', 'c')").unwrap();
        assert!(matches!(ast_fn, ExprAst::FnCall { ref name, ref args } if name.local_name == "concat" && args.len() == 3));
    }

    /// Verifies parsing of additional operators, axis variations, and if-then-else branches.
    #[test]
    fn test_parse_additional_expressions_and_paths() {
        // Additional binary operators
        let ast_div = parse_expr("20 div 4").unwrap();
        assert!(matches!(ast_div, ExprAst::Binary { op: BinaryOp::Div, .. }));

        let ast_sub = parse_expr("20 - 4").unwrap();
        assert!(matches!(ast_sub, ExprAst::Binary { op: BinaryOp::Sub, .. }));

        let ast_le = parse_expr("10 le 20").unwrap();
        assert!(matches!(ast_le, ExprAst::Binary { op: BinaryOp::Le, .. }));

        let ast_ge = parse_expr("20 ge 10").unwrap();
        assert!(matches!(ast_ge, ExprAst::Binary { op: BinaryOp::Ge, .. }));

        // Axis steps variations: parent::., parent::.., self::., parent::name, self::name
        let p_par_dot = parse_path("parent::.").unwrap();
        assert_eq!(p_par_dot.segments(), &[".."]);

        let p_par_dotdot = parse_path("parent::..").unwrap();
        assert_eq!(p_par_dotdot.segments(), &[".."]);

        let p_par_name = parse_path("parent::item").unwrap();
        assert_eq!(p_par_name.segments(), &["..(item)"]);

        let p_self_name = parse_path("self::item").unwrap();
        assert_eq!(p_self_name.segments(), &[".(item)"]);

        // Axis steps with predicates
        let p_par_pred = parse_path("parent::item[1]").unwrap();
        assert_eq!(p_par_pred.segments(), &["..(item)[1]"]);

        let p_self_pred = parse_path("self::item[1]").unwrap();
        assert_eq!(p_self_pred.segments(), &[".(item)[1]"]);

        // Self::.
        let p_self_dot = parse_path("self::.").unwrap();
        assert_eq!(p_self_dot.segments(), &["."]);

        // Wildcard path step rejected per DFDL expression syntax
        assert!(parse_path("/root/*").is_err());

        // IDiv operator
        let ast_idiv = parse_expr("10 idiv 3").unwrap();
        assert!(matches!(ast_idiv, ExprAst::Binary { op: BinaryOp::IDiv, .. }));

        // General comparisons must be rejected
        assert!(parse_expr("5 = 5").is_err());
        assert!(parse_expr("5 != 5").is_err());
        assert!(parse_expr("5 < 5").is_err());
        assert!(parse_expr("5 <= 5").is_err());
        assert!(parse_expr("5 >= 5").is_err());
        assert!(parse_expr("5 > 5").is_err());

        // If then else
        let ast_if = parse_expr("if (10 gt 5) then 'yes' else 'no'").unwrap();
        assert!(matches!(ast_if, ExprAst::IfThenElse { .. }));

        // Unary plus
        let ast_plus = parse_expr("+42").unwrap();
        assert!(matches!(ast_plus, ExprAst::Unary { op: UnaryOp::Plus, .. }));

        // Parent axis with .. and .
        let p_par_dot = parse_path("parent::..").unwrap();
        assert_eq!(p_par_dot.segments(), &[".."]);
        let p_par_dot2 = parse_path("parent::.").unwrap();
        assert_eq!(p_par_dot2.segments(), &[".."]);

        // Function call with multiple arguments and 0 arguments
        let ast_fn = parse_expr("fn:concat('a', 'b', 'c')").unwrap();
        assert!(matches!(ast_fn, ExprAst::FnCall { .. }));
        let ast_fn0 = parse_expr("fn:true()").unwrap();
        assert!(matches!(ast_fn0, ExprAst::FnCall { .. }));

        // Absolute path
        let p_abs = parse_path("/a/b/c").unwrap();
        assert!(p_abs.is_absolute());

        // Empty string with double quotes (line 334)
        let ast_empty_dq = parse_expr("\"\"").unwrap();
        assert_eq!(ast_empty_dq, ExprAst::Literal(DfdlValue::String(alloc::string::String::new())));

        // build_expr with unexpected rule fallback (lines 143-148)
        let pair_hex = DfdlPestParser::parse(Rule::hex_literal, "0x42").unwrap().next().unwrap();
        assert!(build_expr(pair_hex).is_err());

        // General comparisons rejection (lines 86-92, 102-108)
        assert!(parse_expr("1 = 1").is_err());
        assert!(parse_expr("1 != 2").is_err());
        assert!(parse_expr("1 <= 2").is_err());
        assert!(parse_expr("1 >= 2").is_err());
        assert!(parse_expr("1 < 2").is_err());
        assert!(parse_expr("1 > 2").is_err());

        // Value comparisons acceptance
        assert!(parse_expr("1 eq 1").is_ok());
        assert!(parse_expr("1 ne 2").is_ok());
        assert!(parse_expr("1 le 2").is_ok());
        assert!(parse_expr("1 ge 2").is_ok());
        assert!(parse_expr("1 lt 2").is_ok());
        assert!(parse_expr("1 gt 2").is_ok());

        // Unsigned long and overflow literals (lines 294, 298, 308, 310)
        let hex_u64 = parse_expr("0x8000000000000000").unwrap();
        assert_eq!(hex_u64, ExprAst::Literal(DfdlValue::UnsignedLong(0x8000000000000000)));

        assert!(parse_expr("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").is_err());

        let dec_u64 = parse_expr("9223372036854775808").unwrap();
        assert_eq!(dec_u64, ExprAst::Literal(DfdlValue::UnsignedLong(9223372036854775808)));

        let dec_huge = parse_expr("9999999999999999999999999999999999").unwrap();
        assert_eq!(dec_huge, ExprAst::Literal(DfdlValue::Decimal("9999999999999999999999999999999999".into())));

        // Single quoted string literal
        let sq_str = parse_expr("'single quoted string'").unwrap();
        assert_eq!(sq_str, ExprAst::Literal(DfdlValue::String("single quoted string".into())));
    }

    /// Tests additional expression parser branches including unary operators,
    /// arithmetic binary chains, conditional expressions, path axis qualifiers, and wildcard rejection.
    ///
    /// Verifies that:
    /// 1. Unary `+` and `-` operators are parsed correctly.
    /// 2. Binary arithmetic operators (`+`, `-`, `*`, `div`, `idiv`, `mod`) produce expected AST nodes.
    /// 3. `if (...) then ... else ...` constructs are converted to `ExprAst::IfThenElse`.
    /// 4. Path expressions with `parent::` and `self::` axes and predicates format internal steps.
    /// 5. Wildcard steps (`*`) in path expressions are rejected with a schema definition error.
    /// 6. Defensive AST building methods reject mismatched rule pairs.
    #[test]
    fn test_expr_parser_extended_coverage() {
        // 1. Unary operators (lines 237, 238)
        let ast_uplus = parse_expr("+42").unwrap();
        assert!(matches!(ast_uplus, ExprAst::Unary { op: UnaryOp::Plus, .. }));
        let ast_uminus = parse_expr("-42").unwrap();
        assert!(matches!(ast_uminus, ExprAst::Unary { op: UnaryOp::Negate, .. }));

        // 2. Binary arithmetic operators (lines 120, 121, 129, 130, 131, 132)
        assert!(matches!(parse_expr("10 + 2").unwrap(), ExprAst::Binary { op: BinaryOp::Add, .. }));
        assert!(matches!(parse_expr("10 - 2").unwrap(), ExprAst::Binary { op: BinaryOp::Sub, .. }));
        assert!(matches!(parse_expr("10 * 2").unwrap(), ExprAst::Binary { op: BinaryOp::Mul, .. }));
        assert!(matches!(parse_expr("10 div 2").unwrap(), ExprAst::Binary { op: BinaryOp::Div, .. }));
        assert!(matches!(parse_expr("10 idiv 2").unwrap(), ExprAst::Binary { op: BinaryOp::IDiv, .. }));
        assert!(matches!(parse_expr("10 mod 2").unwrap(), ExprAst::Binary { op: BinaryOp::Mod, .. }));

        // 3. If-then-else expressions (lines 169-173)
        let ast_if = parse_expr("if (fn:true()) then 10 else 20").unwrap();
        assert!(matches!(ast_if, ExprAst::IfThenElse { .. }));

        // 4. Path axis qualifiers (lines 444, 456, 458)
        let p_parent_pred = parse_path("parent::item[1]").unwrap();
        assert_eq!(p_parent_pred.segments(), &["..(item)[1]"]);

        let p_self_pred = parse_path("self::item[1]").unwrap();
        assert_eq!(p_self_pred.segments(), &[".(item)[1]"]);

        let p_self_bare = parse_path("self::item").unwrap();
        assert_eq!(p_self_bare.segments(), &[".(item)"]);

        // 5. Wildcard rejection in path (line 415)
        let err_wildcard = parse_path("/root/*").unwrap_err();
        assert!(err_wildcard.message.as_str().contains("Wildcard '*' is unsupported"));

        // 6. Defensive AST building errors (lines 245, 342)
        let pair_primary = DfdlPestParser::parse(Rule::primary, "(1)").unwrap().next().unwrap();
        assert!(build_literal(pair_primary).is_err());

        let pair_uop = DfdlPestParser::parse(Rule::unary_op, "+").unwrap().next().unwrap();
        assert!(build_unary(pair_uop).is_err());

        // 7. Path axis step node forms: parent::.., parent::., self::. (lines 440, 452)
        let p_parent_dotdot = parse_path("parent::..").unwrap();
        assert_eq!(p_parent_dotdot.segments(), &[".."]);

        let p_parent_dot = parse_path("parent::.").unwrap();
        assert_eq!(p_parent_dot.segments(), &[".."]);

        let p_self_dot = parse_path("self::.").unwrap();
        assert_eq!(p_self_dot.segments(), &["."]);

        // 8. Variable reference AST building (lines 348-352)
        let pair_var = DfdlPestParser::parse(Rule::var_ref, "$myVar").unwrap().next().unwrap();
        let ast_var = build_var_ref(pair_var).unwrap();
        assert!(matches!(ast_var, ExprAst::Variable(ref q) if q.local_name == "myVar"));

        // 9. Unary not operator (line 239)
        let pair_unary_not = DfdlPestParser::parse(Rule::unary, "not fn:true()").unwrap().next().unwrap();
        let ast_not = build_unary(pair_unary_not).unwrap();
        assert!(matches!(ast_not, ExprAst::Unary { op: UnaryOp::Not, .. }));

        // 10. Axis with and without predicate (lines 438-442, 450-454)
        let p_parent_pred = parse_path("parent::item[1]").unwrap();
        assert_eq!(p_parent_pred.segments(), &["..(item)[1]"]);

        let p_parent_no_pred = parse_path("parent::item").unwrap();
        assert_eq!(p_parent_no_pred.segments(), &["..(item)"]);

        let p_self_pred = parse_path("self::item[1]").unwrap();
        assert_eq!(p_self_pred.segments(), &[".(item)[1]"]);

        let p_self_no_pred = parse_path("self::item").unwrap();
        assert_eq!(p_self_no_pred.segments(), &[".(item)"]);

        // 11. Defensive error checks on inner-less AST builders (lines 140, 166, 225, 236, 300)
        let pair_axis = DfdlPestParser::parse(Rule::axis_specifier, "self::").unwrap().next().unwrap();
        assert!(build_if_expr(pair_axis.clone()).is_err());
        assert!(build_binary_chain(pair_axis.clone(), |_| Ok(BinaryOp::Or)).is_err());
        assert!(build_primary(pair_axis.clone()).is_err());
        assert!(build_literal(pair_axis.clone()).is_err());
        assert!(build_fn_call(pair_axis).is_err());

        // 12. Expression containing path invokes build_path (lines 415-417)
        let ast_path = parse_expr("/foo/bar").unwrap();
        assert!(matches!(ast_path, ExprAst::Path(_)));

        let ast_path_rel = parse_expr("../sibling").unwrap();
        assert!(matches!(ast_path_rel, ExprAst::Path(_)));
    }
}

