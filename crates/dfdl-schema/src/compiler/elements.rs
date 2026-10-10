//! Element, simple type, and complex type parsing and facet validation.

use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::types::QName;
use dfdl_xml::{Attribute, XmlEvent, XmlReader};

use crate::annotation::{extract_dfdl_attributes, extract_dfdl_attributes_for_element};
use crate::xsd_ast::{XsdElement, XsdSequence, XsdType};
use super::*;

pub(crate) fn validate_simple_type_facets(
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



impl SchemaCompiler {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_element_node(
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

        let parsed_ref = ref_opt.as_ref().map(|r| QName::parse(r));
        let name_str = name_opt
            .or_else(|| parsed_ref.as_ref().map(|p| p.local_name.clone()))
            .unwrap_or_else(|| elem_qname.local_name.clone());

        let ref_ns = parsed_ref.as_ref().and_then(|p| {
            p.prefix
                .as_deref()
                .and_then(|pref| reader.resolve_prefix(pref).map(alloc::string::ToString::to_string))
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


    pub(crate) fn parse_xsd_type_name(&self, type_name: &str, xsd_prefixes: &[String]) -> XsdType {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepRangeOrVal {
    Single(i64, String),
    Range(i64, i64, String),
}

pub(crate) fn parse_and_validate_enumeration_rep_attributes(
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

