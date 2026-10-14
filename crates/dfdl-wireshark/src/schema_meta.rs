//! Generic schema metadata extraction for Wireshark field documentation and unit conversions.
//!
//! Introspects any DFDL XML Schema (`.dfdl.xsd`) to discover elements, simpleTypes,
//! human-readable documentation (`<xs:documentation>`), and custom annotations
//! (e.g. `<ast:unit>`, `<ast:resolution>`, `<ast:lsb>`, `<ast:description>`).
//!
//! This module contains NO hardcoded protocol or field names; all metadata is extracted
//! dynamically from the supplied DFDL schema.

use dfdl_xml::{XmlEvent, XmlReader};
use std::collections::{HashMap, HashSet};

/// Metadata associated with an XML Schema element or simpleType.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldMetadata {
    /// Element or type identifier as declared in the schema.
    pub name: String,
    /// Standard item reference or identifier (from `<*:item>` or similar).
    pub item_ref: String,
    /// Human-readable title or description.
    pub description: String,
    /// Complete documentation text from `<xs:documentation>`.
    pub documentation: String,
    /// Measurement unit (from `<*:unit>` e.g. `seconds`, `NM`, `degrees`, `FL`, `m`).
    pub unit: String,
    /// Raw resolution or quantum string (from `<*:resolution>`, e.g. `1/128`, `360/65536`).
    pub resolution_str: String,
    /// Descriptive LSB representation (from `<*:lsb>`).
    pub lsb_str: String,
    /// Multiplicative scale factor to convert raw binary integers to physical values.
    pub scale_factor: f64,
    /// Optional minimum allowed value.
    pub min_inclusive: Option<f64>,
    /// Optional maximum allowed value.
    pub max_inclusive: Option<f64>,
    /// Wireshark proto field type: `uint32`, `int32`, `double`, `string`, `bool`.
    pub proto_type: String,
    /// Enumeration mapping from numeric value to human-readable label.
    pub enum_map: HashMap<i64, String>,
}

impl FieldMetadata {
    /// Creates a default metadata descriptor for a given field name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            item_ref: String::new(),
            description: String::new(),
            documentation: String::new(),
            unit: String::new(),
            resolution_str: String::new(),
            lsb_str: String::new(),
            scale_factor: 1.0,
            min_inclusive: None,
            max_inclusive: None,
            proto_type: "string".to_string(),
            enum_map: HashMap::new(),
        }
    }

    /// Looks up the textual enumeration label for a given numeric value.
    #[must_use]
    pub fn get_enum_label(&self, value: i64) -> Option<&str> {
        self.enum_map.get(&value).map(String::as_str)
    }
}

/// Generic registry mapping field names and type names to their extracted schema metadata.
#[derive(Debug, Clone)]
pub struct SchemaMetadataRegistry {
    /// Name of the primary root element discovered in the schema.
    pub root_element: Option<String>,
    /// Protocol name inferred from schema root element or targetNamespace.
    pub protocol_name: String,
    /// Discovered record-level elements that start a hierarchical scope.
    pub record_elements: HashSet<String>,
    /// Child containment mapping: parent element or complexType -> list of (child_element_name, child_type).
    pub containment: HashMap<String, Vec<(String, String)>>,
    /// Precomputed hierarchical paths: `(hierarchical_path, leaf_element_name)`.
    pub hierarchical_fields: Vec<(String, String)>,
    /// Map of metadata indexed by lowercase element or type name.
    entries: HashMap<String, FieldMetadata>,
    /// Map of element names to their associated simpleType names.
    element_to_type: HashMap<String, String>,
}

impl Default for SchemaMetadataRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaMetadataRegistry {
    /// Creates an empty schema metadata registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root_element: None,
            protocol_name: String::new(),
            record_elements: HashSet::new(),
            containment: HashMap::new(),
            hierarchical_fields: Vec::new(),
            entries: HashMap::new(),
            element_to_type: HashMap::new(),
        }
    }

    /// Returns `true` if the element name is a record-level element.
    #[must_use]
    pub fn is_record_element(&self, name: &str) -> bool {
        self.record_elements.contains(name) || self.record_elements.contains(&name.to_lowercase())
    }

    /// Registers a field metadata entry.
    pub fn insert(&mut self, meta: FieldMetadata) {
        let key = meta.name.to_lowercase();
        self.entries.insert(key, meta);
    }

    /// Associates an element name with a simpleType name.
    pub fn link_element_type(&mut self, element_name: &str, type_name: &str) {
        self.element_to_type.insert(element_name.to_lowercase(), type_name.to_string());
    }

    /// Retrieves metadata for a field name, checking direct element metadata first
    /// and falling back to its associated simpleType metadata.
    #[must_use]
    pub fn get(&self, field_name: &str) -> Option<&FieldMetadata> {
        let key = field_name.to_lowercase();
        if let Some(meta) = self.entries.get(&key) {
            if !meta.unit.is_empty() || !meta.documentation.is_empty() || meta.scale_factor != 1.0 {
                return Some(meta);
            }
        }

        // Check if field is linked to a declared type
        if let Some(type_name) = self.element_to_type.get(&key) {
            let type_key = type_name.to_lowercase();
            if let Some(type_meta) = self.entries.get(&type_key) {
                return Some(type_meta);
            }
        }

        self.entries.get(&key)
    }

    /// Returns the total count of registered metadata entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no metadata entries exist in the registry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Inspects all registered metadata entries.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &FieldMetadata)> {
        self.entries.iter()
    }

    /// Parses any DFDL XML schema string and builds the metadata registry dynamically.
    ///
    /// Scans all `<xs:element>`, `<xs:simpleType>`, `<xs:complexType>`, and nested
    /// `<xs:annotation>` elements without any hardcoded protocol assumptions.
    #[must_use]
    pub fn from_schema_xml(schema_xml: &str) -> Self {
        #[derive(Debug, Default)]
        struct DeclContext {
            tag: String,
            name: String,
            type_name: String,
            is_ref: bool,
            doc: String,
            item: String,
            desc: String,
            unit: String,
            res: String,
            lsb: String,
            min: Option<f64>,
            max: Option<f64>,
            enums: HashMap<i64, String>,
        }

        let mut registry = Self::new();
        let mut reader = XmlReader::new(schema_xml);

        let mut decl_stack: Vec<DeclContext> = Vec::new();
        let mut in_doc = false;
        let mut in_appinfo = false;
        let mut choice_depth = 0usize;
        let mut current_appinfo_tag = String::new();
        let mut pending_enum_val: Option<i64> = None;

        while let Ok(Some(ev)) = reader.next_event() {
            match ev {
                XmlEvent::StartElement { name, attributes, .. } => {
                    let local = name.local_name.as_str();

                    if local == "schema" {
                        if let Some(tns) = attributes.iter().find(|a| a.name.local_name == "targetNamespace") {
                            if registry.protocol_name.is_empty() {
                                let val = tns.value.trim();
                                if let Some((_, last)) = val.rsplit_once(':') {
                                    registry.protocol_name = last.to_string();
                                } else if let Some((_, last)) = val.rsplit_once('/') {
                                    registry.protocol_name = last.to_string();
                                }
                            }
                        }
                    } else if local == "choice" {
                        choice_depth = choice_depth.saturating_add(1);
                    } else if local == "simpleType" || local == "element" || local == "complexType" || local == "group" {
                        let name_attr = attributes.iter().find(|a| a.name.local_name == "name");
                        let ref_attr = attributes.iter().find(|a| a.name.local_name == "ref");
                        let type_attr = attributes.iter().find(|a| a.name.local_name == "type");
                        // An element declared as a direct branch of a top-level dispatching choice
                        // represents an independent message/record variant in the protocol hierarchy.
                        let is_top_choice_branch = choice_depth > 0
                            && decl_stack.len() <= 2
                            && decl_stack.first().map(|d| d.tag.as_str()) == Some("element");

                        if let Some(r) = ref_attr {
                            let ref_val = clean_type_name(r.value.as_ref());
                            if let Some(parent) = decl_stack.last() {
                                registry.containment.entry(parent.name.clone()).or_default().push((ref_val.clone(), ref_val.clone()));
                                if !parent.type_name.is_empty() {
                                    registry.containment.entry(parent.type_name.clone()).or_default().push((ref_val.clone(), ref_val.clone()));
                                }
                            }
                            if is_top_choice_branch {
                                registry.record_elements.insert(ref_val.clone());
                            }
                            decl_stack.push(DeclContext {
                                tag: local.to_string(),
                                name: ref_val,
                                is_ref: true,
                                ..Default::default()
                            });
                        } else if let Some(n) = name_attr {
                            let name_val = n.value.to_string();
                            if local == "element" {
                                if registry.root_element.is_none() {
                                    registry.root_element = Some(name_val.clone());
                                    if registry.protocol_name.is_empty() {
                                        registry.protocol_name = name_val.clone();
                                    }
                                }
                                if is_top_choice_branch {
                                    registry.record_elements.insert(name_val.clone());
                                }
                            }
                            let mut type_val = String::new();
                            if let Some(t) = type_attr {
                                type_val = clean_type_name(t.value.as_ref());
                                registry.link_element_type(&name_val, &type_val);
                            }
                            if let Some(parent) = decl_stack.last() {
                                registry.containment.entry(parent.name.clone()).or_default().push((name_val.clone(), type_val.clone()));
                                if !parent.type_name.is_empty() {
                                    registry.containment.entry(parent.type_name.clone()).or_default().push((name_val.clone(), type_val.clone()));
                                }
                            }
                            decl_stack.push(DeclContext {
                                tag: local.to_string(),
                                name: name_val,
                                type_name: type_val,
                                is_ref: false,
                                ..Default::default()
                            });
                        } else if local == "complexType" {
                            let parent_name = decl_stack.last().map(|d| d.name.clone()).unwrap_or_default();
                            decl_stack.push(DeclContext {
                                tag: local.to_string(),
                                name: parent_name,
                                ..Default::default()
                            });
                        }
                    } else if (local == "enum" || local == "enumeration") && !decl_stack.is_empty() {
                        let val_opt = attributes
                            .iter()
                            .find(|a| a.name.local_name == "value")
                            .and_then(|a| a.value.trim().parse::<i64>().ok());
                        let label_opt = attributes
                            .iter()
                            .find(|a| {
                                a.name.local_name == "label"
                                    || a.name.local_name == "name"
                                    || a.name.local_name == "description"
                            })
                            .map(|a| a.value.trim().to_string());

                        if let (Some(val), Some(label)) = (val_opt, label_opt) {
                            if let Some(top) = decl_stack.last_mut() {
                                top.enums.insert(val, label);
                            }
                        } else if let Some(val) = val_opt {
                            pending_enum_val = Some(val);
                        }
                    } else if local == "documentation" && !decl_stack.is_empty() {
                        in_doc = true;
                        if let Some(top) = decl_stack.last_mut() {
                            top.doc.clear();
                        }
                    } else if local == "appinfo" && !decl_stack.is_empty() {
                        in_appinfo = true;
                    } else if in_appinfo {
                        current_appinfo_tag = local.to_string();
                    } else if !decl_stack.is_empty() && local == "minInclusive" {
                        if let Some(v) = attributes.iter().find(|a| a.name.local_name == "value") {
                            if let Some(top) = decl_stack.last_mut() {
                                top.min = v.value.trim().parse::<f64>().ok();
                            }
                        }
                    } else if !decl_stack.is_empty() && local == "maxInclusive" {
                        if let Some(v) = attributes.iter().find(|a| a.name.local_name == "value") {
                            if let Some(top) = decl_stack.last_mut() {
                                top.max = v.value.trim().parse::<f64>().ok();
                            }
                        }
                    } else if !decl_stack.is_empty() && local == "restriction" {
                        if let Some(b) = attributes.iter().find(|a| a.name.local_name == "base") {
                            let base_type = clean_type_name(b.value.as_ref());
                            if let Some(top) = decl_stack.last_mut() {
                                if top.type_name.is_empty() {
                                    top.type_name = base_type.clone();
                                }
                                registry.link_element_type(&top.name, &base_type);
                            }
                        }
                    }
                }
                XmlEvent::Text { ref content, .. } => {
                    let text = content.as_ref();
                    if in_doc {
                        if let Some(top) = decl_stack.last_mut() {
                            top.doc.push_str(text);
                            if let Some(val) = pending_enum_val {
                                let label = text.trim();
                                if !label.is_empty() {
                                    top.enums.insert(val, label.to_string());
                                }
                            }
                        }
                    } else if in_appinfo {
                        let t = text.trim();
                        if let Some(top) = decl_stack.last_mut() {
                            match current_appinfo_tag.as_str() {
                                "item" | "itemReference" | "ref" => top.item.push_str(t),
                                "description" | "itemDescription" | "title" => top.desc.push_str(t),
                                "unit" | "units" => top.unit.push_str(t),
                                "resolution" | "quantum" | "scale" => top.res.push_str(t),
                                "lsb" => top.lsb.push_str(t),
                                _ => {}
                            }
                        }
                    }
                }
                XmlEvent::CData { content, .. } => {
                    if in_doc {
                        if let Some(top) = decl_stack.last_mut() {
                            top.doc.push_str(content);
                            if let Some(val) = pending_enum_val {
                                let label = content.trim();
                                if !label.is_empty() {
                                    top.enums.insert(val, label.to_string());
                                }
                            }
                        }
                    } else if in_appinfo {
                        let t = content.trim();
                        if let Some(top) = decl_stack.last_mut() {
                            match current_appinfo_tag.as_str() {
                                "item" | "itemReference" | "ref" => top.item.push_str(t),
                                "description" | "itemDescription" | "title" => top.desc.push_str(t),
                                "unit" | "units" => top.unit.push_str(t),
                                "resolution" | "quantum" | "scale" => top.res.push_str(t),
                                "lsb" => top.lsb.push_str(t),
                                _ => {}
                            }
                        }
                    }
                }
                XmlEvent::EndElement { name, .. } => {
                    let local = name.local_name.as_str();
                    if local == "choice" {
                        choice_depth = choice_depth.saturating_sub(1);
                    } else if local == "enumeration" || local == "enum" {
                        pending_enum_val = None;
                    } else if local == "documentation" {
                        in_doc = false;
                    } else if local == "appinfo" {
                        in_appinfo = false;
                        current_appinfo_tag.clear();
                    } else if local == "simpleType" || local == "element" || local == "complexType" || local == "group" {
                        if let Some(pos) = decl_stack.iter().rposition(|d| d.tag == local) {
                            let mut decl = decl_stack.remove(pos);
                            if !decl.is_ref && !decl.name.is_empty() {
                                let doc_clean = decl.doc.trim().to_string();
                                let scale = parse_generic_scale(&decl.res, &doc_clean);

                                parse_enums_from_text(&doc_clean, &mut decl.enums);

                                let desc = if !decl.desc.is_empty() {
                                    decl.desc
                                } else {
                                    extract_concise_description(&doc_clean)
                                };

                                let ptype = infer_proto_type(&decl.type_name, scale);

                                let meta = FieldMetadata {
                                    name: decl.name,
                                    item_ref: decl.item,
                                    description: desc,
                                    documentation: doc_clean,
                                    unit: decl.unit,
                                    resolution_str: decl.res,
                                    lsb_str: decl.lsb,
                                    scale_factor: scale,
                                    min_inclusive: decl.min,
                                    max_inclusive: decl.max,
                                    proto_type: ptype,
                                    enum_map: decl.enums,
                                };
                                registry.insert(meta);
                            }
                        }
                        pending_enum_val = None;
                    }
                }
                _ => {}
            }
        }

        // Propagate simpleType and complexType metadata to linked elements
        let links: Vec<(String, String)> = registry
            .element_to_type
            .iter()
            .map(|(elem, ty)| (elem.clone(), ty.to_lowercase()))
            .collect();

        for (elem_key, type_key) in links {
            if let Some(type_meta) = registry.entries.get(&type_key).cloned() {
                if let Some(elem_meta) = registry.entries.get_mut(&elem_key) {
                    if elem_meta.description.is_empty() {
                        elem_meta.description = type_meta.description;
                    }
                    if elem_meta.item_ref.is_empty() {
                        elem_meta.item_ref = type_meta.item_ref;
                    }
                    if elem_meta.unit.is_empty() {
                        elem_meta.unit = type_meta.unit;
                    }
                    if elem_meta.resolution_str.is_empty() {
                        elem_meta.resolution_str = type_meta.resolution_str;
                    }
                    if elem_meta.lsb_str.is_empty() {
                        elem_meta.lsb_str = type_meta.lsb_str;
                    }
                    if (elem_meta.scale_factor - 1.0).abs() < f64::EPSILON
                        && (type_meta.scale_factor - 1.0).abs() >= f64::EPSILON
                    {
                        elem_meta.scale_factor = type_meta.scale_factor;
                    }
                    if elem_meta.proto_type == "string" && type_meta.proto_type != "string" {
                        elem_meta.proto_type = type_meta.proto_type;
                    }
                    if elem_meta.documentation.is_empty() {
                        elem_meta.documentation = type_meta.documentation;
                    }
                    if elem_meta.min_inclusive.is_none() {
                        elem_meta.min_inclusive = type_meta.min_inclusive;
                    }
                    if elem_meta.max_inclusive.is_none() {
                        elem_meta.max_inclusive = type_meta.max_inclusive;
                    }
                    if elem_meta.enum_map.is_empty() {
                        elem_meta.enum_map = type_meta.enum_map;
                    }
                }
            }
        }

        // If no sub-records were identified, treat root element as record
        if registry.record_elements.is_empty() {
            if let Some(ref root) = registry.root_element {
                registry.record_elements.insert(root.clone());
            }
        }

        let record_names: Vec<String> = registry.record_elements.iter().cloned().collect();
        for rec_name in record_names {
            registry.hierarchical_fields.push((rec_name.clone(), rec_name.clone()));
            let mut path_set = HashSet::new();
            expand_hierarchy(
                &rec_name,
                &rec_name,
                &registry.containment,
                &registry.element_to_type,
                &mut registry.hierarchical_fields,
                &mut path_set,
                0,
            );
        }

        registry
    }

    /// Formats an escaped, single-line tooltip description from field metadata.
    #[must_use]
    pub fn build_field_tooltip(meta: &FieldMetadata) -> String {
        let mut parts = Vec::new();
        if !meta.description.is_empty() {
            parts.push(meta.description.clone());
        }
        if !meta.item_ref.is_empty() {
            parts.push(format!("Item: {}", meta.item_ref));
        }
        if !meta.unit.is_empty() {
            parts.push(format!("Unit: {}", meta.unit));
        }
        if !meta.resolution_str.is_empty() {
            parts.push(format!("Resolution: {}", meta.resolution_str));
        }
        if !meta.lsb_str.is_empty() {
            parts.push(format!("LSB: {}", meta.lsb_str));
        }
        if !meta.enum_map.is_empty() {
            let mut sorted_enums: Vec<_> = meta.enum_map.iter().collect();
            sorted_enums.sort_by_key(|(k, _)| **k);
            let enum_str = sorted_enums
                .into_iter()
                .take(8)
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("Values: [{enum_str}]"));
        }
        if !meta.documentation.is_empty() {
            let single_line = meta
                .documentation
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            parts.push(single_line);
        }
        if parts.is_empty() {
            meta.name.clone()
        } else {
            parts.join(" | ")
        }
    }

    /// Serializes registered fields into a JSON array for dynamic Lua / FFI dissectors.
    #[must_use]
    pub fn to_fields_json(&self) -> String {
        let mut out = String::from("[");
        let mut first = true;

        // 1. Output all hierarchical fields (primary display filters)
        for (path, leaf_name) in &self.hierarchical_fields {
            if !first {
                out.push(',');
            }
            first = false;
            let meta_opt = self.get(leaf_name);
            let description = meta_opt
                .map(|m| m.description.as_str())
                .unwrap_or("")
                .trim();
            let label = if !description.is_empty() && description != leaf_name {
                format!("{description} ({leaf_name})")
            } else {
                leaf_name.clone()
            };
            let unit = meta_opt.map(|m| m.unit.as_str()).unwrap_or("");
            let ptype = meta_opt.map(|m| m.proto_type.as_str()).unwrap_or("string");
            let tooltip = meta_opt
                .map(Self::build_field_tooltip)
                .unwrap_or_else(|| leaf_name.clone());

            out.push('{');
            out.push_str("\"name\":");
            crate::dissector::escape_json_str(leaf_name, &mut out);
            out.push_str(",\"path\":");
            crate::dissector::escape_json_str(path, &mut out);
            out.push_str(",\"label\":");
            crate::dissector::escape_json_str(&label, &mut out);
            out.push_str(",\"description\":");
            crate::dissector::escape_json_str(description, &mut out);
            out.push_str(",\"unit\":");
            crate::dissector::escape_json_str(unit, &mut out);
            out.push_str(",\"proto_type\":");
            crate::dissector::escape_json_str(ptype, &mut out);
            out.push_str(",\"tooltip\":");
            crate::dissector::escape_json_str(&tooltip, &mut out);
            out.push('}');
        }

        // 2. Also output leaf entries for flat fallback
        for meta in self.entries.values() {
            if !first {
                out.push(',');
            }
            first = false;
            let name = &meta.name;
            let label = if !meta.description.is_empty() && &meta.description != name {
                format!("{} ({})", meta.description, name)
            } else {
                name.clone()
            };
            let tooltip = Self::build_field_tooltip(meta);

            out.push('{');
            out.push_str("\"name\":");
            crate::dissector::escape_json_str(name, &mut out);
            out.push_str(",\"path\":");
            crate::dissector::escape_json_str(name, &mut out);
            out.push_str(",\"label\":");
            crate::dissector::escape_json_str(&label, &mut out);
            out.push_str(",\"description\":");
            crate::dissector::escape_json_str(&meta.description, &mut out);
            out.push_str(",\"unit\":");
            crate::dissector::escape_json_str(&meta.unit, &mut out);
            out.push_str(",\"proto_type\":");
            crate::dissector::escape_json_str(&meta.proto_type, &mut out);
            out.push_str(",\"tooltip\":");
            crate::dissector::escape_json_str(&tooltip, &mut out);
            out.push('}');
        }
        out.push(']');
        out
    }
}

/// Recursively computes hierarchical dot-separated paths for all nested child elements.
fn expand_hierarchy(
    curr_path: &str,
    curr_key: &str,
    containment: &HashMap<String, Vec<(String, String)>>,
    element_to_type: &HashMap<String, String>,
    out: &mut Vec<(String, String)>,
    path_set: &mut HashSet<String>,
    depth: usize,
) {
    if depth > 12 || path_set.contains(curr_key) {
        return;
    }
    path_set.insert(curr_key.to_string());

    let mut children = containment.get(curr_key).cloned().unwrap_or_default();
    if children.is_empty() {
        if let Some(type_name) = element_to_type.get(&curr_key.to_lowercase()) {
            if let Some(ch) = containment.get(type_name) {
                children = ch.clone();
            }
        }
    }

    for (child_name, child_type) in children {
        let is_group = child_name.ends_with("Group") || child_type.ends_with("Group");
        let child_path = if is_group {
            curr_path.to_string()
        } else {
            format!("{curr_path}.{child_name}")
        };
        if !is_group {
            out.push((child_path.clone(), child_name.clone()));
        }

        let next_key = if !child_type.is_empty() {
            child_type
        } else {
            child_name
        };
        expand_hierarchy(
            &child_path,
            &next_key,
            containment,
            element_to_type,
            out,
            path_set,
            depth.saturating_add(1),
        );
    }

    path_set.remove(curr_key);
}

/// Infers Wireshark ProtoField type based on XML Schema type and scale factor.
fn infer_proto_type(type_name: &str, scale_factor: f64) -> String {
    let lower = type_name.to_lowercase();
    if (scale_factor - 1.0).abs() >= f64::EPSILON
        || lower.contains("double")
        || lower.contains("float")
        || lower.contains("decimal")
    {
        "double".to_string()
    } else if lower.contains("unsigned") || lower.contains("nonnegative") {
        "uint32".to_string()
    } else if lower.contains("int") || lower.contains("short") || lower.contains("byte") || lower.contains("integer") {
        "int32".to_string()
    } else if lower.contains("bool") {
        "bool".to_string()
    } else {
        "string".to_string()
    }
}

/// Strips namespace prefix from type name (e.g. `ast:TimeOfDayType` -> `TimeOfDayType`).
fn clean_type_name(raw: &str) -> String {
    if let Some((_, local)) = raw.split_once(':') {
        local.to_string()
    } else {
        raw.to_string()
    }
}

/// Evaluates a scale factor from resolution string or documentation text generically.
///
/// Parses standard mathematical expressions without any hardcoded field names:
/// - Fractional form: `1/128`, `360/65536`, `1/256`, `360/256`, `180/8388608`
/// - Power-of-two forms: `2^-14`, `2^(-14)`
/// - Complex fractions with powers: `360 / 2^16`, `360 / 2^13`, `180 / 2^23`
/// - Decimal forms: `0.25`, `25`, `6.25`, `1.0`
#[must_use]
pub fn parse_generic_scale(resolution_str: &str, documentation: &str) -> f64 {
    let candidate = if !resolution_str.trim().is_empty() {
        resolution_str.trim()
    } else if let Some(extracted) = extract_scale_from_doc(documentation) {
        return extracted;
    } else {
        return 1.0;
    };

    eval_math_expression(candidate).unwrap_or(1.0)
}

/// Tries to evaluate a mathematical scale expression (e.g. `360 / 2^16`, `1/128`, `2^-14`).
fn eval_math_expression(expr: &str) -> Option<f64> {
    let clean = expr.trim();

    // Case 1: Simple decimal or integer
    if let Ok(v) = clean.parse::<f64>() {
        return Some(v);
    }

    // Case 2: Pure power of two `2^-N` or `2^(-N)`
    if clean.starts_with("2^") || clean.starts_with("2^-") {
        let pwr_part = clean.trim_start_matches("2^").trim_matches(['(', ')']);
        if let Ok(exp) = pwr_part.parse::<f64>() {
            return Some((2.0f64).powf(exp));
        }
    }

    // Case 3: Division `numerator / denominator`
    if clean.contains('/') {
        let mut parts = clean.splitn(2, '/');
        let num_str = parts.next()?.trim();
        let den_str = parts.next()?.trim();

        let num = eval_sub_expr(num_str)?;
        let den = eval_sub_expr(den_str)?;

        if den.abs() > f64::EPSILON {
            return Some(num / den);
        }
    }

    None
}

/// Evaluates a numerator or denominator term that may include powers (e.g. `2^16`, `2^13`, `65536`).
fn eval_sub_expr(term: &str) -> Option<f64> {
    let clean = term.trim();
    if clean.contains('^') {
        let mut p = clean.splitn(2, '^');
        let base = p.next()?.trim().parse::<f64>().ok()?;
        let exp = p.next()?.trim().trim_matches(['(', ')']).parse::<f64>().ok()?;
        Some(base.powf(exp))
    } else {
        clean.parse::<f64>().ok()
    }
}

/// Extracts scale factor from documentation text searching for `LSB = <expr>` or `LSB: <expr>`.
fn extract_scale_from_doc(doc: &str) -> Option<f64> {
    for line in doc.lines() {
        if let Some(pos) = line.find("LSB") {
            let after = line.get(pos.saturating_add(3)..)?.trim_start_matches([':', '=', ' ']);
            // Take the first token representing the scale
            let token = after
                .split(|c: char| c.is_whitespace() || c == '(' || c == ';' || c == ',')
                .next()?;
            if let Some(scale) = eval_math_expression(token) {
                return Some(scale);
            }
        }
    }
    None
}

/// Extracts enumeration mappings from documentation strings (e.g. `(0=Maintaining, 1=Climbing)` or `- 1: North Marker Message`).
pub fn parse_enums_from_text(text: &str, out: &mut HashMap<i64, String>) {
    if text.is_empty() {
        return;
    }

    // Pattern 1: `(0=Maintaining, 1=Climbing, 2=Descending, 3=Unknown)` or `(0 = Actual, 1 = Simulated)`
    let mut search_segments = Vec::new();
    let mut in_paren = false;
    let mut paren_start = 0;
    for (i, c) in text.char_indices() {
        if c == '(' {
            in_paren = true;
            paren_start = i.saturating_add(1);
        } else if c == ')' && in_paren {
            in_paren = false;
            if i > paren_start {
                search_segments.push(&text[paren_start..i]);
            }
        }
    }

    if search_segments.is_empty() {
        search_segments.push(text);
    }

    for segment in search_segments {
        for part in segment.split([',', ';']) {
            let trimmed = part.trim();
            if let Some((lhs, rhs)) = trimmed.split_once('=') {
                let lhs = lhs.trim();
                let rhs = rhs.trim().trim_end_matches('.').trim_matches('"');
                if let Ok(num) = lhs.parse::<i64>() {
                    if !rhs.is_empty() && !out.contains_key(&num) {
                        out.insert(num, clean_enum_label(rhs));
                    }
                } else if lhs.len() >= 2 && lhs.chars().all(|c| c == '0' || c == '1') {
                    if let Ok(num) = i64::from_str_radix(lhs, 2) {
                        if !rhs.is_empty() && !out.contains_key(&num) {
                            out.insert(num, clean_enum_label(rhs));
                        }
                    }
                }
            }
        }
    }

    // Pattern 2: Bullet points like `- 1: North Marker Message` or `1: North Marker Message`
    for line in text.lines() {
        let clean_line = line.trim().trim_start_matches(['-', '*', '•']).trim();
        if let Some((lhs, rhs)) = clean_line.split_once(':') {
            let lhs = lhs.trim();
            let rhs = rhs.trim().trim_end_matches('.').trim_matches('"');
            if let Ok(num) = lhs.parse::<i64>() {
                if !rhs.is_empty() && !out.contains_key(&num) && rhs.len() < 80 {
                    let label = clean_enum_label(rhs);
                    out.insert(num, label);
                }
            }
        }
    }

    // Pattern 3: `surveillance chain (0) vs RDP (1)` or `target report (0) vs reply (1)`
    if out.is_empty() && text.contains(" vs ") {
        for part in text.split(" vs ") {
            let trimmed = part.trim();
            if let Some(open) = trimmed.rfind('(') {
                if let Some(close) = trimmed.rfind(')') {
                    if close > open {
                        let num_str = trimmed[open.saturating_add(1)..close].trim();
                        if let Ok(num) = num_str.parse::<i64>() {
                            let mut label = trimmed[..open].trim();
                            for prep in &["Report from ", "Target Report from ", "from ", "Provided by ", "by "] {
                                if let Some(stripped) = label.strip_prefix(prep) {
                                    label = stripped.trim();
                                }
                            }
                            if !label.is_empty() && !out.contains_key(&num) {
                                out.insert(num, clean_enum_label(label));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Cleans an extracted enum label.
fn clean_enum_label(s: &str) -> String {
    let mut label = s.trim();
    if let Some((main, _)) = label.split_once(" / ") {
        label = main.trim();
    }
    label = label.trim_matches('"').trim_matches('\'').trim();
    label.to_string()
}

/// Extracts a concise description from documentation text if none is explicitly provided.
#[must_use]
pub fn extract_concise_description(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }

    let first_line = doc.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return String::new();
    }

    let title_part = if let Some((_, after_colon)) = first_line.split_once(':') {
        after_colon.trim()
    } else {
        first_line
    };

    let title_clean = title_part.split(['.', ';']).next().unwrap_or("").trim();
    if title_clean.len() >= 3 && title_clean.len() <= 60 {
        title_clean.to_string()
    } else {
        String::new()
    }
}

