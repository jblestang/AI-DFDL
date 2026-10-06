//! Main Schema Compiler implementation transforming XSD XML into CompiledSchema IR.
//!
//! Conforms strictly to DFDL 1.0 Specification §4, §5, §6, §7. Panic-free `#![no_std]` + `alloc`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::variables::VariableMap;
use dfdl_core::expr::PropertyStore;
use dfdl_core::infoset::value::{DfdlSimpleType, DfdlValue};
use dfdl_core::schema::builder::SchemaBuilder;
use dfdl_core::schema::ir::{
    CompiledChoice, CompiledElement, CompiledSchema, CompiledSequence, CompiledType, NodeId,
    ResolvedProperties, TermKind,
};
use dfdl_core::types::{QName, UnqualifiedPathStepPolicy};
use dfdl_xml::limits::XmlReaderLimits;
use dfdl_xml::{Attribute, XmlEvent, XmlReader};

use crate::annotation::{extract_dfdl_attributes, extract_dfdl_attributes_for_element};
use crate::xsd_ast::{XsdChoice, XsdElement, XsdSchema, XsdSequence, XsdTerm, XsdType};

/// Policy for handling invalid facet restrictions on simple types (`daf:invalidRestrictionPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum InvalidRestrictionPolicy {
    /// Allow invalid restrictions and validate them against the infoset.
    #[default]
    Validate,
    /// Treat invalid facet restrictions as schema definition errors.
    Error,
    /// Ignore invalid facet restrictions without applying them during validation.
    Ignore,
}

/// Validates facet applicability and values against an XSD simple type (DFDL 1.0 & XSD Part 2 §4.3).
fn validate_simple_type_facets(
    st: DfdlSimpleType,
    props: &PropertyStore,
    elem_name: &str,
    invalid_restriction_policy: InvalidRestrictionPolicy,
) -> DFDLResult<()> {
    if st != DfdlSimpleType::String
        && props.get_property("pattern").is_some()
        && invalid_restriction_policy == InvalidRestrictionPolicy::Error
    {
        let msg = alloc::format!(
            "Schema Definition Error: Pattern restriction is only allowed on types derived from string on element '{}'",
            elem_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    // 1. Facet applicability:
    // minLength, maxLength are ONLY applicable to xs:string and xs:hexBinary.
    // XSD Part 2 §4.3.2, §4.3.3. (Note: 'length' is a DFDL format property).
    if !matches!(st, DfdlSimpleType::String | DfdlSimpleType::HexBinary) {
        if props.get_property("minLength").is_some() {
            let msg = alloc::format!(
                "Schema Definition Error: Facet 'minLength' is not valid for type '{:?}' on element '{}'",
                st, elem_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
        if props.get_property("maxLength").is_some() {
            let msg = alloc::format!(
                "Schema Definition Error: Facet 'maxLength' is not valid for type '{:?}' on element '{}'",
                st, elem_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
    }

    // 2. fractionDigits is ONLY applicable to xs:decimal.
    if st != DfdlSimpleType::Decimal && props.get_property("fractionDigits").is_some() {
        let msg = alloc::format!(
            "Schema Definition Error: Facet 'fractionDigits' is not valid for type '{:?}' on element '{}'",
            st, elem_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }

    // 3. totalDigits is ONLY applicable to xs:decimal and integer derived types.
    if !matches!(
        st,
        DfdlSimpleType::Decimal
            | DfdlSimpleType::Int
            | DfdlSimpleType::Long
            | DfdlSimpleType::Short
            | DfdlSimpleType::Byte
            | DfdlSimpleType::UnsignedInt
            | DfdlSimpleType::UnsignedLong
            | DfdlSimpleType::UnsignedShort
            | DfdlSimpleType::UnsignedByte
    ) && props.get_property("totalDigits").is_some()
    {
        let msg = alloc::format!(
            "Schema Definition Error: Facet 'totalDigits' is not valid for type '{:?}' on element '{}'",
            st, elem_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }

    // 4. totalDigits value must be positive integer (> 0).
    if let Some(td_str) = props.get_property("totalDigits") {
        match td_str.parse::<u32>() {
            Ok(0) => {
                let msg = alloc::format!(
                    "Schema Definition Error: totalDigits facet must be a positive integer greater than 0 on element '{}'",
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            Err(_) => {
                let msg = alloc::format!(
                    "Schema Definition Error: totalDigits facet value '{}' is not a valid positive integer on element '{}'",
                    td_str, elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            Ok(td) => {
                if let Some(fd_str) = props.get_property("fractionDigits") {
                    if let Ok(fd) = fd_str.parse::<u32>() {
                        if fd > td {
                            let msg = alloc::format!(
                                "Schema Definition Error: fractionDigits ({}) cannot exceed totalDigits ({}) on element '{}'",
                                fd, td, elem_name
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
        }
    }

    // 5. minLength and maxLength consistency:
    if let (Some(min_l_str), Some(max_l_str)) = (
        props.get_property("minLength"),
        props.get_property("maxLength"),
    ) {
        if let (Ok(min_l), Ok(max_l)) = (min_l_str.parse::<usize>(), max_l_str.parse::<usize>()) {
            if min_l > max_l {
                let msg = alloc::format!(
                    "Schema Definition Error: minLength ({}) cannot be greater than maxLength ({}) on element '{}'",
                    min_l, max_l, elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
    }

    // 6. Range facets cannot have value "NaN".
    for facet_name in &[
        "minInclusive",
        "maxInclusive",
        "minExclusive",
        "maxExclusive",
    ] {
        if let Some(val) = props.get_property(facet_name) {
            let val_clean = val.trim();
            if val_clean.eq_ignore_ascii_case("nan") {
                let msg = alloc::format!(
                    "Schema Definition Error: Facet '{}' cannot be NaN on element '{}'",
                    facet_name,
                    elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
    }

    // 7. Numeric range facet bounds for integer types must fit into that integer type.
    for facet_name in &[
        "minInclusive",
        "maxInclusive",
        "minExclusive",
        "maxExclusive",
    ] {
        if let Some(val_str) = props.get_property(facet_name) {
            let val_clean = val_str.trim();
            let fits = match st {
                DfdlSimpleType::Byte => val_clean.parse::<i8>().is_ok(),
                DfdlSimpleType::Short => val_clean.parse::<i16>().is_ok(),
                DfdlSimpleType::Int => val_clean.parse::<i32>().is_ok(),
                DfdlSimpleType::Long => val_clean.parse::<i64>().is_ok(),
                DfdlSimpleType::UnsignedByte => val_clean.parse::<u8>().is_ok(),
                DfdlSimpleType::UnsignedShort => val_clean.parse::<u16>().is_ok(),
                DfdlSimpleType::UnsignedInt => val_clean.parse::<u32>().is_ok(),
                DfdlSimpleType::UnsignedLong => val_clean.parse::<u64>().is_ok(),
                _ => true,
            };
            if !fits {
                let msg = alloc::format!(
                    "Schema Definition Error: Facet '{}' value '{}' does not fit in type '{:?}' on element '{}'",
                    facet_name, val_str, st, elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
    }

    // 8. Inclusivity / Exclusivity conflict checks:
    if props.get_property("minInclusive").is_some() && props.get_property("minExclusive").is_some()
    {
        let msg = alloc::format!(
            "Schema Definition Error: minInclusive and minExclusive cannot both be specified on element '{}'",
            elem_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }
    if props.get_property("maxInclusive").is_some() && props.get_property("maxExclusive").is_some()
    {
        let msg = alloc::format!(
            "Schema Definition Error: maxInclusive and maxExclusive cannot both be specified on element '{}'",
            elem_name
        );
        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
    }

    Ok(())
}

/// Schema Compiler transforming XML Schema (XSD) documents containing DFDL annotations into [`CompiledSchema`].
#[derive(Debug, Default)]
pub struct SchemaCompiler {
    limits: XmlReaderLimits,
    /// Inverse of the `allowSignedIntegerLength1Bit` tunable (default `false` = allowed).
    disallow_signed_integer_length_1bit: bool,
    /// `maxOccursBounds` tunable (`None` = unlimited).
    max_occurs_bounds: Option<usize>,
    /// `requireEncodingErrorPolicyProperty` tunable: text elements must define the property.
    require_encoding_error_policy: bool,
    /// `requireTextBidiProperty` tunable: text elements must define `dfdl:textBidi`.
    require_text_bidi: bool,
    /// `requireFloatingProperty` tunable: text elements must define `dfdl:floating`.
    require_floating: bool,
    /// Whether to escalate schema definition warnings to errors (`daf:escalateWarningsToErrors`).
    pub escalate_warnings: bool,
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: UnqualifiedPathStepPolicy,
    /// Maximum allowed byte length for xs:hexBinary values.
    pub max_hex_binary_length_in_bytes: Option<usize>,
    /// Policy for handling invalid facet restrictions (`daf:invalidRestrictionPolicy`).
    pub invalid_restriction_policy: InvalidRestrictionPolicy,
}

impl SchemaCompiler {
    /// Constructs a new [`SchemaCompiler`] with default limits.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: XmlReaderLimits::default(),
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            require_encoding_error_policy: false,
            require_text_bidi: false,
            require_floating: false,
            escalate_warnings: false,
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
            invalid_restriction_policy: InvalidRestrictionPolicy::Validate,
        }
    }

    /// Constructs a [`SchemaCompiler`] with custom XML reader limits.
    #[inline]
    #[must_use]
    pub const fn with_limits(limits: XmlReaderLimits) -> Self {
        Self {
            limits,
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            require_encoding_error_policy: false,
            require_text_bidi: false,
            require_floating: false,
            escalate_warnings: false,
            unqualified_path_step_policy: UnqualifiedPathStepPolicy::NoNamespace,
            max_hex_binary_length_in_bytes: None,
            invalid_restriction_policy: InvalidRestrictionPolicy::Validate,
        }
    }

    /// Sets whether to escalate warnings to errors (`daf:escalateWarningsToErrors`).
    #[inline]
    #[must_use]
    pub const fn with_escalate_warnings(mut self, escalate: bool) -> Self {
        self.escalate_warnings = escalate;
        self
    }

    /// Sets the policy for resolving unqualified path steps in DFDL expressions (§23).
    #[inline]
    #[must_use]
    pub const fn with_unqualified_path_step_policy(mut self, policy: UnqualifiedPathStepPolicy) -> Self {
        self.unqualified_path_step_policy = policy;
        self
    }

    /// Sets the `maxHexBinaryLengthInBytes` tunable: maximum allowed byte length for xs:hexBinary values.
    #[inline]
    #[must_use]
    pub const fn with_max_hex_binary_length_in_bytes(mut self, limit: Option<usize>) -> Self {
        self.max_hex_binary_length_in_bytes = limit;
        self
    }

    /// Sets the `invalidRestrictionPolicy` tunable.
    #[inline]
    #[must_use]
    pub const fn with_invalid_restriction_policy(mut self, policy: InvalidRestrictionPolicy) -> Self {
        self.invalid_restriction_policy = policy;
        self
    }

    /// Sets the `allowSignedIntegerLength1Bit` tunable. When `allow` is `false`, signed binary
    /// integers of length 1 bit are rejected with a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_allow_signed_integer_length_1bit(mut self, allow: bool) -> Self {
        self.disallow_signed_integer_length_1bit = !allow;
        self
    }

    /// Sets the `maxOccursBounds` tunable: parsing more array occurrences than `bound` is an error.
    #[inline]
    #[must_use]
    pub const fn with_max_occurs_bounds(mut self, bound: Option<usize>) -> Self {
        self.max_occurs_bounds = bound;
        self
    }

    /// Sets the `requireEncodingErrorPolicyProperty` tunable: a text simple element whose
    /// `dfdl:encodingErrorPolicy` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_encoding_error_policy(mut self, require: bool) -> Self {
        self.require_encoding_error_policy = require;
        self
    }

    /// Sets the `requireTextBidiProperty` tunable: a text simple element whose
    /// `dfdl:textBidi` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_text_bidi(mut self, require: bool) -> Self {
        self.require_text_bidi = require;
        self
    }

    /// Sets the `requireFloatingProperty` tunable: a text simple element whose
    /// `dfdl:floating` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_floating(mut self, require: bool) -> Self {
        self.require_floating = require;
        self
    }

    /// Compiles an XSD schema document XML string into a [`CompiledSchema`].
    pub fn compile_str(&self, schema_xml: &str) -> DFDLResult<CompiledSchema> {
        self.compile_str_with_resolver(schema_xml, |_| None)
    }

    /// Compiles an XSD schema document with a custom import/include schema resolver callback.
    pub fn compile_str_with_resolver<F>(
        &self,
        schema_xml: &str,
        resolver: F,
    ) -> DFDLResult<CompiledSchema>
    where
        F: FnMut(&str) -> Option<String>,
    {
        self.compile_str_with_resolver_and_root(schema_xml, resolver, None)
    }

    /// Compiles an XSD schema document with a custom resolver and specific target root element name.
    pub fn compile_str_with_resolver_and_root<F>(
        &self,
        schema_xml: &str,
        mut resolver: F,
        target_root: Option<&str>,
    ) -> DFDLResult<CompiledSchema>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut reader = XmlReader::with_limits(schema_xml, self.limits);
        reader.set_permissive_namespaces(true);
        let mut visited = Vec::new();
        let xsd_schema = self.parse_schema_document_internal(
            &mut reader,
            &mut resolver,
            &mut visited,
            Some(schema_xml),
            None,
            None,
        )?;
        if xsd_schema.top_level_elements.is_empty() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "No top-level element found in XSD schema (parse)",
            ));
        }
        self.lower_schema_to_ir_with_root(&xsd_schema, target_root)
    }

    fn parse_schema_document_internal<F>(
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
                            let clean_ct = ct_name.split(':').next_back().unwrap_or(ct_name);
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
                            let clean_st = st_name.split(':').next_back().unwrap_or(st_name);
                            let st_qname = match schema.target_namespace.as_deref() {
                                Some(ns) => QName::with_namespace(ns, clean_st, None),
                                None => QName::local(clean_st),
                            };
                            if schema
                                .named_simple_types
                                .iter()
                                .any(|(n, _, _)| n == &st_qname)
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
                            schema
                                .named_simple_types
                                .push((st_qname, st_type, st_props));
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
                                        for (fmt_name, fmt_props) in &mut imported_schema.defined_formats {
                                            fmt_name.namespace = tns.clone();
                                            update_prop_clark(fmt_props);
                                        }
                                        for (es_name, es_props) in &mut imported_schema.defined_escape_schemes {
                                            es_name.namespace = tns.clone();
                                            update_prop_clark(es_props);
                                        }
                                        update_prop_clark(&mut imported_schema.global_format);
                                        for elem in &mut imported_schema.top_level_elements {
                                            elem.name.namespace = tns.clone();
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
                                            gname.namespace = tns.clone();
                                            update_prop_clark(&mut seq.properties);
                                            for m in &mut seq.members {
                                                update_term_clark(m, &update_prop_clark);
                                            }
                                        }
                                        for (ctname, cttype) in &mut imported_schema.named_complex_types {
                                            ctname.namespace = tns.clone();
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
                                        for (stname, _, st_props) in &mut imported_schema.named_simple_types {
                                            stname.namespace = tns.clone();
                                            update_prop_clark(st_props);
                                        }
                                        for var in &mut imported_schema.defined_variables {
                                            var.name.namespace = tns.clone();
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
                            let clean_fname = fmt_name.split(':').next_back().unwrap_or(fmt_name);
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

    fn parse_group_definition(
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
    fn parse_element_node(
        &self,
        reader: &mut XmlReader,
        elem_qname: &QName,
        attributes: &[Attribute],
        xsd_prefixes: &[String],
        target_namespace: Option<&str>,
        element_form_default: bool,
        is_top_level: bool,
    ) -> DFDLResult<XsdElement> {
        let mut name_opt = None;
        let mut ref_opt = None;
        let mut type_opt = None;
        let mut form_opt = None;
        let mut min_occurs = 1;
        let mut max_occurs = Some(1);
        let mut nillable = false;
        let mut default_val = None;
        let mut local_props = PropertyStore::new();

        for attr in attributes {
            if attr.name.prefix.is_some() {
                continue;
            }
            match attr.name.local_name.as_str() {
                "name" => name_opt = Some(alloc::string::ToString::to_string(&attr.value)),
                "ref" => ref_opt = Some(alloc::string::ToString::to_string(&attr.value)),
                "type" => type_opt = Some(alloc::string::ToString::to_string(&attr.value)),
                "form" => form_opt = Some(alloc::string::ToString::to_string(&attr.value)),
                "minOccurs" => {
                    min_occurs = attr.value.parse::<usize>().map_err(|_| {
                        DFDLError::new_static(
                            DFDLErrorKind::SchemaDefinition,
                            "Invalid minOccurs attribute integer",
                        )
                    })?;
                }
                "maxOccurs" => {
                    if attr.value == "unbounded" {
                        max_occurs = None;
                    } else {
                        max_occurs = Some(attr.value.parse::<usize>().map_err(|_| {
                            DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Invalid maxOccurs attribute integer",
                            )
                        })?);
                    }
                }
                "nillable" => nillable = attr.value == "true",
                "default" => default_val = Some(alloc::string::ToString::to_string(&attr.value)),
                _ => {}
            }
        }

        extract_dfdl_attributes_for_element(attributes, &mut local_props, Some("xs:element"))?;
        local_props.add_namespaces(&reader.in_scope_namespace_bindings());

        if let Some(ref r) = ref_opt {
            let _ = local_props.set_property("__dfdl_element_ref", r);
        }

        if ref_opt.is_some() && type_opt.is_some() {
            return Err(DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: The 'name' and 'type' attributes cannot appear when 'ref' is specified on an element declaration",
            ));
        }

        if name_opt.is_none() && ref_opt.is_none() {
            return Err(DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: The 'name' attribute is required for an element declaration without 'ref'",
            ));
        }

        let name_str = name_opt
            .or_else(|| {
                ref_opt.as_ref().map(|r| {
                    let clean = r.split(':').next_back().unwrap_or(r);
                    alloc::string::ToString::to_string(clean)
                })
            })
            .unwrap_or_else(|| elem_qname.local_name.clone());

        let ref_ns = ref_opt.as_ref().and_then(|r| {
            if let Some((p, _)) = r.split_once(':') {
                reader.resolve_prefix(p).map(alloc::string::ToString::to_string)
            } else {
                None
            }
        });

        let is_qualified = if is_top_level {
            target_namespace.is_some()
        } else if let Some(ref form) = form_opt {
            form == "qualified" && target_namespace.is_some()
        } else if ref_opt.is_some() {
            ref_ns.is_some() || target_namespace.is_some()
        } else {
            element_form_default && target_namespace.is_some()
        };

        let qname = if is_qualified {
            let effective_ns = ref_ns.as_deref().or(target_namespace);
            if let Some(target_ns) = effective_ns {
                let prefix = reader.find_prefixes_for_uri(target_ns).into_iter().next();
                QName::with_namespace(target_ns, &name_str, prefix.as_deref())
            } else {
                QName::local(&name_str)
            }
        } else {
            QName::local(&name_str)
        };

        let mut inline_type = None;
        let mut inline_base_type_name = None;
        let mut in_complex_type = false;
        let mut complex_type_has_model_group = false;
        let mut seen_rep_items: Vec<(String, RepRangeOrVal)> = Vec::new();

        while let Some(event) = reader.next_event()? {
            match event {
                XmlEvent::StartElement {
                    ref name,
                    ref attributes,
                    ..
                } => {
                    let local = name.local_name.as_str();
                    if local == "sequence" {
                        if in_complex_type {
                            let seq = self.parse_sequence_node(
                                reader,
                                attributes,
                                xsd_prefixes,
                                target_namespace,
                                element_form_default,
                            )?;
                            if seq.properties.get_property("hiddenGroupRef").is_some() {
                                return Err(DFDLError::new_static(
                                    DFDLErrorKind::SchemaDefinition,
                                    "Schema Definition Error: A complex type cannot have a sequence with a hiddenGroupRef as its model group",
                                ));
                            }
                            inline_type = Some(XsdType::InlineSequence(seq));
                            complex_type_has_model_group = true;
                        } else {
                            reader.push_back(event);
                            break;
                        }
                    } else if local == "choice" {
                        if in_complex_type {
                            let choice = self.parse_choice_node(
                                reader,
                                attributes,
                                xsd_prefixes,
                                target_namespace,
                                element_form_default,
                            )?;
                            inline_type = Some(XsdType::InlineChoice(choice));
                            complex_type_has_model_group = true;
                        } else {
                            reader.push_back(event);
                            break;
                        }

                    } else if local == "group" {
                        if in_complex_type {
                            let group_term =
                                self.parse_group_ref_node(reader, attributes, xsd_prefixes)?;
                            let seq = XsdSequence {
                                members: alloc::vec![group_term],
                                properties: PropertyStore::new(),
                            };
                            inline_type = Some(XsdType::InlineSequence(seq));
                            complex_type_has_model_group = true;
                        } else {
                            reader.push_back(event);
                            break;
                        }
                    } else if local == "format" {
                        if !in_complex_type {
                            extract_dfdl_attributes(attributes, &mut local_props)?;
                        }
                    } else if local == "annotation" || local == "appinfo" {
                        let mut dummy_defs = Vec::new();
                        let mut dummy_fmts = Vec::new();
                        let mut dummy_schemes = Vec::new();
                        if in_complex_type {
                            let mut dummy_props = PropertyStore::new();
                            self.parse_annotation_container(
                                reader,
                                local,
                                &mut dummy_props,
                                &mut dummy_defs,
                                &mut dummy_fmts,
                                &mut dummy_schemes,
                                Some("xs:complexType"),
                                xsd_prefixes,
                                target_namespace,
                            )?;
                        } else {
                            extract_dfdl_attributes(attributes, &mut local_props)?;
                            self.parse_annotation_container(
                                reader,
                                local,
                                &mut local_props,
                                &mut dummy_defs,
                                &mut dummy_fmts,
                                &mut dummy_schemes,
                                Some("xs:element"),
                                xsd_prefixes,
                                target_namespace,
                            )?;
                        }
                    } else if local == "assert" {
                        if in_complex_type {
                            continue;
                        }
                        let test_kind_str = attributes
                            .iter()
                            .find(|a| a.name.local_name == "testKind")
                            .map(|a| &a.value[..])
                            .unwrap_or("expression");
                        let test_kind = if test_kind_str == "pattern" {
                            let test_pat = attributes
                                .iter()
                                .find(|a| a.name.local_name == "testPattern")
                                .map(|a| &a.value[..]);
                            let is_empty = match test_pat {
                                None => true,
                                Some(p) => p.trim().is_empty(),
                            };
                            if is_empty {
                                local_props.add_assert_error(
                                    "Schema Definition Error: The attribute testPattern must not be empty for testKind='pattern'",
                                );
                            }
                            dfdl_core::schema::ir::TestKind::Pattern
                        } else {
                            dfdl_core::schema::ir::TestKind::Expression
                        };
                        let test = attributes
                            .iter()
                            .find(|a| {
                                a.name.local_name == "test" || a.name.local_name == "testPattern"
                            })
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
                        let msg = attributes
                            .iter()
                            .find(|a| a.name.local_name == "message")
                            .map(|a| &a.value[..]);
                        if let Some(t) = test {
                            local_props.add_assert_with_failure_type(test_kind, t, msg, failure_type);
                        }
                    } else if local == "complexType" {
                        in_complex_type = true;
                    } else if local == "simpleType" || local == "restriction" {
                        extract_dfdl_attributes(attributes, &mut local_props)?;
                        local_props.update_namespaces(&reader.current_element_namespace_bindings());
                        if local == "restriction" {
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
                                let parsed = self.parse_xsd_type_name(base, xsd_prefixes);
                                let (pfx_opt, _local_base) = if let Some((p, l)) = base.split_once(':') {
                                    (Some(p), l)
                                } else {
                                    (None, base)
                                };
                                let is_xsd = match pfx_opt {
                                    Some(p) => xsd_prefixes.iter().any(|xp| xp == p),
                                    None => false,
                                };
                                if is_xsd && matches!(parsed, XsdType::Complex(_)) {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: Unknown base type for xs:restriction: '{}'",
                                        base
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                                inline_type = Some(parsed);
                                inline_base_type_name =
                                    Some(alloc::string::ToString::to_string(base));
                            }
                        }
                    } else if local == "minInclusive"
                        || local == "maxInclusive"
                        || local == "minExclusive"
                        || local == "maxExclusive"
                        || local == "pattern"
                        || local == "enumeration"
                        || local == "minLength"
                        || local == "maxLength"
                        || local == "totalDigits"
                        || local == "fractionDigits"
                        || local == "length"
                    {
                        if let Some(val) = attributes
                            .iter()
                            .find(|a| a.name.local_name == "value")
                            .map(|a| &a.value[..])
                        {
                            if local == "totalDigits" {
                                if let Ok(td) = val.parse::<u32>() {
                                    if td == 0 {
                                        return Err(DFDLError::new(
                                            DFDLErrorKind::SchemaDefinition,
                                            "Schema Definition Error: totalDigits facet must be a positive integer greater than 0",
                                        ));
                                    }
                                } else {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::SchemaDefinition,
                                        "Schema Definition Error: totalDigits facet value must be a valid positive integer",
                                    ));
                                }
                            }
                            if local == "enumeration" {
                                local_props.add_enumeration(val)?;
                                parse_and_validate_enumeration_rep_attributes(
                                    attributes,
                                    val,
                                    &mut local_props,
                                    &mut seen_rep_items,
                                )?;
                            } else {
                                let prop_name = if local == "length" {
                                    "xsdLength"
                                } else {
                                    local
                                };
                                let _ = local_props.set_property(prop_name, val);
                            }
                        }
                    } else {
                        reader.push_back(event);
                        break;
                    }
                }
                XmlEvent::EndElement { ref name, .. } => {
                    let local = name.local_name.as_str();
                    if local == "complexType" {
                        if in_complex_type {
                            in_complex_type = false;
                            if !complex_type_has_model_group {
                                inline_type = Some(XsdType::EmptyComplex);
                            }
                        }
                    } else if local == "element"
                        || local == elem_qname.local_name
                        || local == name_str
                    {
                        break;
                    } else if local == "annotation"
                        || local == "appinfo"
                        || local == "format"
                        || local == "assert"
                        || local == "discriminator"
                        || local == "simpleType"
                        || local == "restriction"
                        || local == "minInclusive"
                        || local == "maxInclusive"
                        || local == "minExclusive"
                        || local == "maxExclusive"
                        || local == "pattern"
                        || local == "enumeration"
                        || local == "minLength"
                        || local == "maxLength"
                        || local == "totalDigits"
                        || local == "fractionDigits"
                        || local == "length"
                    {
                        // Child end tags, skip
                    } else {
                        reader.push_back(event);
                        break;
                    }
                }
                _ => {}
            }
        }

        let elem_type = if let Some(ref it) = inline_type {
            it.clone()
        } else if let Some(ref type_name) = type_opt {
            let parsed = self.parse_xsd_type_name(type_name, xsd_prefixes);
            let (pfx_opt, _local_base) = if let Some((p, l)) = type_name.split_once(':') {
                (Some(p), l)
            } else {
                (None, type_name.as_str())
            };
            let is_xsd = match pfx_opt {
                Some(p) => xsd_prefixes.iter().any(|xp| xp == p),
                None => false,
            };
            if is_xsd && matches!(parsed, XsdType::Complex(_)) {
                let msg = alloc::format!(
                    "Schema Definition Error: Unknown type '{}' on element '{}'",
                    type_name, name_str
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            parsed
        } else {
            XsdType::Simple(DfdlSimpleType::String)
        };

        let effective_type_name = type_opt.clone().or_else(|| inline_base_type_name.clone());

        if !in_complex_type {
            let elem_rep_type = local_props
                .get_property("repType")
                .or_else(|| local_props.get_property("dfdlx:repType"));
            if let Some(rep_type_name) = elem_rep_type {
                if inline_base_type_name.is_some() && seen_rep_items.is_empty() {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Schema Definition Error: A type with dfdlx:repType must define at least one enumeration.",
                    ));
                }
                if type_opt.is_some() && inline_base_type_name.is_none() {
                    let msg = alloc::format!(
                        "Schema Definition Error: A simpleType with dfdlx:repType '{}' must have an xs:restriction base. A restriction is required.",
                        rep_type_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
            let has_ovc = local_props.get_property("outputValueCalc").is_some();
            let has_reptype = elem_rep_type.is_some();
            if has_ovc && has_reptype {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: dfdl:outputValueCalc and dfdlx:repType cannot be defined on the same element",
                ));
            }
        }

        if is_top_level && ref_opt.is_none() {
            let _ = local_props.set_property("__dfdl_is_top_level", "true");
        }
        Ok(XsdElement {
            name: qname,
            elem_type,
            type_name: effective_type_name,
            min_occurs,
            max_occurs,
            is_nillable: nillable,
            default_value: default_val,
            properties: local_props,
        })
    }

    fn parse_sequence_node(
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
            let clean_ref = hgr.trim_start_matches("tns:").trim_start_matches("ex:");
            let clean_ref = clean_ref.split(':').next_back().unwrap_or(clean_ref);
            let mut hg_props = PropertyStore::new();
            let _ = hg_props.set_property("is_hidden_group", "true");
            members.push(XsdTerm::GroupRef(QName::local(clean_ref), hg_props));
            let _ = seq_props.set_property("hiddenGroupRef", clean_ref);
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

    fn parse_choice_node(
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

    fn validate_choice_branches(options: &[XsdTerm]) -> DFDLResult<()> {
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

    fn parse_group_ref_node(
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
        let clean_ref = ref_val.trim_start_matches("tns:").trim_start_matches("ex:");
        let clean_ref = clean_ref.split(':').next_back().unwrap_or(clean_ref);
        let qname = QName::local(clean_ref);

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

    fn validate_model_group_members(members: &[XsdTerm]) -> DFDLResult<()> {
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

    fn can_term_produce_output_without_infoset(
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

    fn validate_hidden_group_elements(
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

    fn validate_term_assertions(props: &PropertyStore) -> DFDLResult<()> {
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
        Ok(())
    }

    fn read_annotation_inner_text(
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
    fn resolve_variable_value(
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

    fn resolve_qname_properties(reader: &XmlReader, store: &mut PropertyStore) {
        store.update_namespaces(&reader.current_element_namespace_bindings());
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

    fn parse_define_escape_scheme_element(
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

        let clean_es_name = es_name.split(':').next_back().unwrap_or(es_name);
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
    fn parse_annotation_container(
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
                            let key = r.split(':').next_back().unwrap_or(r);
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
                                let key = n.split(':').next_back().unwrap_or(n);
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
                            let clean_vname = var_name.split(':').next_back().unwrap_or(var_name);
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
                            let clean_fname = fmt_name.split(':').next_back().unwrap_or(fmt_name);
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

    fn parse_xsd_type_name(&self, type_name: &str, xsd_prefixes: &[String]) -> XsdType {
        let clean = type_name.trim();
        let (prefix_opt, name) = if let Some((p, l)) = clean.split_once(':') {
            (Some(p), l)
        } else {
            (None, clean)
        };

        let is_xsd_ns = match prefix_opt {
            Some(p) => xsd_prefixes.iter().any(|xp| xp == p),
            None => true,
        };

        if is_xsd_ns {
            match name {
                "int" => return XsdType::Simple(DfdlSimpleType::Int),
                "long" => return XsdType::Simple(DfdlSimpleType::Long),
                "integer" | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger"
                | "negativeInteger" => return XsdType::Simple(DfdlSimpleType::Decimal),
                "short" => return XsdType::Simple(DfdlSimpleType::Short),
                "byte" => return XsdType::Simple(DfdlSimpleType::Byte),
                "unsignedInt" => return XsdType::Simple(DfdlSimpleType::UnsignedInt),
                "unsignedLong" => return XsdType::Simple(DfdlSimpleType::UnsignedLong),
                "unsignedShort" => return XsdType::Simple(DfdlSimpleType::UnsignedShort),
                "unsignedByte" => return XsdType::Simple(DfdlSimpleType::UnsignedByte),
                "boolean" => return XsdType::Simple(DfdlSimpleType::Boolean),
                "float" => return XsdType::Simple(DfdlSimpleType::Float),
                "double" => return XsdType::Simple(DfdlSimpleType::Double),
                "hexBinary" => return XsdType::Simple(DfdlSimpleType::HexBinary),
                "dateTime" => return XsdType::Simple(DfdlSimpleType::DateTime),
                "date" => return XsdType::Simple(DfdlSimpleType::Date),
                "time" => return XsdType::Simple(DfdlSimpleType::Time),
                "decimal" => return XsdType::Simple(DfdlSimpleType::Decimal),
                "string" | "normalizedString" | "token" | "anyURI" | "language" | "Name"
                | "NCName" => {
                    return XsdType::Simple(DfdlSimpleType::String);
                }
                _ => {}
            }
        }

        XsdType::Complex(QName {
            namespace: None,
            local_name: String::from(name),
            prefix: prefix_opt.map(String::from),
        })
    }

    #[allow(dead_code)]
    fn lower_schema_to_ir(&self, schema: &XsdSchema) -> DFDLResult<CompiledSchema> {
        self.lower_schema_to_ir_with_root(schema, None)
    }

    fn validate_simple_type_enumeration_subsets(schema: &XsdSchema) -> DFDLResult<()> {
        for (st_qname, st_type, st_props) in &schema.named_simple_types {
            let local_enums: Vec<&str> = st_props
                .bindings()
                .iter()
                .filter(|b| b.key == "enumeration")
                .map(|b| b.value.as_str())
                .collect();
            if local_enums.is_empty() {
                continue;
            }
            let mut curr = st_type;
            while let XsdType::Complex(ref base_q) = curr {
                if let Some((_, next_type, base_props)) = schema
                    .named_simple_types
                    .iter()
                    .find(|(n, _, _)| n.local_name == base_q.local_name)
                {
                    let base_enums: Vec<&str> = base_props
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
                                    local_val, st_qname.local_name, base_q.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                        break;
                    }
                    curr = next_type;
                } else {
                    break;
                }
            }
        }
        Ok(())
    }

    fn lower_schema_to_ir_with_root(
        &self,
        schema: &XsdSchema,
        target_root: Option<&str>,
    ) -> DFDLResult<CompiledSchema> {
        Self::validate_simple_type_enumeration_subsets(schema)?;
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
            let clean_r = r_name.split(':').next_back().unwrap_or(r_name);
            schema
                .top_level_elements
                .iter()
                .find(|e| e.name.local_name == clean_r)
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

    fn resolve_ref_formats(
        &self,
        store: &mut PropertyStore,
        defined_formats: &[(QName, PropertyStore)],
    ) -> DFDLResult<()> {
        let mut visited = Vec::new();
        self.resolve_ref_formats_bounded(store, defined_formats, &mut visited)
    }

    fn resolve_ref_formats_bounded(
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
    fn validate_escape_delimiter_conflict(
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

    fn resolve_escape_scheme_ref(
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

    fn lower_element_to_ir_bounded(
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

        let mut curr_st = match ref_type {
            XsdType::Complex(qname) => Some(qname.clone()),
            _ => None,
        };
        while let Some(qname) = curr_st {
            curr_st = None;
            if let Some((st_qname, st_type, st_props)) = schema
                .named_simple_types
                .iter()
                .find(|(n, _, _)| n.local_name == qname.local_name)
            {
                let mut resolved_st_props = st_props.clone();
                self.resolve_ref_formats(&mut resolved_st_props, &schema.defined_formats)?;

                let st_tag = if let Some(ref p) = st_qname.prefix {
                    alloc::format!("{}:{}", p, st_qname.local_name)
                } else if let Some(ref p) = qname.prefix {
                    alloc::format!("{}:{}", p, qname.local_name)
                } else {
                    st_qname.local_name.clone()
                };

                let elem_enums: Vec<&str> = elem_direct_props
                    .bindings()
                    .iter()
                    .filter(|b| b.key == "enumeration")
                    .map(|b| b.value.as_str())
                    .collect();
                if !elem_enums.is_empty() {
                    let base_enums: Vec<&str> = st_props
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

                for binding in resolved_st_props.bindings() {
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
                    for binding in resolved_st_props.bindings() {
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

                seen_st_props.push((st_tag, resolved_st_props.clone()));
                effective_elem_props.extend_excluding(&resolved_st_props, &elem_direct_props);
                if let XsdType::Complex(base_q) = st_type {
                    curr_st = Some(base_q.clone());
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

        if let Some(plt) = effective_elem_props
            .get_property("prefixLengthType")
            .map(String::from)
        {
            let parts: Vec<&str> = plt.split(':').collect();
            let clean_plt = if parts.len() == 2 {
                parts.get(1).copied().unwrap_or(&plt)
            } else if !parts.is_empty() {
                parts.first().copied().unwrap_or(&plt)
            } else {
                &plt
            };
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
            ) || schema.named_simple_types.iter().any(|(n, st_type, _)| {
                n.local_name == clean_plt
                    && matches!(
                        st_type,
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
            if let Some((_, st_type, simple_type_props)) = schema
                .named_simple_types
                .iter()
                .find(|(n, _, _)| n.local_name == clean_plt)
            {
                if simple_type_props.has_asserts() || simple_type_props.discriminator_count > 0 {
                    let msg = alloc::format!(
                        "Schema Definition Error: prefixLengthType '{}' specifies one or more statement annotations: dfdl:assert",
                        plt
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                let mut resolved_st = simple_type_props.clone();
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
                // If length is explicitly specified on the simpleType, take it into account for bounds validation.
                let effective_len = if len_kind == "explicit" || len.is_some() {
                    len.map(alloc::string::ToString::to_string)
                } else if len_kind == "prefixed" {
                    if let Some(nested_plt) = resolved_st.get_property("prefixLengthType") {
                        let nested_parts: Vec<&str> = nested_plt.split(':').collect();
                        let nested_clean_plt = if nested_parts.len() == 2 {
                            nested_parts.get(1).copied().unwrap_or(nested_plt)
                        } else if !nested_parts.is_empty() {
                            nested_parts.first().copied().unwrap_or(nested_plt)
                        } else {
                            nested_plt
                        };
                        if let Some((_, _, nested_st_props)) = schema
                            .named_simple_types
                            .iter()
                            .find(|(n, _, _)| n.local_name == nested_clean_plt)
                        {
                            let mut resolved_nested_st = nested_st_props.clone();
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
                            let n_min = nested_st_props.get_property("minInclusive").unwrap_or("");
                            let n_max = nested_st_props.get_property("maxInclusive").unwrap_or("");
                            Some(alloc::format!("@{nested_clean_plt},{n_rep},{n_len},{n_units},{n_min},{n_max}@"))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
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
                let min_inc = simple_type_props.get_property("minInclusive").unwrap_or("");
                let max_inc = simple_type_props.get_property("maxInclusive").unwrap_or("");
                let pad_char = resolved_st
                    .get_property("textNumberPadCharacter")
                    .or_else(|| resolved_st.get_property("textPadChar"))
                    .unwrap_or("");
                let desc = if let Some(ref l) = effective_len {
                    alloc::format!("{clean_plt}:{rep}:{l}:{units}:{min_inc}:{max_inc}:{pad_char}")
                } else {
                    alloc::format!("{clean_plt}:{rep}::{units}:{min_inc}:{max_inc}:{pad_char}")
                };
                let _ = effective_elem_props.set_property("prefixLengthType", &desc);
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
                let desc = alloc::format!("{clean_plt}:{rep}:{len}:{units}:::");
                let _ = effective_elem_props.set_property("prefixLengthType", &desc);
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
                if let Some((_, st_type, _)) = schema
                    .named_simple_types
                    .iter()
                    .find(|(n, _, _)| n.local_name == qname.local_name)
                {
                    let mut curr_type = st_type;
                    let mut type_depth: usize = 0;
                    while let XsdType::Complex(ref next_qname) = curr_type {
                        if type_depth >= 16 {
                            break;
                        }
                        type_depth = type_depth.saturating_add(1);
                        if let Some((_, next_st, _)) = schema
                            .named_simple_types
                            .iter()
                            .find(|(n, _, _)| n.local_name == next_qname.local_name)
                        {
                            curr_type = next_st;
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
        {
            if effective_elem_props
                .get_property("terminator")
                .is_some_and(|t| !t.is_empty() && t != "%ES;")
            {
                let _ = effective_elem_props.set_property("lengthKind", "delimited");
            } else if effective_elem_props.get_property("length").is_none()
                && effective_elem_props.get_property("inputValueCalc").is_none()
                && effective_elem_props.get_property("outputValueCalc").is_none()
                && effective_elem_props.get_property("prefixLengthType").is_none()
            {
                let msg = alloc::format!(
                    "Schema Definition Error: Required DFDL property 'lengthKind' is not defined for element '{}'. Non-default Properties searched in multi_A_03.dfdl.xsd, multi_B_03.dfdl.xsd, multi_C_03.dfdl.xsd, multi_D_03.dfdl.xsd, multi_E_03.dfdl.xsd.",
                    elem.name.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
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
            let rep_local = rep_type_str.split(':').next_back().unwrap_or(rep_type_str);
            if let Some((_, rep_xsd_type, rep_props)) = schema
                .named_simple_types
                .iter()
                .find(|(n, _, _)| n.local_name == rep_local)
            {
                let mut rep_props_resolved = rep_props.clone();
                self.resolve_ref_formats(&mut rep_props_resolved, &schema.defined_formats)?;
                let mut curr_rep = rep_xsd_type;
                while let XsdType::Complex(ref next_qname) = curr_rep {
                    if let Some((_, next_st, _)) = schema
                        .named_simple_types
                        .iter()
                        .find(|(n, _, _)| n.local_name == next_qname.local_name)
                    {
                        curr_rep = next_st;
                    } else {
                        break;
                    }
                }
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
                    if rep_props.get_property("lengthUnits").is_some() {
                        resolved_props.length_units = resolved_rep_props.length_units;
                    }
                    if rep_props.get_property("alignment").is_some() {
                        resolved_props.alignment = resolved_rep_props.alignment;
                    }
                    if rep_props.get_property("alignmentUnits").is_some() {
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
        let clean_type_name = elem
            .type_name
            .as_deref()
            .map(|s| s.split(':').next_back().unwrap_or(s))
            .unwrap_or("");
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

    fn lower_sequence_to_ir_bounded(
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

        for member in &seq.members {
            let child_id = self.lower_term_to_ir_bounded(
                builder,
                schema,
                member,
                &effective_props,
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

        let seq_term = TermKind::Sequence(CompiledSequence { members });
        builder.add_term_with_props(QName::local("sequence"), seq_term, resolved_props)
    }

    fn lower_choice_to_ir_bounded(
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

        for option in &choice.options {
            let child_id = self.lower_term_to_ir_bounded(
                builder,
                schema,
                option,
                &effective_props,
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

    fn lower_term_to_ir_bounded(
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

fn compute_implicit_alignment(
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

fn coerce_dfdl_value(val: &DfdlValue, simple_type: DfdlSimpleType) -> Option<DfdlValue> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RepRangeOrVal {
    Single(i64, String),
    Range(i64, i64, String),
}

fn parse_and_validate_enumeration_rep_attributes(
    attributes: &[Attribute],
    val: &str,
    st_props: &mut PropertyStore,
    seen_rep_items: &mut Vec<(String, RepRangeOrVal)>,
) -> DFDLResult<()> {
    let ranges_attr = attributes.iter().find(|a| {
        (a.name.prefix.as_deref() == Some("dfdlx")
            || a.name.prefix.as_deref() == Some("dfdl")
            || a.name.prefix.is_none())
            && a.name.local_name == "repValueRanges"
    });
    if let Some(attr) = ranges_attr {
        let tokens: Vec<&str> = attr.value.split_whitespace().collect();
        if !tokens.len().is_multiple_of(2) {
            return Err(DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Schema Definition Error: dfdlx:repValueRanges must have an even number of values",
            ));
        }
        for chunk in tokens.chunks(2) {
            let (Some(&c0), Some(&c1)) = (chunk.first(), chunk.get(1)) else {
                continue;
            };
            let low: i64 = c0.parse().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Could not parse value in dfdlx:repValueRanges as integer",
                )
            })?;
            let high: i64 = c1.parse().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Could not parse value in dfdlx:repValueRanges as integer",
                )
            })?;
            if low > high {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: In dfdlx:repValueRanges, low value must be less than or equal to high value",
                ));
            }
            let range_str = alloc::format!("{} {}", c0, c1);
            for (_, existing) in seen_rep_items.iter() {
                match existing {
                    RepRangeOrVal::Single(s, raw) => {
                        if *s >= low && *s <= high {
                            let msg = alloc::format!(
                                "Schema Definition Error: Overlapping dfdlx:repValueRanges '{}' and dfdlx:repValues '{}'",
                                range_str, raw
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                    RepRangeOrVal::Range(el, eh, raw) => {
                        if low <= *eh && high >= *el {
                            let msg = alloc::format!(
                                "Schema Definition Error: Overlapping dfdlx:repValueRanges '{}' and dfdlx:repValueRanges '{}'",
                                range_str, raw
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
            seen_rep_items.push((String::from(val), RepRangeOrVal::Range(low, high, range_str)));
        }
    }

    let values_attr = attributes.iter().find(|a| {
        (a.name.prefix.as_deref() == Some("dfdlx")
            || a.name.prefix.as_deref() == Some("dfdl")
            || a.name.prefix.is_none())
            && a.name.local_name == "repValues"
    });
    if let Some(attr) = values_attr {
        for tok in attr.value.split_whitespace() {
            let v: i64 = tok.parse().map_err(|_| {
                DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Schema Definition Error: Could not parse value in dfdlx:repValues as integer",
                )
            })?;
            for (_, existing) in seen_rep_items.iter() {
                match existing {
                    RepRangeOrVal::Single(s, raw) => {
                        if *s == v {
                            let msg = alloc::format!(
                                "Schema Definition Error: Overlapping dfdlx:repValues '{}' and dfdlx:repValues '{}'",
                                tok, raw
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                    RepRangeOrVal::Range(el, eh, raw) => {
                        if v >= *el && v <= *eh {
                            let msg = alloc::format!(
                                "Schema Definition Error: Overlapping dfdlx:repValueRanges '{}' and dfdlx:repValues '{}'",
                                raw, tok
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
            }
            seen_rep_items.push((String::from(val), RepRangeOrVal::Single(v, String::from(tok))));
        }
    }

    if let Some(attr) = values_attr {
        for tok in attr.value.split_whitespace() {
            st_props.add_enumeration_rep(val, tok);
        }
    }

    if let Some(attr) = ranges_attr {
        for chunk in attr.value.split_whitespace().collect::<Vec<_>>().chunks(2) {
            if let (Some(&c0), Some(&c1)) = (chunk.first(), chunk.get(1)) {
                if let (Ok(low), Ok(high)) = (c0.parse::<i64>(), c1.parse::<i64>()) {
                    if high.saturating_sub(low) <= 100_000 {
                        for r in low..=high {
                            let r_str = alloc::format!("{}", r);
                            st_props.add_enumeration_rep(val, &r_str);
                        }
                    } else {
                        st_props.add_enumeration_rep(val, c0);
                    }
                }
            }
        }
    }

    Ok(())
}

fn parse_default_value(
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
    let clean = layer_name.split(':').next_back().unwrap_or(layer_name);
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
