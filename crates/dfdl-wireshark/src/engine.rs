//! Generic DFDL Dissector Engine.
//!
//! Compiles any DFDL XML schema into an in-memory execution plan and introspects its
//! elements, simpleTypes, units, and documentation. Decodes arbitrary network payloads
//! into structured Wireshark protocol dissection trees.
//!
//! This engine is completely generic and contains zero hardcoded protocol rules.

use crate::dissector::{AnomalySeverity, DissectionAnomaly, DissectionNode, DissectionTree};
use crate::schema_meta::SchemaMetadataRegistry;
use dfdl_core::error::DFDLResult;
use dfdl_core::io::bitstream::BitReader;
use dfdl_core::io::source::SliceByteSource;
use dfdl_core::io::traits::{BitOrder, ByteOrder};
use dfdl_core::kernel::ParserEngine;
use dfdl_core::limits::WorkBudget;
use dfdl_core::schema::ir::CompiledSchema;
use dfdl_schema::SchemaCompiler;

/// Generic dissector engine driven strictly by DFDL XML Schemas.
#[derive(Debug, Clone)]
pub struct DfdlDissectorEngine {
    /// Raw schema XML definition string.
    schema_xml: String,
    /// In-memory compiled schema graph.
    compiled_schema: CompiledSchema,
    /// Schema metadata registry discovered from element annotations.
    registry: SchemaMetadataRegistry,
    /// Target root element name, if specified.
    target_root: Option<String>,
}

impl DfdlDissectorEngine {
    /// Creates a new generic dissector engine by compiling a DFDL schema XML string.
    ///
    /// # Errors
    /// Returns [`dfdl_core::error::DFDLError`] if the schema cannot be compiled.
    pub fn from_schema_str(schema_xml: &str, target_root: Option<&str>) -> DFDLResult<Self> {
        let compiler = SchemaCompiler::new();
        let compiled_schema = compiler.compile_str_with_resolver_and_root(
            schema_xml,
            |_| None,
            target_root,
        )?;
        let registry = SchemaMetadataRegistry::from_schema_xml(schema_xml);

        Ok(Self {
            schema_xml: schema_xml.to_string(),
            compiled_schema,
            registry,
            target_root: target_root.map(ToString::to_string),
        })
    }

    /// Creates a new generic dissector engine by reading and compiling a schema file from disk.
    ///
    /// # Errors
    /// Returns a formatted error string if reading or compiling fails.
    pub fn from_schema_file(
        path: impl AsRef<std::path::Path>,
        target_root: Option<&str>,
    ) -> Result<Self, String> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read schema file: {e}"))?;
        Self::from_schema_str(&content, target_root)
            .map_err(|e| format!("Failed to compile DFDL schema: {e}"))
    }

    /// Returns a reference to the discovered schema metadata registry.
    #[must_use]
    pub fn registry(&self) -> &SchemaMetadataRegistry {
        &self.registry
    }

    /// Returns the raw schema XML string.
    #[must_use]
    pub fn schema_xml(&self) -> &str {
        &self.schema_xml
    }

    /// Returns the active protocol display name inferred from the schema.
    #[must_use]
    pub fn protocol_name(&self) -> &str {
        if !self.registry.protocol_name.is_empty() {
            &self.registry.protocol_name
        } else if let Some(ref r) = self.target_root {
            r.as_str()
        } else {
            "DFDL Protocol"
        }
    }

    /// Dissects a binary network payload into a hierarchical [`DissectionTree`].
    ///
    /// Runs the DFDL ParserEngine against the raw bytes, converting parsed elements
    /// into rich Wireshark tree nodes decorated with units and documentation.
    ///
    /// # Errors
    /// Returns [`dfdl_core::error::DFDLError`] if parsing fails.
    pub fn dissect_payload(&self, data: &[u8]) -> DFDLResult<DissectionTree> {
        if data.is_empty() {
            return Ok(DissectionTree {
                protocol_name: self.protocol_name().to_string(),
                root: None,
                anomalies: Vec::new(),
            });
        }

        let src = SliceByteSource::new(data);
        let mut reader = BitReader::new(
            src,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(2_000_000);

        let mut documents = Vec::new();
        let mut root_anomalies = Vec::new();

        while !reader.is_eof() && reader.remaining_bytes() > 0 {
            let prev_pos = reader.position();
            let mut parser = ParserEngine::new(&self.compiled_schema, &mut reader, &mut budget);

            match parser.parse_document() {
                Ok(doc) => {
                    documents.push(doc);
                    if reader.position() == prev_pos {
                        // Prevent potential infinite loop if zero bits consumed
                        break;
                    }
                }
                Err(e) => {
                    if documents.is_empty() {
                        return Err(e);
                    }
                    root_anomalies.push(DissectionAnomaly {
                        severity: AnomalySeverity::Error,
                        message: format!("Dissection truncated early: {e}"),
                    });
                    break;
                }
            }
        }

        // Enhancement 5: Detect leftover / residual trailing bytes in payload
        if reader.remaining_bytes() > 0 {
            let leftover = reader.remaining_bytes();
            root_anomalies.push(DissectionAnomaly {
                severity: AnomalySeverity::Warn,
                message: format!("Malformed packet: {leftover} unparsed trailing residual byte(s)"),
            });
        }

        if documents.len() == 1 {
            let doc = match documents.pop() {
                Some(d) => d,
                None => {
                    return Ok(DissectionTree {
                        protocol_name: self.protocol_name().to_string(),
                        root: None,
                        anomalies: root_anomalies,
                    })
                }
            };
            let mut tree = DissectionTree::from_infoset(
                &doc,
                &self.registry,
                self.protocol_name(),
            );
            if !root_anomalies.is_empty() {
                if let Some(ref mut r) = tree.root {
                    r.anomalies.extend(root_anomalies.clone());
                }
                tree.anomalies.extend(root_anomalies);
            }
            return Ok(tree);
        }

        // Multiple top-level records or messages found in payload
        let mut root_children = Vec::new();
        for doc in &documents {
            if let Some(ref r) = doc.root {
                root_children.push(DissectionNode::from_element(r, &self.registry));
            }
        }

        let total_offset = root_children.first().map_or(0, |c| c.offset);
        let total_length = root_children
            .iter()
            .map(|c| c.offset.saturating_add(c.length))
            .max()
            .unwrap_or(0)
            .saturating_sub(total_offset);
        let container_name = format!("{}_Block", self.protocol_name());
        let root_node = DissectionNode {
            name: container_name.clone(),
            description: String::new(),
            label: format!("{} ({} items)", self.protocol_name(), root_children.len()),
            raw_value: String::new(),
            converted_value: String::new(),
            unit: String::new(),
            resolution: String::new(),
            documentation: format!("Container of {} decoded DFDL messages", root_children.len()),
            offset: total_offset,
            length: total_length,
            bit_offset: total_offset.saturating_mul(8),
            bit_length: total_length.saturating_mul(8),
            bitmask: 0,
            bitmask_dots: String::new(),
            anomalies: root_anomalies.clone(),
            children: root_children,
        };

        Ok(DissectionTree {
            protocol_name: self.protocol_name().to_string(),
            root: Some(root_node),
            anomalies: root_anomalies,
        })
    }

    /// Dissects a raw byte slice and serializes the result directly into JSON.
    ///
    /// # Errors
    /// Returns a formatted error string if dissection fails.
    pub fn dissect_payload_to_json(&self, data: &[u8]) -> Result<String, String> {
        let tree = self
            .dissect_payload(data)
            .map_err(|e| format!("Dissection error: {e}"))?;
        Ok(tree.to_json())
    }
}
