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
use crate::types::{InfosetPath, QName};
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

    let pairs = DfdlPestParser::parse(Rule::expression, trimmed).map_err(|e| {
        let msg = alloc::format!("Pest parse error: {}", e);
        DFDLError::new(DFDLErrorKind::ExpressionError, &msg)
    })?;

    for pair in pairs {
        if pair.as_rule() == Rule::expression {
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::expr {
                    return build_expr(inner);
                }
            }
        }
    }

    Err(DFDLError::new_static(
        DFDLErrorKind::ExpressionError,
        "Failed to extract root expression AST node",
    ))
}

fn build_expr(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    match pair.as_rule() {
        Rule::expression | Rule::expr => {
            let inner = match pair.into_inner().next() {
                Some(p) => p,
                None => {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::ExpressionError,
                        "Empty expression node",
                    ))
                }
            };
            build_expr(inner)
        }
        Rule::if_expr => build_if_expr(pair),
        Rule::logic_or => {
            build_binary_chain(pair, |rule| matches!(rule, Rule::or_op), |_| Ok(BinaryOp::Or))
        }
        Rule::logic_and => {
            build_binary_chain(pair, |rule| matches!(rule, Rule::and_op), |_| Ok(BinaryOp::And))
        }
        Rule::equality => build_binary_chain(
            pair,
            |rule| matches!(rule, Rule::eq_op),
            |op_str| match op_str {
                "=" | "!=" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('eq' or 'ne') instead",
                        op_str
                    ),
                )),
                "eq" => Ok(BinaryOp::Eq),
                "ne" => Ok(BinaryOp::Ne),
                _ => Ok(BinaryOp::Eq),
            },
        ),
        Rule::relational => build_binary_chain(
            pair,
            |rule| matches!(rule, Rule::rel_op),
            |op_str| match op_str {
                "<=" | ">=" | "<" | ">" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('lt', 'le', 'gt', 'ge') instead",
                        op_str
                    ),
                )),
                "le" => Ok(BinaryOp::Le),
                "ge" => Ok(BinaryOp::Ge),
                "lt" => Ok(BinaryOp::Lt),
                "gt" => Ok(BinaryOp::Gt),
                _ => Ok(BinaryOp::Lt),
            },
        ),
        Rule::additive => build_binary_chain(
            pair,
            |rule| matches!(rule, Rule::add_op),
            |op_str| match op_str {
                "+" => Ok(BinaryOp::Add),
                "-" => Ok(BinaryOp::Sub),
                _ => Ok(BinaryOp::Add),
            },
        ),
        Rule::multiplicative => build_binary_chain(
            pair,
            |rule| matches!(rule, Rule::mul_op),
            |op_str| match op_str {
                "*" => Ok(BinaryOp::Mul),
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
    let cond_pair = inner.next().ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::ExpressionError, "Missing if condition")
    })?;
    let then_pair = inner.next().ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::ExpressionError, "Missing then expression")
    })?;
    let else_pair = inner.next().ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::ExpressionError, "Missing else expression")
    })?;

    let cond = build_expr(cond_pair)?;
    let then_expr = build_expr(then_pair)?;
    let else_expr = build_expr(else_pair)?;

    Ok(ExprAst::IfThenElse {
        cond: Box::new(cond),
        then_expr: Box::new(then_expr),
        else_expr: Box::new(else_expr),
    })
}

fn build_binary_chain<FIsOp, FMapOp>(
    pair: pest::iterators::Pair<Rule>,
    is_op: FIsOp,
    map_op: FMapOp,
) -> DFDLResult<ExprAst>
where
    FIsOp: Fn(Rule) -> bool,
    FMapOp: Fn(&str) -> DFDLResult<BinaryOp>,
{
    let mut inner = pair.into_inner();
    let first = match inner.next() {
        Some(p) => build_expr(p)?,
        None => {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Missing left operand in binary expression",
            ))
        }
    };

    let mut current = first;
    while let Some(op_pair) = inner.next() {
        if is_op(op_pair.as_rule()) {
            let op = map_op(op_pair.as_str())?;
            let right_pair = match inner.next() {
                Some(p) => p,
                None => {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::ExpressionError,
                        "Missing right operand in binary expression",
                    ))
                }
            };
            let right = build_expr(right_pair)?;
            current = ExprAst::Binary {
                op,
                left: Box::new(current),
                right: Box::new(right),
            };
        } else {
            current = build_expr(op_pair)?;
        }
    }

    Ok(current)
}

fn build_unary(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let mut inner = pair.into_inner();
    let first = match inner.next() {
        Some(p) => p,
        None => {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Empty unary expression pair",
            ))
        }
    };

    if first.as_rule() == Rule::unary_op {
        let op = match first.as_str() {
            "+" => UnaryOp::Plus,
            "-" => UnaryOp::Negate,
            "not" => UnaryOp::Not,
            _ => UnaryOp::Negate,
        };
        let next_pair = match inner.next() {
            Some(p) => p,
            None => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "Missing operand for unary operator",
                ))
            }
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
    let inner_pair = match pair.into_inner().next() {
        Some(p) => p,
        None => {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Empty primary node",
            ))
        }
    };

    build_expr(inner_pair)
}

fn build_literal(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let child = match pair.into_inner().next() {
        Some(p) => p,
        None => {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Empty literal node",
            ))
        }
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
            let val = f64::from_str(child.as_str()).map_err(|_| {
                DFDLError::new_static(DFDLErrorKind::ExpressionError, "Invalid float literal")
            })?;
            Ok(ExprAst::Literal(DfdlValue::Double(val)))
        }
        Rule::bool_literal => {
            let val = child.as_str() == "true";
            Ok(ExprAst::Literal(DfdlValue::Boolean(val)))
        }
        Rule::string_literal => {
            let raw = child.as_str();
            let content = if (raw.starts_with('"') && raw.ends_with('"'))
                || (raw.starts_with('\'') && raw.ends_with('\''))
            {
                let len = raw.len();
                if len >= 2 {
                    raw.get(1..len.saturating_sub(1)).unwrap_or_default()
                } else {
                    ""
                }
            } else {
                raw
            };
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
    let fn_name_pair = match inner.next() {
        Some(p) => p,
        None => {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "Missing function name",
            ))
        }
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
    let mut segments = Vec::new();

    for step in pair.into_inner() {
        if step.as_rule() == Rule::path_step {
            let mut axis_opt: Option<&str> = None;
            let mut step_node_str = "";
            let mut predicate_str: Option<&str> = None;

            for sub in step.into_inner() {
                match sub.as_rule() {
                    Rule::axis_specifier => {
                        axis_opt = Some(sub.as_str());
                    }
                    Rule::step_node => {
                        for node_inner in sub.into_inner() {
                            match node_inner.as_rule() {
                                Rule::parent_step => step_node_str = "..",
                                Rule::self_step => step_node_str = ".",
                                Rule::wildcard_step => {
                                    let msg = "Schema Definition Error: Wildcard '*' is unsupported in DFDL Expression Syntax.";
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, msg));
                                }
                                Rule::qname_step => {
                                    step_node_str = node_inner.as_str();
                                }
                                _ => {}
                            }
                        }
                    }
                    Rule::predicate => {
                        predicate_str = Some(sub.as_str());
                    }
                    _ => {}
                }
            }

            let full_step = if let Some(pred) = predicate_str {
                alloc::format!("{}{}", step_node_str, pred)
            } else {
                alloc::string::ToString::to_string(step_node_str)
            };

            match axis_opt {
                Some("parent::") => {
                    if step_node_str.is_empty() || step_node_str == ".." || step_node_str == "." {
                        try_push(&mut segments, String::from(".."))?;
                    } else {
                        let parent_step = if let Some(pred) = predicate_str {
                            alloc::format!("..({})[{}]", step_node_str, pred)
                        } else {
                            alloc::format!("..({})", step_node_str)
                        };
                        try_push(&mut segments, parent_step)?;
                    }
                }
                Some("self::") => {
                    if step_node_str == "." || step_node_str.is_empty() {
                        try_push(&mut segments, String::from("."))?;
                    } else {
                        let self_step = if let Some(pred) = predicate_str {
                            alloc::format!(".({})[{}]", step_node_str, pred)
                        } else {
                            alloc::format!(".({})", step_node_str)
                        };
                        try_push(&mut segments, self_step)?;
                    }
                }
                _ => {
                    if !step_node_str.is_empty() {
                        try_push(&mut segments, full_step)?;
                    }
                }
            }
        }
    }

    Ok(InfosetPath::from_parts(segments, is_absolute))
}

fn build_path(pair: pest::iterators::Pair<Rule>) -> DFDLResult<ExprAst> {
    let path = parse_path_from_pair(pair)?;
    Ok(ExprAst::Path(path))
}
