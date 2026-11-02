//! Sequence, Choice, and Group lowering to CompiledSchema IR terms.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::{PropertyKey, PropertyScope, PropertyStore};
use dfdl_core::schema::builder::SchemaBuilder;
use dfdl_core::schema::ir::{
    CompiledChoice, CompiledSequence, NodeId, ResolvedProperties, TermKind,
};
use dfdl_core::types::QName;

use crate::compiler::lower::helpers::is_known_layer;
use crate::compiler::SchemaCompiler;
use crate::xsd_ast::{XsdChoice, XsdElement, XsdSchema, XsdSequence, XsdTerm, XsdType};

impl SchemaCompiler {
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
        let doc_fmt = seq
            .properties
            .get_property("__dfdl_target_ns")
            .or_else(|| parent_props.get_property("__dfdl_target_ns"))
            .and_then(|ns| {
                schema
                    .doc_formats
                    .iter()
                    .find(|(n, _)| n.as_deref().unwrap_or("") == ns)
                    .map(|(_, f)| f)
            });
        if let Some(fmt) = doc_fmt {
            effective_props.extend(fmt);
        }
        effective_props.merge(parent_props);
        if seq.properties.get_property("ignoreCase").is_none() {
            if let Some(parent_ign) = parent_props.get_property("ignoreCase") {
                let _ = effective_props.set_property("ignoreCase", parent_ign);
            }
        }
        if seq.properties.get_property("encodingErrorPolicy").is_none() {
            if let Some(parent_eep) = parent_props.get_property("encodingErrorPolicy") {
                let _ = effective_props.set_property("encodingErrorPolicy", parent_eep);
            }
        }
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
            if seq.properties.get_property("separator").is_some()
                || seq.properties.get_property("dfdl:separator").is_some()
                || effective_props.get_property("separator").is_some_and(|s| !s.is_empty())
                || effective_props.get_property("dfdl:separator").is_some_and(|s| !s.is_empty())
            {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: A sequence with a layer cannot specify dfdl:separator",
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

        let resolved_props = effective_props
            .to_resolved_properties_with_limits(Some(parent_props), &self.resource_limits())?;

        // DFDL §16.1.1 & §16.1.2: The restriction that occursCountKind='implicit' with unbounded
        // maxOccurs must be the last element applies to positional sequences (ordered sequences
        let has_separator = resolved_props.separator.as_ref().is_some_and(|s| !s.is_empty());
        let seq_scope = PropertyScope::root(&effective_props);
        if resolved_props.sequence_kind != dfdl_core::schema::ir::SequenceKind::Unordered {
            for (i, member) in seq.members.iter().enumerate() {
                if let XsdTerm::Element(elem) = member {
                    if elem.max_occurs.is_none() {
                        let elem_scope = seq_scope.child(&elem.properties);
                        let ock = elem_scope
                            .get_property_key(PropertyKey::OccursCountKind)
                            .unwrap_or("implicit");
                        if ock == "implicit" {
                            let ssp = seq_scope
                                .get_property_key(PropertyKey::SeparatorSuppressionPolicy)
                                .unwrap_or("trailingEmpty");
                            let is_positional = !has_separator
                                || matches!(ssp, "never" | "trailingEmpty" | "trailingEmptyStrict");
                            let cannot_precede =
                                is_positional && !elem_has_discriminator(elem, schema);
                            if cannot_precede {
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
                                    let msg = alloc::format!(
                                        "Schema Definition Error: occursCountKind='implicit' with unbounded maxOccurs only allowed for last element of a positional sequence or when terminated by a discriminator: '{}'",
                                        elem.name.local_name
                                    );
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::SchemaDefinition,
                                        &msg,
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
        let doc_fmt = choice
            .properties
            .get_property("__dfdl_target_ns")
            .or_else(|| parent_props.get_property("__dfdl_target_ns"))
            .and_then(|ns| {
                schema
                    .doc_formats
                    .iter()
                    .find(|(n, _)| n.as_deref().unwrap_or("") == ns)
                    .map(|(_, f)| f)
            });
        if let Some(fmt) = doc_fmt {
            effective_props.extend(fmt);
        }
        effective_props.merge(parent_props);
        if choice.properties.get_property("ignoreCase").is_none() {
            if let Some(parent_ign) = parent_props.get_property("ignoreCase") {
                let _ = effective_props.set_property("ignoreCase", parent_ign);
            }
        }
        if choice.properties.get_property("encodingErrorPolicy").is_none() {
            if let Some(parent_eep) = parent_props.get_property("encodingErrorPolicy") {
                let _ = effective_props.set_property("encodingErrorPolicy", parent_eep);
            }
        }
        let mut resolved_global_format = schema.global_format.clone();
        self.resolve_ref_formats(&mut resolved_global_format, &schema.defined_formats)?;
        effective_props.extend(&resolved_global_format);
        Self::validate_term_assertions(&effective_props)?;
        let resolved_props = effective_props
            .to_resolved_properties_with_limits(Some(parent_props), &self.resource_limits())?;
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

fn elem_has_discriminator(elem: &XsdElement, schema: &XsdSchema) -> bool {
    if elem.properties.discriminator_count > 0
        || elem.properties.get_property("discriminator").is_some()
        || elem.properties.get_property("dfdl:discriminator").is_some()
    {
        return true;
    }
    if elem.properties.get_property("__dfdl_element_ref").is_some() {
        if let Some(target) = schema.top_level_elements.iter().find(|g| {
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
        }).or_else(|| schema.top_level_elements.iter().find(|g| g.name.local_name == elem.name.local_name)) {
            if elem_has_discriminator(target, schema) {
                return true;
            }
        }
    }
    match &elem.elem_type {
        XsdType::InlineSequence(seq) => seq_has_discriminator(seq, schema),
        XsdType::InlineChoice(choice) => choice_has_discriminator(choice, schema),
        XsdType::Complex(type_qname) => {
            if let Some((_, ct)) = schema.named_complex_types.iter().find(|(q, _)| q.local_name == type_qname.local_name) {
                match ct {
                    XsdType::InlineSequence(seq) => seq_has_discriminator(seq, schema),
                    XsdType::InlineChoice(choice) => choice_has_discriminator(choice, schema),
                    _ => false,
                }
            } else {
                false
            }
        }
        _ => false,
    }
}

fn seq_has_discriminator(seq: &XsdSequence, schema: &XsdSchema) -> bool {
    if seq.properties.discriminator_count > 0
        || seq.properties.get_property("discriminator").is_some()
        || seq.properties.get_property("dfdl:discriminator").is_some()
    {
        return true;
    }
    seq.members.iter().any(|m| term_has_discriminator(m, schema))
}

fn choice_has_discriminator(choice: &XsdChoice, schema: &XsdSchema) -> bool {
    if choice.properties.discriminator_count > 0
        || choice.properties.get_property("discriminator").is_some()
        || choice.properties.get_property("dfdl:discriminator").is_some()
    {
        return true;
    }
    choice.options.iter().any(|o| term_has_discriminator(o, schema))
}

fn term_has_discriminator(term: &XsdTerm, schema: &XsdSchema) -> bool {
    match term {
        XsdTerm::Element(e) => elem_has_discriminator(e, schema),
        XsdTerm::Sequence(s) => seq_has_discriminator(s, schema),
        XsdTerm::Choice(c) => choice_has_discriminator(c, schema),
        XsdTerm::GroupRef(qname, props) => {
            if props.discriminator_count > 0
                || props.get_property("discriminator").is_some()
                || props.get_property("dfdl:discriminator").is_some()
            {
                return true;
            }
            if let Some((_, grp)) = schema.named_groups.iter().find(|(q, _)| q.local_name == qname.local_name) {
                seq_has_discriminator(grp, schema)
            } else {
                false
            }
        }
    }
}

