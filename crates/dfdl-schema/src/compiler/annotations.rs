//! DFDL annotations, format definitions, and escape schemes.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_core::types::QName;
use dfdl_xml::{Attribute, XmlEvent, XmlReader};

use crate::annotation::extract_dfdl_attributes;
use crate::xsd_ast::XsdType;
use super::*;

impl SchemaCompiler {
    pub(crate) fn validate_term_assertions(props: &PropertyStore) -> DFDLResult<()> {
        if let Some(err) = props.assert_errors().first() {
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, err));
        }
        if props.has_asserts()
            && (props.discriminator_count > 0 || props.get_property("discriminator").is_some())
        {
            let msg = "Schema Definition Error: A component cannot have both a discriminator and an assert statement.";
            return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
        }
        if props.discriminator_count > 1 {
            let msg = "Schema Definition Error: A component cannot have more than one discriminator statement.";
            return Err(DFDLError::new_static(DFDLErrorKind::SchemaDefinition, msg));
        }
        for assert in props.asserts() {
            if assert.test_kind == dfdl_core::schema::ir::TestKind::Pattern
                && assert.test_expr.trim().is_empty()
            {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: The attribute testPattern must not be empty for testKind='pattern'",
                ));
            }
        }
        if let Some(disc) = props.get_property("discriminator") {
            if props.get_property("discriminatorTestKind") == Some("pattern")
                && disc.trim().is_empty()
            {
                return Err(DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: The attribute testPattern must not be empty for testKind='pattern'",
                ));
            }
        }
        for (nvi_qname, def_val) in props.new_variable_instances() {
            if def_val.is_some()
                && props
                    .set_variables()
                    .iter()
                    .any(|(sv_qname, _)| sv_qname.local_name == nvi_qname.local_name)
            {
                let msg = alloc::format!(
                    "Schema Definition Error: In the unparse direction, a default value cannot be used on newVariableInstance in combination with setVariable as it creates a race condition with forward referencing expression for variable '{}'",
                    nvi_qname.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        Ok(())
    }

    pub(crate) fn read_annotation_inner_text(
        &self,
        reader: &mut XmlReader,
        target_tag: &str,
    ) -> DFDLResult<String> {
        let mut inner_text = String::new();
        while let Some(sub_ev) = reader.next_event()? {
            match sub_ev {
                XmlEvent::Text { content, .. } => {
                    if content.contains('\n') || content.contains('\r') {
                        inner_text.push_str(content.trim());
                    } else {
                        inner_text.push_str(content.as_ref());
                    }
                }
                XmlEvent::CData { content, .. } => {
                    inner_text.push_str(content);
                }
                XmlEvent::EndElement {
                    name: ref end_n, ..
                } if end_n.local_name == target_tag
                    || (target_tag == "defineVariable"
                        && end_n.local_name == "newVariableInstance")
                    || (target_tag == "newVariableInstance"
                        && end_n.local_name == "defineVariable") =>
                {
                    break;
                }
                other => {
                    reader.push_back(other);
                    break;
                }
            }
        }
        Ok(inner_text)
    }

    /// Resolves a variable default/value that may be written as an attribute
    /// or as element content, but never both (DFDL §7.2, §7.7). Returns the
    /// attribute value, else the non-blank body, else `None`; `both_msg` is the
    /// Schema Definition Error raised when both are supplied.
    pub(crate) fn resolve_variable_value(
        attr: Option<&str>,
        body: &str,
        both_msg: &'static str,
    ) -> DFDLResult<Option<String>> {
        let body_given = !body.trim().is_empty();
        match (attr, body_given) {
            (Some(_), true) => Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                both_msg,
            )),
            (Some(a), false) => Ok(Some(String::from(a))),
            (None, true) => Ok(Some(String::from(body))),
            (None, false) => Ok(None),
        }
    }

    pub(crate) fn resolve_qname_properties(reader: &XmlReader, store: &mut PropertyStore) {
        for prop in &["escapeSchemeRef", "ref"] {
            if let Some(val) = store.get_property(prop) {
                if val.trim().is_empty() {
                    continue;
                }
                if !val.starts_with('{') {
                    if let Some((prefix, local)) = val.split_once(':') {
                        let uri_opt = reader
                            .resolve_prefix(prefix)
                            .or_else(|| {
                                store
                                    .in_scope_namespaces()
                                    .iter()
                                    .find(|(p, _)| p == prefix)
                                    .map(|(_, u)| u.as_str())
                            });
                        if let Some(uri) = uri_opt {
                            let clark = alloc::format!("{{{}}}{}", uri, local);
                            let _ = store.set_property(prop, &clark);
                        }
                    } else {
                        let uri = reader
                            .resolve_prefix("")
                            .or_else(|| reader.resolve_default_ns())
                            .or_else(|| {
                                store
                                    .in_scope_namespaces()
                                    .iter()
                                    .find(|(p, _)| p.is_empty())
                                    .map(|(_, u)| u.as_str())
                            })
                            .unwrap_or("");
                        let clark = alloc::format!("{{{}}}{}", uri, val);
                        let _ = store.set_property(prop, &clark);
                    }
                }
            }
        }
    }

    pub(crate) fn parse_define_escape_scheme_element(
        reader: &mut XmlReader,
        attributes: &[Attribute],
        defined_escape_schemes: &mut Vec<(QName, PropertyStore)>,
        target_namespace: Option<&str>,
    ) -> DFDLResult<()> {
        let name_attr = attributes
            .iter()
            .find(|a| a.name.local_name == "name" || a.name.local_name == "NCName")
            .map(|a| a.value.trim());
        let es_name = match name_attr {
            Some(n) if !n.is_empty() => {
                if n.contains(':') {
                    let msg = alloc::format!(
                        "Schema Definition Error: defineEscapeScheme name '{}' must be an NCName and cannot contain a colon",
                        n
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                n
            }
            _ => {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: defineEscapeScheme requires a name attribute",
                ));
            }
        };

        let mut scheme_store = PropertyStore::new();
        let mut has_child_scheme = false;
        let mut depth: usize = 1;
        while let Some(sub_ev) = reader.next_event()? {
            match sub_ev {
                XmlEvent::StartElement {
                    name: ref sub_n,
                    attributes: ref sub_attrs,
                    ..
                } => {
                    if sub_n.local_name == "defineEscapeScheme" {
                        depth = depth.saturating_add(1);
                    } else if sub_n.local_name == "escapeScheme" {
                        has_child_scheme = true;
                        extract_dfdl_attributes(sub_attrs, &mut scheme_store)?;
                    }
                }
                XmlEvent::EndElement {
                    name: ref end_n, ..
                } if end_n.local_name == "defineEscapeScheme" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }

        if !has_child_scheme {
            return Err(DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: The content of element 'dfdl:defineEscapeScheme' is not complete. Expected 'dfdl:escapeScheme'",
            ));
        }

        if let Err(err) = scheme_store.validate_property_entities() {
            let msg = alloc::format!("{}", err);
            scheme_store.add_assert_error(&msg);
        } else {
            let _ = scheme_store.to_resolved_properties(None);
        }

        let parsed_es = QName::parse(es_name);
        let clean_es_name = parsed_es.local_name.as_str();
        if defined_escape_schemes
            .iter()
            .any(|(n, _)| n.local_name == clean_es_name && n.namespace.as_ref().map(|ns| ns.as_str()) == target_namespace)
        {
            let msg = alloc::format!(
                "Schema Definition Error: More than one definition for escapeScheme {}",
                clean_es_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
        let scheme_qname = match target_namespace {
            Some(ns) => QName::with_namespace(ns, clean_es_name, None),
            None => QName::local(clean_es_name),
        };
        defined_escape_schemes.push((scheme_qname, scheme_store));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_annotation_container(
        &self,
        reader: &mut XmlReader,
        container_tag: &str,
        store: &mut PropertyStore,
        defined_vars: &mut Vec<crate::xsd_ast::DfdlVariableDef>,
        defined_formats: &mut Vec<(QName, PropertyStore)>,
        defined_escape_schemes: &mut Vec<(QName, PropertyStore)>,
        component_tag: Option<&str>,
        xsd_prefixes: &[String],
        target_namespace: Option<&str>,
    ) -> DFDLResult<()> {
        let mut enclosing_annotation: Option<String> = None;
        // Variable names already bound by setVariable / newVariableInstance in this annotation.
        let mut set_refs: Vec<String> = Vec::new();
        let mut nvi_refs: Vec<String> = Vec::new();
        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    name, attributes, ..
                } => {
                    store.update_namespaces(&reader.current_element_namespace_bindings());
                    let prefix = name.prefix.as_deref().unwrap_or("");
                    let local = name.local_name.as_str();
                    if (prefix == "xs" || prefix == "xsd")
                        && local != "appinfo"
                        && local != "annotation"
                        && local != "documentation"
                    {
                        let qtag = alloc::format!("{}:{}", prefix, local);
                        let msg = alloc::format!(
                            "Schema Definition Error: Invalid dfdl annotation: {}",
                            qtag
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    if local == "appinfo" {
                        if let Some(src) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "source")
                            .map(|a| &a.value[..])
                        {
                            if src.contains("dfdl") && src != "http://www.ogf.org/dfdl/" {
                                let _ = store.set_property("__dfdl_appinfo_source_warning", src);
                            }
                        }
                    }
                    if component_tag == Some("xs:element")
                        && (local == "sequence" || local == "choice")
                    {
                        let msg = alloc::format!(
                            "Schema Definition Error: DFDL annotation type dfdl:{} expected dfdl:element for badElementFormProperty2",
                            local
                        );
                        store.add_assert_error(&msg);
                    }
                    if local == "element" || local == "sequence" || local == "choice" || local == "format" || local == "simpleType" {
                        enclosing_annotation = Some(String::from(local));
                        for a in &attributes {
                            if a.name.prefix.as_deref() == Some("dfdl") {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Attribute 'dfdl:{}' is not allowed on annotation element '<dfdl:{}>'",
                                    a.name.local_name, local
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            let is_component_with_short_form = matches!(
                                component_tag,
                                Some("xs:element")
                                    | Some("xs:sequence")
                                    | Some("xs:choice")
                                    | Some("xs:group")
                                    | Some("xs:simpleType")
                            );
                            if is_component_with_short_form {
                                let attr_local = a.name.local_name.as_str();
                                if attr_local != "ref"
                                    && a.name.prefix.as_deref() != Some("xmlns")
                                    && attr_local != "xmlns"
                                    && store.get_property(attr_local).is_some()
                                {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Property '{}' is defined in both short and long form on component '{}'",
                                        attr_local, component_tag.unwrap_or("")
                                    );
                                    store.add_assert_error(&msg);
                                }
                            }
                        }
                    }
                    let is_element_or_group = local == "element" || local == "sequence" || local == "choice" || local == "group";
                    if is_element_or_group {
                        let has_long_ref = attributes.iter().any(|a| {
                            (a.name.prefix.as_deref() == Some("dfdl") || a.name.prefix.as_deref().unwrap_or("").is_empty())
                                && a.name.local_name == "ref"
                        });
                        if has_long_ref && store.get_property("ref").is_some() {
                            store.add_assert_error(
                                "Schema Definition Error: Both long form and short form ref attribute found",
                            );
                        }
                    }
                    if local != "appinfo"
                        && local != "annotation"
                        && local != "documentation"
                        && local != "setVariable"
                        && local != "newVariableInstance"
                        && local != "defineVariable"
                    {
                        extract_dfdl_attributes(&attributes, store)?;
                        Self::resolve_qname_properties(reader, store);
                    }
                    if local == "setVariable" {
                        let ref_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "ref" || a.name.local_name == "name")
                            .map(|a| &a.value[..]);
                        let val_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "value")
                            .map(|a| &a.value[..]);
                        let inner_text = self.read_annotation_inner_text(reader, local)?;
                        let final_val = match Self::resolve_variable_value(
                            val_opt,
                            &inner_text,
                            "Schema Definition Error: Cannot have both a value attribute and an element value",
                        ) {
                            Ok(v) => v.unwrap_or_default(),
                            Err(e) => {
                                store.add_assert_error(&alloc::format!("{}", e));
                                String::new()
                            }
                        };
                        if let Some(r) = ref_opt {
                            let parsed_r = QName::parse(r);
                            let key = parsed_r.local_name.as_str();
                            if set_refs.iter().any(|s| s == key) {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Variables set by setVariable must be distinct at the same location: {}",
                                    key
                                );
                                store.add_assert_error(&msg);
                            }
                            set_refs.push(String::from(key));
                            store.add_set_variable(r, &final_val);
                        }
                    } else if local == "assert" {
                        if store.get_property("discriminator").is_some() {
                            let msg = "Schema Definition Error: A component cannot have both a discriminator and an assert statement.";
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                msg,
                            ));
                        }
                        for a in &attributes {
                            let p = a.name.prefix.as_deref().unwrap_or("");
                            let l = a.name.local_name.as_str();
                            if p == "xmlns" || l == "xmlns" {
                                continue;
                            }
                            let is_allowed = matches!(l, "test" | "testKind" | "testPattern" | "message" | "failureType");
                            if p.is_empty() && is_allowed {
                                continue;
                            }
                            let full_attr_name = if p.is_empty() {
                                alloc::string::ToString::to_string(l)
                            } else {
                                alloc::format!("{}:{}", p, l)
                            };
                            let msg = alloc::format!(
                                "Schema Definition Error: Attribute '{}' is not allowed to appear in element 'dfdl:{}'",
                                full_attr_name, local
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        let test_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "test")
                            .map(|a| a.value.trim());
                        let test_pattern_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "testPattern")
                            .map(|a| a.value.trim());
                        let inner_text = self.read_annotation_inner_text(reader, local)?;
                        let body_text = inner_text.trim();

                        if test_attr.is_some() && test_pattern_attr.is_some() {
                            let msg = "Schema Definition Error: You may not specify both test and testPattern attributes";
                            store.add_assert_error(msg);
                        }
                        if test_attr.is_some() && !body_text.is_empty() {
                            let msg = "Schema Definition Error: You may not specify both test attribute and a body expression";
                            store.add_assert_error(msg);
                        }
                        if test_pattern_attr.is_some() && !body_text.is_empty() {
                            let msg = "Schema Definition Error: You may not specify both testPattern attribute and a body expression";
                            store.add_assert_error(msg);
                        }

                        let final_test = test_attr.or(test_pattern_attr).unwrap_or(body_text);

                        let test_kind_str = attributes
                            .iter()
                            .find(|a| a.name.local_name == "testKind")
                            .map(|a| &a.value[..]);
                        let test_kind = match test_kind_str {
                            Some("pattern") => dfdl_core::schema::ir::TestKind::Pattern,
                            Some("expression") => dfdl_core::schema::ir::TestKind::Expression,
                            _ => {
                                if test_pattern_attr.is_some() {
                                    dfdl_core::schema::ir::TestKind::Pattern
                                } else {
                                    dfdl_core::schema::ir::TestKind::Expression
                                }
                            }
                        };
                        let msg_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "message")
                            .map(|a| &a.value[..]);
                        let failure_type = match attributes
                            .iter()
                            .find(|a| a.name.local_name == "failureType")
                            .map(|a| a.value.trim())
                        {
                            Some("recoverableError") => {
                                dfdl_core::schema::ir::FailureType::RecoverableError
                            }
                            _ => dfdl_core::schema::ir::FailureType::ProcessingError,
                        };
                        store.add_assert_with_failure_type(test_kind, final_test, msg_opt, failure_type);
                    } else if local == "discriminator" {
                        store.discriminator_count = store.discriminator_count.saturating_add(1);
                        for a in &attributes {
                            let p = a.name.prefix.as_deref().unwrap_or("");
                            let l = a.name.local_name.as_str();
                            if p == "xmlns" || l == "xmlns" {
                                continue;
                            }
                            let is_allowed = matches!(l, "test" | "testKind" | "testPattern" | "message");
                            if p.is_empty() && is_allowed {
                                continue;
                            }
                            let full_attr_name = if p.is_empty() {
                                alloc::string::ToString::to_string(l)
                            } else {
                                alloc::format!("{}:{}", p, l)
                            };
                            let msg = alloc::format!(
                                "Schema Definition Error: Attribute '{}' is not allowed to appear in element 'dfdl:{}'",
                                full_attr_name, local
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        let test_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "test")
                            .map(|a| a.value.trim());
                        let test_pattern_attr = attributes
                            .iter()
                            .find(|a| a.name.local_name == "testPattern")
                            .map(|a| a.value.trim());
                        let inner_text = self.read_annotation_inner_text(reader, local)?;
                        let body_text = inner_text.trim();

                        if test_attr.is_some() && test_pattern_attr.is_some() {
                            let msg = "Schema Definition Error: You may not specify both test and testPattern attributes";
                            store.add_assert_error(msg);
                        }
                        if test_attr.is_some() && !body_text.is_empty() {
                            let msg = "Schema Definition Error: You may not specify both test attribute and a body expression";
                            store.add_assert_error(msg);
                        }
                        if test_pattern_attr.is_some() && !body_text.is_empty() {
                            let msg = "Schema Definition Error: You may not specify both testPattern attribute and a body expression";
                            store.add_assert_error(msg);
                        }

                        let final_test = test_attr.or(test_pattern_attr).unwrap_or(body_text);

                        let test_kind_str = attributes
                            .iter()
                            .find(|a| a.name.local_name == "testKind")
                            .map(|a| &a.value[..])
                            .unwrap_or_else(|| {
                                if test_pattern_attr.is_some() {
                                    "pattern"
                                } else {
                                    "expression"
                                }
                            });
                        let msg_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "message")
                            .map(|a| &a.value[..]);
                        store.set_property("discriminator", final_test)?;
                        store.set_property("discriminatorTestKind", test_kind_str)?;
                        if let Some(m) = msg_opt {
                            store.set_property("discriminatorMessage", m)?;
                        }
                    } else if local == "property" {
                        if enclosing_annotation.is_none() {
                            let msg = alloc::format!(
                                "Schema Definition Error: The dfdl:property annotation element is not allowed directly under xs:appinfo in '{}'",
                                component_tag.unwrap_or("unknown")
                            );
                            store.add_assert_error(&msg);
                        }
                        let name_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name")
                            .map(|a| &a.value[..]);
                        let val_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "value")
                            .map(|a| &a.value[..]);
                        let inner_text = self.read_annotation_inner_text(reader, local)?;
                        let final_val = val_opt.unwrap_or(inner_text.as_str());
                        if let Some(prop_name) = name_opt {
                            if prop_name == "ref" {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::SchemaDefinition,
                                    "Schema Definition Error: 'ref' is not a valid value for dfdl:property name. The ref property may only be specified as an attribute or in short form (DFDL-7-016R).",
                                ));
                            }
                            let is_component_with_short_form = matches!(
                                component_tag,
                                Some("xs:element")
                                    | Some("xs:sequence")
                                    | Some("xs:choice")
                                    | Some("xs:group")
                                    | Some("xs:simpleType")
                            );
                            if is_component_with_short_form
                                && store.get_property(prop_name).is_some()
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: Property '{}' is defined in multiple forms on component '{}'",
                                    prop_name, component_tag.unwrap_or("")
                                );
                                store.add_assert_error(&msg);
                            }
                            store.set_property(prop_name, final_val)?;
                            Self::resolve_qname_properties(reader, store);
                        }
                    } else if local == "defineVariable" || local == "newVariableInstance" {
                        let name_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "name" || a.name.local_name == "ref")
                            .map(|a| &a.value[..]);
                        let type_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "type")
                            .map(|a| &a.value[..])
                            .unwrap_or("xs:string");
                        let default_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "defaultValue")
                            .map(|a| alloc::string::ToString::to_string(&a.value));
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
                        let inner_text = self.read_annotation_inner_text(reader, local)?;
                        let final_default = match Self::resolve_variable_value(
                            default_opt.as_deref(),
                            &inner_text,
                            "Schema Definition Error: Default value of variable was supplied both as attribute and element value",
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                store.add_assert_error(&alloc::format!("{}", e));
                                None
                            }
                        };
                        if local == "newVariableInstance" {
                            if component_tag == Some("xs:element") {
                                store.add_assert_error(
                                    "Schema Definition Error: newVariableInstance may only be used on group reference, sequence or choice",
                                );
                            }
                            if let Some(n) = name_opt {
                                let parsed_n = QName::parse(n);
                                let key = parsed_n.local_name.as_str();
                                if nvi_refs.iter().any(|s| s == key) {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: newVariableInstances must all be distinct within the same scope: {}",
                                        key
                                    );
                                    store.add_assert_error(&msg);
                                }
                                nvi_refs.push(String::from(key));
                            }
                        }
                        if let Some(var_name) = name_opt {
                            let parsed_vname = QName::parse(var_name);
                            let clean_vname = parsed_vname.local_name.as_str();
                            let st = self.parse_xsd_type_name(type_opt, xsd_prefixes);
                            let has_explicit_type =
                                attributes.iter().any(|a| a.name.local_name == "type");
                            if let XsdType::Simple(var_type) = st {
                                if let Some(existing) = defined_vars
                                    .iter_mut()
                                    .find(|v| v.name.local_name == clean_vname)
                                {
                                    if local == "defineVariable" {
                                        if has_explicit_type {
                                            existing.var_type = var_type;
                                        }
                                        if final_default.is_some() {
                                            existing.default_value = final_default.clone();
                                        }
                                        existing.direction = direction;
                                    }
                                } else {
                                    defined_vars.push(crate::xsd_ast::DfdlVariableDef {
                                        name: QName::local(clean_vname),
                                        var_type,
                                        default_value: final_default.clone(),
                                        direction,
                                    });
                                }
                            }
                            if local == "newVariableInstance" {
                                store.add_new_variable_instance(clean_vname, final_default.as_deref());
                            }
                        }
                    } else if local == "defineFormat" {
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
                            let fmt_qname = if let Some(tns) = target_namespace {
                                QName::with_namespace(tns, clean_fname, None)
                            } else {
                                QName::local(clean_fname)
                            };
                            if defined_formats
                                .iter()
                                .any(|(n, _)| n == &fmt_qname)
                            {
                                let msg = alloc::format!(
                                    "Schema Definition Error: More than one definition for {}",
                                    clean_fname
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            defined_formats.push((fmt_qname, fmt_props));
                        }
                    } else if local == "defineEscapeScheme" {
                        Self::parse_define_escape_scheme_element(
                            reader,
                            &attributes,
                            defined_escape_schemes,
                            target_namespace,
                        )?;
                    } else if local == "appinfo" {
                        let source_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "source")
                            .map(|a| &a.value[..]);
                        let is_valid_dfdl_source = match source_opt {
                            Some(src) => {
                                if src.contains("dfdl")
                                    && src != "http://www.ogf.org/dfdl/"
                                    && src != "http://www.ogf.org/dfdl/dfdl-1.0/"
                                    && src != "http://www.dfdl.org/7793"
                                {
                                    let _ = store.set_property("__dfdl_appinfo_source_warning", src);
                                }
                                src == "http://www.ogf.org/dfdl/"
                                    || src == "http://www.ogf.org/dfdl/dfdl-1.0/"
                                    || src == "http://www.dfdl.org/7793"
                                    || src.starts_with("http://www.ogf.org/dfdl")
                            }
                            None => {
                                // DFDL §3.3: xs:appinfo without source attribute is not interpreted as DFDL annotations.
                                let _ = store.set_property("__dfdl_appinfo_missing_source_warning", "true");
                                false
                            }
                        };
                        if is_valid_dfdl_source {
                            self.parse_annotation_container(
                                reader,
                                local,
                                store,
                                defined_vars,
                                defined_formats,
                                defined_escape_schemes,
                                component_tag,
                                xsd_prefixes,
                                target_namespace,
                            )?;
                        } else {
                            // Non-DFDL appinfo: skip its entire subtree until matching </xs:appinfo>
                            let mut depth: usize = 1;
                            while let Some(ev) = reader.next_event()? {
                                match ev {
                                    XmlEvent::StartElement { .. } => {
                                        depth = depth.saturating_add(1);
                                    }
                                    XmlEvent::EndElement { name: end_name, .. } if depth > 0 => {
                                        depth = depth.saturating_sub(1);
                                        if depth == 0 && end_name.local_name == local {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    } else if local == "annotation" {
                        self.parse_annotation_container(
                            reader,
                            local,
                            store,
                            defined_vars,
                            defined_formats,
                            defined_escape_schemes,
                            component_tag,
                            xsd_prefixes,
                            target_namespace,
                        )?;
                    }
                }
                XmlEvent::EndElement { name, .. } => {
                    if let Some(ref enc) = enclosing_annotation {
                        if enc == &name.local_name {
                            enclosing_annotation = None;
                        }
                    }
                    if name.local_name == container_tag {
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }


    pub(crate) fn resolve_ref_formats(
        &self,
        store: &mut PropertyStore,
        defined_formats: &[(QName, PropertyStore)],
    ) -> DFDLResult<()> {
        let mut visited = Vec::new();
        self.resolve_ref_formats_bounded(store, defined_formats, &mut visited)
    }

    pub(crate) fn resolve_ref_formats_bounded(
        &self,
        store: &mut PropertyStore,
        defined_formats: &[(QName, PropertyStore)],
        visited_refs: &mut Vec<String>,
    ) -> DFDLResult<()> {
        let mut current_ref = store.get_property("ref").map(String::from);
        while let Some(ref_name) = current_ref {
            let (target_ns_or_prefix, clean_ref) = if let Some(stripped) = ref_name.strip_prefix('{') {
                if let Some((ns, local)) = stripped.split_once('}') {
                    (Some(ns), local)
                } else {
                    (None, ref_name.as_str())
                }
            } else if let Some((p, l)) = ref_name.split_once(':') {
                (Some(p), l)
            } else {
                (None, ref_name.as_str())
            };
            if visited_refs.iter().any(|r| r == &ref_name) {
                break;
            }
            visited_refs.push(ref_name.clone());
            let fmt_opt = if let Some(p) = target_ns_or_prefix {
                if p.is_empty() {
                    defined_formats.iter().find(|(n, _)| {
                        n.local_name == clean_ref
                            && (n.namespace.is_none() || n.namespace.as_ref().map(|ns| ns.as_str()) == Some(""))
                    })
                } else {
                    let resolved_uri = store
                        .in_scope_namespaces()
                        .iter()
                        .find(|(pfx, _)| pfx == p)
                        .map(|(_, uri)| uri.as_str());
                    defined_formats.iter().find(|(n, _)| {
                        n.local_name == clean_ref
                            && (n.prefix.as_deref() == Some(p)
                                || n.namespace.as_ref().map(|ns| ns.as_str() == p).unwrap_or(false)
                                || (resolved_uri.is_some() && n.namespace.as_ref().map(|ns| ns.as_str()) == resolved_uri))
                    })
                }
            } else {
                let default_ns = store
                    .in_scope_namespaces()
                    .iter()
                    .find(|(pfx, _)| pfx.is_empty())
                    .map(|(_, uri)| uri.as_str());
                defined_formats.iter().find(|(n, _)| {
                    n.local_name == clean_ref
                        && match default_ns {
                            Some(def_uri) => n.namespace.as_ref().map(|ns| ns.as_str()) == Some(def_uri),
                            None => n.namespace.is_none() || n.namespace.as_ref().map(|ns| ns.as_str()) == Some(""),
                        }
                })
            };
            if let Some((_, fmt_store)) = fmt_opt {
                let mut resolved_fmt = fmt_store.clone();
                self.resolve_ref_formats_bounded(&mut resolved_fmt, defined_formats, visited_refs)?;
                store.extend(&resolved_fmt);
                current_ref = fmt_store.get_property("ref").map(String::from);
            } else {
                let ns_display = if let Some(p) = target_ns_or_prefix {
                    if let Some((_, uri)) = store.in_scope_namespaces().iter().find(|(pfx, _)| pfx == p) {
                        uri.as_str()
                    } else {
                        p
                    }
                } else {
                    ""
                };
                let msg = alloc::format!(
                    "Schema Definition Error: defineFormat {{{}}}{} not found",
                    ns_display, clean_ref
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        Ok(())
    }

    /// DFDL §13.2.1: the element's `terminator` and the enclosing sequence's
    /// `separator` may not begin with the scheme's `escapeCharacter` (only for
    /// `escapeKind="escapeCharacter"`) or `escapeEscapeCharacter`. Expressions
    /// and entity-based escape characters can only be checked at runtime and
    /// are skipped here.
    pub(crate) fn validate_escape_delimiter_conflict(
        elem_props: &PropertyStore,
        parent_props: &PropertyStore,
    ) -> DFDLResult<()> {
        if elem_props.get_property("escapeSchemeRef").is_none_or(|s| s.trim().is_empty() || s == "{}") {
            return Ok(());
        }
        let kind = elem_props.get_property("escapeKind").unwrap_or("escapeCharacter");
        let mut escapes: Vec<&str> = Vec::new();
        if kind == "escapeCharacter" {
            if let Some(c) = elem_props.get_property("escapeCharacter") {
                escapes.push(c);
            }
        }
        if let Some(c) = elem_props.get_property("escapeEscapeCharacter") {
            escapes.push(c);
        }
        escapes.retain(|c| !c.is_empty() && !c.starts_with('{') && !c.starts_with('%'));
        let delimiters = [
            elem_props.get_property("terminator"),
            parent_props.get_property("separator"),
        ];
        for delim in delimiters.into_iter().flatten() {
            if delim.starts_with('{') {
                continue;
            }
            for alt in delim.split_whitespace() {
                if escapes.iter().any(|e| alt.starts_with(e)) {
                    return Err(DFDLError::new_static(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: dfdl:terminator and dfdl:separator may not begin with the dfdl:escapeCharacter or dfdl:escapeEscapeCharacter: the escape character cannot be the same as the terminating markup",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_escape_scheme_ref(
        &self,
        store: &mut PropertyStore,
        defined_escape_schemes: &[(QName, PropertyStore)],
    ) -> DFDLResult<()> {
        if let Some(es_ref) = store.get_property("escapeSchemeRef").map(String::from) {
            let trimmed = es_ref.trim();
            if trimmed.is_empty() || trimmed == "{}" {
                return Ok(());
            }
            let (target_ns_or_prefix, clean_ref) = if let Some(stripped) = trimmed.strip_prefix('{') {
                if let Some((ns, local)) = stripped.split_once('}') {
                    (Some(ns), local)
                } else {
                    (None, trimmed)
                }
            } else if let Some((p, l)) = trimmed.split_once(':') {
                (Some(p), l)
            } else {
                (None, trimmed)
            };

            if clean_ref.trim().is_empty() {
                return Ok(());
            }

            let es_opt = if let Some(p) = target_ns_or_prefix {
                defined_escape_schemes.iter().find(|(n, _)| {
                    n.local_name == clean_ref
                        && (n.namespace.as_ref().map(|ns| ns.as_str() == p).unwrap_or(false)
                            || n.prefix.as_deref() == Some(p)
                            || store.in_scope_namespaces().iter().any(|(prefix, uri)| {
                                (prefix == p || uri == p)
                                    && n.namespace.as_ref().map(|ns| ns.as_str() == uri.as_str()).unwrap_or(false)
                            }))
                }).or_else(|| {
                    defined_escape_schemes.iter().find(|(n, _)| n.local_name == clean_ref)
                })
            } else {
                defined_escape_schemes.iter().find(|(n, _)| n.local_name == clean_ref)
            };

            if let Some((_, es_store)) = es_opt {
                if let Some(err) = es_store.assert_errors().first() {
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, err));
                }
                store.extend(es_store);
            } else {
                let msg = alloc::format!(
                    "Schema Definition Error: Failed to resolve escapeSchemeRef '{}'",
                    es_ref
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        Ok(())
    }


}
