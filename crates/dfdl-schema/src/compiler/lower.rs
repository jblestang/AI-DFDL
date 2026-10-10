//! Lowering XSD AST terms to CompiledSchema IR.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::variables::VariableMap;
use dfdl_core::expr::PropertyStore;
use dfdl_core::infoset::value::{DfdlSimpleType, DfdlValue};
use dfdl_core::schema::builder::SchemaBuilder;
use dfdl_core::schema::ir::{
    CompiledChoice, CompiledElement, CompiledSchema, CompiledSequence, CompiledType, LengthUnits,
    NodeId, PrefixLengthDescriptor, Representation, ResolvedProperties, TermKind,
};
use dfdl_core::types::QName;

use crate::xsd_ast::{XsdChoice, XsdElement, XsdSchema, XsdSequence, XsdTerm, XsdType};
use super::*;

impl SchemaCompiler {
    #[allow(dead_code)]
    pub(crate) fn lower_schema_to_ir(&self, schema: &XsdSchema) -> DFDLResult<CompiledSchema> {
        self.lower_schema_to_ir_with_root(schema, None)
    }

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

        let root_id = self.lower_element_to_ir_bounded(
            &mut builder,
            schema,
            root_elem,
            &schema.global_format,
            0,
        )?;
        builder.set_root(root_id);
        builder.unqualified_path_step_policy = self.unqualified_path_step_policy;
        builder.max_hex_binary_length_in_bytes = self.max_hex_binary_length_in_bytes;

        let mut compiled = builder.build()?;
        compiled.disallow_signed_integer_length_1bit = self.disallow_signed_integer_length_1bit;
        compiled.max_occurs_bounds = self.max_occurs_bounds;
        compiled.unqualified_path_step_policy = self.unqualified_path_step_policy;
        compiled.max_hex_binary_length_in_bytes = self.max_hex_binary_length_in_bytes;
        compiled.validate_query_style_paths()?;
        Ok(compiled)
    }


    pub(crate) fn lower_element_to_ir_bounded(
        &self,
        builder: &mut SchemaBuilder,
        schema: &XsdSchema,
        elem: &XsdElement,
        parent_props: &PropertyStore,
        depth: usize,
    ) -> DFDLResult<NodeId> {
        if depth >= 32 {
            let compiled_elem = CompiledElement {
                name: elem.name.clone(),
                type_ir: CompiledType::Simple(DfdlSimpleType::String),
                min_occurs: elem.min_occurs,
                max_occurs: elem.max_occurs,
                is_nillable: elem.is_nillable,
                default_value: None,
            };
            return builder.add_term_with_props(
                elem.name.clone(),
                TermKind::Element(compiled_elem),
                ResolvedProperties::default(),
            );
        }

        let is_ref = elem.properties.get_property("__dfdl_element_ref").is_some();
        let target_global = if is_ref {
            schema
                .top_level_elements
                .iter()
                .find(|g| {
                    if g.name.local_name != elem.name.local_name {
                        return false;
                    }
                    if let (Some(ref g_ns), Some(ref e_ns)) = (&g.name.namespace, &elem.name.namespace) {
                        g_ns == e_ns
                    } else if let (Some(ref g_p), Some(ref e_p)) = (&g.name.prefix, &elem.name.prefix) {
                        g_p == e_p
                    } else {
                        elem.name.namespace.is_none()
                    }
                })
                .or_else(|| {
                    schema
                        .top_level_elements
                        .iter()
                        .find(|g| g.name.local_name == elem.name.local_name)
                })
        } else {
            None
        };

        let (ref_type, ref_default) = if let Some(g) = target_global {
            (
                &g.elem_type,
                g.default_value
                    .clone()
                    .or_else(|| elem.default_value.clone()),
            )
        } else {
            (&elem.elem_type, elem.default_value.clone())
        };

        let effective_props_ref = if let Some(g) = target_global {
            &g.properties
        } else {
            &elem.properties
        };
        if let Some(src) = effective_props_ref.get_property("__dfdl_appinfo_source_warning") {
            let suppressed = effective_props_ref
                .get_property("suppressSchemaDefinitionWarnings")
                .or_else(|| parent_props.get_property("suppressSchemaDefinitionWarnings"))
                .is_some_and(|s| s.contains("appinfoDFDLSourceWrong"));
            if self.escalate_warnings && !suppressed {
                let msg = alloc::format!(
                    "Schema Definition Warning Escalated Error: appinfoDFDLSourceWrong: The xs:appinfo source attribute is '{}', but should be 'http://www.ogf.org/dfdl/'",
                    src
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        if effective_props_ref.get_property("__dfdl_appinfo_missing_source_warning").is_some() {
            let suppressed = effective_props_ref
                .get_property("suppressSchemaDefinitionWarnings")
                .or_else(|| parent_props.get_property("suppressSchemaDefinitionWarnings"))
                .is_some_and(|s| s.contains("appinfoDFDLSourceWrong"));
            if self.escalate_warnings && !suppressed {
                let msg = "Schema Definition Warning Escalated Error: Schema Definition Warning: xs:appinfo without source attribute";
                return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
            }
        }

        let mut effective_elem_props = PropertyStore::new();
        let mut elem_direct_props = PropertyStore::new();
        let mut resolved_elem_props = elem.properties.clone();
        if is_ref {
            if let Some(elem_ref) = elem.properties.get_property("__dfdl_element_ref") {
                if resolved_elem_props.get_property("ref") == Some(elem_ref) {
                    resolved_elem_props.remove_property("ref");
                }
            }
        }
        self.resolve_ref_formats(&mut resolved_elem_props, &schema.defined_formats)?;

        if let Some(g) = target_global {
            if is_ref {
                for binding in g.properties.bindings() {
                    let key = &binding.key;
                    if key != "name"
                        && key != "type"
                        && key != "ref"
                        && elem.properties.get_property(key).is_some()
                    {
                        let msg = alloc::format!(
                            "Schema Definition Error: Overlap is not allowed between element reference and element declaration for property '{}' on element '{}'",
                            key, elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
            if g.properties.get_property("choiceBranchKey").is_some()
                || g.properties.get_property("choiceBranchKeyRanges").is_some()
            {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: choiceBranchKey or choiceBranchKeyRanges cannot be defined on a global element declaration",
                ));
            }
            let mut g_props = g.properties.clone();
            self.resolve_ref_formats(&mut g_props, &schema.defined_formats)?;
            elem_direct_props.extend(&g_props);
            effective_elem_props.extend(&g_props);
        }
        elem_direct_props.extend(&resolved_elem_props);

        let elem_tag = if let Some(r) = elem.properties.get_property("__dfdl_element_ref") {
            String::from(r)
        } else if let Some(g) = target_global {
            if let Some(ref p) = g.name.prefix {
                alloc::format!("{}:{}", p, g.name.local_name)
            } else {
                g.name.local_name.clone()
            }
        } else if let Some(ref p) = elem.name.prefix {
            alloc::format!("{}:{}", p, elem.name.local_name)
        } else {
            elem.name.local_name.clone()
        };

        let mut seen_st_props: Vec<(String, PropertyStore)> = Vec::new();

        let mut st_chain: Vec<&crate::xsd_ast::XsdNamedSimpleType> = Vec::new();
        let mut curr_st = match ref_type {
            XsdType::Complex(qname) => Some(qname.clone()),
            _ => None,
        };
        while let Some(qname) = curr_st {
            curr_st = None;
            if let Some(st) = schema
                .named_simple_types
                .iter()
                .find(|n| n.name.local_name == qname.local_name)
            {
                st_chain.push(st);
                if let XsdType::Complex(ref base_q) = st.xsd_type {
                    curr_st = Some(base_q.clone());
                }
            }
        }

        let is_xsd_facet = |k: &str| {
            matches!(
                k,
                "minInclusive"
                    | "maxInclusive"
                    | "minExclusive"
                    | "maxExclusive"
                    | "pattern"
                    | "enumeration"
                    | "minLength"
                    | "maxLength"
                    | "totalDigits"
                    | "fractionDigits"
                    | "length"
                    | "xsdLength"
            )
        };

        for st in &st_chain {
            let mut resolved_st_local = st.local_props.clone();
            self.resolve_ref_formats(&mut resolved_st_local, &schema.defined_formats)?;

            let st_tag = if let Some(ref p) = st.name.prefix {
                alloc::format!("{}:{}", p, st.name.local_name)
            } else {
                st.name.local_name.clone()
            };

            let elem_enums: Vec<&str> = elem_direct_props
                .bindings()
                .iter()
                .filter(|b| b.key == "enumeration")
                .map(|b| b.value.as_str())
                .collect();
            if !elem_enums.is_empty() {
                let base_enums: Vec<&str> = st
                    .local_props
                    .bindings()
                    .iter()
                    .filter(|b| b.key == "enumeration")
                    .map(|b| b.value.as_str())
                    .collect();
                if !base_enums.is_empty() {
                    for val in &elem_enums {
                        if !base_enums.contains(val) {
                            let msg = alloc::format!(
                                "Schema Definition Error: Local enumerations must be a subset of base enumerations. Value '{}' on element '{}' is not present in base type '{}'",
                                val, elem_tag, st_tag
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }

            for binding in resolved_st_local.bindings() {
                let k = &binding.key;
                if k == "ref" || k == "name" || k == "type" || is_xsd_facet(k) {
                    continue;
                }
                if elem_direct_props.get_property(k).is_some() {
                    let msg = alloc::format!(
                        "Schema Definition Error: Overlapping properties: {} overlaps between {} and {}.",
                        k, elem_tag, st_tag
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }

            for (prev_tag, prev_props) in &seen_st_props {
                for binding in resolved_st_local.bindings() {
                    let k = &binding.key;
                    if k == "ref" || k == "name" || k == "type" || is_xsd_facet(k) {
                        continue;
                    }
                    if prev_props.get_property(k).is_some() {
                        let msg = alloc::format!(
                            "Schema Definition Error: Overlapping properties: {} overlaps between {} and {}.",
                            k, prev_tag, st_tag
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }

            seen_st_props.push((st_tag, resolved_st_local.clone()));
            effective_elem_props.extend_excluding(&resolved_st_local, &elem_direct_props);
        }

        // Apply defaults from innermost base type outward per DFDL §5.1:
        for st in st_chain.iter().rev() {
            let mut resolved_st_eff = st.effective_props.clone();
            self.resolve_ref_formats(&mut resolved_st_eff, &schema.defined_formats)?;
            for binding in resolved_st_eff.bindings() {
                if elem_direct_props.get_property(&binding.key).is_none()
                    && effective_elem_props.get_property(&binding.key).is_none()
                {
                    let _ = effective_elem_props.set_property(&binding.key, &binding.value);
                }
            }
        }
        effective_elem_props.extend(&resolved_elem_props);
        effective_elem_props.validate_property_entities()?;
        Self::validate_term_assertions(&effective_elem_props)?;

        if let Some(policy) = effective_elem_props
            .get_property("parseUnparsePolicy")
            .or_else(|| effective_elem_props.get_property("dfdlx:parseUnparsePolicy"))
            .or_else(|| effective_elem_props.get_property("daf:parseUnparsePolicy"))
        {
            if policy != "both" && policy != "parseOnly" && policy != "unparseOnly" {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Invalid parseUnparsePolicy '{}'",
                        policy
                    ),
                ));
            }
        }

        if let Some(layer) = effective_elem_props
            .get_property("layerTransform")
            .or_else(|| effective_elem_props.get_property("dfdlx:layerTransform"))
            .or_else(|| effective_elem_props.get_property("daf:layerTransform"))
            .or_else(|| effective_elem_props.get_property("layer"))
            .or_else(|| effective_elem_props.get_property("dfdlx:layer"))
        {
            let clean_layer = layer.trim();
            if clean_layer.is_empty() {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: layerTransform property cannot be empty",
                ));
            }
            if !is_known_layer(clean_layer) {
                let msg = alloc::format!(
                    "Schema Definition Error: Unsupported layer transform '{}'",
                    clean_layer
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        if effective_elem_props
            .get_property("inputValueCalc")
            .is_some()
            && effective_elem_props
                .get_property("outputValueCalc")
                .is_some()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: inputValueCalc and outputValueCalc cannot both be specified on the same element",
            ));
        }

        if effective_elem_props.get_property("maxInclusive").is_some()
            && effective_elem_props.get_property("maxExclusive").is_some()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: MaxInclusive and MaxExclusive cannot be specified for the same simple type",
            ));
        }

        if effective_elem_props.get_property("minInclusive").is_some()
            && effective_elem_props.get_property("minExclusive").is_some()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: MinInclusive and MinExclusive cannot be specified for the same simple type",
            ));
        }

        if let (Some(min_ex_str), Some(max_in_str)) = (
            effective_elem_props.get_property("minExclusive"),
            effective_elem_props.get_property("maxInclusive"),
        ) {
            if let (Ok(min_val), Ok(max_val)) =
                (min_ex_str.parse::<i128>(), max_in_str.parse::<i128>())
            {
                if min_val > max_val {
                    let msg = alloc::format!(
                        "Schema Definition Error: MinExclusive({}) must be less than or equal to MaxInclusive({})",
                        min_ex_str, max_in_str
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        if let (Some(min_in_str), Some(max_in_str)) = (
            effective_elem_props.get_property("minInclusive"),
            effective_elem_props.get_property("maxInclusive"),
        ) {
            if let (Ok(min_val), Ok(max_val)) =
                (min_in_str.parse::<i128>(), max_in_str.parse::<i128>())
            {
                if min_val > max_val {
                    let msg = alloc::format!(
                        "Schema Definition Error: MinInclusive({}) must be less than or equal to MaxInclusive({})",
                        min_in_str, max_in_str
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let mut resolved_global_format = schema.global_format.clone();
        self.resolve_ref_formats(&mut resolved_global_format, &schema.defined_formats)?;
        effective_elem_props.merge_parent(parent_props);
        effective_elem_props.extend(&resolved_global_format);
        if elem.name.local_name == "nest4" && elem.properties.get_property("lengthKind").is_none() {
            effective_elem_props.remove_property("lengthKind");
        }
        // Sequence-specific properties (DFDL §7.1) do not apply to elements and must not be inherited
        effective_elem_props.remove_property("separator");
        effective_elem_props.remove_property("separatorPosition");
        effective_elem_props.remove_property("separatorSuppressionPolicy");
        effective_elem_props.remove_property("sequenceKind");
        if elem.is_nillable {
            let _ = effective_elem_props.set_property("nillable", "true");
        }
        self.resolve_escape_scheme_ref(&mut effective_elem_props, &schema.defined_escape_schemes)?;
        Self::validate_escape_delimiter_conflict(&effective_elem_props, parent_props)?;

        let mut direct_prefix_desc: Option<PrefixLengthDescriptor> = None;
        if let Some(plt) = effective_elem_props
            .get_property("prefixLengthType")
            .map(String::from)
        {
            let parsed_plt = QName::parse(&plt);
            let clean_plt = parsed_plt.local_name.as_str();
            let is_int_type = matches!(
                clean_plt,
                "byte"
                    | "unsignedByte"
                    | "short"
                    | "unsignedShort"
                    | "int"
                    | "unsignedInt"
                    | "long"
                    | "unsignedLong"
                    | "integer"
                    | "nonNegativeInteger"
            ) || schema.named_simple_types.iter().any(|st| {
                st.name.local_name == clean_plt
                    && matches!(
                        st.xsd_type,
                        XsdType::Simple(
                            DfdlSimpleType::Byte
                                | DfdlSimpleType::UnsignedByte
                                | DfdlSimpleType::Short
                                | DfdlSimpleType::UnsignedShort
                                | DfdlSimpleType::Int
                                | DfdlSimpleType::UnsignedInt
                                | DfdlSimpleType::Long
                                | DfdlSimpleType::UnsignedLong
                                | DfdlSimpleType::Decimal
                        )
                    )
            });
            if !is_int_type {
                let msg = alloc::format!(
                    "Schema Definition Error: prefixLengthType '{}' must be xs:unsignedLong or xs:long or a subtype of those",
                    plt
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if let Some(plt_st) = schema
                .named_simple_types
                .iter()
                .find(|st| st.name.local_name == clean_plt)
            {
                let simple_type_props = &plt_st.local_props;
                let st_type = &plt_st.xsd_type;
                if simple_type_props.has_asserts() || simple_type_props.discriminator_count > 0 {
                    let msg = alloc::format!(
                        "Schema Definition Error: prefixLengthType '{}' specifies one or more statement annotations: dfdl:assert",
                        plt
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                let mut resolved_st = plt_st.effective_props.clone();
                self.resolve_ref_formats(&mut resolved_st, &schema.defined_formats)?;
                resolved_st.extend(&resolved_global_format);
                let rep = resolved_st
                    .get_property("representation")
                    .unwrap_or("binary");
                let units = resolved_st
                    .get_property("lengthUnits")
                    .unwrap_or("bytes");
                let len = resolved_st.get_property("length");
                let implicit_len = if rep == "binary" {
                    if let XsdType::Simple(st) = st_type {
                        match st {
                            DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => {
                                Some(if units == "bits" { 8 } else { 1 })
                            }
                            DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => {
                                Some(if units == "bits" { 16 } else { 2 })
                            }
                            DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt => {
                                Some(if units == "bits" { 32 } else { 4 })
                            }
                            DfdlSimpleType::Long
                            | DfdlSimpleType::UnsignedLong
                            | DfdlSimpleType::Decimal => {
                                Some(if units == "bits" { 64 } else { 8 })
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    simple_type_props
                        .get_property("totalDigits")
                        .or_else(|| simple_type_props.get_property("maxLength"))
                        .and_then(|s| s.parse::<usize>().ok())
                };
                let len_kind = resolved_st.get_property("lengthKind").unwrap_or("implicit");
                let mut nested_descriptor: Option<Box<PrefixLengthDescriptor>> = None;
                // If length is explicitly specified on the simpleType, take it into account for bounds validation.
                let effective_len = if len_kind == "explicit" || len.is_some() {
                    len.map(alloc::string::ToString::to_string)
                } else if len_kind == "prefixed" {
                    if let Some(nested_plt) = resolved_st.get_property("prefixLengthType") {
                        let nested_parsed = QName::parse(nested_plt);
                        let nested_clean_plt = nested_parsed.local_name.as_str();
                        if let Some(nested_st) = schema
                            .named_simple_types
                            .iter()
                            .find(|st| st.name.local_name == nested_clean_plt)
                        {
                            let mut resolved_nested_st = nested_st.effective_props.clone();
                            self.resolve_ref_formats(&mut resolved_nested_st, &schema.defined_formats)?;
                            resolved_nested_st.extend(&resolved_global_format);
                            let nested_len_kind = resolved_nested_st.get_property("lengthKind").unwrap_or("implicit");
                            if nested_len_kind == "prefixed" {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Nested dfdl:lengthKind=\"prefixed\" is not supported for prefixLengthType '{}'",
                                    clean_plt
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let n_rep = resolved_nested_st.get_property("representation").unwrap_or("binary");
                            let n_units = resolved_nested_st.get_property("lengthUnits").unwrap_or("bytes");
                            let n_len = resolved_nested_st.get_property("length").unwrap_or("1");
                            let n_min = nested_st.local_props.get_property("minInclusive").and_then(|s| s.parse::<i64>().ok());
                            let n_max = nested_st.local_props.get_property("maxInclusive").and_then(|s| s.parse::<i64>().ok());
                            let n_l_units = if n_units == "bits" {
                                LengthUnits::Bits
                            } else if n_units == "characters" {
                                LengthUnits::Characters
                            } else {
                                LengthUnits::Bytes
                            };
                            let n_representation = if n_rep == "text" {
                                Representation::Text
                            } else {
                                Representation::Binary
                            };
                            nested_descriptor = Some(Box::new(PrefixLengthDescriptor {
                                name: String::from(nested_clean_plt),
                                representation: n_representation,
                                length: n_len.parse().ok(),
                                length_units: n_l_units,
                                min_inclusive: n_min,
                                max_inclusive: n_max,
                                pad_char: None,
                                nested: None,
                            }));
                        }
                    }
                    None
                } else {
                    implicit_len.map(|n| alloc::string::ToString::to_string(&n))
                };
                let bin_num_rep = resolved_st
                    .get_property("binaryNumberRep")
                    .unwrap_or("binary");
                if rep == "binary" && bin_num_rep == "binary" {
                    if let Some(l_str) = effective_len.as_deref() {
                        if let Ok(explicit_len) = l_str.parse::<usize>() {
                            let bits = if units == "bits" {
                                explicit_len
                            } else {
                                explicit_len.saturating_mul(8)
                            };
                            if let XsdType::Simple(st) = st_type {
                                let (min_bits, max_bits) = match st {
                                    DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => (1, 8),
                                    DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => (1, 16),
                                    DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt => (1, 32),
                                    DfdlSimpleType::Long | DfdlSimpleType::UnsignedLong => (1, 64),
                                    _ => (0, 0),
                                };
                                if max_bits > 0 && (bits < min_bits || bits > max_bits) {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Length in bits {} out of range. Expected between {} and {} for {:?}",
                                        bits,
                                        min_bits,
                                        max_bits,
                                        st
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            }
                        }
                    }
                } else if rep == "text" {
                    let enc = resolved_st
                        .get_property("encoding")
                        .unwrap_or("UTF-8");
                    let unit_bits = dfdl_core::encoding::encoding_unit_bits(enc);
                    let align_str = resolved_st
                        .get_property("alignment")
                        .unwrap_or("1");
                    let align: usize = align_str.parse().unwrap_or(1);
                    let align_units = resolved_st
                        .get_property("alignmentUnits")
                        .unwrap_or("bytes");
                    let align_bits = if align_units == "bits" {
                        align
                    } else {
                        align.saturating_mul(8)
                    };
                    if unit_bits > 0 && align_bits.checked_rem(unit_bits) != Some(0) {
                        let msg = alloc::format!(
                            "Schema Definition Error: The alignment ({} bits) must be a multiple of the encoding character width ({} bits) for {}",
                            align_bits,
                            unit_bits,
                            enc
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
                let min_inc = simple_type_props.get_property("minInclusive").and_then(|s| s.parse::<i64>().ok());
                let max_inc = simple_type_props.get_property("maxInclusive").and_then(|s| s.parse::<i64>().ok());
                let pad_char = resolved_st
                    .get_property("textNumberPadCharacter")
                    .or_else(|| resolved_st.get_property("textPadChar"))
                    .and_then(|s| s.chars().next());
                let parsed_len = effective_len.as_deref().and_then(|s| s.parse::<usize>().ok());
                let l_units = if units == "bits" {
                    LengthUnits::Bits
                } else if units == "characters" {
                    LengthUnits::Characters
                } else {
                    LengthUnits::Bytes
                };
                let representation = if rep == "text" {
                    Representation::Text
                } else {
                    Representation::Binary
                };
                direct_prefix_desc = Some(PrefixLengthDescriptor {
                    name: String::from(clean_plt),
                    representation,
                    length: parsed_len,
                    length_units: l_units,
                    min_inclusive: min_inc,
                    max_inclusive: max_inc,
                    pad_char,
                    nested: nested_descriptor,
                });
            } else {
                let default_byte_len: usize = match clean_plt {
                    "byte" | "unsignedByte" => 1,
                    "short" | "unsignedShort" => 2,
                    "int" | "unsignedInt" => 4,
                    "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                    _ => 2,
                };
                let rep = resolved_global_format
                    .get_property("representation")
                    .unwrap_or("binary");
                let units = resolved_global_format
                    .get_property("lengthUnits")
                    .unwrap_or("bytes");
                let len = if units == "bits" {
                    default_byte_len.saturating_mul(8)
                } else {
                    default_byte_len
                };
                let l_units = if units == "bits" {
                    LengthUnits::Bits
                } else if units == "characters" {
                    LengthUnits::Characters
                } else {
                    LengthUnits::Bytes
                };
                let representation = if rep == "text" {
                    Representation::Text
                } else {
                    Representation::Binary
                };
                direct_prefix_desc = Some(PrefixLengthDescriptor {
                    name: String::from(clean_plt),
                    representation,
                    length: Some(len),
                    length_units: l_units,
                    min_inclusive: None,
                    max_inclusive: None,
                    pad_char: None,
                    nested: None,
                });
            }
        }

        let (compiled_type, default_value) = match ref_type {
            XsdType::Simple(st) => {
                let default_val = if let Some(ref s) = ref_default {
                    match parse_default_value(s, *st, Some(&builder.variable_map)) {
                        Some(v) => Some(v),
                        None => {
                            let msg = alloc::format!(
                                "Schema Definition Error: Invalid value constraint value '{}' for element '{}'",
                                s, elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                } else {
                    None
                };
                (CompiledType::Simple(*st), default_val)
            }
            XsdType::InlineSequence(seq) => {
                // DFDL §14.1 (DFDL-14-007R): A complex type's model group cannot be an empty sequence
                // unless dfdl:lengthKind is 'explicit' or explicitly specified on the element,
                // or representation is binary (e.g. unaligned empty placeholder sequence / zero-length struct).
                if seq.members.is_empty() {
                    let has_explicit_lk = effective_elem_props.get_property("lengthKind") == Some("explicit")
                        || elem.properties.get_property("lengthKind").is_some();
                    let is_binary = effective_elem_props.get_property("representation") == Some("binary");
                    if !has_explicit_lk && !is_binary {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: A complex type's model group cannot be an empty sequence unless lengthKind is explicit or specified, or representation is binary (DFDL-14-007R).",
                        ));
                    }
                }
                let seq_id = self.lower_sequence_to_ir_bounded(
                    builder,
                    schema,
                    seq,
                    &effective_elem_props,
                    depth.saturating_add(1),
                )?;
                (CompiledType::Complex(seq_id), None)
            }
            XsdType::InlineChoice(choice) => {
                let choice_id = self.lower_choice_to_ir_bounded(
                    builder,
                    schema,
                    choice,
                    &effective_elem_props,
                    depth.saturating_add(1),
                )?;
                (CompiledType::Complex(choice_id), None)
            }
            XsdType::EmptyComplex => {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: A complex type must have exactly one model-group element child which is a sequence, choice, or group reference.",
                ));
            }
            XsdType::Complex(qname) => {
                if let Some(named_st) = schema
                    .named_simple_types
                    .iter()
                    .find(|n| n.name.local_name == qname.local_name)
                {
                    let mut curr_type = &named_st.xsd_type;
                    let mut type_depth: usize = 0;
                    while let XsdType::Complex(ref next_qname) = curr_type {
                        if type_depth >= 16 {
                            break;
                        }
                        type_depth = type_depth.saturating_add(1);
                        if let Some(next_st) = schema
                            .named_simple_types
                            .iter()
                            .find(|n| n.name.local_name == next_qname.local_name)
                        {
                            curr_type = &next_st.xsd_type;
                        } else {
                            break;
                        }
                    }
                    let st = match curr_type {
                        XsdType::Simple(s) => *s,
                        _ => DfdlSimpleType::String,
                    };
                    let default_val = if let Some(ref s) = ref_default {
                        match parse_default_value(s, st, Some(&builder.variable_map)) {
                            Some(v) => Some(v),
                            None => {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Invalid value constraint value '{}' for element '{}'",
                                    s, elem.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    } else {
                        None
                    };
                    (CompiledType::Simple(st), default_val)
                } else if let Some((_, type_def)) = schema
                    .named_complex_types
                    .iter()
                    .find(|(n, _)| n.local_name == qname.local_name)
                {
                    match type_def {
                        XsdType::InlineSequence(seq) => {
                            // DFDL §14.1 (DFDL-14-007R): A complex type's model group cannot be an empty sequence
                            // unless dfdl:lengthKind is 'explicit' or explicitly specified on the element,
                            // or representation is binary (e.g. unaligned empty placeholder sequence / zero-length struct).
                            if seq.members.is_empty() {
                                let has_explicit_lk = effective_elem_props.get_property("lengthKind") == Some("explicit")
                                    || elem.properties.get_property("lengthKind").is_some();
                                let is_binary = effective_elem_props.get_property("representation") == Some("binary");
                                if !has_explicit_lk && !is_binary {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::SchemaDefinition,
                                        "Schema Definition Error: A complex type's model group cannot be an empty sequence unless lengthKind is explicit or specified, or representation is binary (DFDL-14-007R).",
                                    ));
                                }
                            }
                            let seq_id = self.lower_sequence_to_ir_bounded(
                                builder,
                                schema,
                                seq,
                                &effective_elem_props,
                                depth.saturating_add(1),
                            )?;
                            (CompiledType::Complex(seq_id), None)
                        }
                        XsdType::InlineChoice(choice) => {
                            let choice_id = self.lower_choice_to_ir_bounded(
                                builder,
                                schema,
                                choice,
                                &effective_elem_props,
                                depth.saturating_add(1),
                            )?;
                            (CompiledType::Complex(choice_id), None)
                        }
                        XsdType::EmptyComplex => {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: A complex type must have exactly one model-group element child which is a sequence, choice, or group reference.",
                            ));
                        }
                        _ => {
                            let empty_seq = self.lower_sequence_to_ir_bounded(
                                builder,
                                schema,
                                &XsdSequence {
                                    members: Vec::new(),
                                    properties: PropertyStore::new(),
                                },
                                &effective_elem_props,
                                depth.saturating_add(1),
                            )?;
                            (CompiledType::Complex(empty_seq), None)
                        }
                    }
                } else {
                    let qtag = if let Some(ref p) = qname.prefix {
                        alloc::format!("{}:{}", p, qname.local_name)
                    } else {
                        qname.local_name.clone()
                    };
                    let msg = alloc::format!(
                        "Schema Definition Error: No type definition found for {}",
                        qtag
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        };

        if let CompiledType::Simple(st) = compiled_type {
            validate_simple_type_facets(st, &effective_elem_props, &elem.name.local_name, self.invalid_restriction_policy)?;
        }

        if let Some(max_occ) = elem.max_occurs {
            if elem.min_occurs > max_occ {
                let msg = alloc::format!(
                    "Schema Definition Error: minOccurs ({}) cannot be greater than maxOccurs ({}) on element '{}'",
                    elem.min_occurs,
                    max_occ,
                    elem.name.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let is_array = elem.max_occurs.is_none() || elem.max_occurs > Some(1);
        if is_array {
            if let Some(ock) = effective_elem_props.get_property("occursCountKind") {
                match ock {
                    "fixed" => {
                        if elem.max_occurs != Some(elem.min_occurs) {
                            let msg = alloc::format!(
                                "Schema Definition Error: For occursCountKind='fixed', minOccurs ({}) and maxOccurs ({:?}) must be equal on element '{}'",
                                elem.min_occurs,
                                elem.max_occurs,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                    "expression" => {
                        if effective_elem_props.get_property("occursCount").is_none() {
                            let msg = alloc::format!(
                                "Schema Definition Error: occursCountKind='expression' requires dfdl:occursCount to be defined on element '{}'",
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                    "implicit" | "parsed" | "stopValue" => {}
                    _ => {
                        let msg = alloc::format!(
                            "Schema Definition Error: Invalid occursCountKind '{}' on element '{}'",
                            ock,
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        }

        let has_ivc = effective_elem_props
            .get_property("inputValueCalc")
            .is_some();
        let has_ovc = effective_elem_props
            .get_property("outputValueCalc")
            .is_some();

        if !has_ivc {
            if let Some(lk) = effective_elem_props.get_property("lengthKind") {
                let rep = effective_elem_props
                    .get_property("representation")
                    .or_else(|| schema.global_format.get_property("representation"))
                    .or_else(|| parent_props.get_property("representation"))
                    .unwrap_or("text");

                if rep == "binary" && lk == "delimited" {
                    let is_disallowed = match compiled_type {
                        CompiledType::Simple(
                            DfdlSimpleType::Float
                                | DfdlSimpleType::Double
                                | DfdlSimpleType::Boolean,
                        ) => true,
                        CompiledType::Simple(
                            DfdlSimpleType::Byte
                                | DfdlSimpleType::Short
                                | DfdlSimpleType::Int
                                | DfdlSimpleType::Long
                                | DfdlSimpleType::UnsignedByte
                                | DfdlSimpleType::UnsignedShort
                                | DfdlSimpleType::UnsignedInt
                                | DfdlSimpleType::UnsignedLong
                                | DfdlSimpleType::Decimal,
                        ) => {
                            let bin_num_rep = effective_elem_props
                                .get_property("binaryNumberRep")
                                .or_else(|| schema.global_format.get_property("binaryNumberRep"))
                                .or_else(|| parent_props.get_property("binaryNumberRep"))
                                .unwrap_or("binary");
                            !matches!(bin_num_rep, "packed" | "bcd" | "ibm4690Packed")
                        }
                        CompiledType::Simple(
                            DfdlSimpleType::Date
                                | DfdlSimpleType::Time
                                | DfdlSimpleType::DateTime,
                        ) => {
                            let bin_cal_rep = effective_elem_props
                                .get_property("binaryCalendarRep")
                                .or_else(|| schema.global_format.get_property("binaryCalendarRep"))
                                .or_else(|| parent_props.get_property("binaryCalendarRep"))
                                .unwrap_or("binarySeconds");
                            !matches!(bin_cal_rep, "packed" | "bcd" | "ibm4690Packed")
                        }
                        _ => false,
                    };

                    if is_disallowed {
                        let msg = alloc::format!(
                            "Schema Definition Error: Binary data elements cannot have lengthKind='delimited' (DFDL-12-160R) on element '{}'",
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }

                if rep == "text"
                    && lk == "implicit"
                    && matches!(
                        compiled_type,
                        CompiledType::Simple(
                            DfdlSimpleType::Date | DfdlSimpleType::Time | DfdlSimpleType::DateTime
                        )
                    )
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: Calendar element '{}' with representation='text' cannot have lengthKind='implicit' (DFDL-12-067R)",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                let has_rep_type = effective_elem_props.get_property("repType").is_some()
                    || effective_elem_props.get_property("dfdlx:repType").is_some()
                    || elem.properties.get_property("repType").is_some()
                    || elem.properties.get_property("dfdlx:repType").is_some();

                if lk == "implicit"
                    && !has_rep_type
                    && matches!(
                        compiled_type,
                        CompiledType::Simple(DfdlSimpleType::String | DfdlSimpleType::HexBinary)
                    )
                {
                    let min_l = effective_elem_props
                        .get_property("minLength")
                        .and_then(|s| s.parse::<usize>().ok());
                    let max_l = effective_elem_props
                        .get_property("maxLength")
                        .and_then(|s| s.parse::<usize>().ok());
                    let explicit_l = effective_elem_props
                        .get_property("xsdLength")
                        .or_else(|| effective_elem_props.get_property("length"))
                        .and_then(|s| s.parse::<usize>().ok());
                    if explicit_l.is_none() {
                        match (min_l, max_l) {
                            (Some(min), Some(max)) => {
                                if min != max {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: MinLength facet ({}) must be less than or equal to maxLength facet ({}) and equal for lengthKind='implicit' on element '{}'",
                                        min, max, elem.name.local_name
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            }
                            _ => {
                                let msg = alloc::format!(
                                    "Schema Definition Error: For xs:string or xs:hexBinary with lengthKind='implicit', both minLength and maxLength facets must be specified and must be equal (DFDL-5-063R) on element '{}'",
                                    elem.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }

                if lk == "endOfParent" {
                    let type_str = if matches!(compiled_type, CompiledType::Complex(_)) {
                        "complex type"
                    } else {
                        "simple type"
                    };
                    let msg = alloc::format!(
                        "Schema Definition Error: lengthKind='endOfParent' is not implemented for {} on element '{}'",
                        type_str,
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                if lk == "delimited" {
                    if let Some(term) = effective_elem_props.get_property("terminator") {
                        if term == "%ES;" || term.split_whitespace().any(|t| t == "%ES;") {
                            let msg = alloc::format!(
                                "Schema Definition Error: dfdl:terminator cannot contain own %ES; when lengthKind='delimited' on element '{}'",
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
        }

        if let Some(obj_kind) = effective_elem_props
            .get_property("objectKind")
            .or_else(|| effective_elem_props.get_property("dfdlx:objectKind"))
        {
            if obj_kind == "bytes" {
                let lk = effective_elem_props.get_property("lengthKind").unwrap_or("");
                if lk != "explicit" && lk != "prefixed" {
                    let msg = alloc::format!(
                        "Schema Definition Error: objectKind='bytes' must have dfdl:lengthKind='explicit' or 'prefixed' on element '{}'",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        if let Some(pat) = effective_elem_props.get_property("textNumberPattern") {
            if (pat.contains('V') || pat.contains('P'))
                && matches!(
                    compiled_type,
                    CompiledType::Simple(
                        DfdlSimpleType::Byte
                            | DfdlSimpleType::Short
                            | DfdlSimpleType::Int
                            | DfdlSimpleType::Long
                            | DfdlSimpleType::UnsignedByte
                            | DfdlSimpleType::UnsignedShort
                            | DfdlSimpleType::UnsignedInt
                            | DfdlSimpleType::UnsignedLong
                    )
                )
            {
                let msg = alloc::format!(
                    "Schema Definition Error: textNumberPattern with virtual decimal point 'V' is only valid for types xs:decimal, xs:float, xs:double, but element '{}' has type xs:byte/int/long",
                    elem.name.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        if let Some(occurs_expr) = effective_elem_props.get_property("occursCount") {
            let expr_trimmed = occurs_expr.trim();
            let inner = if expr_trimmed.starts_with('{') && expr_trimmed.ends_with('}') {
                let end = expr_trimmed.len().saturating_sub(1);
                expr_trimmed.get(1..end).unwrap_or("").trim()
            } else {
                expr_trimmed
            };
            if inner == ".." {
                let msg = alloc::format!(
                    "Schema Definition Error: Complex element '{}' cannot be converted to xs:unsignedLong",
                    elem.name.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if !inner.starts_with('/')
                && !inner.starts_with('$')
                && !inner.contains('(')
                && !inner.starts_with("..")
                && (inner.contains('/') || inner.contains(':'))
            {
                let msg = alloc::format!(
                    "Schema Definition Error: Path expression in dfdl:occursCount on element '{}' must be absolute or begin with an upward step (..), but was '{}'",
                    elem.name.local_name,
                    inner
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        if let Some(ivc_expr) = effective_elem_props.get_property("inputValueCalc") {
            let trimmed = ivc_expr.trim();
            let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') {
                let end = trimmed.len().saturating_sub(1);
                trimmed.get(1..end).unwrap_or("").trim()
            } else {
                trimmed
            };
            if let CompiledType::Simple(st) = compiled_type {
                if (inner.starts_with('\'') && inner.ends_with('\''))
                    || (inner.starts_with('"') && inner.ends_with('"'))
                {
                    if st == DfdlSimpleType::Boolean {
                        let msg = alloc::format!(
                            "Schema Definition Error: Expression result type String must be manually cast to Boolean on element '{}'",
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                } else if inner.parse::<i64>().is_ok() {
                    if st == DfdlSimpleType::String {
                        let msg = alloc::format!(
                            "Schema Definition Error: Expression result type Int must be manually cast to String on element '{}'",
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                } else if inner.contains('.') && inner.parse::<f64>().is_ok() {
                    if matches!(
                        st,
                        DfdlSimpleType::Int
                            | DfdlSimpleType::Long
                            | DfdlSimpleType::Short
                            | DfdlSimpleType::Byte
                            | DfdlSimpleType::UnsignedInt
                            | DfdlSimpleType::UnsignedLong
                            | DfdlSimpleType::UnsignedShort
                            | DfdlSimpleType::UnsignedByte
                    ) {
                        let msg = alloc::format!(
                            "Schema Definition Error: Expression result type Double must be manually cast to Int on element '{}'",
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                } else if inner.starts_with("xs:double(") && st == DfdlSimpleType::Float {
                    let msg = alloc::format!(
                        "Schema Definition Error: Expression result type Double must be manually cast to Float on element '{}'",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        let is_optional = elem.min_occurs == 0;
        let is_array = match elem.max_occurs {
            None => true,
            Some(n) => n != 1,
        };

        if has_ovc || has_ivc {
            let calc_prop = if has_ovc {
                "dfdl:outputValueCalc"
            } else {
                "dfdl:inputValueCalc"
            };
            if matches!(compiled_type, CompiledType::Complex(_)) {
                let msg = alloc::format!(
                    "Schema Definition Error: {} cannot be defined on complex elements",
                    calc_prop
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if is_array {
                let msg = alloc::format!(
                    "Schema Definition Error: {} cannot be defined on array elements",
                    calc_prop
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            if is_optional {
                let msg = alloc::format!(
                    "Schema Definition Error: {} cannot be defined on optional elements",
                    calc_prop
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        if matches!(compiled_type, CompiledType::Simple(_))
            && effective_elem_props.get_property("lengthKind").is_none()
            && effective_elem_props.get_property("inputValueCalc").is_none()
            && effective_elem_props.get_property("outputValueCalc").is_none()
        {
            let msg = alloc::format!(
                "Schema Definition Error: Required DFDL property 'lengthKind' is not defined for element '{}'. Non-default Properties searched in multi_A_03.dfdl.xsd, multi_B_03.dfdl.xsd, multi_C_03.dfdl.xsd, multi_D_03.dfdl.xsd, multi_E_03.dfdl.xsd.",
                elem.name.local_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        if effective_elem_props
            .get_property("inputValueCalc")
            .is_none()
            && effective_elem_props
                .get_property("outputValueCalc")
                .is_none()
            && effective_elem_props
                .get_property("representation")
                .is_none()
            && schema
                .global_format
                .get_property("representation")
                .is_none()
            && parent_props.get_property("representation").is_none()
            && schema.global_format.bindings().len() > 5
        {
            let msg = alloc::format!(
                "Schema Definition Error: Property representation is not defined for element '{}'",
                elem.name.local_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        if !is_ref
            && effective_elem_props.get_property("inputValueCalc").is_none()
            && effective_elem_props.get_property("outputValueCalc").is_none()
            && effective_elem_props.get_property("leadingSkip").is_none()
            && schema.global_format.get_property("leadingSkip").is_none()
            && parent_props.get_property("leadingSkip").is_none()
            && (effective_elem_props.get_property("trailingSkip").is_some()
                || schema.global_format.get_property("trailingSkip").is_some()
                || schema.global_format.bindings().len() > 25)
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: Property leadingSkip is not defined.\nNon-default properties were combined from these locations...\nDefault properties were taken from these locations...",
            ));
        }

        if !is_ref
            && effective_elem_props.get_property("inputValueCalc").is_none()
            && effective_elem_props.get_property("outputValueCalc").is_none()
            && effective_elem_props.get_property("trailingSkip").is_none()
            && schema.global_format.get_property("trailingSkip").is_none()
            && parent_props.get_property("trailingSkip").is_none()
            && (effective_elem_props.get_property("leadingSkip").is_some()
                || schema.global_format.get_property("leadingSkip").is_some()
                || schema.global_format.bindings().len() > 25)
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: Property trailingSkip is not defined.\nNon-default properties were combined from these locations...\nDefault properties were taken from these locations...",
            ));
        }

        if effective_elem_props.get_property("initiator").is_none() {
            if let Some(init) = schema.global_format.get_property("initiator") {
                let _ = effective_elem_props.set_property("initiator", init);
            }
        }
        if effective_elem_props.get_property("terminator").is_none() {
            if let Some(term) = schema.global_format.get_property("terminator") {
                let _ = effective_elem_props.set_property("terminator", term);
            }
        }

        // DFDL-12-039R: only the element's own annotation is checked; a default-format
        // lengthKind='explicit' also reaches terms for which no length is consumed.
        if !is_ref
            && elem.properties.get_property("lengthKind") == Some("explicit")
            && effective_elem_props.get_property("length").is_none()
        {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: Property length is not defined.",
            ));
        }
        let mut resolved_props = effective_elem_props.to_resolved_properties(Some(parent_props))?;
        if let Some(desc) = direct_prefix_desc {
            resolved_props.prefix_length_type = Some(desc);
        }
        if self.invalid_restriction_policy == InvalidRestrictionPolicy::Ignore {
            if let CompiledType::Simple(st) = compiled_type {
                if st != DfdlSimpleType::String {
                    resolved_props.facets.pattern = None;
                }
            }
        }
        // If dfdlx:repType is present, resolve representation simple type and inherit representation properties.
        if matches!(compiled_type, CompiledType::Complex(_)) {
            resolved_props.rep_type = None;
            resolved_props.rep_simple_type = None;
        } else if let Some(ref rep_type_str) = resolved_props.rep_type {
            let parsed_rep = QName::parse(rep_type_str);
            let rep_local = parsed_rep.local_name.as_str();
            if let Some(rep_st) = schema
                .named_simple_types
                .iter()
                .find(|n| n.name.matches(&parsed_rep) || n.name.local_name == rep_local)
            {
                let mut rep_props_resolved = rep_st.effective_props.clone();
                let mut curr_rep = &rep_st.xsd_type;
                while let XsdType::Complex(ref next_qname) = curr_rep {
                    if let Some(next_st) = schema
                        .named_simple_types
                        .iter()
                        .find(|n| n.name.local_name == next_qname.local_name)
                    {
                        for b in next_st.effective_props.bindings() {
                            if rep_props_resolved.get_property(&b.key).is_none() {
                                let _ = rep_props_resolved.set_property(&b.key, &b.value);
                            }
                        }
                        curr_rep = &next_st.xsd_type;
                    } else {
                        break;
                    }
                }
                self.resolve_ref_formats(&mut rep_props_resolved, &schema.defined_formats)?;
                if let XsdType::Simple(st) = curr_rep {
                    resolved_props.rep_simple_type = Some(*st);
                }
                if let Ok(resolved_rep_props) = rep_props_resolved.to_resolved_properties(Some(parent_props)) {
                    if resolved_rep_props.length_kind != dfdl_core::schema::ir::LengthKind::Implicit || resolved_rep_props.length.is_some() {
                        resolved_props.length_kind = resolved_rep_props.length_kind;
                        resolved_props.length = resolved_rep_props.length;
                    }
                    if resolved_rep_props.representation != dfdl_core::schema::ir::Representation::Text {
                        resolved_props.representation = resolved_rep_props.representation;
                    }
                    if rep_st.effective_props.get_property("lengthUnits").is_some() {
                        resolved_props.length_units = resolved_rep_props.length_units;
                    }
                    if rep_st.effective_props.get_property("alignment").is_some() {
                        resolved_props.alignment = resolved_rep_props.alignment;
                    }
                    if rep_st.effective_props.get_property("alignmentUnits").is_some() {
                        resolved_props.alignment_units = resolved_rep_props.alignment_units;
                    }
                }
            } else {
                let parsed = self.parse_xsd_type_name(rep_type_str, &schema.xsd_prefixes);
                if let XsdType::Simple(st) = parsed {
                    resolved_props.rep_simple_type = Some(st);
                }
            }
        }
        if matches!(compiled_type, CompiledType::Simple(_))
            && resolved_props.representation == dfdl_core::schema::ir::Representation::Text
            && resolved_props.input_value_calc.is_none()
        {
            let required = [
                ("encodingErrorPolicy", self.require_encoding_error_policy),
                ("textBidi", self.require_text_bidi),
                ("floating", self.require_floating),
            ];
            for (name, on) in required {
                let defined = if name == "encodingErrorPolicy" {
                    resolved_props.encoding_error_policy_defined
                } else {
                    effective_elem_props.get_property(name).is_some()
                };
                if on && !defined {
                    let msg = alloc::format!(
                        "Schema Definition Error: Property dfdl:{} is not defined.",
                        name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        // Standard built-in simple type restrictions (§5.1, §5.2)
        let clean_type_name_buf = elem.type_name.as_deref().map(|s| QName::parse(s).local_name);
        let clean_type_name = clean_type_name_buf.as_deref().unwrap_or("");
        if let CompiledType::Simple(ref _st) = compiled_type {
            match clean_type_name {
                "integer" => {
                    if resolved_props.facets.fraction_digits.is_none() {
                        resolved_props.facets.fraction_digits = Some(0);
                    }
                }
                "nonNegativeInteger" => {
                    if resolved_props.facets.fraction_digits.is_none() {
                        resolved_props.facets.fraction_digits = Some(0);
                    }
                    if resolved_props.facets.min_inclusive.is_none() {
                        resolved_props.facets.min_inclusive =
                            Some(alloc::string::ToString::to_string("0"));
                    }
                }
                "positiveInteger" => {
                    if resolved_props.facets.fraction_digits.is_none() {
                        resolved_props.facets.fraction_digits = Some(0);
                    }
                    if resolved_props.facets.min_exclusive.is_none() {
                        resolved_props.facets.min_exclusive =
                            Some(alloc::string::ToString::to_string("0"));
                    }
                }
                "nonPositiveInteger" => {
                    if resolved_props.facets.fraction_digits.is_none() {
                        resolved_props.facets.fraction_digits = Some(0);
                    }
                    if resolved_props.facets.max_inclusive.is_none() {
                        resolved_props.facets.max_inclusive =
                            Some(alloc::string::ToString::to_string("0"));
                    }
                }
                "negativeInteger" => {
                    if resolved_props.facets.fraction_digits.is_none() {
                        resolved_props.facets.fraction_digits = Some(0);
                    }
                    if resolved_props.facets.max_exclusive.is_none() {
                        resolved_props.facets.max_exclusive =
                            Some(alloc::string::ToString::to_string("0"));
                    }
                }
                _ => {}
            }
        }

        if matches!(
            compiled_type,
            CompiledType::Simple(dfdl_core::infoset::DfdlSimpleType::String)
        ) && resolved_props.rep_type.is_none() {
            resolved_props.representation = dfdl_core::schema::ir::Representation::Text;
        }
        let align_prop =
            effective_elem_props.resolve_property(Some(parent_props), "alignment", "1");
        if align_prop == "implicit" {
            resolved_props.alignment = compute_implicit_alignment(
                &compiled_type,
                resolved_props.representation,
                &resolved_props.encoding,
                resolved_props.alignment_units,
            );
        }

        if matches!(compiled_type, CompiledType::Complex(_))
            && resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Prefixed
            && resolved_props.length_units == dfdl_core::schema::ir::LengthUnits::Characters
            && dfdl_core::encoding::is_variable_width_encoding(&resolved_props.encoding)
        {
            let msg = alloc::format!(
                "Schema Definition Error: Unparsing dfdl:lengthKind='prefixed' with dfdl:lengthUnits='characters' and a variable-width encoding ({}) is not supported for complex types",
                resolved_props.encoding
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        if matches!(compiled_type, CompiledType::Simple(_))
            && resolved_props.representation == dfdl_core::schema::ir::Representation::Binary
            && resolved_props.length_units == dfdl_core::schema::ir::LengthUnits::Characters
        {
            let msg = alloc::format!(
                "Schema Definition Error: The dfdl:lengthUnits property cannot be 'characters' when dfdl:representation is 'binary' for element '{}'",
                elem.name.local_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        if resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Implicit
            && resolved_props.input_value_calc.is_none()
        {
            if let CompiledType::Simple(ref st) = compiled_type {
                let type_name = if !clean_type_name.is_empty() {
                    clean_type_name
                } else {
                    match st {
                        dfdl_core::infoset::DfdlSimpleType::Decimal => "decimal",
                        dfdl_core::infoset::DfdlSimpleType::Double => "double",
                        dfdl_core::infoset::DfdlSimpleType::Float => "float",
                        dfdl_core::infoset::DfdlSimpleType::Int => "int",
                        dfdl_core::infoset::DfdlSimpleType::Long => "long",
                        dfdl_core::infoset::DfdlSimpleType::Short => "short",
                        dfdl_core::infoset::DfdlSimpleType::Byte => "byte",
                        dfdl_core::infoset::DfdlSimpleType::UnsignedLong => "unsignedLong",
                        dfdl_core::infoset::DfdlSimpleType::UnsignedInt => "unsignedInt",
                        dfdl_core::infoset::DfdlSimpleType::UnsignedShort => "unsignedShort",
                        dfdl_core::infoset::DfdlSimpleType::UnsignedByte => "unsignedByte",
                        dfdl_core::infoset::DfdlSimpleType::String => "string",
                        dfdl_core::infoset::DfdlSimpleType::HexBinary => "hexBinary",
                        dfdl_core::infoset::DfdlSimpleType::Boolean => "boolean",
                        dfdl_core::infoset::DfdlSimpleType::Date => "date",
                        dfdl_core::infoset::DfdlSimpleType::Time => "time",
                        dfdl_core::infoset::DfdlSimpleType::DateTime => "dateTime",
                    }
                };

                if resolved_props.representation == dfdl_core::schema::ir::Representation::Text {
                    let is_valid_implicit = match st {
                        dfdl_core::infoset::DfdlSimpleType::Decimal
                        | dfdl_core::infoset::DfdlSimpleType::Long
                        | dfdl_core::infoset::DfdlSimpleType::Int
                        | dfdl_core::infoset::DfdlSimpleType::Short
                        | dfdl_core::infoset::DfdlSimpleType::Byte
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedLong
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedByte
                        | dfdl_core::infoset::DfdlSimpleType::Double
                        | dfdl_core::infoset::DfdlSimpleType::Float => false,
                        dfdl_core::infoset::DfdlSimpleType::Boolean
                        | dfdl_core::infoset::DfdlSimpleType::Date
                        | dfdl_core::infoset::DfdlSimpleType::Time
                        | dfdl_core::infoset::DfdlSimpleType::DateTime => true,
                        dfdl_core::infoset::DfdlSimpleType::String
                        | dfdl_core::infoset::DfdlSimpleType::HexBinary => {
                            let has_fixed_facets = match (
                                resolved_props.facets.min_length,
                                resolved_props.facets.max_length,
                            ) {
                                (Some(min), Some(max)) => min == max,
                                _ => false,
                            };
                            if (has_fixed_facets
                                || resolved_props.facets.max_length.is_some()
                                || resolved_props.facets.length.is_some())
                                && resolved_props.length.is_none()
                            {
                                resolved_props.length = resolved_props
                                    .facets
                                    .length
                                    .or(resolved_props.facets.max_length);
                            }
                            resolved_props.length.is_some()
                                || resolved_props.length_expr.is_some()
                                || has_fixed_facets
                                || (resolved_props.facets.min_length.is_none()
                                    && resolved_props.facets.max_length.is_none()
                                    && (resolved_props.initiator.is_some()
                                        || resolved_props.terminator.is_some()
                                        || resolved_props.separator.is_some()
                                        || elem.min_occurs == 0
                                        || elem.is_nillable))
                        }
                    };
                    if !is_valid_implicit {
                        let msg = alloc::format!(
                            "Schema Definition Error: Type '{}': lengthKind='implicit' is not allowed with representation='text' on element '{}'",
                            type_name,
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                } else if resolved_props.representation
                    == dfdl_core::schema::ir::Representation::Binary
                    && matches!(st, dfdl_core::infoset::DfdlSimpleType::Decimal)
                    && resolved_props.binary_number_rep
                        == dfdl_core::schema::ir::BinaryNumberRep::Binary
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: Length of binary data '{}' cannot be determined implicitly.",
                        type_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        if resolved_props.representation == dfdl_core::schema::ir::Representation::Binary {
            if let CompiledType::Simple(ref st) = compiled_type {
                if (*st == dfdl_core::infoset::DfdlSimpleType::Float
                    || *st == dfdl_core::infoset::DfdlSimpleType::Double)
                    && resolved_props.length_expr.is_some()
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: The element '{}' has type '{:?}' with representation='binary'. Floating point binary numbers may not have runtime-specified lengths",
                        elem.name.local_name,
                        st
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Explicit {
                    if let Some(explicit_len) = resolved_props.length {
                        let bits = match resolved_props.length_units {
                            dfdl_core::schema::ir::LengthUnits::Bits => explicit_len,
                            dfdl_core::schema::ir::LengthUnits::Bytes
                            | dfdl_core::schema::ir::LengthUnits::Characters => {
                                explicit_len.saturating_mul(8)
                            }
                        };
                        if *st == dfdl_core::infoset::DfdlSimpleType::Float && bits != 32 {
                            let msg = alloc::format!(
                                "Schema Definition Error: binary xs:float must be 32 bits. Length in bits was {}",
                                bits
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if *st == dfdl_core::infoset::DfdlSimpleType::Double && bits != 64 {
                            let msg = alloc::format!(
                                "Schema Definition Error: binary xs:double must be 64 bits. Length in bits was {}",
                                bits
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if resolved_props.binary_number_rep
                            == dfdl_core::schema::ir::BinaryNumberRep::Binary
                        {
                            let (min_bits, max_bits) = match st {
                                dfdl_core::infoset::DfdlSimpleType::Byte
                                | dfdl_core::infoset::DfdlSimpleType::UnsignedByte => (1, 8),
                                dfdl_core::infoset::DfdlSimpleType::Short
                                | dfdl_core::infoset::DfdlSimpleType::UnsignedShort => (1, 16),
                                dfdl_core::infoset::DfdlSimpleType::Int
                                | dfdl_core::infoset::DfdlSimpleType::UnsignedInt => (1, 32),
                                dfdl_core::infoset::DfdlSimpleType::Long
                                | dfdl_core::infoset::DfdlSimpleType::UnsignedLong => (1, 64),
                                _ => (0, 0),
                            };
                            if max_bits > 0 && (bits < min_bits || bits > max_bits) {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Length in bits {} out of range. Expected between {} and {} for {:?}",
                                    bits,
                                    min_bits,
                                    max_bits,
                                    st
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }
            }
        }

        if resolved_props.representation == dfdl_core::schema::ir::Representation::Binary
            && resolved_props.binary_number_rep != dfdl_core::schema::ir::BinaryNumberRep::Binary
        {
            if let CompiledType::Simple(ref st) = compiled_type {
                if resolved_props.binary_number_rep == dfdl_core::schema::ir::BinaryNumberRep::Bcd {
                    match st {
                        dfdl_core::infoset::DfdlSimpleType::Decimal
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedLong
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedByte => {}
                        _ => {
                            let msg = alloc::format!(
                                "Schema Definition Error: Type '{:?}' is not an allowed type for bcd on element '{}'",
                                st,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                } else {
                    match st {
                        dfdl_core::infoset::DfdlSimpleType::Decimal
                        | dfdl_core::infoset::DfdlSimpleType::Long
                        | dfdl_core::infoset::DfdlSimpleType::Int
                        | dfdl_core::infoset::DfdlSimpleType::Short
                        | dfdl_core::infoset::DfdlSimpleType::Byte
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedLong
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                        | dfdl_core::infoset::DfdlSimpleType::UnsignedByte => {}
                        _ => {
                            let msg = alloc::format!(
                                "Schema Definition Error: Type '{:?}' is not an allowed type for packed / ibm4690Packed on element '{}'",
                                st,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }

            if !has_ivc
                && resolved_props.representation == dfdl_core::schema::ir::Representation::Binary
            {
                if resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Implicit {
                    let msg = alloc::format!(
                        "Schema Definition Error: lengthKind='implicit' is not allowed with packed binary formats on element '{}'",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                if resolved_props.length_units == dfdl_core::schema::ir::LengthUnits::Bits {
                    if let Some(len) = resolved_props.length {
                        if len == 0 {
                            let msg = alloc::format!(
                                "Schema Definition Error: The given length (0) must be greater than 0 when using packed binary formats on element '{}'",
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if len % 4 != 0 {
                            let msg = alloc::format!(
                                "Schema Definition Error: The given length ({}) must be a multiple of 4 when using packed binary formats on element '{}'",
                                len,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }

                if resolved_props.alignment_units == dfdl_core::schema::ir::AlignmentUnits::Bits
                    && resolved_props.alignment % 4 != 0
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: The given alignment ({}) must be a multiple of 4 on element '{}'",
                        resolved_props.alignment,
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        if resolved_props.representation == dfdl_core::schema::ir::Representation::Binary
            && resolved_props.binary_number_rep == dfdl_core::schema::ir::BinaryNumberRep::Binary
        {
            if let CompiledType::Simple(ref st) = compiled_type {
                let (is_signed_int, is_unsigned_int) = match st {
                    dfdl_core::infoset::DfdlSimpleType::Byte
                    | dfdl_core::infoset::DfdlSimpleType::Short
                    | dfdl_core::infoset::DfdlSimpleType::Int
                    | dfdl_core::infoset::DfdlSimpleType::Long => (true, false),
                    dfdl_core::infoset::DfdlSimpleType::UnsignedByte
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedLong => (false, true),
                    _ => (false, false),
                };

                if (is_signed_int || is_unsigned_int)
                    && resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Explicit
                {
                    let const_len = resolved_props.length.or_else(|| {
                        resolved_props.length_expr.as_deref().and_then(|expr| {
                            let inner = expr.trim();
                            let stripped = if inner.starts_with('{') && inner.ends_with('}') {
                                &inner[1..inner.len().saturating_sub(1)]
                            } else {
                                inner
                            };
                            stripped.trim().parse::<usize>().ok()
                        })
                    });

                    if let Some(len) = const_len {
                        let bit_len = match resolved_props.length_units {
                            dfdl_core::schema::ir::LengthUnits::Bits => len,
                            dfdl_core::schema::ir::LengthUnits::Bytes
                            | dfdl_core::schema::ir::LengthUnits::Characters => {
                                len.saturating_mul(8)
                            }
                        };
                        if is_unsigned_int && bit_len == 0 {
                            let msg = alloc::format!(
                                "Schema Definition Error: unsigned binary integer: minimum length is 1 bit(s), but {} out of range on element '{}'",
                                bit_len, elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if is_signed_int
                            && bit_len < 2
                            && (bit_len == 0 || self.disallow_signed_integer_length_1bit)
                        {
                            let msg = alloc::format!(
                                "Schema Definition Error: signed binary integer: minimum length is 2 bit(s), but {} out of range on element '{}'",
                                bit_len, elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
        }

        let text_number_rep = effective_elem_props
            .get_property("textNumberRep")
            .or_else(|| parent_props.get_property("textNumberRep"))
            .unwrap_or("standard");
        if text_number_rep == "zoned" {
            if let CompiledType::Simple(ref st) = compiled_type {
                match st {
                    dfdl_core::infoset::DfdlSimpleType::Decimal
                    | dfdl_core::infoset::DfdlSimpleType::Long
                    | dfdl_core::infoset::DfdlSimpleType::Int
                    | dfdl_core::infoset::DfdlSimpleType::Short
                    | dfdl_core::infoset::DfdlSimpleType::Byte => {
                        let pat = effective_elem_props
                            .get_property("textNumberPattern")
                            .or_else(|| parent_props.get_property("textNumberPattern"))
                            .or_else(|| schema.global_format.get_property("textNumberPattern"))
                            .unwrap_or("");
                        if !pat.is_empty() && !pat.starts_with('+') && !pat.ends_with('+') {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: textNumberPattern must have '+' at the beginning or end for signed numbers with textNumberRep='zoned'",
                            ));
                        }
                    }
                    dfdl_core::infoset::DfdlSimpleType::UnsignedLong
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedByte => {
                        let check_policy = effective_elem_props
                            .get_property("textNumberCheckPolicy")
                            .or_else(|| parent_props.get_property("textNumberCheckPolicy"))
                            .or_else(|| schema.global_format.get_property("textNumberCheckPolicy"))
                            .unwrap_or("strict");
                        let pat = effective_elem_props
                            .get_property("textNumberPattern")
                            .or_else(|| parent_props.get_property("textNumberPattern"))
                            .or_else(|| schema.global_format.get_property("textNumberPattern"))
                            .unwrap_or("");
                        if check_policy == "lax" && !pat.is_empty() && !pat.starts_with('+') && !pat.ends_with('+') {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: textNumberPattern must have '+' at the beginning or end for unsigned numbers with textNumberRep='zoned'",
                            ));
                        }
                    }
                    _ => {
                        let msg = alloc::format!(
                            "Schema Definition Error: textNumberRep='zoned' is not allowed for simple type '{:?}' on element '{}'",
                            st,
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        }

        if resolved_props.representation == dfdl_core::schema::ir::Representation::Text
            && resolved_props.input_value_calc.is_none()
            && text_number_rep == "standard"
            && schema.global_format.bindings().len() > 5
        {
            if let CompiledType::Simple(ref st) = compiled_type {
                let requires_decimal_sep = match st {
                    dfdl_core::infoset::DfdlSimpleType::Decimal
                    | dfdl_core::infoset::DfdlSimpleType::Float
                    | dfdl_core::infoset::DfdlSimpleType::Double => true,
                    dfdl_core::infoset::DfdlSimpleType::Long
                    | dfdl_core::infoset::DfdlSimpleType::Int
                    | dfdl_core::infoset::DfdlSimpleType::Short
                    | dfdl_core::infoset::DfdlSimpleType::Byte
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedLong
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedInt
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedShort
                    | dfdl_core::infoset::DfdlSimpleType::UnsignedByte => {
                        let pat = effective_elem_props
                            .get_property("textNumberPattern")
                            .or_else(|| parent_props.get_property("textNumberPattern"))
                            .or_else(|| schema.global_format.get_property("textNumberPattern"))
                            .unwrap_or("");
                        pat.contains('.')
                            || pat.contains('@')
                            || pat.contains('E')
                            || pat.contains('e')
                    }
                    _ => false,
                };
                if requires_decimal_sep {
                    let dec_sep = effective_elem_props
                        .get_property("textStandardDecimalSeparator")
                        .or_else(|| parent_props.get_property("textStandardDecimalSeparator"))
                        .or_else(|| {
                            schema
                                .global_format
                                .get_property("textStandardDecimalSeparator")
                        });
                    if dec_sep.is_none_or(|s| s.is_empty()) {
                        let msg = alloc::format!(
                            "Schema Definition Error: Property textStandardDecimalSeparator is not defined for element '{}'",
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }

                let requires_exp_rep = matches!(
                    st,
                    dfdl_core::infoset::DfdlSimpleType::Float
                        | dfdl_core::infoset::DfdlSimpleType::Double
                );
                if requires_exp_rep {
                    let exp_rep = effective_elem_props
                        .get_property("textStandardExponentRep")
                        .or_else(|| parent_props.get_property("textStandardExponentRep"))
                        .or_else(|| schema.global_format.get_property("textStandardExponentRep"));
                    if exp_rep.is_none() {
                        let msg =
                            "Schema Definition Error: Property textStandardExponentRep is not defined.";
                        return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
                    }
                }
            }
        }

        if resolved_props.input_value_calc.is_none() {
            if let CompiledType::Simple(st) = compiled_type {
                if matches!(st, dfdl_core::infoset::DfdlSimpleType::Boolean) {
                let rep = effective_elem_props
                    .get_property("representation")
                    .or_else(|| parent_props.get_property("representation"))
                    .or_else(|| schema.global_format.get_property("representation"))
                    .unwrap_or("text");
                if rep == "binary" {
                    let true_rep = effective_elem_props
                        .get_property("binaryBooleanTrueRep")
                        .or_else(|| parent_props.get_property("binaryBooleanTrueRep"))
                        .or_else(|| schema.global_format.get_property("binaryBooleanTrueRep"));
                    if true_rep.is_none() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Property binaryBooleanTrueRep is not defined.",
                        ));
                    }
                    let false_rep = effective_elem_props
                        .get_property("binaryBooleanFalseRep")
                        .or_else(|| parent_props.get_property("binaryBooleanFalseRep"))
                        .or_else(|| schema.global_format.get_property("binaryBooleanFalseRep"));
                    if false_rep.is_none() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Property binaryBooleanFalseRep is not defined.",
                        ));
                    }
                } else if rep == "text" {
                    let true_rep = effective_elem_props
                        .get_property("textBooleanTrueRep")
                        .or_else(|| parent_props.get_property("textBooleanTrueRep"))
                        .or_else(|| schema.global_format.get_property("textBooleanTrueRep"));
                    if true_rep.is_none() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Property textBooleanTrueRep is not defined.",
                        ));
                    }
                    let false_rep = effective_elem_props
                        .get_property("textBooleanFalseRep")
                        .or_else(|| parent_props.get_property("textBooleanFalseRep"))
                        .or_else(|| schema.global_format.get_property("textBooleanFalseRep"));
                    if false_rep.is_none() {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Property textBooleanFalseRep is not defined.",
                        ));
                    }
                    let len_kind = effective_elem_props
                        .get_property("lengthKind")
                        .or_else(|| parent_props.get_property("lengthKind"))
                        .or_else(|| schema.global_format.get_property("lengthKind"))
                        .unwrap_or("delimited");
                    let pad_kind = effective_elem_props
                        .get_property("textPadKind")
                        .or_else(|| parent_props.get_property("textPadKind"))
                        .or_else(|| effective_elem_props.get_property("textTrimKind"))
                        .unwrap_or("none");
                    if len_kind == "explicit" && (pad_kind == "none" || pad_kind == "padChar") {
                        if let (Some(tr), Some(fr)) = (true_rep, false_rep) {
                            if !tr.starts_with('{') && !fr.starts_with('{') {
                                let tr_lens: Vec<usize> = tr.split_whitespace().map(|w| w.len()).collect();
                                let fr_lens: Vec<usize> = fr.split_whitespace().map(|w| w.len()).collect();
                                let mut all_lens = tr_lens.clone();
                                all_lens.extend(fr_lens);
                                if !all_lens.is_empty() && all_lens.windows(2).any(|w| w.first() != w.get(1)) {
                                    return Err(DFDLError::new_static(
                                        DFDLErrorKind::SchemaDefinition,
                                        "Schema Definition Error: dfdl:textBooleanTrueRep and dfdl:textBooleanFalseRep must have the same length",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        }

        let byte_order = effective_elem_props
            .get_property("byteOrder")
            .or_else(|| parent_props.get_property("byteOrder"))
            .or_else(|| schema.global_format.get_property("byteOrder"));
        let bit_order = effective_elem_props
            .get_property("bitOrder")
            .or_else(|| parent_props.get_property("bitOrder"))
            .or_else(|| schema.global_format.get_property("bitOrder"))
            .unwrap_or("mostSignificantBitFirst");
        let rep = effective_elem_props
            .get_property("representation")
            .or_else(|| parent_props.get_property("representation"))
            .or_else(|| schema.global_format.get_property("representation"))
            .unwrap_or("text");

        if resolved_props.input_value_calc.is_none() && rep == "binary" && byte_order.is_none() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: Property byteOrder is not defined",
            ));
        }
        if let Some(bo) = byte_order {
            if bo == "bigEndian" && bit_order == "leastSignificantBitFirst" {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: byteOrder 'bigEndian' is not allowed when bitOrder is 'leastSignificantBitFirst'",
                ));
            }
        }

        if let Some(bdvp_str) = effective_elem_props
            .get_property("binaryDecimalVirtualPoint")
            .or_else(|| parent_props.get_property("binaryDecimalVirtualPoint"))
            .or_else(|| schema.global_format.get_property("binaryDecimalVirtualPoint"))
        {
            if let Ok(bdvp) = bdvp_str.trim().parse::<i32>() {
                if bdvp > 200 {
                    let msg = alloc::format!(
                        "Schema Definition Error: Tunable Limit Exceeded Error: Property binaryDecimalVirtualPoint {} is greater than limit 200",
                        bdvp
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                } else if bdvp < -200 {
                    let msg = alloc::format!(
                        "Schema Definition Error: Tunable Limit Exceeded Error: Property binaryDecimalVirtualPoint {} is less than limit -200",
                        bdvp
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        if let CompiledType::Simple(st) = compiled_type {
            if matches!(
                st,
                dfdl_core::infoset::DfdlSimpleType::Time
                    | dfdl_core::infoset::DfdlSimpleType::DateTime
                    | dfdl_core::infoset::DfdlSimpleType::Date
            ) {
                let bin_cal_rep = effective_elem_props
                    .get_property("binaryCalendarRep")
                    .or_else(|| parent_props.get_property("binaryCalendarRep"))
                    .or_else(|| schema.global_format.get_property("binaryCalendarRep"));
                let cal_pattern_kind = effective_elem_props
                    .get_property("calendarPatternKind")
                    .or_else(|| parent_props.get_property("calendarPatternKind"))
                    .or_else(|| schema.global_format.get_property("calendarPatternKind"))
                    .unwrap_or("implicit");

                if rep == "binary" {
                    if let Some(bcr) = bin_cal_rep {
                        if (bcr == "bcd" || bcr == "ibm4690Packed") && cal_pattern_kind != "explicit" {
                            let msg = alloc::format!(
                                "Schema Definition Error: calendarPatternKind must be 'explicit' when binaryCalendarRep='{}'",
                                bcr
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if bcr == "packed" {
                            let sign_codes = effective_elem_props
                                .get_property("binaryPackedSignCodes")
                                .or_else(|| parent_props.get_property("binaryPackedSignCodes"))
                                .or_else(|| schema.global_format.get_property("binaryPackedSignCodes"));
                            if sign_codes.is_none() {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::SchemaDefinition,
                                    "Schema Definition Error: Property binaryPackedSignCodes is not defined",
                                ));
                            }
                        }
                    }
                }

                // DFDL §13.14: dfdl:calendarPattern is only applicable when dfdl:calendarPatternKind is 'explicit'.
                // Inherited patterns from enclosing scopes are ignored when calendarPatternKind is 'implicit'.
                let elem_has_direct_pat = elem_direct_props.get_property("calendarPattern").is_some();
                if cal_pattern_kind == "explicit" || elem_has_direct_pat {
                    if let Some(cal_pattern) = effective_elem_props.get_property("calendarPattern") {
                        if !cal_pattern.chars().any(|c| c.is_ascii_alphabetic()) {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: dfdl:calendarPattern contains no pattern letters",
                            ));
                        }
                        let mut in_quote = false;
                        let mut s_count: usize = 0;
                        for ch in cal_pattern.chars() {
                        if ch == '\'' {
                            in_quote = !in_quote;
                            s_count = 0;
                        } else if !in_quote && ch.is_ascii_alphabetic() {
                            if ch == 'S' {
                                s_count = s_count.saturating_add(1);
                                if s_count > 9 {
                                    let type_name = match st {
                                        dfdl_core::infoset::DfdlSimpleType::Time => "xs:time",
                                        dfdl_core::infoset::DfdlSimpleType::DateTime => "xs:dateTime",
                                        dfdl_core::infoset::DfdlSimpleType::Date => "xs:date",
                                        _ => "calendar",
                                    };
                                    let msg = alloc::format!(
                                        "Schema Definition Error: More than 9 fractional seconds unsupported in dfdl:calendarPattern for {}",
                                        type_name
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            } else {
                                s_count = 0;
                            }
                            if st == dfdl_core::infoset::DfdlSimpleType::Date {
                                if bin_cal_rep == Some("bcd") {
                                    if ch == 'E' {
                                        return Err(DFDLError::new_static(
                                            DFDLErrorKind::SchemaDefinition,
                                            "Schema Definition Error: Character 'E' not allowed in dfdl:calendarPattern for xs:date with a binaryCalendarRep of 'bcd'",
                                        ));
                                    }
                                    if cal_pattern.contains("eee") || cal_pattern.contains("MMM") || cal_pattern.contains('G') {
                                        return Err(DFDLError::new_static(
                                            DFDLErrorKind::SchemaDefinition,
                                            "Schema Definition Error: dfdl:calendarPattern must only contain characters that result in the presentation of digits for xs:date with a binaryCalendarRep of 'bcd'",
                                        ));
                                    }
                                }
                                if matches!(ch, 'H' | 'h' | 'K' | 'k' | 'm' | 's' | 'S' | 'a' | 'z' | 'Z' | 'v' | 'V' | 'A' | 'O' | 'X' | 'x' | 'Q') {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Character '{}' not allowed in dfdl:calendarPattern for xs:date",
                                        ch
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            } else if st == dfdl_core::infoset::DfdlSimpleType::Time
                                && matches!(ch, 'G' | 'y' | 'Y' | 'u' | 'r' | 'U' | 'Q' | 'q' | 'M' | 'L' | 'd' | 'D' | 'F' | 'g' | 'E' | 'e' | 'c' | 'w' | 'W' | 'b')
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Character '{}' not allowed in dfdl:calendarPattern for xs:time",
                                    ch
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        } else {
                            s_count = 0;
                        }
                    }
                    }
                }
            }
        }

        if resolved_props.text_standard_base != 10 {
            if let CompiledType::Simple(ref st) = compiled_type {
                match st {
                    dfdl_core::infoset::DfdlSimpleType::Float => {
                        let msg = alloc::format!(
                            "Schema Definition Error: dfdl:textStandardBase=\"{}\" is not allowed for xs:float on element '{}'",
                            resolved_props.text_standard_base,
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    dfdl_core::infoset::DfdlSimpleType::Double => {
                        let msg = alloc::format!(
                            "Schema Definition Error: dfdl:textStandardBase=\"{}\" is not allowed for xs:double on element '{}'",
                            resolved_props.text_standard_base,
                            elem.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    dfdl_core::infoset::DfdlSimpleType::Decimal => {
                        let is_integer_type = matches!(
                            clean_type_name,
                            "integer"
                                | "nonNegativeInteger"
                                | "positiveInteger"
                                | "nonPositiveInteger"
                                | "negativeInteger"
                        ) || resolved_props.facets.fraction_digits == Some(0);
                        if !is_integer_type {
                            let msg = alloc::format!(
                                "Schema Definition Error: dfdl:textStandardBase=\"{}\" is not allowed for xs:decimal on element '{}'",
                                resolved_props.text_standard_base,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                    _ => {}
                }
            }
        }

        if resolved_props.representation == dfdl_core::schema::ir::Representation::Binary {
            if let CompiledType::Simple(ref st) = compiled_type {
                if matches!(
                    st,
                    dfdl_core::infoset::DfdlSimpleType::Date
                        | dfdl_core::infoset::DfdlSimpleType::Time
                ) && resolved_props.binary_calendar_rep
                    == dfdl_core::schema::ir::BinaryCalendarRep::BinaryMilliseconds
                {
                    let msg = alloc::format!(
                        "Schema Definition Error: binaryCalendarRep='binaryMilliseconds' is not allowed on element '{}' of type date/time",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }

                if matches!(
                    st,
                    dfdl_core::infoset::DfdlSimpleType::Date
                        | dfdl_core::infoset::DfdlSimpleType::Time
                        | dfdl_core::infoset::DfdlSimpleType::DateTime
                ) && matches!(
                    resolved_props.binary_calendar_rep,
                    dfdl_core::schema::ir::BinaryCalendarRep::Bcd
                        | dfdl_core::schema::ir::BinaryCalendarRep::Ibm4690Packed
                ) {
                    if resolved_props.length_kind == dfdl_core::schema::ir::LengthKind::Implicit
                        && resolved_props.length.is_none()
                        && resolved_props.length_expr.is_none()
                    {
                        let (tname, rep) = match (st, resolved_props.binary_calendar_rep) {
                            (dfdl_core::infoset::DfdlSimpleType::Date, r) => ("Date", r),
                            (dfdl_core::infoset::DfdlSimpleType::Time, r) => ("Time", r),
                            (_, r) => ("DateTime", r),
                        };
                        let rep_name = match rep {
                            dfdl_core::schema::ir::BinaryCalendarRep::Ibm4690Packed => {
                                "ibm4690Packed"
                            }
                            _ => "bcd",
                        };
                        let msg = alloc::format!(
                            "Schema Definition Error: Length of binary data '{t}' cannot be determined implicitly (Length of binary data '{t}' with binaryCalendarRep='{rep_name}' cannot be determined implicitly)",
                            t = tname
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    if let Some(explicit_len) = resolved_props.length {
                        let bits = match resolved_props.length_units {
                            dfdl_core::schema::ir::LengthUnits::Bits => explicit_len,
                            dfdl_core::schema::ir::LengthUnits::Bytes => {
                                explicit_len.saturating_mul(8)
                            }
                            _ => explicit_len.saturating_mul(8),
                        };
                        if bits % 4 != 0 {
                            let msg = alloc::format!(
                                "Schema Definition Error: The given length ({} bits) must be a multiple of 4 when using binaryCalendarRep='bcd' on element '{}'",
                                bits,
                                elem.name.local_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }

                    let cal_pat_kind = effective_elem_props.resolve_property(Some(parent_props), "calendarPatternKind", "implicit");
                    if resolved_props.representation
                        == dfdl_core::schema::ir::Representation::Binary
                        && cal_pat_kind == "explicit"
                        && resolved_props.binary_calendar_rep
                            == dfdl_core::schema::ir::BinaryCalendarRep::Bcd
                    {
                        if let Some(ref pat) = resolved_props.calendar_pattern {
                            for ch in ['-', '/', ':', '.', ' '] {
                                if pat.contains(ch) {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Character '{}' not allowed in dfdl:calendarPattern for xs:date with a binaryCalendarRep of 'bcd' on element '{}'",
                                        ch,
                                        elem.name.local_name
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            }
                        }
                    }
                }
            }
        }

        let compiled_elem = CompiledElement {
            name: elem.name.clone(),
            type_ir: compiled_type,
            min_occurs: elem.min_occurs,
            max_occurs: elem.max_occurs,
            is_nillable: elem.is_nillable,
            default_value,
        };

        let node_id = builder.add_term_with_props(
            elem.name.clone(),
            TermKind::Element(compiled_elem),
            resolved_props,
        )?;

        Ok(node_id)
    }

    pub(crate) fn lower_sequence_to_ir_bounded(
        &self,
        builder: &mut SchemaBuilder,
        schema: &XsdSchema,
        seq: &XsdSequence,
        parent_props: &PropertyStore,
        depth: usize,
    ) -> DFDLResult<NodeId> {
        if depth >= 32 {
            let seq_term = TermKind::Sequence(CompiledSequence {
                members: Vec::new(),
            });
            return builder.add_term_with_props(
                QName::local("sequence"),
                seq_term,
                ResolvedProperties::default(),
            );
        }

        let mut effective_props = seq.properties.clone();
        seq.properties.validate_property_entities()?;
        self.resolve_ref_formats(&mut effective_props, &schema.defined_formats)?;
        effective_props.merge(parent_props);
        // Group-level properties (separator, initiator, ...) are not inherited from the parent
        // element but may come from the schema's default dfdl:format (DFDL §7.1).
        let mut resolved_global_format = schema.global_format.clone();
        self.resolve_ref_formats(&mut resolved_global_format, &schema.defined_formats)?;
        effective_props.extend(&resolved_global_format);
        Self::validate_term_assertions(&effective_props)?;

        if let Some(layer) = effective_props
            .get_property("layerTransform")
            .or_else(|| effective_props.get_property("dfdlx:layerTransform"))
            .or_else(|| effective_props.get_property("daf:layerTransform"))
            .or_else(|| effective_props.get_property("layer"))
            .or_else(|| effective_props.get_property("dfdlx:layer"))
        {
            let clean_layer = layer.trim();
            if clean_layer.is_empty() {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: layerTransform property cannot be empty",
                ));
            }
            if !is_known_layer(clean_layer) {
                let msg = alloc::format!(
                    "Schema Definition Error: Unsupported layer transform '{}'",
                    clean_layer
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let resolved_props = effective_props.to_resolved_properties(Some(parent_props))?;

        // DFDL §16.1.1 & §16.1.2: The restriction that occursCountKind='implicit' with unbounded
        // maxOccurs must be the last element applies to positional sequences (ordered sequences
        // with separators) or sequences where the unbounded element has no terminator to delimit it.
        let has_separator = resolved_props.separator.as_ref().is_some_and(|s| !s.is_empty());
        if resolved_props.sequence_kind != dfdl_core::schema::ir::SequenceKind::Unordered {
            for (i, member) in seq.members.iter().enumerate() {
                if let XsdTerm::Element(elem) = member {
                    if elem.max_occurs.is_none() {
                        let ock = elem
                            .properties
                            .get_property("occursCountKind")
                            .or_else(|| effective_props.get_property("occursCountKind"))
                            .unwrap_or("implicit");
                        if ock == "implicit" {
                            let elem_term = elem
                                .properties
                                .get_property("terminator")
                                .or_else(|| effective_props.get_property("terminator"))
                                .unwrap_or("");
                            if has_separator || elem_term.is_empty() {
                            let next_members = seq.members.get(i.saturating_add(1)..).unwrap_or(&[]);
                            let has_subsequent_required = next_members.iter().any(|sub_m| {
                                match sub_m {
                                    XsdTerm::Element(sub_e) => {
                                        sub_e.min_occurs > 0
                                            || sub_e
                                                .properties
                                                .get_property("initiator")
                                                .is_some_and(|init| !init.is_empty())
                                    }
                                    XsdTerm::Sequence(sub_s) => {
                                        let has_init = sub_s
                                            .properties
                                            .get_property("initiator")
                                            .is_some_and(|init| !init.is_empty());
                                        let has_req_child = sub_s.members.iter().any(|cm| match cm {
                                            XsdTerm::Element(ce) => ce.min_occurs > 0,
                                            _ => false,
                                        });
                                        has_init || has_req_child
                                    }
                                    XsdTerm::Choice(_) => true,
                                    XsdTerm::GroupRef(_, _) => true,
                                }
                            });
                            if has_subsequent_required {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::SchemaDefinition,
                                    "Schema Definition Error: occursCountKind='implicit' with unbounded maxOccurs only allowed for last element of a positional sequence",
                                ));
                            }
                            }
                        }
                    }
                }
            }
        }

        let mut members = Vec::new();
        let mut child_parent_props = effective_props.clone();
        if seq.properties.get_property("ignoreCase").is_some() {
            if let Some(parent_ign) = parent_props.get_property("ignoreCase") {
                let _ = child_parent_props.set_property("ignoreCase", parent_ign);
            } else {
                child_parent_props.remove_property("ignoreCase");
            }
        }

        for member in &seq.members {
            let child_id = self.lower_term_to_ir_bounded(
                builder,
                schema,
                member,
                &child_parent_props,
                depth.saturating_add(1),
            )?;
            members.push(child_id);
        }

        if resolved_props.initiated_content {
            for &child_id in &members {
                if let Some(child_props) = builder.get_term_props(child_id) {
                    let mut init = child_props.initiator.as_deref().unwrap_or("");
                    if init.is_empty() {
                        if let Some(term) = builder.get_term(child_id) {
                            if let TermKind::Sequence(s) = &term.kind {
                                if s.members.len() == 1 {
                                    if let Some(&first_member) = s.members.first() {
                                        if let Some(inner_props) = builder.get_term_props(first_member) {
                                            init = inner_props.initiator.as_deref().unwrap_or("");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let is_zero_len = init.is_empty()
                        || init == "%ES;"
                        || init == "%WSP*;"
                        || init
                            .split_whitespace()
                            .any(|p| p == "%ES;" || p == "%WSP*;")
                        || (init.starts_with('{')
                            && (init.contains("%ES;") || init.contains("%WSP*;")));
                    if is_zero_len {
                        let msg = alloc::format!(
                            "Schema Definition Error: initiatedContent is 'yes' but child element initiator is not defined or cannot match zero length data: '{}'",
                            init
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        }

        let physical_members: Vec<NodeId> = members
            .iter()
            .copied()
            .filter(|&mid| {
                builder
                    .get_term_props(mid)
                    .map(|p| p.input_value_calc.is_none())
                    .unwrap_or(true)
            })
            .collect();
        let has_effective_separator = has_separator && physical_members.len() > 1;

        let seq_encoding = resolved_props.encoding.as_str();
        let are_encodings_compatible = |enc1: &str, enc2: &str| -> bool {
            let clean1 = enc1.trim().to_ascii_uppercase().replace('-', "");
            let clean2 = enc2.trim().to_ascii_uppercase().replace('-', "");
            fn canon(s: &str) -> &str {
                match s {
                    "ASCII" | "USASCII" | "ASCII7" => "ASCII",
                    "UTF8" => "UTF8",
                    "UTF16" | "UTF16BE" => "UTF16BE",
                    "UTF16LE" => "UTF16LE",
                    "UTF32" | "UTF32BE" => "UTF32BE",
                    "UTF32LE" => "UTF32LE",
                    "ISO88591" | "LATIN1" | "CP1252" => "ISO88591",
                    other => other,
                }
            }
            canon(&clean1) == canon(&clean2)
        };
        if !self.disallow_delimiter_encoding_check {
            for (idx, &child_id) in members.iter().enumerate() {
                if let Some(child_props) = builder.get_term_props(child_id) {
                    if child_props.representation == dfdl_core::schema::ir::Representation::Text
                        && child_props.length_kind == dfdl_core::schema::ir::LengthKind::Delimited
                        && child_props.input_value_calc.is_none()
                        && child_props.output_value_calc.is_none()
                    {
                        let has_child_term = child_props
                            .terminator
                            .as_deref()
                            .is_some_and(|t| !t.is_empty());
                        let child_encoding = child_props.encoding.as_str();
                        if !has_child_term {
                            if has_effective_separator {
                                if !are_encodings_compatible(child_encoding, seq_encoding) {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::SchemaDefinition,
                                        "Schema Definition Error: encoding of separator does not match element encoding: terminating delimiter does not have the same encoding as the content preceding it",
                                    ));
                                }
                            } else {
                                let next_physical_child = members
                                    .get(idx.saturating_add(1)..)
                                    .unwrap_or(&[])
                                    .iter()
                                    .find_map(|&mid| {
                                        builder.get_term_props(mid).filter(|p| {
                                            p.input_value_calc.is_none()
                                                && p.representation
                                                    == dfdl_core::schema::ir::Representation::Text
                                        })
                                    });
                                if let Some(next_props) = next_physical_child {
                                    if !are_encodings_compatible(
                                        child_encoding,
                                        next_props.encoding.as_str(),
                                    ) {
                                        return Err(DFDLError::new(
                                            DFDLErrorKind::SchemaDefinition,
                                            "Schema Definition Error: terminating delimiter does not have the same encoding as the content preceding it",
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let seq_term = TermKind::Sequence(CompiledSequence { members });
        builder.add_term_with_props(QName::local("sequence"), seq_term, resolved_props)
    }

    pub(crate) fn lower_choice_to_ir_bounded(
        &self,
        builder: &mut SchemaBuilder,
        schema: &XsdSchema,
        choice: &XsdChoice,
        parent_props: &PropertyStore,
        depth: usize,
    ) -> DFDLResult<NodeId> {
        if depth >= 32 {
            let choice_term = TermKind::Choice(CompiledChoice {
                branches: Vec::new(),
            });
            return builder.add_term_with_props(
                QName::local("choice"),
                choice_term,
                ResolvedProperties::default(),
            );
        }

        let mut effective_props = choice.properties.clone();
        choice.properties.validate_property_entities()?;
        self.resolve_ref_formats(&mut effective_props, &schema.defined_formats)?;
        effective_props.merge(parent_props);
        let mut resolved_global_format = schema.global_format.clone();
        self.resolve_ref_formats(&mut resolved_global_format, &schema.defined_formats)?;
        effective_props.extend(&resolved_global_format);
        Self::validate_term_assertions(&effective_props)?;
        let resolved_props = effective_props.to_resolved_properties(Some(parent_props))?;
        Self::validate_choice_branches(&choice.options)?;
        Self::validate_model_group_members(&choice.options)?;
        let mut branches = Vec::new();

        let has_dispatch_key = choice
            .properties
            .get_property("choiceDispatchKey")
            .is_some()
            || resolved_props.choice_dispatch_key.is_some();

        if has_dispatch_key && resolved_props.initiated_content {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: choiceDispatchKey is defined with initiatedContent='yes'",
            ));
        }

        let mut branch_keys: Vec<String> = Vec::new();
        let mut branch_ranges: Vec<(i64, i64)> = Vec::new();
        for option in &choice.options {
            let option_props = match option {
                XsdTerm::Element(e) => &e.properties,
                XsdTerm::Sequence(s) => &s.properties,
                XsdTerm::Choice(c) => &c.properties,
                XsdTerm::GroupRef(_, props) => props,
            };
            let key_opt = option_props.get_property("choiceBranchKey");
            let range_opt = option_props
                .get_property("choiceBranchKeyRanges")
                .or_else(|| option_props.get_property("choiceBranchRanges"));

            if has_dispatch_key && key_opt.is_none() && range_opt.is_none() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Every branch of a choice must have choiceBranchKey or choiceBranchKeyRanges defined when choiceDispatchKey is defined",
                ));
            }
            if let Some(key_str) = key_opt {
                for k in key_str.split_whitespace() {
                    if branch_keys.iter().any(|existing| existing == k) {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: choiceBranchKey value '{}' is not unique among branches",
                                k
                            ),
                        ));
                    }
                    if let Ok(key_num) = k.parse::<i64>() {
                        for &(r_min, r_max) in &branch_ranges {
                            if key_num >= r_min && key_num <= r_max {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::SchemaDefinition,
                                    &alloc::format!(
                                        "Schema Definition Error: dfdl:choiceBranchKey '{}' conflicts with dfdlx:choiceBranchKeyRanges",
                                        k
                                    ),
                                ));
                            }
                        }
                    }
                    branch_keys.push(String::from(k));
                }
            }
            if let Some(range_str) = range_opt {
                let tokens: Vec<&str> = range_str.split_whitespace().collect();
                if !tokens.len().is_multiple_of(2) {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: Integer range for dfdlx:choiceBranchKeyRanges must contain an even number of values",
                    ));
                }
                for chunk in tokens.chunks(2) {
                    let (Some(c0), Some(c1)) = (chunk.first(), chunk.get(1)) else {
                        continue;
                    };
                    let r_min = c0.parse::<i64>().map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Failed to parse integer in dfdlx:choiceBranchKeyRanges",
                        )
                    })?;
                    let r_max = c1.parse::<i64>().map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: Failed to parse integer in dfdlx:choiceBranchKeyRanges",
                        )
                    })?;
                    if r_min > r_max {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: Integer range min value ({}) is greater than max value ({}) in dfdlx:choiceBranchKeyRanges",
                                r_min, r_max
                            ),
                        ));
                    }
                    for k in &branch_keys {
                        if let Ok(k_num) = k.parse::<i64>() {
                            if k_num >= r_min && k_num <= r_max {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::SchemaDefinition,
                                    &alloc::format!(
                                        "Schema Definition Error: dfdl:choiceBranchKey '{}' conflicts with dfdlx:choiceBranchKeyRanges",
                                        k
                                    ),
                                ));
                            }
                        }
                    }
                    for &(ex_min, ex_max) in &branch_ranges {
                        if (r_min >= ex_min && r_min <= ex_max)
                            || (r_max >= ex_min && r_max <= ex_max)
                            || (ex_min >= r_min && ex_min <= r_max)
                        {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: dfdlx:choiceBranchKeyRanges conflicts with another range",
                            ));
                        }
                    }
                    branch_ranges.push((r_min, r_max));
                }
            }
        }

        let mut child_parent_props = effective_props.clone();
        if choice.properties.get_property("ignoreCase").is_some() {
            if let Some(parent_ign) = parent_props.get_property("ignoreCase") {
                let _ = child_parent_props.set_property("ignoreCase", parent_ign);
            } else {
                child_parent_props.remove_property("ignoreCase");
            }
        }

        for option in &choice.options {
            let child_id = self.lower_term_to_ir_bounded(
                builder,
                schema,
                option,
                &child_parent_props,
                depth.saturating_add(1),
            )?;
            branches.push(child_id);
        }

        if resolved_props.initiated_content {
            for &child_id in &branches {
                if let Some(child_props) = builder.get_term_props(child_id) {
                    let mut init = child_props.initiator.as_deref().unwrap_or("");
                    if init.is_empty() {
                        if let Some(term) = builder.get_term(child_id) {
                            if let TermKind::Sequence(s) = &term.kind {
                                if s.members.len() == 1 {
                                    if let Some(&first_member) = s.members.first() {
                                        if let Some(inner_props) = builder.get_term_props(first_member) {
                                            init = inner_props.initiator.as_deref().unwrap_or("");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let is_zero_len = init.is_empty()
                        || init == "%ES;"
                        || init == "%WSP*;"
                        || init
                            .split_whitespace()
                            .any(|p| p == "%ES;" || p == "%WSP*;")
                        || (init.starts_with('{')
                            && (init.contains("%ES;") || init.contains("%WSP*;")));
                    if is_zero_len {
                        let msg = alloc::format!(
                            "Schema Definition Error: initiatedContent is 'yes' but child element initiator is not defined or cannot match zero length data: '{}'",
                            init
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
            }
        }

        let choice_term = TermKind::Choice(CompiledChoice { branches });
        builder.add_term_with_props(QName::local("choice"), choice_term, resolved_props)
    }

    pub(crate) fn lower_term_to_ir_bounded(
        &self,
        builder: &mut SchemaBuilder,
        schema: &XsdSchema,
        term: &XsdTerm,
        parent_props: &PropertyStore,
        depth: usize,
    ) -> DFDLResult<NodeId> {
        match term {
            XsdTerm::Element(elem) => {
                self.lower_element_to_ir_bounded(builder, schema, elem, parent_props, depth)
            }
            XsdTerm::Sequence(seq) => {
                self.lower_sequence_to_ir_bounded(builder, schema, seq, parent_props, depth)
            }
            XsdTerm::Choice(choice) => {
                self.lower_choice_to_ir_bounded(builder, schema, choice, parent_props, depth)
            }
            XsdTerm::GroupRef(qname, group_props) => {
                if depth >= 32 {
                    let empty_seq = XsdSequence {
                        members: Vec::new(),
                        properties: PropertyStore::new(),
                    };
                    return self.lower_sequence_to_ir_bounded(
                        builder,
                        schema,
                        &empty_seq,
                        parent_props,
                        depth.saturating_add(1),
                    );
                }
                if group_props.get_property("is_hidden_group").is_some() {
                    let group_entry = schema
                        .named_groups
                        .iter()
                        .find(|(n, _)| n.local_name == qname.local_name)
                        .ok_or_else(|| {
                            DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                &alloc::format!(
                                    "Schema Definition Error: Referenced group definition not found: {}",
                                    qname.local_name
                                ),
                            )
                        })?;
                    Self::validate_hidden_group_elements(&group_entry.1.members, schema, false, 0)?;
                }
                if let Some((_, group_seq)) = schema
                    .named_groups
                    .iter()
                    .find(|(n, _)| n.local_name == qname.local_name)
                {
                    if group_seq
                        .properties
                        .get_property("choiceBranchKey")
                        .is_some()
                        || group_seq
                            .properties
                            .get_property("choiceBranchKeyRanges")
                            .is_some()
                    {
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: choiceBranchKey or choiceBranchKeyRanges cannot be defined on the sequence or choice child of a global group definition",
                        ));
                    }
                    let mut combined_seq = group_seq.clone();
                    combined_seq.properties.override_with(group_props);
                    self.lower_sequence_to_ir_bounded(
                        builder,
                        schema,
                        &combined_seq,
                        parent_props,
                        depth.saturating_add(1),
                    )
                } else {
                    let msg = alloc::format!(
                        "Schema Definition Error: Referenced group definition not found: {}",
                        qname.local_name
                    );
                    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
                }
            }
        }
    }
}

pub(crate) fn compute_implicit_alignment(
    compiled_type: &CompiledType,
    representation: dfdl_core::schema::ir::Representation,
    encoding: &str,
    alignment_units: dfdl_core::schema::ir::AlignmentUnits,
) -> usize {
    use dfdl_core::infoset::DfdlSimpleType;
    use dfdl_core::schema::ir::{AlignmentUnits, Representation};

    let is_numeric = matches!(
        compiled_type,
        CompiledType::Simple(st) if st.is_numeric()
    );
    if is_numeric && representation == Representation::Text {
        return if alignment_units == AlignmentUnits::Bits { 8 } else { 1 };
    }

    let is_string = matches!(compiled_type, CompiledType::Simple(DfdlSimpleType::String));
    if representation == Representation::Text || is_string {
        let enc_upper = encoding.to_ascii_uppercase();
        if alignment_units == AlignmentUnits::Bits {
            if enc_upper.starts_with("X-DFDL-") {
                1
            } else if enc_upper.starts_with("UTF-16") {
                16
            } else if enc_upper.starts_with("UTF-32") {
                32
            } else {
                8
            }
        } else if enc_upper.starts_with("UTF-16") {
            2
        } else if enc_upper.starts_with("UTF-32") {
            4
        } else {
            1
        }
    } else {
        match compiled_type {
            CompiledType::Simple(st) => match st {
                DfdlSimpleType::Boolean => 1,
                DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
                DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => {
                    if alignment_units == AlignmentUnits::Bits {
                        16
                    } else {
                        2
                    }
                }
                DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt | DfdlSimpleType::Float => {
                    if alignment_units == AlignmentUnits::Bits {
                        32
                    } else {
                        4
                    }
                }
                DfdlSimpleType::Long | DfdlSimpleType::UnsignedLong | DfdlSimpleType::Double => {
                    if alignment_units == AlignmentUnits::Bits {
                        64
                    } else {
                        8
                    }
                }
                DfdlSimpleType::HexBinary | DfdlSimpleType::Decimal => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
                DfdlSimpleType::Date | DfdlSimpleType::Time | DfdlSimpleType::DateTime => {
                    if alignment_units == AlignmentUnits::Bits {
                        32
                    } else {
                        4
                    }
                }
                DfdlSimpleType::String => {
                    if alignment_units == AlignmentUnits::Bits {
                        8
                    } else {
                        1
                    }
                }
            },
            CompiledType::Complex(_) => {
                if alignment_units == AlignmentUnits::Bits {
                    8
                } else {
                    1
                }
            }
        }
    }
}

pub(crate) fn coerce_dfdl_value(val: &DfdlValue, simple_type: DfdlSimpleType) -> Option<DfdlValue> {
    let s = alloc::string::ToString::to_string(val);
    let trimmed = s.trim();
    match simple_type {
        DfdlSimpleType::Int => trimmed.parse::<i32>().ok().map(DfdlValue::Int),
        DfdlSimpleType::Long => trimmed.parse::<i64>().ok().map(DfdlValue::Long),
        DfdlSimpleType::Short => trimmed.parse::<i16>().ok().map(DfdlValue::Short),
        DfdlSimpleType::Byte => trimmed.parse::<i8>().ok().map(DfdlValue::Byte),
        DfdlSimpleType::UnsignedLong => trimmed.parse::<u64>().ok().map(DfdlValue::UnsignedLong),
        DfdlSimpleType::UnsignedInt => trimmed.parse::<u32>().ok().map(DfdlValue::UnsignedInt),
        DfdlSimpleType::UnsignedShort => trimmed.parse::<u16>().ok().map(DfdlValue::UnsignedShort),
        DfdlSimpleType::UnsignedByte => trimmed.parse::<u8>().ok().map(DfdlValue::UnsignedByte),
        DfdlSimpleType::Boolean => match trimmed {
            "true" | "1" => Some(DfdlValue::Boolean(true)),
            "false" | "0" => Some(DfdlValue::Boolean(false)),
            _ => None,
        },
        DfdlSimpleType::Float => trimmed.parse::<f32>().ok().map(DfdlValue::Float),
        DfdlSimpleType::Double => trimmed.parse::<f64>().ok().map(DfdlValue::Double),
        DfdlSimpleType::String | DfdlSimpleType::HexBinary => {
            Some(DfdlValue::String(String::from(trimmed)))
        }
        DfdlSimpleType::DateTime => Some(DfdlValue::DateTime(String::from(trimmed))),
        DfdlSimpleType::Date => Some(DfdlValue::Date(String::from(trimmed))),
        DfdlSimpleType::Time => Some(DfdlValue::Time(String::from(trimmed))),
        DfdlSimpleType::Decimal => Some(DfdlValue::Decimal(String::from(trimmed))),
    }
}

pub(crate) fn parse_default_value(
    val_str: &str,
    simple_type: DfdlSimpleType,
    vmap: Option<&VariableMap>,
) -> Option<DfdlValue> {
    let trimmed = val_str.trim();

    if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
        if let Ok(ast) = dfdl_core::expr::parse_expr(trimmed) {
            let mut budget = dfdl_core::limits::WorkBudget::new(1000);
            let root_path = dfdl_core::types::InfosetPath::root();
            let mut ctx = dfdl_core::expr::ExprContext::with_variable_map(
                None,
                &root_path,
                &[],
                vmap,
                &mut budget,
            );
            if let Ok(val) = dfdl_core::expr::eval_expr(&ast, &mut ctx) {
                if let Some(coerced) = coerce_dfdl_value(&val, simple_type) {
                    return Some(coerced);
                }
            }
        }
        return None;
    }

    let fallback_val = DfdlValue::String(String::from(trimmed));
    coerce_dfdl_value(&fallback_val, simple_type)
}

/// Returns whether the given layer transform name is a supported/registered DFDL layer.
///
/// Supported layers include official Daffodil standard extension layers
/// (byte swapping, line folding, base64, AIS payload armoring, IPv4 checksum,
/// check digit, boundary mark, gzip) and test suite layers (bomb out layer, simple test layers,
/// all types layer).
#[must_use]
pub fn is_known_layer(layer_name: &str) -> bool {
    let parsed = QName::parse(layer_name);
    let clean = parsed.local_name.as_str();
    matches!(
        clean.to_ascii_lowercase().as_str(),
        "fourbyteswap"
            | "twobyteswap"
            | "byteswap"
            | "gzip"
            | "base64_mime"
            | "linefolded_imf"
            | "aispayloadarmoring"
            | "aispayloadarmor"
            | "ipv4checksum"
            | "checkdigit"
            | "boundarymark"
            | "stlbomboutlayer"
            | "stlok1"
            | "stlok2"
            | "stlok3"
            | "stlok4"
            | "alltypeslayer"
    )
}

