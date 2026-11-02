//! Unified payload generation coordinator and exhaustive test matrix builder.
//!
//! Orchestrates valid, syntactically mutated, and semantically mutated payload generation,
//! and writes the resulting packets to PCAP captures.

use std::io::Write;

use dfdl_core::error::DFDLResult;
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::schema::ir::CompiledSchema;

use crate::pcap::{LinkType, NetworkConfig, PcapWriter, TransportProtocol};
use crate::prng::FastPrng;
use crate::schema_walk::SchemaAnalysis;
use crate::semantic::{SemanticAnomaly, SemanticAnomalyInjector};
use crate::syntax::{SyntaxAnomaly, SyntaxAnomalyInjector};
use crate::valid::{GenerationConfig, ValidPayloadBuilder};

/// Mode and classification of generated payload test cases.
#[derive(Debug, Clone, PartialEq)]
pub enum PayloadMode {
    /// Valid payload strictly conforming to DFDL schema syntax and semantics.
    Valid,
    /// Syntactically invalid payload (framing, length, or bitstream corruption).
    SyntaxAnomaly(SyntaxAnomaly),
    /// Semantically invalid payload (facet, enum, or dispatch key violation).
    SemanticAnomaly(SemanticAnomaly),
}

/// Generated test case payload with full provenance and metadata.
#[derive(Debug, Clone)]
pub struct GeneratedPayload {
    /// Distinct test identifier (e.g. "VALID_BRANCH_0" or "SEMANTIC_WRONG_ENUM_terminalState").
    pub name: String,
    /// Detailed diagnostic description of the payload.
    pub description: String,
    /// Operational mode and classification.
    pub mode: PayloadMode,
    /// Raw binary payload octets.
    pub bytes: Vec<u8>,
}

/// Unified DFDL payload generator.
pub struct PayloadGenerator<'a> {
    schema: &'a CompiledSchema,
    analysis: SchemaAnalysis,
    prng: FastPrng,
    pcap_writer: PcapWriter,
}

impl<'a> PayloadGenerator<'a> {
    /// Creates a new payload generator for the given compiled schema.
    pub fn new(schema: &'a CompiledSchema, seed: u64) -> Self {
        let analysis = SchemaAnalysis::analyze(schema);
        let prng = FastPrng::new(seed);
        let pcap_writer = PcapWriter::new(LinkType::Ethernet);
        Self {
            schema,
            analysis,
            prng,
            pcap_writer,
        }
    }

    /// Accesses schema analysis results.
    pub fn analysis(&self) -> &SchemaAnalysis {
        &self.analysis
    }

    /// Generates a single valid payload with default configuration.
    pub fn generate_valid(&mut self) -> DFDLResult<GeneratedPayload> {
        self.generate_valid_with_config(GenerationConfig::default(), "VALID_NOMINAL", "Valid nominal payload")
    }

    /// Generates a valid payload with specified configuration.
    pub fn generate_valid_with_config(
        &mut self,
        config: GenerationConfig,
        name: &str,
        description: &str,
    ) -> DFDLResult<GeneratedPayload> {
        let mut builder = ValidPayloadBuilder::new(self.schema, &self.analysis, self.prng, config);
        let bytes = builder.generate_bytes()?;
        Ok(GeneratedPayload {
            name: name.to_string(),
            description: description.to_string(),
            mode: PayloadMode::Valid,
            bytes,
        })
    }

    /// Generates a syntactically invalid payload by corrupting a valid base payload.
    pub fn generate_syntax_anomaly(&mut self, anomaly: SyntaxAnomaly) -> DFDLResult<GeneratedPayload> {
        let valid_payload = self.generate_valid()?;
        let corrupted_bytes = SyntaxAnomalyInjector::apply(&valid_payload.bytes, &anomaly, &mut self.prng);

        let desc = format!("Syntactic anomaly: {:?}", anomaly);
        Ok(GeneratedPayload {
            name: format!("SYNTAX_{:?}", anomaly),
            description: desc,
            mode: PayloadMode::SyntaxAnomaly(anomaly),
            bytes: corrupted_bytes,
        })
    }

    /// Generates a semantically invalid payload by mutating an Infoset DOM before serialization.
    pub fn generate_semantic_anomaly(&mut self, anomaly: SemanticAnomaly) -> DFDLResult<GeneratedPayload> {
        let mut builder = ValidPayloadBuilder::new(
            self.schema,
            &self.analysis,
            self.prng,
            GenerationConfig::default(),
        );
        let mut doc = builder.build_infoset()?;
        let _ = SemanticAnomalyInjector::apply(&mut doc, &anomaly);
        let bytes = builder.unparse_to_bytes(&doc)?;

        let desc = format!("Semantic anomaly: {:?}", anomaly);
        Ok(GeneratedPayload {
            name: format!("SEMANTIC_{:?}", anomaly),
            description: desc,
            mode: PayloadMode::SemanticAnomaly(anomaly),
            bytes,
        })
    }

    /// Generates an exhaustive test matrix covering:
    /// - Every choice branch
    /// - Range boundaries (min/max)
    /// - Syntactic corruptions (truncation, length mismatches, framing corruption, bit misalignments)
    /// - Semantic corruptions (invalid enums, out-of-range facets, unmatched dispatch keys)
    pub fn generate_exhaustive_suite(&mut self) -> DFDLResult<Vec<GeneratedPayload>> {
        let mut suite = Vec::new();
        let choice_branches = self.analysis.choice_branches.clone();
        let enum_fields = self.analysis.enum_fields.clone();
        let bounded_fields = self.analysis.bounded_fields.clone();

        // 1. Valid Payloads covering EVERY Choice Branch
        for branch in &choice_branches {
            let mut cfg = GenerationConfig::default();
            cfg.preferred_branch_index = Some(branch.branch_index);
            let name = format!("VALID_CHOICE_BRANCH_{}", branch.branch_index);
            let desc = format!(
                "Valid payload exercising choice branch {} (key: {:?})",
                branch.branch_index, branch.branch_key
            );
            if let Ok(payload) = self.generate_valid_with_config(cfg, &name, &desc) {
                suite.push(payload);
            }
        }

        // 2. Valid Nominal Payload if no choice branches exist
        if suite.is_empty() {
            suite.push(self.generate_valid()?);
        }

        // 3. Valid Boundary Values (min/max facets)
        let mut boundary_cfg = GenerationConfig::default();
        boundary_cfg.use_boundary_values = true;
        if let Ok(payload) = self.generate_valid_with_config(
            boundary_cfg,
            "VALID_FACET_BOUNDARIES",
            "Valid payload testing facet boundary extremes",
        ) {
            suite.push(payload);
        }

        // 4. Syntactic Anomalies: Truncations
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 4 })?);
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 1 })?);

        // 5. Syntactic Anomalies: Trailing Garbage / Overflow
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 8 })?);

        // 6. Syntactic Anomalies: Declared Header / Prefix Length Mismatches
        let (len_offset, len_width) = if let Some(len_field) = self.analysis.find_length_field() {
            (len_field.estimated_offset, len_field.estimated_width.max(1))
        } else {
            (0, 2)
        };
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
            field_offset: len_offset,
            field_width: len_width,
            delta: 16,
        })?);
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
            field_offset: len_offset,
            field_width: len_width,
            delta: -10,
        })?);

        // 7. Syntactic Anomalies: Bitstream Misalignment
        suite.push(self.generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 })?);

        // 8. Semantic Anomalies: Wrong Enum Values
        for field in &enum_fields {
            let invalid_enum = if field.enumerations.iter().all(|s| s.chars().all(|c| c.is_ascii_digit())) {
                let max_val = field
                    .enumerations
                    .iter()
                    .filter_map(|s| s.parse::<u64>().ok())
                    .max()
                    .unwrap_or(0);
                (max_val.saturating_add(99)).to_string()
            } else {
                "INVALID_ENUM_ANOMALY".to_string()
            };
            let anomaly = SemanticAnomaly::WrongEnum {
                target_name: field.name.clone(),
                invalid_value: invalid_enum,
            };
            if let Ok(payload) = self.generate_semantic_anomaly(anomaly) {
                suite.push(payload);
            }
        }

        // 9. Semantic Anomalies: Out-of-bounds Facets
        for field in &bounded_fields {
            let invalid_val = if let Some(ref max_s) = field.max_inclusive {
                if let Ok(max_num) = max_s.parse::<i64>() {
                    DfdlValue::Long(max_num.saturating_add(10))
                } else {
                    DfdlValue::String("999999".into())
                }
            } else if let Some(ref min_s) = field.min_inclusive {
                if let Ok(min_num) = min_s.parse::<i64>() {
                    DfdlValue::Long(min_num.saturating_sub(10))
                } else {
                    DfdlValue::String("0".into())
                }
            } else {
                DfdlValue::Long(9999)
            };

            let anomaly = SemanticAnomaly::FacetOutOfBounds {
                target_name: field.name.clone(),
                invalid_value: invalid_val,
            };
            if let Ok(payload) = self.generate_semantic_anomaly(anomaly) {
                suite.push(payload);
            }
        }

        // 10. Semantic Anomalies: Unmatched Choice Dispatch Key
        for (discriminator_name, valid_keys) in self.analysis.find_choice_discriminators() {
            let invalid_key = if valid_keys.iter().all(|s| s.chars().all(|c| c.is_ascii_digit())) {
                let max_key = valid_keys
                    .iter()
                    .filter_map(|s| s.parse::<u64>().ok())
                    .max()
                    .unwrap_or(0);
                DfdlValue::UnsignedInt((max_key.saturating_add(99)) as u32)
            } else {
                DfdlValue::String("__INVALID_DISPATCH_KEY__".to_string())
            };

            let anomaly = SemanticAnomaly::WrongDispatchKey {
                discriminator_name,
                invalid_key,
            };
            if let Ok(payload) = self.generate_semantic_anomaly(anomaly) {
                suite.push(payload);
            }
        }

        // 11. Ensure at least one semantic anomaly is present in the exhaustive suite
        if !suite.iter().any(|p| matches!(p.mode, PayloadMode::SemanticAnomaly(_))) {
            let target_name = self
                .analysis
                .all_elements
                .first()
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "element".to_string());
            let anomaly = SemanticAnomaly::FacetOutOfBounds {
                target_name,
                invalid_value: DfdlValue::UnsignedByte(255),
            };
            if let Ok(payload) = self.generate_semantic_anomaly(anomaly) {
                suite.push(payload);
            }
        }

        Ok(suite)
    }

    /// Writes all payloads to a PCAP stream with network encapsulation.
    pub fn write_pcap<W: Write>(
        &self,
        w: &mut W,
        payloads: &[GeneratedPayload],
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> std::io::Result<()> {
        self.pcap_writer.write_header(w)?;

        let mut sec = 1_700_000_000u32;
        let mut usec = 0u32;

        for payload in payloads {
            self.pcap_writer.write_encapsulated_packet(
                w,
                sec,
                usec,
                config,
                transport,
                &payload.bytes,
            )?;
            usec = usec.saturating_add(10_000);
            if usec >= 1_000_000 {
                sec = sec.saturating_add(1);
                usec = 0;
            }
        }

        Ok(())
    }

    /// Converts generated payloads into a standalone PCAP file byte vector.
    pub fn export_pcap_bytes(
        &self,
        payloads: &[GeneratedPayload],
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.write_pcap(&mut bytes, payloads, config, transport)?;
        Ok(bytes)
    }

    /// Generates a suite of exclusively valid payloads covering the schema exhaustively:
    /// - Every choice branch discovered in the schema
    /// - Nominal payloads
    /// - Range boundaries (min/max facet values)
    /// - Array occurrence variations (min_occurs, nominal, max_occurs)
    pub fn generate_valid_suite(&mut self) -> DFDLResult<Vec<GeneratedPayload>> {
        let mut suite = Vec::new();
        let choice_branches = self.analysis.choice_branches.clone();

        // 1. Valid Payloads covering EVERY Choice Branch
        for branch in &choice_branches {
            let mut cfg = GenerationConfig::default();
            cfg.preferred_branch_index = Some(branch.branch_index);
            let name = format!("VALID_CHOICE_BRANCH_{}", branch.branch_index);
            let desc = format!(
                "Valid payload exercising choice branch {} (name: '{}', key: {:?})",
                branch.branch_index, branch.branch_name, branch.branch_key
            );
            if let Ok(payload) = self.generate_valid_with_config(cfg, &name, &desc) {
                suite.push(payload);
            }
        }

        // 2. Valid Nominal Payload
        suite.push(self.generate_valid()?);

        // 3. Valid Boundary Values (min/max facets)
        let mut boundary_cfg = GenerationConfig::default();
        boundary_cfg.use_boundary_values = true;
        if let Ok(payload) = self.generate_valid_with_config(
            boundary_cfg,
            "VALID_FACET_BOUNDARIES",
            "Valid payload testing facet boundary extremes",
        ) {
            suite.push(payload);
        }

        // 4. Valid Array Occurrences Variations
        for count in [1, 2, 3] {
            let mut arr_cfg = GenerationConfig::default();
            arr_cfg.array_occurrences = count;
            let name = format!("VALID_ARRAY_OCCURRENCES_{}", count);
            let desc = format!("Valid payload with {} array occurrences", count);
            if let Ok(payload) = self.generate_valid_with_config(arr_cfg, &name, &desc) {
                suite.push(payload);
            }
        }

        Ok(suite)
    }

    /// Generates a sequence of `count` valid payloads.
    pub fn generate_valid_sequence(&mut self, count: usize) -> DFDLResult<Vec<GeneratedPayload>> {
        let mut suite = Vec::with_capacity(count);
        for i in 0..count {
            let name = format!("VALID_PACKET_{}", i);
            let desc = format!("Valid payload sequence packet #{}", i);
            let payload = self.generate_valid_with_config(GenerationConfig::default(), &name, &desc)?;
            suite.push(payload);
        }
        Ok(suite)
    }

    /// Generates a valid PCAP capture containing exclusively valid, schema-compliant packets.
    ///
    /// If `count == 0`, generates the exhaustive valid suite (all choice branches, boundaries, and occurrences).
    /// If `count > 0`, generates `count` valid sequence packets.
    pub fn write_valid_pcap<W: Write>(
        &mut self,
        w: &mut W,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<()> {
        let payloads = if count == 0 {
            self.generate_valid_suite()?
        } else {
            self.generate_valid_sequence(count)?
        };
        self.write_pcap(w, &payloads, config, transport)
            .map_err(|e| dfdl_core::error::DFDLError::new(
                dfdl_core::error::DFDLErrorKind::Unparse,
                &e.to_string(),
            ))
    }

    /// Exports an exclusively valid PCAP capture directly as a byte vector.
    pub fn export_valid_pcap_bytes(
        &mut self,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<Vec<u8>> {
        let mut buf = Vec::new();
        self.write_valid_pcap(&mut buf, count, config, transport)?;
        Ok(buf)
    }
}
