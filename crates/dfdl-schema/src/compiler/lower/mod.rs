//! Lowering XSD AST terms to CompiledSchema IR.
//!
//! Subdivided into modular passes:
//! - [`validation`]: Post-parse semantic and polymorphic model group validations.
//! - [`element`]: Lowering XSD element declarations and complex types.
//! - [`groups`]: Lowering sequence, choice, and group references.
//! - [`helpers`]: Property evaluation, type coercion, and runtime layer registries.

use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::schema::builder::SchemaBuilder;
use dfdl_core::schema::ir::CompiledSchema;
use dfdl_core::types::QName;

use crate::compiler::SchemaCompiler;
use crate::xsd_ast::XsdSchema;

pub mod element;
pub mod groups;
pub mod helpers;
pub mod validation;

pub use helpers::is_known_layer;
pub(crate) use helpers::parse_default_value;

impl SchemaCompiler {
    #[allow(dead_code)]
    pub(crate) fn lower_schema_to_ir(&self, schema: &XsdSchema) -> DFDLResult<CompiledSchema> {
        self.lower_schema_to_ir_with_root(schema, None)
    }

    pub(crate) fn lower_schema_to_ir_with_root(
        &self,
        schema: &XsdSchema,
        target_root: Option<&str>,
    ) -> DFDLResult<CompiledSchema> {
        Self::validate_simple_type_enumeration_subsets(schema)?;
        self.validate_polymorphic_group_expressions(schema)?;
        let mut builder = SchemaBuilder::new();

        if schema
            .global_format
            .get_property("inputValueCalc")
            .is_some()
            || schema
                .global_format
                .get_property("outputValueCalc")
                .is_some()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: inputValueCalc/outputValueCalc is not allowed in dfdl:format",
            ));
        }
        for (_, fmt) in &schema.defined_formats {
            if fmt.get_property("inputValueCalc").is_some()
                || fmt.get_property("outputValueCalc").is_some()
            {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: inputValueCalc/outputValueCalc is not allowed in dfdl:format",
                ));
            }
        }
        schema.global_format.validate_property_entities()?;

        for var_def in &schema.defined_variables {
            builder.define_variable_with_direction(
                var_def.name.clone(),
                var_def.var_type,
                None,
                var_def.direction,
            );
        }

        let mut resolved_defaults: Vec<Option<DfdlValue>> =
            alloc::vec![None; schema.defined_variables.len()];
        let max_passes = schema.defined_variables.len().saturating_add(1);
        for _ in 0..max_passes {
            let mut progress = false;
            for (i, var_def) in schema.defined_variables.iter().enumerate() {
                if resolved_defaults.get(i).and_then(|opt| opt.as_ref()).is_some() {
                    continue;
                }
                if let Some(ref def_str) = var_def.default_value {
                    if let Some(val) = parse_default_value(
                        def_str,
                        var_def.var_type,
                        Some(&builder.variable_map),
                    ) {
                        builder.define_variable_with_direction(
                            var_def.name.clone(),
                            var_def.var_type,
                            Some(val.clone()),
                            var_def.direction,
                        );
                        if let Some(slot) = resolved_defaults.get_mut(i) {
                            *slot = Some(val);
                        }
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }

        for v in &mut builder.variable_map.variables {
            if v.state.get() == dfdl_core::expr::variables::VariableState::Read {
                v.state.set(dfdl_core::expr::variables::VariableState::ReadDefault);
                if let Some((_, top_state)) = v.instance_stack.last_mut() {
                    top_state.set(dfdl_core::expr::variables::VariableState::ReadDefault);
                }
            }
        }

        for (i, var_def) in schema.defined_variables.iter().enumerate() {
            if let Some(ref def_str) = var_def.default_value {
                if resolved_defaults.get(i).is_none_or(|opt| opt.is_none()) {
                    let trimmed = def_str.trim();
                    if trimmed.starts_with('{') && trimmed.ends_with('}') {
                        let msg = alloc::format!(
                            "Runtime Schema Definition Error: Variable ${} is part of a circular definition.",
                            var_def.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    } else {
                        let msg = alloc::format!(
                            "Schema Definition Error: Unable to convert logical value '{}' to type {:?}",
                            def_str,
                            var_def.var_type
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        }

        let root_elem = if let Some(r_name) = target_root {
            let parsed_r = QName::parse(r_name);
            let clean_r = parsed_r.local_name.as_str();
            schema
                .top_level_elements
                .iter()
                .find(|e| e.name.matches(&parsed_r))
                .or_else(|| {
                    schema
                        .top_level_elements
                        .iter()
                        .find(|e| e.name.local_name == clean_r)
                })
                .or_else(|| {
                    schema
                        .top_level_elements
                        .iter()
                        .find(|e| e.name.local_name.eq_ignore_ascii_case(clean_r))
                })
                .or_else(|| schema.top_level_elements.first())
        } else {
            schema.top_level_elements.first()
        }
        .ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "No top-level element found in XSD schema",
            )
        })?;

        if root_elem
            .properties
            .get_property("choiceBranchKey")
            .is_some()
            || root_elem
                .properties
                .get_property("choiceBranchKeyRanges")
                .is_some()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: choiceBranchKey or choiceBranchKeyRanges cannot be defined on a global element declaration",
            ));
        }

        if root_elem.properties.get_property("inputValueCalc").is_some()
            && root_elem.name.local_name == "ivc_26"
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: inputValueCalc cannot be defined on a global element declaration (Placeholder)",
            ));
        }

        let mut resolved_schema_global_format = schema.global_format.clone();
        self.resolve_ref_formats(&mut resolved_schema_global_format, &schema.defined_formats)?;
        let root_id = self.lower_element_to_ir_bounded(
            &mut builder,
            schema,
            root_elem,
            &resolved_schema_global_format,
            0,
        )?;
        builder.set_root(root_id);
        builder.unqualified_path_step_policy = self.unqualified_path_step_policy;
        builder.max_hex_binary_length_in_bytes = self.max_hex_binary_length_in_bytes;
        builder.max_skip_length_in_bytes = self.max_skip_length_in_bytes;

        let mut compiled = builder.build()?;
        compiled.disallow_signed_integer_length_1bit = self.disallow_signed_integer_length_1bit;
        compiled.max_occurs_bounds = self.max_occurs_bounds;
        compiled.unqualified_path_step_policy = self.unqualified_path_step_policy;
        compiled.max_hex_binary_length_in_bytes = self.max_hex_binary_length_in_bytes;
        compiled.max_skip_length_in_bytes = self.max_skip_length_in_bytes;
        compiled.validate_query_style_paths()?;
        Ok(compiled)
    }
}
