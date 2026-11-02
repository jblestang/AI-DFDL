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

/// Configuration for controlling generation volume and data size budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GenerationLimits {
    /// Maximum number of messages/packets to generate.
    /// `None` indicates no explicit message count limit.
    pub max_messages: Option<usize>,
    /// Maximum cumulative size in bytes (for PCAP file or payload stream).
    /// Once this threshold would be exceeded, generation terminates.
    pub max_total_size: Option<usize>,
    /// Maximum allowed size in bytes for an individual payload.
    /// Payloads exceeding this size will be truncated to fit.
    pub max_payload_size: Option<usize>,
}

impl GenerationLimits {
    /// Creates a new default (unconstrained) limits specification.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new unconstrained limits specification.
    #[inline]
    #[must_use]
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Creates limits specifying a maximum number of messages.
    #[inline]
    #[must_use]
    pub fn with_message_count(count: usize) -> Self {
        Self {
            max_messages: Some(count),
            ..Default::default()
        }
    }

    /// Creates limits specifying a maximum total generated size in bytes.
    #[inline]
    #[must_use]
    pub fn with_max_size(max_bytes: usize) -> Self {
        Self {
            max_total_size: Some(max_bytes),
            ..Default::default()
        }
    }

    /// Sets the maximum number of messages.
    #[inline]
    #[must_use]
    pub fn max_messages(mut self, count: usize) -> Self {
        self.max_messages = Some(count);
        self
    }

    /// Sets the maximum total generated size in bytes.
    #[inline]
    #[must_use]
    pub fn max_total_size(mut self, max_bytes: usize) -> Self {
        self.max_total_size = Some(max_bytes);
        self
    }

    /// Sets the maximum size for an individual payload in bytes.
    #[inline]
    #[must_use]
    pub fn max_payload_size(mut self, max_bytes: usize) -> Self {
        self.max_payload_size = Some(max_bytes);
        self
    }
}

/// Execution report providing metrics on generated messages and data volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GenerationReport {
    /// Number of messages/packets successfully written or emitted.
    pub message_count: usize,
    /// Total bytes generated (including headers if PCAP).
    pub total_bytes: usize,
    /// Indicates whether generation stopped due to reaching `max_messages`.
    pub reached_message_limit: bool,
    /// Indicates whether generation stopped due to reaching `max_total_size`.
    pub reached_size_limit: bool,
}

/// Unified DFDL payload generator.
pub struct PayloadGenerator<'a> {
    schema: &'a CompiledSchema,
    analysis: SchemaAnalysis,
    prng: FastPrng,
    pcap_writer: PcapWriter,
    limits: GenerationLimits,
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
            limits: GenerationLimits::default(),
        }
    }

    /// Configures generation limits for controlling message count and data size.
    #[inline]
    #[must_use]
    pub fn with_limits(mut self, limits: GenerationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Configures the maximum number of messages to generate.
    #[inline]
    #[must_use]
    pub fn with_max_messages(mut self, count: usize) -> Self {
        self.limits.max_messages = Some(count);
        self
    }

    /// Configures the maximum total cumulative generated size in bytes.
    #[inline]
    #[must_use]
    pub fn with_max_total_size(mut self, max_bytes: usize) -> Self {
        self.limits.max_total_size = Some(max_bytes);
        self
    }

    /// Configures the maximum individual payload size in bytes.
    #[inline]
    #[must_use]
    pub fn with_max_payload_size(mut self, max_bytes: usize) -> Self {
        self.limits.max_payload_size = Some(max_bytes);
        self
    }

    /// Mutably configures generation limits.
    #[inline]
    pub fn set_limits(&mut self, limits: GenerationLimits) {
        self.limits = limits;
    }

    /// Mutably sets the maximum message count limit.
    #[inline]
    pub fn set_max_messages(&mut self, count: Option<usize>) {
        self.limits.max_messages = count;
    }

    /// Mutably sets the maximum total size limit in bytes.
    #[inline]
    pub fn set_max_total_size(&mut self, max_bytes: Option<usize>) {
        self.limits.max_total_size = max_bytes;
    }

    /// Mutably sets the maximum individual payload size limit in bytes.
    #[inline]
    pub fn set_max_payload_size(&mut self, max_bytes: Option<usize>) {
        self.limits.max_payload_size = max_bytes;
    }

    /// Returns the currently configured generation limits.
    #[inline]
    #[must_use]
    pub fn limits(&self) -> GenerationLimits {
        self.limits
    }

    /// Returns a mutable reference to the generation limits.
    #[inline]
    pub fn limits_mut(&mut self) -> &mut GenerationLimits {
        &mut self.limits
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
        mut config: GenerationConfig,
        name: &str,
        description: &str,
    ) -> DFDLResult<GeneratedPayload> {
        if config.max_payload_size.is_none() {
            config.max_payload_size = self.limits.max_payload_size;
        }
        let mut builder = ValidPayloadBuilder::new(self.schema, &self.analysis, self.prng, config);
        let mut bytes = builder.generate_bytes()?;
        if let Some(max_p) = self.limits.max_payload_size {
            if bytes.len() > max_p {
                bytes.truncate(max_p);
            }
        }
        Ok(GeneratedPayload {
            name: name.to_string(),
            description: description.to_string(),
            mode: PayloadMode::Valid,
            bytes,
        })
    }

    /// Generates a syntactically invalid payload by corrupting a valid base payload or dispatch key.
    pub fn generate_syntax_anomaly(&mut self, anomaly: SyntaxAnomaly) -> DFDLResult<GeneratedPayload> {
        match anomaly {
            SyntaxAnomaly::WrongDispatchKey {
                ref discriminator_name,
                ref invalid_key,
            } => {
                let mut config = GenerationConfig::default();
                if config.max_payload_size.is_none() {
                    config.max_payload_size = self.limits.max_payload_size;
                }
                let mut builder = ValidPayloadBuilder::new(
                    self.schema,
                    &self.analysis,
                    self.prng,
                    config,
                );
                let mut doc = builder.build_infoset()?;
                if let Some(ref mut root) = doc.root {
                    let _ = SemanticAnomalyInjector::mutate_element_value(
                        root,
                        discriminator_name,
                        invalid_key.clone(),
                    );
                }
                let mut bytes = builder.unparse_to_bytes(&doc)?;
                if let Some(max_p) = self.limits.max_payload_size {
                    if bytes.len() > max_p {
                        bytes.truncate(max_p);
                    }
                }

                let desc = format!(
                    "Syntactic anomaly: invalid choice dispatch key {:?} for discriminator '{}'",
                    invalid_key, discriminator_name
                );
                Ok(GeneratedPayload {
                    name: format!("SYNTAX_DISPATCH_KEY_{}", discriminator_name),
                    description: desc,
                    mode: PayloadMode::SyntaxAnomaly(anomaly),
                    bytes,
                })
            }
            _ => {
                let valid_payload = self.generate_valid()?;
                let mut corrupted_bytes =
                    SyntaxAnomalyInjector::apply(&valid_payload.bytes, &anomaly, &mut self.prng);
                if let Some(max_p) = self.limits.max_payload_size {
                    if corrupted_bytes.len() > max_p {
                        corrupted_bytes.truncate(max_p);
                    }
                }

                let desc = format!("Syntactic anomaly: {:?}", anomaly);
                Ok(GeneratedPayload {
                    name: format!("SYNTAX_{:?}", anomaly),
                    description: desc,
                    mode: PayloadMode::SyntaxAnomaly(anomaly),
                    bytes: corrupted_bytes,
                })
            }
        }
    }

    /// Generates a semantically invalid payload by mutating an Infoset DOM before serialization.
    pub fn generate_semantic_anomaly(&mut self, anomaly: SemanticAnomaly) -> DFDLResult<GeneratedPayload> {
        let mut config = GenerationConfig::default();
        if config.max_payload_size.is_none() {
            config.max_payload_size = self.limits.max_payload_size;
        }
        let mut builder = ValidPayloadBuilder::new(
            self.schema,
            &self.analysis,
            self.prng,
            config,
        );
        let mut doc = builder.build_infoset()?;
        let _ = SemanticAnomalyInjector::apply(&mut doc, &anomaly);
        let mut bytes = builder.unparse_to_bytes(&doc)?;
        if let Some(max_p) = self.limits.max_payload_size {
            if bytes.len() > max_p {
                bytes.truncate(max_p);
            }
        }

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
        let limits = self.limits;

        let try_push = |suite: &mut Vec<GeneratedPayload>, payload: GeneratedPayload| -> bool {
            if let Some(max_m) = limits.max_messages {
                if suite.len() >= max_m {
                    return false;
                }
            }
            suite.push(payload);
            true
        };

        // 1. Valid Payloads covering EVERY Choice Branch
        for branch in &choice_branches {
            let cfg = GenerationConfig {
                preferred_branch_index: Some(branch.branch_index),
                ..Default::default()
            };
            let name = format!("VALID_CHOICE_BRANCH_{}", branch.branch_index);
            let desc = format!(
                "Valid payload exercising choice branch {} (key: {:?})",
                branch.branch_index, branch.branch_key
            );
            if let Ok(payload) = self.generate_valid_with_config(cfg, &name, &desc) {
                if !try_push(&mut suite, payload) {
                    return Ok(suite);
                }
            }
        }

        // 2. Valid Nominal Payload if no choice branches exist
        if suite.is_empty() {
            let nominal = self.generate_valid()?;
            if !try_push(&mut suite, nominal) {
                return Ok(suite);
            }
        }

        // 3. Valid Boundary Values (min/max facets)
        let boundary_cfg = GenerationConfig {
            use_boundary_values: true,
            ..Default::default()
        };
        if let Ok(payload) = self.generate_valid_with_config(
            boundary_cfg,
            "VALID_FACET_BOUNDARIES",
            "Valid payload testing facet boundary extremes",
        ) {
            if !try_push(&mut suite, payload) {
                return Ok(suite);
            }
        }

        // 4. Syntactic Anomalies: Truncations
        let p_trunc4 = self.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 4 })?;
        if !try_push(&mut suite, p_trunc4) {
            return Ok(suite);
        }
        let p_trunc1 = self.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 1 })?;
        if !try_push(&mut suite, p_trunc1) {
            return Ok(suite);
        }

        // 5. Syntactic Anomalies: Trailing Garbage / Overflow
        let p_garbage = self.generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 8 })?;
        if !try_push(&mut suite, p_garbage) {
            return Ok(suite);
        }

        // 6. Syntactic Anomalies: Declared Header / Prefix Length Mismatches
        let (len_offset, len_width) = if let Some(len_field) = self.analysis.find_length_field() {
            (len_field.estimated_offset, len_field.estimated_width.max(1))
        } else {
            (0, 2)
        };
        let p_len1 = self.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
            field_offset: len_offset,
            field_width: len_width,
            delta: 16,
        })?;
        if !try_push(&mut suite, p_len1) {
            return Ok(suite);
        }
        let p_len2 = self.generate_syntax_anomaly(SyntaxAnomaly::DeclaredLengthMismatch {
            field_offset: len_offset,
            field_width: len_width,
            delta: -10,
        })?;
        if !try_push(&mut suite, p_len2) {
            return Ok(suite);
        }

        // 7. Syntactic Anomalies: Bitstream Misalignment
        let p_bit = self.generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 })?;
        if !try_push(&mut suite, p_bit) {
            return Ok(suite);
        }

        // 8. Syntactic Anomalies: Unmatched Choice Dispatch Key (Syntactic branch resolution failure)
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

            let anomaly = SyntaxAnomaly::WrongDispatchKey {
                discriminator_name,
                invalid_key,
            };
            if let Ok(payload) = self.generate_syntax_anomaly(anomaly) {
                if !try_push(&mut suite, payload) {
                    return Ok(suite);
                }
            }
        }

        // 9. Semantic Anomalies: Wrong Enum Values
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
                if !try_push(&mut suite, payload) {
                    return Ok(suite);
                }
            }
        }

        // 10. Semantic Anomalies: Out-of-bounds Facets
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
                if !try_push(&mut suite, payload) {
                    return Ok(suite);
                }
            }
        }

        // 11. Semantic Anomalies: Wrong Constant / Fixed Values
        for (field, expected_val) in self.analysis.find_constant_fields() {
            let invalid_val = match &expected_val {
                DfdlValue::UnsignedByte(b) => DfdlValue::UnsignedByte(b.wrapping_add(99)),
                DfdlValue::UnsignedShort(s) => DfdlValue::UnsignedShort(s.wrapping_add(99)),
                DfdlValue::UnsignedInt(i) => DfdlValue::UnsignedInt(i.wrapping_add(99)),
                DfdlValue::UnsignedLong(l) => DfdlValue::UnsignedLong(l.wrapping_add(99)),
                DfdlValue::Byte(b) => DfdlValue::Byte(b.wrapping_add(99)),
                DfdlValue::Short(s) => DfdlValue::Short(s.wrapping_add(99)),
                DfdlValue::Int(i) => DfdlValue::Int(i.wrapping_add(99)),
                DfdlValue::Long(l) => DfdlValue::Long(l.wrapping_add(99)),
                DfdlValue::String(s) => DfdlValue::String(format!("{}_INVALID_CONST", s)),
                _ => DfdlValue::UnsignedByte(255),
            };

            let anomaly = SemanticAnomaly::WrongConstantValue {
                target_name: field.name.clone(),
                expected_value: expected_val,
                invalid_value: invalid_val,
            };
            if let Ok(payload) = self.generate_semantic_anomaly(anomaly) {
                if !try_push(&mut suite, payload) {
                    return Ok(suite);
                }
            }
        }

        // 12. Ensure at least one semantic anomaly is present in the exhaustive suite (if limits allow)
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
                let _ = try_push(&mut suite, payload);
            }
        }

        Ok(suite)
    }

    /// Writes payloads to a PCAP stream with network encapsulation, cycling through `payloads`
    /// up to `count` packets (or one complete pass if `count == 0`), strictly respecting
    /// configured message count and total size limits.
    pub fn write_cycled_pcap<W: Write>(
        &self,
        w: &mut W,
        payloads: &[GeneratedPayload],
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> std::io::Result<GenerationReport> {
        let mut report = GenerationReport::default();
        const PCAP_GLOBAL_HEADER_LEN: usize = 24;

        if let Some(max_bytes) = self.limits.max_total_size {
            if max_bytes < PCAP_GLOBAL_HEADER_LEN {
                report.reached_size_limit = true;
                return Ok(report);
            }
        }

        self.pcap_writer.write_header(w)?;
        report.total_bytes = PCAP_GLOBAL_HEADER_LEN;

        if payloads.is_empty() {
            return Ok(report);
        }

        let target_count = if count > 0 {
            match self.limits.max_messages {
                Some(m) => count.min(m),
                None => count,
            }
        } else {
            match self.limits.max_messages {
                Some(m) => payloads.len().min(m),
                None => payloads.len(),
            }
        };

        let mut sec = 1_700_000_000u32;
        let mut usec = 0u32;

        for i in 0..target_count {
            let payload = &payloads[i % payloads.len()];
            if let Some(max_msg) = self.limits.max_messages {
                if report.message_count >= max_msg {
                    report.reached_message_limit = true;
                    break;
                }
            }

            let frame_len = match transport {
                TransportProtocol::Raw => payload.bytes.len(),
                TransportProtocol::Udp { .. } => payload.bytes.len() + 42,
            };
            let packet_record_len = 16 + frame_len.min(self.pcap_writer.snaplen as usize);

            if let Some(max_bytes) = self.limits.max_total_size {
                if report.total_bytes.saturating_add(packet_record_len) > max_bytes {
                    report.reached_size_limit = true;
                    break;
                }
            }

            self.pcap_writer.write_encapsulated_packet(
                w,
                sec,
                usec,
                config,
                transport,
                &payload.bytes,
            )?;

            report.total_bytes = report.total_bytes.saturating_add(packet_record_len);
            report.message_count += 1;

            if let Some(max_msg) = self.limits.max_messages {
                if report.message_count >= max_msg {
                    report.reached_message_limit = true;
                    break;
                }
            }

            usec = usec.saturating_add(10_000);
            if usec >= 1_000_000 {
                sec = sec.saturating_add(1);
                usec = 0;
            }
        }

        if (count > 0 && report.message_count >= count)
            || self.limits.max_messages.is_some_and(|m| report.message_count >= m)
        {
            report.reached_message_limit = true;
        }

        Ok(report)
    }

    /// Writes all payloads to a PCAP stream with network encapsulation, strictly respecting
    /// configured message count and total size limits.
    pub fn write_pcap<W: Write>(
        &self,
        w: &mut W,
        payloads: &[GeneratedPayload],
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> std::io::Result<GenerationReport> {
        self.write_cycled_pcap(w, payloads, payloads.len(), config, transport)
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

    /// Converts generated payloads into a standalone PCAP file byte vector along with its generation report.
    pub fn export_pcap_with_report(
        &self,
        payloads: &[GeneratedPayload],
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> std::io::Result<(Vec<u8>, GenerationReport)> {
        let mut bytes = Vec::new();
        let report = self.write_pcap(&mut bytes, payloads, config, transport)?;
        Ok((bytes, report))
    }

    /// Generates a suite of exclusively valid payloads covering the schema exhaustively:
    /// - Every choice branch discovered in the schema
    /// - Nominal payloads
    /// - Range boundaries (min/max facet values)
    /// - Array occurrence variations (min_occurs, nominal, max_occurs)
    ///
    /// Respects `max_messages` limits if configured.
    pub fn generate_valid_suite(&mut self) -> DFDLResult<Vec<GeneratedPayload>> {
        let mut suite = Vec::new();
        let choice_branches = self.analysis.choice_branches.clone();

        // 1. Valid Payloads covering EVERY Choice Branch
        for branch in &choice_branches {
            if let Some(max_m) = self.limits.max_messages {
                if suite.len() >= max_m {
                    return Ok(suite);
                }
            }
            let cfg = GenerationConfig {
                preferred_branch_index: Some(branch.branch_index),
                ..Default::default()
            };
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
        if self.limits.max_messages.is_none_or(|m| suite.len() < m) {
            if let Ok(nominal) = self.generate_valid() {
                suite.push(nominal);
            }
        }

        // 3. Valid Boundary Values (min/max facets)
        if self.limits.max_messages.is_none_or(|m| suite.len() < m) {
            let boundary_cfg = GenerationConfig {
                use_boundary_values: true,
                ..Default::default()
            };
            if let Ok(payload) = self.generate_valid_with_config(
                boundary_cfg,
                "VALID_FACET_BOUNDARIES",
                "Valid payload testing facet boundary extremes",
            ) {
                suite.push(payload);
            }
        }

        // 4. Valid Array Occurrences Variations
        for count in [1, 2, 3] {
            if let Some(max_m) = self.limits.max_messages {
                if suite.len() >= max_m {
                    return Ok(suite);
                }
            }
            let arr_cfg = GenerationConfig {
                array_occurrences: count,
                ..Default::default()
            };
            let name = format!("VALID_ARRAY_OCCURRENCES_{}", count);
            let desc = format!("Valid payload with {} array occurrences", count);
            if let Ok(payload) = self.generate_valid_with_config(arr_cfg, &name, &desc) {
                suite.push(payload);
            }
        }

        Ok(suite)
    }

    /// Generates a sequence of valid payloads respecting configured message and size limits.
    pub fn generate_valid_sequence(&mut self, count: usize) -> DFDLResult<Vec<GeneratedPayload>> {
        let target_count = match self.limits.max_messages {
            Some(m) => count.min(m),
            None => count,
        };
        let mut suite = Vec::with_capacity(target_count);
        let mut cumulative_bytes = 0usize;

        for i in 0..target_count {
            let name = format!("VALID_PACKET_{}", i);
            let desc = format!("Valid payload sequence packet #{}", i);
            let payload = self.generate_valid_with_config(GenerationConfig::default(), &name, &desc)?;

            if let Some(max_total) = self.limits.max_total_size {
                if cumulative_bytes.saturating_add(payload.bytes.len()) > max_total {
                    break;
                }
            }
            cumulative_bytes = cumulative_bytes.saturating_add(payload.bytes.len());
            suite.push(payload);
        }
        Ok(suite)
    }

    /// Generates a valid PCAP capture containing exclusively valid, schema-compliant packets,
    /// respecting configured message count and total size limits.
    ///
    /// If `count == 0` and `limits.max_messages` is None, generates the exhaustive valid suite.
    /// If `count > 0` or `limits.max_messages` is Some, generates up to `count` (or `max_messages`) valid packets.
    /// In all cases, generation stops immediately once `max_total_size` is reached.
    pub fn write_valid_pcap<W: Write>(
        &mut self,
        w: &mut W,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<GenerationReport> {
        let payloads = if count == 0 && self.limits.max_messages.is_none() {
            self.generate_valid_suite()?
        } else {
            let target_count = if count > 0 {
                count
            } else {
                self.limits.max_messages.unwrap_or(10)
            };
            let mut seq = Vec::with_capacity(target_count);
            for i in 0..target_count {
                let name = format!("VALID_PACKET_{}", i);
                let desc = format!("Valid payload sequence packet #{}", i);
                let p = self.generate_valid_with_config(GenerationConfig::default(), &name, &desc)?;
                let p_len = p.bytes.len();
                seq.push(p);

                // Early exit optimization if size limit is far exceeded
                if let Some(max_s) = self.limits.max_total_size {
                    let approx_pcap_size = 24 + seq.len() * (p_len + 58);
                    if approx_pcap_size > max_s.saturating_add(p_len + 58) {
                        break;
                    }
                }
            }
            seq
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

    /// Exports an exclusively valid PCAP capture directly as a byte vector along with its generation report.
    pub fn export_valid_pcap_with_report(
        &mut self,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<(Vec<u8>, GenerationReport)> {
        let mut buf = Vec::new();
        let report = self.write_valid_pcap(&mut buf, count, config, transport)?;
        Ok((buf, report))
    }

    /// Generates an exhaustive PCAP capture exercising valid, syntactic, and semantic cases,
    /// cycling through the test suite up to `count` packets (or one complete pass if `count == 0`),
    /// strictly respecting configured message count and total size limits.
    pub fn write_exhaustive_pcap<W: Write>(
        &mut self,
        w: &mut W,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<GenerationReport> {
        let suite = self.generate_exhaustive_suite()?;
        self.write_cycled_pcap(w, &suite, count, config, transport)
            .map_err(|e| dfdl_core::error::DFDLError::new(
                dfdl_core::error::DFDLErrorKind::Unparse,
                &e.to_string(),
            ))
    }

    /// Exports an exhaustive PCAP capture directly as a byte vector.
    pub fn export_exhaustive_pcap_bytes(
        &mut self,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<Vec<u8>> {
        let mut buf = Vec::new();
        self.write_exhaustive_pcap(&mut buf, count, config, transport)?;
        Ok(buf)
    }

    /// Exports an exhaustive PCAP capture directly as a byte vector along with its generation report.
    pub fn export_exhaustive_pcap_with_report(
        &mut self,
        count: usize,
        config: &NetworkConfig,
        transport: &TransportProtocol,
    ) -> DFDLResult<(Vec<u8>, GenerationReport)> {
        let mut buf = Vec::new();
        let report = self.write_exhaustive_pcap(&mut buf, count, config, transport)?;
        Ok((buf, report))
    }
}
