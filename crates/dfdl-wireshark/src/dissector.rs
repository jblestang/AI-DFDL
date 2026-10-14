//! Hierarchical Wireshark Dissector Tree construction and rendering.
//!
//! Transforms the raw DFDL Infoset document into an enriched Wireshark dissection
//! tree where every node carries its label, raw value, converted physical value,
//! measurement unit, and schema documentation tooltip.

use crate::converter::{convert_value, ConvertedValue};
use crate::schema_meta::SchemaMetadataRegistry;
use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};

/// Severity level of a Wireshark dissection anomaly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalySeverity {
    /// Informational or protocol events.
    Chat,
    /// Noteworthy protocol state or operational condition.
    Note,
    /// Protocol warning or unexpected range (e.g. out of schema min/max bounds).
    Warn,
    /// Critical protocol error or anomaly (e.g. schema constraint violation, malformed trailing bytes).
    Error,
}

impl AnomalySeverity {
    /// Returns the Wireshark-compatible string representation of the severity.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Note => "Note",
            Self::Warn => "Warn",
            Self::Error => "Error",
        }
    }
}

/// A protocol anomaly, constraint violation, or operational event flagged on a dissection node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DissectionAnomaly {
    /// Severity level of the anomaly.
    pub severity: AnomalySeverity,
    /// Human-readable message explaining the anomaly.
    pub message: String,
}

/// A single node in the Wireshark protocol dissection tree.
#[derive(Debug, Clone, PartialEq)]
pub struct DissectionNode {
    /// XML element name (e.g. `timeOfDay`, `rho`, `record`).
    pub name: String,
    /// Hierarchical dot-separated display filter path (e.g. `Cat048Record.record.targetDescriptor.typ`).
    pub path: String,
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
    /// Byte offset in the raw packet data.
    pub offset: usize,
    /// Byte length in the raw packet data.
    pub length: usize,
    /// Bit offset in the raw packet data.
    pub bit_offset: usize,
    /// Bit length in the raw packet data.
    pub bit_length: usize,
    /// Sub-byte bitmask within the enclosing byte (e.g. `0x04`, `0x80`), or 0 if byte-aligned.
    pub bitmask: u32,
    /// Formatted dotted bit breakdown (e.g. `.... .1..` or `.010 ....`).
    pub bitmask_dots: String,
    /// Detected protocol anomalies, range violations, or operational alerts.
    pub anomalies: Vec<DissectionAnomaly>,
    /// Nested child fields or sub-records.
    pub children: Vec<DissectionNode>,
}

impl DissectionNode {
    /// Creates a leaf or parent dissection node from an [`InfosetElement`].
    #[must_use]
    pub fn from_element(elem: &InfosetElement, registry: &SchemaMetadataRegistry) -> Self {
        Self::from_element_with_parent(elem, registry, None)
    }

    /// Recursively creates a dissection node while tracking the hierarchical record-level path.
    fn from_element_with_parent(
        elem: &InfosetElement,
        registry: &SchemaMetadataRegistry,
        parent_path: Option<&str>,
    ) -> Self {
        let name = elem.name.local_name.clone();
        let meta_opt = registry.get(&name);

        let is_rec = registry.is_record_element(&name);
        let path = if is_rec {
            name.clone()
        } else if let Some(p) = parent_path {
            format!("{p}.{name}")
        } else {
            name.clone()
        };

        let child_parent_path = if is_rec || parent_path.is_some() {
            Some(path.as_str())
        } else {
            None
        };

        let mut raw_val = String::new();
        let mut children = Vec::new();

        match &elem.state {
            ElementState::Value(val) => {
                raw_val = val.to_string();
            }
            ElementState::NoValue | ElementState::Empty => {
                for child_node in &elem.children {
                    let InfosetNode::Element(ref child_elem) = child_node;
                    children.push(Self::from_element_with_parent(child_elem, registry, child_parent_path));
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

        let (bit_offset, bit_length) = elem.location.unwrap_or((0, 0));
        let offset = bit_offset / 8;
        let length = if bit_length > 0 {
            let end_bit = bit_offset.saturating_add(bit_length);
            let end_byte = end_bit.div_ceil(8);
            end_byte.saturating_sub(offset)
        } else {
            0
        };

        // Enhancement 6: Sub-Byte Bitfield Dotted Mask Breakdown (e.g. ".... .1..")
        let mut bitmask: u32 = 0;
        let mut bitmask_dots = String::new();
        if bit_length > 0 && bit_length < 8 {
            let bit_in_byte = bit_offset % 8;
            if bit_in_byte.saturating_add(bit_length) <= 8 {
                let shift = 8usize.saturating_sub(bit_in_byte.saturating_add(bit_length));
                let ones = (1u32.checked_shl(bit_length as u32).unwrap_or(0)).saturating_sub(1);
                bitmask = ones.checked_shl(shift as u32).unwrap_or(0);

                let val_int = raw_val.trim().parse::<u64>().unwrap_or(0);
                let mut dots = String::with_capacity(10);
                for i in 0..8 {
                    if i == 4 {
                        dots.push(' ');
                    }
                    if i >= bit_in_byte && i < bit_in_byte.saturating_add(bit_length) {
                        let bit_idx_in_field = i.saturating_sub(bit_in_byte);
                        let bit_weight = 1u64.checked_shl((bit_length.saturating_sub(1).saturating_sub(bit_idx_in_field)) as u32).unwrap_or(0);
                        if (val_int & bit_weight) != 0 {
                            dots.push('1');
                        } else {
                            dots.push('0');
                        }
                    } else {
                        dots.push('.');
                    }
                }
                bitmask_dots = dots;
            }
        }

        // Enhancement 5: Wireshark Expert Info & Anomaly Detection
        let mut anomalies = Vec::new();

        // 1. Schema constraint bounds validation (<xs:minInclusive>, <xs:maxInclusive>)
        if let Some(meta) = meta_opt {
            if let Ok(num) = raw_val.trim().parse::<f64>() {
                if let Some(min) = meta.min_inclusive {
                    if num < min {
                        anomalies.push(DissectionAnomaly {
                            severity: AnomalySeverity::Warn,
                            message: format!("Field '{name}' value {num} is below schema minInclusive ({min})"),
                        });
                    }
                }
                if let Some(max) = meta.max_inclusive {
                    if num > max {
                        anomalies.push(DissectionAnomaly {
                            severity: AnomalySeverity::Warn,
                            message: format!("Field '{name}' value {num} exceeds schema maxInclusive ({max})"),
                        });
                    }
                }
            }
        }

        // 2. Schema-driven enumeration alert and out-of-bounds validation
        if let Some(meta) = meta_opt {
            let parsed_int = raw_val.trim().parse::<i64>().ok().or_else(|| {
                let t = raw_val.trim();
                if let Some(hex_str) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    i64::from_str_radix(hex_str, 16).ok()
                } else {
                    None
                }
            });

            if let Some(num) = parsed_int {
                if let Some(label) = meta.get_enum_label(num) {
                    let lower = label.to_ascii_lowercase();
                    if lower.contains("emergency")
                        || lower.contains("hijack")
                        || lower.contains("failure")
                        || lower.contains("alarm")
                        || lower.contains("fatal")
                    {
                        anomalies.push(DissectionAnomaly {
                            severity: AnomalySeverity::Error,
                            message: format!("{name}: {label}"),
                        });
                    } else if lower.contains("warn")
                        || lower.contains("alert")
                        || lower.contains("fault")
                        || lower.contains("invalid")
                    {
                        anomalies.push(DissectionAnomaly {
                            severity: AnomalySeverity::Warn,
                            message: format!("{name}: {label}"),
                        });
                    }
                } else if !meta.enum_map.is_empty() {
                    anomalies.push(DissectionAnomaly {
                        severity: AnomalySeverity::Warn,
                        message: format!("Field '{name}' value {num} is outside expected enum values"),
                    });
                }
            }
        }

        // If this is a parent node whose length is 0, span the bounding box of its children
        let mut final_offset = offset;
        let mut final_length = length;
        if final_length == 0 && !children.is_empty() {
            let valid_children: Vec<_> = children.iter().filter(|c| c.length > 0).collect();
            if let Some(first) = valid_children.first() {
                let start = valid_children.iter().map(|c| c.offset).min().unwrap_or(first.offset);
                let end = valid_children
                    .iter()
                    .map(|c| c.offset.saturating_add(c.length))
                    .max()
                    .unwrap_or_else(|| first.offset.saturating_add(first.length));
                final_offset = start;
                final_length = end.saturating_sub(start);
            }
        }

        Self {
            name,
            path,
            description,
            label,
            raw_value: raw,
            converted_value: formatted,
            unit,
            resolution,
            documentation: tooltip,
            offset: final_offset,
            length: final_length,
            bit_offset,
            bit_length,
            bitmask,
            bitmask_dots,
            anomalies,
            children,
        }
    }

    /// Renders the node and its children as Wireshark-like indented ASCII text.
    pub fn render_ascii(&self, indent_level: usize, out: &mut String) {
        let indent = "  ".repeat(indent_level);
        let display_label = if !self.bitmask_dots.is_empty() {
            format!("{} = {}", self.bitmask_dots, self.label)
        } else {
            self.label.clone()
        };
        if self.children.is_empty() {
            out.push_str(&format!("{indent}- {display_label}\n"));
        } else {
            out.push_str(&format!("{indent}+ {display_label}\n"));
            for child in &self.children {
                child.render_ascii(match indent_level.checked_add(1) {
                    Some(lvl) => lvl,
                    None => indent_level,
                }, out);
            }
        }
        for an in &self.anomalies {
            out.push_str(&format!("{indent}  [!] [{}] {}\n", an.severity.as_str(), an.message));
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
        out.push_str(",\"path\":");
        escape_json_str(&self.path, out);
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
        out.push_str(",\"offset\":");
        out.push_str(&self.offset.to_string());
        out.push_str(",\"length\":");
        out.push_str(&self.length.to_string());
        out.push_str(",\"bit_offset\":");
        out.push_str(&self.bit_offset.to_string());
        out.push_str(",\"bit_length\":");
        out.push_str(&self.bit_length.to_string());
        out.push_str(",\"bitmask\":");
        out.push_str(&self.bitmask.to_string());
        out.push_str(",\"bitmask_dots\":");
        escape_json_str(&self.bitmask_dots, out);
        out.push_str(",\"anomalies\":[");
        for (i, an) in self.anomalies.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"severity\":\"");
            out.push_str(an.severity.as_str());
            out.push_str("\",\"message\":");
            escape_json_str(&an.message, out);
            out.push('}');
        }
        out.push(']');
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
    /// Root-level anomalies (e.g. leftover unparsed bytes, malformed packet tail).
    pub anomalies: Vec<DissectionAnomaly>,
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
            anomalies: Vec::new(),
        }
    }

    /// Generates a concise, informative 1-line summary of the packet for Wireshark's Info column.
    #[must_use]
    pub fn summary(&self) -> String {
        let Some(ref root) = self.root else {
            return self.protocol_name.clone();
        };

        // Find records: either direct children of root, or root itself if it has no children
        let records: Vec<&DissectionNode> = if !root.children.is_empty() {
            root.children.iter().collect()
        } else {
            vec![root]
        };

        if records.is_empty() {
            return self.protocol_name.clone();
        }

        let mut record_summaries = Vec::new();
        for rec in &records {
            let mut highlights = Vec::new();

            // Recursively search for up to 4 prominent leaf fields within this record
            fn find_highlights(node: &DissectionNode, highlights: &mut Vec<String>) {
                if highlights.len() >= 4 {
                    return;
                }
                if node.children.is_empty() {
                    let display_val = if !node.converted_value.is_empty() {
                        node.converted_value.trim()
                    } else {
                        node.raw_value.trim()
                    };
                    if !display_val.is_empty() {
                        let lower = node.name.to_ascii_lowercase();
                        // Prioritize salient identifiers, types, status or numbers
                        let is_salient = lower.contains("id")
                            || lower.contains("type")
                            || lower.contains("code")
                            || lower.contains("status")
                            || lower.contains("num")
                            || lower.contains("addr")
                            || lower.contains("time")
                            || lower.contains("cat");
                        if is_salient {
                            let entry = format!("{}: {}", node.name, display_val);
                            if !highlights.contains(&entry) {
                                highlights.push(entry);
                            }
                        }
                    }
                }
                for child in &node.children {
                    find_highlights(child, highlights);
                }
            }

            find_highlights(rec, &mut highlights);

            // If no salient fields were found, fallback to the first few leaf fields
            if highlights.is_empty() {
                fn find_first_leaves(node: &DissectionNode, leaves: &mut Vec<String>) {
                    if leaves.len() >= 3 {
                        return;
                    }
                    if node.children.is_empty() {
                        let val = if !node.converted_value.is_empty() {
                            node.converted_value.trim()
                        } else {
                            node.raw_value.trim()
                        };
                        if !val.is_empty() {
                            leaves.push(format!("{}: {}", node.name, val));
                        }
                    }
                    for child in &node.children {
                        find_first_leaves(child, leaves);
                    }
                }
                find_first_leaves(rec, &mut highlights);
            }

            let prefix = if !rec.description.is_empty() && rec.description != rec.name {
                rec.description.clone()
            } else {
                rec.name.clone()
            };

            if highlights.is_empty() {
                record_summaries.push(prefix);
            } else {
                record_summaries.push(format!("{} [{}]", prefix, highlights.join(", ")));
            }
        }

        if record_summaries.is_empty() {
            format!("{}: {} records", self.protocol_name, records.len())
        } else {
            record_summaries.join(", ")
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
        let summary = self.summary();
        out.push_str(",\"summary\":");
        escape_json_str(&summary, &mut out);
        out.push_str(",\"anomalies\":[");
        for (i, an) in self.anomalies.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"severity\":\"");
            out.push_str(an.severity.as_str());
            out.push_str("\",\"message\":");
            escape_json_str(&an.message, &mut out);
            out.push('}');
        }
        out.push(']');
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
pub fn escape_json_str(input: &str, out: &mut String) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_meta::FieldMetadata;
    use dfdl_core::infoset::state::ElementState;
    use dfdl_core::infoset::tree::InfosetElement;
    use dfdl_core::infoset::value::DfdlValue;
    use dfdl_core::types::QName;

    #[test]
    fn test_sub_byte_bitmask_and_dots_typ() {
        let registry = SchemaMetadataRegistry::new();
        let mut elem = InfosetElement::simple(QName::local("typ"), ElementState::Value(DfdlValue::String("5".into())));
        // bit_offset = 0, bit_length = 3
        elem.location = Some((0, 3));

        let node = DissectionNode::from_element(&elem, &registry);
        assert_eq!(node.bitmask, 0xE0);
        assert_eq!(node.bitmask_dots, "101. ....");
    }

    #[test]
    fn test_sub_byte_bitmask_and_dots_flags() {
        let registry = SchemaMetadataRegistry::new();

        // SIM flag: bit_offset = 3, bit_length = 1, val = 0
        let mut sim_elem = InfosetElement::simple(QName::local("sim"), ElementState::Value(DfdlValue::String("0".into())));
        sim_elem.location = Some((3, 1));
        let sim_node = DissectionNode::from_element(&sim_elem, &registry);
        assert_eq!(sim_node.bitmask, 0x10);
        assert_eq!(sim_node.bitmask_dots, "...0 ....");

        // SPI flag: bit_offset = 5, bit_length = 1, val = 1
        let mut spi_elem = InfosetElement::simple(QName::local("spi"), ElementState::Value(DfdlValue::String("1".into())));
        spi_elem.location = Some((5, 1));
        let spi_node = DissectionNode::from_element(&spi_elem, &registry);
        assert_eq!(spi_node.bitmask, 0x04);
        assert_eq!(spi_node.bitmask_dots, ".... .1..");
    }

    #[test]
    fn test_schema_bounds_anomaly_detection() {
        let mut registry = SchemaMetadataRegistry::new();
        let mut meta = FieldMetadata::new("flightLevel");
        meta.min_inclusive = Some(0.0);
        meta.max_inclusive = Some(600.0);
        registry.insert(meta);

        // Within bounds
        let elem_ok = InfosetElement::simple(QName::local("flightLevel"), ElementState::Value(DfdlValue::String("330".into())));
        let node_ok = DissectionNode::from_element(&elem_ok, &registry);
        assert!(node_ok.anomalies.is_empty());

        // Above maxInclusive
        let elem_high = InfosetElement::simple(QName::local("flightLevel"), ElementState::Value(DfdlValue::String("750".into())));
        let node_high = DissectionNode::from_element(&elem_high, &registry);
        assert_eq!(node_high.anomalies.len(), 1);
        if let Some(an_high) = node_high.anomalies.first() {
            assert_eq!(an_high.severity, AnomalySeverity::Warn);
            assert!(an_high.message.contains("exceeds schema maxInclusive"));
        }

        // Below minInclusive
        let elem_low = InfosetElement::simple(QName::local("flightLevel"), ElementState::Value(DfdlValue::String("-10".into())));
        let node_low = DissectionNode::from_element(&elem_low, &registry);
        assert_eq!(node_low.anomalies.len(), 1);
        if let Some(an_low) = node_low.anomalies.first() {
            assert_eq!(an_low.severity, AnomalySeverity::Warn);
            assert!(an_low.message.contains("below schema minInclusive"));
        }
    }

    #[test]
    fn test_schema_driven_enum_anomalies() {
        let mut registry = SchemaMetadataRegistry::new();
        let mut meta = FieldMetadata::new("status");
        meta.enum_map.insert(0, "Nominal".to_string());
        meta.enum_map.insert(1, "Subsystem Warning Alert".to_string());
        meta.enum_map.insert(2, "Critical Failure Emergency".to_string());
        registry.insert(meta);

        // Nominal
        let elem_norm = InfosetElement::simple(QName::local("status"), ElementState::Value(DfdlValue::String("0".into())));
        let node_norm = DissectionNode::from_element(&elem_norm, &registry);
        assert!(node_norm.anomalies.is_empty());

        // Warning Alert
        let elem_warn = InfosetElement::simple(QName::local("status"), ElementState::Value(DfdlValue::String("1".into())));
        let node_warn = DissectionNode::from_element(&elem_warn, &registry);
        assert_eq!(node_warn.anomalies.len(), 1);
        if let Some(an_warn) = node_warn.anomalies.first() {
            assert_eq!(an_warn.severity, AnomalySeverity::Warn);
            assert!(an_warn.message.contains("Subsystem Warning Alert"));
        }

        // Critical Failure Emergency
        let elem_crit = InfosetElement::simple(QName::local("status"), ElementState::Value(DfdlValue::String("2".into())));
        let node_crit = DissectionNode::from_element(&elem_crit, &registry);
        assert_eq!(node_crit.anomalies.len(), 1);
        if let Some(an_crit) = node_crit.anomalies.first() {
            assert_eq!(an_crit.severity, AnomalySeverity::Error);
            assert!(an_crit.message.contains("Critical Failure Emergency"));
        }

        // Out-of-bounds enum value (e.g. 99)
        let elem_unknown = InfosetElement::simple(QName::local("status"), ElementState::Value(DfdlValue::String("99".into())));
        let node_unknown = DissectionNode::from_element(&elem_unknown, &registry);
        assert_eq!(node_unknown.anomalies.len(), 1);
        if let Some(an_un) = node_unknown.anomalies.first() {
            assert_eq!(an_un.severity, AnomalySeverity::Warn);
            assert!(an_un.message.contains("outside expected enum values"));
        }
        assert!(node_unknown.converted_value.contains("Unknown (99)"));
    }

    #[test]
    fn test_tree_json_serialization_with_anomalies() {
        let tree = DissectionTree {
            protocol_name: "TestProto".to_string(),
            root: None,
            anomalies: vec![DissectionAnomaly {
                severity: AnomalySeverity::Error,
                message: "Malformed packet: 10 residual bytes".to_string(),
            }],
        };

        let json = tree.to_json();
        assert!(json.contains("\"anomalies\":[{\"severity\":\"Error\",\"message\":\"Malformed packet: 10 residual bytes\"}]"));
    }
}

