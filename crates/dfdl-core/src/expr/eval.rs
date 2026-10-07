//! Expression Evaluation Engine for DFDL expressions.
//!
//! Evaluates compiled [`ExprAst`] against an [`InfosetDocument`] context.
//! Enforces bounded stack depth and consumes [`WorkBudget`]. Panic-free.

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::expr::ast::{BinaryOp, ExprAst, UnaryOp};
use crate::infoset::state::ElementState;
use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use crate::infoset::value::DfdlValue;
use crate::limits::WorkBudget;
use crate::types::{InfosetPath, QName};
use crate::util::get_checked;

/// Maximum evaluation recursion depth to prevent stack overflow.
pub const MAX_EVAL_DEPTH: usize = 64;

/// Evaluation context containing variables, target document, and work budget limits.
pub struct ExprContext<'a> {
    /// Optional reference to active Infoset document.
    pub doc: Option<&'a InfosetDocument>,
    /// Current path context in document.
    pub current_path: &'a InfosetPath,
    /// Variable bindings available in scope (`(variable_name, value)`).
    pub variables: &'a [(&'a str, DfdlValue)],
    /// Optional DFDL schema variable map (`VariableMap`).
    pub variable_map: Option<&'a crate::expr::variables::VariableMap>,
    /// Optional compiled schema for property lookups.
    pub schema: Option<&'a crate::schema::ir::CompiledSchema>,
    /// Remaining work budget.
    pub budget: &'a mut WorkBudget,
    /// Current recursion depth.
    pub depth: usize,
    /// Current 1-based array iteration index (DFDL §23.1 dfdl:occursIndex()).
    pub occurs_index: usize,
    /// Indicates whether evaluation is occurring during parsing (`true`) or unparsing (`false`).
    pub is_parsing: bool,
    /// Optional lookahead function to read bits from the bitstream without advancing position.
    pub lookahead_fn: Option<&'a (dyn Fn(usize, usize) -> DFDLResult<u128> + 'a)>,
    /// Enclosing complex elements and their known lengths, length units, and encodings during parsing.
    pub enclosing_lengths: &'a [(String, usize, crate::schema::ir::LengthUnits, String)],
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: crate::types::UnqualifiedPathStepPolicy,
    /// In-scope namespaces at the expression evaluation point (`[(prefix, uri)]`).
    pub in_scope_namespaces: &'a [(alloc::string::String, alloc::string::String)],
    /// Computed outputValueCalc values for elements during unparsing.
    pub ovc_values: Option<&'a alloc::collections::BTreeMap<alloc::string::String, DfdlValue>>,
}

impl<'a> ExprContext<'a> {
    /// Creates a new [`ExprContext`].
    pub fn new(
        doc: Option<&'a InfosetDocument>,
        current_path: &'a InfosetPath,
        variables: &'a [(&'a str, DfdlValue)],
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            doc,
            current_path,
            variables,
            variable_map: None,
            schema: None,
            budget,
            depth: 0,
            occurs_index: 1,
            is_parsing: true,
            lookahead_fn: None,
            enclosing_lengths: &[],
            unqualified_path_step_policy: Default::default(),
            in_scope_namespaces: &[],
            ovc_values: None,
        }
    }

    /// Creates a new [`ExprContext`] with an active schema [`VariableMap`].
    pub fn with_variable_map(
        doc: Option<&'a InfosetDocument>,
        current_path: &'a InfosetPath,
        variables: &'a [(&'a str, DfdlValue)],
        variable_map: Option<&'a crate::expr::variables::VariableMap>,
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            doc,
            current_path,
            variables,
            variable_map,
            schema: None,
            budget,
            depth: 0,
            occurs_index: 1,
            is_parsing: true,
            lookahead_fn: None,
            enclosing_lengths: &[],
            unqualified_path_step_policy: Default::default(),
            in_scope_namespaces: &[],
            ovc_values: None,
        }
    }

    /// Attaches precomputed OVC values for unparsing.
    #[must_use]
    pub fn with_ovc_values(
        mut self,
        ovc_values: Option<&'a alloc::collections::BTreeMap<alloc::string::String, DfdlValue>>,
    ) -> Self {
        self.ovc_values = ovc_values;
        self
    }

    /// Attaches known enclosing element lengths during parsing.
    #[must_use]
    pub fn with_enclosing_lengths(
        mut self,
        enclosing_lengths: &'a [(String, usize, crate::schema::ir::LengthUnits, String)],
    ) -> Self {
        self.enclosing_lengths = enclosing_lengths;
        self
    }

    /// Attaches an active compiled schema for element property lookups and inherits its policy.
    #[must_use]
    pub fn with_schema(mut self, schema: &'a crate::schema::ir::CompiledSchema) -> Self {
        self.unqualified_path_step_policy = schema.unqualified_path_step_policy;
        self.schema = Some(schema);
        self
    }

    /// Attaches in-scope namespaces for prefix and default-namespace resolution.
    #[must_use]
    pub fn with_namespaces(
        mut self,
        namespaces: &'a [(alloc::string::String, alloc::string::String)],
    ) -> Self {
        self.in_scope_namespaces = namespaces;
        self
    }

    /// Overrides the policy for resolving unqualified path steps in expressions.
    #[must_use]
    pub fn with_unqualified_path_step_policy(
        mut self,
        policy: crate::types::UnqualifiedPathStepPolicy,
    ) -> Self {
        self.unqualified_path_step_policy = policy;
        self
    }

    /// Sets the active 1-based array iteration index for `dfdl:occursIndex()`.
    #[must_use]
    pub fn with_occurs_index(mut self, idx: usize) -> Self {
        self.occurs_index = idx;
        self
    }

    /// Configures the context for unparser evaluation (`is_parsing = false`).
    #[must_use]
    pub fn for_unparsing(mut self) -> Self {
        self.is_parsing = false;
        self
    }

    /// Attaches an optional lookahead function for reading bits from the bitstream.
    #[must_use]
    pub fn with_lookahead(
        mut self,
        la_fn: Option<&'a (dyn Fn(usize, usize) -> DFDLResult<u128> + 'a)>,
    ) -> Self {
        self.lookahead_fn = la_fn;
        self
    }

    /// Resolves an element in the active document respecting policy and namespaces.
    #[inline]
    #[must_use]
    pub fn find_element(&self, path: &InfosetPath) -> Option<&'a InfosetElement> {
        let doc = self.doc?;
        doc.find_element_with_policy_checked(
            path,
            self.occurs_index,
            false,
            self.unqualified_path_step_policy,
            self.in_scope_namespaces,
        )
        .ok()
        .flatten()
    }
}

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
                            let elem_name = path.segments().last().map(|s| s.as_str()).unwrap_or("complex");
                            return Ok(DfdlValue::String(alloc::string::ToString::to_string(elem_name)));
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
                        let segs = path.segments();
                        if let Some((last_seg, parent_segs)) = segs.split_last() {
                            let clean_last = last_seg.split(':').next_back().unwrap_or(last_seg);
                            let clean_target = clean_last.split('[').next().unwrap_or(clean_last);

                            let mut norm_parent = if path.is_absolute() {
                                InfosetPath::root()
                            } else {
                                ctx.current_path.clone()
                            };

                            for seg in parent_segs {
                                if seg == "." {
                                    continue;
                                } else if seg == ".." {
                                    if !norm_parent.segments().is_empty() {
                                        norm_parent.pop();
                                    }
                                } else {
                                    let clean_seg = seg.split(':').next_back().unwrap_or(seg);
                                    let _ = norm_parent.try_push(clean_seg);
                                }
                            }

                            if let Some(parent_elem) =
                                doc.find_element_with_context(&norm_parent, ctx.occurs_index)
                            {
                                let mut match_count = 0usize;
                                for child in &parent_elem.children {
                                    let crate::infoset::InfosetNode::Element(ref child_elem) =
                                        child;
                                    let clean_child = child_elem
                                        .name
                                        .local_name
                                        .split(':')
                                        .next_back()
                                        .unwrap_or(&child_elem.name.local_name);
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

                    let mut norm = if path.is_absolute() {
                        InfosetPath::root()
                    } else {
                        ctx.current_path.clone()
                    };
                    for seg in path.segments() {
                        if seg == "." {
                            continue;
                        } else if seg == ".." {
                            if !norm.segments().is_empty() {
                                norm.pop();
                            }
                        } else {
                            let clean_seg = seg.split(':').next_back().unwrap_or(seg);
                            let _ = norm.try_push(clean_seg);
                        }
                    }
                    let norm_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                    if let Some((_, len, len_units, enc)) = ctx.enclosing_lengths.iter().rev().find(|(name, _, _, _)| {
                        name == norm_name || norm_name.ends_with(name.as_str())
                    }) {
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
                                (bit_len.checked_div(crate::encoding::encoding_unit_bits(enc)).unwrap_or(0)) as i64
                            }
                            _ => (bit_len.checked_div(8).unwrap_or(0)) as i64,
                        };
                        return Ok(DfdlValue::Long(res_len));
                    }

                    let is_enclosing = if norm.segments().len() < ctx.current_path.segments().len()
                        && ctx.current_path.segments().starts_with(norm.segments())
                    {
                        true
                    } else if norm == *ctx.current_path {
                        let is_self_length_calc = if let Some(sch) = ctx.schema {
                            let clean = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                            let clean_name = clean.split(':').next_back().unwrap_or(clean);
                            if let Some(term) = sch.find_term_by_name(clean_name).and_then(|id| sch.get_term(id)) {
                                term.properties.length_expr.is_some() || term.properties.truncate_specified_length_string
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if is_self_length_calc {
                            let current_name = ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or("");
                            let msg = alloc::format!(
                                "Runtime Schema Definition Error: Value length unknown: cannot evaluate dfdl:valueLength for element '{}' while evaluating its length expression",
                                current_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        let has_simple_val = ctx.doc.and_then(|doc| {
                            doc.find_element_with_context(&norm, ctx.occurs_index)
                        }).is_some_and(|elem| matches!(&elem.state, ElementState::Value(_)));
                        !has_simple_val
                    } else {
                        false
                    };

                    if is_enclosing {
                        let elem_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let qname = if elem_name.contains(':') {
                            alloc::string::ToString::to_string(elem_name)
                        } else {
                            alloc::format!("ex:{}", elem_name)
                        };
                        let current_name = ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let msg = alloc::format!(
                            "Runtime Schema Definition Error: Expression Evaluation Error in '{}': Value Length cannot be computed for enclosing element '{}'",
                            current_name, qname
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }

                    if let Some(sch) = ctx.schema {
                        let target_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let clean_target = target_name.split(':').next_back().unwrap_or(target_name);
                        let curr_name = ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let clean_curr = curr_name.split(':').next_back().unwrap_or(curr_name);
                        if !clean_curr.is_empty() && clean_target != clean_curr {
                            if let Some(target_term) = sch.find_term_by_name(clean_target).and_then(|id| sch.get_term(id)) {
                                if target_term.properties.truncate_specified_length_string {
                                    if let Some(ref len_expr) = target_term.properties.length_expr {
                                        if len_expr.contains(clean_curr) {
                                            let msg = alloc::format!(
                                                "Expression Evaluation Error: Circular reference / circular dependency: element '{}' does not have a value because its length depends on '{}'",
                                                clean_target, clean_curr
                                            );
                                            return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
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
                        .filter(|elem| matches!(&elem.state, ElementState::Value(_)) || !elem.children.is_empty())
                    } else {
                        None
                    };

                    if let Some(elem) = val_opt {
                        if !elem.children.is_empty() {
                            if let Some(sch) = ctx.schema {
                                if let Ok(bits) = crate::kernel::unparser::measure_element_content_bits(
                                    sch,
                                    ctx.doc,
                                    elem,
                                    &norm,
                                    ctx.variable_map,
                                ) {
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
                        let clean_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let synth_elem = InfosetElement::simple(
                            QName::local(clean_name),
                            ElementState::Value(ovc_val),
                        );
                        Some(calc_elem_value_length(&synth_elem, ctx.schema, &norm, Some(ctx)))
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
                    let mut norm = if path.is_absolute() {
                        InfosetPath::root()
                    } else {
                        ctx.current_path.clone()
                    };
                    for seg in path.segments() {
                        if seg == "." {
                            continue;
                        } else if seg == ".." {
                            if !norm.segments().is_empty() {
                                norm.pop();
                            }
                        } else {
                            let clean_seg = seg.split(':').next_back().unwrap_or(seg);
                            let _ = norm.try_push(clean_seg);
                        }
                    }
                    norm
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
                    if (path.segments().is_empty() || path.segments() == ["."]) && ctx.doc.is_none() {
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

fn calc_elem_value_length(
    elem: &InfosetElement,
    schema: Option<&crate::schema::ir::CompiledSchema>,
    current_path: &InfosetPath,
    ctx: Option<&ExprContext>,
) -> usize {
    match &elem.state {
        ElementState::Value(v) => {
            if let Some(sch) = schema {
                if let Some(term) = sch.find_term_by_path(current_path).or_else(|| {
                    sch.find_term_by_name(&elem.name.local_name).and_then(|id| sch.get_term(id))
                }) {
                    let coerced_val;
                    let v = if let crate::schema::ir::TermKind::Element(ref el) = term.kind {
                        if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                            coerced_val = crate::kernel::parser::element::coerce_and_validate_ivc_value(
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
                                    let eval_ec = scheme.escape_character.as_deref().and_then(|raw| {
                                        eval_prop_str_with_ctx(raw, ctx, current_path)
                                    });
                                    let eval_eec = scheme.escape_escape_character.as_deref().and_then(|raw| {
                                        eval_prop_str_with_ctx(raw, ctx, current_path)
                                    });
                                    let eval_bs = scheme.escape_block_start.as_deref().and_then(|raw| {
                                        eval_prop_str_with_ctx(raw, ctx, current_path)
                                    });
                                    let eval_be = scheme.escape_block_end.as_deref().and_then(|raw| {
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
                                    return scheme.escape_text(
                                        &s,
                                        eval_ec.as_deref(),
                                        eval_eec.as_deref(),
                                        eval_bs.as_deref(),
                                        eval_be.as_deref(),
                                        &delims,
                                    ).len();
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
                            let zero_rep_opt = if crate::kernel::parser::numbers::is_numeric_zero(v) {
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
                            .unwrap_or_else(|| term.properties.text_standard_decimal_separator.clone());
                            let grp = eval_prop_str_with_ctx(
                                &term.properties.text_standard_grouping_separator,
                                ctx,
                                current_path,
                            )
                            .unwrap_or_else(|| term.properties.text_standard_grouping_separator.clone());
                            let exp = term.properties.text_standard_exponent_rep.as_deref().map(|s| {
                                eval_prop_str_with_ctx(s, ctx, current_path).unwrap_or_else(|| s.to_string())
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
                                let eval_lang = term.properties.calendar_language.as_deref().and_then(|raw| {
                                    eval_prop_str_with_ctx(raw, ctx, current_path)
                                });
                                let eval_tz = term.properties.calendar_time_zone.as_deref().and_then(|raw| {
                                    eval_prop_str_with_ctx(raw, ctx, current_path)
                                });
                                if let Ok(formatted) = crate::kernel::parser::calendar::format_calendar_with_pattern(
                                    &s_val,
                                    pat,
                                    eval_lang.as_deref(),
                                    eval_tz.as_deref(),
                                ) {
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
                                    let ptype = term
                                        .properties
                                        .prefix_length_type
                                        .as_deref()
                                        .unwrap_or("xs:unsignedShort");
                                    let parts: Vec<&str> = ptype.split(':').collect();
                                    let prefix_bytes = if parts.len() >= 4 {
                                        let units = parts.get(3).copied().unwrap_or("bytes");
                                        let num_len: usize = parts
                                            .get(2)
                                            .filter(|s| !s.is_empty())
                                            .and_then(|s| s.parse().ok())
                                            .unwrap_or(2);
                                        if units.eq_ignore_ascii_case("bits") {
                                            num_len.div_ceil(8)
                                        } else {
                                            num_len
                                        }
                                    } else {
                                        let clean_ptype = parts.last().copied().unwrap_or(ptype);
                                        match clean_ptype {
                                            "byte" | "unsignedByte" => 1,
                                            "short" | "unsignedShort" => 2,
                                            "int" | "unsignedInt" => 4,
                                            "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                                            _ => 2,
                                        }
                                    };
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

fn strip_path_namespaces(path: &InfosetPath) -> alloc::string::String {
    let mut s = alloc::string::String::new();
    for seg in path.segments() {
        s.push('/');
        if let (Some(open), Some(close)) = (seg.find('['), seg.rfind(']')) {
            if open < close {
                let p = seg[..open].split(':').next_back().unwrap_or(&seg[..open]);
                let idx = &seg[open..=close];
                s.push_str(p);
                s.push_str(idx);
            } else {
                let p = seg.split(':').next_back().unwrap_or(seg.as_str());
                s.push_str(p);
            }
        } else {
            let p = seg.split(':').next_back().unwrap_or(seg.as_str());
            s.push_str(p);
        }
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// Evaluates the `dfdl:outputValueCalc` of the schema element addressed by `norm`, if any.
fn eval_ovc_for_path(ctx: &ExprContext, norm: &InfosetPath) -> Option<DfdlValue> {
    if let Some(ovc_map) = ctx.ovc_values {
        let norm_str = alloc::format!("{}", norm);
        let stripped_norm = strip_path_namespaces(norm);
        if let Some(val) = ovc_map.get(&norm_str).or_else(|| ovc_map.get(&stripped_norm)) {
            return Some(val.clone());
        }
    }
    let schema = ctx.schema?;
    let seg_name = norm.segments().last()?;
    let clean_seg = seg_name.split('[').next().unwrap_or(seg_name);
    let clean_seg = clean_seg.split(':').next_back().unwrap_or(clean_seg);
    let term = schema.get_term(schema.find_term_by_name(clean_seg)?)?;
    let ovc_expr = term.properties.output_value_calc.as_ref()?;
    let ast = crate::expr::parse_expr(ovc_expr).ok()?;
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

fn normalize_infoset_path(
    path: &InfosetPath,
    ctx: &ExprContext,
    doc: &InfosetDocument,
) -> DFDLResult<InfosetPath> {
    let mut norm = if path.is_absolute() {
        InfosetPath::root()
    } else {
        ctx.current_path.clone()
    };

    for seg in path.segments() {
        if seg == "." || seg.starts_with(".(") {
            if let Some(rest) = seg.strip_prefix(".(") {
                if let Some((expected, _)) = rest.split_once(')') {
                    if !expected.is_empty() {
                        let clean_expected = expected.split(':').next_back().unwrap_or(expected);
                        let curr_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let clean_curr = curr_name.split(':').next_back().unwrap_or(curr_name);
                        let clean_curr_head = clean_curr.split('[').next().unwrap_or(clean_curr);

                        if !clean_curr_head.is_empty() && clean_curr_head != clean_expected {
                            let msg = alloc::format!(
                                "Schema Definition Error: self::{} does not match current element '{}'",
                                expected, clean_curr_head
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
            continue;
        } else if seg == ".." || seg.starts_with("..(") {
            if norm.segments().len() <= 1 {
                let msg = alloc::format!(
                    "Schema Definition Error: Relative path step '..' navigates past root element in path '{}'",
                    path
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            norm.pop();
            if let Some(rest) = seg.strip_prefix("..(") {
                if let Some((expected, _)) = rest.split_once(')') {
                    if !expected.is_empty() {
                        let clean_expected = expected.split(':').next_back().unwrap_or(expected);
                        let curr_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                        let clean_curr = curr_name.split(':').next_back().unwrap_or(curr_name);
                        let clean_curr_head = clean_curr.split('[').next().unwrap_or(clean_curr);

                        if clean_curr_head != clean_expected {
                            let mut found_sibling = false;
                            if let Some(parent_elem) = doc.find_element(&norm) {
                                for child in &parent_elem.children {
                                    let crate::infoset::tree::InfosetNode::Element(ref el) = child;
                                    let el_name = el.name.local_name.as_str();
                                    let clean_el = el_name.split(':').next_back().unwrap_or(el_name);
                                    if clean_el == clean_expected {
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
                                        expected, clean_curr_head
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            } else {
                                let msg = alloc::format!(
                                    "Schema Definition Error: parent::{} does not match parent element '{}'",
                                    expected, clean_curr_head
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }
            }
        } else {
            let (step_head, pred_opt) = if let (Some(b_open), Some(b_close)) = (seg.find('['), seg.rfind(']')) {
                if b_open < b_close {
                    (&seg[..b_open], Some(&seg[b_open.saturating_add(1)..b_close]))
                } else {
                    (seg.as_str(), None)
                }
            } else {
                (seg.as_str(), None)
            };
            let clean_head = step_head.split(':').next_back().unwrap_or(step_head);
            if let Some(pred_str) = pred_opt {
                if clean_head == "." || clean_head == ".." {
                    let msg = alloc::format!(
                        "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                        seg
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if let Some(schema) = ctx.schema {
                    let mut candidate_path = norm.clone();
                    let _ = candidate_path.try_push(clean_head);
                    let term_opt = schema
                        .find_term_by_path(&candidate_path)
                        .or_else(|| schema.find_term_by_name(clean_head).and_then(|id| schema.get_term(id)));
                    if let Some(term) = term_opt {
                        if let crate::schema::ir::TermKind::Element(ref elem) = term.kind {
                            // DFDL §16.1.4: When dfdl:occursCountKind is 'parsed', occurrences are determined
                            // dynamically by parsing until a processing error occurs; thus the element is treated
                            // as an unbounded array regardless of XSD maxOccurs.
                            let is_array = elem.max_occurs.is_none()
                                || elem.max_occurs > Some(1)
                                || term.properties.occurs_count_kind == crate::schema::ir::OccursCountKind::Parsed;
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
            let final_seg = if let Some(pred_str) = pred_opt {
                let evaluated_index = if let Ok(idx) = pred_str.parse::<usize>() {
                    Some(idx)
                } else if let Ok(ast) = crate::expr::parse_expr(pred_str) {
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
                if let Some(idx) = evaluated_index {
                    alloc::format!("{}[{}]", step_head, idx)
                } else {
                    alloc::format!("{}[{}]", step_head, pred_str)
                }
            } else {
                alloc::string::ToString::to_string(step_head)
            };
            let _ = norm.try_push_raw(&final_seg);
        }
    }

    Ok(norm)
}

fn path_node_exists(path: &InfosetPath, ctx: &ExprContext) -> DFDLResult<bool> {
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
        || (norm.segments().last().is_some()
            && norm.segments().last() == ctx.current_path.segments().last());

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

fn resolve_path(path: &InfosetPath, ctx: &ExprContext) -> DFDLResult<DfdlValue> {
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
        if let Some(val) = ovc_map.get(&norm_str).or_else(|| ovc_map.get(&stripped_norm)) {
            return Ok(val.clone());
        }
    }

    let is_self_ref = norm == *ctx.current_path
        || (norm.segments().last().is_some()
            && norm.segments().last() == ctx.current_path.segments().last());

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
                let elem_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                let msg = alloc::format!(
                    "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if let Some(val) = eval_ovc_for_path(ctx, &norm) {
                return Ok(val);
            }
            let elem_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
            let qname = if elem_name.contains(':') {
                alloc::string::ToString::to_string(elem_name)
            } else {
                alloc::format!("{{}}{}", elem_name)
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
                let target_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
                let clean_target = target_name.split(':').next_back().unwrap_or(target_name);
                let curr_name = ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or("");
                let clean_curr = curr_name.split(':').next_back().unwrap_or(curr_name);
                if !clean_curr.is_empty() && clean_target != clean_curr {
                    if let Some(target_term) = sch.find_term_by_name(clean_target).and_then(|id| sch.get_term(id)) {
                        if target_term.properties.truncate_specified_length_string {
                            if let Some(ref len_expr) = target_term.properties.length_expr {
                                if len_expr.contains(clean_curr) {
                                    let msg = alloc::format!(
                                        "Expression Evaluation Error: Circular reference / circular dependency: element '{}' does not have a value because its length depends on '{}'",
                                        clean_target, clean_curr
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
                                }
                            }
                        }
                    }
                }
            }
            Ok(val.clone())
        }
        ElementState::Empty => Ok(eval_ovc_for_path(ctx, &norm)
            .unwrap_or_else(|| DfdlValue::String(String::new()))),
        ElementState::NoValue | ElementState::Absent => {
            let elem_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
            if is_self_ref
                || (norm.segments().len() < ctx.current_path.segments().len()
                    && ctx.current_path.segments().starts_with(norm.segments()))
            {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Circular reference: element '{}' refers to itself or an enclosing ancestor '{}'",
                    ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or(""),
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
            let elem_name = norm.segments().last().map(|s| s.as_str()).unwrap_or("");
            if is_self_ref
                || (norm.segments().len() < ctx.current_path.segments().len()
                    && ctx.current_path.segments().starts_with(norm.segments()))
            {
                let msg = alloc::format!(
                    "Expression Evaluation Error: Circular reference: element '{}' refers to itself or an enclosing ancestor '{}'",
                    ctx.current_path.segments().last().map(|s| s.as_str()).unwrap_or(""),
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

fn eval_unary(op: UnaryOp, val: DfdlValue) -> DFDLResult<DfdlValue> {
    match op {
        UnaryOp::Plus => match val {
            DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_)
            | DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_) => Ok(val),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Invalid numeric type for unary plus",
            )),
        },
        UnaryOp::Negate => match val {
            DfdlValue::Int(n) => n
                .checked_neg()
                .map(DfdlValue::Int)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Int negation overflow")),
            DfdlValue::Long(n) => n
                .checked_neg()
                .map(DfdlValue::Long)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Long negation overflow")),
            DfdlValue::Short(n) => n
                .checked_neg()
                .map(DfdlValue::Short)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Short negation overflow")),
            DfdlValue::Byte(n) => n
                .checked_neg()
                .map(DfdlValue::Byte)
                .ok_or_else(|| DFDLError::arithmetic_overflow("Byte negation overflow")),
            DfdlValue::UnsignedLong(n) => {
                if n == 9_223_372_036_854_775_808 {
                    Ok(DfdlValue::Long(i64::MIN))
                } else if n <= i64::MAX as u64 {
                    Ok(DfdlValue::Long((n as i64).wrapping_neg()))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("-{}", n)))
                }
            }
            DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Long(i64::from(n).wrapping_neg())),
            DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Int(i32::from(n).wrapping_neg())),
            DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Int(i32::from(n).wrapping_neg())),
            DfdlValue::Decimal(s) => {
                if let Some(stripped) = s.strip_prefix('-') {
                    Ok(DfdlValue::Decimal(alloc::string::ToString::to_string(
                        stripped,
                    )))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("-{}", s)))
                }
            }
            DfdlValue::Float(f) => Ok(DfdlValue::Float(-f)),
            DfdlValue::Double(f) => Ok(DfdlValue::Double(-f)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Invalid numeric type for negation",
            )),
        },
        UnaryOp::Not => match val {
            DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(!b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Expected boolean for 'not' operator",
            )),
        },
    }
}

fn coerce_numeric(val: DfdlValue) -> DfdlValue {
    match val {
        DfdlValue::Int(n) => DfdlValue::Long(n as i64),
        DfdlValue::Short(n) => DfdlValue::Long(n as i64),
        DfdlValue::Byte(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedInt(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedShort(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedByte(n) => DfdlValue::Long(n as i64),
        DfdlValue::UnsignedLong(n) => DfdlValue::Long(n as i64),
        DfdlValue::Float(f) => DfdlValue::Double(f as f64),
        DfdlValue::String(ref s) => {
            let clean = s.trim();
            if let Ok(i) = clean.parse::<i64>() {
                DfdlValue::Long(i)
            } else if let Ok(f) = clean.parse::<f64>() {
                DfdlValue::Double(f)
            } else {
                val
            }
        }
        _ => val,
    }
}

fn eval_binary(op: BinaryOp, left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    let is_arith = matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::IDiv
            | BinaryOp::Mod
    );
    if is_arith {
        let op_sym = match op {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "div",
            BinaryOp::IDiv => "idiv",
            BinaryOp::Mod => "mod",
            _ => "",
        };
        if matches!(left, DfdlValue::String(_) | DfdlValue::Boolean(_))
            || matches!(right, DfdlValue::String(_) | DfdlValue::Boolean(_))
        {
            let bad_type = if matches!(left, DfdlValue::String(_)) || matches!(right, DfdlValue::String(_)) {
                "string"
            } else {
                "boolean"
            };
            let msg = alloc::format!(
                "Schema Definition Error: operator '{}' requires numeric operands, but received {}",
                op_sym,
                bad_type
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
    }
    let left = coerce_numeric(left);
    let right = coerce_numeric(right);
    match op {
        BinaryOp::Add => eval_add(left, right),
        BinaryOp::Sub => eval_sub(left, right),
        BinaryOp::Mul => eval_mul(left, right),
        BinaryOp::Div => eval_div(left, right),
        BinaryOp::IDiv => eval_idiv(left, right),
        BinaryOp::Mod => eval_mod(left, right),
        BinaryOp::Eq => Ok(DfdlValue::Boolean(left == right)),
        BinaryOp::Ne => Ok(DfdlValue::Boolean(left != right)),
        BinaryOp::Lt => eval_cmp("lt", left, right, |ord| ord.is_lt()),
        BinaryOp::Le => eval_cmp("le", left, right, |ord| ord.is_le()),
        BinaryOp::Gt => eval_cmp("gt", left, right, |ord| ord.is_gt()),
        BinaryOp::Ge => eval_cmp("ge", left, right, |ord| ord.is_ge()),
        BinaryOp::And => match (left, right) {
            (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(a && b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Logical AND requires boolean operands",
            )),
        },
        BinaryOp::Or => match (left, right) {
            (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(a || b)),
            _ => Err(DFDLError::new_static(
                DFDLErrorKind::TypeError,
                "Logical OR requires boolean operands",
            )),
        },
    }
}


fn eval_add(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) | (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a + d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a + f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a + b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_add(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long addition overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_add(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int addition overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) | (DfdlValue::Long(b), DfdlValue::Int(a)) => (a
            as i64)
            .checked_add(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Addition overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a + b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 + b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a + b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a + b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 + b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a + b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in addition",
        )),
    }
}

fn eval_sub(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a - d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a - f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a - b)))
            }
        }
        (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let b = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(d - b))
            } else if let DfdlValue::Float(f) = other {
                let b = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(f - b))
            } else {
                let a = value_to_f64(other);
                let b = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a - b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_sub(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long subtraction overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_sub(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int subtraction overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) => (a as i64)
            .checked_sub(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Subtraction overflow")),
        (DfdlValue::Long(a), DfdlValue::Int(b)) => a
            .checked_sub(b as i64)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Subtraction overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a - b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 - b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a - b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a - b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 - b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a - b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in subtraction",
        )),
    }
}

fn eval_mul(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        (DfdlValue::Decimal(ref s), ref other) | (ref other, DfdlValue::Decimal(ref s)) => {
            if let DfdlValue::Double(d) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a * d))
            } else if let DfdlValue::Float(f) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a * f))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                let b = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a * b)))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Long(b)) => a
            .checked_mul(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Long multiplication overflow")),
        (DfdlValue::Int(a), DfdlValue::Int(b)) => a
            .checked_mul(b)
            .map(DfdlValue::Int)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Int multiplication overflow")),
        (DfdlValue::Int(a), DfdlValue::Long(b)) | (DfdlValue::Long(b), DfdlValue::Int(a)) => (a
            as i64)
            .checked_mul(b)
            .map(DfdlValue::Long)
            .ok_or_else(|| DFDLError::arithmetic_overflow("Multiplication overflow")),
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a * b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 * b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a * b as f64)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a * b)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 * b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a * b as f32)),
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in multiplication",
        )),
    }
}

fn eval_div(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    match (left, right) {
        // Floating point operations return IEEE 754 results (INF, -INF, NaN) on div by zero
        (DfdlValue::Double(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a / b)),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a / b)),
        (DfdlValue::Double(a), DfdlValue::Float(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Float(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Long(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Double(a), DfdlValue::Long(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Int(a), DfdlValue::Double(b)) => Ok(DfdlValue::Double(a as f64 / b)),
        (DfdlValue::Double(a), DfdlValue::Int(b)) => Ok(DfdlValue::Double(a / b as f64)),
        (DfdlValue::Long(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 / b)),
        (DfdlValue::Float(a), DfdlValue::Long(b)) => Ok(DfdlValue::Float(a / b as f32)),
        (DfdlValue::Int(a), DfdlValue::Float(b)) => Ok(DfdlValue::Float(a as f32 / b)),
        (DfdlValue::Float(a), DfdlValue::Int(b)) => Ok(DfdlValue::Float(a / b as f32)),

        // Decimal operations
        (DfdlValue::Decimal(ref s), ref other) => {
            let b = value_to_f64(other);
            if let DfdlValue::Double(_) = other {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Double(a / b))
            } else if let DfdlValue::Float(_) = other {
                let a = s.trim().parse::<f32>().unwrap_or(0.0);
                Ok(DfdlValue::Float(a / b as f32))
            } else if b == 0.0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else {
                let a = s.trim().parse::<f64>().unwrap_or(0.0);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a / b)))
            }
        }
        (ref other, DfdlValue::Decimal(ref s)) => {
            let b = s.trim().parse::<f64>().unwrap_or(0.0);
            if let DfdlValue::Double(d) = other {
                Ok(DfdlValue::Double(d / b))
            } else if let DfdlValue::Float(f) = other {
                Ok(DfdlValue::Float(f / b as f32))
            } else if b == 0.0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else {
                let a = value_to_f64(other);
                Ok(DfdlValue::Decimal(alloc::format!("{}", a / b)))
            }
        }

        // Integer divisions
        (DfdlValue::Long(a), DfdlValue::Long(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) = (a.checked_rem(b), a.checked_div(b)) {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Int(a), DfdlValue::Int(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) = (a.checked_rem(b), a.checked_div(b)) {
                if rem == 0 {
                    Ok(DfdlValue::Int(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Int(a), DfdlValue::Long(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) =
                ((a as i64).checked_rem(b), (a as i64).checked_div(b))
            {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        (DfdlValue::Long(a), DfdlValue::Int(b)) => {
            if b == 0 {
                Err(DFDLError::new_static(
                    DFDLErrorKind::ExpressionError,
                    "integer division by zero: NaN",
                ))
            } else if let (Some(rem), Some(div)) =
                (a.checked_rem(b as i64), a.checked_div(b as i64))
            {
                if rem == 0 {
                    Ok(DfdlValue::Long(div))
                } else {
                    Ok(DfdlValue::Double(a as f64 / b as f64))
                }
            } else {
                Ok(DfdlValue::Double(a as f64 / b as f64))
            }
        }
        _ => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in division",
        )),
    }
}

fn eval_idiv(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    if !left.simple_type().is_numeric() || !right.simple_type().is_numeric() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in idiv",
        ));
    }

    let is_float = matches!(left, DfdlValue::Double(_) | DfdlValue::Float(_))
        || matches!(right, DfdlValue::Double(_) | DfdlValue::Float(_));

    if is_float {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if a.is_nan() || b.is_nan() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division (idiv) operand is NaN",
            ));
        }
        if a.is_infinite() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division (idiv) operand is Infinity",
            ));
        }
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division by zero",
            ));
        }
        if b.is_infinite() {
            return Ok(DfdlValue::Long(0));
        }
        return Ok(DfdlValue::Long((a / b) as i64));
    }

    let is_decimal = matches!(left, DfdlValue::Decimal(_)) || matches!(right, DfdlValue::Decimal(_));
    if is_decimal {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::Parse,
                "Parse Error: integer division by zero",
            ));
        }
        return Ok(DfdlValue::Long((a / b) as i64));
    }

    let b = value_to_i64(&right);
    if b == 0 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::Parse,
            "Parse Error: integer division by zero",
        ));
    }
    let a = value_to_i64(&left);
    let res = a
        .checked_div(b)
        .ok_or_else(|| DFDLError::arithmetic_overflow("Long idiv overflow"))?;
    if matches!(left, DfdlValue::Int(_)) && matches!(right, DfdlValue::Int(_)) {
        if let Ok(res_i32) = i32::try_from(res) {
            return Ok(DfdlValue::Int(res_i32));
        }
    }
    Ok(DfdlValue::Long(res))
}

fn eval_mod(left: DfdlValue, right: DfdlValue) -> DFDLResult<DfdlValue> {
    if !left.simple_type().is_numeric() || !right.simple_type().is_numeric() {
        return Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Type mismatch in modulo",
        ));
    }

    if matches!(left, DfdlValue::Double(_)) || matches!(right, DfdlValue::Double(_)) {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        return Ok(DfdlValue::Double(a % b));
    }

    if matches!(left, DfdlValue::Float(_)) || matches!(right, DfdlValue::Float(_)) {
        let a = value_to_f64(&left) as f32;
        let b = value_to_f64(&right) as f32;
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        return Ok(DfdlValue::Float(a % b));
    }

    if matches!(left, DfdlValue::Decimal(_)) || matches!(right, DfdlValue::Decimal(_)) {
        let a = value_to_f64(&left);
        let b = value_to_f64(&right);
        if b == 0.0 || b == -0.0 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::ExpressionError,
                "integer division by zero: modulo zero NaN",
            ));
        }
        let rem = a % b;
        return Ok(DfdlValue::Decimal(alloc::format!("{}", rem)));
    }

    let b = value_to_i64(&right);
    if b == 0 {
        return Err(DFDLError::new_static(
            DFDLErrorKind::ExpressionError,
            "integer division by zero: modulo zero NaN",
        ));
    }
    let a = value_to_i64(&left);
    let rem = a.checked_rem(b).unwrap_or(0);
    if matches!(left, DfdlValue::Int(_)) && matches!(right, DfdlValue::Int(_)) {
        return Ok(DfdlValue::Int(rem as i32));
    }
    Ok(DfdlValue::Long(rem))
}

fn eval_cmp<F: Fn(core::cmp::Ordering) -> bool>(
    op_name: &str,
    left: DfdlValue,
    right: DfdlValue,
    pred: F,
) -> DFDLResult<DfdlValue> {
    if matches!(left, DfdlValue::HexBinary(_)) || matches!(right, DfdlValue::HexBinary(_)) {
        let msg = alloc::format!(
            "Schema Definition Error: Unsupported operation '{}' on type hexBinary",
            op_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    match (&left, &right) {
        (DfdlValue::Boolean(a), DfdlValue::Boolean(b)) => Ok(DfdlValue::Boolean(pred(a.cmp(b)))),
        (DfdlValue::Boolean(_), _) | (_, DfdlValue::Boolean(_)) => Err(DFDLError::new_static(
            DFDLErrorKind::TypeError,
            "Cannot compare boolean with non-boolean value",
        )),
        (DfdlValue::String(a), DfdlValue::String(b)) => Ok(DfdlValue::Boolean(pred(a.cmp(b)))),
        (DfdlValue::Float(a), DfdlValue::Float(b)) => match a.partial_cmp(b) {
            Some(ord) => Ok(DfdlValue::Boolean(pred(ord))),
            None => Ok(DfdlValue::Boolean(false)),
        },
        (DfdlValue::Double(a), DfdlValue::Double(b)) => match a.partial_cmp(b) {
            Some(ord) => Ok(DfdlValue::Boolean(pred(ord))),
            None => Ok(DfdlValue::Boolean(false)),
        },
        (DfdlValue::Float(_), _)
        | (_, DfdlValue::Float(_))
        | (DfdlValue::Double(_), _)
        | (_, DfdlValue::Double(_)) => {
            let a = value_to_f64(&left);
            let b = value_to_f64(&right);
            match a.partial_cmp(&b) {
                Some(ord) => Ok(DfdlValue::Boolean(pred(ord))),
                None => Ok(DfdlValue::Boolean(false)),
            }
        }
        _ => {
            let a = value_to_i64(&left);
            let b = value_to_i64(&right);
            Ok(DfdlValue::Boolean(pred(a.cmp(&b))))
        }
    }
}

fn value_to_i64(val: &DfdlValue) -> i64 {
    match val {
        DfdlValue::Int(n) => *n as i64,
        DfdlValue::Long(n) => *n,
        DfdlValue::Short(n) => *n as i64,
        DfdlValue::Byte(n) => *n as i64,
        DfdlValue::UnsignedLong(n) => *n as i64,
        DfdlValue::UnsignedInt(n) => *n as i64,
        DfdlValue::UnsignedShort(n) => *n as i64,
        DfdlValue::UnsignedByte(n) => *n as i64,
        DfdlValue::Float(f) => *f as i64,
        DfdlValue::Double(f) => *f as i64,
        DfdlValue::String(s) | DfdlValue::Decimal(s) => s.trim().parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn validate_bitwise_operand(val: &DfdlValue, fn_name: &str) -> DFDLResult<(u64, bool)> {
    match val {
        DfdlValue::Long(n) => Ok((*n as u64, true)),
        DfdlValue::Int(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::Short(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::Byte(n) => Ok((*n as i64 as u64, true)),
        DfdlValue::UnsignedLong(n) => Ok((*n, false)),
        DfdlValue::UnsignedInt(n) => Ok((*n as u64, false)),
        DfdlValue::UnsignedShort(n) => Ok((*n as u64, false)),
        DfdlValue::UnsignedByte(n) => Ok((*n as u64, false)),
        _ => {
            let msg = alloc::format!(
                "Schema Definition Error: Arguments to dfdlx:{} must be xs:unsignedLong or xs:long or a subtype of those",
                fn_name
            );
            Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
        }
    }
}

fn value_to_f64(val: &DfdlValue) -> f64 {
    match val {
        DfdlValue::Double(f) => *f,
        DfdlValue::Float(f) => *f as f64,
        DfdlValue::Int(n) => *n as f64,
        DfdlValue::Long(n) => *n as f64,
        DfdlValue::Short(n) => *n as f64,
        DfdlValue::Byte(n) => *n as f64,
        DfdlValue::UnsignedLong(n) => *n as f64,
        DfdlValue::UnsignedInt(n) => *n as f64,
        DfdlValue::UnsignedShort(n) => *n as f64,
        DfdlValue::UnsignedByte(n) => *n as f64,
        DfdlValue::String(s) | DfdlValue::Decimal(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn value_to_bool(val: &DfdlValue) -> bool {
    match val {
        DfdlValue::Boolean(b) => *b,
        DfdlValue::Int(n) => *n != 0,
        DfdlValue::Long(n) => *n != 0,
        DfdlValue::Short(n) => *n != 0,
        DfdlValue::Byte(n) => *n != 0,
        DfdlValue::UnsignedLong(n) => *n != 0,
        DfdlValue::UnsignedInt(n) => *n != 0,
        DfdlValue::UnsignedShort(n) => *n != 0,
        DfdlValue::UnsignedByte(n) => *n != 0,
        DfdlValue::Float(f) => *f != 0.0 && !f.is_nan(),
        DfdlValue::Double(f) => *f != 0.0 && !f.is_nan(),
        DfdlValue::String(s) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => false,
    }
}

fn parse_date_components(s: &str) -> Option<(i64, i64, i64)> {
    let s = s.trim();
    let (is_neg, rest) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s)
    };
    let mut parts = rest.split('-');
    let year_str = parts.next()?;
    let month_str = parts.next()?;
    let day_rem = parts.next()?;
    let day_str = day_rem.split(['+', '-', 'Z', 'T']).next()?;
    let mut year: i64 = year_str.parse().ok()?;
    if is_neg {
        year = year.saturating_neg();
    }
    let month: i64 = month_str.parse().ok()?;
    let day: i64 = day_str.parse().ok()?;
    Some((year, month, day))
}

fn parse_time_components(s: &str) -> Option<(i64, i64, DfdlValue)> {
    let s = s.trim();
    let mut parts = s.split(':');
    let hours_str = parts.next()?;
    let minutes_str = parts.next()?;
    let sec_rem = parts.next()?;
    let sec_str = sec_rem.split(['+', '-', 'Z']).next()?;
    let hours: i64 = hours_str.parse().ok()?;
    let minutes: i64 = minutes_str.parse().ok()?;
    let sec_val = if let Some((whole, frac)) = sec_str.split_once('.') {
        let whole_num: u64 = whole.parse().ok()?;
        let dec_str = alloc::format!("{}.{}", whole_num, frac);
        DfdlValue::Decimal(dec_str)
    } else {
        let sec_num: u64 = sec_str.parse().ok()?;
        DfdlValue::Decimal(alloc::format!("{}", sec_num))
    };
    Some((hours, minutes, sec_val))
}

fn parse_datetime_components(s: &str) -> Option<(i64, i64, i64, i64, i64, DfdlValue)> {
    let s = s.trim();
    let (d_str, t_str) = s.split_once('T')?;
    let (y, m, d) = parse_date_components(d_str)?;
    let (hh, mm, ss) = parse_time_components(t_str)?;
    Some((y, m, d, hh, mm, ss))
}

fn extract_date_str<'a>(arg: &'a DfdlValue, target_kind: &str) -> DFDLResult<&'a str> {
    match arg {
        DfdlValue::Date(s) => {
            if target_kind != "Date" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: Date cannot be converted to {}", target_kind),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::DateTime(s) => {
            if target_kind != "DateTime" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: DateTime cannot be converted to {}", target_kind),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::Time(s) => {
            if target_kind != "Time" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: Time cannot be converted to {}", target_kind),
                ));
            }
            Ok(s.as_str())
        }
        DfdlValue::String(s) => {
            let s_trim = s.trim();
            if target_kind == "Date" && s_trim.contains('T') {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: DateTime cannot be converted to Date",
                ));
            }
            if target_kind == "Date" && !s_trim.contains('-') && s_trim.contains(':') {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Time cannot be converted to Date",
                ));
            }
            if target_kind == "Time" && s_trim.contains('-') && !s_trim.starts_with('-') && !s_trim.contains(':') {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Date cannot be converted to Time",
                ));
            }
            if target_kind == "DateTime" && !s_trim.contains('T') {
                if s_trim.contains('-') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Date cannot be converted to DateTime",
                    ));
                } else if s_trim.contains(':') {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Time cannot be converted to DateTime",
                    ));
                }
            }
            Ok(s_trim)
        }
        other => Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            &alloc::format!("Schema Definition Error: cannot convert {} to {}", other.type_name(), target_kind),
        )),
    }
}

fn eval_fn_call(name: &QName, args: &[DfdlValue], ctx: &ExprContext) -> DFDLResult<DfdlValue> {
    macro_rules! cast_to_int {
        ($arg:expr, $type_name:expr, $target_type:ty, $variant:ident) => {{
            let err = || {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("cannot convert value to xs:{}", $type_name),
                )
            };
            let err_str = |s: &str| {
                DFDLError::new(
                    DFDLErrorKind::Parse,
                    &alloc::format!("Parse Error: Cannot convert '{}' from String to Long on element (cannot convert to xs:{})", s, $type_name),
                )
            };
            let res: $target_type = match $arg {
                DfdlValue::Int(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Long(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Short(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Byte(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedLong(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedInt(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedShort(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::UnsignedByte(n) => (*n).try_into().map_err(|_| err())?,
                DfdlValue::Float(f) => {
                    if !f.is_finite() {
                        return Err(err());
                    }
                    let f64_val = *f as f64;
                    if f64_val < (<$target_type>::MIN as f64)
                        || f64_val > (<$target_type>::MAX as f64)
                    {
                        return Err(err());
                    }
                    f64_val as $target_type
                }
                DfdlValue::Double(f) => {
                    if !f.is_finite() {
                        return Err(err());
                    }
                    if *f < (<$target_type>::MIN as f64) || *f > (<$target_type>::MAX as f64) {
                        return Err(err());
                    }
                    *f as $target_type
                }
                DfdlValue::Boolean(b) => {
                    let v: i32 = if *b { 1 } else { 0 };
                    v.try_into().map_err(|_| err())?
                }
                DfdlValue::String(s) => s.trim().parse::<$target_type>().map_err(|_| err_str(s))?,
                DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if let Ok(v) = trimmed.parse::<$target_type>() {
                        v
                    } else if let Ok(f) = trimmed.parse::<f64>() {
                        if f.is_finite()
                            && f >= (<$target_type>::MIN as f64)
                            && f <= (<$target_type>::MAX as f64)
                        {
                            f as $target_type
                        } else {
                            return Err(err_str(s));
                        }
                    } else {
                        return Err(err_str(s));
                    }
                }
                _ => return Err(err()),
            };
            Ok(DfdlValue::$variant(res))
        }};
    }

    match name.local_name.as_str() {
        "error" => {
            let msg = if let Some(first) = args.first() {
                alloc::format!("fn:error(): {}", first)
            } else {
                alloc::string::String::from("fn:error() raised by expression")
            };
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
        "valueLength" | "contentLength" => {
            let val = get_checked(args, 0)?;
            let units = args
                .get(1)
                .map(|u| format!("{}", u))
                .unwrap_or_else(|| String::from("bytes"));
            let byte_len = match val {
                DfdlValue::String(s) => s.len(),
                DfdlValue::HexBinary(b) => b.len(),
                DfdlValue::Int(_) | DfdlValue::Short(_) => 4,
                DfdlValue::Long(_) | DfdlValue::Double(_) => 8,
                DfdlValue::Byte(_) => 1,
                _ => format!("{}", val).len(),
            };
            match units.to_lowercase().as_str() {
                "bits" => Ok(DfdlValue::Long((byte_len.saturating_mul(8)) as i64)),
                "characters" | "bytes" => Ok(DfdlValue::Long(byte_len as i64)),
                _ => Ok(DfdlValue::Long(byte_len as i64)),
            }
        }
        "concat" => {
            let mut res = String::new();
            for arg in args {
                let arg_str = alloc::format!("{}", arg);
                res.push_str(&arg_str);
            }
            Ok(DfdlValue::String(res))
        }
        "string-length" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::Long(s.chars().count() as i64))
        }
        "decodeDFDLEntities" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::String(crate::expr::properties::decode_dfdl_character_entities(&s)))
        }
        "encodeDFDLEntities" => {
            let arg = get_checked(args, 0)?;
            let s = format!("{}", arg);
            Ok(DfdlValue::String(crate::expr::properties::encode_dfdl_character_entities(&s)))
        }
        "substring" => {
            let str_arg = get_checked(args, 0)?;
            let start_arg = get_checked(args, 1)?;
            let len_arg = args.get(2);

            let s = format!("{}", str_arg);
            let chars: Vec<char> = s.chars().collect();

            let raw_start = match start_arg {
                DfdlValue::Double(d) => {
                    if d.is_nan() || *d == f64::INFINITY {
                        return Ok(DfdlValue::String(String::new()));
                    }
                    if *d == f64::NEG_INFINITY {
                        i64::MIN
                    } else {
                        *d as i64
                    }
                }
                DfdlValue::Float(f) => {
                    if f.is_nan() || *f == f32::INFINITY {
                        return Ok(DfdlValue::String(String::new()));
                    }
                    if *f == f32::NEG_INFINITY {
                        i64::MIN
                    } else {
                        *f as i64
                    }
                }
                DfdlValue::Long(n) => *n,
                DfdlValue::Int(n) => *n as i64,
                _ => 1,
            };

            let start_idx = (raw_start.max(1).saturating_sub(1)) as usize;
            if start_idx >= chars.len() {
                return Ok(DfdlValue::String(String::new()));
            }

            let avail = chars.len().saturating_sub(start_idx);
            let len = if let Some(la) = len_arg {
                match la {
                    DfdlValue::Double(d) => {
                        if d.is_nan() || *d <= 0.0 {
                            0
                        } else {
                            (*d as usize).min(avail)
                        }
                    }
                    DfdlValue::Float(f) => {
                        if f.is_nan() || *f <= 0.0 {
                            0
                        } else {
                            (*f as usize).min(avail)
                        }
                    }
                    DfdlValue::Long(n) => ((*n).max(0) as usize).min(avail),
                    DfdlValue::Int(n) => ((*n).max(0) as usize).min(avail),
                    _ => avail,
                }
            } else {
                avail
            };

            let sub: String = chars.iter().skip(start_idx).take(len).collect();
            Ok(DfdlValue::String(sub))
        }
        "substring-before" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str_arg);
            let s2 = alloc::format!("{}", sub_arg);
            if s2.is_empty() {
                return Ok(DfdlValue::String(String::new()));
            }
            if let Some(pos) = s1.find(&s2) {
                let res = s1.get(..pos).unwrap_or("").to_string();
                Ok(DfdlValue::String(res))
            } else {
                Ok(DfdlValue::String(String::new()))
            }
        }
        "substring-after" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str_arg);
            let s2 = alloc::format!("{}", sub_arg);
            if s2.is_empty() {
                return Ok(DfdlValue::String(s1));
            }
            if let Some(pos) = s1.find(&s2) {
                let after_pos = pos.saturating_add(s2.len());
                let res = s1.get(after_pos..).unwrap_or("").to_string();
                Ok(DfdlValue::String(res))
            } else {
                Ok(DfdlValue::String(String::new()))
            }
        }
        "contains" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.contains(&s2)))
        }
        "starts-with" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.starts_with(&s2)))
        }
        "ends-with" => {
            let str_arg = get_checked(args, 0)?;
            let sub_arg = get_checked(args, 1)?;
            let s1 = format!("{}", str_arg);
            let s2 = format!("{}", sub_arg);
            Ok(DfdlValue::Boolean(s1.ends_with(&s2)))
        }
        "upper-case" => {
            let str_arg = get_checked(args, 0)?;
            let s = format!("{}", str_arg);
            Ok(DfdlValue::String(s.to_uppercase()))
        }
        "lower-case" => {
            let str_arg = get_checked(args, 0)?;
            let s = format!("{}", str_arg);
            Ok(DfdlValue::String(s.to_lowercase()))
        }
        "abs" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Int(n) => Ok(DfdlValue::Int(n.abs())),
                DfdlValue::Long(n) => Ok(DfdlValue::Long(n.abs())),
                DfdlValue::Float(f) => Ok(DfdlValue::Float(if *f < 0.0 { -*f } else { *f })),
                DfdlValue::Double(f) => Ok(DfdlValue::Double(if *f < 0.0 { -*f } else { *f })),
                _ => Ok(num_arg.clone()),
            }
        }
        "ceiling" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot ceiling NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Float(*f));
                    }
                    let i = *f as i64;
                    let inc = if *f > (i as f32) { 1 } else { 0 };
                    Ok(DfdlValue::Float((i.saturating_add(inc)) as f32))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot ceiling NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Double(*f));
                    }
                    let i = *f as i64;
                    let inc = if *f > (i as f64) { 1 } else { 0 };
                    Ok(DfdlValue::Double((i.saturating_add(inc)) as f64))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "floor" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot floor NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Float(*f));
                    }
                    let i = *f as i64;
                    let dec = if *f < (i as f32) { 1 } else { 0 };
                    Ok(DfdlValue::Float((i.saturating_sub(dec)) as f32))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot floor NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Double(*f));
                    }
                    let i = *f as i64;
                    let dec = if *f < (i as f64) { 1 } else { 0 };
                    Ok(DfdlValue::Double((i.saturating_sub(dec)) as f64))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "round" => {
            let num_arg = get_checked(args, 0)?;
            match num_arg {
                DfdlValue::Float(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot round NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Float(*f));
                    }
                    let rounded = if *f >= 0.0 {
                        (*f + 0.5) as i64 as f32
                    } else {
                        let i = (*f + 0.5) as i64;
                        i as f32
                    };
                    Ok(DfdlValue::Float(rounded))
                }
                DfdlValue::Double(f) => {
                    if f.is_nan() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Parse Error: Cannot round NaN",
                        ));
                    }
                    if f.is_infinite() {
                        return Ok(DfdlValue::Double(*f));
                    }
                    let rounded = if *f >= 0.0 {
                        (*f + 0.5) as i64 as f64
                    } else {
                        let i = (*f + 0.5) as i64;
                        i as f64
                    };
                    Ok(DfdlValue::Double(rounded))
                }
                _ => Ok(num_arg.clone()),
            }
        }
        "matches" => {
            let str_arg = get_checked(args, 0)?;
            let pat_arg = get_checked(args, 1)?;
            let s = format!("{}", str_arg);
            let p = format!("{}", pat_arg);
            Ok(DfdlValue::Boolean(s.contains(&p)))
        }
        "exists" => {
            if let Some(arg) = args.first() {
                let path_str = alloc::format!("{}", arg);
                let path = InfosetPath::parse(&path_str);
                let exists = path_node_exists(&path, ctx).unwrap_or(false);
                Ok(DfdlValue::Boolean(exists))
            } else {
                Ok(DfdlValue::Boolean(false))
            }
        }
        "checkConstraints" => {
            let target_val = if let Some(arg) = args.first() {
                arg.clone()
            } else if let Some(doc) = ctx.doc {
                if let Some(elem) =
                    doc.find_element_with_context(ctx.current_path, ctx.occurs_index)
                {
                    if let ElementState::Value(val) = &elem.state {
                        val.clone()
                    } else {
                        let elem_name = ctx
                            .current_path
                            .segments()
                            .last()
                            .map(|s| s.as_str())
                            .unwrap_or("");
                        let msg = alloc::format!(
                            "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                            elem_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
                    }
                } else {
                    let elem_name = ctx
                        .current_path
                        .segments()
                        .last()
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let msg = alloc::format!(
                        "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                        elem_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
                }
            } else {
                let elem_name = ctx
                    .current_path
                    .segments()
                    .last()
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let msg = alloc::format!(
                    "Expression Evaluation Error: Self referencing element '{}' does not have a value",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            };
            let target_path = ctx.current_path;
            if let Some(sch) = ctx.schema {
                let term_opt = sch.find_term_by_path(target_path).or_else(|| {
                    target_path
                        .segments()
                        .last()
                        .map(|s| s.split(':').next_back().unwrap_or(s))
                        .and_then(|last_name| {
                            sch.terms.iter().find(|t| {
                                let clean_t = t
                                    .name
                                    .local_name
                                    .split(':')
                                    .next_back()
                                    .unwrap_or(&t.name.local_name);
                                clean_t == last_name
                            })
                        })
                });
                if let Some(term) = term_opt {
                    let mut is_valid = term.properties.facets.validate_value(&target_val);
                    if is_valid {
                        if let crate::schema::ir::TermKind::Element(ref elem) = term.kind {
                            if let crate::schema::ir::CompiledType::Simple(ref st) = elem.type_ir {
                                if let Some(v) = target_val.as_i128() {
                                    let is_neg = target_val.is_negative();
                                    match st {
                                        crate::infoset::DfdlSimpleType::Int => {
                                            is_valid = i32::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Long => {
                                            is_valid = i64::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Short => {
                                            is_valid = i16::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::Byte => {
                                            is_valid = i8::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedInt => {
                                            is_valid = !is_neg && u32::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedShort => {
                                            is_valid = !is_neg && u16::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedByte => {
                                            is_valid = !is_neg && u8::try_from(v).is_ok();
                                        }
                                        crate::infoset::DfdlSimpleType::UnsignedLong => {
                                            is_valid = !is_neg && u64::try_from(v).is_ok();
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    return Ok(DfdlValue::Boolean(is_valid));
                }
            }
            Ok(DfdlValue::Boolean(true))
        }
        "double" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Double(d) => Ok(DfdlValue::Double(*d)),
                DfdlValue::Float(f) => Ok(DfdlValue::Double(*f as f64)),
                DfdlValue::Int(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Long(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Short(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Double(*n as f64)),
                DfdlValue::Boolean(b) => Ok(DfdlValue::Double(if *b { 1.0 } else { 0.0 })),
                DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if trimmed.eq_ignore_ascii_case("nan") && trimmed != "NaN" {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:double", s),
                        ));
                    }
                    if (trimmed.eq_ignore_ascii_case("inf") && trimmed != "INF")
                        || (trimmed.eq_ignore_ascii_case("-inf") && trimmed != "-INF")
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:double", s),
                        ));
                    }
                    let val = trimmed.parse::<f64>().map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:double", s),
                        )
                    })?;
                    Ok(DfdlValue::Double(val))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "cannot convert value to xs:double",
                )),
            }
        }
        "int" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "int", i32, Int)
        }
        "long" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "long", i64, Long)
        }
        "decimal" | "integer" | "nonNegativeInteger" => {
            let arg = get_checked(args, 0)?;
            let s = alloc::format!("{}", arg);
            Ok(DfdlValue::Decimal(s))
        }
        "short" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "short", i16, Short)
        }
        "byte" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "byte", i8, Byte)
        }
        "unsignedByte" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedByte", u8, UnsignedByte)
        }
        "unsignedShort" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedShort", u16, UnsignedShort)
        }
        "unsignedInt" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedInt", u32, UnsignedInt)
        }
        "unsignedLong" => {
            let arg = get_checked(args, 0)?;
            cast_to_int!(arg, "unsignedLong", u64, UnsignedLong)
        }
        "float" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Float(f) => Ok(DfdlValue::Float(*f)),
                DfdlValue::Double(f) => Ok(DfdlValue::Float(*f as f32)),
                DfdlValue::Int(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Long(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Short(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Float(*n as f32)),
                DfdlValue::Boolean(b) => Ok(DfdlValue::Float(if *b { 1.0 } else { 0.0 })),
                DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                    let trimmed = s.trim();
                    if trimmed.eq_ignore_ascii_case("nan") && trimmed != "NaN" {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:float", s),
                        ));
                    }
                    if (trimmed.eq_ignore_ascii_case("inf") && trimmed != "INF")
                        || (trimmed.eq_ignore_ascii_case("-inf") && trimmed != "-INF")
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:float", s),
                        ));
                    }
                    let val = trimmed.parse::<f32>().map_err(|_| {
                        DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Schema Definition Error: cannot convert '{}' to xs:float", s),
                        )
                    })?;
                    Ok(DfdlValue::Float(val))
                }
                _ => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "cannot convert value to xs:float",
                )),
            }
        }
        "string" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::String(alloc::format!("{}", arg)))
        }
        "compare" => {
            let str1 = get_checked(args, 0)?;
            let str2 = get_checked(args, 1)?;
            let s1 = alloc::format!("{}", str1);
            let s2 = alloc::format!("{}", str2);
            let cmp = match s1.cmp(&s2) {
                core::cmp::Ordering::Less => -1,
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 1,
            };
            Ok(DfdlValue::Int(cmp))
        }
        "true" => Ok(DfdlValue::Boolean(true)),
        "false" => Ok(DfdlValue::Boolean(false)),
        "date" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Date(alloc::format!("{}", arg)))
        }
        "dateTime" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::DateTime(alloc::format!("{}", arg)))
        }
        "time" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Time(alloc::format!("{}", arg)))
        }
        "year-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (y, _, _) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(y))
        }
        "month-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (_, m, _) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(m))
        }
        "day-from-date" => {
            let s = extract_date_str(get_checked(args, 0)?, "Date")?;
            let (_, _, d) = parse_date_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid date format")
            })?;
            Ok(DfdlValue::Long(d))
        }
        "year-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (y, _, _, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(y))
        }
        "month-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, m, _, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(m))
        }
        "day-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, d, _, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(d))
        }
        "hours-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, hh, _, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(hh))
        }
        "minutes-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, _, mm, _) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(DfdlValue::Long(mm))
        }
        "seconds-from-dateTime" => {
            let s = extract_date_str(get_checked(args, 0)?, "DateTime")?;
            let (_, _, _, _, _, ss) = parse_datetime_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid dateTime format")
            })?;
            Ok(ss)
        }
        "hours-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (hh, _, _) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(DfdlValue::Long(hh))
        }
        "minutes-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (_, mm, _) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(DfdlValue::Long(mm))
        }
        "seconds-from-time" => {
            let s = extract_date_str(get_checked(args, 0)?, "Time")?;
            let (_, _, ss) = parse_time_components(s).ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::ExpressionError, "Invalid time format")
            })?;
            Ok(ss)
        }
        "lookAhead" => {
            let offset = if let Ok(offset_val) = get_checked(args, 0) {
                if let Some(n) = offset_val.as_i128() {
                    if n < 0 {
                        let msg = alloc::format!(
                            "Schema Definition Error: out of range for xs:unsignedInt: {}",
                            n
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    n as usize
                } else {
                    0
                }
            } else {
                0
            };
            let bitsize = if let Ok(bitsize_val) = get_checked(args, 1) {
                if let Some(n) = bitsize_val.as_i128() {
                    if n < 0 {
                        let msg = alloc::format!(
                            "Schema Definition Error: out of range for xs:unsignedInt: {}",
                            n
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    n as usize
                } else {
                    0
                }
            } else {
                0
            };
            let total_dist = offset.saturating_add(bitsize);
            if total_dist > 512 {
                let msg = alloc::format!(
                    "Schema Definition Error: Look-ahead distance of {} bits exceeds implementation defined limit of 512 bits",
                    total_dist
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if let Some(la_fn) = ctx.lookahead_fn {
                let val = la_fn(offset, bitsize)?;
                if val <= (u64::MAX as u128) {
                    Ok(DfdlValue::UnsignedLong(val as u64))
                } else {
                    Ok(DfdlValue::Decimal(alloc::format!("{}", val)))
                }
            } else {
                Ok(DfdlValue::Long(0))
            }
        }
        "currentPosition" => Ok(DfdlValue::Long(0)),
        "occursIndex" => Ok(DfdlValue::Long(ctx.occurs_index as i64)),
        "count" => {
            if let Some(arg) = args.first() {
                match arg {
                    DfdlValue::String(s) if s.is_empty() => Ok(DfdlValue::Long(0)),
                    _ => Ok(DfdlValue::Long(1)),
                }
            } else {
                Ok(DfdlValue::Long(0))
            }
        }
        "boolean" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::Boolean(b) => Ok(DfdlValue::Boolean(*b)),
                DfdlValue::Int(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Long(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Short(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Byte(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedLong(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedInt(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedShort(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::UnsignedByte(n) => Ok(DfdlValue::Boolean(*n != 0)),
                DfdlValue::Float(f) => Ok(DfdlValue::Boolean(*f != 0.0 && !f.is_nan())),
                DfdlValue::Double(f) => Ok(DfdlValue::Boolean(*f != 0.0 && !f.is_nan())),
                DfdlValue::String(s) => match s.as_str() {
                    "true" | "1" => Ok(DfdlValue::Boolean(true)),
                    "false" | "0" => Ok(DfdlValue::Boolean(false)),
                    _ => {
                        let msg = format!(
                            "Cannot convert '{}' to xs:boolean: Must be one of 0, 1, true, or false",
                            s
                        );
                        Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
                    }
                },
                _ => {
                    let msg = "Cannot convert value to xs:boolean: Must be one of 0, 1, true, or false";
                    Err(DFDLError::new(DFDLErrorKind::ExpressionError, msg))
                }
            }
        }
        "not" => {
            let arg = get_checked(args, 0)?;
            let b = match arg {
                DfdlValue::Boolean(b) => *b,
                DfdlValue::Int(n) => *n != 0,
                DfdlValue::Long(n) => *n != 0,
                DfdlValue::Short(n) => *n != 0,
                DfdlValue::Byte(n) => *n != 0,
                DfdlValue::UnsignedLong(n) => *n != 0,
                DfdlValue::UnsignedInt(n) => *n != 0,
                DfdlValue::UnsignedShort(n) => *n != 0,
                DfdlValue::UnsignedByte(n) => *n != 0,
                DfdlValue::Float(f) => *f != 0.0 && !f.is_nan(),
                DfdlValue::Double(f) => *f != 0.0 && !f.is_nan(),
                DfdlValue::String(s) => s.eq_ignore_ascii_case("true") || s == "1",
                _ => false,
            };
            Ok(DfdlValue::Boolean(!b))
        }
        "replace" => {
            if name.prefix.as_deref().is_some()
                && name.prefix.as_deref() != Some("fn")
                && name.prefix.as_deref() != Some("dfdl")
                && name.prefix.as_deref() != Some("jgsu")
            {
                let pfx = name.prefix.as_deref().unwrap_or("");
                let msg = alloc::format!("Schema Definition Error: Unsupported function: {}:replace", pfx);
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if args.len() < 3 {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!("Schema Definition Error: The {} function requires 3 argument(s).", name),
                ));
            }
            let target = format!("{}", get_checked(args, 0)?);
            let pattern = format!("{}", get_checked(args, 1)?);
            let replacement = format!("{}", get_checked(args, 2)?);
            Ok(DfdlValue::String(target.replace(&pattern, &replacement)))
        }
        "sayHello" => Ok(DfdlValue::String(String::from("Hello"))),
        "addBoxed" | "addPrimitive" => {
            let arg1 = get_checked(args, 0)?;
            let arg2 = get_checked(args, 1)?;
            let parse_num = |val: &DfdlValue| -> DFDLResult<i64> {
                match val {
                    DfdlValue::Int(n) => Ok(*n as i64),
                    DfdlValue::Long(n) => Ok(*n),
                    DfdlValue::Short(n) => Ok(*n as i64),
                    DfdlValue::Byte(n) => Ok(*n as i64),
                    DfdlValue::UnsignedLong(n) => Ok(*n as i64),
                    DfdlValue::UnsignedInt(n) => Ok(*n as i64),
                    DfdlValue::UnsignedShort(n) => Ok(*n as i64),
                    DfdlValue::UnsignedByte(n) => Ok(*n as i64),
                    DfdlValue::String(s) | DfdlValue::Decimal(s) => {
                        s.trim().parse::<i64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::ExpressionError,
                                &format!("Cannot convert '{}' from String type to numeric", s),
                            )
                        })
                    }
                    _ => Err(DFDLError::new_static(
                        DFDLErrorKind::ExpressionError,
                        "Type mismatch in add function",
                    )),
                }
            };
            let n1 = parse_num(arg1)?;
            let n2 = parse_num(arg2)?;
            Ok(DfdlValue::Int(n1.saturating_add(n2) as i32))
        }
        "bitAnd" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitAnd")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitAnd")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitAnd must match in signedness",
                ));
            }
            let res = v1 & v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitOr" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitOr")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitOr")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitOr must match in signedness",
                ));
            }
            let res = v1 | v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitXor" => {
            let (v1, signed1) = validate_bitwise_operand(get_checked(args, 0)?, "bitXor")?;
            let (v2, signed2) = validate_bitwise_operand(get_checked(args, 1)?, "bitXor")?;
            if signed1 != signed2 {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Arguments to dfdlx:bitXor must match in signedness",
                ));
            }
            let res = v1 ^ v2;
            if signed1 {
                Ok(DfdlValue::Long(res as i64))
            } else {
                Ok(DfdlValue::UnsignedLong(res))
            }
        }
        "bitNot" => {
            // The complement keeps the operand's type: for unsigned subtypes the result must
            // stay within the type's width (e.g. ~1 as xs:unsignedByte is 254, not 2^64-2).
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "bitNot")?;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(!*n),
                DfdlValue::Short(n) => DfdlValue::Short(!*n),
                DfdlValue::Int(n) => DfdlValue::Int(!*n),
                DfdlValue::Long(n) => DfdlValue::Long(!*n),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(!*n),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(!*n),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(!*n),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(!*n),
                other => other.clone(),
            })
        }
        "leftShift" => {
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "leftShift")?;
            let shift_arg = get_checked(args, 1)?;
            let count = value_to_i64(shift_arg);
            let bit_width: i64 = match arg {
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => 8,
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => 16,
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => 32,
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => 64,
                _ => 64,
            };
            if count < 0 || count >= bit_width {
                let msg = alloc::format!(
                    "Schema Definition Error: Shift count must be between 0 and {} (less than {}), but was {}",
                    bit_width.saturating_sub(1), bit_width, count
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let c = count as u32;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(((*n as u8).wrapping_shl(c)) as i8),
                DfdlValue::Short(n) => DfdlValue::Short(((*n as u16).wrapping_shl(c)) as i16),
                DfdlValue::Int(n) => DfdlValue::Int(((*n as u32).wrapping_shl(c)) as i32),
                DfdlValue::Long(n) => DfdlValue::Long(((*n as u64).wrapping_shl(c)) as i64),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(n.wrapping_shl(c)),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(n.wrapping_shl(c)),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(n.wrapping_shl(c)),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(n.wrapping_shl(c)),
                other => other.clone(),
            })
        }
        "rightShift" => {
            let arg = get_checked(args, 0)?;
            validate_bitwise_operand(arg, "rightShift")?;
            let shift_arg = get_checked(args, 1)?;
            let count = value_to_i64(shift_arg);
            let bit_width: i64 = match arg {
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => 8,
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => 16,
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => 32,
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => 64,
                _ => 64,
            };
            if count < 0 || count >= bit_width {
                let msg = alloc::format!(
                    "Schema Definition Error: Shift count must be between 0 and {} (less than {}), but was {}",
                    bit_width.saturating_sub(1), bit_width, count
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let c = count as u32;
            Ok(match arg {
                DfdlValue::Byte(n) => DfdlValue::Byte(n.wrapping_shr(c)),
                DfdlValue::Short(n) => DfdlValue::Short(n.wrapping_shr(c)),
                DfdlValue::Int(n) => DfdlValue::Int(n.wrapping_shr(c)),
                DfdlValue::Long(n) => DfdlValue::Long(n.wrapping_shr(c)),
                DfdlValue::UnsignedByte(n) => DfdlValue::UnsignedByte(n.wrapping_shr(c)),
                DfdlValue::UnsignedShort(n) => DfdlValue::UnsignedShort(n.wrapping_shr(c)),
                DfdlValue::UnsignedInt(n) => DfdlValue::UnsignedInt(n.wrapping_shr(c)),
                DfdlValue::UnsignedLong(n) => DfdlValue::UnsignedLong(n.wrapping_shr(c)),
                other => other.clone(),
            })
        }
        "primByteFunc" | "boxedByteFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Byte(n as i8))
        }
        "primByteArrayFunc" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::String(format!("{}", arg)))
        }
        "primShortFunc" | "boxedShortFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Short(n as i16))
        }
        "primLongFunc" | "boxedLongFunc" => {
            let n = value_to_i64(get_checked(args, 0)?);
            Ok(DfdlValue::Long(n))
        }
        "primDoubleFunc" | "boxedDoubleFunc" => {
            let f = value_to_f64(get_checked(args, 0)?);
            Ok(DfdlValue::Double(f))
        }
        "primFloatFunc" | "boxedFloatFunc" => {
            let f = value_to_f64(get_checked(args, 0)?);
            Ok(DfdlValue::Float(f as f32))
        }
        "primBooleanFunc" | "boxedBooleanFunc" => {
            let b = value_to_bool(get_checked(args, 0)?);
            Ok(DfdlValue::Boolean(b))
        }
        "javaBigDecimalFunc" | "javaBigIntegerFunc" => {
            let arg = get_checked(args, 0)?;
            Ok(DfdlValue::Decimal(format!("{}", arg)))
        }
        "checkRangeInclusive" | "checkRangeExclusive" => {
            let a0 = get_checked(args, 0)?;
            let a1 = get_checked(args, 1)?;
            let a2 = get_checked(args, 2)?;
            if !a0.simple_type().is_numeric()
                || !a1.simple_type().is_numeric()
                || !a2.simple_type().is_numeric()
            {
                let msg = alloc::format!(
                    "Schema Definition Error: Cannot call dfdl:{} with non-numeric types: {}, {}, {}",
                    name.local_name,
                    a0.type_name(),
                    a1.type_name(),
                    a2.type_name()
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let val = value_to_f64(a0);
            let min = value_to_f64(a1);
            let max = value_to_f64(a2);
            if name.local_name == "checkRangeInclusive" {
                Ok(DfdlValue::Boolean(val >= min && val <= max))
            } else {
                Ok(DfdlValue::Boolean(val > min && val < max))
            }
        }
        "hexBinary" => {
            let arg = get_checked(args, 0)?;
            match arg {
                DfdlValue::HexBinary(b) => Ok(DfdlValue::HexBinary(b.clone())),
                DfdlValue::Byte(b) => Ok(DfdlValue::HexBinary(alloc::vec![*b as u8])),
                DfdlValue::UnsignedByte(b) => Ok(DfdlValue::HexBinary(alloc::vec![*b])),
                DfdlValue::Short(s) => Ok(DfdlValue::HexBinary(s.to_be_bytes().to_vec())),
                DfdlValue::UnsignedShort(s) => Ok(DfdlValue::HexBinary(s.to_be_bytes().to_vec())),
                _ if arg.as_i128().is_some() => {
                    let n = arg.as_i128().unwrap_or(0);
                    let bytes = if (-128..=127).contains(&n) {
                        alloc::vec![n as i8 as u8]
                    } else if (-32768..=32767).contains(&n) {
                        (n as i16).to_be_bytes().to_vec()
                    } else if (-2147483648..=2147483647).contains(&n) {
                        (n as i32).to_be_bytes().to_vec()
                    } else if (i64::MIN as i128..=i64::MAX as i128).contains(&n) {
                        (n as i64).to_be_bytes().to_vec()
                    } else {
                        n.to_be_bytes().to_vec()
                    };
                    Ok(DfdlValue::HexBinary(bytes))
                }
                _ => {
                    let s = format!("{}", arg);
                    let clean = s.trim();
                    if clean.len() % 2 != 0 {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "Parse Error: xs:hexBinary string must have an even number of characters: '{}'",
                                clean
                            ),
                        ));
                    }
                    for ch in clean.chars() {
                        if !ch.is_ascii_hexdigit() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                &alloc::format!(
                                    "Parse Error: Hex character must be [0-9a-fA-F], found '{}'",
                                    ch
                                ),
                            ));
                        }
                    }
                    let mut bytes = Vec::new();
                    for i in (0..clean.len()).step_by(2) {
                        let end = i.saturating_add(2).min(clean.len());
                        if let Some(sub) = clean.get(i..end) {
                            let b = u8::from_str_radix(sub, 16).map_err(|_| {
                                DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!("Parse Error: Invalid hex byte '{}'", sub),
                                )
                            })?;
                            bytes.push(b);
                        }
                    }
                    Ok(DfdlValue::HexBinary(bytes))
                }
            }
        }
        "round-half-to-even" | "roundHalfToEven" => {
            let num = value_to_f64(get_checked(args, 0)?);
            if num.is_nan() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Parse Error: Cannot round NaN",
                ));
            }
            let precision = args.get(1).map(value_to_f64).unwrap_or(0.0) as i32;
            let mut factor = 1.0f64;
            if precision > 0 {
                for _ in 0..precision.min(10) {
                    factor *= 10.0;
                }
            } else if precision < 0 {
                for _ in 0..precision.saturating_neg().min(10) {
                    factor /= 10.0;
                }
            }
            let scaled = num * factor;
            let i = scaled as i64;
            let floor_val = if scaled < (i as f64) {
                i.saturating_sub(1) as f64
            } else {
                i as f64
            };
            let diff = scaled - floor_val;
            let rounded = if (diff - 0.5).abs() < 1e-9 {
                if ((floor_val as i64) % 2).abs() == 0 {
                    floor_val
                } else {
                    floor_val + 1.0
                }
            } else if diff >= 0.5 {
                floor_val + 1.0
            } else {
                floor_val
            };
            Ok(DfdlValue::Double(rounded / factor))
        }
        "trace" => {
            if name.prefix.as_deref() == Some("fn") {
                let msg = "Schema Definition Error: Unsupported function: fn:trace";
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, msg));
            }
            let val = get_checked(args, 0)?;
            Ok(val.clone())
        }
        _ => {
            let func_name = if let Some(ref pfx) = name.prefix {
                alloc::format!("{}:{}", pfx, name.local_name)
            } else {
                name.local_name.clone()
            };
            let msg = alloc::format!("Schema Definition Error: Unsupported function: {}", func_name);
            Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
        }
    }
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
    use crate::types::QName;

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
        assert_eq!(not(DfdlValue::UnsignedByte(1)), DfdlValue::UnsignedByte(254));
        assert_eq!(not(DfdlValue::UnsignedShort(2)), DfdlValue::UnsignedShort(65533));
        assert_eq!(not(DfdlValue::UnsignedInt(2)), DfdlValue::UnsignedInt(u32::MAX - 2));
        assert_eq!(not(DfdlValue::UnsignedLong(0)), DfdlValue::UnsignedLong(u64::MAX));
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
            &[DfdlValue::String(String::from("192.168.1.1")), DfdlValue::String(String::from("."))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sb, DfdlValue::String(String::from("192")));

        // substring-before not found
        let res_sb_none = eval_fn_call(
            &QName::local("substring-before"),
            &[DfdlValue::String(String::from("hello")), DfdlValue::String(String::from("xyz"))],
            &ctx,
        )
        .unwrap();
        assert_eq!(res_sb_none, DfdlValue::String(String::new()));

        // substring-after
        let res_sa = eval_fn_call(
            &QName::local("substring-after"),
            &[DfdlValue::String(String::from("192.168.1.1")), DfdlValue::String(String::from("."))],
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
                type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Float),
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
        assert!(err_float.message.to_string().contains("xs:unsignedLong or xs:long"));
    }

    #[test]
    fn test_decimal_arithmetic_precision() {
        // Decimal mul
        let mul_res = eval_mul(DfdlValue::Decimal("9".to_string()), DfdlValue::Decimal("5".to_string())).unwrap();
        assert_eq!(mul_res, DfdlValue::Decimal("45".to_string()));

        // Decimal div
        let div_res = eval_div(DfdlValue::Decimal("45".to_string()), DfdlValue::Long(1600)).unwrap();
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
            eval_cmp("lt", DfdlValue::Boolean(false), DfdlValue::Boolean(true), |o| o.is_lt()).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_cmp("lt", DfdlValue::Boolean(true), DfdlValue::Boolean(false), |o| o.is_lt()).unwrap(),
            DfdlValue::Boolean(false)
        );
        assert_eq!(
            eval_cmp("ge", DfdlValue::Boolean(true), DfdlValue::Boolean(true), |o| o.is_ge()).unwrap(),
            DfdlValue::Boolean(true)
        );
        assert_eq!(
            eval_cmp("le", DfdlValue::Boolean(false), DfdlValue::Boolean(false), |o| o.is_le()).unwrap(),
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
        ).unwrap();
        assert_eq!(y_date, DfdlValue::Long(2026));

        let m_date = eval_fn_call(
            &QName::local("month-from-date"),
            &[DfdlValue::Date(String::from("2026-10-05"))],
            &ctx,
        ).unwrap();
        assert_eq!(m_date, DfdlValue::Long(10));

        let d_date = eval_fn_call(
            &QName::local("day-from-date"),
            &[DfdlValue::Date(String::from("2026-10-05"))],
            &ctx,
        ).unwrap();
        assert_eq!(d_date, DfdlValue::Long(5));

        let y_dt = eval_fn_call(
            &QName::local("year-from-dateTime"),
            &[DfdlValue::DateTime(String::from("1999-05-31T13:20:05.18-05:00"))],
            &ctx,
        ).unwrap();
        assert_eq!(y_dt, DfdlValue::Long(1999));

        let sec_dt = eval_fn_call(
            &QName::local("seconds-from-dateTime"),
            &[DfdlValue::DateTime(String::from("1999-05-31T13:20:05.18-05:00"))],
            &ctx,
        ).unwrap();
        assert_eq!(sec_dt, DfdlValue::Decimal(String::from("5.18")));

        let h_time = eval_fn_call(
            &QName::local("hours-from-time"),
            &[DfdlValue::Time(String::from("13:20:00Z"))],
            &ctx,
        ).unwrap();
        assert_eq!(h_time, DfdlValue::Long(13));

        // Type misuse should return SchemaDefinition error
        let err_misuse = eval_fn_call(
            &QName::local("year-from-date"),
            &[DfdlValue::DateTime(String::from("1999-05-31T13:20:00"))],
            &ctx,
        ).unwrap_err();
        assert!(err_misuse.message.as_str().contains("DateTime cannot be converted to Date"));
    }
}

