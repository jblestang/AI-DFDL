//! XPath path resolution and infoset navigation.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::state::ElementState;
use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use crate::infoset::value::DfdlValue;
use crate::limits::WorkBudget;
use crate::types::InfosetPath;
use super::context::ExprContext;
use super::eval_prop_str_with_ctx;
use super::eval_expr;

pub(crate) fn calc_elem_value_length(
    elem: &InfosetElement,
    schema: Option<&crate::schema::ir::CompiledSchema>,
    current_path: &InfosetPath,
    ctx: Option<&ExprContext>,
) -> usize {
    match &elem.state {
        ElementState::Value(v) => {
            if let Some(sch) = schema {
                if let Some(term) = sch.find_term_by_path(current_path).or_else(|| {
                    sch.find_term_by_name(&elem.name.local_name)
                        .and_then(|id| sch.get_term(id))
                }) {
                    let coerced_val;
                    let v = if let crate::schema::ir::TermKind::Element(ref el) = term.kind {
                        if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                            coerced_val =
                                crate::kernel::parser::element::coerce_and_validate_ivc_value(
                                    v,
                                    st,
                                    &term.properties,
                                )
                                .unwrap_or_else(|_| v.clone());
                            &coerced_val
                        } else {
                            v
                        }
                    } else {
                        v
                    };
                    if let crate::schema::ir::TermKind::Element(ref el) = term.kind {
                        if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                            if let DfdlValue::HexBinary(b) = v {
                                return b.len();
                            }
                            if *st == crate::infoset::DfdlSimpleType::String {
                                let s = alloc::format!("{}", v);
                                if let Some(ref scheme) = term.properties.escape_scheme {
                                    let eval_ec =
                                        scheme.escape_character.as_deref().and_then(|raw| {
                                            eval_prop_str_with_ctx(raw, ctx, current_path)
                                        });
                                    let eval_eec =
                                        scheme.escape_escape_character.as_deref().and_then(|raw| {
                                            eval_prop_str_with_ctx(raw, ctx, current_path)
                                        });
                                    let eval_bs =
                                        scheme.escape_block_start.as_deref().and_then(|raw| {
                                            eval_prop_str_with_ctx(raw, ctx, current_path)
                                        });
                                    let eval_be =
                                        scheme.escape_block_end.as_deref().and_then(|raw| {
                                            eval_prop_str_with_ctx(raw, ctx, current_path)
                                        });
                                    let mut delims = Vec::new();
                                    if let Some(ref t) = term.properties.terminator {
                                        delims.push(t.as_str());
                                    }
                                    if let Some(ref s) = term.properties.separator {
                                        delims.push(s.as_str());
                                    }
                                    if let Some(ref i) = term.properties.initiator {
                                        delims.push(i.as_str());
                                    }
                                    return scheme
                                        .escape_text(
                                            &s,
                                            eval_ec.as_deref(),
                                            eval_eec.as_deref(),
                                            eval_bs.as_deref(),
                                            eval_be.as_deref(),
                                            &delims,
                                        )
                                        .len();
                                }
                                return s.len();
                            }
                            if *st == crate::infoset::DfdlSimpleType::HexBinary {
                                return match v {
                                    DfdlValue::HexBinary(b) => b.len(),
                                    _ => {
                                        let s = alloc::format!("{}", v);
                                        s.trim().len() / 2
                                    }
                                };
                            }
                        }
                    }
                    if term.properties.representation == crate::schema::ir::Representation::Text {
                        let is_numeric = matches!(
                            v,
                            DfdlValue::Float(_)
                                | DfdlValue::Double(_)
                                | DfdlValue::Decimal(_)
                                | DfdlValue::Int(_)
                                | DfdlValue::Long(_)
                                | DfdlValue::Short(_)
                                | DfdlValue::Byte(_)
                                | DfdlValue::UnsignedInt(_)
                                | DfdlValue::UnsignedLong(_)
                                | DfdlValue::UnsignedShort(_)
                                | DfdlValue::UnsignedByte(_)
                        );
                        if is_numeric {
                            let zero_rep_opt = if crate::kernel::parser::numbers::is_numeric_zero(v)
                            {
                                term.properties.text_standard_zero_rep.as_deref().and_then(|raw_zero| {
                                    let eval_rep = eval_prop_str_with_ctx(raw_zero, ctx, current_path)
                                        .unwrap_or_else(|| raw_zero.to_string());
                                    eval_rep.split_whitespace().next().and_then(|first| {
                                        if !first.is_empty() {
                                            Some(crate::expr::properties::decode_dfdl_character_entities(first))
                                        } else {
                                            None
                                        }
                                    })
                                })
                            } else {
                                None
                            };
                            if let Some(z) = zero_rep_opt {
                                return z.len();
                            }
                            let dec = eval_prop_str_with_ctx(
                                &term.properties.text_standard_decimal_separator,
                                ctx,
                                current_path,
                            )
                            .unwrap_or_else(|| {
                                term.properties.text_standard_decimal_separator.clone()
                            });
                            let grp = eval_prop_str_with_ctx(
                                &term.properties.text_standard_grouping_separator,
                                ctx,
                                current_path,
                            )
                            .unwrap_or_else(|| {
                                term.properties.text_standard_grouping_separator.clone()
                            });
                            let exp =
                                term.properties
                                    .text_standard_exponent_rep
                                    .as_deref()
                                    .map(|s| {
                                        eval_prop_str_with_ctx(s, ctx, current_path)
                                            .unwrap_or_else(|| s.to_string())
                                    });
                            let s = crate::kernel::parser::numbers::format_text_number(
                                v,
                                term.properties.text_number_pattern.as_deref(),
                                &dec,
                                &grp,
                                exp.as_deref(),
                                crate::kernel::parser::rounding::NumberRounding::from_props(
                                    &term.properties,
                                ),
                            );
                            return s.len();
                        }
                        // Calendar types (Date, Time, DateTime) formatted with text representation:
                        // apply calendarPattern, calendarLanguage, and calendarTimeZone to get accurate byte length.
                        let is_calendar = matches!(
                            v,
                            DfdlValue::Date(_) | DfdlValue::Time(_) | DfdlValue::DateTime(_)
                        );
                        if is_calendar {
                            if let Some(ref pat) = term.properties.calendar_pattern {
                                let s_val = alloc::format!("{}", v);
                                let eval_lang = term
                                    .properties
                                    .calendar_language
                                    .as_deref()
                                    .and_then(|raw| eval_prop_str_with_ctx(raw, ctx, current_path));
                                let eval_tz = term
                                    .properties
                                    .calendar_time_zone
                                    .as_deref()
                                    .and_then(|raw| eval_prop_str_with_ctx(raw, ctx, current_path));
                                if let Ok(formatted) =
                                    crate::kernel::parser::calendar::format_calendar_with_pattern(
                                        &s_val,
                                        pat,
                                        eval_lang.as_deref(),
                                        eval_tz.as_deref(),
                                    )
                                {
                                    return formatted.len();
                                }
                            }
                        }
                        return alloc::format!("{}", v).len();
                    }
                }
            }
            match v {
                DfdlValue::String(s) => s.len(),
                DfdlValue::HexBinary(b) => b.len(),
                DfdlValue::DateTime(s)
                | DfdlValue::Date(s)
                | DfdlValue::Time(s)
                | DfdlValue::Decimal(s) => s.len(),
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) | DfdlValue::Float(_) => 4,
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) | DfdlValue::Double(_) => 8,
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => 2,
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) | DfdlValue::Boolean(_) => 1,
            }
        }
        _ => {
            let mut total = 0usize;
            for child in &elem.children {
                match child {
                    InfosetNode::Element(sub) => {
                        let mut sub_path = current_path.clone();
                        let _ = sub_path.try_push(&sub.name.local_name);
                        let sub_len = calc_elem_value_length(sub, schema, &sub_path, ctx);
                        total = total.saturating_add(sub_len);
                        if let Some(sch) = schema {
                            if let Some(term) = sch.find_term_by_path(&sub_path) {
                                if term.properties.length_kind
                                    == crate::schema::ir::LengthKind::Prefixed
                                {
                                    let prefix_bytes = term
                                        .properties
                                        .prefix_length_type
                                        .as_ref()
                                        .map(|d| d.prefix_bytes())
                                        .unwrap_or(2);
                                    total = total.saturating_add(prefix_bytes);
                                }
                                if let Some(ref term_str) = term.properties.terminator {
                                    total = total.saturating_add(term_str.len());
                                }
                                if let Some(ref init_str) = term.properties.initiator {
                                    total = total.saturating_add(init_str.len());
                                }
                            }
                        }
                    }
                }
            }
            total
        }
    }
}

pub(crate) fn strip_path_namespaces(path: &InfosetPath) -> alloc::string::String {
    let mut s = alloc::string::String::new();
    for step in path.steps() {
        s.push('/');
        s.push_str(step.local_name());
        if let Some(idx) = step.index_predicate {
            let _ = core::fmt::write(&mut s, format_args!("[{}]", idx));
        } else if let Some(ref pred) = step.predicate_expr {
            let _ = core::fmt::write(&mut s, format_args!("[{}]", pred));
        }
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// Evaluates the `dfdl:outputValueCalc` of the schema element addressed by `norm`, if any.
pub(crate) fn eval_ovc_for_path(ctx: &ExprContext, norm: &InfosetPath) -> Option<DfdlValue> {
    if let Some(ovc_map) = ctx.ovc_values {
        let norm_str = alloc::format!("{}", norm);
        let stripped_norm = strip_path_namespaces(norm);
        if let Some(val) = ovc_map
            .get(&norm_str)
            .or_else(|| ovc_map.get(&stripped_norm))
        {
            return Some(val.clone());
        }
    }
    let schema = ctx.schema?;
    let clean_seg = norm.last_step()?.local_name();
    let term = schema.get_term(schema.find_term_by_name(clean_seg)?)?;
    let ast_cow = if let Some(ref ovc_prop) = term.properties.output_value_calc_prop {
        match ovc_prop {
            crate::schema::ir::DfdlProp::Constant(c) => {
                Some(alloc::borrow::Cow::Owned(crate::expr::ast::ExprAst::Literal(
                    crate::infoset::value::DfdlValue::String(c.clone()),
                )))
            }
            crate::schema::ir::DfdlProp::Expression { ast, .. } => {
                Some(alloc::borrow::Cow::Borrowed(ast.as_ref()))
            }
        }
    } else if let Some(ref ovc_expr) = term.properties.output_value_calc {
        crate::expr::parse_expr(ovc_expr).ok().map(alloc::borrow::Cow::Owned)
    } else {
        None
    };
    let ast = ast_cow?;
    let mut sub_budget = WorkBudget::new(100_000);
    let namespaces = if !term.properties.in_scope_namespaces.is_empty() {
        &term.properties.in_scope_namespaces
    } else {
        ctx.in_scope_namespaces
    };
    let mut sub_ctx = ExprContext::with_variable_map(
        ctx.doc,
        norm,
        ctx.variables,
        ctx.variable_map,
        &mut sub_budget,
    )
    .with_occurs_index(ctx.occurs_index)
    .with_schema(schema)
    .with_namespaces(namespaces);
    if let Some(ovc_map) = ctx.ovc_values {
        sub_ctx = sub_ctx.with_ovc_values(Some(ovc_map));
    }
    if !ctx.is_parsing {
        sub_ctx = sub_ctx.for_unparsing();
    }
    eval_expr(&ast, &mut sub_ctx).ok()
}

pub(crate) fn normalize_infoset_path(
    path: &InfosetPath,
    ctx: &ExprContext,
    doc: &InfosetDocument,
) -> DFDLResult<InfosetPath> {
    let mut norm = if path.is_absolute() {
        InfosetPath::root()
    } else {
        ctx.current_path.clone()
    };

    for step in path.steps() {
        if step.is_self() {
            if step.index_predicate.is_some() || step.predicate_expr.is_some() {
                let msg = alloc::format!(
                    "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                    step.to_segment_string()
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let clean_expected = step.local_name();
            if clean_expected != "." && clean_expected != ".." && !clean_expected.is_empty() {
                let clean_curr_head = norm.last_step().map(|s| s.local_name()).unwrap_or("");

                if !clean_curr_head.is_empty() && clean_curr_head != clean_expected {
                    let msg = alloc::format!(
                        "Schema Definition Error: self::{} does not match current element '{}'",
                        step.raw_target(),
                        clean_curr_head
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
            continue;
        } else if step.is_parent() {
            if step.index_predicate.is_some() || step.predicate_expr.is_some() {
                let msg = alloc::format!(
                    "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                    step.to_segment_string()
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if norm.steps().len() <= 1 {
                let msg = alloc::format!(
                    "Schema Definition Error: Relative path step '..' navigates past root element in path '{}'",
                    path
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            norm.pop();
            let clean_expected = step.local_name();
            if clean_expected != "." && clean_expected != ".." && !clean_expected.is_empty() {
                let clean_curr_head = norm.last_step().map(|s| s.local_name()).unwrap_or("");

                if clean_curr_head != clean_expected {
                    let mut found_sibling = false;
                    if let Some(parent_elem) = doc.find_element(&norm) {
                        for child in &parent_elem.children {
                            let crate::infoset::tree::InfosetNode::Element(ref el) = child;
                            if el.name.local_name.as_str() == clean_expected {
                                found_sibling = true;
                                break;
                            }
                        }
                    }
                    if found_sibling {
                        let _ = norm.try_push(clean_expected);
                    } else if let Some(sch) = ctx.schema {
                        if sch.find_term_by_name(clean_expected).is_some() {
                            let _ = norm.try_push(clean_expected);
                        } else {
                            let msg = alloc::format!(
                                "Schema Definition Error: parent::{} does not match parent element '{}'",
                                step.raw_target(), clean_curr_head
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    } else {
                        let msg = alloc::format!(
                            "Schema Definition Error: parent::{} does not match parent element '{}'",
                            step.raw_target(), clean_curr_head
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        } else {
            let clean_head = step.local_name();
            let pred_opt = step.predicate_expr.as_deref();
            if let Some(pred_str) = pred_opt {
                if clean_head == "." || clean_head == ".." {
                    let msg = alloc::format!(
                        "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                        step.to_segment_string()
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if let Some(schema) = ctx.schema {
                    let mut candidate_path = norm.clone();
                    let _ = candidate_path.try_push(clean_head);
                    let term_opt = schema.find_term_by_path(&candidate_path).or_else(|| {
                        schema
                            .find_term_by_name(clean_head)
                            .and_then(|id| schema.get_term(id))
                    });
                    if let Some(term) = term_opt {
                        if let crate::schema::ir::TermKind::Element(ref elem) = term.kind {
                            // DFDL §16.1.4: When dfdl:occursCountKind is 'parsed', occurrences are determined
                            // dynamically by parsing until a processing error occurs; thus the element is treated
                            // as an unbounded array regardless of XSD maxOccurs.
                            let is_array = elem.max_occurs.is_none()
                                || elem.max_occurs > Some(1)
                                || term.properties.occurs_count_kind
                                    == crate::schema::ir::OccursCountKind::Parsed
                                || term.properties.occurs_count_kind
                                    == crate::schema::ir::OccursCountKind::Expression
                                || term.properties.occurs_count_kind
                                    == crate::schema::ir::OccursCountKind::StopValue;
                            if !is_array {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}[{}]' for non-array element '{}'",
                                    clean_head,
                                    pred_str,
                                    clean_head
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }
            }
            if let Some(idx) = step.index_predicate {
                let mut new_step = step.clone();
                new_step.index_predicate = Some(idx);
                let _ = norm.try_push_step(new_step);
            } else if let Some(pred_str) = pred_opt {
                let evaluated_index = if let Ok(ast) = crate::expr::parse_expr(pred_str) {
                    let mut pred_budget = WorkBudget::new(100_000);
                    let mut sub_ctx = ExprContext::with_variable_map(
                        ctx.doc,
                        ctx.current_path,
                        ctx.variables,
                        ctx.variable_map,
                        &mut pred_budget,
                    )
                    .with_occurs_index(ctx.occurs_index);
                    if let Some(schema) = ctx.schema {
                        sub_ctx = sub_ctx.with_schema(schema);
                    }
                    if let Ok(val) = eval_expr(&ast, &mut sub_ctx) {
                        match val {
                            DfdlValue::Int(i) if i > 0 => Some(i as usize),
                            DfdlValue::Long(i) if i > 0 => Some(i as usize),
                            DfdlValue::Short(i) if i > 0 => Some(i as usize),
                            DfdlValue::Byte(i) if i > 0 => Some(i as usize),
                            DfdlValue::UnsignedInt(u) if u > 0 => Some(u as usize),
                            DfdlValue::UnsignedLong(u) if u > 0 => Some(u as usize),
                            DfdlValue::UnsignedShort(u) if u > 0 => Some(u as usize),
                            DfdlValue::UnsignedByte(u) if u > 0 => Some(u as usize),
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut new_step = step.clone();
                if let Some(idx) = evaluated_index {
                    new_step.index_predicate = Some(idx);
                    new_step.predicate_expr = None;
                }
                let _ = norm.try_push_step(new_step);
            } else {
                let _ = norm.try_push_step(step.clone());
            }
        }
    }

    Ok(norm)
}

pub(crate) fn path_node_exists(path: &InfosetPath, ctx: &ExprContext) -> DFDLResult<bool> {
    let doc = match ctx.doc {
        Some(d) => d,
        None => return Ok(false),
    };
    let norm = match normalize_infoset_path(path, ctx, doc) {
        Ok(n) => n,
        Err(e) if e.kind == DFDLErrorKind::SchemaDefinition => return Err(e),
        Err(_) => return Ok(false),
    };
    let is_self_ref = norm == *ctx.current_path
        || (norm.last_step().is_some() && norm.last_step() == ctx.current_path.last_step());

    match doc.find_element_with_policy_checked(
        &norm,
        ctx.occurs_index,
        is_self_ref,
        ctx.unqualified_path_step_policy,
        ctx.in_scope_namespaces,
    ) {
        Ok(Some(elem)) => Ok(!matches!(elem.state, ElementState::Absent)),
        Ok(None) => Ok(false),
        Err(e) if e.kind == DFDLErrorKind::SchemaDefinition => Err(e),
        Err(_) => Ok(false),
    }
}

/// Resolves an [`InfosetPath`] expression against the current [`ExprContext`].
pub fn resolve_path(path: &InfosetPath, ctx: &ExprContext) -> DFDLResult<DfdlValue> {
    let doc = match ctx.doc {
        Some(d) => d,
        None => {
            let msg = format!(
                "No Infoset document provided for path resolution: '{}'",
                path
            );
            return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
        }
    };

    let norm = normalize_infoset_path(path, ctx, doc)?;

    if let Some(ovc_map) = ctx.ovc_values {
        let norm_str = alloc::format!("{}", norm);
        let stripped_norm = strip_path_namespaces(&norm);
        if let Some(val) = ovc_map
            .get(&norm_str)
            .or_else(|| ovc_map.get(&stripped_norm))
        {
            return Ok(val.clone());
        }
    }

    let is_self_ref = norm == *ctx.current_path
        || (norm.last_step().is_some() && norm.last_step() == ctx.current_path.last_step());

    let target_elem = match doc.find_element_with_policy_checked(
        &norm,
        ctx.occurs_index,
        is_self_ref,
        ctx.unqualified_path_step_policy,
        ctx.in_scope_namespaces,
    )? {
        Some(elem) => elem,
        None => {
            if is_self_ref {
                let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
                let msg = alloc::format!(
                    "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if let Some(val) = eval_ovc_for_path(ctx, &norm) {
                return Ok(val);
            }
            let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
            let qname = match norm.last_step().map(|s| &s.target) {
                Some(crate::types::StepTarget::Prefixed { raw, .. }) => raw.clone(),
                Some(crate::types::StepTarget::Clark { raw, .. }) => raw.clone(),
                Some(crate::types::StepTarget::Unprefixed(name)) => alloc::format!("{{}}{}", name),
                _ => alloc::format!("{{}}{}", elem_name),
            };
            let msg = format!(
                "Runtime Schema Definition Error: Expression Evaluation Error in '{}': element '{}' does not exist in Infoset document (Path not found in Infoset document: '{}')",
                ctx.current_path, qname, path
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
    };

    match &target_elem.state {
        ElementState::Value(val) => {
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
            Ok(val.clone())
        }
        ElementState::Empty => {
            Ok(eval_ovc_for_path(ctx, &norm).unwrap_or_else(|| DfdlValue::String(String::new())))
        }
        ElementState::NoValue | ElementState::Absent => {
            let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
            if is_self_ref
                || (norm.steps().len() < ctx.current_path.steps().len()
                    && ctx.current_path.steps().starts_with(norm.steps()))
            {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Circular reference: element '{}' refers to itself or an enclosing ancestor '{}'",
                    ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or(""),
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if let Some(val) = eval_ovc_for_path(ctx, &norm) {
                return Ok(val);
            }
            if !target_elem.children.is_empty() {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Complex element '{}' does not have a simple value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            let msg = alloc::format!(
                "Expression Evaluation Error: Element '{}' does not have a value",
                elem_name
            );
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
        _ => {
            let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
            if is_self_ref
                || (norm.steps().len() < ctx.current_path.steps().len()
                    && ctx.current_path.steps().starts_with(norm.steps()))
            {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Circular reference: element '{}' refers to itself or an enclosing ancestor '{}'",
                    ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or(""),
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if let Some(val) = eval_ovc_for_path(ctx, &norm) {
                return Ok(val);
            }
            if !target_elem.children.is_empty() {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Complex element '{}' does not have a simple value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            let msg = alloc::format!(
                "Expression Evaluation Error: Element '{}' does not have a value",
                elem_name
            );
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
    }
}
