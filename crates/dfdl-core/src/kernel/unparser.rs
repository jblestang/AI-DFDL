//! DFDL Runtime Unparser Engine.
//!
//! Serializes an in-memory DFDL Infoset (`InfosetDocument`) into an outgoing bitstream (`BitWriter`)
//! according to a compiled schema graph (`CompiledSchema`).
//!
//! Aligned with DFDL 1.0 Specification §11, §12. Panic-free `#![no_std]` + `alloc`.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::schema::CompiledTerm;

/// Returns the fill byte used for skip/alignment padding, or a Schema Definition Error when
/// `dfdl:fillByte` is needed (padding is actually emitted) but was never defined.
fn fill_byte_value(props: &crate::schema::ir::ResolvedProperties) -> DFDLResult<u64> {
    if props.fill_byte_defined {
        Ok(u64::from(props.fill_byte))
    } else {
        Err(DFDLError::new_static(
            DFDLErrorKind::SchemaDefinition,
            "Schema Definition Error: Property fillByte is not defined.",
        ))
    }
}

/// Writes `pad_bits` bits of padding using the in-scope `fillByte`.
///
/// Per DFDL 1.0 §13.3.1:
/// - When padding to an alignment boundary, full bytes of fillByte are written.
/// - If a fraction of a byte is required (sub-byte alignment), the most significant
///   bits of the fillByte are used when bitOrder is mostSignificantBitFirst, and the
///   least significant bits when bitOrder is leastSignificantBitFirst.
fn write_fill_padding<W: ByteSink>(
    writer: &mut BitWriter<W>,
    fill: u64,
    pad_bits: usize,
) -> DFDLResult<()> {
    if pad_bits == 0 {
        return Ok(());
    }
    let full_bytes = pad_bits / 8;
    for _ in 0..full_bytes {
        writer.write_bits(fill & 0xFF, 8)?;
    }
    let rem_bits = pad_bits % 8;
    if rem_bits > 0 {
        let fill_byte = (fill & 0xFF) as u8;
        let bits_to_write = match writer.bit_order() {
            BitOrder::MostSignificantBitFirst => {
                u64::from(fill_byte >> (8 - rem_bits))
            }
            BitOrder::LeastSignificantBitFirst => {
                u64::from(fill_byte & ((1 << rem_bits) - 1))
            }
        };
        writer.write_bits(bits_to_write, rem_bits)?;
    }
    Ok(())
}

use crate::infoset::state::ElementState;
use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use crate::infoset::value::DfdlValue;
use crate::io::bitstream::BitWriter;
use crate::io::traits::{BitOrder, ByteOrder, ByteSink};
use crate::io::VecByteSink;
use crate::limits::WorkBudget;
use crate::schema::ir::{CompiledSchema, NodeId, Representation, ResolvedProperties, TermKind};
use crate::util::try_push;

/// DFDL Runtime Streaming Unparser Engine.
pub struct UnparserEngine<'a, S: ByteSink> {
    schema: &'a CompiledSchema,
    writer: &'a mut BitWriter<S>,
    budget: &'a mut WorkBudget,
    variable_map: crate::expr::variables::VariableMap,
    doc: Option<&'a InfosetDocument>,
    current_path: crate::types::InfosetPath,
    current_occurs_index: usize,
    child_cursor: usize,
    active_delimiters: Vec<String>,
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
            variable_map: schema.variable_map.clone(),
            schema,
            writer,
            budget,
            doc: None,
            current_path: crate::types::InfosetPath::root(),
            current_occurs_index: 1,
            child_cursor: 0,
            active_delimiters: Vec::new(),
        }
    }

    /// Sets an external variable value on the unparser's active variable map.
    pub fn set_external_variable(&mut self, name: &str, value: &str) -> DFDLResult<()> {
        self.variable_map.set_variable_validated(
            &crate::types::QName::local(name),
            crate::infoset::value::DfdlValue::String(alloc::string::ToString::to_string(value)),
            false,
        )
    }

    /// Sets whether to escalate warnings to errors (daf:escalateWarningsToErrors).
    #[inline]
    pub fn set_escalate_warnings(&mut self, escalate: bool) {
        self.variable_map.escalate_warnings = escalate;
    }

    /// Unparses an [`InfosetDocument`] into the output bitstream according to the compiled schema.
    pub fn unparse_document(&mut self, doc: &'a InfosetDocument) -> DFDLResult<()> {
        self.doc = Some(doc);
        self.current_path = crate::types::InfosetPath::root();
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

        self.unparse_element(root_term_id, root_elem)?;
        self.writer.flush()?;
        Ok(())
    }

    fn unparse_term(&mut self, term_id: NodeId, parent_elem: &InfosetElement) -> DFDLResult<()> {
        self.budget.consume(1)?;

        let term = self.schema.get_term(term_id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;

        // Enforce bitOrder change only on byte boundary (§11.2)
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

        let is_element = matches!(term.kind, TermKind::Element(_));

        let has_nvi = !term.properties.new_variable_instances.is_empty();
        if has_nvi {
            self.execute_new_variable_instances(term)?;
        }

        if !is_element {
            self.execute_set_variables(term)?;
            // Left framing: leadingSkip
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

            // Align bitstream if required
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
                let rem = current_pos % align_bits;
                if rem > 0 {
                    let pad = align_bits - rem;
                    let fill = fill_byte_value(&term.properties)?;
                    write_fill_padding(self.writer, fill, pad)?;
                }
            }

            if let Some(ref init) = term.properties.initiator {
                self.write_evaluated_delimiter(init, &term.properties)?;
            }
        }

        let res = match &term.kind {
            TermKind::Element(ref el) => {
                let children = self.find_child_elements(parent_elem, term_id);
                if children.is_empty() {
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
                        if children.len() < el.min_occurs && el.default_value.is_none() {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Expected at least {} occurrence(s) of element '{}' for unparsing, but found {}",
                                    el.min_occurs,
                                    el.name.local_name,
                                    children.len()
                                ),
                            ));
                        }
                        if term.properties.occurs_count_kind != crate::schema::ir::OccursCountKind::Expression {
                            if let Some(max) = el.max_occurs {
                                if children.len() > max {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::Unparse,
                                        &alloc::format!(
                                            "Unparse Error: Expected array end event for '{}', received element start event (found {} occurrences exceeding maxOccurs {})",
                                            el.name.local_name,
                                            children.len(),
                                            max
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                    let is_array = children.len() > 1
                        || match term.kind {
                            TermKind::Element(ref el) => {
                                el.max_occurs.is_none()
                                    || el.max_occurs.is_some_and(|m| m > 1)
                                    || term.properties.occurs_count_kind
                                        != crate::schema::ir::OccursCountKind::Implicit
                            }
                            _ => false,
                        };
                    for (idx, child_elem) in children.into_iter().enumerate() {
                        let prev_idx = self.current_occurs_index;
                        if is_array {
                            self.current_occurs_index = idx.saturating_add(1);
                        }
                        let seg = if is_array {
                            alloc::format!("{}[{}]", child_elem.name.local_name, idx.saturating_add(1))
                        } else {
                            child_elem.name.local_name.clone()
                        };
                        let elem_res = self.unparse_element_with_seg(term_id, child_elem, Some(&seg));
                        self.current_occurs_index = prev_idx;
                        elem_res?;
                    }
                }
                Ok(())
            }
            TermKind::Sequence(seq) => {
                if let Some(ref layer_name) = term.properties.layer {
                    let clean_layer = layer_name.split(':').next_back().unwrap_or(layer_name);
                    if clean_layer == "stlBombOutLayer" {
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
                            .unwrap_or_else(|| alloc::string::String::from("PE"));
                        if let Some(ref bw) = bomb_where {
                            if bw == "closeOutput" || bw == "write" || bw == "wrapOutput" || bw == "getter" || bw == "setter" {
                                if bomb_how.eq_ignore_ascii_case("RSDE") {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::SchemaDefinition,
                                        &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", bw),
                                    ));
                                } else {
                                    return Err(DFDLError::new(
                                        DFDLErrorKind::Unparse,
                                        &alloc::format!("Unparse Error: Bombed out at {}", bw),
                                    ));
                                }
                            }
                        }
                    }
                    if clean_layer == "IPv4Checksum"
                        || clean_layer == "checkDigit"
                        || clean_layer.eq_ignore_ascii_case("twobyteswap")
                        || clean_layer.eq_ignore_ascii_case("twoByteSwap")
                    {
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
                        sub_unparser.current_path = self.current_path.clone();
                        sub_unparser.active_delimiters = self.active_delimiters.clone();
                        sub_unparser.current_occurs_index = self.current_occurs_index;
                        sub_unparser.variable_map = self.variable_map.clone();
                        sub_unparser.child_cursor = self.child_cursor;

                        for &member_id in &seq.members {
                            sub_unparser.unparse_term(member_id, parent_elem)?;
                        }

                        self.child_cursor = sub_unparser.child_cursor;
                        self.variable_map = sub_unparser.variable_map;

                        sub_writer.flush()?;
                        let mut buf = sub_writer.into_sink().into_vec();

                        if clean_layer.eq_ignore_ascii_case("twobyteswap")
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
                            if req_words && !buf.len().is_multiple_of(2) {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Unparse,
                                    "Unparse Error: not a multiple of 2 for twoByteSwap layer",
                                ));
                            }
                        } else if clean_layer == "IPv4Checksum" {
                            if buf.len() >= 20 {
                                let chk = crate::kernel::layer::compute_ipv4_checksum(buf.get(..20).unwrap_or(&[]));
                                self.variable_map.set_variable_validated(
                                    &crate::types::QName::with_namespace(
                                        "urn:org.apache.daffodil.layers.IPv4Checksum",
                                        "IPv4Checksum",
                                        None,
                                    ),
                                    crate::infoset::value::DfdlValue::UnsignedShort(chk),
                                    false,
                                )?;
                                let be_bytes = chk.to_be_bytes();
                                if let Some(b10) = buf.get_mut(10) {
                                    *b10 = *be_bytes.first().unwrap_or(&0);
                                }
                                if let Some(b11) = buf.get_mut(11) {
                                    *b11 = *be_bytes.get(1).unwrap_or(&0);
                                }
                            }
                        } else if clean_layer == "checkDigit" {
                            let cd = crate::kernel::layer::compute_check_digit(&buf);
                            self.variable_map.set_variable_validated(
                                &crate::types::QName::with_namespace(
                                    "urn:org.apache.daffodil.layers.checkDigit",
                                    "checkDigit",
                                    None,
                                ),
                                crate::infoset::value::DfdlValue::UnsignedShort(cd),
                                false,
                            )?;
                        }

                        for &b in &buf {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                        return Ok(());
                    }
                }

                let saved_delims_len = self.active_delimiters.len();
                if let Some(ref sep) = term.properties.separator {
                    let sep_tokens = self.extract_delimiter_tokens(sep, &term.properties);
                    self.active_delimiters.extend(sep_tokens);
                }
                if let Some(ref term_str) = term.properties.terminator {
                    let term_tokens = self.extract_delimiter_tokens(term_str, &term.properties);
                    self.active_delimiters.extend(term_tokens);
                }
                let sep_opt = term.properties.separator.as_deref();
                let sep_pos = term.properties.separator_position;
                let mut total_element_count: usize = 0;

                for (m_idx, &member_id) in seq.members.iter().enumerate() {
                    let has_subsequent_output = seq
                        .members
                        .get(m_idx.saturating_add(1)..)
                        .is_some_and(|subsequent| {
                            subsequent
                                .iter()
                                .any(|&sub_m| self.sequence_member_has_output(parent_elem, sub_m))
                        });
                    let children = self.find_child_elements(parent_elem, member_id);
                    if children.is_empty() {
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
                            if suppress {
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
                                            if children.len() > max {
                                                return Err(DFDLError::new(
                                                    DFDLErrorKind::Unparse,
                                                    &alloc::format!(
                                                        "Unparse Error: Expected array end event for '{}', received element start event (found {} occurrences exceeding maxOccurs {})",
                                                        el.name.local_name,
                                                        children.len(),
                                                        max
                                                    ),
                                                ));
                                            }
                                        }
                                    }
                                    if children.len() < el.min_occurs && el.default_value.is_none() {
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
                        let children_count = children.len();
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

                        for (idx, child_elem) in children.iter().copied().enumerate() {
                            let is_required = if is_array {
                                idx < min_occurs
                            } else {
                                min_occurs >= 1
                            };

                            let is_empty = self.is_empty_infoset_element(child_elem, member_id);
                            let is_trailing = is_empty
                                && !has_subsequent_output
                                && children
                                    .get(idx.saturating_add(1)..)
                                    .is_none_or(|rem| {
                                        rem.iter().all(|c| self.is_empty_infoset_element(c, member_id))
                                    });
                            let suppress_sep = if is_required {
                                false
                            } else if is_empty {
                                match term.properties.separator_suppression_policy {
                                    crate::schema::ir::SeparatorSuppressionPolicy::Never => false,
                                    crate::schema::ir::SeparatorSuppressionPolicy::AnyEmpty => true,
                                    crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmpty
                                    | crate::schema::ir::SeparatorSuppressionPolicy::TrailingEmptyStrict => is_trailing,
                                }
                            } else {
                                false
                            };

                            let prev_idx = self.current_occurs_index;
                            if is_array {
                                self.current_occurs_index = idx.saturating_add(1);
                            }
                            let seg = if is_array {
                                alloc::format!("{}[{}]", child_elem.name.local_name, idx.saturating_add(1))
                            } else {
                                child_elem.name.local_name.clone()
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
                                    self.write_evaluated_delimiter(sep, &term.properties)?;
                                }
                            }
                            let start_pos = self.writer.position().0;
                            let elem_res = self.unparse_element_with_seg(member_id, child_elem, Some(&seg));
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
                        let target_bits = match term.properties.length_units {
                            crate::schema::ir::LengthUnits::Bits => target_len,
                            _ => target_len.saturating_mul(8),
                        };
                        if bits_written < target_bits {
                            let pad_bits = target_bits - bits_written;
                            let fill_byte = term.properties.fill_byte;
                            let full_bytes = pad_bits / 8;
                            for _ in 0..full_bytes {
                                self.writer.write_bits(fill_byte as u64, 8)?;
                            }
                            let rem_bits = pad_bits % 8;
                            if rem_bits > 0 {
                                self.writer.write_bits((fill_byte >> (8 - rem_bits)) as u64, rem_bits)?;
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
    fn resolve_delimiter_string(
        &mut self,
        raw_delim: &str,
        props: &ResolvedProperties,
    ) -> DFDLResult<String> {
        let trimmed = raw_delim.trim();
        let eval_str = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
            let ast = crate::expr::parse_expr(trimmed)?;
            let mut ctx = self.make_expr_context();
            let val = crate::expr::eval_expr(&ast, &mut ctx)?;
            alloc::format!("{}", val)
        } else if let Some(stripped) = trimmed.strip_prefix("{{") {
            alloc::format!("{{{stripped}")
        } else {
            String::from(raw_delim)
        };

        // If delimiter contains alternatives, choose the first alternative for unparsing (DFDL §12.3)
        let alts = crate::kernel::parser::delimiters::split_delimiter_alternatives(&eval_str);
        let first_alt = alts.first().map(|s| s.as_str()).unwrap_or(&eval_str);

        // Resolve target newline convention from outputNewLine
        let nl_replacement = self.resolve_output_new_line(props);

        Ok(decode_unparse_delimiter(first_alt, &nl_replacement))
    }

    /// Resolves the target newline string from `dfdl:outputNewLine`, evaluating dynamic expressions if present.
    fn resolve_output_new_line(&mut self, props: &ResolvedProperties) -> String {
        let raw = match props.output_new_line.as_deref() {
            Some(r) => r,
            None => return String::from("\n"),
        };
        let trimmed = raw.trim();
        let eval_str = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
            if let Ok(ast) = crate::expr::parse_expr(trimmed) {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                    alloc::format!("{}", val)
                } else {
                    String::from(raw)
                }
            } else {
                String::from(raw)
            }
        } else if let Some(stripped) = trimmed.strip_prefix("{{") {
            alloc::format!("{{{stripped}")
        } else {
            String::from(raw)
        };

        match eval_str.as_str() {
            "%CR;%LF;" | "\r\n" => String::from("\r\n"),
            "%LF;" | "\n" => String::from("\n"),
            "%CR;" | "\r" => String::from("\r"),
            "%NEL;" => String::from("\u{0085}"),
            "%LS;" => String::from("\u{2028}"),
            custom => crate::expr::properties::decode_dfdl_character_entities(custom),
        }
    }

    /// Evaluates a delimiter and writes its encoded bytes according to the target character set.
    fn write_evaluated_delimiter(
        &mut self,
        raw_delim: &str,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let decoded = self.resolve_delimiter_string(raw_delim, props)?;
        if decoded.is_empty() {
            return Ok(());
        }

        let encoded_bytes = crate::encoding::encode_text_string(&decoded, &props.encoding);
        for &byte in &encoded_bytes {
            self.writer.write_bits(byte as u64, 8)?;
        }
        Ok(())
    }

    fn extract_delimiter_tokens(
        &mut self,
        raw_delim: &str,
        props: &ResolvedProperties,
    ) -> Vec<String> {
        let trimmed = raw_delim.trim();
        let eval_str = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
            if let Ok(ast) = crate::expr::parse_expr(trimmed) {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                    alloc::format!("{}", val)
                } else {
                    String::from(raw_delim)
                }
            } else {
                String::from(raw_delim)
            }
        } else if let Some(stripped) = trimmed.strip_prefix("{{") {
            alloc::format!("{{{stripped}")
        } else {
            String::from(raw_delim)
        };

        let nl_replacement = self.resolve_output_new_line(props);

        let alts = crate::kernel::parser::delimiters::split_delimiter_alternatives(&eval_str);
        let mut tokens = Vec::new();
        for alt in alts {
            let decoded = decode_unparse_delimiter(&alt, &nl_replacement);
            if !decoded.is_empty() {
                tokens.push(decoded);
            }
        }
        tokens
    }

    fn make_expr_context(&mut self) -> crate::expr::ExprContext<'_> {
        crate::expr::ExprContext::with_variable_map(
            self.doc,
            &self.current_path,
            &[],
            Some(&self.variable_map),
            self.budget,
        )
        .with_schema(self.schema)
        .with_occurs_index(self.current_occurs_index)
        .for_unparsing()
    }

    fn execute_set_variables(&mut self, term: &CompiledTerm) -> DFDLResult<()> {
        if !term.properties.set_variables.is_empty() {
            for (var_name, val_expr) in &term.properties.set_variables {
                let trimmed = val_expr.trim();
                let evaluated = if trimmed.starts_with('{') && trimmed.ends_with('}') && !trimmed.starts_with("{{") {
                    let expr_body = trimmed
                        .get(1..trimmed.len().saturating_sub(1))
                        .unwrap_or("")
                        .trim();
                    let ast = crate::expr::parse_expr(expr_body)?;
                    let mut ctx = self.make_expr_context();
                    crate::expr::eval_expr(&ast, &mut ctx)?
                } else {
                    DfdlValue::String(val_expr.clone())
                };
                self.variable_map
                    .set_variable_validated(var_name, evaluated, false)?;
            }
        }
        Ok(())
    }

    /// Executes `dfdl:newVariableInstance` statements attached to `term` (§7.7).
    fn execute_new_variable_instances(&mut self, term: &CompiledTerm) -> DFDLResult<()> {
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
                if var.direction == crate::expr::variables::VariableDirection::ParseOnly {
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
                    let mut ctx = self.make_expr_context();
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
    fn pop_new_variable_instances(&mut self, term: &CompiledTerm) {
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
                if var.direction == crate::expr::variables::VariableDirection::ParseOnly {
                    continue;
                }
            }
            self.variable_map.pop_variable_instance(var_name);
        }
    }

    fn eval_runtime_prop_str(&mut self, raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
            if let Ok(ast) = crate::expr::parse_expr(trimmed) {
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                    return alloc::format!("{}", val);
                }
            }
        } else if let Some(stripped) = trimmed.strip_prefix("{{") {
            return alloc::format!("{{{stripped}");
        }
        String::from(raw)
    }

    fn apply_escape_scheme(
        &mut self,
        text: &str,
        scheme: &crate::schema::ir::CompiledEscapeScheme,
    ) -> String {
        let eval_ec = scheme
            .escape_character
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_eec = scheme
            .escape_escape_character
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_bs = scheme
            .escape_block_start
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));
        let eval_be = scheme
            .escape_block_end
            .as_deref()
            .map(|s| self.eval_runtime_prop_str(s));

        let delims: Vec<&str> = self
            .active_delimiters
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect();

        scheme.escape_text(
            text,
            eval_ec.as_deref(),
            eval_eec.as_deref(),
            eval_bs.as_deref(),
            eval_be.as_deref(),
            &delims,
        )
    }

    /// Checks if a branch term or any of its nested members matches the element at cursor in parent.
    fn branch_matches_current_element(
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

    fn branch_matches_element(
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
    fn branch_can_be_empty(&self, branch_term_id: NodeId) -> bool {
        let term = match self.schema.get_term(branch_term_id) {
            Some(t) => t,
            None => return false,
        };
        match &term.kind {
            TermKind::Element(el) => {
                if el.min_occurs == 0 {
                    return true;
                }
                if el.default_value.is_some()
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

    fn find_child_elements<'e>(
        &mut self,
        parent: &'e InfosetElement,
        term_id: NodeId,
    ) -> Vec<&'e InfosetElement> {
        let mut result = Vec::new();
        let term = match self.schema.get_term(term_id) {
            Some(t) => t,
            None => return result,
        };
        // Hidden elements are not in the infoset per DFDL §14.4
        if term.properties.is_hidden {
            return result;
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
        let mut c = self.child_cursor;
        while let Some(child_node) = parent.children.get(c) {
            match child_node {
                InfosetNode::Element(ref elem) => {
                    if matches_term(&elem.name) {
                        let _ = try_push(&mut result, elem);
                        c = c.saturating_add(1);
                    } else {
                        break;
                    }
                }
            }
        }
        self.child_cursor = c;
        result
    }

    /// Determines whether a sequence member produces any unparsed output from the infoset,
    /// used to evaluate trailing empty separator suppression per DFDL §14.2.
    fn sequence_member_has_output(&self, parent: &InfosetElement, term_id: NodeId) -> bool {
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
    fn is_empty_infoset_element(&self, elem: &InfosetElement, term_id: NodeId) -> bool {
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
        match &elem.state {
            ElementState::Empty => !has_initiator && !has_terminator,
            ElementState::NoValue => {
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

    fn unparse_element(&mut self, term_id: NodeId, elem: &InfosetElement) -> DFDLResult<()> {
        self.unparse_element_with_seg(term_id, elem, None)
    }

    fn unparse_element_with_seg(
        &mut self,
        term_id: NodeId,
        elem: &InfosetElement,
        path_seg: Option<&str>,
    ) -> DFDLResult<()> {
        let term = self.schema.get_term(term_id).ok_or_else(|| {
            DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "Term NodeId missing from compiled schema graph",
            )
        })?;

        let seg = path_seg.unwrap_or(&elem.name.local_name);
        let _ = self.current_path.try_push(seg);
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
                    "Runtime Schema Definition Error: Variable width character encoding '{}' with {} and lengthUnits 'characters' is not supported for complex types on element '{}'",
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
            let mut local_props;
            let mut has_local = false;
            if term.properties.encoding.starts_with('{')
                && term.properties.encoding.ends_with('}')
            {
                let enc_expr = &term.properties.encoding[1..term.properties.encoding.len().saturating_sub(1)].trim();
                let ast = crate::expr::parse_expr(enc_expr)?;
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                let dyn_encoding = match val {
                    DfdlValue::String(s) => s,
                    other => alloc::format!("{}", other),
                };
                let dyn_upper = dyn_encoding.to_ascii_uppercase();
                if dyn_upper.contains("6-BIT")
                    || dyn_upper.contains("5-BIT")
                    || dyn_upper.contains("7-BIT")
                    || dyn_upper.contains("BIT-PACKED")
                {
                    let msg = alloc::format!(
                        "Runtime Schema Definition Error: Only encodings with byte-sized code units can be computed via expressions. The encoding '{}' must be specified as a literal encoding name in the DFDL schema.",
                        dyn_encoding
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                local_props = term.properties.clone();
                local_props.encoding = dyn_encoding;
                if let Some(ref fb) = local_props.fill_byte_raw {
                    if !fb.starts_with('%') && fb.chars().count() == 1 {
                        if let Some(ch) = fb.chars().next() {
                        let enc_upper = local_props.encoding.to_ascii_uppercase();
                        if enc_upper.contains("6-BIT")
                            || enc_upper.contains("5-BIT")
                            || enc_upper.contains("7-BIT")
                            || enc_upper.contains("BIT-PACKED")
                        {
                            let msg = alloc::format!(
                                "Runtime Schema Definition Error: fillByte property requires a single-byte character for encoding '{}', but '{}' is not a byte-sized encoding",
                                local_props.encoding, local_props.encoding
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                        if local_props.encoding.eq_ignore_ascii_case("UTF-8") && ch.len_utf8() > 1 {
                            let msg = alloc::format!(
                                "Runtime Schema Definition Error: fillByte property must be a single-byte character for encoding '{}', but '{}' takes {} bytes",
                                local_props.encoding, ch, ch.len_utf8()
                            );
                            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                        }
                    }
                }
                }
                has_local = true;
            } else {
                local_props = term.properties.clone();
            }
            if let Some(ref bo_expr) = term.properties.byte_order_expr {
                let expr_clean = bo_expr.trim().strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(bo_expr).trim();
                let ast = crate::expr::parse_expr(expr_clean)?;
                let mut ctx = self.make_expr_context();
                if let Ok(val) = crate::expr::eval_expr(&ast, &mut ctx) {
                    local_props.byte_order =
                        crate::expr::properties::parse_byte_order(&alloc::format!("{}", val))?;
                    has_local = true;
                }
            }
            let props = if has_local {
                &local_props
            } else {
                &term.properties
            };

            if let Some(ref term_str) = props.terminator {
                let term_tokens = self.extract_delimiter_tokens(term_str, props);
                self.active_delimiters.extend(term_tokens);
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

            // Enforce bitOrder change only on byte boundary (§11.2)
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

            // Left framing: leadingSkip
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

            // Left framing: alignment
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
                let rem = current_pos % align_bits;
                if rem > 0 {
                    let pad = align_bits - rem;
                    let fill = fill_byte_value(props)?;
                    write_fill_padding(self.writer, fill, pad)?;
                }
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

            if let Some(ref ovc_expr) = props.output_value_calc {
                let ast = crate::expr::parse_expr(ovc_expr)?;
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(&ast, &mut ctx)?;
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
                                )?;
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
                                    sub_unparser.current_path = self.current_path.clone();
                                    sub_unparser.active_delimiters = self.active_delimiters.clone();
                                    sub_unparser.current_occurs_index = self.current_occurs_index;
                                    sub_unparser.variable_map = self.variable_map.clone();
                                    sub_unparser.unparse_term(child_id, elem)?;
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

        self.active_delimiters.truncate(saved_delims_len);
        self.current_path.pop();
        res
    }

    fn write_prefix_length(
        &mut self,
        val_len: usize,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let ptype = props
            .prefix_length_type
            .as_deref()
            .unwrap_or("xs:unsignedShort");
        if ptype.contains('@') {
            let msg = alloc::format!(
                "Unparse Error: Nested dfdl:lengthKind='prefixed' is not supported (prefixLengthType '{}')",
                ptype
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }
        let parts: Vec<&str> = ptype.split(':').collect();
        let (is_text, prefix_bits) = if parts.len() >= 4 {
            let rep = parts.get(1).copied().unwrap_or("binary");
            let units = parts.get(3).copied().unwrap_or("bytes");
            let is_txt = rep.eq_ignore_ascii_case("text");
            let num_len: usize = parts
                .get(2)
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    let clean_ptype = parts.first().copied().unwrap_or(ptype);
                    match clean_ptype {
                        "byte" | "unsignedByte" => 1,
                        "short" | "unsignedShort" => 2,
                        "int" | "unsignedInt" => 4,
                        "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 8,
                        _ => 2,
                    }
                });
            let bits = if units.eq_ignore_ascii_case("bits") {
                num_len
            } else {
                num_len.saturating_mul(8)
            };
            (is_txt, bits)
        } else {
            let clean_ptype = parts.last().copied().unwrap_or(ptype);
            let bits = match clean_ptype {
                "byte" | "unsignedByte" => 8,
                "short" | "unsignedShort" => 16,
                "int" | "unsignedInt" => 32,
                "long" | "unsignedLong" | "integer" | "nonNegativeInteger" => 64,
                _ => 16,
            };
            (false, bits)
        };
        let prefix_units = match props.length_units {
            crate::schema::ir::LengthUnits::Bits => prefix_bits,
            crate::schema::ir::LengthUnits::Bytes
            | crate::schema::ir::LengthUnits::Characters => prefix_bits.div_ceil(8),
        };
        let raw_val = if props.prefix_includes_prefix_length {
            (val_len as u64).saturating_add(prefix_units as u64)
        } else {
            val_len as u64
        };
        let min_inc = parts.get(4).copied().unwrap_or("");
        let max_inc = parts.get(5).copied().unwrap_or("");
        if !max_inc.is_empty() {
            if let Ok(max) = max_inc.parse::<u64>() {
                if raw_val > max {
                    let name = self
                        .current_path
                        .segments()
                        .last()
                        .map(|s| s.as_str())
                        .unwrap_or("element");
                    let msg = alloc::format!("Unparse Error: failed check {name} ({raw_val}) facet maxInclusive ({max})");
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }
            }
        }
        if !min_inc.is_empty() {
            if let Ok(min) = min_inc.parse::<u64>() {
                if raw_val < min {
                    let name = self
                        .current_path
                        .segments()
                        .last()
                        .map(|s| s.as_str())
                        .unwrap_or("element");
                    let msg = alloc::format!("Unparse Error: failed check {name} ({raw_val}) facet minInclusive ({min})");
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }
            }
        }
        if is_text {
            let s = alloc::format!("{}", raw_val);
            let target_bytes = prefix_bits.div_ceil(8);
            let out = if s.len() < target_bytes {
                let pad_len = target_bytes - s.len();
                let mut padded = String::with_capacity(target_bytes);
                for _ in 0..pad_len {
                    padded.push(' ');
                }
                padded.push_str(&s);
                padded
            } else {
                s
            };
            for &b in out.as_bytes() {
                self.writer.write_bits(b as u64, 8)?;
            }
        } else {
            let prefix_val_u64 = match props.byte_order {
                ByteOrder::BigEndian => raw_val,
                ByteOrder::LittleEndian => match prefix_bits {
                    16 => (raw_val as u16).swap_bytes() as u64,
                    24 => {
                        let b0 = raw_val & 0xFF;
                        let b1 = (raw_val >> 8) & 0xFF;
                        let b2 = (raw_val >> 16) & 0xFF;
                        (b0 << 16) | (b1 << 8) | b2
                    }
                    32 => (raw_val as u32).swap_bytes() as u64,
                    64 => raw_val.swap_bytes(),
                    _ => raw_val,
                },
            };
            self.writer.write_bits(prefix_val_u64, prefix_bits)?;
        }
        Ok(())
    }

/// Remaps Unicode Private Use Area (PUA) codepoints representing XML-illegal or protected
/// control characters back to their raw ASCII/control characters per DFDL infoset representation.
///
/// In DFDL infosets (e.g. XML Infoset serialization in TDML test suites), characters that cannot
/// be represented directly in XML 1.0 (such as ASCII NUL or C0 control characters) or characters
/// that would be normalized by XML parsers (such as carriage return `%CR;` U+000D) are mapped
/// to the Unicode Private Use Area:
/// - U+E000..=U+E01F maps to raw C0 control bytes 0x00..=0x1F (e.g. U+E000 -> 0x00, U+E00D -> 0x0D `%CR;`).
/// - U+E07F maps to raw ASCII DEL 0x7F.
/// - U+E080..=U+E09F maps to raw C1 control bytes 0x80..=0x9F.
///
/// When unparsing, these PUA characters must be mapped back to their original byte/character values.
fn remap_pua_to_raw_chars(text: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(text.len());
    for c in text.chars() {
        let u = c as u32;
        if (0xE000..=0xE01F).contains(&u) || (0xE080..=0xE09F).contains(&u) {
            if let Some(mapped) = core::char::from_u32(u - 0xE000) {
                out.push(mapped);
            } else {
                out.push(c);
            }
        } else if u == 0xE07F {
            out.push('\x7F');
        } else {
            out.push(c);
        }
    }
    out
}

    fn unparse_simple_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let remapped_val;
        let val = if let DfdlValue::String(ref s) = val {
            if s.chars().any(|c| {
                (0xE000..=0xE01F).contains(&(c as u32))
                    || c == '\u{E07F}'
                    || (0xE080..=0xE09F).contains(&(c as u32))
            }) {
                remapped_val = DfdlValue::String(Self::remap_pua_to_raw_chars(s));
                &remapped_val
            } else {
                val
            }
        } else {
            val
        };

        if props.length_kind == crate::schema::ir::LengthKind::Prefixed {
            let val_len = match val {
                DfdlValue::String(s) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => s.len().saturating_mul(8),
                    _ => s.len(),
                },
                DfdlValue::HexBinary(b) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => b.len().saturating_mul(8),
                    _ => b.len(),
                },
                DfdlValue::DateTime(s)
                | DfdlValue::Date(s)
                | DfdlValue::Time(s)
                | DfdlValue::Decimal(s) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => s.len().saturating_mul(8),
                    _ => s.len(),
                },
                DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) | DfdlValue::Float(_) | DfdlValue::Boolean(_) => {
                    match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => 32,
                        _ => 4,
                    }
                }
                DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) | DfdlValue::Double(_) => {
                    match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => 64,
                        _ => 8,
                    }
                }
                DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => 16,
                    _ => 2,
                },
                DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => 8,
                    _ => 1,
                },
            };
            self.write_prefix_length(val_len, props)?;
        }

        match props.representation {
            Representation::Binary => self.unparse_binary_value(val, props),
            Representation::Text => {
                // DFDL 1.0 §13.7: Raw binary data (xs:hexBinary or blob bytes) must always
                // be unparsed using binary byte output even if representation defaulted to text.
                if let DfdlValue::HexBinary(_) = val {
                    self.unparse_binary_value(val, props)
                } else {
                    self.unparse_text_value(val, props)
                }
            }
        }
    }

    fn resolve_explicit_length(&mut self, props: &ResolvedProperties) -> DFDLResult<Option<usize>> {
        if let Some(l) = props.length {
            return Ok(Some(l));
        }
        if let Some(ref expr_str) = props.length_expr {
            if let Ok(ast) = crate::expr::parse_expr(expr_str) {
                let mut ctx = self.make_expr_context();
                let val = crate::expr::eval_expr(&ast, &mut ctx)?;
                let len_opt = match val {
                    DfdlValue::Int(v) if v >= 0 => Some(v as usize),
                    DfdlValue::Long(v) if v >= 0 => Some(v as usize),
                    DfdlValue::UnsignedInt(v) => Some(v as usize),
                    DfdlValue::UnsignedLong(v) => Some(v as usize),
                    DfdlValue::UnsignedShort(v) => Some(v as usize),
                    DfdlValue::UnsignedByte(v) => Some(v as usize),
                    DfdlValue::Short(v) if v >= 0 => Some(v as usize),
                    DfdlValue::Byte(v) if v >= 0 => Some(v as usize),
                    DfdlValue::String(s) => s.trim().parse::<usize>().ok(),
                    _ => None,
                };
                if len_opt.is_some() {
                    return Ok(len_opt);
                }
            }
            let inner = expr_str.trim();
            let stripped = if inner.starts_with('{') && inner.ends_with('}') {
                &inner[1..inner.len().saturating_sub(1)]
            } else {
                inner
            };
            if let Ok(val) = stripped.trim().parse::<usize>() {
                return Ok(Some(val));
            }
        }
        Ok(None)
    }

    fn unparse_binary_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let explicit_len_opt = self.resolve_explicit_length(props)?;
        let requested_bits_opt = explicit_len_opt.map(|len| match props.length_units {
            crate::schema::ir::LengthUnits::Bits => len,
            crate::schema::ir::LengthUnits::Bytes | crate::schema::ir::LengthUnits::Characters => {
                len.saturating_mul(8)
            }
        });

        if props.binary_number_rep == crate::schema::ir::BinaryNumberRep::Binary {
            if let Some(bits) = requested_bits_opt {
                let is_unsigned_int = matches!(
                    val,
                    DfdlValue::UnsignedByte(_)
                        | DfdlValue::UnsignedShort(_)
                        | DfdlValue::UnsignedInt(_)
                        | DfdlValue::UnsignedLong(_)
                ) || (matches!(val, DfdlValue::Decimal(_)) && !props.decimal_signed);
                let is_signed_int = matches!(
                    val,
                    DfdlValue::Byte(_)
                        | DfdlValue::Short(_)
                        | DfdlValue::Int(_)
                        | DfdlValue::Long(_)
                ) || (matches!(val, DfdlValue::Decimal(_)) && props.decimal_signed);

                if is_unsigned_int && bits == 0 {
                    let msg = alloc::format!(
                        "Unparse Error: unsigned binary number: minimum length is 1 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }

                if is_signed_int
                    && bits < 2
                    && (bits == 0 || self.schema.disallow_signed_integer_length_1bit)
                {
                    let msg = alloc::format!(
                        "Unparse Error: signed binary number: minimum length is 2 bit(s), but {} out of range",
                        bits
                    );
                    return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                }

                let (min_bits, max_bits, type_name) = match val {
                    DfdlValue::Byte(_) | DfdlValue::UnsignedByte(_) => (1, 8, "Byte"),
                    DfdlValue::Short(_) | DfdlValue::UnsignedShort(_) => (1, 16, "Short"),
                    DfdlValue::Int(_) | DfdlValue::UnsignedInt(_) => (1, 32, "Int"),
                    DfdlValue::Long(_) | DfdlValue::UnsignedLong(_) => (1, 64, "Long"),
                    _ => (0, 0, ""),
                };
                if max_bits > 0 && (bits < min_bits || bits > max_bits) {
                    let msg = alloc::format!(
                        "Schema Definition Error: Length in bits {} out of range. Expected between {} and {} for {}",
                        bits,
                        min_bits,
                        max_bits,
                        type_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
            }
        }

        match props.binary_number_rep {
            crate::schema::ir::BinaryNumberRep::Packed => {
                let val_i64 = match val {
                    DfdlValue::Int(v) => *v as i64,
                    DfdlValue::Long(v) => *v,
                    DfdlValue::Short(v) => *v as i64,
                    DfdlValue::Byte(v) => *v as i64,
                    DfdlValue::UnsignedInt(v) => *v as i64,
                    DfdlValue::UnsignedLong(v) => *v as i64,
                    DfdlValue::UnsignedShort(v) => *v as i64,
                    DfdlValue::UnsignedByte(v) => *v as i64,
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..(-scale) {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                        rounded as i64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as packed decimal", val),
                        ));
                    }
                };
                let bytes = crate::util::encode_packed_decimal(val_i64);
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let final_bytes = if let Some(target) = min_bytes {
                    if bytes.len() < target {
                        let pad = target.saturating_sub(bytes.len());
                        let mut padded = alloc::vec![0u8; pad];
                        padded.extend_from_slice(&bytes);
                        padded
                    } else {
                        bytes
                    }
                } else {
                    bytes
                };
                for b in final_bytes {
                    self.writer.write_bits(b as u64, 8)?;
                }
                return Ok(());
            }
            crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                let val_i64 = match val {
                    DfdlValue::Int(v) => *v as i64,
                    DfdlValue::Long(v) => *v,
                    DfdlValue::Short(v) => *v as i64,
                    DfdlValue::Byte(v) => *v as i64,
                    DfdlValue::UnsignedInt(v) => *v as i64,
                    DfdlValue::UnsignedLong(v) => *v as i64,
                    DfdlValue::UnsignedShort(v) => *v as i64,
                    DfdlValue::UnsignedByte(v) => *v as i64,
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..(-scale) {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                        rounded as i64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as ibm4690 packed decimal", val),
                        ));
                    }
                };
                let bytes = crate::util::encode_ibm4690_packed(val_i64);
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let final_bytes = if let Some(target) = min_bytes {
                    if bytes.len() < target {
                        let pad = target.saturating_sub(bytes.len());
                        let mut padded = alloc::vec![0xFFu8; pad];
                        padded.extend_from_slice(&bytes);
                        padded
                    } else {
                        bytes
                    }
                } else {
                    bytes
                };
                for b in final_bytes {
                    self.writer.write_bits(b as u64, 8)?;
                }
                return Ok(());
            }
            crate::schema::ir::BinaryNumberRep::Bcd => {
                let val_u64 = match val {
                    DfdlValue::UnsignedInt(v) => *v as u64,
                    DfdlValue::UnsignedLong(v) => *v,
                    DfdlValue::UnsignedShort(v) => *v as u64,
                    DfdlValue::UnsignedByte(v) => *v as u64,
                    DfdlValue::Int(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Long(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Short(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Byte(v) => {
                        if *v < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    v
                                ),
                            ));
                        }
                        *v as u64
                    }
                    DfdlValue::Decimal(s) | DfdlValue::String(s) => {
                        let parsed = s.parse::<f64>().map_err(|_| {
                            DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!("Invalid decimal: {}", s),
                            )
                        })?;
                        if parsed < 0.0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: bcd only positive, cannot be negative: {}",
                                    parsed
                                ),
                            ));
                        }
                        let scale = props.binary_decimal_virtual_point;
                        let mut multiplier = 1.0f64;
                        if scale >= 0 {
                            for _ in 0..scale {
                                multiplier *= 10.0;
                            }
                        } else {
                            for _ in 0..(-scale) {
                                multiplier /= 10.0;
                            }
                        }
                        let prod = parsed * multiplier;
                        let rounded = prod + 0.5;
                        rounded as u64
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!("Cannot unparse {:?} as bcd", val),
                        ));
                    }
                };
                let min_bytes = requested_bits_opt.map(|b| b.saturating_add(7) / 8);
                let bytes = crate::util::encode_bcd(val_u64, min_bytes)?;
                for b in bytes {
                    self.writer.write_bits(b as u64, 8)?;
                }
                return Ok(());
            }
            _ => {}
        }

        let needs_byte_swap = props.byte_order == ByteOrder::LittleEndian
            && props.bit_order == BitOrder::MostSignificantBitFirst;

        match val {
            DfdlValue::Int(v) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let val_u64 = if needs_byte_swap && bits == 32 {
                    (*v as u32).swap_bytes() as u64
                } else {
                    (*v as u32) as u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Long(v) => {
                let bits = requested_bits_opt.unwrap_or(
                    if *v >= (i32::MIN as i64) && *v <= (u32::MAX as i64) {
                        32
                    } else {
                        64
                    },
                );
                let val_u64 = if needs_byte_swap {
                    match bits {
                        32 => (*v as u32).swap_bytes() as u64,
                        64 => (*v as u64).swap_bytes(),
                        _ => *v as u64,
                    }
                } else if bits == 32 {
                    (*v as u64) & 0xFFFF_FFFF
                } else {
                    *v as u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Short(v) => {
                let bits = requested_bits_opt.unwrap_or(16);
                let val_u64 = if needs_byte_swap && bits == 16 {
                    (*v as u16).swap_bytes() as u64
                } else {
                    (*v as u16) as u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::Byte(v) => {
                let bits = requested_bits_opt.unwrap_or(8);
                self.writer.write_bits((*v as u8) as u64, bits)
            }
            DfdlValue::UnsignedInt(v) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let val_u64 = if needs_byte_swap && bits == 32 {
                    v.swap_bytes() as u64
                } else {
                    *v as u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedLong(v) => {
                let bits = requested_bits_opt.unwrap_or(64);
                let val_u64 = if needs_byte_swap {
                    match bits {
                        32 => (*v as u32).swap_bytes() as u64,
                        64 => v.swap_bytes(),
                        _ => *v,
                    }
                } else {
                    *v
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedShort(v) => {
                let bits = requested_bits_opt.unwrap_or(16);
                let val_u64 = if needs_byte_swap && bits == 16 {
                    v.swap_bytes() as u64
                } else {
                    *v as u64
                };
                self.writer.write_bits(val_u64, bits)
            }
            DfdlValue::UnsignedByte(v) => {
                let bits = requested_bits_opt.unwrap_or(8);
                self.writer.write_bits(*v as u64, bits)
            }
            DfdlValue::Boolean(b) => {
                let bits = requested_bits_opt.unwrap_or(32);
                let mask = if bits >= 64 {
                    u64::MAX
                } else {
                    (1u64 << bits).wrapping_sub(1)
                };
                let raw_val: u64 = if *b {
                    match props.binary_boolean_true_rep {
                        crate::schema::ir::BinaryBooleanRep::Value(t) => (t as u64) & mask,
                        crate::schema::ir::BinaryBooleanRep::Empty => {
                            match props.binary_boolean_false_rep {
                                crate::schema::ir::BinaryBooleanRep::Value(f) => (!f as u64) & mask,
                                _ => 1 & mask,
                            }
                        }
                        crate::schema::ir::BinaryBooleanRep::NotSpecified => 1 & mask,
                    }
                } else {
                    match props.binary_boolean_false_rep {
                        crate::schema::ir::BinaryBooleanRep::Value(f) => (f as u64) & mask,
                        crate::schema::ir::BinaryBooleanRep::Empty => {
                            match props.binary_boolean_true_rep {
                                crate::schema::ir::BinaryBooleanRep::Value(t) => (!t as u64) & mask,
                                _ => 0,
                            }
                        }
                        crate::schema::ir::BinaryBooleanRep::NotSpecified => 0,
                    }
                };
                let ordered = if needs_byte_swap {
                    match bits {
                        16 => (raw_val as u16).swap_bytes() as u64,
                        24 => {
                            let b0 = raw_val & 0xFF;
                            let b1 = (raw_val >> 8) & 0xFF;
                            let b2 = (raw_val >> 16) & 0xFF;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (raw_val as u32).swap_bytes() as u64,
                        64 => raw_val.swap_bytes(),
                        _ => raw_val,
                    }
                } else {
                    raw_val
                };
                self.writer.write_bits(ordered, bits)
            }
            DfdlValue::Float(f) => {
                let bits = if needs_byte_swap {
                    f.to_bits().swap_bytes()
                } else {
                    f.to_bits()
                };
                self.writer.write_bits(bits as u64, 32)
            }
            DfdlValue::Double(d) => {
                let bits = if needs_byte_swap {
                    d.to_bits().swap_bytes()
                } else {
                    d.to_bits()
                };
                self.writer.write_bits(bits, 64)
            }
            DfdlValue::HexBinary(b) => {
                let fill = fill_byte_value(props)?;
                let target_bytes = if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    self.resolve_explicit_length(props)?.map(|target_len| match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                        _ => target_len,
                    })
                } else if props.length_kind == crate::schema::ir::LengthKind::Implicit {
                    props
                        .facets
                        .length
                        .or(props.facets.max_length)
                        .or(props.facets.min_length)
                } else if props.length_kind == crate::schema::ir::LengthKind::Delimited {
                    props.facets.min_length
                } else {
                    None
                };
                if let Some(target) = target_bytes {
                    if b.len() > target {
                        let actual_bits = b.len().saturating_mul(8);
                        let target_bits = match (props.length_kind, props.length_units) {
                            (crate::schema::ir::LengthKind::Explicit, crate::schema::ir::LengthUnits::Bits) => {
                                self.resolve_explicit_length(props)?.unwrap_or(target.saturating_mul(8))
                            }
                            _ => target.saturating_mul(8),
                        };
                        let msg = alloc::format!(
                            "Unparse Error: Blob length {} bits exceeds {} bits",
                            actual_bits, target_bits
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
                    }
                    for &byte in b {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                    if b.len() < target {
                        for _ in 0..(target.saturating_sub(b.len())) {
                            self.writer.write_bits(fill as u64, 8)?;
                        }
                    }
                } else {
                    for &byte in b {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                }
                Ok(())
            }
            DfdlValue::String(s) => {
                let clean = s.trim();
                if clean.len() % 2 == 0
                    && !clean.is_empty()
                    && clean.chars().all(|c| c.is_ascii_hexdigit())
                {
                    let b_arr = clean.as_bytes();
                    for i in (0..b_arr.len()).step_by(2) {
                        if let (Some(&b1), Some(&b2)) =
                            (b_arr.get(i), b_arr.get(i.saturating_add(1)))
                        {
                            if let (Some(h1), Some(h2)) =
                                ((b1 as char).to_digit(16), (b2 as char).to_digit(16))
                            {
                                self.writer.write_bits(((h1 << 4) | h2) as u64, 8)?;
                            }
                        }
                    }
                    Ok(())
                } else if let Ok(val_i64) = clean.parse::<i64>() {
                    let bits = requested_bits_opt.unwrap_or(32);
                    let val_u64 = match props.byte_order {
                        ByteOrder::BigEndian => {
                            (val_i64 as u64)
                                & (if bits == 64 {
                                    u64::MAX
                                } else {
                                    (1u64 << bits).wrapping_sub(1)
                                })
                        }
                        ByteOrder::LittleEndian => match bits {
                            32 => (val_i64 as u32).swap_bytes() as u64,
                            64 => (val_i64 as u64).swap_bytes(),
                            _ => val_i64 as u64,
                        },
                    };
                    self.writer.write_bits(val_u64, bits)
                } else {
                    for &byte in s.as_bytes() {
                        self.writer.write_bits(byte as u64, 8)?;
                    }
                    Ok(())
                }
            }
            DfdlValue::Decimal(s) => {
                let parsed = s.parse::<f64>().map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Invalid decimal: {}", s),
                    )
                })?;
                let scale = props.binary_decimal_virtual_point;
                let mut multiplier = 1.0f64;
                if scale >= 0 {
                    for _ in 0..scale {
                        multiplier *= 10.0;
                    }
                } else {
                    for _ in 0..(-scale) {
                        multiplier /= 10.0;
                    }
                }
                let prod = parsed * multiplier;
                let rounded = if prod >= 0.0 { prod + 0.5 } else { prod - 0.5 };
                let val_i64 = rounded as i64;
                let bits = requested_bits_opt.unwrap_or(32);
                let raw_u64 = if bits >= 64 {
                    val_i64 as u64
                } else {
                    (val_i64 as u64) & ((1u64 << bits).wrapping_sub(1))
                };
                let ordered = match props.byte_order {
                    ByteOrder::BigEndian => raw_u64,
                    ByteOrder::LittleEndian => match bits {
                        16 => (raw_u64 as u16).swap_bytes() as u64,
                        24 => {
                            let b0 = raw_u64 & 0xFF;
                            let b1 = (raw_u64 >> 8) & 0xFF;
                            let b2 = (raw_u64 >> 16) & 0xFF;
                            (b0 << 16) | (b1 << 8) | b2
                        }
                        32 => (raw_u64 as u32).swap_bytes() as u64,
                        64 => raw_u64.swap_bytes(),
                        _ => raw_u64,
                    },
                };
                self.writer.write_bits(ordered, bits)
            }
            DfdlValue::DateTime(s)
            | DfdlValue::Date(s)
            | DfdlValue::Time(s) => {
                for &byte in s.as_bytes() {
                    self.writer.write_bits(byte as u64, 8)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn unparse_text_value(
        &mut self,
        val: &DfdlValue,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let mut s = match val {
            DfdlValue::Boolean(b) => {
                let rep_opt = if *b {
                    props.text_boolean_true_rep.as_deref()
                } else {
                    props.text_boolean_false_rep.as_deref()
                };
                if let Some(raw_rep) = rep_opt {
                    let trimmed = raw_rep.trim();
                    let eval_rep = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                        let ast = crate::expr::parse_expr(trimmed)?;
                        let mut ctx = self.make_expr_context();
                        let v = crate::expr::eval_expr(&ast, &mut ctx)?;
                        alloc::format!("{}", v)
                    } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                        alloc::format!("{{{stripped}")
                    } else {
                        String::from(raw_rep)
                    };
                    let first_alt = eval_rep.split_whitespace().next().unwrap_or("").to_string();
                    crate::expr::properties::decode_dfdl_character_entities(&first_alt)
                } else {
                    alloc::string::ToString::to_string(val)
                }
            }
            _ if props.text_standard_base != 10 => {
                if let Some(n) = val.as_i128() {
                    if n < 0 {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                                n,
                                props.text_standard_base
                            ),
                        ));
                    }
                    match props.text_standard_base {
                        2 => alloc::format!("{:b}", n),
                        8 => alloc::format!("{:o}", n),
                        16 => alloc::format!("{:x}", n),
                        _ => alloc::string::ToString::to_string(val),
                    }
                } else if let DfdlValue::String(ref s_val) = val {
                    let trimmed = s_val.trim();
                    if let Ok(n) = trimmed.parse::<i128>() {
                        if n < 0 {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                                    n,
                                    props.text_standard_base
                                ),
                            ));
                        }
                        match props.text_standard_base {
                            2 => alloc::format!("{:b}", n),
                            8 => alloc::format!("{:o}", n),
                            16 => alloc::format!("{:x}", n),
                            _ => alloc::string::ToString::to_string(val),
                        }
                    } else {
                        alloc::string::ToString::to_string(val)
                    }
                } else {
                    alloc::string::ToString::to_string(val)
                }
            }
            DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_)
            | DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_) => {
                let zero_rep_opt = if crate::kernel::parser::numbers::is_numeric_zero(val) {
                    props.text_standard_zero_rep.as_deref().and_then(|raw_zero| {
                        let eval_rep = self.eval_runtime_prop_str(raw_zero);
                        eval_rep.split_whitespace().next().and_then(|first| {
                            if !first.is_empty() {
                                Some(crate::expr::properties::decode_dfdl_character_entities(first))
                            } else {
                                None
                            }
                        })
                    })
                } else {
                    None
                };
                if let Some(z) = zero_rep_opt {
                    z
                } else {
                    let dec = self.eval_runtime_prop_str(&props.text_standard_decimal_separator);
                    let grp = self.eval_runtime_prop_str(&props.text_standard_grouping_separator);
                    let exp = props
                        .text_standard_exponent_rep
                        .as_deref()
                        .map(|s| self.eval_runtime_prop_str(s));
                    crate::kernel::parser::numbers::format_text_number(
                        val,
                        props.text_number_pattern.as_deref(),
                        &dec,
                        &grp,
                        exp.as_deref(),
                        crate::kernel::parser::rounding::NumberRounding::from_props(props),
                    )
                }
            }
            DfdlValue::DateTime(s_val) | DfdlValue::Date(s_val) | DfdlValue::Time(s_val) => {
                if let Some(ref pat) = props.calendar_pattern {
                    if !pat.is_empty() {
                        let lang = if let Some(ref raw_lang) = props.calendar_language {
                            let trimmed = raw_lang.trim();
                            let eval_lang = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                                let ast = crate::expr::parse_expr(trimmed)?;
                                let mut ctx = self.make_expr_context();
                                let v = crate::expr::eval_expr(&ast, &mut ctx)?;
                                alloc::format!("{}", v)
                            } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                                alloc::format!("{{{stripped}")
                            } else {
                                String::from(raw_lang)
                            };
                            crate::kernel::parser::calendar::validate_calendar_language_syntax(&eval_lang)?;
                            Some(eval_lang)
                        } else {
                            None
                        };
                        let tz = if let Some(ref raw_tz) = props.calendar_time_zone {
                            let trimmed = raw_tz.trim();
                            let eval_tz = if trimmed.starts_with('{') && !trimmed.starts_with("{{") {
                                let ast = crate::expr::parse_expr(trimmed)?;
                                let mut ctx = self.make_expr_context();
                                let v = crate::expr::eval_expr(&ast, &mut ctx)?;
                                alloc::format!("{}", v)
                            } else if let Some(stripped) = trimmed.strip_prefix("{{") {
                                alloc::format!("{{{stripped}")
                            } else {
                                String::from(raw_tz)
                            };
                            Some(eval_tz)
                        } else {
                            None
                        };
                        crate::kernel::parser::calendar::format_calendar_with_pattern(
                            s_val,
                            pat,
                            lang.as_deref(),
                            tz.as_deref(),
                        )?
                    } else {
                        alloc::string::ToString::to_string(val)
                    }
                } else {
                    alloc::string::ToString::to_string(val)
                }
            }
            _ => alloc::string::ToString::to_string(val),
        };
        if s.chars().any(|c| {
            (0xE000..=0xE01F).contains(&(c as u32))
                || c == '\u{E07F}'
                || (0xE080..=0xE09F).contains(&(c as u32))
        }) {
            s = Self::remap_pua_to_raw_chars(&s);
        }
        if let Some(cb) = crate::encoding::encoding_char_bits(&props.encoding) {
            return self.unparse_sub_byte_text(&s, val, props, cb);
        }
        if props.encoding_error_policy_error {
            if let Some(ch) = crate::encoding::find_unmappable_char(&s, &props.encoding) {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: UnmappableCharacterException: character '{}' (U+{:04X}) cannot be encoded in {}",
                        ch,
                        ch as u32,
                        props.encoding
                    ),
                ));
            }
        }
        validate_text_number_rounding(val, props)?;

        if let Some(ref scheme) = props.escape_scheme {
            s = self.apply_escape_scheme(&s, scheme);
        }

        let pad_str = match val {
            DfdlValue::Int(_)
            | DfdlValue::Long(_)
            | DfdlValue::Short(_)
            | DfdlValue::Byte(_)
            | DfdlValue::UnsignedInt(_)
            | DfdlValue::UnsignedLong(_)
            | DfdlValue::UnsignedShort(_)
            | DfdlValue::UnsignedByte(_)
            | DfdlValue::Float(_)
            | DfdlValue::Double(_)
            | DfdlValue::Decimal(_) => props
                .text_number_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            DfdlValue::Boolean(_) => props
                .text_boolean_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            DfdlValue::DateTime(_) | DfdlValue::Date(_) | DfdlValue::Time(_) => props
                .text_calendar_pad_character
                .as_deref()
                .unwrap_or(props.text_pad_char.as_str()),
            _ => props.text_pad_char.as_str(),
        };
        let pad_ch = pad_str.chars().next().unwrap_or(' ');
        let pad_byte = pad_str.as_bytes().first().copied().unwrap_or(b' ');

        let is_chars = props.length_units == crate::schema::ir::LengthUnits::Characters;
        let justification = props.text_justification_for_value(val);

        if props.length_kind == crate::schema::ir::LengthKind::Delimited
            && props.text_pad_kind == crate::schema::ir::TextPadKind::PadChar
            && props.text_output_min_length > 0
        {
            let target_len = props.text_output_min_length;
            if is_chars {
                let char_count = s.chars().count();
                if char_count < target_len {
                    let pad_count = target_len - char_count;
                    let mut padded = String::with_capacity(s.len() + pad_count * 4);
                    match justification {
                        crate::schema::ir::TextJustification::Right => {
                            for _ in 0..pad_count {
                                padded.push(pad_ch);
                            }
                            padded.push_str(&s);
                        }
                        crate::schema::ir::TextJustification::Center => {
                            let pad_left = pad_count.div_ceil(2);
                            let pad_right = pad_count / 2;
                            for _ in 0..pad_left {
                                padded.push(pad_ch);
                            }
                            padded.push_str(&s);
                            for _ in 0..pad_right {
                                padded.push(pad_ch);
                            }
                        }
                        _ => {
                            if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                            } else {
                                padded.push_str(&s);
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                            }
                        }
                    }
                    s = padded;
                }
            } else {
                let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                let target_bytes = match props.length_units {
                    crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                    _ => target_len,
                };
                if encoded.len() < target_bytes {
                    let pad_count = target_bytes - encoded.len();
                    let mut padded_bytes = Vec::with_capacity(encoded.len() + pad_count);
                    match justification {
                        crate::schema::ir::TextJustification::Right => {
                            for _ in 0..pad_count {
                                padded_bytes.push(pad_byte);
                            }
                            padded_bytes.extend_from_slice(&encoded);
                        }
                        crate::schema::ir::TextJustification::Center => {
                            let pad_left = pad_count.div_ceil(2);
                            let pad_right = pad_count / 2;
                            for _ in 0..pad_left {
                                padded_bytes.push(pad_byte);
                            }
                            padded_bytes.extend_from_slice(&encoded);
                            for _ in 0..pad_right {
                                padded_bytes.push(pad_byte);
                            }
                        }
                        _ => {
                            if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                for _ in 0..pad_count {
                                    padded_bytes.push(pad_byte);
                                }
                                padded_bytes.extend_from_slice(&encoded);
                            } else {
                                padded_bytes.extend_from_slice(&encoded);
                                for _ in 0..pad_count {
                                    padded_bytes.push(pad_byte);
                                }
                            }
                        }
                    }
                    for &b in &padded_bytes {
                        self.writer.write_bits(b as u64, 8)?;
                    }
                    return Ok(());
                }
            }
        }

        if props.length_kind == crate::schema::ir::LengthKind::Explicit {
            if let Some(target_len) = self.resolve_explicit_length(props)? {
                if is_chars {
                    let char_count = s.chars().count();
                    if char_count < target_len {
                        if props.text_pad_kind == crate::schema::ir::TextPadKind::None {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length {} doesn't match expected output data length {} and textPadKind is 'none'",
                                    char_count,
                                    target_len
                                ),
                            ));
                        }
                        let pad_count = target_len - char_count;
                        let mut padded = String::with_capacity(s.len() + pad_count * 4);
                        match justification {
                            crate::schema::ir::TextJustification::Right => {
                                for _ in 0..pad_count {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                            }
                            crate::schema::ir::TextJustification::Center => {
                                let pad_left = pad_count.div_ceil(2);
                                let pad_right = pad_count / 2;
                                for _ in 0..pad_left {
                                    padded.push(pad_ch);
                                }
                                padded.push_str(&s);
                                for _ in 0..pad_right {
                                    padded.push(pad_ch);
                                }
                            }
                            _ => {
                                if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                    for _ in 0..pad_count {
                                        padded.push(pad_ch);
                                    }
                                    padded.push_str(&s);
                                } else {
                                    padded.push_str(&s);
                                    for _ in 0..pad_count {
                                        padded.push(pad_ch);
                                    }
                                }
                            }
                        }
                        let encoded = crate::encoding::encode_text_string(&padded, &props.encoding);
                        for &b in &encoded {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                    } else if char_count > target_len {
                        let can_truncate = props.truncate_specified_length_string;
                        if !can_truncate {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                    char_count,
                                    target_len
                                ),
                            ));
                        }
                        if justification == crate::schema::ir::TextJustification::Center {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                    char_count,
                                    target_len
                                ),
                            ));
                        }
                        let truncated: String = match justification {
                            crate::schema::ir::TextJustification::Right => {
                                let skip = char_count.saturating_sub(target_len);
                                s.chars().skip(skip).collect()
                            }
                            _ => s.chars().take(target_len).collect(),
                        };
                        let encoded = crate::encoding::encode_text_string(&truncated, &props.encoding);
                        for &b in &encoded {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                    } else {
                        let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                        for &b in &encoded {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                    }
                } else {
                    let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
                    let target_bytes = match props.length_units {
                        crate::schema::ir::LengthUnits::Bits => target_len.div_ceil(8),
                        _ => target_len,
                    };
                    if encoded.len() < target_bytes {
                        if props.text_pad_kind == crate::schema::ir::TextPadKind::None {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length {} doesn't match expected output data length {} and textPadKind is 'none'",
                                    encoded.len(),
                                    target_bytes
                                ),
                            ));
                        }
                        let pad_count = target_bytes - encoded.len();
                        match justification {
                            crate::schema::ir::TextJustification::Right => {
                                for _ in 0..pad_count {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                                for &b in &encoded {
                                    self.writer.write_bits(b as u64, 8)?;
                                }
                            }
                            crate::schema::ir::TextJustification::Center => {
                                let pad_left = pad_count.div_ceil(2);
                                let pad_right = pad_count / 2;
                                for _ in 0..pad_left {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                                for &b in &encoded {
                                    self.writer.write_bits(b as u64, 8)?;
                                }
                                for _ in 0..pad_right {
                                    self.writer.write_bits(pad_byte as u64, 8)?;
                                }
                            }
                            _ => {
                                if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                                    for _ in 0..pad_count {
                                        self.writer.write_bits(pad_byte as u64, 8)?;
                                    }
                                    for &b in &encoded {
                                        self.writer.write_bits(b as u64, 8)?;
                                    }
                                } else {
                                    for &b in &encoded {
                                        self.writer.write_bits(b as u64, 8)?;
                                    }
                                    for _ in 0..pad_count {
                                        self.writer.write_bits(pad_byte as u64, 8)?;
                                    }
                                }
                            }
                        }
                    } else if encoded.len() > target_bytes {
                        let can_truncate = props.truncate_specified_length_string;
                        if !can_truncate {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                    encoded.len(),
                                    target_bytes
                                ),
                            ));
                        }
                        if justification == crate::schema::ir::TextJustification::Center {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                    encoded.len(),
                                    target_bytes
                                ),
                            ));
                        }
                        let truncated_bytes = match justification {
                            crate::schema::ir::TextJustification::Right => {
                                let skip = encoded.len().saturating_sub(target_bytes);
                                encoded.get(skip..).unwrap_or(&[])
                            }
                            _ => encoded.get(..target_bytes).unwrap_or(&[]),
                        };
                        for &b in truncated_bytes {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                    } else {
                        for &b in &encoded {
                            self.writer.write_bits(b as u64, 8)?;
                        }
                    }
                }
                return Ok(());
            }
        }

        let encoded = crate::encoding::encode_text_string(&s, &props.encoding);
        for &b in &encoded {
            self.writer.write_bits(b as u64, 8)?;
        }
        Ok(())
    }

    /// Writes `s` in an encoding whose characters are `cb` bits wide (e.g. 5-bit packed, octal).
    ///
    /// Each character becomes one `cb`-bit group written in the writer's bit order. With an
    /// explicit length the text is padded with `textPadCharacter` or truncated (when allowed)
    ///
    /// A character with no code in the encoding raises an `UnmappableCharacterException`
    /// unparse error under `encodingErrorPolicy="error"`, otherwise it is written as the
    /// encoding's replacement code. Unmappable pad characters are written as code 0.
    fn unparse_sub_byte_text(
        &mut self,
        s: &str,
        val: &DfdlValue,
        props: &ResolvedProperties,
        cb: usize,
    ) -> DFDLResult<()> {
        let mut codes: Vec<u64> = Vec::new();
        for ch in s.chars() {
            let code = match crate::encoding::strict_sub_byte_code(ch, &props.encoding) {
                Some(c) => c,
                None if props.encoding_error_policy_error => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: UnmappableCharacterException: character '{}' (U+{:04X}) cannot be encoded in {}",
                            ch,
                            ch as u32,
                            props.encoding
                        ),
                    ));
                }
                None => crate::encoding::sub_byte_replacement_code(&props.encoding),
            };
            codes.push(code);
        }
        if props.length_kind == crate::schema::ir::LengthKind::Explicit {
            if let Some(len) = self.resolve_explicit_length(props)? {
                let target = match props.length_units {
                    crate::schema::ir::LengthUnits::Characters => len,
                    crate::schema::ir::LengthUnits::Bits => len.checked_div(cb).unwrap_or(0),
                    crate::schema::ir::LengthUnits::Bytes => {
                        len.saturating_mul(8).checked_div(cb).unwrap_or(0)
                    }
                };
                if codes.len() > target {
                    let can_truncate = matches!(val, DfdlValue::String(_))
                        && props.truncate_specified_length_string;
                    if !can_truncate {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Data length ({}) exceeds explicit length ({}) and cannot truncate",
                                codes.len(),
                                target
                            ),
                        ));
                    }
                    if props.text_string_justification == crate::schema::ir::TextJustification::Center {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: textStringJustification is 'center' but truncateSpecifiedLengthString is 'yes', cannot truncate data length ({}) to explicit length ({})",
                                codes.len(),
                                target
                            ),
                        ));
                    }
                    codes.truncate(target);
                } else {
                    let pad_ch = props.text_pad_char.chars().next().unwrap_or(' ');
                    let pad_code =
                        crate::encoding::strict_sub_byte_code(pad_ch, &props.encoding).unwrap_or(0);
                    let pad = alloc::vec![pad_code; target.saturating_sub(codes.len())];
                    if props.text_trim_kind == crate::schema::ir::TextTrimKind::Head {
                        codes.splice(0..0, pad);
                    } else {
                        codes.extend(pad);
                    }
                }
            }
        }
        for code in codes {
            self.writer.write_bits(code, cb)?;
        }
        Ok(())
    }
}

/// Validates text number rounding rules (§13.7.1.4). When `textNumberRoundingMode="roundUnnecessary"`,
/// raises an unparse error if rounding is required to format into the pattern.
fn validate_text_number_rounding(val: &DfdlValue, props: &ResolvedProperties) -> DFDLResult<()> {
    if props.text_number_rounding_mode != crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary {
        return Ok(());
    }
    let pat = match props.text_number_pattern.as_deref() {
        Some(p) => p,
        None => return Ok(()),
    };
    let pos_pat = pat.split(';').next().unwrap_or(pat);
    let max_frac_digits = if let Some(dot_pos) = pos_pat.find('.') {
        pos_pat[dot_pos + 1..]
            .chars()
            .take_while(|c| *c == '#' || *c == '0')
            .count()
    } else {
        0
    };
    let val_str = match val {
        DfdlValue::Decimal(s) => s.clone(),
        DfdlValue::Float(f) => alloc::format!("{}", f),
        DfdlValue::Double(d) => alloc::format!("{}", d),
        DfdlValue::String(s) => s.clone(),
        _ => return Ok(()),
    };
    if let Some(dot_idx) = val_str.find('.') {
        let frac_str = &val_str[dot_idx + 1..];
        let trimmed_frac = frac_str.trim_end_matches('0');
        if trimmed_frac.len() > max_frac_digits {
            let msg = alloc::format!(
                "Unparse Error: rounding is required for value '{}' with pattern '{}' but textNumberRoundingMode is 'roundUnnecessary'",
                val_str, pat
            );
            return Err(DFDLError::new(DFDLErrorKind::Unparse, &msg));
        }
    }
    Ok(())
}

fn validate_unparse_value(
    val: &DfdlValue,
    st: &crate::infoset::DfdlSimpleType,
    props: &ResolvedProperties,
    max_hex_binary_length_in_bytes: Option<usize>,
) -> DFDLResult<()> {
    if props.binary_number_rep == crate::schema::ir::BinaryNumberRep::Bcd && val.is_negative() {
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: bcd only positive, cannot be negative: {}",
                val
            ),
        ));
    }

    let str_rep = alloc::format!("{}", val);
    let trimmed = str_rep.trim();

    if !props.decimal_signed && (val.is_negative() || trimmed.starts_with('-')) {
        let rep_msg = match props.binary_number_rep {
            crate::schema::ir::BinaryNumberRep::Packed
            | crate::schema::ir::BinaryNumberRep::Ibm4690Packed => {
                "Packed binary negative decimalSigned no"
            }
            _ => "Binary negative decimalSigned no",
        };
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: {}: cannot unparse negative value when decimalSigned is 'no'",
                rep_msg
            ),
        ));
    }

    if props.representation == crate::schema::ir::Representation::Text
        && props.text_standard_base != 10
        && (val.is_negative() || trimmed.starts_with('-'))
    {
        return Err(DFDLError::new(
            DFDLErrorKind::Unparse,
            &alloc::format!(
                "Unparse Error: Unable to unparse negative value {} when textStandardBase=\"{}\"",
                trimmed,
                props.text_standard_base
            ),
        ));
    }

    if props.representation == crate::schema::ir::Representation::Text
        && props.text_standard_base != 10
    {
        return Ok(());
    }

    match st {
        crate::infoset::DfdlSimpleType::Int => {
            if let Some(val_num) = val.as_i128() {
                i32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:int", trimmed),
                    )
                })?;
            } else if trimmed.parse::<i32>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:int", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Long => {
            if let Some(val_num) = val.as_i128() {
                i64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:long", trimmed),
                    )
                })?;
            } else if trimmed.parse::<i64>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:long", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Short => {
            if let Some(val_num) = val.as_i128() {
                i16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:short", trimmed),
                    )
                })?;
            } else if trimmed.parse::<i16>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:short", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Byte => {
            if let Some(val_num) = val.as_i128() {
                i8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value {} is not a valid xs:byte", trimmed),
                    )
                })?;
            } else if trimmed.parse::<i8>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:byte", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedLong => {
            if val.is_negative() || trimmed.starts_with('-') {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedLong",
                        trimmed
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u64::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedLong",
                            trimmed
                        ),
                    )
                })?;
            } else if trimmed.parse::<u64>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedLong",
                        trimmed
                    ),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedInt => {
            if val.is_negative() || trimmed.starts_with('-') {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedInt",
                        trimmed
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u32::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedInt",
                            trimmed
                        ),
                    )
                })?;
            } else if trimmed.parse::<u32>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedInt",
                        trimmed
                    ),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedShort => {
            if val.is_negative() || trimmed.starts_with('-') {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedShort",
                        trimmed
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u16::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedShort",
                            trimmed
                        ),
                    )
                })?;
            } else if trimmed.parse::<u16>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedShort",
                        trimmed
                    ),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::UnsignedByte => {
            if val.is_negative() || trimmed.starts_with('-') {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedByte",
                        trimmed
                    ),
                ));
            }
            if let Some(val_num) = val.as_i128() {
                u8::try_from(val_num).map_err(|_| {
                    DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:unsignedByte",
                            trimmed
                        ),
                    )
                })?;
            } else if trimmed.parse::<u8>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!(
                        "Unparse Error: Value {} is not a valid xs:unsignedByte",
                        trimmed
                    ),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Double => {
            if matches!(val, DfdlValue::String(_)) && trimmed.parse::<f64>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:double", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Float => {
            if matches!(val, DfdlValue::String(_)) && trimmed.parse::<f32>().is_err() {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value {} is not a valid xs:float", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::HexBinary => {
            if let DfdlValue::String(ref s) = val {
                let clean = s.trim();
                if clean.len() % 2 != 0 {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:hexBinary: even number of characters required",
                            clean
                        ),
                    ));
                }
                if !clean.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:hexBinary: invalid hex digits",
                            clean
                        ),
                    ));
                }
                if let Some(max_len) = max_hex_binary_length_in_bytes {
                    let hex_len = clean.len() / 2;
                    if hex_len > max_len {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: xs:hexBinary length ({}) exceeds maximum allowed length of {} bytes",
                                hex_len, max_len
                            ),
                        ));
                    }
                }
                if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    if let Some(exp_len) = props.length {
                        let exp_bits = match props.length_units {
                            crate::schema::ir::LengthUnits::Bits => exp_len,
                            crate::schema::ir::LengthUnits::Bytes
                            | crate::schema::ir::LengthUnits::Characters => {
                                exp_len.saturating_mul(8)
                            }
                        };
                        let data_bits = (clean.len() / 2).saturating_mul(8);
                        if data_bits > exp_bits {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: xs:hexBinary calculated length ({} bits) is greater than explicit length ({} bits)",
                                    data_bits, exp_bits
                                ),
                            ));
                        }
                    }
                }
            } else if let DfdlValue::HexBinary(ref b) = val {
                if let Some(max_len) = max_hex_binary_length_in_bytes {
                    if b.len() > max_len {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: xs:hexBinary length ({}) exceeds maximum allowed length of {} bytes",
                                b.len(), max_len
                            ),
                        ));
                    }
                }
                if props.length_kind == crate::schema::ir::LengthKind::Explicit {
                    if let Some(exp_len) = props.length {
                        let exp_bits = match props.length_units {
                            crate::schema::ir::LengthUnits::Bits => exp_len,
                            crate::schema::ir::LengthUnits::Bytes
                            | crate::schema::ir::LengthUnits::Characters => {
                                exp_len.saturating_mul(8)
                            }
                        };
                        let data_bits = b.len().saturating_mul(8);
                        if data_bits > exp_bits {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Unparse,
                                &alloc::format!(
                                    "Unparse Error: xs:hexBinary calculated length ({} bits) is greater than explicit length ({} bits)",
                                    data_bits, exp_bits
                                ),
                            ));
                        }
                    }
                }
            }
        }
        crate::infoset::DfdlSimpleType::Boolean => {
            if !matches!(trimmed, "true" | "false" | "1" | "0") {
                return Err(DFDLError::new(
                    DFDLErrorKind::Unparse,
                    &alloc::format!("Unparse Error: Value '{}' is not a valid xs:boolean", trimmed),
                ));
            }
        }
        crate::infoset::DfdlSimpleType::Date
        | crate::infoset::DfdlSimpleType::Time
        | crate::infoset::DfdlSimpleType::DateTime => {
            if let DfdlValue::String(ref s) = val {
                let clean = s.trim();
                let is_cal = clean.contains('-') || clean.contains(':') || clean.contains('T');
                if !is_cal {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Value '{}' is not a calendar", clean),
                    ));
                }
            }
        }
        crate::infoset::DfdlSimpleType::Decimal => {
            if props.facets.min_inclusive.as_deref() == Some("0") {
                let is_non_neg_int = !trimmed.starts_with('-')
                    && !trimmed.contains('.')
                    && trimmed
                        .strip_prefix('+')
                        .unwrap_or(trimmed)
                        .chars()
                        .all(|c| c.is_ascii_digit());
                if !is_non_neg_int {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is out of range for type xs:nonNegativeInteger",
                            trimmed
                        ),
                    ));
                }
            } else if props.facets.fraction_digits == Some(0) {
                let is_int = !trimmed.contains('.')
                    && trimmed
                        .strip_prefix('+')
                        .or_else(|| trimmed.strip_prefix('-'))
                        .unwrap_or(trimmed)
                        .chars()
                        .all(|c| c.is_ascii_digit());
                if !is_int {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!(
                            "Unparse Error: Value {} is not a valid xs:integer",
                            trimmed
                        ),
                    ));
                }
            }
        }
        _ => {}
    }

    if let Some(explicit_len) = props.length {
        let bits = explicit_len.saturating_mul(8);
        if bits > 0 && bits < 64 {
            if let Some(v_i128) = val.as_i128() {
                if val.is_negative() {
                    let min_val = -(1_i128 << (bits.saturating_sub(1)));
                    if v_i128 < min_val {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Value {} cannot fit in signed binary number {} bit(s)",
                                val,
                                bits
                            ),
                        ));
                    }
                } else {
                    let max_val = (1_i128 << bits).saturating_sub(1);
                    if v_i128 > max_val {
                        let is_unsigned = matches!(
                            st,
                            crate::infoset::DfdlSimpleType::UnsignedLong
                                | crate::infoset::DfdlSimpleType::UnsignedInt
                                | crate::infoset::DfdlSimpleType::UnsignedShort
                                | crate::infoset::DfdlSimpleType::UnsignedByte
                        );
                        let kind_str = if is_unsigned { "unsigned" } else { "signed" };
                        return Err(DFDLError::new(
                            DFDLErrorKind::Unparse,
                            &alloc::format!(
                                "Unparse Error: Value {} cannot fit in {} binary number {} bit(s)",
                                val,
                                kind_str,
                                bits
                            ),
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Decodes DFDL character entities for unparsing into a string.
///
/// Converts DFDL entity references (%NL;, %CR;, %LF;, %SP;, etc.) into their
/// runtime string values, replacing %NL; with the target newline convention string.
pub fn decode_unparse_delimiter(val: &str, output_new_line: &str) -> String {
    let mut out = String::with_capacity(val.len());
    let bytes = val.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &val[i..];
        if rest.starts_with("%%") {
            out.push('%');
            i += 2;
        } else if rest.starts_with("%,") {
            out.push(',');
            i += 2;
        } else if rest.starts_with("%NL;") {
            out.push_str(output_new_line);
            i += 4;
        } else if rest.starts_with("%CR;") {
            out.push('\r');
            i += 4;
        } else if rest.starts_with("%LF;") {
            out.push('\n');
            i += 4;
        } else if rest.starts_with("%NEL;") {
            out.push('\u{0085}');
            i += 5;
        } else if rest.starts_with("%LS;") {
            out.push('\u{2028}');
            i += 4;
        } else if rest.starts_with("%FF;") {
            out.push('\x0C');
            i += 4;
        } else if rest.starts_with("%VT;") {
            out.push('\x0B');
            i += 4;
        } else if rest.starts_with("%SP;") {
            out.push(' ');
            i += 4;
        } else if rest.starts_with("%HT;") {
            out.push('\t');
            i += 4;
        } else if rest.starts_with("%NUL;") {
            out.push('\0');
            i += 5;
        } else if rest.starts_with("%ES;") {
            i += 4;
        } else if rest.starts_with("%WSP*;") {
            i += 6;
        } else if rest.starts_with("%WSP+;") {
            out.push(' ');
            i += 6;
        } else if rest.starts_with("%WSP;") {
            out.push(' ');
            i += 5;
        } else if rest.starts_with("%#") {
            if let Some(semi_pos) = rest.find(';') {
                let entity = &rest[2..semi_pos];
                let val_opt = if let Some(hex_part) = entity
                    .strip_prefix('x')
                    .or_else(|| entity.strip_prefix('r'))
                {
                    u32::from_str_radix(hex_part, 16).ok()
                } else if let Some(dec_part) = entity.strip_prefix('d') {
                    dec_part.parse::<u32>().ok()
                } else if entity.chars().all(|c| c.is_ascii_digit()) {
                    entity.parse::<u32>().ok()
                } else {
                    None
                };
                if let Some(code) = val_opt.and_then(char::from_u32) {
                    out.push(code);
                    i += semi_pos + 1;
                    continue;
                }
            }
            let ch = val[i..].chars().next().unwrap_or('%');
            out.push(ch);
            i += ch.len_utf8();
        } else {
            let ch = val[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Measures the exact value length in bits of an infoset element according to schema (§17).
///
/// For complex elements, unparses children with their alignment and formatting, but excludes
/// outer initiator, terminator, and outer framing.
/// For simple elements, unparses the value without framing.
pub fn measure_element_content_bits(
    schema: &CompiledSchema,
    doc: Option<&InfosetDocument>,
    elem: &InfosetElement,
    path: &crate::types::InfosetPath,
    vmap: Option<&crate::expr::variables::VariableMap>,
) -> DFDLResult<usize> {
    let term = schema.find_term_by_path(path).or_else(|| {
        schema.find_term_by_name(&elem.name.local_name).and_then(|id| schema.get_term(id))
    }).ok_or_else(|| {
        DFDLError::new_static(DFDLErrorKind::SchemaDefinition, "Term not found for measurement")
    })?;

    let sink = VecByteSink::new();
    let mut writer = BitWriter::new(sink, term.properties.bit_order, term.properties.byte_order);
    let mut budget = WorkBudget::new(usize::MAX);
    let mut unparser = UnparserEngine::new(schema, &mut writer, &mut budget);
    if let Some(vm) = vmap {
        unparser.variable_map = vm.clone();
    }
    unparser.doc = doc;
    unparser.current_path = path.clone();

    if let TermKind::Element(ref el) = term.kind {
        match el.type_ir {
            crate::schema::ir::CompiledType::Complex(child_id) => {
                unparser.unparse_term(child_id, elem)?;
            }
            crate::schema::ir::CompiledType::Simple(_) => {
                if let ElementState::Value(ref val) = elem.state {
                    unparser.unparse_simple_value(val, &term.properties)?;
                }
            }
        }
    }
    Ok(writer.position().0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infoset::DfdlValue;
    use crate::io::bitstream::BitWriter;
    use crate::io::traits::BitOrder;
    use crate::io::VecByteSink;
    use crate::limits::WorkBudget;
    use crate::schema::builder::SchemaBuilder;
    use crate::schema::ir::{LengthKind, Representation, ResolvedProperties};

    /// Unparses `val` as binary with the given byte order and returns the emitted bytes.
    fn unparse_binary_bytes(val: &DfdlValue, order: ByteOrder) -> alloc::vec::Vec<u8> {
        unparse_binary_bytes_len(val, order, None)
    }

    /// Like [`unparse_binary_bytes`] with an optional explicit length in bits.
    fn unparse_binary_bytes_len(
        val: &DfdlValue,
        order: ByteOrder,
        bits: Option<usize>,
    ) -> alloc::vec::Vec<u8> {
        let mut writer = BitWriter::new(
            VecByteSink::new(),
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Float),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        {
            let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
            let props = ResolvedProperties {
                representation: Representation::Binary,
                binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
                byte_order: order,
                length: bits,
                length_units: crate::schema::ir::LengthUnits::Bits,
                ..Default::default()
            };
            unparser.unparse_binary_value(val, &props).unwrap();
        }
        writer.flush().unwrap();
        writer.into_sink().into_vec()
    }

    /// Floats and doubles must be byte-swapped for little-endian output.
    #[test]
    fn test_unparse_binary_float_double_byte_order() {
        let f = DfdlValue::Float(1.0);
        assert_eq!(unparse_binary_bytes(&f, ByteOrder::BigEndian), [0x3F, 0x80, 0, 0]);
        assert_eq!(unparse_binary_bytes(&f, ByteOrder::LittleEndian), [0, 0, 0x80, 0x3F]);
        let d = DfdlValue::Double(1.0);
        assert_eq!(
            unparse_binary_bytes(&d, ByteOrder::LittleEndian),
            [0, 0, 0, 0, 0, 0, 0xF0, 0x3F]
        );
    }

    /// A narrow little-endian xs:int keeps its value; only 32-bit fields are byte-swapped.
    #[test]
    fn test_unparse_int_little_endian_narrow_not_swapped() {
        let one = DfdlValue::Int(1);
        assert_eq!(unparse_binary_bytes_len(&one, ByteOrder::LittleEndian, Some(8)), [1]);
        assert_eq!(
            unparse_binary_bytes_len(&one, ByteOrder::LittleEndian, Some(32)),
            [1, 0, 0, 0]
        );
    }

    #[test]
    fn test_unparse_number_truncation_prohibited_error() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::Double,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(4),
            truncate_specified_length_string: false,
            ..Default::default()
        };
        let val = DfdlValue::Double(0.555);
        let res = unparser.unparse_text_value(&val, &props);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.message.as_str().contains("cannot truncate"));
    }

    /// Runs `unparse_text_value` on a bare schema and returns the bytes (or the error message).
    fn unparse_text_bytes(
        val: &DfdlValue,
        props: &ResolvedProperties,
        order: crate::io::traits::BitOrder,
    ) -> Result<Vec<u8>, String> {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(sink, order, crate::io::traits::ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        {
            let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
            unparser
                .unparse_text_value(val, props)
                .map_err(|e| String::from(e.message.as_str()))?;
        }
        writer.flush().unwrap();
        Ok(writer.into_sink().into_vec())
    }

    /// Sub-byte encodings write `cb`-bit groups: "XX" in 5-bit LSBF is 29,29 packed from bit 0.
    #[test]
    fn test_unparse_sub_byte_text_packs_codes() {
        let props = ResolvedProperties {
            representation: Representation::Text,
            encoding: String::from("X-DFDL-5-BIT-PACKED-LSBF"),
            length_kind: LengthKind::Explicit,
            length: Some(10),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..Default::default()
        };
        let lsb = crate::io::traits::BitOrder::LeastSignificantBitFirst;
        let out = unparse_text_bytes(&DfdlValue::String(String::from("XX")), &props, lsb).unwrap();
        assert_eq!(out, [0xBD, 0x03]);
        // Too long for the 2-character budget and truncation is off: error.
        let err = unparse_text_bytes(&DfdlValue::String(String::from("XXX")), &props, lsb);
        assert!(err.unwrap_err().contains("cannot truncate"));
    }

    /// Octal text is 3 bits per digit.
    #[test]
    fn test_unparse_octal_text_three_bits_per_digit() {
        let props = ResolvedProperties {
            representation: Representation::Text,
            encoding: String::from("X-DFDL-OCTAL-MSBF"),
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        // 101 010 11(pad): "52" then "3" -> 101 010 011 -> 0xA9, 0x80
        let out = unparse_text_bytes(&DfdlValue::String(String::from("523")), &props, msb).unwrap();
        assert_eq!(out, [0xA9, 0x80]);
    }

    /// Unmappable characters: `replace` writes the replacement code, `error` fails the unparse.
    #[test]
    fn test_unparse_sub_byte_encoding_error_policy() {
        let mut props = ResolvedProperties {
            representation: Representation::Text,
            encoding: String::from("X-DFDL-US-ASCII-7-BIT-PACKED"),
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        let pound = DfdlValue::String(String::from("\u{A3}"));
        // '?' = 0111111 (7 bits) then flushed with zero padding.
        let out = unparse_text_bytes(&pound, &props, msb).unwrap();
        assert_eq!(out, [0b0111_1110]);
        props.encoding_error_policy_error = true;
        let err = unparse_text_bytes(&pound, &props, msb).unwrap_err();
        assert!(err.contains("UnmappableCharacterException"));
    }

    /// Byte encodings honour encodingErrorPolicy too: `replace` writes '?', `error` fails.
    #[test]
    fn test_unparse_byte_encoding_error_policy() {
        let mut props = ResolvedProperties {
            representation: Representation::Text,
            encoding: String::from("US-ASCII"),
            ..Default::default()
        };
        let msb = crate::io::traits::BitOrder::MostSignificantBitFirst;
        let pound = DfdlValue::String(String::from("\u{A3}"));
        assert_eq!(unparse_text_bytes(&pound, &props, msb).unwrap(), [b'?']);
        props.encoding_error_policy_error = true;
        let err = unparse_text_bytes(&pound, &props, msb).unwrap_err();
        assert!(err.contains("UnmappableCharacterException"));
    }

    #[test]
    fn test_unparse_nil_attribute_validation() {
        let sink = VecByteSink::new();
        let mut writer = BitWriter::new(
            sink,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut budget = WorkBudget::new(1000);
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("root"),
                TermKind::Element(elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Element with nil_attribute: Some(true) on non-nillable element
        let mut unparser1 = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let doc1 = crate::infoset::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                crate::types::QName::local("root"),
                crate::infoset::state::ElementState::Nil,
            )
            .with_nil_attribute(Some(true)),
        );
        let err1 = unparser1.unparse_document(&doc1).unwrap_err();
        assert!(err1
            .message
            .as_str()
            .contains("defines nil property but is not nillable"));

        // 2. Element with nil_attribute: Some(false) on non-nillable element
        let mut unparser2 = UnparserEngine::new(&schema, &mut writer, &mut budget);
        let doc2 = crate::infoset::InfosetDocument::with_root(
            crate::infoset::tree::InfosetElement::simple(
                crate::types::QName::local("root"),
                crate::infoset::state::ElementState::Value(DfdlValue::String(
                    alloc::string::String::from("val"),
                )),
            )
            .with_nil_attribute(Some(false)),
        );
        let err2 = unparser2.unparse_document(&doc2).unwrap_err();
        assert!(err2
            .message
            .as_str()
            .contains("defines nil property but is not nillable"));
    }

    #[test]
    fn test_unparse_unexpected_child_and_simple_element_with_children() {
        let mut budget = WorkBudget::new(1000);

        // 1. Simple element with child elements
        let sink1 = VecByteSink::new();
        let mut writer1 = BitWriter::new(
            sink1,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut builder = SchemaBuilder::new();
        let simple_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("simple_root"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("simple_root"),
                TermKind::Element(simple_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema1 = builder.build().unwrap();

        let mut unparser1 = UnparserEngine::new(&schema1, &mut writer1, &mut budget);
        let mut bad_simple_elem = crate::infoset::tree::InfosetElement::complex(
            crate::types::QName::local("simple_root"),
        );
        bad_simple_elem
            .try_add_child(crate::infoset::tree::InfosetNode::Element(
                crate::infoset::tree::InfosetElement::simple(
                    crate::types::QName::local("child"),
                    crate::infoset::state::ElementState::Value(DfdlValue::Int(1)),
                ),
            ))
            .unwrap();
        let doc1 = crate::infoset::InfosetDocument::with_root(bad_simple_elem);
        let err1 = unparser1.unparse_document(&doc1).unwrap_err();
        assert!(err1.message.as_str().contains("Illegal content"));
        assert!(err1.message.as_str().contains("simple element"));

        // 2. Choice with unexpected child
        let sink2 = VecByteSink::new();
        let mut writer2 = BitWriter::new(
            sink2,
            crate::io::traits::BitOrder::MostSignificantBitFirst,
            crate::io::traits::ByteOrder::BigEndian,
        );
        let mut builder2 = SchemaBuilder::new();
        let c1_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("c1"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let c1_id = builder2
            .add_term_with_props(
                crate::types::QName::local("c1"),
                TermKind::Element(c1_elem),
                ResolvedProperties::default(),
            )
            .unwrap();

        let c2_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("c2"),
            type_ir: crate::schema::ir::CompiledType::Simple(
                crate::infoset::DfdlSimpleType::String,
            ),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let c2_id = builder2
            .add_term_with_props(
                crate::types::QName::local("c2"),
                TermKind::Element(c2_elem),
                ResolvedProperties::default(),
            )
            .unwrap();

        let choice_id = builder2
            .add_term_with_props(
                crate::types::QName::local("choice"),
                TermKind::Choice(crate::schema::ir::CompiledChoice {
                    branches: alloc::vec![c1_id, c2_id],
                }),
                ResolvedProperties::default(),
            )
            .unwrap();

        let choice_root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root_choice"),
            type_ir: crate::schema::ir::CompiledType::Complex(choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let choice_root_id = builder2
            .add_term_with_props(
                crate::types::QName::local("root_choice"),
                TermKind::Element(choice_root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder2.set_root(choice_root_id);
        let schema2 = builder2.build().unwrap();

        let mut unparser2 = UnparserEngine::new(&schema2, &mut writer2, &mut budget);
        let mut bad_choice_elem = crate::infoset::tree::InfosetElement::complex(
            crate::types::QName::local("root_choice"),
        );
        bad_choice_elem
            .try_add_child(crate::infoset::tree::InfosetNode::Element(
                crate::infoset::tree::InfosetElement::simple(
                    crate::types::QName::local("unexpected_branch"),
                    crate::infoset::state::ElementState::Value(DfdlValue::Int(1)),
                ),
            ))
            .unwrap();
        let doc2 = crate::infoset::InfosetDocument::with_root(bad_choice_elem);
        let err2 = unparser2.unparse_document(&doc2).unwrap_err();
        assert!(err2.message.as_str().contains("Found next element"));
        assert!(err2.message.as_str().contains("expected one of"));
        assert!(err2.message.as_str().contains("c1"));
        assert!(err2.message.as_str().contains("c2"));
    }
    /// Padding with an undefined fillByte is a Schema Definition Error (DFDL 12.3.3 / Table 3).
    #[test]
    fn test_fill_byte_value_requires_definition() {
        let mut props = ResolvedProperties {
            fill_byte: 0x20,
            ..ResolvedProperties::default()
        };
        assert_eq!(fill_byte_value(&props).unwrap(), 0x20);
        props.fill_byte_defined = false;
        let err = fill_byte_value(&props).unwrap_err();
        assert!(err.message.as_str().contains("Property fillByte is not defined"));
    }

    /// Verifies that string truncation and padding respect textStringJustification (§13.2).
    #[test]
    fn test_unparse_string_justification_and_truncation() {
        let mut props = ResolvedProperties {
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(5),
            truncate_specified_length_string: true,
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_pad_char: String::from("#"),
            ..ResolvedProperties::default()
        };

        // Right justification: truncate left, pad left
        props.text_string_justification = crate::schema::ir::TextJustification::Right;
        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut budget = WorkBudget::new(10_000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser
            .unparse_simple_value(&DfdlValue::String(String::from("12345678")), &props)
            .unwrap();
        writer.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer.into_sink().as_slice()).unwrap(), "45678");

        // Padding with Right justification
        let mut writer_pad = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser_pad = UnparserEngine::new(&schema, &mut writer_pad, &mut budget);
        unparser_pad
            .unparse_simple_value(&DfdlValue::String(String::from("12")), &props)
            .unwrap();
        writer_pad.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_pad.into_sink().as_slice()).unwrap(), "###12");

        // Center justification: truncation is prohibited by DFDL §13.7.1.3
        props.text_string_justification = crate::schema::ir::TextJustification::Center;
        let mut writer_ctr = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut unparser_ctr = UnparserEngine::new(&schema, &mut writer_ctr, &mut budget);
        let err = unparser_ctr
            .unparse_simple_value(&DfdlValue::String(String::from("1234567")), &props)
            .unwrap_err();
        assert!(err.message.as_str().contains("textStringJustification is 'center'"));
        assert!(err.message.as_str().contains("truncateSpecifiedLengthString is 'yes'"));
    }

    /// Verifies that number padding respects textNumberJustification and textNumberPadCharacter (§13.2.1).
    #[test]
    fn test_unparse_text_number_justification_and_padding() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let num_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("num"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                crate::types::QName::local("num"),
                TermKind::Element(num_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let base_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            text_pad_kind: crate::schema::ir::TextPadKind::PadChar,
            text_number_pad_character: Some(String::from("x")),
            ..ResolvedProperties::default()
        };

        // 1. Right justification: "xx20"
        let mut props_right = base_props.clone();
        props_right.text_number_justification = crate::schema::ir::TextJustification::Right;
        let mut writer_r = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_r = UnparserEngine::new(&schema, &mut writer_r, &mut budget);
        unparser_r.unparse_simple_value(&DfdlValue::Int(20), &props_right).unwrap();
        writer_r.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_r.into_sink().as_slice()).unwrap(), "xx20");

        // 2. Left justification: "20xx"
        let mut props_left = base_props.clone();
        props_left.text_number_justification = crate::schema::ir::TextJustification::Left;
        let mut writer_l = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_l = UnparserEngine::new(&schema, &mut writer_l, &mut budget);
        unparser_l.unparse_simple_value(&DfdlValue::Int(20), &props_left).unwrap();
        writer_l.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_l.into_sink().as_slice()).unwrap(), "20xx");

        // 3. Center justification: "x20x"
        let mut props_ctr = base_props;
        props_ctr.text_number_justification = crate::schema::ir::TextJustification::Center;
        let mut writer_c = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser_c = UnparserEngine::new(&schema, &mut writer_c, &mut budget);
        unparser_c.unparse_simple_value(&DfdlValue::Int(20), &props_ctr).unwrap();
        writer_c.flush().unwrap();
        assert_eq!(core::str::from_utf8(writer_c.into_sink().as_slice()).unwrap(), "x20x");
    }

    /// Verifies measure_element_content_bits accurately measures complex element children bit length with alignment (§17).
    #[test]
    fn test_measure_element_content_bits_complex_alignment() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        // Child 1: binary 4 bits
        let sublen_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("sublength"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p1 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p1.fill_byte_defined = true;
        let id1 = builder.add_term_with_props(crate::types::QName::local("sublength"), TermKind::Element(sublen_elem), p1).unwrap();

        // Child 2: binary 5 bits
        let sub1_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("subfield1"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p2 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(5),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 1,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p2.fill_byte_defined = true;
        let id2 = builder.add_term_with_props(crate::types::QName::local("subfield1"), TermKind::Element(sub1_elem), p2).unwrap();

        // Child 3: binary 4 bits, alignment 8 bits
        let sub2_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("subfield2"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let mut p3 = ResolvedProperties {
            representation: Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Bits,
            alignment: 8,
            alignment_units: crate::schema::ir::AlignmentUnits::Bits,
            ..ResolvedProperties::default()
        };
        p3.fill_byte_defined = true;
        let id3 = builder.add_term_with_props(crate::types::QName::local("subfield2"), TermKind::Element(sub2_elem), p3).unwrap();

        let seq_id = builder.add_term_with_props(
            crate::types::QName::local("seq"),
            TermKind::Sequence(crate::schema::ir::CompiledSequence { members: alloc::vec![id1, id2, id3] }),
            ResolvedProperties::default(),
        ).unwrap();

        let payload_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("payload"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let payload_id = builder.add_term_with_props(crate::types::QName::local("payload"), TermKind::Element(payload_elem), ResolvedProperties::default()).unwrap();
        builder.set_root(payload_id);
        let schema = builder.build().unwrap();

        let mut payload_node = crate::infoset::tree::InfosetElement::complex(crate::types::QName::local("payload"));
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("sublength"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(5)))
        )).unwrap();
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("subfield1"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(21)))
        )).unwrap();
        payload_node.try_add_child(crate::infoset::tree::InfosetNode::Element(
            crate::infoset::tree::InfosetElement::simple(crate::types::QName::local("subfield2"), crate::infoset::state::ElementState::Value(DfdlValue::UnsignedInt(10)))
        )).unwrap();

        let path = crate::types::InfosetPath::root();
        let bits = measure_element_content_bits(&schema, None, &payload_node, &path, None).unwrap();
        // 4 bits (sublength) + 5 bits (subfield1) + 7 bits (alignment padding to 8) + 4 bits (subfield2) = 20 bits
        assert_eq!(bits, 20);
    }

    /// Verifies that textNumberRoundingMode="roundUnnecessary" errors when rounding is needed (§13.7.1.4).
    #[test]
    fn test_unparse_number_rounding_unnecessary() {
        let props = ResolvedProperties {
            text_number_pattern: Some(String::from("##.##;")),
            text_number_rounding_mode: crate::schema::ir::TextNumberRoundingMode::RoundUnnecessary,
            ..ResolvedProperties::default()
        };
        // 0.125 needs rounding to fit 2 fractional digits -> error
        let err = validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.125")), &props).unwrap_err();
        assert!(err.message.as_str().contains("rounding is required"));
        assert!(err.message.as_str().contains("roundUnnecessary"));

        // 0.12 needs no rounding -> ok
        assert!(validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.12")), &props).is_ok());
        // 0.1200 trailing zeros -> ok
        assert!(validate_text_number_rounding(&DfdlValue::Decimal(String::from("0.1200")), &props).is_ok());
    }

    /// Verifies root element namespace validation during unparsing.
    #[test]
    fn test_unparse_root_element_namespace_mismatch() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace(
                "http://example.com",
                "root",
                None,
            ),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(10_000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Document has unqualified root {}root instead of {http://example.com}root
        let unqual_elem = crate::infoset::tree::InfosetElement::simple(
            crate::types::QName::local("root"),
            crate::infoset::state::ElementState::Value(DfdlValue::String(String::from("ok"))),
        );
        let doc = crate::infoset::InfosetDocument::with_root(unqual_elem);
        let err = unparser.unparse_document(&doc).unwrap_err();
        assert!(err.message.as_str().contains("expected element start '{http://example.com}root'"));
        assert!(err.message.as_str().contains("received '{}root'"));
    }

    /// Verifies binary decimal unparsing, virtual scale point, and minimum length bounds (§13.7).
    #[test]
    fn test_unparse_binary_decimal_signedness_and_bounds() {
        let mut builder = crate::schema::builder::SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Decimal),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                root_elem.name.clone(),
                TermKind::Element(root_elem),
                ResolvedProperties::default(),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        // 1. Unsigned binary decimal with 0 bits length -> error
        let mut writer1 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget1 = WorkBudget::new(1000);
        let mut unparser1 = UnparserEngine::new(&schema, &mut writer1, &mut budget1);
        let props_u = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: false,
            length: Some(0),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..ResolvedProperties::default()
        };
        let err1 = unparser1.unparse_simple_value(&DfdlValue::Decimal(String::from("1")), &props_u).unwrap_err();
        assert!(err1.message.as_str().contains("unsigned binary number: minimum length is 1 bit(s), but 0 out of range"));

        // 2. Signed binary decimal with 1 bit when disallow_signed_integer_length_1bit is true -> error
        let mut schema_disallow = schema.clone();
        schema_disallow.disallow_signed_integer_length_1bit = true;
        let mut writer2 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget2 = WorkBudget::new(1000);
        let mut unparser2 = UnparserEngine::new(&schema_disallow, &mut writer2, &mut budget2);
        let props_s = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: true,
            length: Some(1),
            length_units: crate::schema::ir::LengthUnits::Bits,
            ..ResolvedProperties::default()
        };
        let err2 = unparser2.unparse_simple_value(&DfdlValue::Decimal(String::from("1")), &props_s).unwrap_err();
        assert!(err2.message.as_str().contains("signed binary number: minimum length is 2 bit(s), but 1 out of range"));

        // 3. Binary decimal unparsing with virtual decimal point
        let mut writer3 = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget3 = WorkBudget::new(1000);
        let mut unparser3 = UnparserEngine::new(&schema, &mut writer3, &mut budget3);
        let props_scale = ResolvedProperties {
            representation: Representation::Binary,
            binary_number_rep: crate::schema::ir::BinaryNumberRep::Binary,
            decimal_signed: false,
            length: Some(16),
            length_units: crate::schema::ir::LengthUnits::Bits,
            binary_decimal_virtual_point: 2,
            ..ResolvedProperties::default()
        };
        // "12.34" * 10^2 = 1234 (0x04D2)
        unparser3.unparse_simple_value(&DfdlValue::Decimal(String::from("12.34")), &props_scale).unwrap();
        writer3.flush().unwrap();
        assert_eq!(writer3.into_sink().into_vec().as_slice(), &[0x04, 0xD2]);
    }

    #[test]
    fn test_unparser_unexpected_child_element_clark_notation() {
        use crate::infoset::state::ElementState;
        use crate::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
        use crate::schema::ir::CompiledSequence;

        let mut builder = SchemaBuilder::new();
        let child_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace("http://example.com", "expected_child", None),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let child_props = ResolvedProperties {
            representation: Representation::Text,
            length_kind: LengthKind::Explicit,
            length: Some(4),
            length_units: crate::schema::ir::LengthUnits::Characters,
            ..Default::default()
        };
        let child_id = builder
            .add_term_with_props(
                crate::types::QName::with_namespace("http://example.com", "expected_child", None),
                TermKind::Element(child_elem),
                child_props,
            )
            .unwrap();

        let seq_id = builder
            .add_term(
                crate::types::QName::local("sequence"),
                TermKind::Sequence(CompiledSequence {
                    members: alloc::vec![child_id],
                }),
            )
            .unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::with_namespace("http://example.com", "root", None),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(
                crate::types::QName::with_namespace("http://example.com", "root", None),
                TermKind::Element(root_elem),
            )
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut root_infoset = InfosetElement::complex(crate::types::QName::with_namespace("http://example.com", "root", None));
        let expected_child = InfosetElement::simple(
            crate::types::QName::with_namespace("http://example.com", "expected_child", None),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("data"))),
        );
        let unexpected_child = InfosetElement::simple(
            crate::types::QName::local("extra"),
            ElementState::Value(DfdlValue::String(alloc::string::String::from("boom"))),
        );
        root_infoset.children.push(InfosetNode::Element(expected_child));
        root_infoset.children.push(InfosetNode::Element(unexpected_child));
        let doc = InfosetDocument::with_root(root_infoset);

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        let err = unparser.unparse_document(&doc).unwrap_err();
        let err_msg = err.message.as_str();
        assert!(err_msg.contains("expected element end for '{http://example.com}root'"), "msg: {}", err_msg);
        assert!(err_msg.contains("received element start '{}extra'"), "msg: {}", err_msg);
    }

    /// Verifies escape scheme unparsing for both EscapeCharacter and EscapeBlock.
    #[test]
    fn test_escape_scheme_unparsing() {
        use crate::schema::ir::{CompiledEscapeScheme, EscapeKind, GenerateEscapeBlock};

        let mut builder = SchemaBuilder::new();
        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::String),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term(crate::types::QName::local("root"), TermKind::Element(root_elem))
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();
        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);
        unparser.active_delimiters.push(alloc::string::String::from(","));

        // EscapeCharacter scheme with '/' as escape char and '//' escaping itself
        let char_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeCharacter,
            escape_character: Some(alloc::string::String::from("/")),
            escape_escape_character: Some(alloc::string::String::from("/")),
            escape_block_start: None,
            escape_block_end: None,
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };

        assert_eq!(unparser.apply_escape_scheme("plain", &char_scheme), "plain");
        assert_eq!(unparser.apply_escape_scheme("a,b", &char_scheme), "a/,b");
        assert_eq!(unparser.apply_escape_scheme("a/b", &char_scheme), "a//b");
        assert_eq!(unparser.apply_escape_scheme("a/,b", &char_scheme), "a///,b");

        // EscapeBlock scheme with quote delimiters and '#' as escape char
        let block_scheme = CompiledEscapeScheme {
            escape_kind: EscapeKind::EscapeBlock,
            escape_character: None,
            escape_escape_character: Some(alloc::string::String::from("#")),
            escape_block_start: Some(alloc::string::String::from("'")),
            escape_block_end: Some(alloc::string::String::from("'")),
            extra_escaped_characters: alloc::vec![],
            generate_escape_block: GenerateEscapeBlock::WhenNeeded,
        };

        assert_eq!(unparser.apply_escape_scheme("plain", &block_scheme), "plain");
        assert_eq!(unparser.apply_escape_scheme("a,b", &block_scheme), "'a,b'");
        assert_eq!(unparser.apply_escape_scheme("a'b", &block_scheme), "'a#'b'");
    }

    #[test]
    fn test_unparse_lsbf_little_endian_float() {
        let mut builder = SchemaBuilder::new();
        let elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("f"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Float),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let props = ResolvedProperties {
            byte_order: ByteOrder::LittleEndian,
            bit_order: BitOrder::LeastSignificantBitFirst,
            representation: crate::schema::ir::Representation::Binary,
            length_kind: crate::schema::ir::LengthKind::Implicit,
            ..Default::default()
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("f"), TermKind::Element(elem), props)
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::LeastSignificantBitFirst, ByteOrder::LittleEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // 1.25f32 has IEEE 754 bits 0x3fa00000. LittleEndian byte representation is [0x00, 0x00, 0xa0, 0x3f].
        let elem_info = InfosetElement::simple(crate::types::QName::local("f"), ElementState::Value(DfdlValue::Float(1.25)));
        unparser.unparse_element(root_id, &elem_info).unwrap();
        writer.flush().unwrap();

        assert_eq!(writer.into_sink().into_vec(), alloc::vec![0x00, 0x00, 0xa0, 0x3f]);
    }

    #[test]
    fn test_hidden_group_unparse_preserves_infoset_elements() {
        let mut builder = SchemaBuilder::new();
        // Hidden element with OVC "{ 1 }"
        let hidden_props = ResolvedProperties {
            is_hidden: true,
            output_value_calc: Some(alloc::string::String::from("1")),
            ..Default::default()
        };
        let hidden_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let hidden_id = builder
            .add_term_with_props(crate::types::QName::local("a"), TermKind::Element(hidden_elem), hidden_props)
            .unwrap();

        // Regular element
        let regular_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("a"),
            type_ir: crate::schema::ir::CompiledType::Simple(crate::infoset::DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let regular_id = builder
            .add_term_with_props(crate::types::QName::local("a"), TermKind::Element(regular_elem), ResolvedProperties::default())
            .unwrap();

        let seq = crate::schema::ir::CompiledSequence {
            members: alloc::vec![hidden_id, regular_id],
        };
        let seq_id = builder
            .add_term_with_props(crate::types::QName::local("seq"), TermKind::Sequence(seq), ResolvedProperties::default())
            .unwrap();

        let root_elem = crate::schema::ir::CompiledElement {
            name: crate::types::QName::local("root"),
            type_ir: crate::schema::ir::CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(crate::types::QName::local("root"), TermKind::Element(root_elem), ResolvedProperties::default())
            .unwrap();
        builder.set_root(root_id);
        let schema = builder.build().unwrap();

        let mut writer = BitWriter::new(VecByteSink::new(), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut budget = WorkBudget::new(1000);
        let mut unparser = UnparserEngine::new(&schema, &mut writer, &mut budget);

        // Infoset has ONLY ONE child <a> with value 10 (the regular element)
        let mut root_info = InfosetElement::complex(crate::types::QName::local("root"));
        root_info.children.push(InfosetNode::Element(InfosetElement::simple(
            crate::types::QName::local("a"),
            ElementState::Value(DfdlValue::Int(10)),
        )));

        // Unparse root: hidden <a> must be synthesized with value 1, and regular <a> must consume the infoset's value 10
        unparser.unparse_element(root_id, &root_info).unwrap();
        writer.flush().unwrap();

        let bytes = writer.into_sink().into_vec();
        let text = core::str::from_utf8(&bytes).unwrap();
        assert_eq!(text, "110");
    }
}

