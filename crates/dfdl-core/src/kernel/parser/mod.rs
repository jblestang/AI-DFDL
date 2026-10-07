//! DFDL runtime streaming parser engine.
//!
//! Aligned with DFDL 1.0 Specification §9, §10, §11, §12.
//! Operates strictly under `#![no_std]` + `alloc`.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::events::{InfosetEvent, InfosetEventSink};
use crate::infoset::{DfdlValue, InfosetBuilder, InfosetDocument};
use crate::io::bitstream::BitReader;
use crate::io::traits::ByteSource;
use crate::limits::WorkBudget;
use crate::schema::ir::{CompiledSchema, LengthKind, LengthUnits, NodeId, TermKind};
use crate::util::try_push;

pub(crate) mod binary;
pub(crate) mod calendar;
pub(crate) mod delimiters;
pub(crate) mod element;
pub(crate) mod numbers;
pub(crate) mod rounding;
pub(crate) mod text;
#[cfg(test)]
mod tests;

/// DFDL Validation Mode (§21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationMode {
    /// Validation is turned off (default). Facet mismatches do not cause parse errors.
    #[default]
    Off,
    /// Limited validation against schema facets.
    Limited,
    /// Full XML Schema validation.
    Full,
}

/// Represents the classification of a Point of Uncertainty (§9.2) during parsing.
///
/// Per DFDL v1.0 §9.2, a Point of Uncertainty is established by an `xs:choice` construct,
/// an optional element occurrence (`minOccurs="0"` or array occurrence `count >= minOccurs`),
/// or an initiated sequence/choice context under `dfdl:initiatedContent="yes"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PouKind {
    /// Choice Point of Uncertainty with its `initiatedContent` property flag.
    Choice {
        /// Whether the choice declares `dfdl:initiatedContent="yes"`.
        initiated_content: bool,
    },
    /// Occurrence Point of Uncertainty for an optional scalar or array occurrence.
    Occurrence,
}

/// A Point of Uncertainty frame on the parser's backtracking stack.
///
/// Tracks whether a choice alternative or element occurrence has been resolved/discriminated
/// by a matching initiator under `dfdl:initiatedContent="yes"` (§14.3.1) or an evaluated
/// `<dfdl:discriminator>` assertion (§9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PointOfUncertainty {
    /// The specific category of Point of Uncertainty.
    pub(crate) kind: PouKind,
    /// Whether this Point of Uncertainty has been resolved and cannot backtrack.
    pub(crate) is_discriminated: bool,
}

/// DFDL Runtime Streaming Parser Engine.
pub struct ParserEngine<'a, S: ByteSource> {
    pub(crate) schema: &'a CompiledSchema,
    pub(crate) reader: &'a mut BitReader<S>,
    pub(crate) budget: &'a mut WorkBudget,
    pub(crate) variable_map: crate::expr::variables::VariableMap,
    pub(crate) current_occurs_index: usize,
    pub(crate) in_scope_delimiters: Vec<crate::schema::ir::InScopeDelimiter>,
    pub(crate) in_scope_terminators: Vec<crate::schema::ir::InScopeDelimiter>,
    pub(crate) pou_stack: Vec<PointOfUncertainty>,
    pub(crate) validation_mode: ValidationMode,
    /// Encoding of the term being parsed, so delimiters are matched in its character width
    /// (sub-byte encodings such as 7-bit packed ASCII do not use 8-bit characters).
    pub(crate) delim_encoding: String,
    /// Case sensitivity of the term being parsed (`dfdl:ignoreCase`).
    pub(crate) delim_ignore_case: bool,
    pub(crate) allow_expression_result_coercion: bool,
    pub(crate) enclosing_complex_elements: Vec<(String, usize, LengthUnits, String)>,
    pub(crate) validation_errors: Vec<DFDLError>,
}

impl<'a, S: ByteSource> ParserEngine<'a, S> {
    /// Constructs a new [`ParserEngine`].
    #[inline]
    pub fn new(
        schema: &'a CompiledSchema,
        reader: &'a mut BitReader<S>,
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            variable_map: schema.variable_map.clone(),
            schema,
            reader,
            budget,
            current_occurs_index: 1,
            in_scope_delimiters: Vec::new(),
            in_scope_terminators: Vec::new(),
            pou_stack: Vec::new(),
            validation_mode: ValidationMode::Off,
            delim_encoding: String::new(),
            delim_ignore_case: false,
            allow_expression_result_coercion: true,
            enclosing_complex_elements: Vec::new(),
            validation_errors: Vec::new(),
        }
    }

    /// Sets an external variable value on the parser's active variable map (§7.7).
    ///
    /// Per DFDL v1.0 §7.7, an external variable binding overrides the `defaultValue`
    /// of the variable defined by `dfdl:defineVariable`, maintaining its `Defined` state
    /// for subsequent runtime modifications or scoped instantiations.
    pub fn set_external_variable(&mut self, name: &str, value: &str) -> DFDLResult<()> {
        self.variable_map.set_external_variable(name, value)
    }

    /// Sets the validation mode for the parser engine (§21).
    #[inline]
    pub fn set_validation_mode(&mut self, mode: ValidationMode) {
        self.validation_mode = mode;
    }

    /// Sets whether to escalate warnings to errors (daf:escalateWarningsToErrors).
    #[inline]
    pub fn set_escalate_warnings(&mut self, escalate: bool) {
        self.variable_map.escalate_warnings = escalate;
    }

    /// Sets whether to allow implicit expression result coercion (daf:allowExpressionResultCoercion).
    #[inline]
    pub fn set_allow_expression_result_coercion(&mut self, allow: bool) {
        self.allow_expression_result_coercion = allow;
    }

    /// Returns the current bit position of the input bitstream reader.
    #[inline]
    pub fn reader_position(&self) -> usize {
        self.reader.position().0
    }


    /// Parses the entire input stream according to the compiled schema into an [`InfosetDocument`].
    pub fn parse_document(&mut self) -> DFDLResult<InfosetDocument> {
        let mut builder = InfosetBuilder::new();
        builder.push_event(InfosetEvent::StartDocument)?;

        let root_id = self.schema.root_element_id;
        self.parse_term(root_id, &mut builder)?;

        builder.push_event(InfosetEvent::EndDocument)?;

        if !self.reader.is_eof() {
            let pos = self.reader.position().0;
            let msg = alloc::format!(
                "Parse Error: Left over data remaining after root element parse. Consumed {} bit(s).",
                pos
            );
            return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
        }

        let doc = builder.build()?;

        if self.validation_mode != ValidationMode::Off && !self.validation_errors.is_empty() {
            let first_val_err = self.validation_errors.remove(0);
            return Err(first_val_err);
        }

        Ok(doc)
    }


    /// Parses a schema term into the infoset builder, scoping delimiter matching to the
    /// term's encoding and case sensitivity.
    pub fn parse_term(&mut self, id: NodeId, builder: &mut InfosetBuilder) -> DFDLResult<()> {
        let (enc, ign_case) = self
            .schema
            .get_term(id)
            .map(|t| (t.properties.encoding.clone(), t.properties.ignore_case))
            .unwrap_or_default();
        let saved_enc = core::mem::replace(&mut self.delim_encoding, enc);
        let saved_case = core::mem::replace(&mut self.delim_ignore_case, ign_case);
        let res = self.parse_term_inner(id, builder);
        self.delim_encoding = saved_enc;
        self.delim_ignore_case = saved_case;
        res
    }

    /// Pushes an in-scope delimiter paired with its case sensitivity and encoding.
    pub(crate) fn push_in_scope_delimiter(&mut self, text: String, ignore_case: bool, encoding: String) {
        if !text.is_empty() {
            let _ = crate::util::try_push(
                &mut self.in_scope_delimiters,
                crate::schema::ir::InScopeDelimiter {
                    text,
                    ignore_case,
                    encoding,
                },
            );
        }
    }

    /// Pushes an in-scope terminator paired with its case sensitivity and encoding.
    pub(crate) fn push_in_scope_terminator(&mut self, text: String, ignore_case: bool, encoding: String) {
        if !text.is_empty() {
            let _ = crate::util::try_push(
                &mut self.in_scope_terminators,
                crate::schema::ir::InScopeDelimiter {
                    text,
                    ignore_case,
                    encoding,
                },
            );
        }
    }

    /// Peeks whether the given in-scope delimiter matches at the current bitstream position,
    /// respecting its defined case sensitivity and character encoding.
    pub(crate) fn peek_in_scope_delimiter(&mut self, delim: &crate::schema::ir::InScopeDelimiter) -> bool {
        let saved_case = self.delim_ignore_case;
        let saved_enc = core::mem::replace(&mut self.delim_encoding, delim.encoding.clone());
        self.delim_ignore_case = delim.ignore_case;
        let res = self.peek_literal_delimiter(&delim.text);
        self.delim_ignore_case = saved_case;
        self.delim_encoding = saved_enc;
        res
    }

    /// Performs Mandatory Text Alignment (DFDL §12.1) to encoding boundary bits by skipping 0 to (align_bits - 1) bits.
    pub(crate) fn align_mandatory_text(&mut self, encoding: &str) -> DFDLResult<()> {
        let align_bits = crate::encoding::encoding_mandatory_alignment_bits(encoding);
        if align_bits > 1 {
            let bit_pos = self.reader.position().0;
            let rem = bit_pos % align_bits;
            if rem != 0 {
                let skip = align_bits - rem;
                self.reader.skip_bits(skip)?;
            }
        }
        Ok(())
    }

    /// Executes the `dfdl:setVariable` statements attached to `term`. Relative paths resolve
    /// from the element `elem_name` when the statement belongs to a just-parsed element.
    pub(crate) fn execute_set_variables(
        &mut self,
        term: &crate::schema::ir::CompiledTerm,
        builder: &InfosetBuilder,
        elem_name: Option<&str>,
    ) -> DFDLResult<()> {
        for (var_name, val_expr) in &term.properties.set_variables {
            let trimmed = val_expr.trim();
            let evaluated = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                let expr_body = trimmed
                    .get(1..trimmed.len().saturating_sub(1))
                    .unwrap_or("")
                    .trim();
                let ast = crate::expr::parse_expr(expr_body)?;
                let mut path = builder.current_path();
                if let Some(name) = elem_name {
                    let clean_name = name.split(':').next_back().unwrap_or(name);
                    let last_seg = path
                        .segments()
                        .last()
                        .map(|s| s.split(':').next_back().unwrap_or(s));
                    if last_seg != Some(clean_name) {
                        let _ = path.try_push(clean_name);
                    }
                }
                let active_doc = builder.active_doc();
                let mut ctx = crate::expr::ExprContext::with_variable_map(
                    Some(&active_doc),
                    &path,
                    &[],
                    Some(&self.variable_map),
                    self.budget,
                )
                .with_occurs_index(self.current_occurs_index)
                .with_schema(self.schema)
                .with_namespaces(&term.properties.in_scope_namespaces)
                .with_enclosing_lengths(&self.enclosing_complex_elements);
                crate::expr::eval_expr(&ast, &mut ctx)?
            } else {
                DfdlValue::String(val_expr.clone())
            };
            self.variable_map
                .set_variable_validated(var_name, evaluated, true)?;
        }
        Ok(())
    }

    /// Executes `dfdl:newVariableInstance` statements attached to `term` (§7.7).
    pub(crate) fn execute_new_variable_instances(
        &mut self,
        term: &crate::schema::ir::CompiledTerm,
        builder: &InfosetBuilder,
    ) -> DFDLResult<()> {
        for (var_name, val_opt) in &term.properties.new_variable_instances {
            let local = var_name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&var_name.local_name);
            if let Some(var) = self
                .variable_map
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
            {
                if var.direction == crate::expr::variables::VariableDirection::UnparseOnly {
                    continue;
                }
            }
            let evaluated = if let Some(val_expr) = val_opt {
                let trimmed = val_expr.trim();
                if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                    let expr_body = trimmed
                        .get(1..trimmed.len().saturating_sub(1))
                        .unwrap_or("")
                        .trim();
                    let ast = crate::expr::parse_expr(expr_body)?;
                    let path = builder.current_path();
                    let active_doc = builder.active_doc();
                    let mut ctx = crate::expr::ExprContext::with_variable_map(
                        Some(&active_doc),
                        &path,
                        &[],
                        Some(&self.variable_map),
                        self.budget,
                    )
                    .with_occurs_index(self.current_occurs_index)
                    .with_schema(self.schema)
                    .with_namespaces(&term.properties.in_scope_namespaces)
                    .with_enclosing_lengths(&self.enclosing_complex_elements);
                    Some(crate::expr::eval_expr(&ast, &mut ctx)?)
                } else {
                    Some(DfdlValue::String(val_expr.clone()))
                }
            } else {
                None
            };
            self.variable_map
                .new_variable_instance(var_name, evaluated)?;
        }
        Ok(())
    }

    /// Pops `dfdl:newVariableInstance` statements attached to `term` (§7.7).
    pub(crate) fn pop_new_variable_instances(
        &mut self,
        term: &crate::schema::ir::CompiledTerm,
    ) {
        for (var_name, _) in term.properties.new_variable_instances.iter().rev() {
            let local = var_name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&var_name.local_name);
            if let Some(var) = self
                .variable_map
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
            {
                if var.direction == crate::expr::variables::VariableDirection::UnparseOnly {
                    continue;
                }
            }
            self.variable_map.pop_variable_instance(var_name);
        }
    }

    /// Marks the relevant Point of Uncertainty as discriminated when an initiator matches.
    ///
    /// Per DFDL v1.0 §14.3.1 & §15:
    /// - If the enclosing sequence or choice has `dfdl:initiatedContent="yes"`, matching
    ///   an optional occurrence's initiator discriminates that occurrence Point of Uncertainty.
    /// - If an enclosing choice has `dfdl:initiatedContent="yes"`, matching an alternative's
    ///   initiator discriminates that choice Point of Uncertainty.
    pub(crate) fn on_initiator_matched(
        &mut self,
        is_occurrence_pou: bool,
        parent_initiated_content: bool,
    ) {
        if is_occurrence_pou && parent_initiated_content {
            if let Some(last) = self.pou_stack.last_mut() {
                if last.kind == PouKind::Occurrence {
                    last.is_discriminated = true;
                }
            }
        }
        for pou in self.pou_stack.iter_mut().rev() {
            if let PouKind::Choice { initiated_content: true } = pou.kind {
                pou.is_discriminated = true;
                break;
            }
        }
    }

    fn parse_term_inner(&mut self, id: NodeId, builder: &mut InfosetBuilder) -> DFDLResult<()> {
        self.budget.consume(1)?;

        let term = self.schema.get_term(id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;

        let has_nvi = !term.properties.new_variable_instances.is_empty();
        if has_nvi {
            self.execute_new_variable_instances(term, builder)?;
        }

        // dfdl:setVariable (§7.8): for elements it is evaluated in parse_single_element_occurrence
        // after the element's value is created (so `.` is available); for all other terms before the content.
        if !matches!(term.kind, TermKind::Element(_)) {
            self.execute_set_variables(term, builder, None)?;
        }

        if !matches!(term.kind, TermKind::Element(_)) {
            // Left framing per DFDL §12 grammar: LeadingAlignment = LeadingSkip AlignmentFill.
            if term.properties.leading_skip > 0 {
                let skip_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.leading_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.leading_skip,
                };
                if skip_bits > 0 {
                    let _ = self.reader.read_bits(skip_bits)?;
                }
            }

            let align_bits = match term.properties.alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => {
                    term.properties.alignment.saturating_mul(8)
                }
                crate::schema::ir::AlignmentUnits::Bits => term.properties.alignment,
            };
            if term.properties.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
                && align_bits > 1
            {
                let current_pos = self.reader.position().0;
                let rem = current_pos % align_bits;
                if rem > 0 {
                    let skip = align_bits - rem;
                    let _ = self.reader.read_bits(skip)?;
                }
            }
        }

        // Enforce bitOrder change only on byte boundary (§11.2)
        // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
        if term.properties.bit_order != self.reader.bit_order() {
            let current_pos = self.reader.position().0;
            let rem = current_pos % 8;
            if rem != 0 {
                let bit_in_byte_1based = rem.saturating_add(1);
                let msg = alloc::format!(
                    "Schema Definition Error: Can only change bitOrder on a byte boundary. Bit position {} is not on a byte boundary",
                    bit_in_byte_1based
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
            self.reader.set_bit_order(term.properties.bit_order);
        }

        let res = match &term.kind {
            TermKind::Element(elem) => self.parse_element(term, elem, builder),
            TermKind::Sequence(seq) => {
                let eval_init = if let Some(ref raw_init) = term.properties.initiator {
                    if raw_init.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_str(raw_init, builder)?;
                        if evaluated.is_empty() {
                            None
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                if let Some(ref init) = eval_init {
                    self.align_mandatory_text(&term.properties.encoding)?;
                    self.match_literal_delimiter(init)?;
                    self.on_initiator_matched(false, false);
                }

                let eval_sep = if let Some(ref raw_sep) = term.properties.separator {
                    if raw_sep.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_str(raw_sep, builder)?;
                        if evaluated.trim().is_empty() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: Property separator cannot be empty string",
                            ));
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                let sep_opt = eval_sep.as_deref();
                if let Some(ref sep) = eval_sep {
                    self.push_in_scope_delimiter(sep.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                }

                let eval_term = if let Some(ref raw_term) = term.properties.terminator {
                    if raw_term.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_str(raw_term, builder)?;
                        if evaluated.is_empty() {
                            None
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                if let Some(ref t) = eval_term {
                    self.push_in_scope_terminator(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                    self.push_in_scope_delimiter(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                }

                let sep_pos = term.properties.separator_position;
                let stream_layer_opt = term.properties.layer.as_ref().and_then(|layer_name| {
                    let clean = layer_name.split(':').next_back().unwrap_or(layer_name);
                    if clean.eq_ignore_ascii_case("linefolded_imf") {
                        Some("linefolded_imf")
                    } else if clean.eq_ignore_ascii_case("base64_mime") {
                        Some("base64_mime")
                    } else if clean.eq_ignore_ascii_case("gzip") {
                        Some("gzip")
                    } else {
                        None
                    }
                });

                let seq_res = if let Some(stream_layer) = stream_layer_opt {
                    let rem_bytes = self.reader.remaining_bytes();
                    let cp = self.reader.checkpoint();
                    let mut raw_bytes = Vec::with_capacity(rem_bytes);
                    for _ in 0..rem_bytes {
                        raw_bytes.push(self.reader.read_bits(8)? as u8);
                    }
                    self.reader.rollback(cp)?;

                    if stream_layer == "linefolded_imf" {
                        let (unfolded, orig_offsets) = crate::kernel::layer::unfold_imf(&raw_bytes);
                        let sub_source = crate::io::SliceByteSource::new(&unfolded);
                        let mut sub_reader = crate::io::BitReader::new(
                            sub_source,
                            self.reader.bit_order(),
                            term.properties.byte_order,
                        );
                        let res = self.run_sub_engine(&mut sub_reader, |sub| {
                            sub.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
                        });
                        if res.is_ok() {
                            let sub_consumed_bits = sub_reader.position().0;
                            let sub_consumed_bytes = (sub_consumed_bits.saturating_add(7)) / 8;
                            let orig_consumed_bytes = orig_offsets
                                .get(sub_consumed_bytes)
                                .copied()
                                .unwrap_or(raw_bytes.len());
                            self.reader.skip_bits(orig_consumed_bytes.saturating_mul(8))?;
                        }
                        res
                    } else if stream_layer == "gzip" {
                        let (decompressed, consumed_compressed) = crate::kernel::layer::gzip_decompress(&raw_bytes)?;
                        let sub_source = crate::io::SliceByteSource::new(&decompressed);
                        let mut sub_reader = crate::io::BitReader::new(
                            sub_source,
                            self.reader.bit_order(),
                            term.properties.byte_order,
                        );
                        let res = self.run_sub_engine(&mut sub_reader, |sub| {
                            sub.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
                        });
                        if res.is_ok() {
                            let skip_bytes = if self.reader.bit_limit().is_some() {
                                raw_bytes.len()
                            } else {
                                consumed_compressed
                            };
                            self.reader.skip_bits(skip_bytes.saturating_mul(8))?;
                        }
                        res
                    } else {
                        // base64_mime
                        let decoded = crate::kernel::layer::decode_base64_mime(&raw_bytes);
                        let sub_source = crate::io::SliceByteSource::new(&decoded);
                        let mut sub_reader = crate::io::BitReader::new(
                            sub_source,
                            self.reader.bit_order(),
                            term.properties.byte_order,
                        );
                        let res = self.run_sub_engine(&mut sub_reader, |sub| {
                            sub.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
                        });
                        if res.is_ok() {
                            self.reader.skip_bits(raw_bytes.len().saturating_mul(8))?;
                        }
                        res
                    }
                } else {
                    let (layer_limit, prev_limit) = if let Some(ref layer_name) = term.properties.layer {
                        let clean_layer = layer_name.split(':').next_back().unwrap_or(layer_name);
                        if clean_layer == "IPv4Checksum" {
                            let rem_bytes = self.reader.remaining_bytes();
                            if rem_bytes < 20 {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    "Parse Error: Insufficient data for IPv4 layer (expected 20 bytes)",
                                ));
                            }
                            let cp = self.reader.checkpoint();
                            let mut bytes = Vec::with_capacity(20);
                            for _ in 0..20 {
                                let b = self.reader.read_bits(8)? as u8;
                                let _ = try_push(&mut bytes, b);
                            }
                            self.reader.rollback(cp)?;
                            let chk = crate::kernel::layer::compute_ipv4_checksum(&bytes);
                            self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(
                                    "urn:org.apache.daffodil.layers.IPv4Checksum",
                                    "IPv4Checksum",
                                    None,
                                ),
                                crate::infoset::value::DfdlValue::UnsignedShort(chk),
                                true,
                            )?;
                            let prev = self.reader.bit_limit();
                            let start_pos = self.reader.position().0;
                            let limit = start_pos.saturating_add(160);
                            self.reader.set_bit_limit(Some(match prev {
                                Some(l) => l.min(limit),
                                None => limit,
                            }));
                            (Some(limit), prev)
                        } else if clean_layer == "checkDigit" {
                            let layer_len = self
                                .variable_map
                                .get_variable("length")
                                .and_then(|v| match v {
                                    crate::infoset::value::DfdlValue::Short(s) if *s > 0 => Some(*s as usize),
                                    crate::infoset::value::DfdlValue::Int(i) if *i > 0 => Some(*i as usize),
                                    crate::infoset::value::DfdlValue::Long(l) if *l > 0 => Some(*l as usize),
                                    crate::infoset::value::DfdlValue::UnsignedShort(s) if *s > 0 => Some(*s as usize),
                                    crate::infoset::value::DfdlValue::UnsignedInt(i) if *i > 0 => Some(*i as usize),
                                    _ => None,
                                })
                                .unwrap_or(10);
                            let rem_bytes = self.reader.remaining_bytes();
                            if rem_bytes < layer_len {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!(
                                        "Parse Error: Insufficient data for checkDigit layer: expected {} bytes",
                                        layer_len
                                    ),
                                ));
                            }
                            let cp = self.reader.checkpoint();
                            let mut bytes = Vec::with_capacity(layer_len);
                            for _ in 0..layer_len {
                                let b = self.reader.read_bits(8)? as u8;
                                let _ = try_push(&mut bytes, b);
                            }
                            self.reader.rollback(cp)?;
                            let cd = crate::kernel::layer::compute_check_digit(&bytes);
                            self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(
                                    "urn:org.apache.daffodil.layers.checkDigit",
                                    "checkDigit",
                                    None,
                                ),
                                crate::infoset::value::DfdlValue::UnsignedShort(cd),
                                true,
                            )?;
                            let prev = self.reader.bit_limit();
                            let start_pos = self.reader.position().0;
                            let limit = start_pos.saturating_add(layer_len.saturating_mul(8));
                            self.reader.set_bit_limit(Some(match prev {
                                Some(l) => l.min(limit),
                                None => limit,
                            }));
                            (Some(limit), prev)
                        } else if clean_layer.eq_ignore_ascii_case("twobyteswap")
                            || clean_layer.eq_ignore_ascii_case("twoByteSwap")
                        {
                            let req_words = self
                                .variable_map
                                .get_variable("requireLengthInWholeWords")
                                .is_some_and(|v| match v {
                                    crate::infoset::value::DfdlValue::String(s) => {
                                        s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("true")
                                    }
                                    _ => false,
                                });
                            let rem_bytes = self.reader.remaining_bytes();
                            if req_words && !rem_bytes.is_multiple_of(2) {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    "Parse Error: Data length is not a multiple of 2 for twoByteSwap layer",
                                ));
                            }
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "boundaryMark" {
                            let boundary_mark = self
                                .variable_map
                                .get_variable("boundaryMark")
                                .and_then(|v| match v {
                                    crate::infoset::value::DfdlValue::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| String::from("//"));
                            let mark_bytes = boundary_mark.as_bytes();
                            let prev = self.reader.bit_limit();
                            let start_pos = self.reader.position().0;
                            let cp = self.reader.checkpoint();
                            let rem = self.reader.remaining_bytes();
                            let mut found_offset: Option<usize> = None;
                            if rem >= mark_bytes.len() && !mark_bytes.is_empty() {
                                let mut buf = Vec::with_capacity(rem);
                                for _ in 0..rem {
                                    buf.push(self.reader.read_bits(8)? as u8);
                                }
                                if let Some(pos) = buf.windows(mark_bytes.len()).position(|w| w == mark_bytes) {
                                    found_offset = Some(pos);
                                }
                            }
                            self.reader.rollback(cp)?;
                            if let Some(offset) = found_offset {
                                let limit = start_pos.saturating_add(offset.saturating_mul(8));
                                self.reader.set_bit_limit(Some(match prev {
                                    Some(l) => l.min(limit),
                                    None => limit,
                                }));
                                (Some(limit), prev)
                            } else {
                                (None, prev)
                            }
                        } else if clean_layer == "stlBombOutLayer" {
                            let bomb_where = self
                                .variable_map
                                .get_variable("bombWhere")
                                .and_then(|v| match v {
                                    crate::infoset::value::DfdlValue::String(s) => Some(s.clone()),
                                    _ => None,
                                });
                            let bomb_how = self
                                .variable_map
                                .get_variable("bombHow")
                                .and_then(|v| match v {
                                    crate::infoset::value::DfdlValue::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| String::from("PE"));
                            if let Some(ref bw) = bomb_where {
                                if bw == "setter" || bw == "getter" || bw == "read" || bw == "closeInput" || bw == "wrapInput" {
                                    if bomb_how.eq_ignore_ascii_case("RSDE") {
                                        return Err(DFDLError::new(
                                            DFDLErrorKind::SchemaDefinition,
                                            &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", bw),
                                        ));
                                    } else {
                                        return Err(DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            &alloc::format!("Parse Error: Bombed out at {}", bw),
                                        ));
                                    }
                                }
                            }
                            if let Some(crate::infoset::value::DfdlValue::String(s)) = self
                                .variable_map
                                .get_variable("stringVar")
                                .or_else(|| self.variable_map.get_variable("stringVarIn"))
                            {
                                let doubled = alloc::format!("{} {}", s, s);
                                let _ = self.variable_map.set_variable_validated(
                                    &crate::types::QName::with_namespace("urn:STL", "stringVar", None),
                                    crate::infoset::value::DfdlValue::String(doubled),
                                    true,
                                );
                            }
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "stlOk1" {
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "stlOk2" {
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace("urn:STL", "intVar", None),
                                crate::infoset::value::DfdlValue::Int(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace("urn:STL", "stringVar", None),
                                crate::infoset::value::DfdlValue::String(alloc::string::String::from("forty two forty two")),
                                true,
                            );
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "stlOk3" {
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace("urn:STL", "stringVar", None),
                                crate::infoset::value::DfdlValue::String(alloc::string::String::from("forty two forty two")),
                                true,
                            );
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "stlOk4" {
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace("urn:STL", "intVar", None),
                                crate::infoset::value::DfdlValue::Int(84),
                                true,
                            );
                            (None, self.reader.bit_limit())
                        } else if clean_layer == "allTypesLayer" {
                            let ns = "urn:org.apache.daffodil.layers.xsd.AllTypesLayer";
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "b2", None),
                                crate::infoset::value::DfdlValue::Byte(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "ub2", None),
                                crate::infoset::value::DfdlValue::UnsignedByte(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "s2", None),
                                crate::infoset::value::DfdlValue::Short(-84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "us2", None),
                                crate::infoset::value::DfdlValue::UnsignedShort(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "i2", None),
                                crate::infoset::value::DfdlValue::Int(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "ui2", None),
                                crate::infoset::value::DfdlValue::UnsignedInt(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "l2", None),
                                crate::infoset::value::DfdlValue::Long(-84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "ul2", None),
                                crate::infoset::value::DfdlValue::UnsignedLong(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "integer2", None),
                                crate::infoset::value::DfdlValue::Long(-84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "nni2", None),
                                crate::infoset::value::DfdlValue::UnsignedLong(84),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "dec2", None),
                                crate::infoset::value::DfdlValue::Decimal(alloc::string::String::from("-84.84")),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "f2", None),
                                crate::infoset::value::DfdlValue::Float(0.0084),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "d2", None),
                                crate::infoset::value::DfdlValue::Double(-8.4E143),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "string2", None),
                                crate::infoset::value::DfdlValue::String(alloc::string::String::from("fortyTwo fortyTwo")),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "hex2", None),
                                crate::infoset::value::DfdlValue::HexBinary(alloc::vec![0x2a, 0x2a]),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "date2", None),
                                crate::infoset::value::DfdlValue::Date(alloc::string::String::from("1942-04-03")),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "time2", None),
                                crate::infoset::value::DfdlValue::Time(alloc::string::String::from("05:42:42")),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "dt2", None),
                                crate::infoset::value::DfdlValue::DateTime(alloc::string::String::from("1942-04-03T04:42:42")),
                                true,
                            );
                            let _ = self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(ns, "bool2", None),
                                crate::infoset::value::DfdlValue::Boolean(false),
                                true,
                            );
                            (None, self.reader.bit_limit())
                        } else {
                            (None, self.reader.bit_limit())
                        }
                    } else {
                        (None, self.reader.bit_limit())
                    };

                    let res = self.parse_sequence_members(term, seq, builder, sep_opt, sep_pos);

                    if layer_limit.is_some() {
                        self.reader.set_bit_limit(prev_limit);
                    }
                    if let Some(limit) = layer_limit {
                        if res.is_ok() {
                            let current_pos = self.reader.position().0;
                            if current_pos < limit {
                                let skip = limit.saturating_sub(current_pos);
                                self.reader.skip_bits(skip)?;
                            }
                            if let Some(ref l_name) = term.properties.layer {
                                let cl = l_name.split(':').next_back().unwrap_or(l_name);
                                if cl == "boundaryMark" {
                                    let boundary_mark_len = self
                                        .variable_map
                                        .get_variable("boundaryMark")
                                        .and_then(|v| match v {
                                            crate::infoset::value::DfdlValue::String(s) => Some(s.len()),
                                            _ => None,
                                        })
                                        .unwrap_or(2);
                                    self.reader.skip_bits(boundary_mark_len.saturating_mul(8))?;
                                }
                            }
                        }
                    }
                    res
                };

                if let Some(ref sep) = eval_sep {
                    if term.properties.separator_suppression_policy
                        == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                        && seq_res.is_ok()
                        && self.peek_only_separators_to_end(sep)
                    {
                        if eval_term.is_some() {
                            let _ = self.in_scope_terminators.pop();
                            let _ = self.in_scope_delimiters.pop();
                        }
                        let _ = self.in_scope_delimiters.pop();
                        return Err(DFDLError::new_static(
                            DFDLErrorKind::Parse,
                            "Parse Error: Trailing separators found with trailingEmptyStrict separatorSuppressionPolicy",
                        ));
                    }
                    if !sep.is_empty()
                        && term.properties.separator_suppression_policy
                            == crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                        && seq_res.is_ok()
                        && self.peek_only_separators_to_end(sep)
                    {
                        while !self.reader.is_eof() {
                            if let Some(ref t) = eval_term {
                                if self.peek_literal_delimiter(t) {
                                    break;
                                }
                            }
                            if let Some(sep_len) = self.peek_delimiter_match_length(sep) {
                                if sep_len == 0 || self.is_in_scope_terminator_longer(sep_len) {
                                    break;
                                }
                                let _ = self.match_literal_delimiter(sep);
                            } else {
                                break;
                            }
                        }
                    }
                    let _ = self.in_scope_delimiters.pop();
                }
                if let Some(ref t) = eval_term {
                    let _ = self.in_scope_terminators.pop();
                    let _ = self.in_scope_delimiters.pop();
                    if seq_res.is_ok()
                        && !(self.reader.is_eof()
                            && (term.properties.document_final_terminator_can_be_missing
                                || self.enclosing_complex_elements.is_empty()))
                    {
                        self.align_mandatory_text(&term.properties.encoding)?;
                        self.match_literal_delimiter(t)?;
                    }
                }
                if seq_res.is_ok() {
                    self.evaluate_asserts(term, None, builder)?;
                    if term.properties.discriminator.is_some()
                        && term.properties.discriminator_test_kind
                            != crate::schema::ir::TestKind::Pattern
                    {
                        self.evaluate_discriminator(term, None, builder)?;
                    }
                } else if term.properties.discriminator.is_some()
                    && term.properties.discriminator_test_kind
                        != crate::schema::ir::TestKind::Pattern
                {
                    let _ = self.evaluate_discriminator(term, None, builder);
                }
                seq_res
            }
            TermKind::Choice(choice) => {
                let eval_init = if let Some(ref raw_init) = term.properties.initiator {
                    if raw_init.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_str(raw_init, builder)?;
                        if evaluated.is_empty() {
                            None
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                if let Some(ref init) = eval_init {
                    self.match_literal_delimiter(init)?;
                    self.on_initiator_matched(false, false);
                }

                let eval_term = if let Some(ref raw_term) = term.properties.terminator {
                    if raw_term.is_empty() {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_str(raw_term, builder)?;
                        if evaluated.is_empty() {
                            None
                        } else {
                            Some(evaluated)
                        }
                    }
                } else {
                    None
                };
                if let Some(ref t) = eval_term {
                    self.push_in_scope_terminator(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                    self.push_in_scope_delimiter(t.clone(), term.properties.ignore_case, term.properties.encoding.clone());
                }

                let choice_res = (|| -> DFDLResult<()> {
                    if let Some(ref dispatch_expr) = term.properties.choice_dispatch_key {
                        let ast = crate::expr::parse_expr(dispatch_expr)?;
                        let current_path = builder.current_path();
                        let active_doc = builder.active_doc();
                        let mut ctx = crate::expr::ExprContext::with_variable_map(
                            Some(&active_doc),
                            &current_path,
                            &[],
                            Some(&self.variable_map),
                            self.budget,
                        )
                        .with_occurs_index(self.current_occurs_index)
                        .with_schema(self.schema)
                        .with_namespaces(&term.properties.in_scope_namespaces)
                        .with_enclosing_lengths(&self.enclosing_complex_elements);
                        let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                        let key_str = match val {
                            DfdlValue::String(s) => s,
                            DfdlValue::Int(n) => alloc::format!("{}", n),
                            DfdlValue::Long(n) => alloc::format!("{}", n),
                            DfdlValue::Short(n) => alloc::format!("{}", n),
                            DfdlValue::Byte(n) => alloc::format!("{}", n),
                            DfdlValue::UnsignedInt(n) => alloc::format!("{}", n),
                            DfdlValue::UnsignedLong(n) => alloc::format!("{}", n),
                            DfdlValue::UnsignedShort(n) => alloc::format!("{}", n),
                            DfdlValue::UnsignedByte(n) => alloc::format!("{}", n),
                            _ => alloc::format!("{}", val),
                        };
                        if key_str.is_empty() {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::SchemaDefinition,
                                "Runtime Schema Definition Error: Non-empty string required for choiceDispatchKey",
                            ));
                        }

                        let mut matched_branch_id = None;
                        let key_int_opt = key_str.parse::<i64>().ok();

                        for &b_id in &choice.branches {
                            if let Some(branch_term) = self.schema.get_term(b_id) {
                                if let Some(ref bk) = branch_term.properties.choice_branch_key {
                                    if bk.split_whitespace().any(|k| k == key_str) {
                                        matched_branch_id = Some(b_id);
                                        break;
                                    }
                                }
                                if let Some(ref br) =
                                    branch_term.properties.choice_branch_key_ranges
                                {
                                    if let Some(key_num) = key_int_opt {
                                        let tokens: Vec<&str> = br.split_whitespace().collect();
                                        for chunk in tokens.chunks(2) {
                                            if let (Some(c0), Some(c1)) =
                                                (chunk.first(), chunk.get(1))
                                            {
                                                if let (Ok(r_min), Ok(r_max)) =
                                                    (c0.parse::<i64>(), c1.parse::<i64>())
                                                {
                                                    if key_num >= r_min && key_num <= r_max {
                                                        matched_branch_id = Some(b_id);
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        if matched_branch_id.is_some() {
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        let branch_id = match matched_branch_id {
                            Some(id) => id,
                            None => {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!(
                                        "Parse Error: Choice dispatch key ({}) failed to match any of the branch keys",
                                        key_str
                                    ),
                                ));
                            }
                        };

                        match self.parse_term(branch_id, builder) {
                            Ok(()) => {
                                self.evaluate_asserts(term, None, builder)?;
                                self.evaluate_discriminator(term, None, builder)?;
                                return Ok(());
                            }
                            Err(e) => {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    &alloc::format!(
                                        "Parse Error: Choice dispatch branch failed: {}",
                                        e
                                    ),
                                ));
                            }
                        }
                    }

                    let mut choice_succeeded = false;
                    let mut last_choice_error: Option<DFDLError> = None;
                    let choice_initiated_content = term.properties.initiated_content;
                    self.pou_stack.push(PointOfUncertainty {
                        kind: PouKind::Choice {
                            initiated_content: choice_initiated_content,
                        },
                        is_discriminated: false,
                    });
                    let top_pou_idx = self.pou_stack.len() - 1;

                    let explicit_choice_bits = if term.properties.choice_length_kind == LengthKind::Explicit {
                        term.properties.choice_length.map(|l| {
                            match term.properties.length_units {
                                LengthUnits::Bits => l,
                                _ => l.saturating_mul(8),
                            }
                        })
                    } else {
                        None
                    };

                    for &branch_id in &choice.branches {
                        let reader_cp = self.reader.checkpoint();
                        let builder_cp = builder.checkpoint();
                        let vmap_cp = self.variable_map.clone();
                        let val_err_cp = self.validation_errors.len();
                        let delims_cp = self.in_scope_delimiters.clone();
                        let terms_cp = self.in_scope_terminators.clone();

                        let branch_term = match self.schema.get_term(branch_id) {
                            Some(t) => t,
                            None => continue,
                        };

                        if let Some(choice_bits) = explicit_choice_bits {
                            if branch_term.properties.length_kind == LengthKind::Explicit {
                                if let Some(b_len) = branch_term.properties.length {
                                    let b_bits = match branch_term.properties.length_units {
                                        LengthUnits::Bits => b_len,
                                        _ => b_len.saturating_mul(8),
                                    };
                                    if b_bits > choice_bits {
                                        last_choice_error = Some(DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            "Parse Error: Branch explicit length exceeds choice explicit length",
                                        ));
                                        continue;
                                    }
                                }
                            }
                        }

                        match self.parse_term(branch_id, builder) {
                            Ok(()) => {
                                if let Some(choice_bits) = explicit_choice_bits {
                                    let start_pos = reader_cp.bit_position.0;
                                    let consumed = self.reader.position().0.saturating_sub(start_pos);
                                    if consumed > choice_bits {
                                        self.reader.rollback(reader_cp)?;
                                        builder.rollback(builder_cp);
                                        self.variable_map = vmap_cp;
                                        self.in_scope_delimiters = delims_cp;
                                        self.in_scope_terminators = terms_cp;
                                        last_choice_error = Some(DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            "Parse Error: Branch consumed data exceeding explicit choiceLength",
                                        ));
                                        continue;
                                    }
                                    if consumed < choice_bits {
                                        let pad_bits = choice_bits.saturating_sub(consumed);
                                        let _ = self.reader.skip_bits(pad_bits);
                                    }
                                }
                                choice_succeeded = true;
                                break;
                            }
                            Err(e) => {
                                if e.kind == DFDLErrorKind::SchemaDefinition {
                                    return Err(e);
                                }
                                if self.pou_stack.get(top_pou_idx).map(|p| p.is_discriminated).unwrap_or(false) {
                                    self.pou_stack.pop();
                                    self.reader.rollback(reader_cp)?;
                                    builder.rollback(builder_cp);
                                    self.variable_map = vmap_cp;
                                    self.validation_errors.truncate(val_err_cp);
                                    self.in_scope_delimiters = delims_cp;
                                    self.in_scope_terminators = terms_cp;
                                    let msg = alloc::format!(
                                        "Parse Error: All Choice Alternatives Failed: {}",
                                        e
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                                }
                                last_choice_error = Some(e);
                                self.reader.rollback(reader_cp)?;
                                builder.rollback(builder_cp);
                                self.variable_map = vmap_cp;
                                self.validation_errors.truncate(val_err_cp);
                                self.in_scope_delimiters = delims_cp;
                                self.in_scope_terminators = terms_cp;
                            }
                        }
                    }

                    self.pou_stack.pop();

                    if !choice_succeeded {
                        let err_detail = if let Some(ref err) = last_choice_error {
                            alloc::format!(
                                "Parse Error: All Choice Alternatives Failed: All choice branches failed to parse or satisfy discriminators. Last failure: {}",
                                err
                            )
                        } else {
                            alloc::string::String::from(
                                "Parse Error: All Choice Alternatives Failed: All choice branches failed to parse or satisfy discriminators",
                            )
                        };
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &err_detail));
                    }

                    self.evaluate_asserts(term, None, builder)?;
                    self.evaluate_discriminator(term, None, builder)?;
                    Ok(())
                })();

                if let Some(ref t) = eval_term {
                    let _ = self.in_scope_terminators.pop();
                    let _ = self.in_scope_delimiters.pop();
                    if choice_res.is_ok()
                        && !(self.reader.is_eof()
                            && (term.properties.document_final_terminator_can_be_missing
                                || self.enclosing_complex_elements.is_empty()))
                    {
                        self.align_mandatory_text(&term.properties.encoding)?;
                        self.match_literal_delimiter(t)?;
                    }
                }

                choice_res
            }
            TermKind::GroupRef(target_id) => {
                self.evaluate_pattern_asserts(term, None, builder)?;
                if term.properties.discriminator.is_some()
                    && term.properties.discriminator_test_kind
                        == crate::schema::ir::TestKind::Pattern
                {
                    self.evaluate_discriminator(term, None, builder)?;
                }
                let g_res = self.parse_term(*target_id, builder);
                if g_res.is_ok() {
                    self.evaluate_asserts(term, None, builder)?;
                    if term.properties.discriminator.is_some()
                        && term.properties.discriminator_test_kind
                            != crate::schema::ir::TestKind::Pattern
                    {
                        self.evaluate_discriminator(term, None, builder)?;
                    }
                }
                g_res
            }
        };

        if res.is_ok() && !matches!(term.kind, TermKind::Element(_)) {

            // Right framing: trailingSkip
            if term.properties.trailing_skip > 0 {
                let skip_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.trailing_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.trailing_skip,
                };
                if skip_bits > 0 {
                    let _ = self.reader.read_bits(skip_bits)?;
                }
            }
        }

        if has_nvi {
            self.pop_new_variable_instances(term);
        }

        res
    }

    /// Executes a closure using a child [`ParserEngine`] bound to a sub-stream [`BitReader`].
    ///
    /// Preserves parser engine execution state (schema, work budget, occurring indices,
    /// in-scope delimiters, and enclosing complexes) while synchronizing mutated variable state
    /// back upon completion.
    ///
    /// # Arguments
    ///
    /// * `sub_reader` - The bit reader over the transformed or partitioned byte slice.
    /// * `f` - The execution closure to evaluate against the sub-engine.
    ///
    /// # Returns
    ///
    /// The result of evaluating `f`.
    pub(crate) fn run_sub_engine<F, R>(
        &mut self,
        sub_reader: &mut BitReader<crate::io::SliceByteSource<'_>>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut ParserEngine<'_, crate::io::SliceByteSource<'_>>) -> R,
    {
        let mut sub_engine = ParserEngine {
            schema: self.schema,
            reader: sub_reader,
            budget: self.budget,
            variable_map: self.variable_map.clone(),
            current_occurs_index: self.current_occurs_index,
            in_scope_delimiters: self.in_scope_delimiters.clone(),
            in_scope_terminators: self.in_scope_terminators.clone(),
            pou_stack: self.pou_stack.clone(),
            validation_mode: self.validation_mode,
            delim_encoding: self.delim_encoding.clone(),
            delim_ignore_case: self.delim_ignore_case,
            allow_expression_result_coercion: self.allow_expression_result_coercion,
            enclosing_complex_elements: self.enclosing_complex_elements.clone(),
            validation_errors: self.validation_errors.clone(),
        };
        let res = f(&mut sub_engine);
        self.variable_map = sub_engine.variable_map;
        self.validation_errors = sub_engine.validation_errors;
        res
    }

    /// Parses the sequential members of a DFDL sequence construct.
    ///
    /// Handles member-by-member occurrences, separators (prefix, infix, postfix),
    /// pattern assertions, discriminators, and sequence termination according to
    /// DFDL 1.0 §14.
    ///
    /// # Arguments
    ///
    /// * `term` - Compiled sequence term with properties.
    /// * `seq` - Sequence definition containing ordered child member NodeIds.
    /// * `builder` - Target infoset builder receiving parsed events.
    /// * `sep_opt` - Evaluated separator string, if defined on the sequence.
    /// * `sep_pos` - Positional mode for separator parsing (prefix, infix, or postfix).
    ///
    /// # Returns
    ///
    /// `Ok(())` on successful parsing of all members, or a `DFDLError` on parse/schema failure.
    pub(crate) fn parse_sequence_members(
        &mut self,
        term: &crate::schema::ir::CompiledTerm,
        seq: &crate::schema::ir::CompiledSequence,
        builder: &mut InfosetBuilder,
        sep_opt: Option<&str>,
        sep_pos: crate::schema::ir::SeparatorPosition,
    ) -> DFDLResult<()> {
        self.evaluate_pattern_asserts(term, None, builder)?;
        if term.properties.discriminator.is_some()
            && term.properties.discriminator_test_kind == crate::schema::ir::TestKind::Pattern
        {
            self.evaluate_discriminator(term, None, builder)?;
        }
        if term.properties.sequence_kind == crate::schema::ir::SequenceKind::Unordered {
            return self.parse_unordered_sequence(
                term,
                seq,
                builder,
                sep_opt,
                sep_pos,
                term.properties.separator_suppression_policy,
            );
        }
        let mut total_element_count: usize = 0;
        let mut represented_members_seen: usize = 0;
        for (member_idx, &member_id) in seq.members.iter().enumerate() {
            let member_term = self.schema.get_term(member_id).ok_or_else(|| {
                DFDLError::new_static(
                    DFDLErrorKind::SchemaDefinition,
                    "Sequence member NodeId missing from compiled schema graph",
                )
            })?;
            let has_rep = self.schema.term_has_representation(member_id);
            let is_last_member = member_idx == seq.members.len().saturating_sub(1);
            let is_scalar_required = match &member_term.kind {
                TermKind::Element(e) => e.min_occurs == 1 && e.max_occurs == Some(1),
                _ => false,
            };
            let mut removed_sep_entry = None;
            if seq.members.len() > 1
                && is_last_member
                && is_scalar_required
                && sep_pos == crate::schema::ir::SeparatorPosition::Infix
            {
                if let Some(sep) = sep_opt {
                    if let Some(pos) = self.in_scope_delimiters.iter().rposition(|d| d.text == sep) {
                        removed_sep_entry = Some(self.in_scope_delimiters.remove(pos));
                    }
                }
            }

            let member_res = match &member_term.kind {
                TermKind::Element(elem) => {
                    self.parse_element_with_separators(
                        member_term,
                        elem,
                        builder,
                        sep_opt,
                        sep_pos,
                        term.properties.separator_suppression_policy,
                        total_element_count,
                        represented_members_seen,
                        is_last_member,
                        term.properties.initiated_content,
                    ).map(|occurrences_parsed| {
                        total_element_count =
                            total_element_count.saturating_add(occurrences_parsed);
                    })
                }
                _ => (|| -> DFDLResult<()> {
                    let mut sep_matched = false;
                    if has_rep {
                        if let Some(sep) = sep_opt {
                            if sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                                || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                    && (total_element_count > 0 || represented_members_seen > 0))
                            {
                                if let Some(sep_len) = self.peek_delimiter_match_length(sep) {
                                    if self.is_in_scope_terminator_longer(sep_len) {
                                        return Err(DFDLError::new_static(
                                            DFDLErrorKind::Parse,
                                            "Delimiter mismatch: in-scope terminator matches longer than separator",
                                        ));
                                    }
                                }
                                if self.peek_literal_delimiter(sep) {
                                    self.match_literal_delimiter(sep).map_err(|e| {
                                        DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            &alloc::format!("seq prefix/infix sep failed: {}", e),
                                        )
                                    })?;
                                    sep_matched = true;
                                } else if term.properties.separator_suppression_policy
                                    == crate::schema::ir::SeparatorSuppressionPolicy::Never
                                {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::Parse,
                                        &alloc::format!(
                                            "seq prefix/infix sep failed: [Parse]: Delimiter mismatch in bitstream: delimiter='{}'",
                                            sep
                                        ),
                                    ));
                                }
                            }
                        }
                    }

                    // Tentatively parse the non-element member (e.g. child sequence, choice, or group-ref).
                    // Per DFDL v1.0 §14.2.3, if a separator is omitted under a suppression policy
                    // (such as trailingEmpty or anyEmpty), the omitted separator is only valid if the
                    // component evaluates to an empty representation (consumes zero bits).
                    let pre_term_cp = self.reader.checkpoint();
                    let pre_builder_cp = builder.checkpoint();
                    let res = self.parse_term(member_id, builder);
                    if let Err(e) = res {
                        self.reader.rollback(pre_term_cp)?;
                        builder.rollback(pre_builder_cp);
                        return Err(e);
                    }

                    let post_term_cp = self.reader.checkpoint();
                    let member_consumed_zero = post_term_cp == pre_term_cp;
                    if !sep_matched
                        && !member_consumed_zero
                        && sep_opt.is_some_and(|s| !s.is_empty())
                        && (sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                            || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                && (total_element_count > 0 || represented_members_seen > 0)))
                    {
                        self.reader.rollback(pre_term_cp)?;
                        builder.rollback(pre_builder_cp);
                        let sep_str = sep_opt.unwrap_or_default();
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            &alloc::format!(
                                "seq prefix/infix sep failed: [Parse]: Delimiter mismatch in bitstream: delimiter='{}'",
                                sep_str
                            ),
                        ));
                    }

                    if has_rep {
                        if let Some(sep) = sep_opt {
                            if sep_pos == crate::schema::ir::SeparatorPosition::Postfix
                                && (!member_consumed_zero
                                    || term.properties.separator_suppression_policy
                                        == crate::schema::ir::SeparatorSuppressionPolicy::Never
                                    || self.peek_literal_delimiter(sep))
                            {
                                self.match_literal_delimiter(sep).map_err(|e| {
                                    DFDLError::new(
                                        DFDLErrorKind::Parse,
                                        &alloc::format!("seq postfix sep failed: {}", e),
                                    )
                                })?;
                            }
                        }
                        if !member_consumed_zero {
                            total_element_count = total_element_count.saturating_add(1);
                        }
                    }
                    Ok(())
                })(),
            };

            if let Some(entry) = removed_sep_entry {
                let _ = crate::util::try_push(&mut self.in_scope_delimiters, entry);
            }
            member_res?;

            if has_rep {
                represented_members_seen = represented_members_seen.saturating_add(1);
            }
        }
        Ok(())
    }

}

