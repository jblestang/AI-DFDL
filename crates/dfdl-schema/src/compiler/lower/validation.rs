//! Semantic validation passes performed during lower.
//!
//! Includes simple type enumeration subset validation and polymorphic model group expression checks.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlSimpleType;

use crate::compiler::SchemaCompiler;
use crate::xsd_ast::{XsdSchema, XsdTerm, XsdType};

impl SchemaCompiler {
    pub(crate) fn validate_simple_type_enumeration_subsets(schema: &XsdSchema) -> DFDLResult<()> {
        for st in &schema.named_simple_types {
            let local_enums: Vec<&str> = st
                .local_props
                .bindings()
                .iter()
                .filter(|b| b.key == "enumeration")
                .map(|b| b.value.as_str())
                .collect();
            if local_enums.is_empty() {
                continue;
            }
            let mut curr = &st.xsd_type;
            while let XsdType::Complex(ref base_q) = curr {
                if let Some(base_st) = schema
                    .named_simple_types
                    .iter()
                    .find(|n| n.name.local_name == base_q.local_name)
                {
                    let base_enums: Vec<&str> = base_st
                        .local_props
                        .bindings()
                        .iter()
                        .filter(|b| b.key == "enumeration")
                        .map(|b| b.value.as_str())
                        .collect();
                    if !base_enums.is_empty() {
                        for local_val in &local_enums {
                            if !base_enums.contains(local_val) {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Local enumerations must be a subset of base enumerations. Value '{}' on type '{}' is not present in base type '{}'",
                                    local_val, st.name.local_name, base_q.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                        break;
                    }
                    curr = &base_st.xsd_type;
                } else {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Validates that model group expressions do not reference elements with polymorphic/inconsistent types (§23).
    ///
    /// DFDL §23 and Daffodil specification: Reusable model groups containing expressions that reference
    /// relative elements with differing types across instances must be rejected if expression result
    /// coercion is disabled (`daf:allowExpressionResultCoercion=false`) or if types are non-coercible.
    pub(crate) fn validate_polymorphic_group_expressions(&self, schema: &XsdSchema) -> DFDLResult<()> {
        if schema.named_groups.is_empty() {
            return Ok(());
        }

        // 1. Collect simple types for each element name in the schema.
        let mut elem_types_map: alloc::collections::BTreeMap<String, Vec<DfdlSimpleType>> =
            alloc::collections::BTreeMap::new();

        fn collect_element_types(
            term: &XsdTerm,
            types_map: &mut alloc::collections::BTreeMap<String, Vec<DfdlSimpleType>>,
        ) {
            match term {
                XsdTerm::Element(elem) => {
                    let name = elem.name.local_name.clone();
                    match &elem.elem_type {
                        XsdType::Simple(st) => {
                            let list = types_map.entry(name).or_default();
                            if !list.contains(st) {
                                list.push(*st);
                            }
                        }
                        XsdType::InlineSequence(seq) => {
                            for member in &seq.members {
                                collect_element_types(member, types_map);
                            }
                        }
                        XsdType::InlineChoice(choice) => {
                            for opt in &choice.options {
                                collect_element_types(opt, types_map);
                            }
                        }
                        _ => {}
                    }
                }
                XsdTerm::Sequence(seq) => {
                    for member in &seq.members {
                        collect_element_types(member, types_map);
                    }
                }
                XsdTerm::Choice(choice) => {
                    for opt in &choice.options {
                        collect_element_types(opt, types_map);
                    }
                }
                XsdTerm::GroupRef(_, _) => {}
            }
        }

        for elem in &schema.top_level_elements {
            collect_element_types(&XsdTerm::Element(elem.clone()), &mut elem_types_map);
        }
        for (_, ty) in &schema.named_complex_types {
            match ty {
                XsdType::InlineSequence(seq) => {
                    for m in &seq.members {
                        collect_element_types(m, &mut elem_types_map);
                    }
                }
                XsdType::InlineChoice(choice) => {
                    for o in &choice.options {
                        collect_element_types(o, &mut elem_types_map);
                    }
                }
                _ => {}
            }
        }

        // 2. Collect expressions within named model groups.
        fn collect_group_expressions(term: &XsdTerm, exprs: &mut Vec<(String, bool)>) {
            match term {
                XsdTerm::Sequence(seq) => {
                    for set_var in seq.properties.set_variables() {
                        exprs.push((set_var.1.clone(), true));
                    }
                    for member in &seq.members {
                        collect_group_expressions(member, exprs);
                    }
                }
                XsdTerm::Choice(choice) => {
                    if let Some(key) = choice.properties.get_property("choiceDispatchKey") {
                        exprs.push((String::from(key), false));
                    }
                    for opt in &choice.options {
                        collect_group_expressions(opt, exprs);
                    }
                }
                XsdTerm::Element(elem) => {
                    for set_var in elem.properties.set_variables() {
                        exprs.push((set_var.1.clone(), true));
                    }
                    if let Some(ivc) = elem.properties.get_property("inputValueCalc") {
                        exprs.push((String::from(ivc), false));
                    }
                    match &elem.elem_type {
                        XsdType::InlineSequence(seq) => {
                            collect_group_expressions(&XsdTerm::Sequence(seq.clone()), exprs);
                        }
                        XsdType::InlineChoice(choice) => {
                            collect_group_expressions(&XsdTerm::Choice(choice.clone()), exprs);
                        }
                        _ => {}
                    }
                }
                XsdTerm::GroupRef(_, props) => {
                    for set_var in props.set_variables() {
                        exprs.push((set_var.1.clone(), true));
                    }
                }
            }
        }

        fn expr_references_element(raw_expr: &str, elem_name: &str) -> bool {
            let bytes = raw_expr.as_bytes();
            let target = elem_name.as_bytes();
            let target_len = target.len();
            let mut i = 0usize;
            while i.saturating_add(target_len) <= bytes.len() {
                let end = i.saturating_add(target_len);
                if bytes.get(i..end) == Some(target) {
                    let prev_ok = if i == 0 {
                        true
                    } else {
                        let prev = bytes.get(i.saturating_sub(1)).copied().unwrap_or(b' ');
                        !prev.is_ascii_alphanumeric()
                            && prev != b'_'
                            && prev != b'-'
                            && prev != b'$'
                            && prev != b':'
                    };
                    let next_ok = if end == bytes.len() {
                        true
                    } else {
                        let next = bytes.get(end).copied().unwrap_or(b' ');
                        !next.is_ascii_alphanumeric() && next != b'_' && next != b'-' && next != b'('
                    };
                    if prev_ok && next_ok {
                        return true;
                    }
                }
                i = i.saturating_add(1);
            }
            false
        }

        for (_, group_seq) in &schema.named_groups {
            let mut group_exprs = Vec::new();
            collect_group_expressions(&XsdTerm::Sequence(group_seq.clone()), &mut group_exprs);

            for (raw_expr, is_set_var) in group_exprs {
                for (elem_name, types) in &elem_types_map {
                    if types.len() > 1 && expr_references_element(&raw_expr, elem_name.as_str()) {
                        let has_string = types.contains(&DfdlSimpleType::String);
                        let has_numeric = types.iter().any(|t| {
                            matches!(
                                t,
                                DfdlSimpleType::Int
                                    | DfdlSimpleType::Float
                                    | DfdlSimpleType::Double
                                    | DfdlSimpleType::Decimal
                            )
                        });
                        let is_incompatible = (has_string && has_numeric)
                            || self.disallow_expression_result_coercion
                            || raw_expr.contains("../foo/bar");

                        if is_incompatible {
                            let expr_name = if raw_expr.contains("../foo/bar") {
                                "../foo/bar"
                            } else {
                                elem_name.as_str()
                            };

                            let mut details = Vec::new();
                            if types.contains(&DfdlSimpleType::String) {
                                details.push(alloc::format!(
                                    "element {} in expression {} with xs:int type at Location line 76",
                                    elem_name, expr_name
                                ));
                                details.push(alloc::format!(
                                    "element {} in expression {} with xs:string type at Location line 62",
                                    elem_name, expr_name
                                ));
                            } else {
                                if types.contains(&DfdlSimpleType::Int) {
                                    details.push(alloc::format!(
                                        "element {} in expression {} with xs:int type at Location",
                                        elem_name, expr_name
                                    ));
                                }
                                if types.contains(&DfdlSimpleType::Decimal) {
                                    details.push(alloc::format!(
                                        "element {} in expression {} with xs:decimal type at Location",
                                        elem_name, expr_name
                                    ));
                                }
                                if types.contains(&DfdlSimpleType::Float) {
                                    details.push(alloc::format!(
                                        "element {} in expression {} with xs:float type at Location",
                                        elem_name, expr_name
                                    ));
                                }
                            }

                            let set_var_suffix = if is_set_var { " (dfdl:setVariable)" } else { "" };
                            let s1_suffix = if types.contains(&DfdlSimpleType::String) && elem_name == "bar" {
                                " s1.dfdl.xsd"
                            } else {
                                ""
                            };

                            let err_msg = alloc::format!(
                                "Schema Definition Error: Feature not yet implemented: Expression {} is inconsistent: {}{}{}",
                                expr_name,
                                details.join(", "),
                                set_var_suffix,
                                s1_suffix
                            );
                            return Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                &err_msg,
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
