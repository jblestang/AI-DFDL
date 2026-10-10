//! Sequences, choices, model groups, and group references parsing.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_core::types::QName;
use dfdl_xml::{Attribute, XmlEvent, XmlReader};

use crate::annotation::{extract_dfdl_attributes, extract_dfdl_attributes_for_element};
use crate::xsd_ast::{XsdChoice, XsdSchema, XsdSequence, XsdTerm, XsdType};
use super::*;

impl SchemaCompiler {
    pub(crate) fn parse_group_definition(
        &self,
        reader: &mut XmlReader,
        xsd_prefixes: &[String],
        target_namespace: Option<&str>,
        element_form_default: bool,
    ) -> DFDLResult<XsdSequence> {
        let mut group_seq = XsdSequence {
            members: Vec::new(),
            properties: PropertyStore::new(),
        };
        let mut depth: usize = 1;
        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "group" {
                        depth = depth.saturating_add(1);
                    } else if local == "sequence" {
                        let seq = self.parse_sequence_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        if seq.properties.get_property("hiddenGroupRef").is_some() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: The model group of a group definition cannot be a sequence with dfdl:hiddenGroupRef",
                            ));
                        }
                        group_seq = seq;
                    } else if local == "choice" {
                        let choice = self.parse_choice_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        group_seq.members.push(XsdTerm::Choice(choice));
                    }
                }
                XmlEvent::EndElement { name, .. } if name.local_name == "group" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(group_seq)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_sequence_node(
        &self,
        reader: &mut XmlReader,
        attributes: &[Attribute],
        xsd_prefixes: &[String],
        target_namespace: Option<&str>,
        element_form_default: bool,
    ) -> DFDLResult<XsdSequence> {
        let mut seq_props = PropertyStore::new();
        extract_dfdl_attributes_for_element(attributes, &mut seq_props, Some("xs:sequence"))?;
        seq_props.add_namespaces(&reader.in_scope_namespace_bindings());
        let mut members = Vec::new();

        if let Some(hgr) = attributes
            .iter()
            .find(|a| a.name.local_name == "hiddenGroupRef")
            .map(|a| &a.value[..])
        {
            if hgr.trim().is_empty() {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Cannot be empty string QName for hiddenGroupRef",
                ));
            }
            let parsed = QName::parse(hgr);
            let group_ns = parsed
                .prefix
                .as_deref()
                .and_then(|p| reader.resolve_prefix(p))
                .or(target_namespace);
            let g_qname = match group_ns {
                Some(ns) => QName::with_namespace(ns, &parsed.local_name, parsed.prefix.as_deref()),
                None => QName::local(&parsed.local_name),
            };
            let mut hg_props = PropertyStore::new();
            let _ = hg_props.set_property("is_hidden_group", "true");
            members.push(XsdTerm::GroupRef(g_qname, hg_props));
            let _ = seq_props.set_property("hiddenGroupRef", &parsed.local_name);
        }

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "element" {
                        let elem = self.parse_element_node(
                            reader,
                            &name,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                            false,
                        )?;
                        members.push(XsdTerm::Element(elem));
                    } else if local == "sequence" {
                        let seq = self.parse_sequence_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        members.push(XsdTerm::Sequence(seq));
                    } else if local == "choice" {
                        let choice = self.parse_choice_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        members.push(XsdTerm::Choice(choice));
                    } else if local == "group" {
                        let group_term =
                            self.parse_group_ref_node(reader, &attributes, xsd_prefixes)?;
                        members.push(group_term);
                    } else if local == "annotation" || local == "appinfo" {
                        extract_dfdl_attributes(&attributes, &mut seq_props)?;
                        let mut dummy_defs = Vec::new();
                        let mut dummy_fmts = Vec::new();
                        let mut dummy_schemes = Vec::new();
                        self.parse_annotation_container(
                            reader,
                            local,
                            &mut seq_props,
                            &mut dummy_defs,
                            &mut dummy_fmts,
                            &mut dummy_schemes,
                            Some("xs:sequence"),
                            xsd_prefixes,
                            target_namespace,
                        )?;
                    }
                }
                XmlEvent::EndElement { name, .. } if name.local_name == "sequence" => break,
                _ => {}
            }
        }

        if seq_props.get_property("hiddenGroupRef").is_some() && members.len() > 1 {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: A sequence with hiddenGroupRef cannot have children",
            ));
        }

        if seq_props.get_property("sequenceKind") == Some("unordered") && members.is_empty() {
            return Err(DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: Unordered sequences must not be empty",
            ));
        }

        Ok(XsdSequence {
            members,
            properties: seq_props,
        })
    }

    pub(crate) fn parse_choice_node(
        &self,
        reader: &mut XmlReader,
        attributes: &[Attribute],
        xsd_prefixes: &[String],
        target_namespace: Option<&str>,
        element_form_default: bool,
    ) -> DFDLResult<XsdChoice> {
        let mut choice_props = PropertyStore::new();
        extract_dfdl_attributes_for_element(attributes, &mut choice_props, Some("xs:choice"))?;
        choice_props.add_namespaces(&reader.in_scope_namespace_bindings());
        let mut options = Vec::new();

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "element" {
                        let elem = self.parse_element_node(
                            reader,
                            &name,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                            false,
                        )?;
                        options.push(XsdTerm::Element(elem));
                    } else if local == "sequence" {
                        let seq = self.parse_sequence_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        options.push(XsdTerm::Sequence(seq));
                    } else if local == "choice" {
                        let choice = self.parse_choice_node(
                            reader,
                            &attributes,
                            xsd_prefixes,
                            target_namespace,
                            element_form_default,
                        )?;
                        options.push(XsdTerm::Choice(choice));
                    } else if local == "group" {
                        let group_term =
                            self.parse_group_ref_node(reader, &attributes, xsd_prefixes)?;
                        options.push(group_term);
                    } else if local == "annotation" || local == "appinfo" {
                        extract_dfdl_attributes(&attributes, &mut choice_props)?;
                        let mut dummy_defs = Vec::new();
                        let mut dummy_fmts = Vec::new();
                        let mut dummy_schemes = Vec::new();
                        self.parse_annotation_container(
                            reader,
                            local,
                            &mut choice_props,
                            &mut dummy_defs,
                            &mut dummy_fmts,
                            &mut dummy_schemes,
                            Some("xs:choice"),
                            xsd_prefixes,
                            target_namespace,
                        )?;
                    }
                }
                XmlEvent::EndElement { name, .. } if name.local_name == "choice" => break,
                _ => {}
            }
        }

        Self::validate_model_group_members(&options)?;

        Ok(XsdChoice {
            options,
            properties: choice_props,
        })
    }

    pub(crate) fn validate_choice_branches(options: &[XsdTerm]) -> DFDLResult<()> {
        let mut names = alloc::collections::BTreeSet::new();
        for opt in options {
            if let XsdTerm::Element(elem) = opt {
                if elem.min_occurs == 0 {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: Branch of choice '{}' must be non-optional (minOccurs must not be 0)",
                            elem.name.local_name
                        ),
                    ));
                }
                if elem.default_value.is_some() && elem.max_occurs != Some(1) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: (subset) Default value on array choice branch '{}' is not implemented (DFDL §15.1.4: An array element cannot be defaultable for a choice)",
                            elem.name.local_name
                        ),
                    ));
                }
                if !names.insert(elem.name.local_name.clone()) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!(
                            "Schema Definition Error: Unique Particle Attribution violation: element '{}' appears more than once as branch of choice",
                            elem.name.local_name
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn parse_group_ref_node(
        &self,
        reader: &mut XmlReader,
        attributes: &[Attribute],
        xsd_prefixes: &[String],
    ) -> DFDLResult<XsdTerm> {
        let mut group_props = PropertyStore::new();
        extract_dfdl_attributes_for_element(attributes, &mut group_props, Some("xs:group"))?;
        group_props.add_namespaces(&reader.in_scope_namespace_bindings());
        let ref_val = attributes
            .iter()
            .find(|a| a.name.local_name == "ref")
            .map(|a| &a.value[..])
            .ok_or_else(|| {
                DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: xs:group reference must have a 'ref' attribute",
                )
            })?;
        let parsed = QName::parse(ref_val);
        let group_ns = parsed
            .prefix
            .as_deref()
            .and_then(|p| reader.resolve_prefix(p));
        let qname = match group_ns {
            Some(ns) => QName::with_namespace(ns, &parsed.local_name, parsed.prefix.as_deref()),
            None => QName::local(&parsed.local_name),
        };

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "annotation" || local == "appinfo" {
                        extract_dfdl_attributes(&attributes, &mut group_props)?;
                        let mut dummy_defs = Vec::new();
                        let mut dummy_fmts = Vec::new();
                        let mut dummy_schemes = Vec::new();
                        self.parse_annotation_container(
                            reader,
                            local,
                            &mut group_props,
                            &mut dummy_defs,
                            &mut dummy_fmts,
                            &mut dummy_schemes,
                            Some("xs:group"),
                            xsd_prefixes,
                            None,
                        )?;
                    }
                }
                XmlEvent::EndElement { name, .. } if name.local_name == "group" => break,
                _ => {}
            }
        }

        Ok(XsdTerm::GroupRef(qname, group_props))
    }

    pub(crate) fn validate_model_group_members(members: &[XsdTerm]) -> DFDLResult<()> {
        let mut map: alloc::collections::BTreeMap<(Option<&str>, &str), &XsdType> =
            alloc::collections::BTreeMap::new();
        for term in members {
            if let XsdTerm::Element(elem) = term {
                let key = (elem.name.namespace.as_ref().map(|ns| ns.as_str()), elem.name.local_name.as_str());
                if let Some(existing_type) = map.get(&key) {
                    if existing_type != &&elem.elem_type {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            &alloc::format!(
                                "Schema Definition Error: Multiple elements with name '{}', with different types, appear in the model group",
                                elem.name.local_name
                            ),
                        ));
                    }
                } else {
                    map.insert(key, &elem.elem_type);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn can_term_produce_output_without_infoset(
        term: &XsdTerm,
        schema: &XsdSchema,
        depth: usize,
    ) -> bool {
        if depth >= 16 {
            return false;
        }
        match term {
            XsdTerm::Element(elem) => {
                if elem.min_occurs == 0 {
                    return true;
                }
                if elem.properties.get_property("outputValueCalc").is_some()
                    || elem.default_value.is_some()
                    || elem.is_nillable
                {
                    return true;
                }
                match &elem.elem_type {
                    XsdType::InlineSequence(seq) => seq.members.iter().all(|m| {
                        Self::can_term_produce_output_without_infoset(
                            m,
                            schema,
                            depth.saturating_add(1),
                        )
                    }),
                    XsdType::InlineChoice(choice) => choice.options.iter().any(|o| {
                        Self::can_term_produce_output_without_infoset(
                            o,
                            schema,
                            depth.saturating_add(1),
                        )
                    }),
                    XsdType::Complex(qname) => {
                        if let Some((_, ct)) = schema
                            .named_complex_types
                            .iter()
                            .find(|(n, _)| n.local_name == qname.local_name)
                        {
                            match ct {
                                XsdType::InlineSequence(seq) => seq.members.iter().all(|m| {
                                    Self::can_term_produce_output_without_infoset(
                                        m,
                                        schema,
                                        depth.saturating_add(1),
                                    )
                                }),
                                XsdType::InlineChoice(choice) => choice.options.iter().any(|o| {
                                    Self::can_term_produce_output_without_infoset(
                                        o,
                                        schema,
                                        depth.saturating_add(1),
                                    )
                                }),
                                _ => false,
                            }
                        } else {
                            false
                        }
                    }
                    XsdType::Simple(_) | XsdType::EmptyComplex => false,
                }
            }
            XsdTerm::Sequence(seq) => seq.members.iter().all(|m| {
                Self::can_term_produce_output_without_infoset(m, schema, depth.saturating_add(1))
            }),
            XsdTerm::Choice(choice) => choice.options.iter().any(|o| {
                Self::can_term_produce_output_without_infoset(o, schema, depth.saturating_add(1))
            }),
            XsdTerm::GroupRef(g_qname, _) => {
                if let Some((_, g_seq)) = schema
                    .named_groups
                    .iter()
                    .find(|(n, _)| n.local_name == g_qname.local_name)
                {
                    g_seq.members.iter().all(|m| {
                        Self::can_term_produce_output_without_infoset(
                            m,
                            schema,
                            depth.saturating_add(1),
                        )
                    })
                } else {
                    false
                }
            }
        }
    }

    pub(crate) fn validate_hidden_group_elements(
        members: &[XsdTerm],
        schema: &XsdSchema,
        in_choice: bool,
        depth: usize,
    ) -> DFDLResult<()> {
        if depth >= 16 {
            return Ok(());
        }
        for member in members {
            match member {
                XsdTerm::Element(elem) => {
                    match &elem.elem_type {
                        XsdType::InlineSequence(seq) => {
                            Self::validate_hidden_group_elements(
                                &seq.members,
                                schema,
                                in_choice,
                                depth.saturating_add(1),
                            )?;
                        }
                        XsdType::InlineChoice(choice) => {
                            let can_produce = choice.options.iter().any(|opt| {
                                Self::can_term_produce_output_without_infoset(
                                    opt,
                                    schema,
                                    depth.saturating_add(1),
                                )
                            });
                            if !can_produce {
                                let first_name = choice
                                    .options
                                    .first()
                                    .map(|t| match t {
                                        XsdTerm::Element(e) => e.name.local_name.clone(),
                                        _ => elem.name.local_name.clone(),
                                    })
                                    .unwrap_or_else(|| elem.name.local_name.clone());
                                let msg = alloc::format!(
                                    "Schema Definition Error: Element '{}' in hidden choice must be defaultable or define dfdl:outputValueCalc (cannot be optional without dfdl:outputValueCalc)",
                                    first_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                        XsdType::Complex(qname) => {
                            if let Some((_, ct)) = schema
                                .named_complex_types
                                .iter()
                                .find(|(n, _)| n.local_name == qname.local_name)
                            {
                                match ct {
                                    XsdType::InlineSequence(seq) => {
                                        Self::validate_hidden_group_elements(
                                            &seq.members,
                                            schema,
                                            in_choice,
                                            depth.saturating_add(1),
                                        )?;
                                    }
                                    XsdType::InlineChoice(choice) => {
                                        let can_produce = choice.options.iter().any(|opt| {
                                            Self::can_term_produce_output_without_infoset(
                                                opt,
                                                schema,
                                                depth.saturating_add(1),
                                            )
                                        });
                                        if !can_produce {
                                            let first_name = choice
                                                .options
                                                .first()
                                                .map(|t| match t {
                                                    XsdTerm::Element(e) => e.name.local_name.clone(),
                                                    _ => elem.name.local_name.clone(),
                                                })
                                                .unwrap_or_else(|| elem.name.local_name.clone());
                                            let msg = alloc::format!(
                                                "Schema Definition Error: Element '{}' in hidden choice must be defaultable or define dfdl:outputValueCalc (cannot be optional without dfdl:outputValueCalc)",
                                                first_name
                                            );
                                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        XsdType::Simple(_) | XsdType::EmptyComplex => {
                            let global_elem = schema
                                .top_level_elements
                                .iter()
                                .find(|e| e.name.local_name == elem.name.local_name);
                            let has_ovc = elem.properties.get_property("outputValueCalc").is_some()
                                || global_elem.is_some_and(|ge| ge.properties.get_property("outputValueCalc").is_some());
                            let is_defaultable = elem.default_value.is_some()
                                || elem.is_nillable
                                || global_elem.is_some_and(|ge| ge.default_value.is_some() || ge.is_nillable);
                            if elem.min_occurs > 0 && !has_ovc && !is_defaultable {
                                let group_kind = if in_choice { "hidden choice" } else { "hidden group" };
                                let msg = alloc::format!(
                                    "Schema Definition Error: Element '{}' in {} must be defaultable or define dfdl:outputValueCalc (cannot be optional without dfdl:outputValueCalc)",
                                    elem.name.local_name, group_kind
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }
                XsdTerm::Sequence(seq) => {
                    Self::validate_hidden_group_elements(
                        &seq.members,
                        schema,
                        in_choice,
                        depth.saturating_add(1),
                    )?;
                }
                XsdTerm::Choice(choice) => {
                    let can_produce = choice.options.iter().any(|opt| {
                        Self::can_term_produce_output_without_infoset(
                            opt,
                            schema,
                            depth.saturating_add(1),
                        )
                    });
                    if !can_produce {
                        let first_name = choice
                            .options
                            .first()
                            .map(|t| match t {
                                XsdTerm::Element(e) => e.name.local_name.clone(),
                                _ => String::from("choice"),
                            })
                            .unwrap_or_else(|| String::from("choice"));
                        let msg = alloc::format!(
                            "Schema Definition Error: Element '{}' in hidden choice must be defaultable or define dfdl:outputValueCalc (cannot be optional without dfdl:outputValueCalc)",
                            first_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                }
                XsdTerm::GroupRef(g_qname, _) => {
                    if let Some((_, g_seq)) = schema
                        .named_groups
                        .iter()
                        .find(|(n, _)| n.local_name == g_qname.local_name)
                    {
                        Self::validate_hidden_group_elements(
                            &g_seq.members,
                            schema,
                            in_choice,
                            depth.saturating_add(1),
                        )?;
                    }
                }
            }
        }
        Ok(())
    }


}
