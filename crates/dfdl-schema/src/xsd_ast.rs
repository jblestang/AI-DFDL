//! XSD Intermediate AST representations for schema compilation.
//!
//! Stores parsed XSD element nodes, type definitions, sequences, choices, and annotation blocks
//! before lower-level DFDL IR graph compilation.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::expr::PropertyStore;
use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::types::QName;

/// Kind of XSD element type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XsdType {
    /// Primitive simple type (e.g., `xs:int`, `xs:string`).
    Simple(DfdlSimpleType),
    /// Inline complex type containing a sequence model group.
    InlineSequence(XsdSequence),
    /// Inline complex type containing a choice model group.
    InlineChoice(XsdChoice),
    /// Named complex type reference.
    Complex(QName),
    /// Complex type with no model group child (invalid per DFDL-14-007R).
    EmptyComplex,
}

/// Parsed XSD Element definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdElement {
    /// Qualified name of the element.
    pub name: QName,
    /// Data type of the element.
    pub elem_type: XsdType,
    /// Declared type name string if specified as an attribute.
    pub type_name: Option<String>,
    /// Minimum occurrences count.
    pub min_occurs: usize,
    /// Maximum occurrences count (`None` means unbounded).
    pub max_occurs: Option<usize>,
    /// Nillable flag.
    pub is_nillable: bool,
    /// Default string value if present.
    pub default_value: Option<String>,
    /// Local DFDL property annotation store.
    pub properties: PropertyStore,
}

/// Parsed XSD Sequence model group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdSequence {
    /// Child terms in sequence order.
    pub members: Vec<XsdTerm>,
    /// Local DFDL property annotation store.
    pub properties: PropertyStore,
}

/// Parsed XSD Choice model group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdChoice {
    /// Alternative terms in choice group.
    pub options: Vec<XsdTerm>,
    /// Local DFDL property annotation store.
    pub properties: PropertyStore,
}

/// Individual term inside an XSD model group.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum XsdTerm {
    /// Element declaration item.
    Element(XsdElement),
    /// Sequence group container.
    Sequence(XsdSequence),
    /// Choice group container.
    Choice(XsdChoice),
    /// Named model group reference (`<xs:group ref="...">`).
    GroupRef(QName, PropertyStore),
}

/// Parsed DFDL Variable Definition declaration (§7.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfdlVariableDef {
    /// Qualified name of the variable.
    pub name: QName,
    /// Simple type of the variable.
    pub var_type: DfdlSimpleType,
    /// Optional default value string.
    pub default_value: Option<String>,
    /// Variable direction (§7.6).
    pub direction: dfdl_core::expr::variables::VariableDirection,
}

/// Parsed DFDL SetVariable statement (§7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfdlSetVariable {
    /// Qualified name of target variable to set.
    pub var_name: QName,
    /// Expression or literal value string.
    pub value_expr: String,
}

/// Named simple type declaration in an XSD schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdNamedSimpleType {
    /// Qualified name of the simple type.
    pub name: QName,
    /// Base XSD type.
    pub xsd_type: XsdType,
    /// Direct properties explicitly specified on `<xs:simpleType>`.
    pub local_props: PropertyStore,
    /// Effective properties including inherited `dfdl:format` defaults from its defining schema.
    pub effective_props: PropertyStore,
}

/// Root parsed XSD Schema container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdSchema {
    /// Target namespace URI if declared.
    pub target_namespace: Option<String>,
    /// Global DFDL format annotation property store.
    pub global_format: PropertyStore,
    /// Global element declarations.
    pub top_level_elements: Vec<XsdElement>,
    /// Named model groups (`<xs:group name="...">`).
    pub named_groups: Vec<(QName, XsdSequence)>,
    /// Table of named complex type declarations (`<xs:complexType name="...">`).
    pub named_complex_types: Vec<(QName, XsdType)>,
    /// Table of named simple type declarations (`<xs:simpleType name="...">`).
    pub named_simple_types: Vec<XsdNamedSimpleType>,
    /// Table of global format definitions (`<dfdl:defineFormat>`).
    pub defined_formats: Vec<(QName, PropertyStore)>,
    /// Table of declared DFDL escape schemes (`<dfdl:defineEscapeScheme>`).
    pub defined_escape_schemes: Vec<(QName, PropertyStore)>,
    /// Table of declared DFDL variables (`<dfdl:defineVariable>`).
    pub defined_variables: Vec<DfdlVariableDef>,
    /// Namespace prefixes mapped to http://www.w3.org/2001/XMLSchema.
    pub xsd_prefixes: Vec<String>,
    /// Whether local elements are qualified by default (elementFormDefault="qualified").
    pub element_form_default: bool,
}

impl Default for XsdSchema {
    fn default() -> Self {
        Self {
            target_namespace: None,
            global_format: PropertyStore::new(),
            top_level_elements: Vec::new(),
            named_groups: Vec::new(),
            named_complex_types: Vec::new(),
            named_simple_types: Vec::new(),
            defined_formats: Vec::new(),
            defined_escape_schemes: Vec::new(),
            defined_variables: Vec::new(),
            xsd_prefixes: alloc::vec![String::from("xs"), String::from("xsd")],
            element_form_default: false,
        }
    }
}

impl XsdSchema {
    /// Merges another parsed schema into this schema (for includes and imports).
    pub fn merge(&mut self, other: Self) -> DFDLResult<()> {
        if !other.global_format.is_empty() {
            self.global_format.merge_parent(&other.global_format);
        }

        for (fmt_name, fmt_store) in other.defined_formats {
            if !self
                .defined_formats
                .iter()
                .any(|(n, _)| n == &fmt_name)
            {
                self.defined_formats.push((fmt_name, fmt_store));
            }
        }

        for (es_name, es_store) in other.defined_escape_schemes {
            if !self
                .defined_escape_schemes
                .iter()
                .any(|(n, _)| n == &es_name)
            {
                self.defined_escape_schemes.push((es_name, es_store));
            }
        }

        for elem in other.top_level_elements {
            let is_ref = elem.properties.get_property("__dfdl_element_ref").is_some()
                || elem.properties.get_property("ref").is_some();
            if let Some(existing_idx) = self
                .top_level_elements
                .iter()
                .position(|e| e.name == elem.name)
            {
                let existing_is_ref = self
                    .top_level_elements
                    .get(existing_idx)
                    .map(|e| {
                        e.properties.get_property("__dfdl_element_ref").is_some()
                            || e.properties.get_property("ref").is_some()
                    })
                    .unwrap_or(false);
                let is_identical = self.top_level_elements.get(existing_idx) == Some(&elem);
                if !is_ref && !existing_is_ref && !is_identical {
                    let msg = alloc::format!(
                        "Schema Definition Error: More than one definition for name: {}",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if existing_is_ref && !is_ref {
                    if let Some(slot) = self.top_level_elements.get_mut(existing_idx) {
                        *slot = elem;
                    }
                }
            } else {
                self.top_level_elements.push(elem);
            }
        }
        for (gname, seq) in other.named_groups {
            if let Some(existing) = self.named_groups.iter().find(|(n, _)| n == &gname) {
                if existing.1 != seq {
                    let msg = alloc::format!(
                        "Schema Definition Error: More than one definition for name: {}",
                        gname.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            } else {
                self.named_groups.push((gname, seq));
            }
        }
        for (ctname, ct) in other.named_complex_types {
            if let Some(existing) = self.named_complex_types.iter().find(|(n, _)| n == &ctname) {
                if existing.1 != ct {
                    let msg = alloc::format!(
                        "Schema Definition Error: More than one definition for name: {}",
                        ctname.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            } else {
                self.named_complex_types.push((ctname, ct));
            }
        }
        for other_st in other.named_simple_types {
            let mut eff_props = other_st.effective_props;
            for binding in other.global_format.bindings() {
                if eff_props.get_property(&binding.key).is_none() {
                    let _ = eff_props.set_property(&binding.key, &binding.value);
                }
            }
            if let Some(existing) = self.named_simple_types.iter().find(|st| st.name == other_st.name) {
                if existing.xsd_type != other_st.xsd_type || existing.local_props != other_st.local_props {
                    let msg = alloc::format!(
                        "Schema Definition Error: More than one definition for name: {}",
                        other_st.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            } else {
                self.named_simple_types.push(XsdNamedSimpleType {
                    name: other_st.name,
                    xsd_type: other_st.xsd_type,
                    local_props: other_st.local_props,
                    effective_props: eff_props,
                });
            }
        }
        for var in other.defined_variables {
            if let Some(existing) = self
                .defined_variables
                .iter_mut()
                .find(|v| v.name.local_name == var.name.local_name)
            {
                if var.var_type != crate::xsd_ast::DfdlSimpleType::String {
                    existing.var_type = var.var_type;
                }
                if existing.default_value.is_none() && var.default_value.is_some() {
                    existing.default_value = var.default_value;
                }
            } else {
                self.defined_variables.push(var);
            }
        }
        Ok(())
    }
}
