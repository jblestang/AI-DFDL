//! DFDL Runtime Unparser Engine.
//!
//! Serializes an in-memory DFDL Infoset (`InfosetDocument`) into an outgoing bitstream (`BitWriter`)
//! according to a compiled schema graph (`CompiledSchema`).
//!
//! Aligned with DFDL 1.0 Specification §11, §12. Panic-free `#![no_std]` + `alloc`.

pub mod element;
pub mod framing;
pub mod suspensions;
#[cfg(test)]
mod tests;

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::state::ElementState;
use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use crate::infoset::value::DfdlValue;
use crate::io::bitstream::BitWriter;
use crate::io::traits::ByteSink;
use crate::io::VecByteSink;
use crate::limits::WorkBudget;
use crate::schema::ir::{CompiledSchema, NodeId, TermKind};
use crate::util::try_push;

pub use element::measure_element_content_bits;
pub use framing::decode_unparse_delimiter;
pub(crate) use framing::{fill_byte_value, write_fill_padding};
use element::validate_unparse_value;

/// DFDL Runtime Streaming Unparser Engine.
pub struct UnparserEngine<'a, S: ByteSink> {
    pub(crate) schema: &'a CompiledSchema,
    pub(crate) writer: &'a mut BitWriter<S>,
    pub(crate) budget: &'a mut WorkBudget,
    pub(crate) active_variable_map: Option<crate::expr::variables::VariableMap>,
    pub(crate) doc: Option<&'a InfosetDocument>,
    pub(crate) base_path: crate::types::InfosetPath,
    pub(crate) current_path: crate::types::InfosetPath,
    pub(crate) path_stack: Vec<(&'a str, Option<usize>)>,
    pub(crate) current_occurs_index: usize,
    pub(crate) child_cursor: usize,
    pub(crate) active_delimiters: Vec<String>,
    pub(crate) current_namespaces: &'a [(alloc::string::String, alloc::string::String)],
    pub(crate) ovc_values: alloc::collections::BTreeMap<alloc::string::String, DfdlValue>,
    pub(crate) validation_mode: crate::kernel::ValidationMode,
    pub(crate) validation_errors: Vec<DFDLError>,
    /// Enables verbose debugging trace.
    pub debug_trace: bool,
}

impl<'a, S: ByteSink> UnparserEngine<'a, S> {
    /// Constructs a new [`UnparserEngine`].
    #[inline]
    pub fn new(
        schema: &'a CompiledSchema,
        writer: &'a mut BitWriter<S>,
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            active_variable_map: None,
            schema,
            writer,
            budget,
            doc: None,
            base_path: crate::types::InfosetPath::root(),
            current_path: crate::types::InfosetPath::root(),
            path_stack: Vec::with_capacity(16),
            current_occurs_index: 1,
            child_cursor: 0,
            active_delimiters: Vec::new(),
            current_namespaces: &[],
            ovc_values: alloc::collections::BTreeMap::new(),
            validation_mode: crate::kernel::ValidationMode::Off,
            validation_errors: Vec::new(),
            debug_trace: false,
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

    /// Sets the validation mode for the unparser (§21).
    #[inline]
    pub fn set_validation_mode(&mut self, mode: crate::kernel::ValidationMode) {
        self.validation_mode = mode;
    }

    /// Returns a slice of collected validation errors.
    #[inline]
    pub fn validation_errors(&self) -> &[DFDLError] {
        &self.validation_errors
    }

    /// Sets an external variable value on the unparser's active variable map (§7.7).
    ///
    /// Per DFDL v1.0 §7.7, an external variable binding overrides the `defaultValue`
    /// of the variable defined by `dfdl:defineVariable`, maintaining its `Defined` state
    /// for subsequent runtime modifications or scoped instantiations.
    pub fn set_external_variable(&mut self, name: &str, value: &str) -> DFDLResult<()> {
        self.variable_map_mut().set_external_variable(name, value)
    }

    /// Sets whether to escalate warnings to errors (daf:escalateWarningsToErrors).
    #[inline]
    pub fn set_escalate_warnings(&mut self, escalate: bool) {
        self.variable_map_mut().escalate_warnings = escalate;
    }

    /// Unparses an [`InfosetDocument`] into the output bitstream according to the compiled schema.
    pub fn unparse_document(&mut self, doc: &'a InfosetDocument) -> DFDLResult<()> {
        self.doc = Some(doc);
        self.current_path = crate::types::InfosetPath::root();
        self.path_stack.clear();
        self.child_cursor = 0;
        let root_term_id = self.schema.root_element_id;
        let root_elem = doc.root.as_ref().ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::Unparse,
                "Cannot unparse an empty InfosetDocument with no root element",
            )
        })?;
        let root_term = self.schema.get_term(root_term_id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;
        if root_term.name.local_name != root_elem.name.local_name
            || root_term.name.namespace != root_elem.name.namespace
        {
            let expected_qname = match &root_term.name.namespace {
                Some(ns) => alloc::format!("{{{}}}{}", ns.as_str(), root_term.name.local_name),
                None => alloc::format!("{{}}{}", root_term.name.local_name),
            };
            let received_qname = match &root_elem.name.namespace {
                Some(ns) => alloc::format!("{{{}}}{}", ns.as_str(), root_elem.name.local_name),
                None => alloc::format!("{{}}{}", root_elem.name.local_name),
            };
            let msg = alloc::format!(
                "Unparse Error: expected element start '{}', received '{}'",
                expected_qname, received_qname
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }

        if !root_term.properties.in_scope_namespaces.is_empty() {
            self.current_namespaces = &root_term.properties.in_scope_namespaces;
        }

        self.unparse_element(root_term_id, root_elem)?;
        self.writer.flush()?;

        if self.validation_mode != crate::kernel::ValidationMode::Off && !self.validation_errors.is_empty() {
            let first_val_err = self.validation_errors.remove(0);
            return Err(first_val_err);
        }

        Ok(())
    }

    pub(crate) fn unparse_term(&mut self, term_id: NodeId, parent_elem: &InfosetElement) -> DFDLResult<()> {
        let prev_ns = self.current_namespaces;
        let res = self.unparse_term_internal(term_id, parent_elem);
        self.current_namespaces = prev_ns;
        res
    }

    pub(crate) fn unparse_term_internal(&mut self, term_id: NodeId, parent_elem: &InfosetElement) -> DFDLResult<()> {
        self.budget.consume(1)?;

        let term = self.schema.get_term(term_id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;

        if !term.properties.in_scope_namespaces.is_empty() {
            self.current_namespaces = &term.properties.in_scope_namespaces;
        }

        let is_element = matches!(term.kind, TermKind::Element(_));

        let has_nvi = !term.properties.new_variable_instances.is_empty();
        if has_nvi {
            self.execute_new_variable_instances(term)?;
        }

        if !is_element {
            self.execute_set_variables(term)?;
            // Left framing per DFDL §12 grammar: LeftFraming = AlignmentFill LeadingSkip.
            let align_bits = match term.properties.alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => {
                    term.properties.alignment.saturating_mul(8)
                }
                crate::schema::ir::AlignmentUnits::Bits => term.properties.alignment,
            };
            if term.properties.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
                && align_bits > 1
            {
                let current_pos = self.writer.position().0;
                let rem = current_pos.checked_rem(align_bits).unwrap_or(0);
                if rem > 0 {
                    let pad = align_bits.saturating_sub(rem);
                    let fill = fill_byte_value(&term.properties)?;
                    write_fill_padding(self.writer, fill, pad)?;
                }
            }

            if term.properties.leading_skip > 0 {
                let skip_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.leading_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.leading_skip,
                };
                let fill = fill_byte_value(&term.properties)?;
                write_fill_padding(self.writer, fill, skip_bits)?;
            }

            // Enforce bitOrder change only on byte boundary (§11.2)
            // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
            if term.properties.bit_order != self.writer.bit_order() {
                let current_pos = self.writer.position().0;
                let rem = current_pos % 8;
                if rem != 0 {
                    let bit_in_byte_1based = rem.saturating_add(1);
                    let msg = alloc::format!(
                        "Schema Definition Error: Can only change bitOrder on a byte boundary. Bit position {} is not on a byte boundary",
                        bit_in_byte_1based
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                self.writer.set_bit_order(term.properties.bit_order);
            }

            if let Some(ref init) = term.properties.initiator {
                self.write_evaluated_delimiter(init, &term.properties)?;
            }
        }

        let res = match &term.kind {
            TermKind::Element(ref el) => {
                let (c_start, c_end) = self.find_child_elements_range(parent_elem, term_id);
                let children_count = c_end.saturating_sub(c_start);
                if children_count == 0 {
                    if (term.properties.is_hidden
                        && (el.min_occurs > 0
                            || el.default_value.is_some()
                            || term.properties.output_value_calc.is_some()))
                        || term.properties.output_value_calc.is_some()
                    {
                        let dummy_elem = if let Some(ref def_val) = el.default_value {
                            InfosetElement::simple(
                                el.name.clone(),
                                ElementState::Value(def_val.clone()),
                            )
                            .with_hidden(true)
                        } else {
                            InfosetElement::complex(el.name.clone()).with_hidden(true)
                        };
                        self.unparse_element(term_id, &dummy_elem)?;
                    } else if el.min_occurs > 0
                        && el.default_value.is_none()
                        && term.properties.occurs_count_kind != crate::schema::ir::OccursCountKind::Parsed
                    {
                        if let Some(child_node) = parent_elem.children.get(self.child_cursor) {
                            let received = match child_node {
                                InfosetNode::Element(e) => e.name.clark_notation(),
                            };
                            let expected = term.name.clark_notation();
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: expected element start for '{}', but received element start '{}'",
                                    expected, received
                                ),
                            ));
                        } else {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Required element '{}' (minOccurs={}) missing from infoset",
                                    term.name.clark_notation(),
                                    el.min_occurs
                                ),
                            ));
                        }
                    }
                } else {
                    if term.properties.occurs_count_kind != crate::schema::ir::OccursCountKind::Parsed {
                        if children_count < el.min_occurs && el.default_value.is_none() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Expected at least {} occurrence(s) of element '{}' for unparsing, but found {}",
                                    el.min_occurs,
                                    el.name.local_name,
                                    children_count
                                ),
                            ));
                        }
                        if term.properties.occurs_count_kind != crate::schema::ir::OccursCountKind::Expression {
                            if let Some(max) = el.max_occurs {
                                if children_count > max {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::Unparse,
                                        &alloc::format!(
                                            "Unparse Error: Expected array end event for '{}', received element start event (found {} occurrences exceeding maxOccurs {})",
                                            el.name.local_name,
                                            children_count,
                                            max
                                        ),
                                    ));
                                }
                            }
                        }
                    } else if self.validation_mode != crate::kernel::ValidationMode::Off {
                        if children_count < el.min_occurs {
                            let msg = alloc::format!(
                                "Validation Error: element '{}' occurs {} times, less than minOccurs {}",
                                el.name.local_name, children_count, el.min_occurs
                            );
                            let _ = crate::util::try_push(
                                &mut self.validation_errors,
                                DFDLError::new(DFDLErrorKind::Validation, &msg),
                            );
                        }
                        if let Some(max) = el.max_occurs {
                            if children_count > max {
                                let msg = alloc::format!(
                                    "Validation Error: element '{}' occurs {} times, exceeding maxOccurs {}",
                                    el.name.local_name, children_count, max
                                );
                                let _ = crate::util::try_push(
                                    &mut self.validation_errors,
                                    DFDLError::new(DFDLErrorKind::Validation, &msg),
                                );
                            }
                        }
                    }
                    let is_array = children_count > 1
                        || match term.kind {
                            TermKind::Element(ref el) => {
                                el.max_occurs.is_none()
                                    || el.max_occurs.is_some_and(|m| m > 1)
                                    || term.properties.occurs_count_kind
                                        != crate::schema::ir::OccursCountKind::Implicit
                            }
                            _ => false,
                        };
                    if let Some(children_slice) = parent_elem.children.get(c_start..c_end) {
                        for (idx, child_node) in children_slice.iter().enumerate() {
                            let InfosetNode::Element(ref child_elem) = child_node;
                            let prev_idx = self.current_occurs_index;
                            let occurs_idx = if is_array {
                                let oi = idx.saturating_add(1);
                                self.current_occurs_index = oi;
                                Some(oi)
                            } else {
                                None
                            };
                            let elem_res = self.unparse_element_with_index(term_id, child_elem, occurs_idx);
                            self.current_occurs_index = prev_idx;
                            elem_res?;
                        }
                    }
                }
                Ok(())
            }
            TermKind::Sequence(seq) => {
                if let Some(ref layer_name) = term.properties.layer {
                    if let Some(layer) = crate::kernel::layer::find_layer(layer_name) {
                        let sink = VecByteSink::new();
                        let mut sub_writer = crate::io::bitstream::BitWriter::new(
                            sink,
                            self.writer.bit_order(),
                            term.properties.byte_order,
                        );
                        let mut sub_unparser = UnparserEngine::new(
                            self.schema,
                            &mut sub_writer,
                            self.budget,
                        );
                        sub_unparser.doc = self.doc;
                        sub_unparser.base_path = self.base_path.clone();
                        sub_unparser.current_path = self.current_path.clone();
                        sub_unparser.path_stack = self.path_stack.clone();
                        sub_unparser.active_variable_map = self.active_variable_map.clone();
                        sub_unparser.child_cursor = self.child_cursor;
                        sub_unparser.validation_mode = self.validation_mode;
                        sub_unparser.debug_trace = self.debug_trace;

                        for &member_id in &seq.members {
                            sub_unparser.unparse_term(member_id, parent_elem)?;
                        }

                        self.child_cursor = sub_unparser.child_cursor;
                        if sub_unparser.active_variable_map.is_some() {
                            self.active_variable_map = sub_unparser.active_variable_map;
                        }
                        self.validation_errors.extend(sub_unparser.validation_errors);

                        sub_writer.flush()?;
                        let mut buf = sub_writer.into_sink().into_vec();

                        let mut ctx = crate::kernel::layer::LayerUnparseContext {
                            variable_map: self.variable_map_mut(),
                        };
                        layer.unparse_transform(&mut buf, &mut ctx)?;

                        self.writer.write_bytes(&buf)?;
                        return Ok(());
                    }
                }

                let saved_delims_len = self.active_delimiters.len();
                if let Some(ref sep) = term.properties.separator {
                    if !sep.is_empty() {
                        let sep_tokens = self.extract_delimiter_tokens(sep, &term.properties);
                        self.active_delimiters.extend(sep_tokens);
                    }
                }
                if let Some(ref term_str) = term.properties.terminator {
                    if !term_str.is_empty() {
                        let term_tokens = self.extract_delimiter_tokens(term_str, &term.properties);
                        self.active_delimiters.extend(term_tokens);
                    }
                }
                let sep_opt = term.properties.separator.as_deref().filter(|s| !s.is_empty());
                let sep_pos = term.properties.separator_position;
                let mut total_element_count: usize = 0;

                for (m_idx, &member_id) in seq.members.iter().enumerate() {
                    let needs_trailing_check = sep_opt.is_some()
                        && matches!(
                            term.properties.separator_suppression_policy,
                            crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict
                        );
                    let has_subsequent_output = if needs_trailing_check {
                        seq.members
                            .get(m_idx.saturating_add(1)..)
                            .is_some_and(|subsequent| {
                                subsequent
                                    .iter()
                                    .any(|&sub_m| self.sequence_member_has_output(parent_elem, sub_m))
                            })
                    } else {
                        true
                    };
                    let (c_start, c_end) = self.find_child_elements_range(parent_elem, member_id);
                    let children_count = c_end.saturating_sub(c_start);
                    if children_count == 0 {
                        let is_optional_empty_elem = if let Some(m_term) = self.schema.get_term(member_id) {
                            if let TermKind::Element(ref el) = m_term.kind {
                                el.min_occurs == 0
                                    && el.default_value.is_none()
                                    && m_term.properties.output_value_calc.is_none()
                                    && !m_term.properties.is_hidden
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if is_optional_empty_elem {
                            let is_trailing = !has_subsequent_output;
                            let suppress = match term.properties.separator_suppression_policy {
                                crate::schema::ir::SeparatorSuppressionPolicy::Never => false,
                                crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty => true,
                                crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict => is_trailing,
                            };
                            if suppress && sep_opt.is_some() {
                                continue;
                            }
                        }

                        if let Some(sep) = sep_opt {
                            if sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                                || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                    && total_element_count > 0)
                            {
                                self.write_evaluated_delimiter(sep, &term.properties)?;
                            }
                        }
                        let start_pos = self.writer.position().0;
                        self.unparse_term(member_id, parent_elem)?;
                        let bits_written = self.writer.position().0.saturating_sub(start_pos);
                        if let Some(sep) = sep_opt {
                            if sep_pos == crate::schema::ir::SeparatorPosition::Postfix {
                                let suppress = if bits_written == 0 {
                                    match term.properties.separator_suppression_policy {
                                        crate::schema::ir::SeparatorSuppressionPolicy::Never => false,
                                        crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty => true,
                                        crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                        | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict => !has_subsequent_output,
                                    }
                                } else {
                                    false
                                };
                                if !suppress {
                                    self.write_evaluated_delimiter(sep, &term.properties)?;
                                }
                            }
                        }
                        total_element_count = total_element_count.saturating_add(1);
                    } else {
                        if let Some(member_term) = self.schema.get_term(member_id) {
                            if let TermKind::Element(ref el) = member_term.kind {
                                if member_term.properties.occurs_count_kind != crate::schema::ir::OccursCountKind::Parsed {
                                    if member_term.properties.occurs_count_kind
                                        != crate::schema::ir::OccursCountKind::Expression
                                    {
                                        if let Some(max) = el.max_occurs {
                                            if children_count > max {
                                                return Err(DFDLError::new(
                                                    DFDLErrorKind::Unparse,
                                                    &alloc::format!(
                                                        "Unparse Error: Expected array end event for '{}', received element start event (found {} occurrences exceeding maxOccurs {})",
                                                        el.name.local_name,
                                                        children_count,
                                                        max
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    if children_count < el.min_occurs && el.default_value.is_none() {
                                        let unexp = parent_elem.children.get(self.child_cursor).map(|node| match node {
                                            InfosetNode::Element(e) => e.name.clark_notation(),
                                        });
                                        let msg = if let Some(found) = unexp {
                                            alloc::format!(
                                                "Unparse Error: expected element start for '{}', but received element start '{}'",
                                                el.name.clark_notation(),
                                                found
                                            )
                                        } else {
                                            alloc::format!(
                                                "Unparse Error: expected element start '{}', but found 0 occurrences (at least {} required)",
                                                el.name.clark_notation(),
                                                el.min_occurs
                                            )
                                        };
                                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                                    }
                                }
                            }
                        }
                        let member_term = self.schema.get_term(member_id);
                        let is_array = children_count > 1
                            || member_term.is_some_and(|m_term| match m_term.kind {
                                TermKind::Element(ref el) => {
                                    el.max_occurs.is_none()
                                        || el.max_occurs.is_some_and(|m| m > 1)
                                        || m_term.properties.occurs_count_kind
                                            != crate::schema::ir::OccursCountKind::Implicit
                                }
                                _ => false,
                            });
                        let min_occurs = member_term
                            .and_then(|m| match m.kind {
                                TermKind::Element(ref el) => Some(el.min_occurs),
                                _ => None,
                            })
                            .unwrap_or(1);

                        if let Some(children_slice) = parent_elem.children.get(c_start..c_end) {
                            for (idx, child_node) in children_slice.iter().enumerate() {
                                let InfosetNode::Element(ref child_elem) = child_node;
                                let is_required = if is_array {
                                    idx < min_occurs
                                } else {
                                    min_occurs >= 1
                                };

                                // Per DFDL §14.2.3: Separators can only be suppressed if a separator is defined on the sequence
                                // and the occurrence is not required. Bypassing empty/trailing checks avoids expensive traversal.
                                let suppress_sep = if sep_opt.is_some() && !is_required {
                                    let is_empty = self.is_empty_infoset_element(child_elem, member_id);
                                    if is_empty {
                                        match term.properties.separator_suppression_policy {
                                            crate::schema::ir::SeparatorSuppressionPolicy::Never => false,
                                            crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty => true,
                                            crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                            | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict => {
                                                !has_subsequent_output
                                                    && children_slice
                                                        .get(idx.saturating_add(1)..)
                                                        .is_none_or(|rem| {
                                                            rem.iter().all(|node| match node {
                                                                InfosetNode::Element(ref c) => self.is_empty_infoset_element(c, member_id),
                                                            })
                                                        })
                                            }
                                        }
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };

                                let prev_idx = self.current_occurs_index;
                                let occurs_idx = if is_array {
                                    let oi = idx.saturating_add(1);
                                    self.current_occurs_index = oi;
                                    Some(oi)
                                } else {
                                    None
                                };
                                if suppress_sep {
                                    self.current_occurs_index = prev_idx;
                                    continue;
                                }
                                if let Some(sep) = sep_opt {
                                    if sep_pos == crate::schema::ir::SeparatorPosition::Prefix
                                        || (sep_pos == crate::schema::ir::SeparatorPosition::Infix
                                            && total_element_count > 0)
                                    {
                                        let mta_bits = crate::encoding::encoding_unit_bits(&term.properties.encoding).max(1);
                                        if mta_bits > 1 {
                                            let current_pos = self.writer.position().0;
                                            let rem = current_pos.checked_rem(mta_bits).unwrap_or(0);
                                            if rem > 0 {
                                                let pad = mta_bits.saturating_sub(rem);
                                                let fill = fill_byte_value(&term.properties)?;
                                                write_fill_padding(self.writer, fill, pad)?;
                                            }
                                        }
                                        self.write_evaluated_delimiter(sep, &term.properties)?;
                                    }
                                }
                                let start_pos = self.writer.position().0;
                                let elem_res = self.unparse_element_with_index(member_id, child_elem, occurs_idx);
                                self.current_occurs_index = prev_idx;
                                let bits_written = self.writer.position().0.saturating_sub(start_pos);
                                if let Some(sep) = sep_opt {
                                    if sep_pos == crate::schema::ir::SeparatorPosition::Postfix {
                                        let suppress_postfix = bits_written == 0
                                            && match term.properties.separator_suppression_policy {
                                                crate::schema::ir::SeparatorSuppressionPolicy::Never => false,
                                                crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty => true,
                                                crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                                | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict => {
                                                    idx.saturating_add(1) >= children_count && !has_subsequent_output
                                                }
                                            };
                                        if !suppress_postfix {
                                            let mta_bits = crate::encoding::encoding_unit_bits(&term.properties.encoding).max(1);
                                            if mta_bits > 1 {
                                                let current_pos = self.writer.position().0;
                                                let rem = current_pos.checked_rem(mta_bits).unwrap_or(0);
                                                if rem > 0 {
                                                    let pad = mta_bits.saturating_sub(rem);
                                                    let fill = fill_byte_value(&term.properties)?;
                                                    write_fill_padding(self.writer, fill, pad)?;
                                                }
                                            }
                                            self.write_evaluated_delimiter(sep, &term.properties)?;
                                        }
                                    }
                                }
                                self.current_occurs_index = prev_idx;
                                elem_res?;
                                total_element_count = total_element_count.saturating_add(1);
                            }
                        }
                    }
                }
                self.active_delimiters.truncate(saved_delims_len);
                Ok(())
            }
            TermKind::Choice(choice) => {
                // Per DFDL 1.0 §15.1.3: choiceDispatchKey is used ONLY during parsing.
                // On unparsing: 1. Match child element in infoset; 2. First branch that can be empty.
                let chosen_branch = if let Some(&branch_id) = choice.branches.iter().find(|&&b_id| self.branch_matches_current_element(b_id, parent_elem)) {
                    branch_id
                } else if let Some(&branch_id) = choice.branches.iter().find(|&&b_id| self.branch_can_be_empty(b_id)) {
                    branch_id
                } else if let Some(child_node) = parent_elem.children.get(self.child_cursor) {
                    let child_name = match child_node {
                        InfosetNode::Element(e) => e.name.clark_notation(),
                    };
                    let mut branch_names = Vec::new();
                    for &b_id in &choice.branches {
                        if let Some(b_term) = self.schema.get_term(b_id) {
                            let _ = try_push(&mut branch_names, b_term.name.clark_notation());
                        }
                    }
                    let branch_list = branch_names.join(", ");
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Found next element '{}', expected one of branches: {}",
                            child_name,
                            branch_list
                        ),
                    ));
                } else if let Some(&first_branch) = choice.branches.first() {
                    first_branch
                } else {
                    return Ok(());
                };

                let start_pos = self.writer.position().0;
                self.unparse_term(chosen_branch, parent_elem)?;
                let bits_written = self.writer.position().0.saturating_sub(start_pos);

                if term.properties.choice_length_kind == crate::schema::ir::LengthKind::Explicit {
                    if let Some(target_len) = term.properties.choice_length {
                        // DFDL §15.1.2: dfdl:choiceLength is always specified in bytes,
                        // regardless of any in-scope dfdl:lengthUnits.
                        let target_bits = target_len.saturating_mul(8);
                        if bits_written < target_bits {
                            let pad_bits = target_bits.saturating_sub(bits_written);
                            let fill_byte = term.properties.fill_byte;
                            let full_bytes = pad_bits.checked_div(8).unwrap_or(0);
                            for _ in 0..full_bytes {
                                self.writer.write_bits(fill_byte as u64, 8)?;
                            }
                            let rem_bits = pad_bits.checked_rem(8).unwrap_or(0);
                            if rem_bits > 0 {
                                let shift = 8usize.saturating_sub(rem_bits);
                                self.writer.write_bits((fill_byte >> shift) as u64, rem_bits)?;
                            }
                        } else if bits_written > target_bits {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Choice branch data exceeded choiceLength (wrote {} bits, choiceLength is {} bits)",
                                    bits_written,
                                    target_bits
                                ),
                            ));
                        }
                    }
                }
                Ok(())
            }
            TermKind::GroupRef(target_id) => self.unparse_term(*target_id, parent_elem),
        };

        if res.is_ok() && !is_element {
            if let Some(ref term_str) = term.properties.terminator {
                self.write_evaluated_delimiter(term_str, &term.properties)?;
            }

            // Right framing: trailingSkip
            if term.properties.trailing_skip > 0 {
                let skip_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.trailing_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.trailing_skip,
                };
                let fill = fill_byte_value(&term.properties)?;
                write_fill_padding(self.writer, fill, skip_bits)?;
            }
        }

        if has_nvi {
            self.pop_new_variable_instances(term);
        }

        res
    }

    /// Evaluates a raw delimiter property, selecting the first alternative and decoding entities.
    pub(crate) fn branch_matches_current_element(
        &self,
        branch_term_id: NodeId,
        parent_elem: &InfosetElement,
    ) -> bool {
        let current_child = match parent_elem.children.get(self.child_cursor) {
            Some(InfosetNode::Element(e)) => e,
            _ => return false,
        };
        self.branch_matches_element(branch_term_id, &current_child.name)
    }

    pub(crate) fn branch_matches_element(
        &self,
        branch_term_id: NodeId,
        target_name: &crate::types::QName,
    ) -> bool {
        let term = match self.schema.get_term(branch_term_id) {
            Some(t) => t,
            None => return false,
        };
        match &term.kind {
            TermKind::Element(el) => {
                if term.properties.is_hidden {
                    return false;
                }
                if el.name.local_name != target_name.local_name {
                    return false;
                }
                match (&el.name.namespace, &target_name.namespace) {
                    (Some(ns1), Some(ns2)) => ns1 == ns2,
                    (None, None) => true,
                    _ => false,
                }
            }
            TermKind::Sequence(seq) => {
                for &m_id in &seq.members {
                    if self.branch_matches_element(m_id, target_name) {
                        return true;
                    }
                    if !self.branch_can_be_empty(m_id) {
                        break;
                    }
                }
                false
            }
            TermKind::Choice(ch) => ch
                .branches
                .iter()
                .any(|&b_id| self.branch_matches_element(b_id, target_name)),
            TermKind::GroupRef(target_id) => self.branch_matches_element(*target_id, target_name),
        }
    }

    /// Recursively checks if a branch term can legally produce zero unparsed infoset events.
    pub(crate) fn branch_can_be_empty(&self, branch_term_id: NodeId) -> bool {
        let term = match self.schema.get_term(branch_term_id) {
            Some(t) => t,
            None => return false,
        };
        match &term.kind {
            TermKind::Element(el) => {
                if el.min_occurs == 0 {
                    return true;
                }
                if (el.default_value.is_some() && el.max_occurs == Some(1))
                    || term.properties.output_value_calc.is_some()
                    || el.is_nillable
                {
                    return true;
                }
                if term.properties.input_value_calc.is_some() {
                    return false;
                }
                match el.type_ir {
                    crate::schema::ir::CompiledType::Complex(complex_id) => {
                        self.branch_can_be_empty(complex_id)
                    }
                    crate::schema::ir::CompiledType::Simple(_) => false,
                }
            }
            TermKind::Sequence(seq) => seq
                .members
                .iter()
                .all(|&m_id| self.branch_can_be_empty(m_id)),
            TermKind::Choice(ch) => ch
                .branches
                .iter()
                .any(|&b_id| self.branch_can_be_empty(b_id)),
            TermKind::GroupRef(target_id) => self.branch_can_be_empty(*target_id),
        }
    }

    /// Finds the range of contiguous child elements in `parent.children` matching `term_id`.
    #[inline]
    pub(crate) fn find_child_elements_range(
        &mut self,
        parent: &InfosetElement,
        term_id: NodeId,
    ) -> (usize, usize) {
        let term = match self.schema.get_term(term_id) {
            Some(t) => t,
            None => return (self.child_cursor, self.child_cursor),
        };
        // Hidden elements are not in the infoset per DFDL §14.4
        if term.properties.is_hidden {
            return (self.child_cursor, self.child_cursor);
        }
        let matches_term = |name: &crate::types::QName| -> bool {
            if name.local_name != term.name.local_name {
                return false;
            }
            match (&name.namespace, &term.name.namespace) {
                (Some(ns1), Some(ns2)) => ns1 == ns2,
                (None, None) => true,
                _ => false,
            }
        };
        let start = self.child_cursor;
        let mut c = start;
        while let Some(child_node) = parent.children.get(c) {
            match child_node {
                InfosetNode::Element(ref elem) => {
                    if matches_term(&elem.name) {
                        c = c.saturating_add(1);
                    } else {
                        break;
                    }
                }
            }
        }
        self.child_cursor = c;
        (start, c)
    }

    #[allow(dead_code)]
    pub(crate) fn find_child_elements<'e>(
        &mut self,
        parent: &'e InfosetElement,
        term_id: NodeId,
    ) -> Vec<&'e InfosetElement> {
        let (start, end) = self.find_child_elements_range(parent, term_id);
        let count = end.saturating_sub(start);
        if count == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(count);
        if let Some(children_slice) = parent.children.get(start..end) {
            for child_node in children_slice {
                let InfosetNode::Element(ref elem) = child_node;
                result.push(elem);
            }
        }
        result
    }

    /// Determines whether a sequence member produces any unparsed output from the infoset,
    /// used to evaluate trailing empty separator suppression per DFDL §14.2.
    pub(crate) fn sequence_member_has_output(&self, parent: &InfosetElement, term_id: NodeId) -> bool {
        let term = match self.schema.get_term(term_id) {
            Some(t) => t,
            None => return false,
        };
        match &term.kind {
            TermKind::Element(el) => {
                let matches_term = |name: &crate::types::QName| -> bool {
                    name.local_name == term.name.local_name
                        && match (&name.namespace, &term.name.namespace) {
                            (Some(ns1), Some(ns2)) => ns1 == ns2,
                            (None, None) => true,
                            _ => false,
                        }
                };
                let has_child = parent.children.iter().any(|c| match c {
                    InfosetNode::Element(e) => matches_term(&e.name),
                });
                has_child
                    || el.default_value.is_some()
                    || term.properties.output_value_calc.is_some()
            }
            TermKind::Sequence(seq) => {
                seq.members.iter().any(|&m| self.sequence_member_has_output(parent, m))
            }
            TermKind::Choice(choice) => {
                choice.branches.iter().any(|&b| self.sequence_member_has_output(parent, b))
            }
            TermKind::GroupRef(target_id) => {
                self.sequence_member_has_output(parent, *target_id)
            }
        }
    }

    /// Determines whether an infoset element is considered empty for separator suppression per DFDL §14.2.3.
    pub(crate) fn is_empty_infoset_element(&self, elem: &InfosetElement, term_id: NodeId) -> bool {
        let term = match self.schema.get_term(term_id) {
            Some(t) => t,
            None => return false,
        };
        let has_initiator = term
            .properties
            .initiator
            .as_deref()
            .is_some_and(|s| !s.is_empty());
        let has_terminator = term
            .properties
            .terminator
            .as_deref()
            .is_some_and(|s| !s.is_empty());

        if elem.nil_attribute == Some(true) || elem.state.is_nil() {
            let has_delims = match term.properties.nil_value_delimiter_policy {
                crate::schema::ir::NilValueDelimiterPolicy::Initiator => has_initiator,
                crate::schema::ir::NilValueDelimiterPolicy::Terminator => has_terminator,
                crate::schema::ir::NilValueDelimiterPolicy::Both => {
                    has_initiator || has_terminator
                }
                crate::schema::ir::NilValueDelimiterPolicy::None => false,
            };
            if has_delims {
                return false;
            }
            if let Some(ref nv) = term.properties.nil_value {
                return nv.is_empty() || nv == "%ES;";
            }
            return term.properties.nil_kind == crate::schema::ir::NilKind::LiteralValue;
        }

        // Per DFDL §14.2.3: "if padding or filling is output, the element does not have empty representation."
        let will_pad = term.properties.text_output_min_length > 0
            || (term.properties.length_kind == crate::schema::ir::LengthKind::Explicit
                && term.properties.length.is_some_and(|l| l > 0)
                && (term.properties.text_pad_kind == crate::schema::ir::TextPadKind::PadChar
                    || term.properties.fill_byte_defined));
        if will_pad {
            return false;
        }

        match &elem.state {
            ElementState::Empty => !has_initiator && !has_terminator,
            ElementState::NoValue => {
                if let TermKind::Element(ref el) = term.kind {
                    if matches!(el.type_ir, crate::schema::ir::CompiledType::Complex(_)) {
                        return false;
                    }
                }
                elem.children.is_empty() && !has_initiator && !has_terminator
            }
            ElementState::Value(v) => match v {
                DfdlValue::String(s) => {
                    s.is_empty()
                        && !has_initiator
                        && !has_terminator
                        && term.properties.text_output_min_length == 0
                }
                DfdlValue::HexBinary(b) => {
                    b.is_empty() && !has_initiator && !has_terminator
                }
                _ => false,
            },
            ElementState::Nil | ElementState::Absent => true,
        }
    }

    pub(crate) fn unparse_element(&mut self, term_id: NodeId, elem: &InfosetElement) -> DFDLResult<()> {
        self.unparse_element_with_index(term_id, elem, None)
    }


    pub(crate) fn unparse_element_with_index(
        &mut self,
        term_id: NodeId,
        elem: &InfosetElement,
        occurs_index: Option<usize>,
    ) -> DFDLResult<()> {
        let prev_ns = self.current_namespaces;
        let res = self.unparse_element_with_index_internal(term_id, elem, occurs_index);
        self.current_namespaces = prev_ns;
        res
    }

    pub(crate) fn unparse_element_with_index_internal(
        &mut self,
        term_id: NodeId,
        elem: &InfosetElement,
        occurs_index: Option<usize>,
    ) -> DFDLResult<()> {
        self.budget.consume(1)?;
        let term = self.schema.get_term(term_id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;

        if !term.properties.in_scope_namespaces.is_empty() {
            self.current_namespaces = &term.properties.in_scope_namespaces;
        }

        self.path_stack.push((term.name.local_name.as_str(), occurs_index));
        #[cfg(feature = "std")]
        if self.debug_trace {
            let _curr_pos = self.writer.position().0;
            std::eprintln!("[UNP_START] elem={}, is_hidden={}, bit_pos={}", term.name.local_name, elem.is_hidden, _curr_pos);
        }
        let saved_delims_len = self.active_delimiters.len();

        if term.properties.parse_unparse_policy == crate::schema::ir::ParseUnparsePolicy::ParseOnly {
            let msg = alloc::format!(
                "Schema Definition Error: Cannot unparse element '{}' without unparse support (parseUnparsePolicy is 'parseOnly')",
                term.name.local_name
            );
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }

        if let TermKind::Element(ref el) = term.kind {
            if matches!(el.type_ir, crate::schema::ir::CompiledType::Complex(_))
                && matches!(
                    term.properties.length_kind,
                    crate::schema::ir::LengthKind::Explicit | crate::schema::ir::LengthKind::Prefixed
                )
                && term.properties.length_units == crate::schema::ir::LengthUnits::Characters
                && crate::encoding::is_variable_width_encoding(&term.properties.encoding)
            {
                let kind_str = match term.properties.length_kind {
                    crate::schema::ir::LengthKind::Explicit => "lengthKind 'explicit'",
                    _ => "lengthKind 'prefixed'",
                };
                let msg = alloc::format!(
                    "Schema Definition Error: Variable width character encoding '{}' with {} and lengthUnits 'characters' is not supported for complex types on element '{}'",
                    term.properties.encoding,
                    kind_str,
                    term.name.local_name
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }

        let res = (|| {
            self.execute_set_variables(term)?;
            // inputValueCalc elements are computed on parse and absent from the data stream.
            if term.properties.input_value_calc.is_some() {
                if let (TermKind::Element(ref el), ElementState::Value(val)) =
                    (&term.kind, &elem.state)
                {
                    if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                        validate_unparse_value(
                            val,
                            st,
                            &term.properties,
                            self.schema.max_hex_binary_length_in_bytes,
                        )?;
                    }
                }
                return Ok(());
            }
            let mut local_props: Option<crate::schema::ir::ResolvedProperties> = None;
            if let Some(ast) = term.properties.encoding_prop.expr_ast() {
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(ast, &mut ctx)?;
                let dyn_encoding = match val {
                    DfdlValue::String(s) => s,
                    other => alloc::format!("{}", other),
                };
                let parsed_kind = crate::encoding::EncodingKind::parse_name(&dyn_encoding);
                if parsed_kind.is_sub_byte() {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: Only encodings with byte-sized code units can be computed via expressions. The encoding '{}' must be specified as a literal encoding name in the DFDL schema.",
                        dyn_encoding
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                let mut lp = term.properties.clone();
                lp.encoding = parsed_kind;
                lp.encoding_char_bits = lp.encoding.char_bits();
                lp.encoding_unit_bits = lp.encoding.unit_bits();
                if let Some(ref fb) = lp.fill_byte_raw {
                    if !fb.starts_with('%') && fb.chars().count() == 1 {
                        if let Some(ch) = fb.chars().next() {
                            if lp.encoding.is_sub_byte() {
                                let msg = alloc::format!(
                                    "Runtime Schema Definition Error: fillByte property requires a single-byte character for encoding '{}', but '{}' is not a byte-sized encoding",
                                    lp.encoding, lp.encoding
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                            if lp.encoding == crate::encoding::EncodingKind::Utf8 && ch.len_utf8() > 1 {
                                let msg = alloc::format!(
                                    "Runtime Schema Definition Error: fillByte property must be a single-byte character for encoding '{}', but '{}' takes {} bytes",
                                    lp.encoding, ch, ch.len_utf8()
                                );
                                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                            }
                        }
                    }
                }
                local_props = Some(lp);
            }
            if let Some(ast) = term.properties.byte_order_prop.expr_ast() {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(ast, &mut ctx) {
                    let bo = crate::expr::properties::parse_byte_order(&alloc::format!("{}", val))?;
                    let mut lp = match local_props {
                        Some(p) => p,
                        None => term.properties.clone(),
                    };
                    lp.byte_order = bo;
                    local_props = Some(lp);
                }
            }
            let props = match local_props {
                Some(ref lp) => lp,
                None => &term.properties,
            };

            if let Some(ref term_str) = props.terminator {
                if !term_str.is_empty() {
                    let term_tokens = self.extract_delimiter_tokens(term_str, props);
                    self.active_delimiters.extend(term_tokens);
                }
            }

            if let Some(ref scheme) = props.escape_scheme {
                if scheme.escape_kind == crate::schema::ir::EscapeKind::EscapeCharacter {
                    if let Some(ref ec) = scheme.escape_character {
                        if !ec.is_empty() {
                            for delim in &self.active_delimiters {
                                if delim.starts_with(ec.as_str()) {
                                    let msg = alloc::format!(
                                        "Schema Definition Error: dfdl:terminator and dfdl:separator may not begin with the dfdl:escapeCharacter '{}'",
                                        ec
                                    );
                                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                                }
                            }
                        }
                    }
                }
            }

            // Left framing per DFDL §12 grammar: LeftFraming = AlignmentFill LeadingSkip.
            let align_bits = match props.alignment_units {
                crate::schema::ir::AlignmentUnits::Bytes => {
                    props.alignment.saturating_mul(8)
                }
                crate::schema::ir::AlignmentUnits::Bits => props.alignment,
            };
            if props.alignment_kind == crate::schema::ir::AlignmentKind::Automatic
                && align_bits > 1
            {
                let current_pos = self.writer.position().0;
                let rem = current_pos.checked_rem(align_bits).unwrap_or(0);
                if rem > 0 {
                    let pad = align_bits.saturating_sub(rem);
                    let fill = fill_byte_value(props)?;
                    write_fill_padding(self.writer, fill, pad)?;
                }
            }

            if props.leading_skip > 0 {
                let skip_bits = match props.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        props.leading_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => props.leading_skip,
                };
                let fill = fill_byte_value(props)?;
                write_fill_padding(self.writer, fill, skip_bits)?;
            }

            // Enforce bitOrder change only on byte boundary (§11.2)
            // Left framing (leadingSkip / alignment) precedes bitOrder change per DFDL §11.2.
            if props.bit_order != self.writer.bit_order() {
                let current_pos = self.writer.position().0;
                let rem = current_pos % 8;
                if rem != 0 {
                    let bit_in_byte_1based = rem.saturating_add(1);
                    let msg = alloc::format!(
                        "Schema Definition Error: Can only change bitOrder on a byte boundary. Bit position {} is not on a byte boundary",
                        bit_in_byte_1based
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                self.writer.set_bit_order(props.bit_order);
            }

            if let Some(ref init) = props.initiator {
                self.write_evaluated_delimiter(init, props)?;
            }

            if let TermKind::Element(ref el) = term.kind {
                if elem.nil_attribute.is_some() && !el.is_nillable {
                    let msg = alloc::format!(
                        "Unparse Error: Element '{}' defines nil property but is not nillable",
                        el.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }
                if let crate::schema::ir::CompiledType::Simple(_) = el.type_ir {
                    if !elem.children.is_empty() {
                        let msg = alloc::format!(
                            "Unparse Error: Illegal content for simple element '{}', element cannot have child elements",
                            el.name.local_name
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                    }
                }
            }

            let opt_val = if let Some(ref ovc_prop) = props.output_value_calc_prop {
                let mut ctx = self.make_expr_context();
                let v = match ovc_prop {
                    crate::schema::ir::DfdlProp::Constant(c) => {
                        let trimmed = c.trim();
                        let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                            trimmed[1..trimmed.len().saturating_sub(1)].trim()
                        } else {
                            trimmed
                        };
                        let ast = crate::expr::parse_expr(inner)?;
                        crate::expr::eval_expr(&ast, &mut ctx)?
                    }
                    crate::schema::ir::DfdlProp::Expression { ast, .. } => {
                        crate::expr::eval_expr(ast, &mut ctx)?
                    }
                };
                Some(v)
            } else if let Some(ref ovc_expr) = props.output_value_calc {
                let ast = crate::expr::parse_expr(ovc_expr)?;
                let mut ctx = self.make_expr_context();
                Some(crate::expr::eval_expr(&ast, &mut ctx)?)
            } else {
                None
            };

            if let Some(val) = opt_val {
                let val = match term.kind {
                    TermKind::Element(ref el) => match el.type_ir {
                        crate::schema::ir::CompiledType::Simple(ref st) => {
                            crate::kernel::parser::element::coerce_and_validate_ivc_value(
                                &val, st, props,
                            )
                            .unwrap_or(val)
                        }
                        _ => val,
                    },
                    _ => val,
                };
                let mut path = self.base_path.clone();
                for &(seg, idx) in &self.path_stack {
                    let step = crate::types::PathStep {
                        axis: crate::types::PathAxis::Child,
                        target: crate::types::StepTarget::Unprefixed(alloc::string::ToString::to_string(seg)),
                        index_predicate: idx,
                        predicate_expr: None,
                    };
                    let _ = path.try_push_step(step);
                }
                self.ovc_values.insert(alloc::format!("{}", path), val.clone());
                self.unparse_simple_value(&val, props)?;
            } else {
                match &elem.state {
                    ElementState::Value(val) => {
                        let coerced;
                        let val_ref = if let TermKind::Element(ref el) = term.kind {
                            if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                                coerced = crate::kernel::parser::element::coerce_and_validate_ivc_value(
                                    val, st, props,
                                )
                                .unwrap_or_else(|_| val.clone());
                                validate_unparse_value(
                                    &coerced,
                                    st,
                                    props,
                                    self.schema.max_hex_binary_length_in_bytes,
                                )
                                .map_err(|e| {
                                    let msg_str = e.message.as_str();
                                    let detail = msg_str.strip_prefix("Unparse Error: ").unwrap_or(msg_str);
                                    DFDLError::new(
                                        DFDLErrorKind::Unparse,
                                        &alloc::format!("Unparse Error: element '{}' {}", elem.name.local_name, detail),
                                    )
                                })?;
                                &coerced
                            } else {
                                val
                            }
                        } else {
                            val
                        };
                        // If element defines dfdlx:repType and enumeration representation values,
                        // map the logical enum string to its canonical representation value on the wire.
                        if (props.rep_type.is_some() || props.rep_simple_type.is_some())
                            && !props.facets.rep_values.is_empty()
                        {
                            if self.validation_mode != crate::kernel::ValidationMode::Off
                                && props.facets.has_facets()
                            {
                                if let Err(e) = props.facets.validate_value_detailed(val_ref) {
                                    let msg_str = e.message.as_str();
                                    let detail = msg_str.strip_prefix("Validation Error: ").unwrap_or(msg_str);
                                    let msg = alloc::format!("Validation Error: element '{}' {}", elem.name.local_name, detail);
                                    let _ = crate::util::try_push(&mut self.validation_errors, DFDLError::new(DFDLErrorKind::Validation, &msg));
                                }
                            }
                            let val_str = alloc::format!("{}", val_ref);
                            if let Some((_, rep_str)) = props.facets.rep_values.iter().find(|(k, _)| k == &val_str) {
                                let rep_st = props.rep_simple_type.unwrap_or(crate::infoset::DfdlSimpleType::UnsignedInt);
                                let rep_val = crate::kernel::parser::element::coerce_and_validate_ivc_value(
                                    &DfdlValue::String(rep_str.clone()),
                                    &rep_st,
                                    props,
                                )
                                .unwrap_or_else(|_| DfdlValue::String(rep_str.clone()));
                                self.unparse_simple_value(&rep_val, props)?;
                            } else {
                                let msg = alloc::format!("Unparse Error: Value '{}' not found in enumeration", val_str);
                                return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                            }
                        } else {
                            if self.validation_mode != crate::kernel::ValidationMode::Off
                                && props.facets.has_facets()
                                && props.rep_type.is_none()
                                && props.rep_simple_type.is_none()
                            {
                                if let Err(e) = props.facets.validate_value_detailed(val_ref) {
                                    let msg_str = e.message.as_str();
                                    let detail = msg_str.strip_prefix("Validation Error: ").unwrap_or(msg_str);
                                    let msg = alloc::format!("Validation Error: element '{}' {}", elem.name.local_name, detail);
                                    let _ = crate::util::try_push(&mut self.validation_errors, DFDLError::new(DFDLErrorKind::Validation, &msg));
                                }
                            }
                            self.unparse_simple_value(val_ref, props)?;
                        }
                    }
                    ElementState::NoValue | ElementState::Absent => {
                        if let TermKind::Element(ref el) = term.kind {
                            if let crate::schema::ir::CompiledType::Complex(child_id) = el.type_ir {
                                if term.properties.length_kind
                                    == crate::schema::ir::LengthKind::Prefixed
                                {
                                    let sink = VecByteSink::new();
                                    let mut sub_writer = crate::io::bitstream::BitWriter::new(
                                        sink,
                                        crate::io::traits::BitOrder::MostSignificantBitFirst,
                                        term.properties.byte_order,
                                    );
                                    let mut sub_unparser = UnparserEngine::new(
                                        self.schema,
                                        &mut sub_writer,
                                        self.budget,
                                    );
                                    sub_unparser.doc = self.doc;
                                    sub_unparser.base_path = self.base_path.clone();
                                    sub_unparser.current_path = self.current_path.clone();
                                    sub_unparser.path_stack = self.path_stack.clone();
                                    sub_unparser.active_delimiters = self.active_delimiters.clone();
                                    sub_unparser.current_occurs_index = self.current_occurs_index;
                                    sub_unparser.active_variable_map = self.active_variable_map.clone();
                                    sub_unparser.validation_mode = self.validation_mode;
                                    sub_unparser.debug_trace = self.debug_trace;
                                    sub_unparser.unparse_term(child_id, elem)?;
                                    self.validation_errors.append(&mut sub_unparser.validation_errors);
                                    if let Some(unconsumed) = elem.children.get(sub_unparser.child_cursor) {
                                        let unconsumed_name = match unconsumed {
                                            InfosetNode::Element(e) => e.name.clark_notation(),
                                        };
                                        let elem_name = elem.name.clark_notation();
                                        let msg = alloc::format!(
                                            "Unparse Error: expected element end for '{}', but received element start '{}'",
                                            elem_name, unconsumed_name
                                        );
                                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                                    }
                                    if sub_unparser.active_variable_map.is_some() {
                                        self.active_variable_map = sub_unparser.active_variable_map.take();
                                    }
                                    drop(sub_unparser);
                                    sub_writer.flush()?;
                                    let buf = sub_writer.into_sink().into_vec();

                                    let val_len = match term.properties.length_units {
                                        crate::schema::ir::LengthUnits::Bits => {
                                            buf.len().saturating_mul(8)
                                        }
                                        _ => buf.len(),
                                    };
                                    self.write_prefix_length(val_len, &term.properties)?;
                                    for &b in &buf {
                                        self.writer.write_bits(b as u64, 8)?;
                                    }
                                } else {
                                    // Per GFD.240 §12.3.7.3:
                                    // When unparsing a complex element with dfdl:lengthKind 'explicit':
                                    // - The content of the complex element is unparsed.
                                    // - If content length > explicit length, it is an Unparse Error.
                                    // - If content length < explicit length, the remaining bits are
                                    //   filled with fillByte before any right framing (terminator/trailingSkip).
                                    let explicit_bits = if term.properties.length_kind
                                        == crate::schema::ir::LengthKind::Explicit
                                    {
                                        if let Some(l) = self.resolve_explicit_length(&term.properties)? {
                                            let bits = match term.properties.length_units {
                                                crate::schema::ir::LengthUnits::Bits => l,
                                                crate::schema::ir::LengthUnits::Bytes => l.saturating_mul(8),
                                                crate::schema::ir::LengthUnits::Characters => {
                                                    l.saturating_mul(crate::encoding::encoding_unit_bits(&term.properties.encoding))
                                                }
                                            };
                                            Some(bits)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    let content_start_bit = self.writer.position().0;
                                    let prev_cursor = self.child_cursor;
                                    self.child_cursor = 0;
                                    let term_res = self.unparse_term(child_id, elem);
                                    term_res?;
                                    if let Some(unconsumed) = elem.children.get(self.child_cursor) {
                                        let unconsumed_name = match unconsumed {
                                            InfosetNode::Element(e) => e.name.clark_notation(),
                                        };
                                        let elem_name = elem.name.clark_notation();
                                        let msg = alloc::format!(
                                            "Unparse Error: expected element end for '{}', but received element start '{}'",
                                            elem_name, unconsumed_name
                                        );
                                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                                    }
                                    self.child_cursor = prev_cursor;

                                    if let Some(target_bits) = explicit_bits {
                                        let current_bit = self.writer.position().0;
                                        let consumed_bits = current_bit.saturating_sub(content_start_bit);
                                        if consumed_bits > target_bits {
                                            let msg = alloc::format!(
                                                "Unparse Error: Complex element '{}' content length ({} bits) exceeds explicit length ({} bits)",
                                                elem.name.local_name,
                                                consumed_bits,
                                                target_bits
                                            );
                                            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                                        }
                                        if consumed_bits < target_bits {
                                            let pad_bits = target_bits.saturating_sub(consumed_bits);
                                            let fill = fill_byte_value(&term.properties)?;
                                            write_fill_padding(self.writer, fill, pad_bits)?;
                                        }
                                    }
                                }
                            } else {
                                let default_val = match el.type_ir {
                                    crate::schema::ir::CompiledType::Simple(
                                        crate::infoset::DfdlSimpleType::String,
                                    ) => Some(DfdlValue::String(alloc::string::String::new())),
                                    crate::schema::ir::CompiledType::Simple(
                                        crate::infoset::DfdlSimpleType::HexBinary,
                                    ) => Some(DfdlValue::HexBinary(alloc::vec![])),
                                    _ => el.default_value.clone(),
                                };
                                if let Some(ref val) = default_val {
                                    if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                                        validate_unparse_value(
                                            val,
                                            st,
                                            props,
                                            self.schema.max_hex_binary_length_in_bytes,
                                        )?;
                                    }
                                    self.unparse_simple_value(val, props)?;
                                }
                            }
                        }
                    }
                    ElementState::Empty => {
                        if let TermKind::Element(ref el) = term.kind {
                            let empty_val = match el.type_ir {
                                crate::schema::ir::CompiledType::Simple(
                                    crate::infoset::DfdlSimpleType::String,
                                ) => Some(DfdlValue::String(alloc::string::String::new())),
                                crate::schema::ir::CompiledType::Simple(
                                    crate::infoset::DfdlSimpleType::HexBinary,
                                ) => Some(DfdlValue::HexBinary(alloc::vec![])),
                                _ => el.default_value.clone(),
                            };
                            if let Some(ref val) = empty_val {
                                if let crate::schema::ir::CompiledType::Simple(ref st) = el.type_ir {
                                    validate_unparse_value(
                                        val,
                                        st,
                                        props,
                                        self.schema.max_hex_binary_length_in_bytes,
                                    )?;
                                }
                                self.unparse_simple_value(val, props)?;
                            }
                        }
                    }
                    ElementState::Nil => {
                        if let TermKind::Element(ref el) = term.kind {
                            if !el.is_nillable {
                                let msg = alloc::format!(
                                    "Unparse Error: Element '{}' defines nil property but is not nillable",
                                    el.name.local_name
                                );
                                return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                            }
                        }
                        if let Some(ref nil_val) = term.properties.nil_value {
                            // DFDL 1.0 §13.15: When dfdl:nilKind is 'literalCharacter',
                            // the single nil character is repeated to fill the specified length
                            // of the element. When dfdl:nilKind is 'literalValue', the nil string
                            // is written as-is.
                            if term.properties.nil_kind == crate::schema::ir::NilKind::LiteralCharacter {
                                let target_len = self.resolve_explicit_length(props)?.unwrap_or(1);
                                for _ in 0..target_len {
                                    self.write_evaluated_delimiter(nil_val, props)?;
                                }
                            } else {
                                self.write_evaluated_delimiter(nil_val, props)?;
                            }
                        } else {
                            return Err(DFDLError::new_static(
                                DFDLErrorKind::Unparse,
                                "Element in Nil state but nilValue property is not defined in schema",
                            ));
                        }
                    }
                }
            }

            if let Some(ref term_str) = term.properties.terminator {
                self.write_evaluated_delimiter(term_str, props)?;
            }

            // Right framing: trailingSkip
            if term.properties.trailing_skip > 0 {
                let skip_bits = match term.properties.alignment_units {
                    crate::schema::ir::AlignmentUnits::Bytes => {
                        term.properties.trailing_skip.saturating_mul(8)
                    }
                    crate::schema::ir::AlignmentUnits::Bits => term.properties.trailing_skip,
                };
                let fill = fill_byte_value(&term.properties)?;
                write_fill_padding(self.writer, fill, skip_bits)?;
            }

            Ok(())
        })();

        #[cfg(feature = "std")]
        if self.debug_trace {
            std::eprintln!("[UNP_END] elem={}, bit_pos={}", term.name.local_name, self.writer.position().0);
        }
        self.active_delimiters.truncate(saved_delims_len);
        self.path_stack.pop();
        res
    }

}
