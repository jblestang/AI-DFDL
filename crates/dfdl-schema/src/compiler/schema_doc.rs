//! Schema document parsing, includes, imports, and URI location resolution.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::types::QName;
use dfdl_xml::{XmlEvent, XmlReader};

use crate::annotation::{extract_dfdl_attributes, extract_dfdl_attributes_for_element};
use crate::xsd_ast::{XsdSchema, XsdSequence, XsdType};
use super::*;

impl SchemaCompiler {
    pub(crate) fn parse_schema_document_internal<F>(
        &self,
        reader: &mut XmlReader,
        resolver: &mut F,
        visited: &mut Vec<String>,
        root_xml: Option<&str>,
        enclosing_tns: Option<&str>,
        base_location: Option<&str>,
    ) -> DFDLResult<XsdSchema>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut schema = XsdSchema::default();
        let mut in_schema = false;

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "schema" {
                        in_schema = true;
                        if let Some(target_ns) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "targetNamespace")
                            .map(|a| &a.value[..])
                        {
                            schema.target_namespace =
                                Some(alloc::string::ToString::to_string(target_ns));
                        }
                        if let Some(form_def) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "elementFormDefault")
                            .map(|a| &a.value[..])
                        {
                            schema.element_form_default = form_def == "qualified";
                        }
                        let mut xsd_prefixes = reader.find_prefixes_for_uri("http://www.w3.org/2001/XMLSchema");
                        for std_p in &["xs", "xsd"] {
                            if let Some(uri) = reader.resolve_prefix(std_p) {
                                if uri == "http://www.w3.org/2001/XMLSchema"
                                    && !xsd_prefixes.iter().any(|p| p == *std_p)
                                {
                                    xsd_prefixes.push(String::from(*std_p));
                                }
                            } else if !xsd_prefixes.iter().any(|p| p == *std_p) {
                                xsd_prefixes.push(String::from(*std_p));
                            }
                        }
                        schema.xsd_prefixes = xsd_prefixes;
                        extract_dfdl_attributes_for_element(
                            &attributes,
                            &mut schema.global_format,
                            Some("xs:schema"),
                        )?;
                        schema.global_format.add_namespaces(&reader.in_scope_namespace_bindings());
                    } else if in_schema && local == "element" {
                        let elem = self.parse_element_node(
                            reader,
                            &name,
                            &attributes,
                            &schema.xsd_prefixes,
                            schema.target_namespace.as_deref(),
                            schema.element_form_default,
                            true,
                        )?;
                        let is_ref = elem.properties.get_property("__dfdl_element_ref").is_some();
                        if let Some(existing_idx) = schema
                            .top_level_elements
                            .iter()
                            .position(|e| e.name == elem.name)
                        {
                            let existing_is_ref = schema
                                .top_level_elements
                                .get(existing_idx)
                                .map(|e| e.properties.get_property("__dfdl_element_ref").is_some())
                                .unwrap_or(false);
                            if !is_ref && !existing_is_ref {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for name: {}",
                                    elem.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            if existing_is_ref && !is_ref {
                                if let Some(slot) = schema.top_level_elements.get_mut(existing_idx) {
                                    *slot = elem;
                                }
                            }
                        } else {
                            schema.top_level_elements.push(elem);
                        }
                    } else if in_schema && local == "group" {
                        if let Some(group_name) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..])
                        {
                            let g_qname = match schema.target_namespace.as_deref() {
                                Some(ns) => QName::with_namespace(ns, group_name, None),
                                None => QName::local(group_name),
                            };
                            if schema
                                .named_groups
                                .iter()
                                .any(|(n, _)| n == &g_qname)
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for name: {}",
                                    group_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let seq = self.parse_group_definition(
                                reader,
                                &schema.xsd_prefixes,
                                schema.target_namespace.as_deref(),
                                schema.element_form_default,
                            )?;
                            schema.named_groups.push((g_qname, seq));
                        }
                    } else if in_schema && local == "complexType" {
                        if let Some(ct_name) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..])
                        {
                            let parsed_ct = QName::parse(ct_name);
                            let clean_ct = parsed_ct.local_name.as_str();
                            let ct_qname = match schema.target_namespace.as_deref() {
                                Some(ns) => QName::with_namespace(ns, clean_ct, None),
                                None => QName::local(clean_ct),
                            };
                            if schema
                                .named_complex_types
                                .iter()
                                .any(|(n, _)| n == &ct_qname)
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for name: {}",
                                    ct_qname.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let mut found_model_group = false;
                            while let Some(sub_ev) = reader.next_event()? {
                                match sub_ev {
                                    XmlEvent::StartElement {
                                        name: ref child_name,
                                        ref attributes,
                                        ..
                                    } => {
                                        let child_local = child_name.local_name.as_str();
                                        if child_local == "sequence" {
                                            let seq = self.parse_sequence_node(
                                                reader,
                                                attributes,
                                                &schema.xsd_prefixes,
                                                schema.target_namespace.as_deref(),
                                                schema.element_form_default,
                                            )?;
                                            if seq
                                                .properties
                                                .get_property("hiddenGroupRef")
                                                .is_some()
                                            {
                                                return Err(DFDLError::new_static(
                                                    DFDLErrorKind::SchemaDefinition,
                                                    "Schema Definition Error: A complex type cannot have a sequence with a hiddenGroupRef as its model group",
                                                ));
                                            }
                                            schema
                                                .named_complex_types
                                                .push((ct_qname.clone(), XsdType::InlineSequence(seq)));
                                            found_model_group = true;
                                            break;
                                        } else if child_local == "choice" {
                                            let choice = self.parse_choice_node(
                                                reader,
                                                attributes,
                                                &schema.xsd_prefixes,
                                                schema.target_namespace.as_deref(),
                                                schema.element_form_default,
                                            )?;
                                            schema
                                                .named_complex_types
                                                .push((ct_qname.clone(), XsdType::InlineChoice(choice)));
                                            found_model_group = true;
                                            break;
                                        } else if child_local == "group" {
                                            let group_term = self.parse_group_ref_node(
                                                reader,
                                                attributes,
                                                &schema.xsd_prefixes,
                                            )?;
                                            let seq = XsdSequence {
                                                members: alloc::vec![group_term],
                                                properties: PropertyStore::new(),
                                            };
                                            schema
                                                .named_complex_types
                                                .push((ct_qname.clone(), XsdType::InlineSequence(seq)));
                                            found_model_group = true;
                                            break;
                                        }
                                    }
                                    XmlEvent::EndElement {
                                        name: ref end_n, ..
                                    } if end_n.local_name == "complexType" => {
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            if !found_model_group {
                                schema
                                    .named_complex_types
                                    .push((ct_qname.clone(), XsdType::EmptyComplex));
                            }
                        }
                    } else if in_schema && local == "simpleType" {
                        if let Some(st_name) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..])
                        {
                            let parsed_st = QName::parse(st_name);
                            let clean_st = parsed_st.local_name.as_str();
                            let st_qname = match schema.target_namespace.as_deref() {
                                Some(ns) => QName::with_namespace(ns, clean_st, None),
                                None => QName::local(clean_st),
                            };
                            if schema
                                .named_simple_types
                                .iter()
                                .any(|st| st.name == st_qname)
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for name: {}",
                                    st_qname.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let mut st_props = PropertyStore::new();
                            extract_dfdl_attributes_for_element(
                                &attributes,
                                &mut st_props,
                                Some("xs:simpleType"),
                            )?;
                            st_props.add_namespaces(&reader.in_scope_namespace_bindings());
                            let mut st_type = XsdType::Simple(DfdlSimpleType::String);
                            let mut seen_rep_items: Vec<(String, RepRangeOrVal)> = Vec::new();

                            let mut in_restriction = false;
                            while let Some(sub_ev) = reader.next_event()? {
                                match sub_ev {
                                    XmlEvent::StartElement {
                                        name: ref child_name,
                                        ref attributes,
                                        ..
                                    } => {
                                        let child_local = child_name.local_name.as_str();
                                        extract_dfdl_attributes_for_element(
                                            attributes,
                                            &mut st_props,
                                            Some(child_local),
                                        )?;
                                        st_props.update_namespaces(&reader.current_element_namespace_bindings());
                                        if child_local == "restriction" {
                                            in_restriction = true;
                                            if let Some(base) = attributes
                                                .iter()
                                                .find(|a| a.name.local_name == "base")
                                                .map(|a| &a.value[..])
                                            {
                                                if base.starts_with(char::is_whitespace)
                                                    || base.ends_with(char::is_whitespace)
                                                {
                                                    let msg = alloc::format!(
                                                        "Schema Definition Error: Failed to resolve base property reference for xs:restriction: '{}'",
                                                        base
                                                    );
                                                    return Err(DFDLError::new(
                                                        DFDLErrorKind::SchemaDefinition,
                                                        &msg,
                                                    ));
                                                }
                                                st_type = self.parse_xsd_type_name(
                                                    base,
                                                    &schema.xsd_prefixes,
                                                );
                                                let (pfx_opt, _local_base) = if let Some((p, l)) = base.split_once(':') {
                                                    (Some(p), l)
                                                } else {
                                                    (None, base)
                                                };
                                                let is_xsd = match pfx_opt {
                                                    Some(p) => schema.xsd_prefixes.iter().any(|xp| xp == p),
                                                    None => false,
                                                };
                                                if is_xsd && matches!(st_type, XsdType::Complex(_)) {
                                                    let msg = alloc::format!(
                                                        "Schema Definition Error: Unknown base type for xs:restriction: '{}'",
                                                        base
                                                    );
                                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                                }
                                            }
                                        } else if child_local == "minInclusive"
                                            || child_local == "maxInclusive"
                                            || child_local == "minExclusive"
                                            || child_local == "maxExclusive"
                                            || child_local == "pattern"
                                            || child_local == "enumeration"
                                            || child_local == "minLength"
                                            || child_local == "maxLength"
                                            || child_local == "totalDigits"
                                            || child_local == "fractionDigits"
                                            || child_local == "length"
                                        {
                                            if let Some(val) = attributes
                                                .iter()
                                                .find(|a| a.name.local_name == "value")
                                                .map(|a| &a.value[..])
                                            {
                                                if child_local == "enumeration" {
                                                    st_props.add_enumeration(val)?;
                                                    parse_and_validate_enumeration_rep_attributes(
                                                        attributes,
                                                        val,
                                                        &mut st_props,
                                                        &mut seen_rep_items,
                                                    )?;
                                                } else {
                                                    let prop_name = if child_local == "length" {
                                                        "xsdLength"
                                                    } else {
                                                        child_local
                                                    };
                                                    let _ = st_props.set_property(prop_name, val);
                                                }
                                            }
                                        } else if child_local == "annotation"
                                            || child_local == "appinfo"
                                            || child_local == "format"
                                        {
                                            let mut dummy_vars = Vec::new();
                                            let mut dummy_fmts = Vec::new();
                                            let mut dummy_schemes = Vec::new();
                                            let annot_comp = if in_restriction {
                                                Some("xs:restriction")
                                            } else {
                                                Some("xs:simpleType")
                                            };
                                            self.parse_annotation_container(
                                                reader,
                                                child_local,
                                                &mut st_props,
                                                &mut dummy_vars,
                                                &mut dummy_fmts,
                                                &mut dummy_schemes,
                                                annot_comp,
                                                &schema.xsd_prefixes,
                                                schema.target_namespace.as_deref(),
                                            )?;
                                        }
                                    }
                                    XmlEvent::EndElement {
                                        name: ref end_n, ..
                                    } => {
                                        if end_n.local_name == "restriction" {
                                            in_restriction = false;
                                        } else if end_n.local_name == "simpleType" {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let has_rep_type = st_props.get_property("repType").is_some()
                                || st_props.get_property("dfdlx:repType").is_some();
                            if has_rep_type && seen_rep_items.is_empty() {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::SchemaDefinition,
                                    "Schema Definition Error: A type with dfdlx:repType must define at least one enumeration.",
                                ));
                            }
                            Self::resolve_qname_properties(reader, &mut st_props);
                            let mut effective_props = st_props.clone();
                            for binding in schema.global_format.bindings() {
                                if effective_props.get_property(&binding.key).is_none() {
                                    let _ = effective_props.set_property(&binding.key, &binding.value);
                                }
                            }
                            schema.named_simple_types.push(crate::xsd_ast::XsdNamedSimpleType {
                                name: st_qname,
                                xsd_type: st_type,
                                local_props: st_props,
                                effective_props,
                            });
                        }
                    } else if in_schema && (local == "import" || local == "include") {
                        let ns_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "namespace")
                            .map(|a| &a.value[..]);
                        let location_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "schemaLocation")
                            .map(|a| &a.value[..]);
                        let target_loc = location_opt.or(ns_opt);
                        if let Some(location) = target_loc {
                            let resolved_loc = resolve_schema_location(base_location, location);
                            let eff_ns = schema
                                .target_namespace
                                .as_deref()
                                .or(enclosing_tns)
                                .unwrap_or("");
                            let visit_key = alloc::format!("{}|{}", eff_ns, resolved_loc);
                            if !visited.iter().any(|v: &String| v == &visit_key) {
                                visited.push(visit_key);
                                let imported_xml_opt = resolver(&resolved_loc).or_else(|| {
                                    if resolved_loc != location {
                                        resolver(location)
                                    } else {
                                        None
                                    }
                                });
                                if let Some(imported_xml) = imported_xml_opt {
                                    if root_xml == Some(imported_xml.as_str()) {
                                        continue;
                                    }
                                    let mut imported_reader =
                                        XmlReader::with_limits(&imported_xml, self.limits);
                                    let next_enclosing_tns = if local == "include" {
                                        schema.target_namespace.as_deref().or(enclosing_tns)
                                    } else {
                                        None
                                    };
                                    let mut imported_schema = self
                                        .parse_schema_document_internal(
                                            &mut imported_reader,
                                            resolver,
                                            visited,
                                            root_xml,
                                            next_enclosing_tns,
                                            Some(&resolved_loc),
                                        )
                                        .map_err(|err| {
                                            let loc_filename =
                                                resolved_loc.rsplit('/').next().unwrap_or(&resolved_loc);
                                            let err_str = err.message.as_str();
                                            if err_str.contains(loc_filename) {
                                                err
                                            } else {
                                                DFDLError::new(
                                                    err.kind,
                                                    &alloc::format!(
                                                        "{} at location '{}'",
                                                        err_str, resolved_loc
                                                    ),
                                                )
                                            }
                                        })?;
                                    if local == "import" {
                                        if let (Some(req_ns), Some(ref actual_ns)) =
                                            (ns_opt, &imported_schema.target_namespace)
                                        {
                                            if req_ns != actual_ns {
                                                let msg = alloc::format!(
                                                    "Schema Definition Error: Import element specifies namespace {} but namespace {} of imported schema does not match",
                                                    req_ns, actual_ns
                                                );
                                                return Err(DFDLError::new(
                                                    DFDLErrorKind::SchemaDefinition,
                                                    &msg,
                                                ));
                                            }
                                        }
                                    } else if local == "include"
                                        && schema.target_namespace.is_some()
                                        && imported_schema.target_namespace.is_none()
                                    {
                                        let tns_str = schema.target_namespace.as_deref().unwrap_or("");
                                        let tns = schema.target_namespace.as_ref().map(|u| dfdl_core::types::Namespace::new(u));
                                        let update_prop_clark = |props: &mut PropertyStore| {
                                            for prop in &["ref", "escapeSchemeRef"] {
                                                if let Some(val) = props.get_property(prop) {
                                                    if let Some(clean) = val.strip_prefix("{}") {
                                                        let new_val = alloc::format!("{{{}}}{}", tns_str, clean);
                                                        let _ = props.set_property(prop, &new_val);
                                                    } else if !val.starts_with('{') && !val.contains(':') {
                                                        let new_val = alloc::format!("{{{}}}{}", tns_str, val);
                                                        let _ = props.set_property(prop, &new_val);
                                                    }
                                                }
                                            }
                                        };
                                        fn update_term_clark<F>(term: &mut crate::xsd_ast::XsdTerm, updater: &F)
                                        where
                                            F: Fn(&mut PropertyStore),
                                        {
                                            match term {
                                                crate::xsd_ast::XsdTerm::Element(el) => {
                                                    updater(&mut el.properties);
                                                    match &mut el.elem_type {
                                                        crate::xsd_ast::XsdType::InlineSequence(seq) => {
                                                            updater(&mut seq.properties);
                                                            for m in &mut seq.members {
                                                                update_term_clark(m, updater);
                                                            }
                                                        }
                                                        crate::xsd_ast::XsdType::InlineChoice(ch) => {
                                                            updater(&mut ch.properties);
                                                            for o in &mut ch.options {
                                                                update_term_clark(o, updater);
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                crate::xsd_ast::XsdTerm::Sequence(seq) => {
                                                    updater(&mut seq.properties);
                                                    for m in &mut seq.members {
                                                        update_term_clark(m, updater);
                                                    }
                                                }
                                                crate::xsd_ast::XsdTerm::Choice(ch) => {
                                                    updater(&mut ch.properties);
                                                    for o in &mut ch.options {
                                                        update_term_clark(o, updater);
                                                    }
                                                }
                                                crate::xsd_ast::XsdTerm::GroupRef(_, props) => {
                                                    updater(props);
                                                }
                                            }
                                        }
                                        let is_targetless = |ns: &Option<dfdl_core::types::Namespace>| {
                                            ns.as_ref().map(|n| n.as_str().is_empty()).unwrap_or(true)
                                        };
                                        for (fmt_name, fmt_props) in &mut imported_schema.defined_formats {
                                            if is_targetless(&fmt_name.namespace) {
                                                fmt_name.namespace = tns.clone();
                                            }
                                            update_prop_clark(fmt_props);
                                        }
                                        for (es_name, es_props) in &mut imported_schema.defined_escape_schemes {
                                            if is_targetless(&es_name.namespace) {
                                                es_name.namespace = tns.clone();
                                            }
                                            update_prop_clark(es_props);
                                        }
                                        update_prop_clark(&mut imported_schema.global_format);
                                        for elem in &mut imported_schema.top_level_elements {
                                            if is_targetless(&elem.name.namespace) {
                                                elem.name.namespace = tns.clone();
                                            }
                                            update_prop_clark(&mut elem.properties);
                                            match &mut elem.elem_type {
                                                crate::xsd_ast::XsdType::InlineSequence(seq) => {
                                                    update_prop_clark(&mut seq.properties);
                                                    for m in &mut seq.members {
                                                        update_term_clark(m, &update_prop_clark);
                                                    }
                                                }
                                                crate::xsd_ast::XsdType::InlineChoice(ch) => {
                                                    update_prop_clark(&mut ch.properties);
                                                    for o in &mut ch.options {
                                                        update_term_clark(o, &update_prop_clark);
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        for (gname, seq) in &mut imported_schema.named_groups {
                                            if is_targetless(&gname.namespace) {
                                                gname.namespace = tns.clone();
                                            }
                                            update_prop_clark(&mut seq.properties);
                                            for m in &mut seq.members {
                                                update_term_clark(m, &update_prop_clark);
                                            }
                                        }
                                        for (ctname, cttype) in &mut imported_schema.named_complex_types {
                                            if is_targetless(&ctname.namespace) {
                                                ctname.namespace = tns.clone();
                                            }
                                            match cttype {
                                                crate::xsd_ast::XsdType::InlineSequence(seq) => {
                                                    update_prop_clark(&mut seq.properties);
                                                    for m in &mut seq.members {
                                                        update_term_clark(m, &update_prop_clark);
                                                    }
                                                }
                                                crate::xsd_ast::XsdType::InlineChoice(ch) => {
                                                    update_prop_clark(&mut ch.properties);
                                                    for o in &mut ch.options {
                                                        update_term_clark(o, &update_prop_clark);
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        for st in &mut imported_schema.named_simple_types {
                                            if is_targetless(&st.name.namespace) {
                                                st.name.namespace = tns.clone();
                                            }
                                            update_prop_clark(&mut st.local_props);
                                            update_prop_clark(&mut st.effective_props);
                                        }
                                        for var in &mut imported_schema.defined_variables {
                                            if is_targetless(&var.name.namespace) {
                                                var.name.namespace = tns.clone();
                                            }
                                        }
                                        imported_schema.target_namespace = schema.target_namespace.clone();
                                    }
                                    schema.merge(imported_schema)?;
                                } else if local == "include" {
                                    // DFDL / XML Schema requires that included schemas must be resolved.
                                    // Failing to resolve an included schema is a fatal Schema Definition Error.
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Failed to include schema at location '{}'",
                                        resolved_loc
                                    );
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::SchemaDefinition,
                                        &msg,
                                    ));
                                }
                            }
                        }
                    } else if in_schema && local == "defineVariable" {
                        let name_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..]);
                        let type_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "type")
                            .map(|a| &a.value[..])
                            .unwrap_or("xs:string");
                        let default_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "defaultValue")
                            .map(|a| &a.value[..]);
                        let body_text = self.read_annotation_inner_text(reader, "defineVariable")?;
                        let default_opt = Self::resolve_variable_value(
                            default_attr,
                            &body_text,
                            "Schema Definition Error: Default value of variable was supplied both as attribute and element value",
                        )?;
                        if let Some(ref def_str) = default_opt {
                            dfdl_core::expr::validate_expression_namespaces(def_str, &reader.in_scope_namespace_bindings())?;
                        }
                        let dir_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "direction")
                            .map(|a| &a.value[..]);
                        let direction = match dir_attr {
                            Some("parseOnly") => {
                                dfdl_core::expr::variables::VariableDirection::ParseOnly
                            }
                            Some("unparseOnly") => {
                                dfdl_core::expr::variables::VariableDirection::UnparseOnly
                            }
                            Some("both") => dfdl_core::expr::variables::VariableDirection::Both,
                            Some(other) => {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Invalid variable direction '{}'",
                                    other
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            None => dfdl_core::expr::variables::VariableDirection::Both,
                        };
                        if let Some(vname) = name_opt {
                            let var_type =
                                match self.parse_xsd_type_name(type_opt, &schema.xsd_prefixes) {
                                    XsdType::Simple(st) => st,
                                    _ => DfdlSimpleType::String,
                                };
                            let clean_vname =
                                vname.trim_start_matches("tns:").trim_start_matches("ex:");
                            schema
                                .defined_variables
                                .push(crate::xsd_ast::DfdlVariableDef {
                                    name: QName::local(clean_vname),
                                    var_type,
                                    default_value: default_opt,
                                    direction,
                                });
                        }
                    } else if in_schema && local == "setVariable" {
                        let ref_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "ref" || a.name.local_name == "name")
                            .map(|a| &a.value[..]);
                        let val_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "value")
                            .map(|a| &a.value[..]);
                        if let (Some(r), Some(v)) = (ref_opt, val_opt) {
                            schema.global_format.add_set_variable(r, v);
                        }
                    } else if in_schema && local == "format" {
                        for a in &attributes {
                            if a.name.prefix.as_deref() == Some("dfdl") {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Attribute 'dfdl:{}' is not allowed on DFDL annotation element '<dfdl:format>'",
                                    a.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                        extract_dfdl_attributes(&attributes, &mut schema.global_format)?;
                        schema.global_format.add_namespaces(&reader.in_scope_namespace_bindings());
                        Self::resolve_qname_properties(reader, &mut schema.global_format);
                    } else if in_schema && local == "defineFormat" {
                        let mut disallowed = alloc::vec::Vec::new();
                        for a in &attributes {
                            if a.name.prefix.as_deref() == Some("dfdl") {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Attribute 'dfdl:{}' is not allowed on DFDL annotation element '<dfdl:defineFormat>'",
                                    a.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let pfx = a.name.prefix.as_deref().unwrap_or("");
                            let loc = a.name.local_name.as_str();
                            if pfx != "xmlns" && loc != "xmlns" && loc != "name" && loc != "ref" {
                                disallowed.push(loc);
                            }
                        }
                        if !disallowed.is_empty() {
                            let msg = alloc::format!(
                                "Schema Definition Error: The attribute(s) {} are not allowed on 'defineFormat'. Format properties must be defined on a 'format' child element.",
                                disallowed.join(", ")
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        let name_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..]);
                        let mut fmt_props = PropertyStore::new();
                        extract_dfdl_attributes(&attributes, &mut fmt_props)?;
                        fmt_props.add_namespaces(&reader.in_scope_namespace_bindings());
                        let mut depth: usize = 1;
                        while let Some(sub_ev) = reader.next_event()? {
                            match sub_ev {
                                XmlEvent::StartElement {
                                    name: ref sub_n,
                                    attributes: ref sub_attrs,
                                    ..
                                } => {
                                    if sub_n.local_name == "defineFormat" {
                                        depth = depth.saturating_add(1);
                                    } else if sub_n.local_name == "format" {
                                        extract_dfdl_attributes(sub_attrs, &mut fmt_props)?;
                                        fmt_props.update_namespaces(&reader.current_element_namespace_bindings());
                                        Self::resolve_qname_properties(reader, &mut fmt_props);
                                    } else if sub_n.local_name == "property" {
                                        let name_opt = sub_attrs
                                            .iter()
                                            .find(|a| a.name.local_name == "name")
                                            .map(|a| &a.value[..]);
                                        let val_opt = sub_attrs
                                            .iter()
                                            .find(|a| a.name.local_name == "value")
                                            .map(|a| &a.value[..]);
                                        let inner_text =
                                            self.read_annotation_inner_text(reader, "property")?;
                                        let final_val = val_opt.unwrap_or(inner_text.as_str());
                                        if let Some(prop_name) = name_opt {
                                            let _ = fmt_props.set_property(prop_name, final_val);
                                        }
                                    }
                                }
                                XmlEvent::EndElement {
                                    name: ref end_n, ..
                                } if end_n.local_name == "defineFormat" => {
                                    depth = depth.saturating_sub(1);
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Self::resolve_qname_properties(reader, &mut fmt_props);
                        if let Some(fmt_name) = name_opt {
                            let parsed_fname = QName::parse(fmt_name);
                            let clean_fname = parsed_fname.local_name.as_str();
                            let fmt_qname = if let Some(ref tns) = schema.target_namespace {
                                QName::with_namespace(tns, clean_fname, None)
                            } else {
                                QName::local(clean_fname)
                            };
                            if schema
                                .defined_formats
                                .iter()
                                .any(|(n, _)| n == &fmt_qname)
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for {}",
                                    clean_fname
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            schema
                                .defined_formats
                                .push((fmt_qname, fmt_props));
                        }
                    } else if in_schema && local == "defineEscapeScheme" {
                        Self::parse_define_escape_scheme_element(
                            reader,
                            &attributes,
                            &mut schema.defined_escape_schemes,
                            schema.target_namespace.as_deref(),
                        )?;
                    } else if in_schema && local == "escapeScheme" {
                        let mut dummy_store = PropertyStore::new();
                        extract_dfdl_attributes(&attributes, &mut dummy_store)?;
                        let mut depth: usize = 1;
                        while let Some(sub_ev) = reader.next_event()? {
                            match sub_ev {
                                XmlEvent::StartElement {
                                    name: ref sub_n,
                                    attributes: sub_attrs,
                                    ..
                                } => {
                                    if sub_n.local_name == local {
                                        depth = depth.saturating_add(1);
                                    }
                                    extract_dfdl_attributes(&sub_attrs, &mut dummy_store)?;
                                }
                                XmlEvent::EndElement {
                                    name: ref end_n, ..
                                } if end_n.local_name == local => {
                                    depth = depth.saturating_sub(1);
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        dummy_store.validate_property_entities()?;
                        dummy_store.to_resolved_properties(None)?;
                    } else if in_schema && (local == "annotation" || local == "appinfo") {
                        extract_dfdl_attributes(&attributes, &mut schema.global_format)?;
                        self.parse_annotation_container(
                            reader,
                            local,
                            &mut schema.global_format,
                            &mut schema.defined_variables,
                            &mut schema.defined_formats,
                            &mut schema.defined_escape_schemes,
                            Some("xs:schema"),
                            &schema.xsd_prefixes,
                            schema.target_namespace.as_deref(),
                        )?;
                    }
                }
                XmlEvent::EndElement { name, .. } if name.local_name == "schema" => break,
                _ => {}
            }
        }

        Ok(schema)
    }

}

/// Resolves a target schema location relative to an optional enclosing base schema location.
///
/// Complies with W3C XML Schema 1.0 Part 1 §4.2.3 and RFC 3986 §5.2 for relative URI
/// resolution. If `target_loc` is already an absolute path (begins with `/`) or includes
/// a scheme (such as `http://` or `urn:`), it is returned verbatim. Otherwise, if an
/// enclosing `base_loc` with directory components is present, the relative path segments
/// (`.` and `..`) are normalized against the enclosing directory path.
#[must_use]
pub fn resolve_schema_location(base_loc: Option<&str>, target_loc: &str) -> String {
    if target_loc.starts_with('/') || target_loc.contains("://") || target_loc.starts_with("urn:") {
        return alloc::string::String::from(target_loc);
    }
    let Some(base) = base_loc else {
        return alloc::string::String::from(target_loc);
    };
    let is_absolute = base.starts_with('/');
    let Some(slash_idx) = base.rfind('/') else {
        return alloc::string::String::from(target_loc);
    };
    let dir = &base[..slash_idx];
    let mut parts: Vec<&str> = Vec::new();
    for seg in dir.split('/') {
        if !seg.is_empty() && seg != "." {
            parts.push(seg);
        }
    }
    for seg in target_loc.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        } else if seg == ".." {
            let _ = parts.pop();
        } else {
            parts.push(seg);
        }
    }
    let joined = parts.join("/");
    if is_absolute {
        alloc::format!("/{}", joined)
    } else {
        joined
    }
}
