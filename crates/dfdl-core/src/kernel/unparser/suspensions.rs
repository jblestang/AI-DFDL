//! DFDL unparser variable map and runtime property evaluation.

extern crate alloc;

use crate::error::DFDLResult;
use crate::infoset::value::DfdlValue;
use crate::io::traits::ByteSink;
use crate::schema::CompiledTerm;
use super::UnparserEngine;

impl<'a, S: ByteSink> UnparserEngine<'a, S> {
    pub(crate) fn make_expr_context(&mut self) -> crate::expr::ExprContext<'_> {
        let mut path = self.base_path.clone();
        for &(seg, idx) in &self.path_stack {
            let step = crate::types::PathStep {
                axis: crate::types::PathAxis::Child,
                target: crate::types::StepTarget::Unprefixed(alloc::string::ToString::to_string(seg)),
                index_predicate: idx,
                predicate_expr: None,
            };
            let _ = path.try_push_step(step);
        }
        self.current_path = path;

        let vmap = self
            .active_variable_map
            .as_ref()
            .unwrap_or(&self.schema.variable_map);
        crate::expr::ExprContext::with_variable_map(
            self.doc,
            &self.current_path,
            &[],
            Some(vmap),
            self.budget,
        )
        .with_schema(self.schema)
        .with_occurs_index(self.current_occurs_index)
        .with_namespaces(self.current_namespaces)
        .with_ovc_values(Some(&self.ovc_values))
        .for_unparsing()
    }

    pub(crate) fn execute_set_variables(&mut self, term: &CompiledTerm) -> DFDLResult<()> {
        if !term.properties.set_variables_compiled.is_empty() {
            for (var_name, ast_opt, raw_str) in &term.properties.set_variables_compiled {
                let evaluated = if let Some(ref ast) = ast_opt {
                    let mut ctx = self.make_expr_context();
                    crate::expr::eval_expr(ast, &mut ctx)?
                } else {
                    DfdlValue::String(raw_str.clone())
                };
                self.variable_map_mut()
                    .set_variable_validated(var_name, evaluated, false)?;
            }
        }
        Ok(())
    }

    /// Executes `dfdl:newVariableInstance` statements attached to `term` (§7.7).
    pub(crate) fn execute_new_variable_instances(&mut self, term: &CompiledTerm) -> DFDLResult<()> {
        for (var_name, ast_opt, raw_str_opt) in &term.properties.new_variable_instances_compiled {
            let local = var_name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&var_name.local_name);
            let is_parse_only = self
                .variable_map()
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
                .is_some_and(|var| var.direction == crate::expr::variables::VariableDirection::ParseOnly);
            if is_parse_only {
                continue;
            }
            let evaluated = if let Some(ref ast) = ast_opt {
                let mut ctx = self.make_expr_context();
                Some(crate::expr::eval_expr(ast, &mut ctx)?)
            } else if let Some(ref raw_str) = raw_str_opt {
                Some(DfdlValue::String(raw_str.clone()))
            } else {
                // DFDL §7.7 / §17: In unparsing, if no defaultValue was declared on newVariableInstance,
                // check if any member of the current sequence contains a forward dfdl:setVariable
                // for this variable instance that can be pre-computed from the input infoset.
                let mut pre_val = None;
                if let crate::schema::ir::TermKind::Sequence(ref seq) = term.kind {
                    let mut all_set_vars = alloc::vec::Vec::new();
                    for &m_id in &seq.members {
                        if let Some(m_term) = self.schema.get_term(m_id) {
                            all_set_vars.extend_from_slice(&m_term.properties.set_variables);
                        }
                    }
                    let var_local = var_name
                        .local_name
                        .split(':')
                        .next_back()
                        .unwrap_or(&var_name.local_name);
                    for (set_var, val_expr) in all_set_vars {
                        let set_local = set_var
                            .local_name
                            .split(':')
                            .next_back()
                            .unwrap_or(&set_var.local_name);
                        if set_local == var_local || set_var.local_name == var_name.local_name {
                            let trimmed = val_expr.trim();
                            if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                                let expr_body = trimmed
                                    .get(1..trimmed.len().saturating_sub(1))
                                    .unwrap_or("")
                                    .trim();
                                if let Ok(ast) = crate::expr::parse_expr(expr_body) {
                                    let mut ctx = self.make_expr_context();
                                    if let Ok(v) = crate::expr::eval_expr(&ast, &mut ctx) {
                                        pre_val = Some(v);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                pre_val
            };
            self.variable_map_mut()
                .new_variable_instance(var_name, evaluated)?;
        }
        Ok(())
    }

    /// Pops `dfdl:newVariableInstance` statements attached to `term` (§7.7).
    pub(crate) fn pop_new_variable_instances(&mut self, term: &CompiledTerm) {
        for (var_name, _) in term.properties.new_variable_instances.iter().rev() {
            let local = var_name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&var_name.local_name);
            let is_parse_only = self
                .variable_map()
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
                .is_some_and(|var| var.direction == crate::expr::variables::VariableDirection::ParseOnly);
            if is_parse_only {
                continue;
            }
            self.variable_map_mut().pop_variable_instance(var_name);
        }
    }


    pub(crate) fn eval_runtime_prop_str<'p>(&mut self, raw: &'p str) -> alloc::borrow::Cow<'p, str> {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
            if let Ok(prop) = crate::schema::ir::DfdlProp::parse_str(raw) {
                match prop {
                    crate::schema::ir::DfdlProp::Constant(_) => alloc::borrow::Cow::Borrowed(raw),
                    crate::schema::ir::DfdlProp::Expression { ref ast, .. } => {
                        let mut ctx = self.make_expr_context();
                        if let Ok(val) = crate::expr::eval_expr(ast, &mut ctx) {
                            alloc::borrow::Cow::Owned(alloc::format!("{}", val))
                        } else {
                            alloc::borrow::Cow::Borrowed(raw)
                        }
                    }
                }
            } else {
                alloc::borrow::Cow::Borrowed(raw)
            }
        } else {
            alloc::borrow::Cow::Borrowed(raw)
        }
    }

}
