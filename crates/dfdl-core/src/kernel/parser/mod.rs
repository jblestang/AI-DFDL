//! DFDL runtime streaming parser engine.
//!
//! Aligned with DFDL 1.0 Specification §9, §10, §11, §12.
//! Operates strictly under `#![no_std]` + `alloc`.

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
    pub(crate) active_variable_map: Option<crate::expr::variables::VariableMap>,
    pub(crate) current_occurs_index: usize,
    pub(crate) in_scope_delimiters: Vec<crate::schema::ir::InScopeDelimiter>,
    pub(crate) in_scope_terminators: Vec<crate::schema::ir::InScopeDelimiter>,
    pub(crate) pou_stack: Vec<PointOfUncertainty>,
    pub(crate) validation_mode: ValidationMode,
    /// Encoding of the term being parsed, so delimiters are matched in its character width
    /// (sub-byte encodings such as 7-bit packed ASCII do not use 8-bit characters).
    pub(crate) delim_encoding: crate::encoding::EncodingKind,
    /// Case sensitivity of the term being parsed (`dfdl:ignoreCase`).
    pub(crate) delim_ignore_case: bool,
    pub(crate) allow_expression_result_coercion: bool,
    pub(crate) enclosing_complex_elements: Vec<(String, usize, LengthUnits, crate::encoding::EncodingKind)>,
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
            active_variable_map: None,
            schema,
            reader,
            budget,
            current_occurs_index: 1,
            in_scope_delimiters: Vec::new(),
            in_scope_terminators: Vec::new(),
            pou_stack: Vec::new(),
            validation_mode: ValidationMode::Off,
            delim_encoding: crate::encoding::EncodingKind::Utf8,
            delim_ignore_case: false,
            allow_expression_result_coercion: true,
            enclosing_complex_elements: Vec::new(),
            validation_errors: Vec::new(),
        }
    }

    /// Returns a reference to the active variable map, falling back to the compiled schema's default map.
    #[inline]
    pub fn variable_map(&self) -> &crate::expr::variables::VariableMap {
        self.active_variable_map
            .as_ref()
            .unwrap_or(&self.schema.variable_map)
    }

    /// Returns a mutable reference to the active variable map, lazily cloning from schema if needed.
    #[inline]
    pub fn variable_map_mut(&mut self) -> &mut crate::expr::variables::VariableMap {
        self.active_variable_map
            .get_or_insert_with(|| self.schema.variable_map.clone())
    }

    /// Returns a checkpoint of the active variable map, or 0 if uninitialized.
    #[inline]
    pub(crate) fn checkpoint_variable_map(&self) -> usize {
        self.active_variable_map.as_ref().map_or(0, |m| m.checkpoint())
    }

    /// Rolls back the active variable map if initialized.
    #[inline]
    pub(crate) fn rollback_variable_map(&mut self, cp: usize) {
        if let Some(m) = self.active_variable_map.as_mut() {
            m.rollback(cp);
        }
    }

    /// Sets an external variable value on the parser's active variable map (§7.7).
    ///
    /// Per DFDL v1.0 §7.7, an external variable binding overrides the `defaultValue`
    /// of the variable defined by `dfdl:defineVariable`, maintaining its `Defined` state
    /// for subsequent runtime modifications or scoped instantiations.
    pub fn set_external_variable(&mut self, name: &str, value: &str) -> DFDLResult<()> {
        self.variable_map_mut().set_external_variable(name, value)
    }

    /// Sets the validation mode for the parser engine (§21).
    #[inline]
    pub fn set_validation_mode(&mut self, mode: ValidationMode) {
        self.validation_mode = mode;
    }

    /// Sets whether to escalate warnings to errors (daf:escalateWarningsToErrors).
    #[inline]
    pub fn set_escalate_warnings(&mut self, escalate: bool) {
        self.variable_map_mut().escalate_warnings = escalate;
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
    pub(crate) fn push_in_scope_delimiter(&mut self, text: String, ignore_case: bool, encoding: crate::encoding::EncodingKind) {
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
    pub(crate) fn push_in_scope_terminator(&mut self, text: String, ignore_case: bool, encoding: crate::encoding::EncodingKind) {
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
        let need_swap = self.delim_encoding != delim.encoding;
        let saved_enc = if need_swap {
            Some(core::mem::replace(&mut self.delim_encoding, delim.encoding.clone()))
        } else {
            None
        };
        self.delim_ignore_case = delim.ignore_case;
        let res = self.peek_literal_delimiter(&delim.text);
        self.delim_ignore_case = saved_case;
        if let Some(enc) = saved_enc {
            self.delim_encoding = enc;
        }
        res
    }

    /// Performs Mandatory Text Alignment (DFDL §12.1) to encoding boundary bits by skipping 0 to (align_bits - 1) bits.
    pub(crate) fn align_mandatory_text(&mut self, encoding: &crate::encoding::EncodingKind) -> DFDLResult<()> {
        let align_bits = encoding.mandatory_alignment_bits();
        if align_bits > 1 {
            let bit_pos = self.reader.position().0;
            let rem = bit_pos.checked_rem(align_bits).unwrap_or(0);
            if rem != 0 {
                let skip = align_bits.saturating_sub(rem);
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
        if term.properties.set_variables_compiled.is_empty() {
            return Ok(());
        }
        for (var_name, ast_opt, raw_str) in &term.properties.set_variables_compiled {
            let evaluated = if let Some(ref ast) = ast_opt {
                let (active_doc, current_path);
                let (doc_ref, path_ref) = if ast.needs_infoset_doc() {
                    let mut path = builder.current_path();
                    if let Some(name) = elem_name {
                        let clean_name = name.split(':').next_back().unwrap_or(name);
                        let last_seg = path.last_step().map(|s| s.local_name());
                        if last_seg != Some(clean_name) {
                            let _ = path.try_push(clean_name);
                        }
                    }
                    active_doc = Some(builder.active_doc());
                    current_path = path;
                    (active_doc.as_ref(), &current_path)
                } else {
                    current_path = crate::types::InfosetPath::root();
                    (None, &current_path)
                };
                let mut ctx = crate::expr::ExprContext::with_variable_map(
                    doc_ref,
                    path_ref,
                    &[],
                    Some(self.active_variable_map.as_ref().unwrap_or(&self.schema.variable_map)),
                    self.budget,
                )
                .with_occurs_index(self.current_occurs_index)
                .with_schema(self.schema)
                .with_namespaces(&term.properties.in_scope_namespaces)
                .with_enclosing_lengths(&self.enclosing_complex_elements);
                crate::expr::eval_expr(ast, &mut ctx)?
            } else {
                DfdlValue::String(raw_str.clone())
            };
            self.variable_map_mut()
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
        if term.properties.new_variable_instances_compiled.is_empty() {
            return Ok(());
        }
        for (var_name, ast_opt, raw_str_opt) in &term.properties.new_variable_instances_compiled {
            let local = var_name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&var_name.local_name);
            let is_unparse_only = self
                .active_variable_map
                .as_ref()
                .unwrap_or(&self.schema.variable_map)
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
                .is_some_and(|var| var.direction == crate::expr::variables::VariableDirection::UnparseOnly);
            if is_unparse_only {
                continue;
            }
            let evaluated = if let Some(ref ast) = ast_opt {
                let (active_doc, current_path);
                let (doc_ref, path_ref) = if ast.needs_infoset_doc() {
                    active_doc = Some(builder.active_doc());
                    current_path = builder.current_path();
                    (active_doc.as_ref(), &current_path)
                } else {
                    current_path = crate::types::InfosetPath::root();
                    (None, &current_path)
                };
                let mut ctx = crate::expr::ExprContext::with_variable_map(
                    doc_ref,
                    path_ref,
                    &[],
                    Some(self.active_variable_map.as_ref().unwrap_or(&self.schema.variable_map)),
                    self.budget,
                )
                .with_occurs_index(self.current_occurs_index)
                .with_schema(self.schema)
                .with_namespaces(&term.properties.in_scope_namespaces)
                .with_enclosing_lengths(&self.enclosing_complex_elements);
                Some(crate::expr::eval_expr(ast, &mut ctx)?)
            } else if let Some(ref raw_str) = raw_str_opt {
                Some(DfdlValue::String(raw_str.clone()))
            } else {
                None
            };
            self.variable_map_mut()
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
            let is_unparse_only = self
                .variable_map()
                .variables
                .iter()
                .find(|v| v.name.local_name == local || v.name.local_name == var_name.local_name)
                .is_some_and(|var| var.direction == crate::expr::variables::VariableDirection::UnparseOnly);
            if is_unparse_only {
                continue;
            }
            self.variable_map_mut().pop_variable_instance(var_name);
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
                let rem = current_pos.checked_rem(align_bits).unwrap_or(0);
                if rem > 0 {
                    let skip = align_bits.saturating_sub(rem);
                    let _ = self.reader.read_bits(skip)?;
                }
            }
        }

        // Enforce bitOrder change only on byte boundary (§11.2)
        // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
        if term.properties.bit_order != self.reader.bit_order() {
            let current_pos = self.reader.position().0;
            let rem = current_pos.checked_rem(8).unwrap_or(0);
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
                let eval_init = if let Some(ref init_prop) = term.properties.initiator_prop {
                    let evaluated = self.evaluate_delimiter_prop(init_prop, builder)?;
                    if evaluated.is_empty() {
                        None
                    } else {
                        Some(evaluated)
                    }
                } else if let Some(ref raw_init) = term.properties.initiator {
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

                let eval_sep = if let Some(ref sep_prop) = term.properties.separator_prop {
                    if sep_prop.as_constant().is_some_and(|s| s.is_empty()) {
                        None
                    } else {
                        let evaluated = self.evaluate_delimiter_prop(sep_prop, builder)?;
                        if evaluated.trim().is_empty() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::SchemaDefinition,
                                "Schema Definition Error: Property separator cannot be empty string",
                            ));
                        } else {
                            Some(evaluated)
                        }
                    }
                } else if let Some(ref raw_sep) = term.properties.separator {
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

                let eval_term = if let Some(ref term_prop) = term.properties.terminator_prop {
                    let evaluated = self.evaluate_delimiter_prop(term_prop, builder)?;
                    if evaluated.is_empty() {
                        None
                    } else {
                        Some(evaluated)
                    }
                } else if let Some(ref raw_term) = term.properties.terminator {
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
                let seq_res = if let Some(ref layer_name) = term.properties.layer {
                    if let Some(layer) = crate::kernel::layer::find_layer(layer_name) {
                        let rem_bytes = self.reader.remaining_bytes();
                        let cp = self.reader.checkpoint();
                        let mut raw_bytes = Vec::with_capacity(rem_bytes);
                        for _ in 0..rem_bytes {
                            raw_bytes.push(self.reader.read_bits(8)? as u8);
                        }
                        self.reader.rollback(cp)?;

                        let bit_limit = self.reader.bit_limit();
                        let current_position = self.reader.position().0;
                        let mut ctx = crate::kernel::layer::LayerParseContext {
                            variable_map: self.variable_map_mut(),
                            bit_limit,
                            current_position,
                        };

                        match layer.parse_wrap(&raw_bytes, &mut ctx)? {
                            crate::kernel::layer::LayerParseInput::Transformed {
                                data,
                                orig_byte_offsets,
                                orig_consumed_bytes,
                            } => {
                                let sub_source = crate::io::SliceByteSource::new(&data);
                                let mut sub_reader = crate::io::BitReader::new(
                                    sub_source,
                                    self.reader.bit_order(),
                                    term.properties.byte_order,
                                );
                                let res = self.run_sub_engine(&mut sub_reader, |sub| {
                                    sub.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
                                });
                                if res.is_ok() {
                                    if let Some(offsets) = orig_byte_offsets {
                                        let sub_consumed_bits = sub_reader.position().0;
                                        let sub_consumed_bytes = (sub_consumed_bits.saturating_add(7)) / 8;
                                        let orig_consumed = offsets
                                            .get(sub_consumed_bytes)
                                            .copied()
                                            .unwrap_or(raw_bytes.len());
                                        self.reader.skip_bits(orig_consumed.saturating_mul(8))?;
                                    } else if let Some(consumed) = orig_consumed_bytes {
                                        let skip_bytes = if self.reader.bit_limit().is_some() {
                                            raw_bytes.len()
                                        } else {
                                            consumed
                                        };
                                        self.reader.skip_bits(skip_bytes.saturating_mul(8))?;
                                    } else {
                                        self.reader.skip_bits(raw_bytes.len().saturating_mul(8))?;
                                    }
                                    let post_bit_limit = self.reader.bit_limit();
                                    let post_pos = self.reader.position().0;
                                    let mut post_ctx = crate::kernel::layer::LayerParseContext {
                                        variable_map: self.variable_map_mut(),
                                        bit_limit: post_bit_limit,
                                        current_position: post_pos,
                                    };
                                    layer.post_parse(&mut post_ctx)?;
                                }
                                res
                            }
                            crate::kernel::layer::LayerParseInput::InPlace {
                                bit_limit,
                                post_skip_bits,
                            } => {
                                let prev_limit = self.reader.bit_limit();
                                let target_limit = bit_limit.map(|offset| {
                                    let start_pos = self.reader.position().0;
                                    let limit = start_pos.saturating_add(offset);
                                    self.reader.set_bit_limit(Some(match prev_limit {
                                        Some(l) => l.min(limit),
                                        None => limit,
                                    }));
                                    limit
                                });

                                let res = self.parse_sequence_members(term, seq, builder, sep_opt, sep_pos);

                                if bit_limit.is_some() {
                                    self.reader.set_bit_limit(prev_limit);
                                }
                                if let Some(limit) = target_limit {
                                    if res.is_ok() {
                                        let current_pos = self.reader.position().0;
                                        if current_pos < limit {
                                            let skip = limit.saturating_sub(current_pos);
                                            self.reader.skip_bits(skip)?;
                                        }
                                        if let Some(post_skip) = post_skip_bits {
                                            self.reader.skip_bits(post_skip)?;
                                        }
                                    }
                                }
                                if res.is_ok() {
                                    let post_bit_limit = self.reader.bit_limit();
                                    let post_pos = self.reader.position().0;
                                    let mut post_ctx = crate::kernel::layer::LayerParseContext {
                                        variable_map: self.variable_map_mut(),
                                        bit_limit: post_bit_limit,
                                        current_position: post_pos,
                                    };
                                    layer.post_parse(&mut post_ctx)?;
                                }
                                res
                            }
                        }
                    } else {
                        self.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
                    }
                } else {
                    self.parse_sequence_members(term, seq, builder, sep_opt, sep_pos)
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
                    if term.properties.choice_dispatch_key_ast.is_some()
                        || term.properties.choice_dispatch_key.is_some()
                    {
                        let dynamic_ast;
                        let ast = match term.properties.choice_dispatch_key_ast.as_ref() {
                            Some(a) => a,
                            None => {
                                let expr_str = term
                                    .properties
                                    .choice_dispatch_key
                                    .as_deref()
                                    .unwrap_or("");
                                dynamic_ast = crate::expr::parse_expr(expr_str)?;
                                &dynamic_ast
                            }
                        };
                        let (active_doc, current_path);
                        let (doc_ref, path_ref) = if ast.needs_infoset_doc() {
                            active_doc = Some(builder.active_doc());
                            current_path = builder.current_path();
                            (active_doc.as_ref(), &current_path)
                        } else {
                            current_path = crate::types::InfosetPath::root();
                            (None, &current_path)
                        };
                        let (variable_map, schema, budget, reader) = (
                            self.active_variable_map.as_ref().unwrap_or(&self.schema.variable_map),
                            self.schema,
                            &mut *self.budget,
                            &mut *self.reader,
                        );
                        let reader_cell = core::cell::RefCell::new(reader);
                        let la_fn = |offset: usize, num_bits: usize| -> DFDLResult<u128> {
                            reader_cell.borrow_mut().lookahead_bits(offset, num_bits)
                        };
                        let mut ctx = crate::expr::ExprContext::with_variable_map(
                            doc_ref,
                            path_ref,
                            &[],
                            Some(variable_map),
                            budget,
                        )
                        .with_occurs_index(self.current_occurs_index)
                        .with_schema(schema)
                        .with_namespaces(&term.properties.in_scope_namespaces)
                        .with_lookahead(Some(&la_fn))
                        .with_enclosing_lengths(&self.enclosing_complex_elements);
                        let val = crate::expr::eval_expr(ast, &mut ctx)?;
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
                    let top_pou_idx = self.pou_stack.len().saturating_sub(1);

                    // DFDL §15.1.2: dfdl:choiceLength is always specified in bytes,
                    // regardless of any in-scope dfdl:lengthUnits.
                    let explicit_choice_bits = if term.properties.choice_length_kind == LengthKind::Explicit {
                        term.properties.choice_length.map(|l| l.saturating_mul(8))
                    } else {
                        None
                    };

                    for &branch_id in &choice.branches {
                        let reader_cp = self.reader.checkpoint();
                        let builder_cp = builder.checkpoint();
                        let vmap_cp = self.checkpoint_variable_map();
                        let val_err_cp = self.validation_errors.len();
                        let delims_cp = self.in_scope_delimiters.len();
                        let terms_cp = self.in_scope_terminators.len();

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
                                        if self.pou_stack.get(top_pou_idx).map(|p| p.is_discriminated).unwrap_or(false) {
                                            self.pou_stack.pop();
                                            self.reader.rollback(reader_cp)?;
                                            builder.rollback(builder_cp);
                                            self.rollback_variable_map(vmap_cp);
                                            self.validation_errors.truncate(val_err_cp);
                                            self.in_scope_delimiters.truncate(delims_cp);
                                            self.in_scope_terminators.truncate(terms_cp);
                                            return Err(DFDLError::new(
                                                DFDLErrorKind::Parse,
                                                "Parse Error: All Choice Alternatives Failed: Branch consumed data exceeding explicit choiceLength",
                                            ));
                                        }
                                        self.reader.rollback(reader_cp)?;
                                        builder.rollback(builder_cp);
                                        self.rollback_variable_map(vmap_cp);
                                        self.validation_errors.truncate(val_err_cp);
                                        self.in_scope_delimiters.truncate(delims_cp);
                                        self.in_scope_terminators.truncate(terms_cp);
                                        last_choice_error = Some(DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            "Parse Error: Branch consumed data exceeding explicit choiceLength",
                                        ));
                                        continue;
                                    }
                                    if consumed < choice_bits {
                                        let pad_bits = choice_bits.saturating_sub(consumed);
                                        // DFDL §15.1.2: ChoiceUnused region must exist in the input stream.
                                        // If skipping padding bits fails (e.g. truncated input), propagate error
                                        // with proper discriminator and backtracking semantics.
                                        if let Err(e) = self.reader.skip_bits(pad_bits) {
                                            if self.pou_stack.get(top_pou_idx).map(|p| p.is_discriminated).unwrap_or(false) {
                                                self.pou_stack.pop();
                                                self.reader.rollback(reader_cp)?;
                                                builder.rollback(builder_cp);
                                                self.rollback_variable_map(vmap_cp);
                                                self.validation_errors.truncate(val_err_cp);
                                                self.in_scope_delimiters.truncate(delims_cp);
                                                self.in_scope_terminators.truncate(terms_cp);
                                                let msg = alloc::format!(
                                                    "Parse Error: All Choice Alternatives Failed: {}",
                                                    e
                                                );
                                                return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                                            }
                                            last_choice_error = Some(e);
                                            self.reader.rollback(reader_cp)?;
                                            builder.rollback(builder_cp);
                                            self.rollback_variable_map(vmap_cp);
                                            self.validation_errors.truncate(val_err_cp);
                                            self.in_scope_delimiters.truncate(delims_cp);
                                            self.in_scope_terminators.truncate(terms_cp);
                                            continue;
                                        }
                                    }
                                }
                                choice_succeeded = true;
                                break;
                            }
                            Err(e) => {
                                if e.is_fatal() {
                                    return Err(e);
                                }
                                if self.pou_stack.get(top_pou_idx).map(|p| p.is_discriminated).unwrap_or(false) {
                                    self.pou_stack.pop();
                                    self.reader.rollback(reader_cp)?;
                                    builder.rollback(builder_cp);
                                    self.rollback_variable_map(vmap_cp);
                                    self.validation_errors.truncate(val_err_cp);
                                    self.in_scope_delimiters.truncate(delims_cp);
                                    self.in_scope_terminators.truncate(terms_cp);
                                    let msg = alloc::format!(
                                        "Parse Error: All Choice Alternatives Failed: {}",
                                        e
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                                }
                                last_choice_error = Some(e);
                                self.reader.rollback(reader_cp)?;
                                builder.rollback(builder_cp);
                                self.rollback_variable_map(vmap_cp);
                                self.validation_errors.truncate(val_err_cp);
                                self.in_scope_delimiters.truncate(delims_cp);
                                self.in_scope_terminators.truncate(terms_cp);
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
            active_variable_map: self.active_variable_map.clone(),
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
        if sub_engine.active_variable_map.is_some() {
            self.active_variable_map = sub_engine.active_variable_map;
        }
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

