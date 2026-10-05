//! DFDL Annotation Parsing and Attribute Extractor.
//!
//! Extracts DFDL properties from XML attributes (`dfdl:property="value"`) and
//! annotation elements (`<dfdl:format>`, `<dfdl:element>`, `<dfdl:property>`).

extern crate alloc;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_xml::Attribute;

/// Extracts DFDL properties from XML element attributes and sets them in a [`PropertyStore`].
pub fn extract_dfdl_attributes(
    attributes: &[Attribute],
    store: &mut PropertyStore,
) -> DFDLResult<()> {
    extract_dfdl_attributes_for_element(attributes, store, None)
}

/// Extracts DFDL properties from XML element attributes and checks for valid attribute syntax on XSD components.
pub fn extract_dfdl_attributes_for_element(
    attributes: &[Attribute],
    store: &mut PropertyStore,
    xsd_elem_name: Option<&str>,
) -> DFDLResult<()> {
    for attr in attributes {
        let prefix = attr.name.prefix.as_deref().unwrap_or("");
        let local = attr.name.local_name.as_str();

        if prefix == "xmlns" || local == "xmlns" {
            continue;
        }

        if prefix == "dfdl" || prefix == "dfdlx" || prefix == "daf" {
            store.set_property(local, &attr.value)?;
        } else if let Some(key) = local
            .strip_prefix("dfdl:")
            .or_else(|| local.strip_prefix("dfdlx:"))
            .or_else(|| local.strip_prefix("daf:"))
        {
            store.set_property(key, &attr.value)?;
        } else if let Some(elem_name) = xsd_elem_name {
            if prefix.is_empty() && !is_known_xsd_attribute(local) {
                let msg = alloc::format!(
                    "Schema Definition Error: Attribute '{}' is not allowed to appear in element '{}'",
                    local, elem_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        } else {
            if prefix.is_empty() && is_known_xsd_attribute(local) && local != "ref" {
                continue;
            }
            store.set_property(local, &attr.value)?;
        }
    }
    Ok(())
}

fn is_known_xsd_attribute(name: &str) -> bool {
    matches!(
        name,
        "name"
            | "type"
            | "ref"
            | "minOccurs"
            | "maxOccurs"
            | "nillable"
            | "default"
            | "fixed"
            | "form"
            | "block"
            | "final"
            | "abstract"
            | "substitutionGroup"
            | "schemaLocation"
            | "namespace"
            | "processContents"
            | "elementFormDefault"
            | "attributeFormDefault"
            | "targetNamespace"
            | "base"
            | "id"
            | "use"
            | "itemType"
            | "memberTypes"
            | "mixed"
            | "value"
            | "source"
            | "test"
            | "testKind"
            | "message"
            | "testPattern"
    )
}

/// Parses a `<dfdl:property name="..." ...>` string value.
pub fn parse_property_element(
    name_attr: Option<&str>,
    value_text: &str,
    store: &mut PropertyStore,
) -> DFDLResult<()> {
    let key = name_attr.ok_or_else(|| {
        DFDLError::new_static(
            DFDLErrorKind::SchemaDefinition,
            "Missing 'name' attribute in <dfdl:property>",
        )
    })?;
    if key == "ref" {
        return Err(DFDLError::new_static(
            DFDLErrorKind::SchemaDefinition,
            "Schema Definition Error: 'ref' is not a valid value for dfdl:property name. The ref property may only be specified as an attribute or in short form (DFDL-7-016R).",
        ));
    }
    store.set_property(key, value_text)?;
    Ok(())
}
