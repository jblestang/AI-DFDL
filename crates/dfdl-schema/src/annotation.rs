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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use dfdl_core::types::{QName, SourceLocation};

    fn make_attr<'a>(prefix: Option<&'a str>, local: &'a str, val: &'a str) -> Attribute<'a> {
        let mut qn = QName::local(local);
        if let Some(p) = prefix {
            qn.prefix = Some(alloc::string::String::from(p));
        }
        Attribute {
            name: qn,
            value: Cow::Borrowed(val),
            location: SourceLocation::at_offset(0),
        }
    }

    /// Verifies attribute extraction from dfdl/dfdlx/daf prefixes, xmlns skipping, and element validation.
    #[test]
    fn test_extract_dfdl_attributes() {
        let attrs = alloc::vec![
            make_attr(Some("xmlns"), "dfdl", "http://example.com"),
            make_attr(None, "xmlns", "http://example.com"),
            make_attr(Some("dfdl"), "initiator", "["),
            make_attr(Some("dfdlx"), "repType", "xs:int"),
            make_attr(Some("daf"), "suppressSchemaDefinitionWarnings", "all"),
            make_attr(None, "dfdl:terminator", "]"),
            make_attr(None, "dfdlx:choiceBranchKey", "1"),
            make_attr(None, "daf:parseUnparsePolicy", "parseOnly"),
            make_attr(None, "name", "myElem"),
        ];

        let mut store = PropertyStore::new();
        assert!(extract_dfdl_attributes(&attrs, &mut store).is_ok());
        assert_eq!(store.get_property("initiator"), Some("["));
        assert_eq!(store.get_property("terminator"), Some("]"));
        assert_eq!(store.get_property("repType"), Some("xs:int"));

        // With xsd_elem_name: invalid unknown attribute returns error
        let bad_attrs = alloc::vec![make_attr(None, "unauthorizedAttr", "val")];
        let mut store_bad = PropertyStore::new();
        let err = extract_dfdl_attributes_for_element(&bad_attrs, &mut store_bad, Some("element"));
        assert!(matches!(err, Err(ref e) if e.kind == DFDLErrorKind::SchemaDefinition));
    }

    /// Verifies parse_property_element error handling and valid property setting.
    #[test]
    fn test_parse_property_element() {
        let mut store = PropertyStore::new();

        // Missing name attribute
        let err_missing = parse_property_element(None, "val", &mut store);
        assert!(matches!(err_missing, Err(ref e) if e.kind == DFDLErrorKind::SchemaDefinition));

        // Prohibited 'ref' name per DFDL-7-016R
        let err_ref = parse_property_element(Some("ref"), "val", &mut store);
        assert!(matches!(err_ref, Err(ref e) if e.kind == DFDLErrorKind::SchemaDefinition));

        // Valid property setting
        assert!(parse_property_element(Some("separator"), ",", &mut store).is_ok());
        assert_eq!(store.get_property("separator"), Some(","));
    }
}

