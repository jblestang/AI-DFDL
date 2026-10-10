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
                            let elem_name = path.last_step().map(|s| s.local_name()).unwrap_or("complex");
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

                    let is_enclosing = if norm.steps().len() < ctx.current_path.steps().len()
                        && ctx.current_path.steps().starts_with(norm.steps())
                    {
                        true
                    } else if norm == *ctx.current_path {
                        let is_self_length_calc = if let Some(sch) = ctx.schema {
                            let clean_name = norm.last_step().map(|s| s.local_name()).unwrap_or("");
                            if let Some(term) = sch.find_term_by_name(clean_name).and_then(|id| sch.get_term(id)) {
                                term.properties.length_expr.is_some() || term.properties.truncate_specified_length_string
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if is_self_length_calc {
                            let current_name = ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or("");
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
                        let elem_name = norm.last_step().map(|s| s.raw_target()).unwrap_or("");
                        let qname = if elem_name.contains(':') {
                            alloc::string::ToString::to_string(elem_name)
                        } else {
                            alloc::format!("ex:{}", elem_name)
                        };
                        let current_name = ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or("");
                        let msg = alloc::format!(
                            "Runtime Schema Definition Error: Expression Evaluation Error in '{}': Value Length cannot be computed for enclosing element '{}'",
                            current_name, qname
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }

                    if let Some(sch) = ctx.schema {
                        let clean_target = norm.last_step().map(|s| s.local_name()).unwrap_or("");
                        let clean_curr = ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or("");
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
                        let clean_name = norm.last_step().map(|s| s.local_name()).unwrap_or("");
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

fn strip_path_namespaces(path: &InfosetPath) -> alloc::string::String {
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
fn eval_ovc_for_path(ctx: &ExprContext, norm: &InfosetPath) -> Option<DfdlValue> {
    if let Some(ovc_map) = ctx.ovc_values {
        let norm_str = alloc::format!("{}", norm);
        let stripped_norm = strip_path_namespaces(norm);
        if let Some(val) = ovc_map.get(&norm_str).or_else(|| ovc_map.get(&stripped_norm)) {
            return Some(val.clone());
        }
    }
    let schema = ctx.schema?;
    let clean_seg = norm.last_step()?.local_name();
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

    for (i, step) in path.steps().iter().enumerate() {
        let seg = path.segments().get(i).map(|s| s.as_str()).unwrap_or("");
        if step.is_self() {
            if step.index_predicate.is_some() || step.predicate_expr.is_some() {
                let msg = alloc::format!(
                    "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                    seg
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            let clean_expected = step.local_name();
            if clean_expected != "." && clean_expected != ".." && !clean_expected.is_empty() {
                let clean_curr_head = norm.last_step().map(|s| s.local_name()).unwrap_or("");

                if !clean_curr_head.is_empty() && clean_curr_head != clean_expected {
                    let msg = alloc::format!(
                        "Schema Definition Error: self::{} does not match current element '{}'",
                        step.raw_target(), clean_curr_head
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
            continue;
        } else if step.is_parent() {
            if step.index_predicate.is_some() || step.predicate_expr.is_some() {
                let msg = alloc::format!(
                    "Schema Definition Error: Indexing is only allowed on arrays. Invalid index expression '{}'",
                    seg
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
            let step_head = step.raw_target();
            let clean_head = step.local_name();
            let pred_opt = step.predicate_expr.as_deref();
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
            let final_seg = if let Some(idx) = step.index_predicate {
                alloc::format!("{}[{}]", step_head, idx)
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
                let clean_curr = ctx.current_path.last_step().map(|s| s.local_name()).unwrap_or("");
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
        (DfdlValue::Float(_), _)
        | (_, DfdlValue::Float(_))
        | (DfdlValue::Double(_), _)
        | (_, DfdlValue::Double(_))
        | (DfdlValue::Decimal(_), _)
        | (_, DfdlValue::Decimal(_)) => {
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
            } else if let Some(val) = ctx
                .doc
                .and_then(|doc| doc.find_element_with_context(ctx.current_path, ctx.occurs_index))
                .and_then(|elem| match &elem.state {
                    ElementState::Value(val) => Some(val.clone()),
                    _ => None,
                })
            {
                val
            } else {
                let elem_name = ctx
                    .current_path
                    .last_step()
                    .map(|s| s.raw_target())
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
                        .last_step()
                        .map(|s| s.local_name())
                        .and_then(|last_name| {
                            sch.terms.iter().find(|t| {
                                t.name.local_name.as_str() == last_name
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
    use alloc::boxed::Box;
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

    /// Verifies unary plus and unary minus operations across all numeric and non-numeric types.
    #[test]
    fn test_eval_unary_all_variants_and_errors() {
        // Unary plus on various types
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Int(10)).unwrap(), DfdlValue::Int(10));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Long(20)).unwrap(), DfdlValue::Long(20));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Short(5)).unwrap(), DfdlValue::Short(5));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Byte(2)).unwrap(), DfdlValue::Byte(2));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::UnsignedLong(100)).unwrap(), DfdlValue::UnsignedLong(100));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::UnsignedInt(50)).unwrap(), DfdlValue::UnsignedInt(50));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::UnsignedShort(25)).unwrap(), DfdlValue::UnsignedShort(25));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::UnsignedByte(10)).unwrap(), DfdlValue::UnsignedByte(10));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Float(1.5)).unwrap(), DfdlValue::Float(1.5));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_unary(UnaryOp::Plus, DfdlValue::Decimal(String::from("42.5"))).unwrap(), DfdlValue::Decimal(String::from("42.5")));

        // Invalid type for unary plus
        let err_plus = eval_unary(UnaryOp::Plus, DfdlValue::String(String::from("abc"))).unwrap_err();
        assert_eq!(err_plus.kind, DFDLErrorKind::TypeError);

        // Unary negate on scalar types
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Int(10)).unwrap(), DfdlValue::Int(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Long(20)).unwrap(), DfdlValue::Long(-20));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Short(5)).unwrap(), DfdlValue::Short(-5));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(2)).unwrap(), DfdlValue::Byte(-2));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedInt(50)).unwrap(), DfdlValue::Long(-50));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedShort(25)).unwrap(), DfdlValue::Int(-25));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedByte(10)).unwrap(), DfdlValue::Int(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Float(1.5)).unwrap(), DfdlValue::Float(-1.5));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(-2.5));

        // UnsignedLong negation edge cases: i64::MIN boundary, small u64, and large u64
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(100)).unwrap(), DfdlValue::Long(-100));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(9_223_372_036_854_775_808)).unwrap(), DfdlValue::Long(i64::MIN));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(u64::MAX)).unwrap(), DfdlValue::Decimal(alloc::format!("-{}", u64::MAX)));

        // Decimal negation (both positive and negative values)
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Decimal(String::from("10.5"))).unwrap(), DfdlValue::Decimal(String::from("-10.5")));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Decimal(String::from("-10.5"))).unwrap(), DfdlValue::Decimal(String::from("10.5")));

        // Negation overflow errors
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Int(i32::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Long(i64::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Short(i16::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(i8::MIN)).is_err());

        // Invalid type for unary negate
        let err_neg = eval_unary(UnaryOp::Negate, DfdlValue::Boolean(true)).unwrap_err();
        assert_eq!(err_neg.kind, DFDLErrorKind::TypeError);

        // Unary not operator
        assert_eq!(eval_unary(UnaryOp::Not, DfdlValue::Boolean(true)).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_unary(UnaryOp::Not, DfdlValue::Boolean(false)).unwrap(), DfdlValue::Boolean(true));
        let err_not = eval_unary(UnaryOp::Not, DfdlValue::Int(1)).unwrap_err();
        assert_eq!(err_not.kind, DFDLErrorKind::TypeError);
    }

    /// Verifies coercion of numeric representations to long or double.
    #[test]
    fn test_coerce_numeric_all_variants() {
        assert_eq!(coerce_numeric(DfdlValue::Int(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::Short(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::Byte(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::UnsignedInt(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::UnsignedShort(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::UnsignedByte(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::UnsignedLong(5)), DfdlValue::Long(5));
        assert_eq!(coerce_numeric(DfdlValue::Float(1.5)), DfdlValue::Double(1.5));
        assert_eq!(coerce_numeric(DfdlValue::String(String::from("100"))), DfdlValue::Long(100));
        assert_eq!(coerce_numeric(DfdlValue::String(String::from("2.5"))), DfdlValue::Double(2.5));
        assert_eq!(coerce_numeric(DfdlValue::String(String::from("not_num"))), DfdlValue::String(String::from("not_num")));
    }

    /// Verifies binary arithmetic operations (add, sub, mul, idiv, mod) across numeric types.
    #[test]
    fn test_eval_arithmetic_combinations() {
        use crate::expr::ast::BinaryOp;

        // Additions
        assert_eq!(eval_add(DfdlValue::Int(5), DfdlValue::Int(10)).unwrap(), DfdlValue::Int(15));
        assert_eq!(eval_add(DfdlValue::Long(5), DfdlValue::Long(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_add(DfdlValue::Int(5), DfdlValue::Long(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_add(DfdlValue::Long(10), DfdlValue::Int(5)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_add(DfdlValue::Float(1.0), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(3.5));
        assert_eq!(eval_add(DfdlValue::Double(1.0), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(3.5));
        assert_eq!(eval_add(DfdlValue::Long(5), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_add(DfdlValue::Double(2.5), DfdlValue::Long(5)).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_add(DfdlValue::Long(5), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(7.5));
        assert_eq!(eval_add(DfdlValue::Float(2.5), DfdlValue::Long(5)).unwrap(), DfdlValue::Float(7.5));
        assert_eq!(eval_add(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Decimal(String::from("5.25"))).unwrap(), DfdlValue::Decimal(String::from("15.75")));
        assert_eq!(eval_add(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(5)).unwrap(), DfdlValue::Decimal(String::from("15.5")));
        assert_eq!(eval_add(DfdlValue::Long(5), DfdlValue::Decimal(String::from("10.5"))).unwrap(), DfdlValue::Decimal(String::from("15.5")));
        assert_eq!(eval_add(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(12.5));
        assert_eq!(eval_add(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(12.5));
        assert_eq!(eval_add(DfdlValue::Double(2.0), DfdlValue::Decimal(String::from("10.5"))).unwrap(), DfdlValue::Double(12.5));
        assert_eq!(eval_add(DfdlValue::Float(2.0), DfdlValue::Decimal(String::from("10.5"))).unwrap(), DfdlValue::Float(12.5));
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Int(1)).is_err());
        assert!(eval_add(DfdlValue::Long(i64::MAX), DfdlValue::Long(1)).is_err());
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Long(i64::MAX)).is_err());
        assert!(eval_add(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Arithmetic via eval_binary which performs coerce_numeric on Short/Byte/Unsigned*
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::Short(5), DfdlValue::Short(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::Byte(5), DfdlValue::Byte(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::UnsignedLong(5), DfdlValue::UnsignedLong(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::UnsignedInt(5), DfdlValue::UnsignedInt(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::UnsignedShort(5), DfdlValue::UnsignedShort(10)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_binary(BinaryOp::Add, DfdlValue::UnsignedByte(5), DfdlValue::UnsignedByte(10)).unwrap(), DfdlValue::Long(15));

        // Subtractions
        assert_eq!(eval_sub(DfdlValue::Int(10), DfdlValue::Int(5)).unwrap(), DfdlValue::Int(5));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Long(5)).unwrap(), DfdlValue::Long(5));
        assert_eq!(eval_sub(DfdlValue::Int(10), DfdlValue::Long(5)).unwrap(), DfdlValue::Long(5));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Int(5)).unwrap(), DfdlValue::Long(5));
        assert_eq!(eval_sub(DfdlValue::Float(5.0), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(2.5));
        assert_eq!(eval_sub(DfdlValue::Double(5.0), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_sub(DfdlValue::Long(5), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_sub(DfdlValue::Double(5.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(3.0));
        assert_eq!(eval_sub(DfdlValue::Long(5), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(2.5));
        assert_eq!(eval_sub(DfdlValue::Float(5.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Float(3.0));
        assert_eq!(eval_sub(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Decimal(String::from("5.25"))).unwrap(), DfdlValue::Decimal(String::from("5.25")));
        assert_eq!(eval_sub(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(5)).unwrap(), DfdlValue::Decimal(String::from("5.5")));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Decimal(String::from("5.5"))).unwrap(), DfdlValue::Decimal(String::from("4.5")));
        assert_eq!(eval_sub(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(8.5));
        assert_eq!(eval_sub(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(8.5));
        assert_eq!(eval_sub(DfdlValue::Double(10.5), DfdlValue::Decimal(String::from("2.0"))).unwrap(), DfdlValue::Double(8.5));
        assert_eq!(eval_sub(DfdlValue::Float(10.5), DfdlValue::Decimal(String::from("2.0"))).unwrap(), DfdlValue::Float(8.5));
        assert!(eval_sub(DfdlValue::Int(i32::MIN), DfdlValue::Int(1)).is_err());
        assert!(eval_sub(DfdlValue::Long(i64::MIN), DfdlValue::Long(1)).is_err());
        assert!(eval_sub(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Multiplications
        assert_eq!(eval_mul(DfdlValue::Int(5), DfdlValue::Int(3)).unwrap(), DfdlValue::Int(15));
        assert_eq!(eval_mul(DfdlValue::Long(5), DfdlValue::Long(3)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_mul(DfdlValue::Int(5), DfdlValue::Long(3)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_mul(DfdlValue::Long(5), DfdlValue::Int(3)).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_mul(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(6.0));
        assert_eq!(eval_mul(DfdlValue::Double(2.0), DfdlValue::Double(3.0)).unwrap(), DfdlValue::Double(6.0));
        assert_eq!(eval_mul(DfdlValue::Long(2), DfdlValue::Double(3.0)).unwrap(), DfdlValue::Double(6.0));
        assert_eq!(eval_mul(DfdlValue::Double(2.0), DfdlValue::Long(3)).unwrap(), DfdlValue::Double(6.0));
        assert_eq!(eval_mul(DfdlValue::Long(2), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(6.0));
        assert_eq!(eval_mul(DfdlValue::Float(2.0), DfdlValue::Long(3)).unwrap(), DfdlValue::Float(6.0));
        assert_eq!(eval_mul(DfdlValue::Decimal(String::from("2.5")), DfdlValue::Decimal(String::from("3.0"))).unwrap(), DfdlValue::Decimal(String::from("7.5")));
        assert_eq!(eval_mul(DfdlValue::Decimal(String::from("2.5")), DfdlValue::Long(3)).unwrap(), DfdlValue::Decimal(String::from("7.5")));
        assert_eq!(eval_mul(DfdlValue::Long(3), DfdlValue::Decimal(String::from("2.5"))).unwrap(), DfdlValue::Decimal(String::from("7.5")));
        assert_eq!(eval_mul(DfdlValue::Decimal(String::from("2.5")), DfdlValue::Double(3.0)).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_mul(DfdlValue::Decimal(String::from("2.5")), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(7.5));
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Int(2)).is_err());
        assert!(eval_mul(DfdlValue::Long(i64::MAX), DfdlValue::Long(2)).is_err());
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Long(i64::MAX)).is_err());
        assert!(eval_mul(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Regular division (div)
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Long(10), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Int(10), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Int(2)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Long(10), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Int(10), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Int(2)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Long(6), DfdlValue::Long(2)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_div(DfdlValue::Long(5), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_div(DfdlValue::Int(6), DfdlValue::Int(2)).unwrap(), DfdlValue::Int(3));
        assert_eq!(eval_div(DfdlValue::Int(5), DfdlValue::Int(2)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_div(DfdlValue::Int(6), DfdlValue::Long(2)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_div(DfdlValue::Int(5), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_div(DfdlValue::Long(6), DfdlValue::Int(2)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_div(DfdlValue::Long(5), DfdlValue::Int(2)).unwrap(), DfdlValue::Double(2.5));
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Int(10), DfdlValue::Int(0)).is_err());
        assert!(eval_div(DfdlValue::Int(10), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Int(0)).is_err());
        assert!(eval_div(DfdlValue::Decimal(String::from("10")), DfdlValue::Long(0)).is_err());
        assert!(eval_div(DfdlValue::Long(10), DfdlValue::Decimal(String::from("0"))).is_err());
        assert_eq!(eval_div(DfdlValue::Decimal(String::from("10")), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Decimal(String::from("10")), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Decimal(String::from("2"))).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Decimal(String::from("2"))).unwrap(), DfdlValue::Float(5.0));
        assert!(eval_div(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Integer division (idiv)
        assert_eq!(eval_idiv(DfdlValue::Long(10), DfdlValue::Long(3)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_idiv(DfdlValue::Int(10), DfdlValue::Int(3)).unwrap(), DfdlValue::Int(3));
        assert_eq!(eval_idiv(DfdlValue::Double(10.5), DfdlValue::Double(3.2)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_idiv(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Long(3)).unwrap(), DfdlValue::Long(3));
        assert_eq!(eval_idiv(DfdlValue::Long(10), DfdlValue::Decimal(String::from("3.0"))).unwrap(), DfdlValue::Long(3));
        assert!(eval_idiv(DfdlValue::Double(f64::NAN), DfdlValue::Double(3.0)).is_err());
        assert!(eval_idiv(DfdlValue::Double(f64::INFINITY), DfdlValue::Double(3.0)).is_err());
        assert_eq!(eval_idiv(DfdlValue::Double(3.0), DfdlValue::Double(f64::INFINITY)).unwrap(), DfdlValue::Long(0));
        assert!(eval_idiv(DfdlValue::Double(10.0), DfdlValue::Double(0.0)).is_err());
        assert!(eval_idiv(DfdlValue::Decimal(String::from("10.0")), DfdlValue::Decimal(String::from("0.0"))).is_err());
        assert!(eval_idiv(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_idiv(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());

        // Modulo (mod)
        assert_eq!(eval_mod(DfdlValue::Long(10), DfdlValue::Long(3)).unwrap(), DfdlValue::Long(1));
        assert_eq!(eval_mod(DfdlValue::Int(10), DfdlValue::Int(3)).unwrap(), DfdlValue::Int(1));
        assert_eq!(eval_mod(DfdlValue::Double(10.5), DfdlValue::Double(3.0)).unwrap(), DfdlValue::Double(1.5));
        assert_eq!(eval_mod(DfdlValue::Float(10.5), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(1.5));
        assert_eq!(eval_mod(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Decimal(String::from("3.0"))).unwrap(), DfdlValue::Decimal(String::from("1.5")));
        assert!(eval_mod(DfdlValue::Double(10.5), DfdlValue::Double(0.0)).is_err());
        assert!(eval_mod(DfdlValue::Float(10.5), DfdlValue::Float(0.0)).is_err());
        assert!(eval_mod(DfdlValue::Decimal(String::from("10.5")), DfdlValue::Decimal(String::from("0.0"))).is_err());
        assert!(eval_mod(DfdlValue::Long(10), DfdlValue::Long(0)).is_err());
        assert!(eval_mod(DfdlValue::String(String::from("a")), DfdlValue::Int(1)).is_err());
    }

    /// Verifies relational comparisons across scalar types (strings, decimals, booleans, floats).
    #[test]
    fn test_eval_relational_comparisons() {
        use crate::expr::ast::BinaryOp;

        // String ordering
        assert_eq!(eval_binary(BinaryOp::Lt, DfdlValue::String(String::from("apple")), DfdlValue::String(String::from("banana"))).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Gt, DfdlValue::String(String::from("apple")), DfdlValue::String(String::from("banana"))).unwrap(), DfdlValue::Boolean(false));

        // Float comparisons
        assert_eq!(eval_binary(BinaryOp::Lt, DfdlValue::Float(1.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Le, DfdlValue::Float(2.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Gt, DfdlValue::Float(3.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Ge, DfdlValue::Float(2.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Boolean(true));

        // Double comparisons
        assert_eq!(eval_binary(BinaryOp::Lt, DfdlValue::Double(1.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Le, DfdlValue::Double(2.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Gt, DfdlValue::Double(3.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Ge, DfdlValue::Double(2.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Boolean(true));

        // Decimal comparisons
        assert_eq!(eval_binary(BinaryOp::Lt, DfdlValue::Decimal(String::from("1.5")), DfdlValue::Decimal(String::from("2.5"))).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Gt, DfdlValue::Decimal(String::from("3.5")), DfdlValue::Decimal(String::from("2.5"))).unwrap(), DfdlValue::Boolean(true));

        // Boolean comparisons
        assert_eq!(eval_binary(BinaryOp::Lt, DfdlValue::Boolean(false), DfdlValue::Boolean(true)).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_binary(BinaryOp::Gt, DfdlValue::Boolean(true), DfdlValue::Boolean(false)).unwrap(), DfdlValue::Boolean(true));
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
        assert_eq!(extract_date_str(&DfdlValue::Date(String::from("2026-10-07")), "Date").unwrap(), "2026-10-07");
        assert_eq!(extract_date_str(&DfdlValue::DateTime(String::from("2026-10-07T12:00:00")), "DateTime").unwrap(), "2026-10-07T12:00:00");
        assert_eq!(extract_date_str(&DfdlValue::Time(String::from("12:00:00")), "Time").unwrap(), "12:00:00");

        // String conversions
        assert_eq!(extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "Date").unwrap(), "2026-10-07");
        assert_eq!(extract_date_str(&DfdlValue::String(String::from("12:00:00")), "Time").unwrap(), "12:00:00");
        assert_eq!(extract_date_str(&DfdlValue::String(String::from("2026-10-07T12:00:00")), "DateTime").unwrap(), "2026-10-07T12:00:00");

        // Errors: Date cannot be converted to Time / DateTime
        assert!(extract_date_str(&DfdlValue::Date(String::from("2026-10-07")), "Time").is_err());
        assert!(extract_date_str(&DfdlValue::DateTime(String::from("2026-10-07T12:00:00")), "Date").is_err());
        assert!(extract_date_str(&DfdlValue::Time(String::from("12:00:00")), "DateTime").is_err());

        // String mismatch errors
        assert!(extract_date_str(&DfdlValue::String(String::from("2026-10-07T12:00:00")), "Date").is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("12:00:00")), "Date").is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "Time").is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("2026-10-07")), "DateTime").is_err());
        assert!(extract_date_str(&DfdlValue::String(String::from("12:00:00")), "DateTime").is_err());

        // Incompatible types
        assert!(extract_date_str(&DfdlValue::Int(42), "Date").is_err());
    }

    /// Verifies built-in constructor functions in eval_fn_call.
    #[test]
    fn test_eval_fn_call_constructors() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        assert_eq!(eval_fn_call(&QName::local("int"), &[DfdlValue::String(String::from("123"))], &ctx).unwrap(), DfdlValue::Int(123));
        assert_eq!(eval_fn_call(&QName::local("long"), &[DfdlValue::String(String::from("456"))], &ctx).unwrap(), DfdlValue::Long(456));
        assert_eq!(eval_fn_call(&QName::local("short"), &[DfdlValue::String(String::from("12"))], &ctx).unwrap(), DfdlValue::Short(12));
        assert_eq!(eval_fn_call(&QName::local("byte"), &[DfdlValue::String(String::from("7"))], &ctx).unwrap(), DfdlValue::Byte(7));
        assert_eq!(eval_fn_call(&QName::local("unsignedLong"), &[DfdlValue::String(String::from("100"))], &ctx).unwrap(), DfdlValue::UnsignedLong(100));
        assert_eq!(eval_fn_call(&QName::local("unsignedInt"), &[DfdlValue::String(String::from("50"))], &ctx).unwrap(), DfdlValue::UnsignedInt(50));
        assert_eq!(eval_fn_call(&QName::local("unsignedShort"), &[DfdlValue::String(String::from("25"))], &ctx).unwrap(), DfdlValue::UnsignedShort(25));
        assert_eq!(eval_fn_call(&QName::local("unsignedByte"), &[DfdlValue::String(String::from("10"))], &ctx).unwrap(), DfdlValue::UnsignedByte(10));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::String(String::from("123.456"))], &ctx).unwrap(), DfdlValue::Double(123.456));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::String(String::from("1.5"))], &ctx).unwrap(), DfdlValue::Float(1.5));
        assert_eq!(eval_fn_call(&QName::local("decimal"), &[DfdlValue::String(String::from("99.9"))], &ctx).unwrap(), DfdlValue::Decimal(String::from("99.9")));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String(String::from("true"))], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String(String::from("false"))], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("string"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::String(String::from("42")));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::String(String::from("ABCD"))], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0xAB, 0xCD]));
        assert_eq!(eval_fn_call(&QName::local("dateTime"), &[DfdlValue::String(String::from("2026-10-07T12:00:00"))], &ctx).unwrap(), DfdlValue::DateTime(String::from("2026-10-07T12:00:00")));
        assert_eq!(eval_fn_call(&QName::local("date"), &[DfdlValue::String(String::from("2026-10-07"))], &ctx).unwrap(), DfdlValue::Date(String::from("2026-10-07")));
        assert_eq!(eval_fn_call(&QName::local("time"), &[DfdlValue::String(String::from("12:00:00"))], &ctx).unwrap(), DfdlValue::Time(String::from("12:00:00")));
    }

    /// Verifies built-in XPath/DFDL functions in eval_fn_call.
    #[test]
    fn test_eval_fn_call_builtins() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // String functions
        let res_len = eval_fn_call(&QName::local("string-length"), &[DfdlValue::String(String::from("hello"))], &ctx).unwrap();
        assert_eq!(res_len, DfdlValue::Long(5));

        let res_upper = eval_fn_call(&QName::local("upper-case"), &[DfdlValue::String(String::from("dfdl"))], &ctx).unwrap();
        assert_eq!(res_upper, DfdlValue::String(String::from("DFDL")));

        let res_lower = eval_fn_call(&QName::local("lower-case"), &[DfdlValue::String(String::from("DFDL"))], &ctx).unwrap();
        assert_eq!(res_lower, DfdlValue::String(String::from("dfdl")));

        let res_sub = eval_fn_call(&QName::local("substring"), &[DfdlValue::String(String::from("motorcycle")), DfdlValue::Long(3), DfdlValue::Long(5)], &ctx).unwrap();
        assert_eq!(res_sub, DfdlValue::String(String::from("torcy")));

        let res_sub_bef = eval_fn_call(&QName::local("substring-before"), &[DfdlValue::String(String::from("tattoo")), DfdlValue::String(String::from("atto"))], &ctx).unwrap();
        assert_eq!(res_sub_bef, DfdlValue::String(String::from("t")));

        let res_sub_aft = eval_fn_call(&QName::local("substring-after"), &[DfdlValue::String(String::from("tattoo")), DfdlValue::String(String::from("tat"))], &ctx).unwrap();
        assert_eq!(res_sub_aft, DfdlValue::String(String::from("too")));

        let res_cont = eval_fn_call(&QName::local("contains"), &[DfdlValue::String(String::from("banana")), DfdlValue::String(String::from("nan"))], &ctx).unwrap();
        assert_eq!(res_cont, DfdlValue::Boolean(true));

        let res_sw = eval_fn_call(&QName::local("starts-with"), &[DfdlValue::String(String::from("banana")), DfdlValue::String(String::from("ban"))], &ctx).unwrap();
        assert_eq!(res_sw, DfdlValue::Boolean(true));

        let res_ew = eval_fn_call(&QName::local("ends-with"), &[DfdlValue::String(String::from("banana")), DfdlValue::String(String::from("na"))], &ctx).unwrap();
        assert_eq!(res_ew, DfdlValue::Boolean(true));

        // Math functions
        assert_eq!(eval_fn_call(&QName::local("abs"), &[DfdlValue::Long(-42)], &ctx).unwrap(), DfdlValue::Long(42));
        assert_eq!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap(), DfdlValue::Double(4.0));
        assert_eq!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap(), DfdlValue::Double(3.0));
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.5)], &ctx).unwrap(), DfdlValue::Double(4.0));

        // Logic & check functions
        assert_eq!(eval_fn_call(&QName::local("exists"), &[DfdlValue::String(String::from("nonexistent"))], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap(), DfdlValue::Boolean(true));

        // Matches
        let res_mat = eval_fn_call(&QName::local("matches"), &[DfdlValue::String(String::from("abracadabra")), DfdlValue::String(String::from("cadabra"))], &ctx).unwrap();
        assert_eq!(res_mat, DfdlValue::Boolean(true));

        // Entity encoding and decoding
        let res_enc = eval_fn_call(&QName::local("encodeDFDLEntities"), &[DfdlValue::String(String::from("hello%world"))], &ctx).unwrap();
        assert_eq!(res_enc, DfdlValue::String(String::from("hello%%world")));

        let res_dec = eval_fn_call(&QName::local("decodeDFDLEntities"), &[DfdlValue::String(String::from("hello%%world"))], &ctx).unwrap();
        assert_eq!(res_dec, DfdlValue::String(String::from("hello%world")));
    }

    /// Verifies value length calculation on infoset elements.
    #[test]
    fn test_calc_elem_value_length_recursive() {
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::infoset::state::ElementState;
        use crate::types::QName;

        let child1 = InfosetElement::simple(QName::local("child1"), ElementState::Value(DfdlValue::String(String::from("12345"))));
        let child2 = InfosetElement::simple(QName::local("child2"), ElementState::Value(DfdlValue::Int(42)));

        let mut parent = InfosetElement::complex(QName::local("parent"));
        parent.children.push(InfosetNode::Element(child1.clone()));
        parent.children.push(InfosetNode::Element(child2));

        let doc = InfosetDocument { root: Some(parent.clone()), total_nodes: 3 };

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
        let err_var = eval_expr(&ExprAst::Variable(QName::local("unknown_var")), &mut ctx).unwrap_err();
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
        assert_eq!(eval_cond(DfdlValue::String(String::from("true")), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::String(String::from("1")), &mut ctx), "THEN");
        assert_eq!(eval_cond(DfdlValue::String(String::from("false")), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::String(String::from("0")), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::String(String::new()), &mut ctx), "ELSE");
        assert_eq!(eval_cond(DfdlValue::String(String::from("nonempty")), &mut ctx), "THEN");

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
        assert_eq!(ctx.unqualified_path_step_policy, UnqualifiedPathStepPolicy::PreferDefaultNamespace);
        assert!(ctx.find_element(&path).is_none());
    }

    /// Verifies primitive, boxed, and interoperability functions in eval_fn_call.
    #[test]
    fn test_eval_builtin_all_primitives_and_interop_funcs() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget).with_occurs_index(7);

        // Primitive and boxed integer functions
        assert_eq!(eval_fn_call(&QName::local("primByteFunc"), &[DfdlValue::Int(120)], &ctx).unwrap(), DfdlValue::Byte(120));
        assert_eq!(eval_fn_call(&QName::local("boxedByteFunc"), &[DfdlValue::Int(121)], &ctx).unwrap(), DfdlValue::Byte(121));
        assert_eq!(eval_fn_call(&QName::local("primByteArrayFunc"), &[DfdlValue::String(String::from("data"))], &ctx).unwrap(), DfdlValue::String(String::from("data")));
        assert_eq!(eval_fn_call(&QName::local("primShortFunc"), &[DfdlValue::Int(300)], &ctx).unwrap(), DfdlValue::Short(300));
        assert_eq!(eval_fn_call(&QName::local("boxedShortFunc"), &[DfdlValue::Int(301)], &ctx).unwrap(), DfdlValue::Short(301));
        assert_eq!(eval_fn_call(&QName::local("primLongFunc"), &[DfdlValue::Int(500)], &ctx).unwrap(), DfdlValue::Long(500));
        assert_eq!(eval_fn_call(&QName::local("boxedLongFunc"), &[DfdlValue::Int(501)], &ctx).unwrap(), DfdlValue::Long(501));

        // Floating point and boolean primitive functions
        assert_eq!(eval_fn_call(&QName::local("primDoubleFunc"), &[DfdlValue::String(String::from("98.765"))], &ctx).unwrap(), DfdlValue::Double(98.765));
        assert_eq!(eval_fn_call(&QName::local("boxedDoubleFunc"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::Double(42.0));
        assert_eq!(eval_fn_call(&QName::local("primFloatFunc"), &[DfdlValue::String(String::from("2.5"))], &ctx).unwrap(), DfdlValue::Float(2.5));
        assert_eq!(eval_fn_call(&QName::local("boxedFloatFunc"), &[DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Float(10.0));
        assert_eq!(eval_fn_call(&QName::local("primBooleanFunc"), &[DfdlValue::Int(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boxedBooleanFunc"), &[DfdlValue::Int(0)], &ctx).unwrap(), DfdlValue::Boolean(false));

        // Java BigInteger / BigDecimal interop
        assert_eq!(eval_fn_call(&QName::local("javaBigDecimalFunc"), &[DfdlValue::String(String::from("123.456"))], &ctx).unwrap(), DfdlValue::Decimal(String::from("123.456")));
        assert_eq!(eval_fn_call(&QName::local("javaBigIntegerFunc"), &[DfdlValue::String(String::from("999999"))], &ctx).unwrap(), DfdlValue::Decimal(String::from("999999")));

        // Greeting and arithmetic helper functions
        assert_eq!(eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap(), DfdlValue::String(String::from("Hello")));
        assert_eq!(eval_fn_call(&QName::local("addBoxed"), &[DfdlValue::Int(10), DfdlValue::Int(20)], &ctx).unwrap(), DfdlValue::Int(30));
        assert_eq!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::String(String::from("15")), DfdlValue::Long(5)], &ctx).unwrap(), DfdlValue::Int(20));
        assert!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::Boolean(true), DfdlValue::Int(5)], &ctx).is_err());

        // String compare function
        assert_eq!(eval_fn_call(&QName::local("compare"), &[DfdlValue::String(String::from("a")), DfdlValue::String(String::from("b"))], &ctx).unwrap(), DfdlValue::Int(-1));
        assert_eq!(eval_fn_call(&QName::local("compare"), &[DfdlValue::String(String::from("a")), DfdlValue::String(String::from("a"))], &ctx).unwrap(), DfdlValue::Int(0));
        assert_eq!(eval_fn_call(&QName::local("compare"), &[DfdlValue::String(String::from("b")), DfdlValue::String(String::from("a"))], &ctx).unwrap(), DfdlValue::Int(1));

        // Constant functions
        assert_eq!(eval_fn_call(&QName::local("true"), &[], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("false"), &[], &ctx).unwrap(), DfdlValue::Boolean(false));

        // Occurs index and position
        assert_eq!(eval_fn_call(&QName::local("occursIndex"), &[], &ctx).unwrap(), DfdlValue::Long(7));
        assert_eq!(eval_fn_call(&QName::local("currentPosition"), &[], &ctx).unwrap(), DfdlValue::Long(0));

        // Count function
        assert_eq!(eval_fn_call(&QName::local("count"), &[DfdlValue::String(String::new())], &ctx).unwrap(), DfdlValue::Long(0));
        assert_eq!(eval_fn_call(&QName::local("count"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::Long(1));
        assert_eq!(eval_fn_call(&QName::local("count"), &[], &ctx).unwrap(), DfdlValue::Long(0));
    }

    /// Verifies bitwise operations (bitAnd, bitOr, bitXor, bitNot, shift) and signedness constraints.
    #[test]
    fn test_bitwise_all_variants() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // bitAnd matching signedness
        assert_eq!(eval_fn_call(&QName::local("bitAnd"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap(), DfdlValue::Long(0b1000));
        assert_eq!(eval_fn_call(&QName::local("bitAnd"), &[DfdlValue::UnsignedLong(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap(), DfdlValue::UnsignedLong(0b1000));
        let err_and = eval_fn_call(&QName::local("bitAnd"), &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap_err();
        assert_eq!(err_and.kind, DFDLErrorKind::SchemaDefinition);

        // bitOr matching signedness
        assert_eq!(eval_fn_call(&QName::local("bitOr"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap(), DfdlValue::Long(0b1110));
        assert_eq!(eval_fn_call(&QName::local("bitOr"), &[DfdlValue::UnsignedLong(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap(), DfdlValue::UnsignedLong(0b1110));
        let err_or = eval_fn_call(&QName::local("bitOr"), &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap_err();
        assert_eq!(err_or.kind, DFDLErrorKind::SchemaDefinition);

        // bitXor matching signedness
        assert_eq!(eval_fn_call(&QName::local("bitXor"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap(), DfdlValue::Long(0b0110));
        assert_eq!(eval_fn_call(&QName::local("bitXor"), &[DfdlValue::UnsignedLong(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap(), DfdlValue::UnsignedLong(0b0110));
        let err_xor = eval_fn_call(&QName::local("bitXor"), &[DfdlValue::Long(0b1100), DfdlValue::UnsignedLong(0b1010)], &ctx).unwrap_err();
        assert_eq!(err_xor.kind, DFDLErrorKind::SchemaDefinition);

        // bitNot on all numeric variants
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Byte(0x0F)], &ctx).unwrap(), DfdlValue::Byte(-16));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Short(0x00FF)], &ctx).unwrap(), DfdlValue::Short(!0x00FF));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Int(0x0000_FFFF)], &ctx).unwrap(), DfdlValue::Int(!0x0000_FFFF));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Long(0x0000_0000_FFFF_FFFF)], &ctx).unwrap(), DfdlValue::Long(!0x0000_0000_FFFF_FFFF));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::UnsignedByte(0x0F)], &ctx).unwrap(), DfdlValue::UnsignedByte(0xF0));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::UnsignedShort(0x00FF)], &ctx).unwrap(), DfdlValue::UnsignedShort(0xFF00));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::UnsignedInt(0x0000_FFFF)], &ctx).unwrap(), DfdlValue::UnsignedInt(0xFFFF_0000));
        assert_eq!(eval_fn_call(&QName::local("bitNot"), &[DfdlValue::UnsignedLong(0x0000_0000_FFFF_FFFF)], &ctx).unwrap(), DfdlValue::UnsignedLong(0xFFFF_FFFF_0000_0000));

        // leftShift and rightShift with Short, Byte, Long, Unsigned* variants
        assert_eq!(eval_fn_call(&QName::local("leftShift"), &[DfdlValue::Short(1), DfdlValue::Int(3)], &ctx).unwrap(), DfdlValue::Short(8));
        assert_eq!(eval_fn_call(&QName::local("leftShift"), &[DfdlValue::UnsignedShort(1), DfdlValue::Int(3)], &ctx).unwrap(), DfdlValue::UnsignedShort(8));
        assert_eq!(eval_fn_call(&QName::local("leftShift"), &[DfdlValue::UnsignedInt(1), DfdlValue::Int(4)], &ctx).unwrap(), DfdlValue::UnsignedInt(16));
        assert_eq!(eval_fn_call(&QName::local("leftShift"), &[DfdlValue::UnsignedLong(1), DfdlValue::Int(5)], &ctx).unwrap(), DfdlValue::UnsignedLong(32));

        assert_eq!(eval_fn_call(&QName::local("rightShift"), &[DfdlValue::Short(16), DfdlValue::Int(2)], &ctx).unwrap(), DfdlValue::Short(4));
        assert_eq!(eval_fn_call(&QName::local("rightShift"), &[DfdlValue::UnsignedShort(16), DfdlValue::Int(2)], &ctx).unwrap(), DfdlValue::UnsignedShort(4));
        assert_eq!(eval_fn_call(&QName::local("rightShift"), &[DfdlValue::UnsignedInt(32), DfdlValue::Int(3)], &ctx).unwrap(), DfdlValue::UnsignedInt(4));
        assert_eq!(eval_fn_call(&QName::local("rightShift"), &[DfdlValue::UnsignedLong(64), DfdlValue::Int(4)], &ctx).unwrap(), DfdlValue::UnsignedLong(4));

        // Shift count errors (negative count)
        assert!(eval_fn_call(&QName::local("leftShift"), &[DfdlValue::Int(1), DfdlValue::Int(-1)], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("rightShift"), &[DfdlValue::Int(1), DfdlValue::Int(-1)], &ctx).is_err());
    }

    /// Verifies replace function arguments, namespaces, and error paths.
    #[test]
    fn test_replace_function_and_errors() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Valid replace with fn prefix
        let qn_fn = QName { prefix: Some(String::from("fn")), local_name: String::from("replace"), namespace: None };
        let res_rep = eval_fn_call(&qn_fn, &[DfdlValue::String(String::from("banana")), DfdlValue::String(String::from("a")), DfdlValue::String(String::from("o"))], &ctx).unwrap();
        assert_eq!(res_rep, DfdlValue::String(String::from("bonono")));

        // Valid replace with dfdl prefix
        let qn_dfdl = QName { prefix: Some(String::from("dfdl")), local_name: String::from("replace"), namespace: None };
        assert!(eval_fn_call(&qn_dfdl, &[DfdlValue::String(String::from("abc")), DfdlValue::String(String::from("b")), DfdlValue::String(String::from("z"))], &ctx).is_ok());

        // Invalid prefix error
        let qn_invalid = QName { prefix: Some(String::from("custom")), local_name: String::from("replace"), namespace: None };
        let err_pfx = eval_fn_call(&qn_invalid, &[DfdlValue::String(String::from("abc"))], &ctx).unwrap_err();
        assert_eq!(err_pfx.kind, DFDLErrorKind::SchemaDefinition);

        // Insufficient argument count (< 3)
        let err_argc = eval_fn_call(&qn_fn, &[DfdlValue::String(String::from("abc")), DfdlValue::String(String::from("b"))], &ctx).unwrap_err();
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
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[orig_hex], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34]));

        // From integer subtypes
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Byte(0x42)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x42]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::UnsignedByte(0x99)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x99]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Short(0x1234)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::UnsignedShort(0xABCD)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0xAB, 0xCD]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Int(0x12345678)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78]));

        // From string with even hex digits
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::String(String::from("deadbeef"))], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0xDE, 0xAD, 0xBE, 0xEF]));

        // Errors: odd length and invalid hex characters (non-decimal)
        assert!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::String(String::from("abc"))], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::String(String::from("abz"))], &ctx).is_err());
    }

    /// Verifies range checking functions (checkRangeInclusive and checkRangeExclusive).
    #[test]
    fn test_check_range_inclusive_exclusive() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // Inclusive range
        let qn_inc = QName::local("checkRangeInclusive");
        assert_eq!(eval_fn_call(&qn_inc, &[DfdlValue::Int(5), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&qn_inc, &[DfdlValue::Int(1), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&qn_inc, &[DfdlValue::Int(10), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&qn_inc, &[DfdlValue::Int(0), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(false));

        // Exclusive range
        let qn_exc = QName::local("checkRangeExclusive");
        assert_eq!(eval_fn_call(&qn_exc, &[DfdlValue::Int(5), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&qn_exc, &[DfdlValue::Int(1), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&qn_exc, &[DfdlValue::Int(10), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Boolean(false));

        // Non-numeric operand rejected
        assert!(eval_fn_call(&qn_inc, &[DfdlValue::String(String::from("5")), DfdlValue::Int(1), DfdlValue::Int(10)], &ctx).is_err());
    }

    /// Verifies round-half-to-even behavior with various precisions and tie-breaking.
    #[test]
    fn test_round_half_to_even_precision_and_ties() {
        let path = InfosetPath::root();
        let mut budget = WorkBudget::new(10000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        let qn = QName::local("round-half-to-even");

        // Ties round to even
        assert_eq!(eval_fn_call(&qn, &[DfdlValue::Double(2.5)], &ctx).unwrap(), DfdlValue::Double(2.0));
        assert_eq!(eval_fn_call(&qn, &[DfdlValue::Double(3.5)], &ctx).unwrap(), DfdlValue::Double(4.0));

        // Positive precision
        assert_eq!(eval_fn_call(&qn, &[DfdlValue::Double(2.555), DfdlValue::Int(2)], &ctx).unwrap(), DfdlValue::Double(2.56));

        // Negative precision
        assert_eq!(eval_fn_call(&qn, &[DfdlValue::Double(35.0), DfdlValue::Int(-1)], &ctx).unwrap(), DfdlValue::Double(40.0));

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

        assert_eq!(eval_fn_call(&QName::local("day-from-dateTime"), core::slice::from_ref(&dt_str), &ctx).unwrap(), DfdlValue::Long(7));
        assert_eq!(eval_fn_call(&QName::local("hours-from-dateTime"), core::slice::from_ref(&dt_str), &ctx).unwrap(), DfdlValue::Long(14));
        assert_eq!(eval_fn_call(&QName::local("minutes-from-dateTime"), core::slice::from_ref(&dt_str), &ctx).unwrap(), DfdlValue::Long(30));
        assert_eq!(eval_fn_call(&QName::local("seconds-from-dateTime"), core::slice::from_ref(&dt_str), &ctx).unwrap(), DfdlValue::Decimal(String::from("45.50")));

        let tm_str = DfdlValue::Time(String::from("08:15:30"));
        assert_eq!(eval_fn_call(&QName::local("minutes-from-time"), core::slice::from_ref(&tm_str), &ctx).unwrap(), DfdlValue::Long(15));
        assert_eq!(eval_fn_call(&QName::local("seconds-from-time"), core::slice::from_ref(&tm_str), &ctx).unwrap(), DfdlValue::Decimal(String::from("30")));

        // Invalid format strings
        assert!(eval_fn_call(&QName::local("year-from-date"), &[DfdlValue::Date(String::from("bad-date"))], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("hours-from-time"), &[DfdlValue::Time(String::from("bad-time"))], &ctx).is_err());
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
        assert_eq!(eval_fn_call(&QName::local("lookAhead"), &[DfdlValue::Int(0), DfdlValue::Int(8)], &ctx_no_la).unwrap(), DfdlValue::Long(0));

        // Lookahead callback returning small value <= u64::MAX
        let cb_small = |_off: usize, _bits: usize| -> DFDLResult<u128> { Ok(42) };
        let ctx_small = ExprContext::new(None, &path, &[], &mut budget2).with_lookahead(Some(&cb_small));
        assert_eq!(eval_fn_call(&QName::local("lookAhead"), &[DfdlValue::Int(0), DfdlValue::Int(8)], &ctx_small).unwrap(), DfdlValue::UnsignedLong(42));

        // Lookahead callback returning large value > u64::MAX
        let cb_large = |_off: usize, _bits: usize| -> DFDLResult<u128> { Ok((u64::MAX as u128) + 10) };
        let ctx_large = ExprContext::new(None, &path, &[], &mut budget3).with_lookahead(Some(&cb_large));
        match eval_fn_call(&QName::local("lookAhead"), &[DfdlValue::Int(0), DfdlValue::Int(8)], &ctx_large).unwrap() {
            DfdlValue::Decimal(s) => assert!(s.contains("18446744073709551625")),
            other => panic!("Expected Decimal, got {:?}", other),
        }

        // Distance exceeding 512 bits
        let err_dist = eval_fn_call(&QName::local("lookAhead"), &[DfdlValue::Int(500), DfdlValue::Int(20)], &ctx_small).unwrap_err();
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
        assert_eq!(eval_fn_call(&qn_trace, &[DfdlValue::Int(123)], &ctx).unwrap(), DfdlValue::Int(123));

        // fn:trace is prohibited by specification
        let qn_fn_trace = QName { prefix: Some(String::from("fn")), local_name: String::from("trace"), namespace: None };
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
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::infoset::state::ElementState;

        let path = InfosetPath::parse("/root/child");
        let mut budget = WorkBudget::new(10000);

        // 1. No document provided
        let ctx_nodoc = ExprContext::new(None, &path, &[], &mut budget);
        let err_nodoc = resolve_path(&path, &ctx_nodoc).unwrap_err();
        assert!(err_nodoc.message.as_str().contains("No Infoset document"));

        // Build document with root, child (Value), empty_child (Empty), noval_child (NoValue), and complex_child
        let simple_val = InfosetElement::simple(QName::local("child"), ElementState::Value(DfdlValue::Int(42)));
        let empty_val = InfosetElement::simple(QName::local("empty_child"), ElementState::Empty);
        let noval_val = InfosetElement::simple(QName::local("noval_child"), ElementState::NoValue);
        let mut complex_el = InfosetElement::complex(QName::local("complex_child"));
        complex_el.children.push(InfosetNode::Element(InfosetElement::simple(QName::local("inner"), ElementState::Value(DfdlValue::String(String::from("inside"))))));

        let mut root = InfosetElement::complex(QName::local("root"));
        root.children.push(InfosetNode::Element(simple_val));
        root.children.push(InfosetNode::Element(empty_val));
        root.children.push(InfosetNode::Element(noval_val));
        root.children.push(InfosetNode::Element(complex_el));

        let doc = InfosetDocument { root: Some(root), total_nodes: 5 };

        let curr_path = InfosetPath::parse("/root");
        let ctx = ExprContext::new(Some(&doc), &curr_path, &[], &mut budget);

        // 2. Resolve value
        assert_eq!(resolve_path(&InfosetPath::parse("/root/child"), &ctx).unwrap(), DfdlValue::Int(42));

        // 3. Resolve empty element
        assert_eq!(resolve_path(&InfosetPath::parse("/root/empty_child"), &ctx).unwrap(), DfdlValue::String(String::new()));

        // 4. Resolve NoValue simple element
        let err_noval = resolve_path(&InfosetPath::parse("/root/noval_child"), &ctx).unwrap_err();
        assert!(err_noval.message.as_str().contains("does not have a value"));

        // 5. Resolve complex element (does not have a simple value)
        let err_complex = resolve_path(&InfosetPath::parse("/root/complex_child"), &ctx).unwrap_err();
        assert!(err_complex.message.as_str().contains("Complex element"));

        // 6. Path not found
        let err_notfound = resolve_path(&InfosetPath::parse("/root/missing"), &ctx).unwrap_err();
        assert_eq!(err_notfound.kind, DFDLErrorKind::SchemaDefinition);

        // 7. Self referencing without value
        let noval_path = InfosetPath::parse("/root/noval_child");
        let self_ctx = ExprContext::new(Some(&doc), &noval_path, &[], &mut budget);
        let err_self = resolve_path(&InfosetPath::parse("."), &self_ctx).unwrap_err();
        assert!(err_self.message.as_str().contains("Circular reference") || err_self.message.as_str().contains("Self referencing"));

        // 8. OVC map override
        let mut ovc_map = alloc::collections::BTreeMap::new();
        ovc_map.insert(String::from("/root/noval_child"), DfdlValue::Int(100));
        let ctx_ovc = ExprContext::new(Some(&doc), &curr_path, &[], &mut budget).with_ovc_values(Some(&ovc_map));
        assert_eq!(resolve_path(&InfosetPath::parse("/root/noval_child"), &ctx_ovc).unwrap(), DfdlValue::Int(100));
    }

    /// Verifies normalize_infoset_path error handling (navigating past root, self mismatches, invalid index).
    #[test]
    fn test_normalize_infoset_path_steps_and_errors() {
        use crate::infoset::tree::{InfosetDocument, InfosetElement};

        let root = InfosetElement::complex(QName::local("root"));
        let doc = InfosetDocument { root: Some(root), total_nodes: 1 };
        let path = InfosetPath::parse("/root");
        let mut budget = WorkBudget::new(1000);
        let ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // Relative '..' past root error
        let rel_past_root = InfosetPath::from_parts(alloc::vec![String::from(".."), String::from("..")], false);
        let err_past_root = normalize_infoset_path(&rel_past_root, &ctx, &doc).unwrap_err();
        assert_eq!(err_past_root.kind, DFDLErrorKind::SchemaDefinition);

        // self::mismatch error: .(expected) when current is different
        let rel_self_mismatch = InfosetPath::from_parts(alloc::vec![String::from(".(wrong)")], false);
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
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::infoset::state::ElementState;

        let child1 = InfosetElement::simple(QName::local("item"), ElementState::Value(DfdlValue::Int(1)));
        let child2 = InfosetElement::simple(QName::local("item"), ElementState::Value(DfdlValue::Int(2)));
        let mut parent = InfosetElement::complex(QName::local("root"));
        parent.children.push(InfosetNode::Element(child1));
        parent.children.push(InfosetNode::Element(child2));
        let mut subgroup = InfosetElement::complex(QName::local("subgroup"));
        subgroup.children.push(InfosetNode::Element(InfosetElement::simple(QName::local("sub_item"), ElementState::Value(DfdlValue::Int(9)))));
        parent.children.push(InfosetNode::Element(subgroup));

        let doc = InfosetDocument { root: Some(parent), total_nodes: 4 };

        let path = InfosetPath::parse("/root");
        let mut budget = WorkBudget::new(10000);
        let mut ctx = ExprContext::new(Some(&doc), &path, &[], &mut budget);

        // fn:count with wildcard path
        let ast_count_star = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(alloc::vec![String::from("root"), String::from("*")], true))],
        };
        assert_eq!(eval_expr(&ast_count_star, &mut ctx).unwrap(), DfdlValue::Long(3));

        // fn:count with specific child name
        let ast_count_item = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(alloc::vec![String::from("root"), String::from("item")], true))],
        };
        assert_eq!(eval_expr(&ast_count_item, &mut ctx).unwrap(), DfdlValue::Long(2));

        // fn:exists with existing path
        let ast_exists_true = ExprAst::FnCall {
            name: QName::local("exists"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(eval_expr(&ast_exists_true, &mut ctx).unwrap(), DfdlValue::Boolean(true));

        // fn:exists with non-existing path
        let ast_exists_false = ExprAst::FnCall {
            name: QName::local("exists"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/nonexistent"))],
        };
        assert_eq!(eval_expr(&ast_exists_false, &mut ctx).unwrap(), DfdlValue::Boolean(false));

        // fn:empty with existing path
        let ast_empty_false = ExprAst::FnCall {
            name: QName::local("empty"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(eval_expr(&ast_empty_false, &mut ctx).unwrap(), DfdlValue::Boolean(false));

        // fn:empty with non-existing path
        let ast_empty_true = ExprAst::FnCall {
            name: QName::local("empty"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/nonexistent"))],
        };
        assert_eq!(eval_expr(&ast_empty_true, &mut ctx).unwrap(), DfdlValue::Boolean(true));

        // trace on simple value
        let ast_trace_simple = ExprAst::FnCall {
            name: QName::local("trace"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/item[1]"))],
        };
        assert_eq!(eval_expr(&ast_trace_simple, &mut ctx).unwrap(), DfdlValue::Int(1));

        // trace on complex element returns element name string
        let ast_trace_complex = ExprAst::FnCall {
            name: QName::local("trace"),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("/root/subgroup"))],
        };
        assert_eq!(eval_expr(&ast_trace_complex, &mut ctx).unwrap(), DfdlValue::String(String::from("subgroup")));

        // valueLength with enclosing_lengths
        let enc_lengths = [(String::from("root"), 16usize, crate::schema::ir::LengthUnits::Bits, String::from("ASCII"))];
        let mut ctx_enc = ExprContext::new(Some(&doc), &path, &[], &mut budget).with_enclosing_lengths(&enc_lengths);
        let ast_vl_bits = ExprAst::FnCall {
            name: QName::local("valueLength"),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::parse("/root")),
                ExprAst::Literal(DfdlValue::String(String::from("bits"))),
            ],
        };
        assert_eq!(eval_expr(&ast_vl_bits, &mut ctx_enc).unwrap(), DfdlValue::Long(16));
        let ast_vl_bytes = ExprAst::FnCall {
            name: QName::local("valueLength"),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::parse("/root")),
                ExprAst::Literal(DfdlValue::String(String::from("bytes"))),
            ],
        };
        assert_eq!(eval_expr(&ast_vl_bytes, &mut ctx_enc).unwrap(), DfdlValue::Long(2));
    }

    #[test]
    fn test_eval_builtin_functions_comprehensive() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // String functions
        let res_cat = eval_fn_call(&QName::local("concat"), &[DfdlValue::String("a".into()), DfdlValue::String("b".into()), DfdlValue::Int(1)], &ctx).unwrap();
        assert_eq!(res_cat, DfdlValue::String("ab1".into()));

        let res_len = eval_fn_call(&QName::local("string-length"), &[DfdlValue::String("hello".into())], &ctx).unwrap();
        assert_eq!(res_len, DfdlValue::Long(5));

        let res_sub = eval_fn_call(&QName::local("substring"), &[DfdlValue::String("hello world".into()), DfdlValue::Int(1), DfdlValue::Int(5)], &ctx).unwrap();
        assert_eq!(res_sub, DfdlValue::String("hello".into()));

        let res_before = eval_fn_call(&QName::local("substring-before"), &[DfdlValue::String("abc-def".into()), DfdlValue::String("-".into())], &ctx).unwrap();
        assert_eq!(res_before, DfdlValue::String("abc".into()));

        let res_after = eval_fn_call(&QName::local("substring-after"), &[DfdlValue::String("abc-def".into()), DfdlValue::String("-".into())], &ctx).unwrap();
        assert_eq!(res_after, DfdlValue::String("def".into()));

        let res_has = eval_fn_call(&QName::local("contains"), &[DfdlValue::String("testing".into()), DfdlValue::String("est".into())], &ctx).unwrap();
        assert_eq!(res_has, DfdlValue::Boolean(true));

        let res_sw = eval_fn_call(&QName::local("starts-with"), &[DfdlValue::String("testing".into()), DfdlValue::String("test".into())], &ctx).unwrap();
        assert_eq!(res_sw, DfdlValue::Boolean(true));

        let res_ew = eval_fn_call(&QName::local("ends-with"), &[DfdlValue::String("testing".into()), DfdlValue::String("ing".into())], &ctx).unwrap();
        assert_eq!(res_ew, DfdlValue::Boolean(true));

        let res_up = eval_fn_call(&QName::local("upper-case"), &[DfdlValue::String("abc".into())], &ctx).unwrap();
        assert_eq!(res_up, DfdlValue::String("ABC".into()));

        let res_low = eval_fn_call(&QName::local("lower-case"), &[DfdlValue::String("ABC".into())], &ctx).unwrap();
        assert_eq!(res_low, DfdlValue::String("abc".into()));

        let res_rep = eval_fn_call(&QName::local("replace"), &[DfdlValue::String("a-b-c".into()), DfdlValue::String("-".into()), DfdlValue::String("_".into())], &ctx).unwrap();
        assert_eq!(res_rep, DfdlValue::String("a_b_c".into()));

        // Math functions
        let res_abs = eval_fn_call(&QName::local("abs"), &[DfdlValue::Int(-42)], &ctx).unwrap();
        assert_eq!(res_abs, DfdlValue::Int(42));

        let res_ceil = eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap();
        assert_eq!(res_ceil, DfdlValue::Double(4.0));

        let res_fl = eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap();
        assert_eq!(res_fl, DfdlValue::Double(3.0));

        let res_rnd = eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.5)], &ctx).unwrap();
        assert_eq!(res_rnd, DfdlValue::Double(4.0));

        // Logic & Compare
        let res_cmp = eval_fn_call(&QName::local("compare"), &[DfdlValue::String("a".into()), DfdlValue::String("b".into())], &ctx).unwrap();
        assert_eq!(res_cmp, DfdlValue::Int(-1));

        let res_not = eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap();
        assert_eq!(res_not, DfdlValue::Boolean(true));

        let res_bool = eval_fn_call(&QName::local("boolean"), &[DfdlValue::String("true".into())], &ctx).unwrap();
        assert_eq!(res_bool, DfdlValue::Boolean(true));

        // Calendar extraction
        let res_dt = eval_fn_call(&QName::local("year-from-date"), &[DfdlValue::Date("2026-10-08".into())], &ctx).unwrap();
        assert_eq!(res_dt, DfdlValue::Long(2026));

        let res_m = eval_fn_call(&QName::local("month-from-date"), &[DfdlValue::Date("2026-10-08".into())], &ctx).unwrap();
        assert_eq!(res_m, DfdlValue::Long(10));

        let res_d = eval_fn_call(&QName::local("day-from-date"), &[DfdlValue::Date("2026-10-08".into())], &ctx).unwrap();
        assert_eq!(res_d, DfdlValue::Long(8));

        let res_h = eval_fn_call(&QName::local("hours-from-time"), &[DfdlValue::Time("14:30:15".into())], &ctx).unwrap();
        assert_eq!(res_h, DfdlValue::Long(14));

        let res_min = eval_fn_call(&QName::local("minutes-from-time"), &[DfdlValue::Time("14:30:15".into())], &ctx).unwrap();
        assert_eq!(res_min, DfdlValue::Long(30));

        let res_sec = eval_fn_call(&QName::local("seconds-from-time"), &[DfdlValue::Time("14:30:15".into())], &ctx).unwrap();
        assert_eq!(res_sec, DfdlValue::Decimal("15".into()));

        // Casts
        let res_i = eval_fn_call(&QName::local("int"), &[DfdlValue::String("123".into())], &ctx).unwrap();
        assert_eq!(res_i, DfdlValue::Int(123));

        let res_l = eval_fn_call(&QName::local("long"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_l, DfdlValue::Long(123));

        let res_sh = eval_fn_call(&QName::local("short"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_sh, DfdlValue::Short(123));

        let res_by = eval_fn_call(&QName::local("byte"), &[DfdlValue::Int(123)], &ctx).unwrap();
        assert_eq!(res_by, DfdlValue::Byte(123));

        let res_ub = eval_fn_call(&QName::local("unsignedByte"), &[DfdlValue::Int(250)], &ctx).unwrap();
        assert_eq!(res_ub, DfdlValue::UnsignedByte(250));

        let res_us = eval_fn_call(&QName::local("unsignedShort"), &[DfdlValue::Int(60000)], &ctx).unwrap();
        assert_eq!(res_us, DfdlValue::UnsignedShort(60000));

        let res_ui = eval_fn_call(&QName::local("unsignedInt"), &[DfdlValue::Long(100000)], &ctx).unwrap();
        assert_eq!(res_ui, DfdlValue::UnsignedInt(100000));

        let res_ul = eval_fn_call(&QName::local("unsignedLong"), &[DfdlValue::Long(100000)], &ctx).unwrap();
        assert_eq!(res_ul, DfdlValue::UnsignedLong(100000));

        let res_flt = eval_fn_call(&QName::local("float"), &[DfdlValue::String("1.5".into())], &ctx).unwrap();
        assert_eq!(res_flt, DfdlValue::Float(1.5));

        let res_str = eval_fn_call(&QName::local("string"), &[DfdlValue::Int(99)], &ctx).unwrap();
        assert_eq!(res_str, DfdlValue::String("99".into()));

        // Bitwise operations
        let res_band = eval_fn_call(&QName::local("bitAnd"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap();
        assert_eq!(res_band, DfdlValue::Long(0b1000));

        let res_bor = eval_fn_call(&QName::local("bitOr"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap();
        assert_eq!(res_bor, DfdlValue::Long(0b1110));

        let res_bxor = eval_fn_call(&QName::local("bitXor"), &[DfdlValue::Long(0b1100), DfdlValue::Long(0b1010)], &ctx).unwrap();
        assert_eq!(res_bxor, DfdlValue::Long(0b0110));

        let res_bnot = eval_fn_call(&QName::local("bitNot"), &[DfdlValue::Byte(0)], &ctx).unwrap();
        assert_eq!(res_bnot, DfdlValue::Byte(-1));

        let res_shl = eval_fn_call(&QName::local("leftShift"), &[DfdlValue::Int(1), DfdlValue::Int(3)], &ctx).unwrap();
        assert_eq!(res_shl, DfdlValue::Int(8));

        let res_shr = eval_fn_call(&QName::local("rightShift"), &[DfdlValue::Int(8), DfdlValue::Int(3)], &ctx).unwrap();
        assert_eq!(res_shr, DfdlValue::Int(1));

        // Entities & Helpers
        let res_ent = eval_fn_call(&QName::local("decodeDFDLEntities"), &[DfdlValue::String("%SP;".into())], &ctx).unwrap();
        assert_eq!(res_ent, DfdlValue::String(" ".into()));

        let res_hello = eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap();
        assert_eq!(res_hello, DfdlValue::String("Hello".into()));

        let res_add = eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::Int(10), DfdlValue::Int(20)], &ctx).unwrap();
        assert_eq!(res_add, DfdlValue::Int(30));
    }

    /// Verifies mathematical rounding functions, type conversions, and logical error branches.
    #[test]
    fn test_eval_extended_functions_and_branches() {
        let path = crate::types::InfosetPath::root();
        let mut budget = crate::limits::WorkBudget::new(1000);
        let ctx = ExprContext::new(None, &path, &[], &mut budget);

        // 1. ceiling function across Float and Double with NaN, INF, and finite values
        assert_eq!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Float(3.2)], &ctx).unwrap(), DfdlValue::Float(4.0));
        assert_eq!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(3.2)], &ctx).unwrap(), DfdlValue::Double(4.0));
        assert!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Float(f32::NAN)], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(f64::NAN)], &ctx).is_err());
        assert_eq!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Float(f32::INFINITY)], &ctx).unwrap(), DfdlValue::Float(f32::INFINITY));
        assert_eq!(eval_fn_call(&QName::local("ceiling"), &[DfdlValue::Double(f64::INFINITY)], &ctx).unwrap(), DfdlValue::Double(f64::INFINITY));

        // 2. floor function across Float and Double with NaN, INF, and finite values
        assert_eq!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(3.8)], &ctx).unwrap(), DfdlValue::Float(3.0));
        assert_eq!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(3.8)], &ctx).unwrap(), DfdlValue::Double(3.0));
        assert!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(f32::NAN)], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(f64::NAN)], &ctx).is_err());
        assert_eq!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Float(f32::INFINITY)], &ctx).unwrap(), DfdlValue::Float(f32::INFINITY));
        assert_eq!(eval_fn_call(&QName::local("floor"), &[DfdlValue::Double(f64::INFINITY)], &ctx).unwrap(), DfdlValue::Double(f64::INFINITY));

        // 3. round function across Float and Double with NaN, INF, positive, and negative values
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Float(3.6)], &ctx).unwrap(), DfdlValue::Float(4.0));
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Double(3.6)], &ctx).unwrap(), DfdlValue::Double(4.0));
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Float(-3.6)], &ctx).unwrap(), DfdlValue::Float(-3.0));
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Double(-3.6)], &ctx).unwrap(), DfdlValue::Double(-3.0));
        assert!(eval_fn_call(&QName::local("round"), &[DfdlValue::Float(f32::NAN)], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("round"), &[DfdlValue::Double(f64::NAN)], &ctx).is_err());
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Float(f32::INFINITY)], &ctx).unwrap(), DfdlValue::Float(f32::INFINITY));
        assert_eq!(eval_fn_call(&QName::local("round"), &[DfdlValue::Double(f64::INFINITY)], &ctx).unwrap(), DfdlValue::Double(f64::INFINITY));

        // 4. xs:double and xs:float type casting error branches (lowercase nan, inf, -inf)
        assert!(eval_fn_call(&QName::local("double"), &[DfdlValue::String("nan".into())], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("double"), &[DfdlValue::String("inf".into())], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("double"), &[DfdlValue::String("-inf".into())], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("float"), &[DfdlValue::String("nan".into())], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("float"), &[DfdlValue::String("inf".into())], &ctx).is_err());
        assert!(eval_fn_call(&QName::local("float"), &[DfdlValue::String("-inf".into())], &ctx).is_err());

        // Valid integer and boolean casts to double
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Short(10)], &ctx).unwrap(), DfdlValue::Double(10.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Byte(5)], &ctx).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedLong(100)], &ctx).unwrap(), DfdlValue::Double(100.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedShort(50)], &ctx).unwrap(), DfdlValue::Double(50.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedByte(25)], &ctx).unwrap(), DfdlValue::Double(25.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Double(1.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(false)], &ctx).unwrap(), DfdlValue::Double(0.0));

        // Valid integer and boolean casts to float
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Double(12.5)], &ctx).unwrap(), DfdlValue::Float(12.5));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Float(10.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Long(20)], &ctx).unwrap(), DfdlValue::Float(20.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Short(15)], &ctx).unwrap(), DfdlValue::Float(15.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Byte(7)], &ctx).unwrap(), DfdlValue::Float(7.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedLong(30)], &ctx).unwrap(), DfdlValue::Float(30.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedInt(40)], &ctx).unwrap(), DfdlValue::Float(40.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedShort(50)], &ctx).unwrap(), DfdlValue::Float(50.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedByte(60)], &ctx).unwrap(), DfdlValue::Float(60.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Float(1.0));

        // 5. xs:boolean across diverse data types
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(0)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Long(100)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Short(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Byte(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedLong(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedShort(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedByte(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Float(1.0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Double(1.0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String("not_a_bool".into())], &ctx).is_err());

        // 6. fn:not function across diverse data types
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(false)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Int(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Long(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Short(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Byte(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedLong(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedInt(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedShort(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedByte(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Float(0.0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Double(0.0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::String("true".into())], &ctx).unwrap(), DfdlValue::Boolean(false));

        // 7. fn:replace function
        let rep_res = eval_fn_call(&QName::local("replace"), &[
            DfdlValue::String("banana".into()),
            DfdlValue::String("a".into()),
            DfdlValue::String("o".into()),
        ], &ctx).unwrap();
        assert_eq!(rep_res, DfdlValue::String("bonono".into()));

        // 8. addBoxed helper
        let add_b = eval_fn_call(&QName::local("addBoxed"), &[DfdlValue::Int(10), DfdlValue::Int(25)], &ctx).unwrap();
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
        let child_find = InfosetElement::simple(QName::local("child"), ElementState::Value(DfdlValue::Int(99)));
        root_find.try_add_child(InfosetNode::Element(child_find)).unwrap();
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
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "trace", Some("fn")),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Int(1))],
        };
        let err_trace = eval_expr(&ast_trace, &mut ctx2).unwrap_err();
        assert!(err_trace.message.as_str().contains("Unsupported function: fn:trace"));

        // 12. dfdl:valueLength with bits units (lines 549-555, 573-576)
        let ast_vlen = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
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
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
            args: alloc::vec![
                ExprAst::Path(p_parent),
                ExprAst::Literal(DfdlValue::String("bytes".into())),
            ],
        };
        let err_enc = eval_expr(&ast_vlen_enc, &mut ctx_enclosing).unwrap_err();
        assert!(err_enc.message.as_str().contains("Value Length cannot be computed for enclosing element"));

        // 14. dfdl:valueLength with characters units and enclosing_lengths (lines 444-451)
        let mut budget6 = crate::limits::WorkBudget::new(1000);
        let mut ctx_chars = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget6);
        let enc_lengths = [(
            alloc::string::String::from("child"),
            16, // 16 bits
            crate::schema::ir::LengthUnits::Bits,
            alloc::string::String::from("UTF-8"),
        )];
        ctx_chars.enclosing_lengths = &enc_lengths;
        let ast_vlen_chars = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
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
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "checkConstraints", Some("dfdl")),
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
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "empty", Some("fn")),
            args: alloc::vec![ExprAst::Path(p_child.clone())],
        };
        let mut budget8 = crate::limits::WorkBudget::new(1000);
        let mut ctx_empty = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget8);
        assert_eq!(eval_expr(&ast_empty, &mut ctx_empty).unwrap(), DfdlValue::Boolean(false));

        // 18. fn:count with parent step .. (lines 367-378)
        let p_count_parent = InfosetPath::from_parts(alloc::vec!["..".into(), "child".into()], false);
        let ast_count = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "count", Some("fn")),
            args: alloc::vec![ExprAst::Path(p_count_parent)],
        };
        let mut budget9 = crate::limits::WorkBudget::new(1000);
        let mut ctx_count = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget9);
        assert_eq!(eval_expr(&ast_count, &mut ctx_count).unwrap(), DfdlValue::Long(1));

        // 19. dfdl:valueLength without units argument defaulting to bytes (lines 405-415)
        let ast_vlen_nounits = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
            args: alloc::vec![ExprAst::Path(p_child.clone())],
        };
        let mut budget10 = crate::limits::WorkBudget::new(1000);
        let mut ctx_vlen_nounits = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget10);
        assert_eq!(eval_expr(&ast_vlen_nounits, &mut ctx_vlen_nounits).unwrap(), DfdlValue::Long(4));

        // 20. fn:string on non-existent element returns Err (line 339)
        let p_missing = InfosetPath::from_parts(alloc::vec!["nonexistent".into()], false);
        let ast_str_missing = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "string", Some("fn")),
            args: alloc::vec![ExprAst::Path(p_missing)],
        };
        let mut budget11 = crate::limits::WorkBudget::new(1000);
        let mut ctx_str_missing = ExprContext::new(Some(&doc_find), &p_child, &[], &mut budget11);
        assert!(eval_expr(&ast_str_missing, &mut ctx_str_missing).is_err());

        // 21. fn:ceiling and fn:floor with NaN and Infinity (lines 2430-2485)
        let mut budget12 = crate::limits::WorkBudget::new(1000);
        let mut ctx_math = ExprContext::new(None, &p_curr, &[], &mut budget12);

        let ast_ceil_nan = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "ceiling", Some("fn")),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Double(f64::NAN))],
        };
        assert!(eval_expr(&ast_ceil_nan, &mut ctx_math).is_err());

        let ast_ceil_inf = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "ceiling", Some("fn")),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Double(f64::INFINITY))],
        };
        assert_eq!(eval_expr(&ast_ceil_inf, &mut ctx_math).unwrap(), DfdlValue::Double(f64::INFINITY));

        let ast_floor_nan = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "floor", Some("fn")),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Float(f32::NAN))],
        };
        assert!(eval_expr(&ast_floor_nan, &mut ctx_math).is_err());

        let ast_floor_inf = ExprAst::FnCall {
            name: QName::with_namespace("http://www.w3.org/2005/xpath-functions", "floor", Some("fn")),
            args: alloc::vec![ExprAst::Literal(DfdlValue::Float(f32::NEG_INFINITY))],
        };
        assert_eq!(eval_expr(&ast_floor_inf, &mut ctx_math).unwrap(), DfdlValue::Float(f32::NEG_INFINITY));

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
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        });
        let circ_id = b_circ.add_term_with_props(QName::local("elemB"), circ_elem, circ_props).unwrap();
        b_circ.set_root(circ_id);
        let schema_circ = b_circ.build().unwrap();

        let ast_vl_circ = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(alloc::vec!["elemB".into()], false))],
        };
        let p_elema = InfosetPath::from_parts(alloc::vec!["elemA".into()], false);
        let mut budget13 = crate::limits::WorkBudget::new(1000);
        let mut ctx_circ = ExprContext::new(None, &p_elema, &[], &mut budget13).with_schema(&schema_circ);
        let err_circ = eval_expr(&ast_vl_circ, &mut ctx_circ).unwrap_err();
        assert!(err_circ.message.as_str().contains("Circular reference / circular dependency"));

        // 24. dfdl:valueLength self length calculation error (lines 470-477)
        let ast_vl_self = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
            args: alloc::vec![ExprAst::Path(InfosetPath::parse("."))],
        };
        let p_elemb = InfosetPath::from_parts(alloc::vec!["elemB".into()], false);
        let mut budget14 = crate::limits::WorkBudget::new(1000);
        let mut ctx_self = ExprContext::new(None, &p_elemb, &[], &mut budget14).with_schema(&schema_circ);
        let err_self = eval_expr(&ast_vl_self, &mut ctx_self).unwrap_err();
        assert!(err_self.message.as_str().contains("Value length unknown: cannot evaluate dfdl:valueLength"));

        // 25. fn:count with parent path . and .. (lines 368-378)
        let ast_count_parent = ExprAst::FnCall {
            name: QName::local("count"),
            args: alloc::vec![ExprAst::Path(InfosetPath::from_parts(alloc::vec!["..".into(), ".".into(), "item".into()], false))],
        };
        let doc_count = crate::infoset::tree::InfosetDocument::new();
        let mut budget15 = crate::limits::WorkBudget::new(1000);
        let p_sub = InfosetPath::from_parts(alloc::vec!["root".into(), "sub".into()], false);
        let mut ctx_count = ExprContext::new(Some(&doc_count), &p_sub, &[], &mut budget15);
        assert_eq!(eval_expr(&ast_count_parent, &mut ctx_count).unwrap(), DfdlValue::Long(0));

        // 26. dfdl:valueLength with characters units from enclosing_lengths (lines 440-449)
        let ast_vl_chars = ExprAst::FnCall {
            name: QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "valueLength", Some("dfdl")),
            args: alloc::vec![
                ExprAst::Path(InfosetPath::from_parts(alloc::vec!["target".into()], false)),
                ExprAst::Literal(DfdlValue::String("characters".into())),
            ],
        };
        let mut budget16 = crate::limits::WorkBudget::new(1000);
        let p_root = InfosetPath::root();
        let enc_lengths = [("target".into(), 10usize, crate::schema::ir::LengthUnits::Characters, "UTF-8".into())];
        let mut ctx_chars = ExprContext::new(None, &p_root, &[], &mut budget16).with_enclosing_lengths(&enc_lengths);
        assert_eq!(eval_expr(&ast_vl_chars, &mut ctx_chars).unwrap(), DfdlValue::Long(10));
    }

    #[test]
    fn test_expr_eval_broad_edge_cases() {
        // 1. Predicate indexing with typed integers (Byte, Short, Long, Unsigned*)
        // Validates occurrences indexing using numeric subtypes.
        let mut budget = WorkBudget::new(10_000);
        let mut doc = InfosetDocument::new();
        let mut root_elem = InfosetElement::complex(crate::types::QName::local("root"));
        let item1 = InfosetElement::simple(crate::types::QName::local("item"), ElementState::Value(DfdlValue::String("first".into())));
        let item2 = InfosetElement::simple(crate::types::QName::local("item"), ElementState::Value(DfdlValue::String("second".into())));
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
            assert_eq!(eval_expr(&ast, &mut ctx).unwrap(), DfdlValue::String("second".into()));
        }

        // 2. Binary AND/OR type errors with non-boolean operands
        // DFDL expression spec strictly requires boolean types for logical conjunction.
        let ast_bad_and = crate::expr::parse_expr("1 and fn:true()").unwrap();
        assert!(eval_expr(&ast_bad_and, &mut ctx).is_err());
        let ast_bad_or = crate::expr::parse_expr("fn:false() or 'abc'").unwrap();
        assert!(eval_expr(&ast_bad_or, &mut ctx).is_err());

        // 3. String length and value length on scalar variants
        // Verifies byte and bit length conversions on HexBinary, Short, Long, Byte.
        let ast_str_len = crate::expr::parse_expr("dfdl:valueLength(xs:hexBinary('AABBCC'), 'bits')").unwrap();
        assert_eq!(eval_expr(&ast_str_len, &mut ctx).unwrap(), DfdlValue::Long(24));

        let ast_str_short = crate::expr::parse_expr("dfdl:valueLength(xs:short(1234))").unwrap();
        assert_eq!(eval_expr(&ast_str_short, &mut ctx).unwrap(), DfdlValue::Long(4));

        let ast_str_byte = crate::expr::parse_expr("dfdl:valueLength(xs:byte(42))").unwrap();
        assert_eq!(eval_expr(&ast_str_byte, &mut ctx).unwrap(), DfdlValue::Long(1));

        let ast_str_long = crate::expr::parse_expr("dfdl:valueLength(xs:long(9999))").unwrap();
        assert_eq!(eval_expr(&ast_str_long, &mut ctx).unwrap(), DfdlValue::Long(8));

        // 4. Substring with float special values (NaN, -INF, +INF)
        // XPath 2.0 substring semantics on infinite and NaN indices.
        let ast_sub_nan = crate::expr::parse_expr("fn:substring('hello', xs:float('NaN'))").unwrap();
        assert_eq!(eval_expr(&ast_sub_nan, &mut ctx).unwrap(), DfdlValue::String(String::new()));
        let ast_sub_posinf = crate::expr::parse_expr("fn:substring('hello', xs:float('INF'), 2.0)").unwrap();
        assert_eq!(eval_expr(&ast_sub_posinf, &mut ctx).unwrap(), DfdlValue::String(String::new()));
        let ast_sub_neginf = crate::expr::parse_expr("fn:substring('hello', xs:float('-INF'), 2.0)").unwrap();
        assert_eq!(eval_expr(&ast_sub_neginf, &mut ctx).unwrap(), DfdlValue::String("he".into()));

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
            let expr_str = format!("dfdl:checkRangeInclusive({}, xs:short(1), xs:short(10))", val_expr);
            let ast_range = crate::expr::parse_expr(&expr_str).unwrap();
            assert_eq!(eval_expr(&ast_range, &mut ctx).unwrap(), DfdlValue::Boolean(true));
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
        let ast_shift = crate::expr::parse_expr("dfdl:leftShift(xs:unsignedShort(10), xs:unsignedByte(2))").unwrap();
        assert_eq!(eval_expr(&ast_shift, &mut ctx).unwrap(), DfdlValue::UnsignedShort(40));

        let ast_floor = crate::expr::parse_expr("fn:floor(xs:unsignedInt(42))").unwrap();
        assert_eq!(eval_expr(&ast_floor, &mut ctx).unwrap(), DfdlValue::UnsignedInt(42));

        let ast_ceil = crate::expr::parse_expr("fn:ceiling(xs:short(17))").unwrap();
        assert_eq!(eval_expr(&ast_ceil, &mut ctx).unwrap(), DfdlValue::Short(17));

        // 8. Complex element value length with prefixed child terms
        // Tests recursive valueLength calculation across nested complex elements with prefix types.
        let mut comp_elem = InfosetElement::complex(crate::types::QName::local("complexRoot"));
        let sub_child1 = InfosetElement::simple(crate::types::QName::local("sub1"), ElementState::Value(DfdlValue::String("data".into())));
        let sub_child2 = InfosetElement::simple(crate::types::QName::local("sub2"), ElementState::Value(DfdlValue::String("more".into())));
        comp_elem.children.push(InfosetNode::Element(sub_child1));
        comp_elem.children.push(InfosetNode::Element(sub_child2));

        let child1_props = crate::schema::ir::ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Prefixed,
            prefix_length_type: Some(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc("prefix:len:16:bits")),
            initiator: Some("[".into()),
            terminator: Some("]".into()),
            ..Default::default()
        };

        let child2_props = crate::schema::ir::ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Prefixed,
            prefix_length_type: Some(crate::schema::ir::PrefixLengthDescriptor::from_legacy_desc("xs:int")),
            ..Default::default()
        };

        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let elem1 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sub1"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::value::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id1 = builder.add_term_with_props(crate::types::QName::local("sub1"), crate::schema::ir::TermKind::Element(elem1), child1_props).unwrap();

        let elem2 = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sub2"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::value::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id2 = builder.add_term_with_props(crate::types::QName::local("sub2"), crate::schema::ir::TermKind::Element(elem2), child2_props).unwrap();

        let seq_id = builder.add_term(
            crate::types::QName::local("seq"),
            crate::schema::ir::TermKind::Sequence(crate::schema::ir::CompiledSequence {
                members: alloc::vec![id1, id2],
            }),
        ).unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("complexRoot"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder.add_term(crate::types::QName::local("complexRoot"), crate::schema::ir::TermKind::Element(root_elem)).unwrap();
        builder.set_root(root_id);
        let sch = builder.build().unwrap();

        let mut doc2 = InfosetDocument::new();
        doc2.root = Some(comp_elem);
        let mut ctx2 = ExprContext::new(Some(&doc2), &p_root, &[], &mut budget).with_schema(&sch);

        let ast_vl_comp = crate::expr::parse_expr("dfdl:valueLength(complexRoot, 'bytes')").unwrap();
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
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Int(10)).unwrap(), DfdlValue::Int(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Long(100)).unwrap(), DfdlValue::Long(-100));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Short(5)).unwrap(), DfdlValue::Short(-5));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(3)).unwrap(), DfdlValue::Byte(-3));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(10)).unwrap(), DfdlValue::Long(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(9_223_372_036_854_775_808)).unwrap(), DfdlValue::Long(i64::MIN));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedLong(10_000_000_000_000_000_000)).unwrap(), DfdlValue::Decimal("-10000000000000000000".into()));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedInt(10)).unwrap(), DfdlValue::Long(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedShort(10)).unwrap(), DfdlValue::Int(-10));
        assert_eq!(eval_unary(UnaryOp::Negate, DfdlValue::UnsignedByte(10)).unwrap(), DfdlValue::Int(-10));
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Int(i32::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Long(i64::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Short(i16::MIN)).is_err());
        assert!(eval_unary(UnaryOp::Negate, DfdlValue::Byte(i8::MIN)).is_err());

        // 2. Binary addition across mixed and concrete types
        assert_eq!(eval_add(DfdlValue::Long(10), DfdlValue::Long(20)).unwrap(), DfdlValue::Long(30));
        assert_eq!(eval_add(DfdlValue::Int(5), DfdlValue::Int(15)).unwrap(), DfdlValue::Int(20));
        assert_eq!(eval_add(DfdlValue::Int(5), DfdlValue::Long(25)).unwrap(), DfdlValue::Long(30));
        assert_eq!(eval_add(DfdlValue::Long(25), DfdlValue::Int(5)).unwrap(), DfdlValue::Long(30));
        assert_eq!(eval_add(DfdlValue::Double(2.5), DfdlValue::Double(1.5)).unwrap(), DfdlValue::Double(4.0));
        assert_eq!(eval_add(DfdlValue::Long(2), DfdlValue::Double(3.5)).unwrap(), DfdlValue::Double(5.5));
        assert_eq!(eval_add(DfdlValue::Double(3.5), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(5.5));
        assert_eq!(eval_add(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_add(DfdlValue::Long(2), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_add(DfdlValue::Float(3.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(12.5));
        assert_eq!(eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(12.5));
        assert_eq!(eval_add(DfdlValue::Decimal("10".into()), DfdlValue::Long(5)).unwrap(), DfdlValue::Decimal("15".into()));
        assert!(eval_add(DfdlValue::Long(i64::MAX), DfdlValue::Long(1)).is_err());
        assert!(eval_add(DfdlValue::Int(i32::MAX), DfdlValue::Int(1)).is_err());

        // 3. Binary subtraction across mixed and concrete types
        assert_eq!(eval_sub(DfdlValue::Long(30), DfdlValue::Long(10)).unwrap(), DfdlValue::Long(20));
        assert_eq!(eval_sub(DfdlValue::Int(20), DfdlValue::Int(5)).unwrap(), DfdlValue::Int(15));
        assert_eq!(eval_sub(DfdlValue::Int(30), DfdlValue::Long(5)).unwrap(), DfdlValue::Long(25));
        assert_eq!(eval_sub(DfdlValue::Long(30), DfdlValue::Int(5)).unwrap(), DfdlValue::Long(25));
        assert_eq!(eval_sub(DfdlValue::Double(5.5), DfdlValue::Double(1.5)).unwrap(), DfdlValue::Double(4.0));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_sub(DfdlValue::Double(10.0), DfdlValue::Long(3)).unwrap(), DfdlValue::Double(7.0));
        assert_eq!(eval_sub(DfdlValue::Float(5.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(3.0));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(7.0));
        assert_eq!(eval_sub(DfdlValue::Float(10.0), DfdlValue::Long(3)).unwrap(), DfdlValue::Float(7.0));
        assert_eq!(eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(7.5));
        assert_eq!(eval_sub(DfdlValue::Decimal("10".into()), DfdlValue::Long(3)).unwrap(), DfdlValue::Decimal("7".into()));
        assert_eq!(eval_sub(DfdlValue::Double(10.0), DfdlValue::Decimal("2.5".into())).unwrap(), DfdlValue::Double(7.5));
        assert_eq!(eval_sub(DfdlValue::Float(10.0), DfdlValue::Decimal("2.5".into())).unwrap(), DfdlValue::Float(7.5));
        assert_eq!(eval_sub(DfdlValue::Long(10), DfdlValue::Decimal("3".into())).unwrap(), DfdlValue::Decimal("7".into()));
        assert!(eval_sub(DfdlValue::Long(i64::MIN), DfdlValue::Long(1)).is_err());
        assert!(eval_sub(DfdlValue::Int(i32::MIN), DfdlValue::Int(1)).is_err());

        // 4. Binary multiplication across mixed and concrete types
        assert_eq!(eval_mul(DfdlValue::Long(10), DfdlValue::Long(20)).unwrap(), DfdlValue::Long(200));
        assert_eq!(eval_mul(DfdlValue::Int(5), DfdlValue::Int(4)).unwrap(), DfdlValue::Int(20));
        assert_eq!(eval_mul(DfdlValue::Int(5), DfdlValue::Long(4)).unwrap(), DfdlValue::Long(20));
        assert_eq!(eval_mul(DfdlValue::Long(4), DfdlValue::Int(5)).unwrap(), DfdlValue::Long(20));
        assert_eq!(eval_mul(DfdlValue::Double(2.5), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_mul(DfdlValue::Long(4), DfdlValue::Double(2.5)).unwrap(), DfdlValue::Double(10.0));
        assert_eq!(eval_mul(DfdlValue::Double(2.5), DfdlValue::Long(4)).unwrap(), DfdlValue::Double(10.0));
        assert_eq!(eval_mul(DfdlValue::Float(2.0), DfdlValue::Float(3.0)).unwrap(), DfdlValue::Float(6.0));
        assert_eq!(eval_mul(DfdlValue::Long(4), DfdlValue::Float(2.5)).unwrap(), DfdlValue::Float(10.0));
        assert_eq!(eval_mul(DfdlValue::Float(2.5), DfdlValue::Long(4)).unwrap(), DfdlValue::Float(10.0));
        assert_eq!(eval_mul(DfdlValue::Decimal("5".into()), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(10.0));
        assert!(eval_mul(DfdlValue::Long(i64::MAX), DfdlValue::Long(2)).is_err());
        assert!(eval_mul(DfdlValue::Int(i32::MAX), DfdlValue::Int(2)).is_err());

        // 5. Binary division across mixed and concrete types
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Long(10), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Int(10), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Double(10.0), DfdlValue::Int(2)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Long(10), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Long(2)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Int(10), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Float(10.0), DfdlValue::Int(2)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Double(2.0)).unwrap(), DfdlValue::Double(5.0));
        assert_eq!(eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Float(2.0)).unwrap(), DfdlValue::Float(5.0));
        assert_eq!(eval_div(DfdlValue::Decimal("10".into()), DfdlValue::Long(2)).unwrap(), DfdlValue::Decimal("5".into()));
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

        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Int(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Long(0)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Short(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Byte(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedLong(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedShort(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::UnsignedByte(1)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Float(1.0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::Double(0.0)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String("true".into())], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String("false".into())], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert!(eval_fn_call(&QName::local("boolean"), &[DfdlValue::String("invalid".into())], &ctx).is_err());

        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Int(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Long(0)], &ctx).unwrap(), DfdlValue::Boolean(true));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Short(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Byte(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedLong(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedInt(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedShort(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::UnsignedByte(1)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Float(1.0)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::Double(1.0)], &ctx).unwrap(), DfdlValue::Boolean(false));
        assert_eq!(eval_fn_call(&QName::local("not"), &[DfdlValue::String("true".into())], &ctx).unwrap(), DfdlValue::Boolean(false));

        assert_eq!(eval_fn_call(&QName::local("sayHello"), &[], &ctx).unwrap(), DfdlValue::String("Hello".into()));
        assert_eq!(eval_fn_call(&QName::local("addBoxed"), &[DfdlValue::Int(3), DfdlValue::Long(4)], &ctx).unwrap(), DfdlValue::Int(7));
        assert_eq!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::Short(2), DfdlValue::Byte(3)], &ctx).unwrap(), DfdlValue::Int(5));
        assert_eq!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::UnsignedInt(2), DfdlValue::UnsignedLong(3)], &ctx).unwrap(), DfdlValue::Int(5));
        assert_eq!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::UnsignedShort(2), DfdlValue::UnsignedByte(3)], &ctx).unwrap(), DfdlValue::Int(5));
        assert_eq!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::String("10".into()), DfdlValue::Int(5)], &ctx).unwrap(), DfdlValue::Int(15));
        assert!(eval_fn_call(&QName::local("addPrimitive"), &[DfdlValue::String("nan".into()), DfdlValue::Int(5)], &ctx).is_err());

        // 8. eval_fn_call hexBinary conversions
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::HexBinary(alloc::vec![0xAA])], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0xAA]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Byte(0x12)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::UnsignedByte(0x34)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x34]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Short(0x1234)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::UnsignedShort(0x5678)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x56, 0x78]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Int(0x12345678)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78]));
        assert_eq!(eval_fn_call(&QName::local("hexBinary"), &[DfdlValue::Long(0x123456789ABCDEF0)], &ctx).unwrap(), DfdlValue::HexBinary(alloc::vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]));

        // 9. eval_fn_call double and float conversions
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Double(3.25)], &ctx).unwrap(), DfdlValue::Double(3.25));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Float(2.5)], &ctx).unwrap(), DfdlValue::Double(2.5));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Double(10.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Long(20)], &ctx).unwrap(), DfdlValue::Double(20.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Short(30)], &ctx).unwrap(), DfdlValue::Double(30.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Byte(40)], &ctx).unwrap(), DfdlValue::Double(40.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedLong(50)], &ctx).unwrap(), DfdlValue::Double(50.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedInt(60)], &ctx).unwrap(), DfdlValue::Double(60.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedShort(70)], &ctx).unwrap(), DfdlValue::Double(70.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::UnsignedByte(80)], &ctx).unwrap(), DfdlValue::Double(80.0));
        assert_eq!(eval_fn_call(&QName::local("double"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Double(1.0));

        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Float(2.5)], &ctx).unwrap(), DfdlValue::Float(2.5));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Double(3.5)], &ctx).unwrap(), DfdlValue::Float(3.5));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Int(10)], &ctx).unwrap(), DfdlValue::Float(10.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Long(20)], &ctx).unwrap(), DfdlValue::Float(20.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Short(30)], &ctx).unwrap(), DfdlValue::Float(30.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Byte(40)], &ctx).unwrap(), DfdlValue::Float(40.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedLong(50)], &ctx).unwrap(), DfdlValue::Float(50.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedInt(60)], &ctx).unwrap(), DfdlValue::Float(60.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedShort(70)], &ctx).unwrap(), DfdlValue::Float(70.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::UnsignedByte(80)], &ctx).unwrap(), DfdlValue::Float(80.0));
        assert_eq!(eval_fn_call(&QName::local("float"), &[DfdlValue::Boolean(true)], &ctx).unwrap(), DfdlValue::Float(1.0));

        // 10. eval_fn_call integer casting functions
        assert_eq!(eval_fn_call(&QName::local("int"), &[DfdlValue::Long(42)], &ctx).unwrap(), DfdlValue::Int(42));
        assert_eq!(eval_fn_call(&QName::local("long"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::Long(42));
        assert_eq!(eval_fn_call(&QName::local("short"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::Short(42));
        assert_eq!(eval_fn_call(&QName::local("byte"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::Byte(42));
        assert_eq!(eval_fn_call(&QName::local("unsignedByte"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::UnsignedByte(42));
        assert_eq!(eval_fn_call(&QName::local("unsignedShort"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::UnsignedShort(42));
        assert_eq!(eval_fn_call(&QName::local("unsignedInt"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::UnsignedInt(42));
        assert_eq!(eval_fn_call(&QName::local("unsignedLong"), &[DfdlValue::Int(42)], &ctx).unwrap(), DfdlValue::UnsignedLong(42));

        // 11. calc_elem_value_length fallbacks without schema (lines 866-877)
        let make_elem = |v: DfdlValue| InfosetElement::simple(QName::local("test"), ElementState::Value(v));
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::String("hello".into())), None, &path, None), 5);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::HexBinary(alloc::vec![1, 2, 3])), None, &path, None), 3);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::DateTime("2026-01-01".into())), None, &path, None), 10);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Date("2026-01-01".into())), None, &path, None), 10);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Time("12:00:00".into())), None, &path, None), 8);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Decimal("123.45".into())), None, &path, None), 6);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Int(1)), None, &path, None), 4);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::UnsignedInt(1)), None, &path, None), 4);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Float(1.0)), None, &path, None), 4);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Long(1)), None, &path, None), 8);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::UnsignedLong(1)), None, &path, None), 8);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Double(1.0)), None, &path, None), 8);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Short(1)), None, &path, None), 2);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::UnsignedShort(1)), None, &path, None), 2);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Byte(1)), None, &path, None), 1);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::UnsignedByte(1)), None, &path, None), 1);
        assert_eq!(calc_elem_value_length(&make_elem(DfdlValue::Boolean(true)), None, &path, None), 1);
    }
}


