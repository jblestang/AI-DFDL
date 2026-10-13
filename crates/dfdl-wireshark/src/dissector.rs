//! Hierarchical Wireshark Dissector Tree construction and rendering.
//!
//! Transforms the raw DFDL Infoset document into an enriched Wireshark dissection
//! tree where every node carries its label, raw value, converted physical value,
//! measurement unit, and schema documentation tooltip.

use crate::converter::{convert_value, ConvertedValue};
use crate::schema_meta::SchemaMetadataRegistry;
use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};

/// A single node in the Wireshark protocol dissection tree.
#[derive(Debug, Clone, PartialEq)]
pub struct DissectionNode {
    /// XML element name (e.g. `timeOfDay`, `rho`, `record`).
    pub name: String,
    /// Human-readable field description from schema metadata.
    pub description: String,
    /// Full display title shown in the Wireshark tree pane.
    pub label: String,
    /// Raw unparsed value string.
    pub raw_value: String,
    /// Converted physical value with unit.
    pub converted_value: String,
    /// Measurement unit string (e.g., `NM`, `degrees`, `FL`, `seconds`).
    pub unit: String,
    /// Fractional or numeric resolution string.
    pub resolution: String,
    /// Detailed documentation tooltip.
    pub documentation: String,
    /// Nested child fields or sub-records.
    pub children: Vec<DissectionNode>,
}

impl DissectionNode {
    /// Creates a leaf or parent dissection node from an [`InfosetElement`].
    #[must_use]
    pub fn from_element(elem: &InfosetElement, registry: &SchemaMetadataRegistry) -> Self {
        let name = elem.name.local_name.clone();
        let meta_opt = registry.get(&name);

        let mut raw_val = String::new();
        let mut children = Vec::new();

        match &elem.state {
            ElementState::Value(val) => {
                raw_val = val.to_string();
            }
            ElementState::NoValue | ElementState::Empty => {
                for child_node in &elem.children {
                    let InfosetNode::Element(ref child_elem) = child_node;
                    children.push(Self::from_element(child_elem, registry));
                }
            }
            ElementState::Nil | ElementState::Absent => {}
        }

        let ConvertedValue {
            raw,
            formatted,
            unit,
            resolution,
            tooltip,
        } = convert_value(&name, &raw_val, meta_opt);

        let description = meta_opt
            .map(|m| m.description.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let label_prefix = if !description.is_empty() && description != name {
            format!("{description} ({name})")
        } else {
            name.clone()
        };

        let label = if children.is_empty() {
            if formatted.is_empty() {
                format!("{label_prefix}: {raw}")
            } else {
                format!("{label_prefix}: {formatted}")
            }
        } else {
            label_prefix
        };

        Self {
            name,
            description,
            label,
            raw_value: raw,
            converted_value: formatted,
            unit,
            resolution,
            documentation: tooltip,
            children,
        }
    }

    /// Renders the node and its children as Wireshark-like indented ASCII text.
    pub fn render_ascii(&self, indent_level: usize, out: &mut String) {
        let indent = "  ".repeat(indent_level);
        if self.children.is_empty() {
            out.push_str(&format!("{indent}- {}\n", self.label));
        } else {
            out.push_str(&format!("{indent}+ {}\n", self.label));
            for child in &self.children {
                child.render_ascii(match indent_level.checked_add(1) {
                    Some(lvl) => lvl,
                    None => indent_level,
                }, out);
            }
        }
    }

    /// Serializes the dissection node into standard Wireshark PDML XML.
    pub fn render_pdml(&self, indent_level: usize, out: &mut String) {
        let indent = "  ".repeat(indent_level);
        if self.children.is_empty() {
            out.push_str(&format!(
                "{indent}<field name=\"{}\" showname=\"",
                self.name
            ));
            escape_xml(&self.label, out);
            out.push_str("\" value=\"");
            escape_xml(&self.raw_value, out);
            out.push_str("\" show=\"");
            escape_xml(&self.converted_value, out);
            out.push('"');

            if !self.unit.is_empty() {
                out.push_str(" unit=\"");
                escape_xml(&self.unit, out);
                out.push('"');
            }
            if !self.documentation.is_empty() {
                out.push_str(" tooltip=\"");
                escape_xml(&self.documentation, out);
                out.push('"');
            }
            out.push_str("/>\n");
        } else {
            out.push_str(&format!(
                "{indent}<field name=\"{}\" showname=\"",
                self.name
            ));
            escape_xml(&self.label, out);
            out.push_str("\">\n");

            for child in &self.children {
                child.render_pdml(match indent_level.checked_add(1) {
                    Some(lvl) => lvl,
                    None => indent_level,
                }, out);
            }

            out.push_str(&indent);
            out.push_str("</field>\n");
        }
    }

    /// Serializes the node into a compact JSON string for Lua or FFI consumers.
    pub fn to_json(&self, out: &mut String) {
        out.push('{');
        out.push_str("\"name\":");
        escape_json_str(&self.name, out);
        out.push_str(",\"description\":");
        escape_json_str(&self.description, out);
        out.push_str(",\"label\":");
        escape_json_str(&self.label, out);
        out.push_str(",\"raw\":");
        escape_json_str(&self.raw_value, out);
        out.push_str(",\"converted\":");
        escape_json_str(&self.converted_value, out);
        out.push_str(",\"unit\":");
        escape_json_str(&self.unit, out);
        out.push_str(",\"resolution\":");
        escape_json_str(&self.resolution, out);
        out.push_str(",\"documentation\":");
        escape_json_str(&self.documentation, out);
        out.push_str(",\"children\":[");
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            child.to_json(out);
        }
        out.push_str("]}");
    }
}

/// A complete Wireshark protocol dissection tree for a packet or data block.
#[derive(Debug, Clone, PartialEq)]
pub struct DissectionTree {
    /// Protocol display name extracted from schema.
    pub protocol_name: String,
    /// Root node of the dissection hierarchy.
    pub root: Option<DissectionNode>,
}

impl DissectionTree {
    /// Builds a [`DissectionTree`] from a parsed [`InfosetDocument`] and schema metadata registry.
    #[must_use]
    pub fn from_infoset(
        doc: &InfosetDocument,
        registry: &SchemaMetadataRegistry,
        protocol_name: &str,
    ) -> Self {
        let root = doc.root.as_ref().map(|r| DissectionNode::from_element(r, registry));
        let proto = if protocol_name.is_empty() {
            root.as_ref().map_or("DFDL Protocol", |r| r.name.as_str()).to_string()
        } else {
            protocol_name.to_string()
        };
        Self {
            protocol_name: proto,
            root,
        }
    }

    /// Renders the tree as human-readable Wireshark packet details text.
    #[must_use]
    pub fn render_ascii(&self) -> String {
        let mut out = String::new();
        if let Some(ref r) = self.root {
            r.render_ascii(0, &mut out);
        }
        out
    }

    /// Renders the tree as standard Wireshark Packet Details Markup Language (PDML).
    #[must_use]
    pub fn render_pdml(&self, packet_num: usize) -> String {
        let mut out = format!(
            "<packet num=\"{packet_num}\">\n  <proto name=\"{}\" showname=\"{}\" pos=\"0\" size=\"0\">\n",
            self.protocol_name.to_ascii_lowercase(),
            self.protocol_name
        );
        if let Some(ref r) = self.root {
            r.render_pdml(2, &mut out);
        }
        out.push_str("  </proto>\n</packet>\n");
        out
    }

    /// Serializes the complete dissection tree into JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\"protocol\":");
        escape_json_str(&self.protocol_name, &mut out);
        out.push_str(",\"root\":");
        if let Some(ref r) = self.root {
            r.to_json(&mut out);
        } else {
            out.push_str("null");
        }
        out.push('}');
        out
    }
}

/// Escapes XML special characters for safe PDML generation.
fn escape_xml(input: &str, out: &mut String) {
    for c in input.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

/// Escapes JSON string characters.
fn escape_json_str(input: &str, out: &mut String) {
    out.push('"');
    for c in input.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
}
