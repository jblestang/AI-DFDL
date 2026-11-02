//! Expression Evaluation Engine for DFDL expressions.
//!
//! Evaluates compiled [`ExprAst`] against an [`InfosetDocument`] context.
//! Enforces bounded stack depth and consumes [`WorkBudget`]. Panic-free.

pub mod context;
pub mod functions;
pub mod helpers;
pub mod navigation;
pub mod operators;
#[cfg(test)]
mod tests;

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::expr::ast::{BinaryOp, ExprAst};
use crate::infoset::state::ElementState;
use crate::infoset::tree::InfosetElement;
use crate::infoset::value::DfdlValue;
use crate::types::{InfosetPath, QName};

pub use context::{ExprContext, MAX_EVAL_DEPTH};
pub use navigation::resolve_path;

use functions::eval_fn_call;
use navigation::{calc_elem_value_length, eval_ovc_for_path, path_node_exists};
use operators::{eval_binary, eval_unary};

/// Evaluates a compiled [`ExprAst`] within an [`ExprContext`].
pub fn eval_expr(ast: &ExprAst, ctx: &mut ExprContext) -> DFDLResult<DfdlValue> {
    ctx.budget.consume(1)?;

    if ctx.depth >= MAX_EVAL_DEPTH {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "Expression evaluation depth limit exceeded",
        ));
    }

    match ast {
        ExprAst::Literal(val) => Ok(val.clone()),
        ExprAst::Variable(qname) => lookup_variable(qname, ctx),
        ExprAst::Path(path) => resolve_path(path, ctx),
        ExprAst::IfThenElse {
            cond,
            then_expr,
            else_expr,
        } => {
            ctx.depth = ctx.depth.saturating_add(1);
            let cond_val = eval_expr(cond, ctx)?;
            ctx.depth = ctx.depth.saturating_sub(1);
            let is_true = match cond_val {
                DfdlValue::Boolean(b) => b,
                DfdlValue::Int(n) => n != 0,
                DfdlValue::Long(n) => n != 0,
                DfdlValue::Short(n) => n != 0,
                DfdlValue::Byte(n) => n != 0,
                DfdlValue::UnsignedLong(n) => n != 0,
                DfdlValue::UnsignedInt(n) => n != 0,
                DfdlValue::UnsignedShort(n) => n != 0,
                DfdlValue::UnsignedByte(n) => n != 0,
                DfdlValue::Float(f) => f != 0.0 && !f.is_nan(),
                DfdlValue::Double(f) => f != 0.0 && !f.is_nan(),
                DfdlValue::String(ref s) => match s.as_str() {
                    "true" | "1" => true,
                    "false" | "0" | "" => false,
                    _ => !s.is_empty(),
                },
                _ => {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::TypeError,
                        "Condition in if-then-else must evaluate to boolean",
                    ));
                }
            };
            if is_true {
                ctx.depth = ctx.depth.saturating_add(1);
                let res = eval_expr(then_expr, ctx)?;
                ctx.depth = ctx.depth.saturating_sub(1);
                Ok(res)
            } else {
                ctx.depth = ctx.depth.saturating_add(1);
                let res = eval_expr(else_expr, ctx)?;
                ctx.depth = ctx.depth.saturating_sub(1);
                Ok(res)
            }
        }
        ExprAst::Unary { op, expr } => {
            ctx.depth = ctx.depth.saturating_add(1);
            let val = eval_expr(expr, ctx)?;
            ctx.depth = ctx.depth.saturating_sub(1);
            eval_unary(*op, val)
        }
        ExprAst::Binary { op, left, right } => {
            ctx.depth = ctx.depth.saturating_add(1);
            match op {
                BinaryOp::And => {
                    let left_val = eval_expr(left, ctx)?;
                    match left_val {
                        DfdlValue::Boolean(false) => {
                            ctx.depth = ctx.depth.saturating_sub(1);
                            Ok(DfdlValue::Boolean(false))
                        }
                        DfdlValue::Boolean(true) => {
                            let right_val = eval_expr(right, ctx)?;
                            ctx.depth = ctx.depth.saturating_sub(1);
                            match right_val {
                                DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(b)),
                                _ => Err(DFDLError::new_static(
                                    DFDLErrorKind::TypeError,
                                    "Logical AND requires boolean operands",
                                )),
                            }
                        }
                        _ => {
                            ctx.depth = ctx.depth.saturating_sub(1);
                            Err(DFDLError::new_static(
                                DFDLErrorKind::TypeError,
                                "Logical AND requires boolean operands",
                            ))
                        }
                    }
                }
                BinaryOp::Or => {
                    let left_val = eval_expr(left, ctx)?;
                    match left_val {
                        DfdlValue::Boolean(true) => {
                            ctx.depth = ctx.depth.saturating_sub(1);
                            Ok(DfdlValue::Boolean(true))
                        }
                        DfdlValue::Boolean(false) => {
                            let right_val = eval_expr(right, ctx)?;
                            ctx.depth = ctx.depth.saturating_sub(1);
                            match right_val {
                                DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(b)),
                                _ => Err(DFDLError::new_static(
                                    DFDLErrorKind::TypeError,
                                    "Logical OR requires boolean operands",
                                )),
                            }
                        }
                        _ => {
                            ctx.depth = ctx.depth.saturating_sub(1);
                            Err(DFDLError::new_static(
                                DFDLErrorKind::TypeError,
                                "Logical OR requires boolean operands",
                            ))
                        }
                    }
                }
                _ => {
                    let left_val = eval_expr(left, ctx)?;
                    let right_val = eval_expr(right, ctx)?;
                    ctx.depth = ctx.depth.saturating_sub(1);
                    eval_binary(*op, left_val, right_val)
                }
            }
        }
        ExprAst::FnCall { name, args } => {
            if name.local_name == "trace" {
                if name.prefix.as_deref() == Some("fn") {
                    let msg = "Schema Definition Error: Unsupported function: fn:trace";
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, msg));
                }
                if let Some(ExprAst::Path(path)) = args.first() {
                    match resolve_path(path, ctx) {
                        Ok(val) => return Ok(val),
                        Err(e) if e.to_string().contains("does not have a simple value") => {
                            let elem_name = path
                                .last_step()
                                .map(|s| s.local_name())
                                .unwrap_or("complex");
                            return Ok(DfdlValue::String(alloc::string::ToString::to_string(
                                elem_name,
                            )));
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            if name.local_name == "exists" || name.local_name == "empty" {
                if let Some(ExprAst::Path(path)) = args.first() {
                    let exists = path_node_exists(path, ctx)?;
                    return Ok(DfdlValue::Boolean(if name.local_name == "exists" {
                        exists
                    } else {
                        !exists
                    }));
                }
            }
            if name.local_name == "count" {
                if let Some(ExprAst::Path(path)) = args.first() {
                    if let Some(doc) = ctx.doc {
                        if let Some(last_step) = path.last_step() {
                            let clean_target = last_step.local_name();
                            let norm_parent = path
                                .parent_path()
                                .map(|p| p.normalized_against(ctx.current_path))
                                .unwrap_or_else(|| ctx.current_path.clone());

                            if let Some(parent_elem) =
                                doc.find_element_with_context(&norm_parent, ctx.occurs_index)
                            {
                                let mut match_count = 0usize;
                                for child in &parent_elem.children {
                                    let crate::infoset::InfosetNode::Element(ref child_elem) =
                                        child;
                                    let clean_child = child_elem.name.local_name.as_str();
                                    if clean_target == "*" || clean_child == clean_target {
                                        match_count = match_count.saturating_add(1);
                                    }
                                }
                                return Ok(DfdlValue::Long(match_count as i64));
                            } else {
                                return Ok(DfdlValue::Long(0));
                            }
                        }
                    }
                }
            }
            if name.local_name == "valueLength" || name.local_name == "contentLength" {
                if let Some(ExprAst::Path(path)) = args.first() {
                    let units_val = if let Some(arg1) = args.get(1) {
                        eval_expr(arg1, ctx).ok()
                    } else {
                        None
                    };
                    let units = units_val
                        .map(|u| alloc::format!("{}", u))
                        .unwrap_or_else(|| String::from("bytes"));

                    let norm = path.normalized_against(ctx.current_path);
                    let norm_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
                    if let Some((_, len, len_units, enc)) =
                        ctx.enclosing_lengths.iter().rev().find(|(name, _, _, _)| {
                            name == norm_name || norm_name.ends_with(name.as_str())
                        })
                    {
                        let bit_len = match len_units {
                            crate::schema::ir::LengthUnits::Bits => *len,
                            crate::schema::ir::LengthUnits::Bytes => len.saturating_mul(8),
                            crate::schema::ir::LengthUnits::Characters => {
                                len.saturating_mul(crate::encoding::encoding_unit_bits(enc))
                            }
                        };
                        let res_len = match units.to_lowercase().as_str() {
                            "bits" => bit_len as i64,
                            "characters" => {
                                (bit_len
                                    .checked_div(crate::encoding::encoding_unit_bits(enc))
                                    .unwrap_or(0)) as i64
                            }
                            _ => (bit_len.checked_div(8).unwrap_or(0)) as i64,
                        };
                        return Ok(DfdlValue::Long(res_len));
                    }

                    let is_enclosing = if norm.steps().len() < ctx.current_path.steps().len()
                        && ctx.current_path.steps().starts_with(norm.steps())
                    {
                        true
                    } else if norm == *ctx.current_path {
                        let is_self_length_calc = if let Some(sch) = ctx.schema {
                            let clean_name = norm.last_step().map(|s| s.local_name()).unwrap_or("");
                            if let Some(term) = sch
                                .find_term_by_name(clean_name)
                                .and_then(|id| sch.get_term(id))
                            {
                                term.properties.length_expr.is_some()
                                    || term.properties.truncate_specified_length_string
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if is_self_length_calc {
                            let current_name = ctx
                                .current_path
                                .last_step()
                                .map(|s| s.local_name())
                                .unwrap_or("");
                            let msg = alloc::format!(
                                "Runtime Schema Definition Error: Value length unknown: cannot evaluate dfdl:valueLength for element '{}' while evaluating its length expression",
                                current_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        let has_simple_val = ctx
                            .doc
                            .and_then(|doc| doc.find_element_with_context(&norm, ctx.occurs_index))
                            .is_some_and(|elem| matches!(&elem.state, ElementState::Value(_)));
                        !has_simple_val
                    } else {
                        false
                    };

                    if is_enclosing {
                        let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
                        let qname = if elem_name.contains(':') {
                            alloc::string::ToString::to_string(elem_name)
                        } else {
                            alloc::format!("ex:{}", elem_name)
                        };
                        let current_name = ctx
                            .current_path
                            .last_step()
                            .map(|s| s.local_name())
                            .unwrap_or("");
                        let msg = alloc::format!(
                            "Runtime Schema Definition Error: Expression Evaluation Error in '{}': Value Length cannot be computed for enclosing element '{}'",
                            current_name, qname
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }

                    if let Some(sch) = ctx.schema {
                        let clean_target = norm.last_step().map(|s| s.local_name()).unwrap_or("");
                        let clean_curr = ctx
                            .current_path
                            .last_step()
                            .map(|s| s.local_name())
                            .unwrap_or("");
                        if !clean_curr.is_empty() && clean_target != clean_curr {
                            if let Some(target_term) = sch
                                .find_term_by_name(clean_target)
                                .and_then(|id| sch.get_term(id))
                            {
                                if target_term.properties.truncate_specified_length_string {
                                    if let Some(ref len_expr) = target_term.properties.length_expr {
                                        if len_expr.contains(clean_curr) {
                                            let msg = alloc::format!(
                                                "Expression Evaluation Error: Circular reference / circular dependency: element '{}' does not have a value because its length depends on '{}'",
                                                clean_target, clean_curr
                                            );
                                            return Err(DFDLError::new(
                                                DFDLErrorKind::ExpressionError,
                                                &msg,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let val_opt = if let Some(doc) = ctx.doc {
                        doc.find_element_with_policy_checked(
                            &norm,
                            ctx.occurs_index,
                            is_enclosing,
                            ctx.unqualified_path_step_policy,
                            ctx.in_scope_namespaces,
                        )
                        .ok()
                        .flatten()
                        .or_else(|| doc.find_element_with_context(&norm, ctx.occurs_index))
                        .filter(|elem| {
                            matches!(&elem.state, ElementState::Value(_))
                                || !elem.children.is_empty()
                        })
                    } else {
                        None
                    };

                    if let Some(elem) = val_opt {
                        if !elem.children.is_empty() {
                            if let Some(sch) = ctx.schema {
                                if let Ok(bits) =
                                    crate::kernel::unparser::measure_element_content_bits(
                                        sch,
                                        ctx.doc,
                                        elem,
                                        &norm,
                                        ctx.variable_map,
                                    )
                                {
                                    let res_len = match units.to_lowercase().as_str() {
                                        "bits" => bits as i64,
                                        _ => (bits.checked_div(8).unwrap_or(0)) as i64,
                                    };
                                    return Ok(DfdlValue::Long(res_len));
                                }
                            }
                        }
                    }

                    let computed_len: Option<usize> = if let Some(elem) = val_opt {
                        Some(calc_elem_value_length(elem, ctx.schema, &norm, Some(ctx)))
                    } else if let Some(ovc_val) = eval_ovc_for_path(ctx, &norm) {
                        let clean_name = norm.last_step().map(|s| s.local_name()).unwrap_or("");
                        let synth_elem = InfosetElement::simple(
                            QName::local(clean_name),
                            ElementState::Value(ovc_val),
                        );
                        Some(calc_elem_value_length(
                            &synth_elem,
                            ctx.schema,
                            &norm,
                            Some(ctx),
                        ))
                    } else {
                        None
                    };

                    if let Some(byte_len) = computed_len {
                        let res_len = match units.to_lowercase().as_str() {
                            "bits" => (byte_len.saturating_mul(8)) as i64,
                            _ => byte_len as i64,
                        };
                        return Ok(DfdlValue::Long(res_len));
                    }
                }
            }
            if name.local_name == "checkConstraints" {
                let norm = if let Some(ExprAst::Path(path)) = args.first() {
                    path.normalized_against(ctx.current_path)
                } else {
                    ctx.current_path.clone()
                };
                if let Some(doc) = ctx.doc {
                    if let Some(elem) = doc.find_element_with_context(&norm, ctx.occurs_index) {
                        if matches!(elem.state, ElementState::Nil) {
                            return Ok(DfdlValue::Boolean(true));
                        }
                    }
                }
                if let Some(ExprAst::Path(path)) = args.first() {
                    if path.is_self_only() && ctx.doc.is_none() {
                        return Ok(DfdlValue::Boolean(true));
                    }
                }
            }
            ctx.depth = ctx.depth.saturating_add(1);
            let mut evaled_args = Vec::new();
            for arg in args {
                let v = match eval_expr(arg, ctx) {
                    Ok(val) => val,
                    Err(e) => {
                        ctx.depth = ctx.depth.saturating_sub(1);
                        return Err(e);
                    }
                };
                evaled_args.push(v);
            }
            ctx.depth = ctx.depth.saturating_sub(1);
            eval_fn_call(name, &evaled_args, ctx)
        }
    }
}

fn eval_prop_str_with_ctx(
    raw: &str,
    ctx: Option<&ExprContext>,
    current_path: &InfosetPath,
) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
        let expr_content = &trimmed[1..trimmed.len().saturating_sub(1)];
        if let Some(c) = ctx {
            if let Ok(ast) = crate::expr::parse_expr(expr_content) {
                let mut local_budget = crate::limits::WorkBudget::new(100_000);
                let mut sub_ctx = crate::expr::ExprContext::with_variable_map(
                    c.doc,
                    current_path,
                    &[],
                    c.variable_map,
                    &mut local_budget,
                );
                if let Some(sch) = c.schema {
                    sub_ctx = sub_ctx.with_schema(sch);
                }
                if let Ok(val) = crate::expr::eval_expr(&ast, &mut sub_ctx) {
                    return Some(alloc::format!("{}", val));
                }
            }
        }
        None
    } else if let Some(stripped) = trimmed.strip_prefix("{{") {
        Some(alloc::format!("{{{stripped}"))
    } else {
        Some(String::from(raw))
    }
}

fn lookup_variable(name: &QName, ctx: &ExprContext) -> DFDLResult<DfdlValue> {
    for (var_name, val) in ctx.variables {
        if *var_name == name.local_name {
            return Ok(val.clone());
        }
    }
    if let Some(vmap) = ctx.variable_map {
        match vmap.get_variable_validated(&name.local_name, ctx.is_parsing) {
            Ok(val) => return Ok(val),
            Err(e) if !e.message.to_string().contains("Undefined DFDL variable") => {
                return Err(e);
            }
            _ => {}
        }
    }
    // DFDL 1.0 §7.4 Pre-defined standard variables
    match name.local_name.as_str() {
        "encoding" => return Ok(DfdlValue::String(String::from("UTF-8"))),
        "byteOrder" => return Ok(DfdlValue::String(String::from("bigEndian"))),
        "binaryFloatRep" => return Ok(DfdlValue::String(String::from("ieee"))),
        "outputNewLine" => return Ok(DfdlValue::String(String::from("\n"))),
        _ => {}
    }
    let msg = format!("Undefined expression variable: '${}'", name.local_name);
    Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
}
