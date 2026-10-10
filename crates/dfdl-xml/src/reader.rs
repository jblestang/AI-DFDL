//! Panic-free, streaming XML tokenizer and namespace resolver powered by `xmlparser`.
//!
//! Implements XML 1.0 and Namespaces in XML 1.0 specs under `#![no_std]` + `alloc`.

#![allow(clippy::arithmetic_side_effects)]

extern crate alloc;
use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;
use core::str;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::types::{QName, SourceLocation};
use dfdl_core::util::{get_checked, try_push};
use xmlparser::{ElementEnd, Token, Tokenizer};

use crate::event::{Attribute, XmlEvent};
use crate::limits::XmlReaderLimits;

fn format_num_commas(n: usize) -> String {
    let s = alloc::format!("{}", n);
    let mut out = String::new();
    let bytes = s.as_bytes();
    let rem = bytes.len() % 3;
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (i % 3 == rem || (rem == 0 && i % 3 == 0)) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// Stack frame maintaining namespace prefix bindings for an element scope.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct NamespaceFrame {
    /// Default namespace URI declared in this scope (xmlns="...").
    default_ns: Option<String>,
    /// Prefixed namespace bindings declared in this scope (xmlns:prefix="...").
    bindings: Vec<(String, String)>,
}

/// Pending start element building state during attribute tokenization.
#[derive(Debug)]
struct StartElementState<'a> {
    raw_prefix: Option<&'a str>,
    raw_local: &'a str,
    raw_attrs: Vec<(Option<&'a str>, &'a str, Cow<'a, str>, SourceLocation)>,
    frame: NamespaceFrame,
    location: SourceLocation,
}

/// Streaming panic-free XML reader backed by `xmlparser`.
#[derive(Debug)]
pub struct XmlReader<'a> {
    tokenizer: Tokenizer<'a>,
    limits: XmlReaderLimits,
    ns_stack: Vec<NamespaceFrame>,
    initial_bindings: Vec<(String, String)>,
    tag_stack: Vec<QName>,
    pending_start: Option<StartElementState<'a>>,
    pending_events: Vec<(XmlEvent<'a>, bool)>,
}

impl<'a> XmlReader<'a> {
    /// Constructs a new [`XmlReader`] with default limits.
    #[inline]
    #[must_use]
    pub fn new(input: &'a str) -> Self {
        Self::with_limits(input, XmlReaderLimits::default())
    }

    /// Constructs a new [`XmlReader`] with custom resource limits.
    #[inline]
    #[must_use]
    pub fn with_limits(mut input: &'a str, limits: XmlReaderLimits) -> Self {
        // Strip UTF-8 BOM if present
        if input.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]) {
            if let Some(rest) = input.get(3..) {
                input = rest;
            }
        }

        Self {
            tokenizer: Tokenizer::from(input),
            limits,
            ns_stack: Vec::new(),
            initial_bindings: Vec::new(),
            tag_stack: Vec::new(),
            pending_start: None,
            pending_events: Vec::new(),
        }
    }

    /// Registers an outer/ambient namespace prefix binding (e.g. from an enclosing test suite).
    pub fn add_namespace_binding(&mut self, prefix: &str, uri: &str) {
        self.initial_bindings
            .push((alloc::string::ToString::to_string(prefix), alloc::string::ToString::to_string(uri)));
    }

    /// Sets whether namespace prefix resolution is permissive (fallback to prefix string if undeclared).
    pub fn set_permissive_namespaces(&mut self, permissive: bool) {
        self.limits.strict_namespaces = !permissive;
    }

    /// Pushes back an unconsumed [`XmlEvent`] to be returned by the next call to [`next_event`](Self::next_event).
    pub fn push_back(&mut self, event: XmlEvent<'a>) {
        self.pending_events.push((event, false));
    }

    /// Resolves namespace URI for a given prefix string in the active scope.
    #[inline]
    #[must_use]
    pub fn resolve_prefix(&self, prefix: &str) -> Option<&str> {
        if prefix == "xml" {
            return Some("http://www.w3.org/XML/1998/namespace");
        }
        if prefix == "xmlns" {
            return Some("http://www.w3.org/2000/xmlns/");
        }

        for frame in self.ns_stack.iter().rev() {
            for (p, uri) in &frame.bindings {
                if p == prefix {
                    return Some(uri.as_str());
                }
            }
        }
        for (p, uri) in &self.initial_bindings {
            if p == prefix {
                return Some(uri.as_str());
            }
        }
        None
    }

    /// Resolves default namespace URI in active scope.
    #[inline]
    #[must_use]
    pub fn resolve_default_ns(&self) -> Option<&str> {
        for frame in self.ns_stack.iter().rev() {
            if let Some(ref uri) = frame.default_ns {
                return if uri.is_empty() {
                    None
                } else {
                    Some(uri.as_str())
                };
            }
        }
        None
    }

    /// Returns all prefixes currently mapped to the given namespace URI.
    #[must_use]
    pub fn find_prefixes_for_uri(&self, uri: &str) -> Vec<String> {
        let mut res = Vec::new();
        for frame in &self.ns_stack {
            for (p, u) in &frame.bindings {
                if u == uri && !res.contains(p) {
                    res.push(p.clone());
                }
            }
            if let Some(ref def_uri) = frame.default_ns {
                if def_uri == uri && !res.iter().any(|p| p.is_empty()) {
                    res.push(String::new());
                }
            }
        }
        for (p, u) in &self.initial_bindings {
            if u == uri && !res.contains(p) {
                res.push(p.clone());
            }
        }
        res
    }

    /// Returns all prefix-to-URI bindings currently in active scope.
    #[must_use]
    pub fn in_scope_namespace_bindings(&self) -> Vec<(String, String)> {
        let mut res = Vec::new();
        if let Some(def_ns) = self.resolve_default_ns() {
            res.push((String::new(), String::from(def_ns)));
        }
        for frame in self.ns_stack.iter().rev() {
            for (p, u) in &frame.bindings {
                if !res.iter().any(|(existing_p, _)| existing_p == p) {
                    res.push((p.clone(), u.clone()));
                }
            }
        }
        for (p, u) in &self.initial_bindings {
            if !res.iter().any(|(existing_p, _)| existing_p == p) {
                res.push((p.clone(), u.clone()));
            }
        }
        res
    }

    /// Returns prefix-to-URI bindings declared explicitly on the current element.
    ///
    /// This reflects only the XML namespace attributes (`xmlns` and `xmlns:prefix`)
    /// defined directly on the topmost element of the current reader scope, without
    /// outer ancestor namespaces.
    #[must_use]
    pub fn current_element_namespace_bindings(&self) -> Vec<(String, String)> {
        let mut res = Vec::new();
        if let Some(frame) = self.ns_stack.last() {
            if let Some(ref def_uri) = frame.default_ns {
                let _ = try_push(&mut res, (String::new(), def_uri.clone()));
            }
            for (p, u) in &frame.bindings {
                let _ = try_push(&mut res, (p.clone(), u.clone()));
            }
        }
        res
    }

    /// Decodes XML entities in character or attribute content.
    fn decode_entities(&self, text: &'a str) -> DFDLResult<Cow<'a, str>> {
        if !text.contains('&') {
            return Ok(Cow::Borrowed(text));
        }

        let mut out = String::new();
        let bytes = text.as_bytes();
        let mut idx = 0;

        while idx < bytes.len() {
            let b = *get_checked(bytes, idx)?;
            if b == b'&' {
                let remaining = bytes.get(idx..).unwrap_or_default();

                let semi_pos = remaining.iter().position(|&c| c == b';').ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unterminated entity reference")
                })?;

                let entity_end = idx.saturating_add(semi_pos);
                let entity_start = idx.saturating_add(1);

                let entity_str = text
                    .get(entity_start..entity_end)
                    .ok_or_else(|| DFDLError::new(DFDLErrorKind::Parse, "Invalid entity slice"))?;

                match entity_str {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ if entity_str.starts_with('#') => {
                        let code_str = entity_str.get(1..).unwrap_or("");

                        let ch_code = if code_str.starts_with('x') || code_str.starts_with('X') {
                            u32::from_str_radix(code_str.get(1..).unwrap_or(""), 16).map_err(
                                |_| DFDLError::new(DFDLErrorKind::Parse, "Invalid hex entity code"),
                            )?
                        } else {
                            code_str.parse::<u32>().map_err(|_| {
                                DFDLError::new(DFDLErrorKind::Parse, "Invalid decimal entity code")
                            })?
                        };

                        let ch = char::from_u32(ch_code).ok_or_else(|| {
                            DFDLError::new(
                                DFDLErrorKind::Parse,
                                "Numeric entity code points to invalid Unicode scalar",
                            )
                        })?;
                        out.push(ch);
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Unsupported or custom DTD entity reference prohibited",
                        ));
                    }
                }

                idx = entity_end.saturating_add(1);
            } else {
                let end_idx = idx.saturating_add(1);
                let ch = text
                    .get(idx..end_idx)
                    .ok_or_else(|| DFDLError::new(DFDLErrorKind::Parse, "Invalid char slice"))?;
                out.push_str(ch);
                idx = end_idx;
            }
        }

        Ok(Cow::Owned(out))
    }

    /// Decodes character and predefined entity references in an attribute value and
    /// normalizes literal whitespace characters (#x9, #xA, #xD) to space (#x20)
    /// in accordance with W3C XML 1.0 §3.3.3.
    fn decode_attribute_value(&self, text: &'a str) -> DFDLResult<Cow<'a, str>> {
        if !text.contains('&') && !text.contains(['\t', '\r', '\n']) {
            return Ok(Cow::Borrowed(text));
        }

        let mut out = String::new();
        let mut idx = 0;

        while idx < text.len() {
            if let Some(amp_rel) = text.get(idx..).and_then(|s| s.find('&')) {
                let literal_end = idx.saturating_add(amp_rel);
                if let Some(prefix_slice) = text.get(idx..literal_end) {
                    for ch in prefix_slice.chars() {
                        if ch == '\t' || ch == '\r' || ch == '\n' {
                            out.push(' ');
                        } else {
                            out.push(ch);
                        }
                    }
                }

                let entity_start = literal_end.saturating_add(1);
                let semi_rel = text
                    .get(entity_start..)
                    .and_then(|s| s.find(';'))
                    .ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Unterminated XML entity reference in attribute value",
                        )
                    })?;
                let entity_end = entity_start.saturating_add(semi_rel);

                let entity_str = text
                    .get(entity_start..entity_end)
                    .ok_or_else(|| DFDLError::new(DFDLErrorKind::Parse, "Invalid entity slice"))?;

                match entity_str {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ if entity_str.starts_with('#') => {
                        let code_str = entity_str.get(1..).unwrap_or("");

                        let ch_code = if code_str.starts_with('x') || code_str.starts_with('X') {
                            u32::from_str_radix(code_str.get(1..).unwrap_or(""), 16).map_err(
                                |_| DFDLError::new(DFDLErrorKind::Parse, "Invalid hex entity code"),
                            )?
                        } else {
                            code_str.parse::<u32>().map_err(|_| {
                                DFDLError::new(DFDLErrorKind::Parse, "Invalid decimal entity code")
                            })?
                        };

                        let ch = char::from_u32(ch_code).ok_or_else(|| {
                            DFDLError::new(
                                DFDLErrorKind::Parse,
                                "Numeric entity code points to invalid Unicode scalar",
                            )
                        })?;
                        // Character references are NOT normalized to space per XML 1.0 §3.3.3
                        out.push(ch);
                    }
                    _ => {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Unsupported or custom DTD entity reference prohibited",
                        ));
                    }
                }

                idx = entity_end.saturating_add(1);
            } else {
                if let Some(remaining) = text.get(idx..) {
                    for ch in remaining.chars() {
                        if ch == '\t' || ch == '\r' || ch == '\n' {
                            out.push(' ');
                        } else {
                            out.push(ch);
                        }
                    }
                }
                break;
            }
        }

        Ok(Cow::Owned(out))
    }

    /// Resolves raw prefix and local name to a [`QName`].
    fn resolve_qname(&self, prefix: Option<&str>, local: &str) -> DFDLResult<QName> {
        if let Some(pref) = prefix {
            let uri = match self.resolve_prefix(pref) {
                Some(u) => u,
                None => {
                    if self.limits.strict_namespaces {
                        let msg =
                            alloc::format!("Undeclared XML namespace prefix reference: '{}'", pref);
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg));
                    } else {
                        pref
                    }
                }
            };
            Ok(QName::with_namespace(uri, local, Some(pref)))
        } else {
            let default_ns = self.resolve_default_ns();
            match default_ns {
                Some(ns) => Ok(QName::with_namespace(ns, local, None)),
                None => Ok(QName::local(local)),
            }
        }
    }

    /// Finalizes building pending start element tag.
    fn finalize_start_element(
        &mut self,
        is_self_closing: bool,
    ) -> DFDLResult<(XmlEvent<'a>, Option<XmlEvent<'a>>)> {
        let state = self.pending_start.take().ok_or_else(|| {
            DFDLError::new(DFDLErrorKind::InternalInvariant, "No pending start element")
        })?;

        try_push(&mut self.ns_stack, state.frame)?;

        let elem_qname = self.resolve_qname(state.raw_prefix, state.raw_local)?;
        let mut attributes = Vec::new();

        if !self.limits.check_attribute_count(state.raw_attrs.len()) {
            return Err(DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "Attribute count limit exceeded",
            )
            .with_location(state.location));
        }

        for (aprefix, alocal, aval, aloc) in state.raw_attrs {
            let aqname = if let Some(pref) = aprefix {
                let auri = self.resolve_prefix(pref).ok_or_else(|| {
                    DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Undeclared XML attribute namespace prefix reference",
                    )
                    .with_location(aloc)
                })?;
                QName::with_namespace(auri, alocal, Some(pref))
            } else {
                QName::local(alocal)
            };

            if attributes.iter().any(|a: &Attribute| a.name == aqname) {
                return Err(DFDLError::new(
                    DFDLErrorKind::Parse,
                    "Duplicate attribute declared on element",
                )
                .with_location(aloc));
            }

            try_push(
                &mut attributes,
                Attribute {
                    name: aqname,
                    value: aval,
                    location: aloc,
                },
            )?;
        }

        let end_event = if is_self_closing {
            Some(XmlEvent::EndElement {
                name: elem_qname.clone(),
                location: state.location,
            })
        } else {
            try_push(&mut self.tag_stack, elem_qname.clone())?;
            None
        };

        Ok((
            XmlEvent::StartElement {
                name: elem_qname,
                attributes,
                location: state.location,
            },
            end_event,
        ))
    }

    /// Reads next streaming XML event.
    pub fn next_event(&mut self) -> DFDLResult<Option<XmlEvent<'a>>> {
        if let Some((ev, pop_ns)) = self.pending_events.pop() {
            if pop_ns {
                self.ns_stack.pop();
            }
            return Ok(Some(ev));
        }

        loop {
            let next_token = match self.tokenizer.next() {
                Some(res) => res.map_err(|e| {
                    DFDLError::new(DFDLErrorKind::Parse, "XML tokenization error").with_location(
                        SourceLocation::at_offset(0)
                            .with_line_col(e.pos().row as usize, e.pos().col as usize),
                    )
                })?,
                None => {
                    if self.pending_start.is_some() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "XML document ended inside unclosed start tag",
                        ));
                    }
                    if !self.tag_stack.is_empty() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "XML document ended with unclosed tags",
                        ));
                    }
                    return Ok(None);
                }
            };

            match next_token {
                Token::Declaration { encoding, span, .. } => {
                    let loc = SourceLocation::at_offset(span.start());
                    let enc_str = encoding.map(|s| s.as_str()).unwrap_or("UTF-8");
                    return Ok(Some(XmlEvent::StartDocument {
                        encoding: if enc_str.is_empty() { "UTF-8" } else { enc_str },
                        location: loc,
                    }));
                }
                Token::ElementStart {
                    prefix,
                    local,
                    span,
                } => {
                    let loc = SourceLocation::at_offset(span.start());

                    let max_name_len = prefix.as_str().len().max(local.as_str().len());
                    if max_name_len > self.limits.max_token_length {
                        let msg = alloc::format!(
                            "XML parse error: length of entity {} exceeds maximum limit {}",
                            format_num_commas(max_name_len),
                            format_num_commas(self.limits.max_token_length)
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg).with_location(loc));
                    }

                    let current_depth = self.ns_stack.len().saturating_add(1);

                    if !self.limits.check_depth(current_depth) {
                        return Err(DFDLError::new(
                            DFDLErrorKind::ImplementationLimit,
                            "XML element nesting depth limit exceeded",
                        )
                        .with_location(loc));
                    }

                    self.pending_start = Some(StartElementState {
                        raw_prefix: if prefix.as_str().is_empty() {
                            None
                        } else {
                            Some(prefix.as_str())
                        },
                        raw_local: local.as_str(),
                        raw_attrs: Vec::new(),
                        frame: NamespaceFrame::default(),
                        location: loc,
                    });
                }
                Token::Attribute {
                    prefix,
                    local,
                    value,
                    span,
                } => {
                    let loc = SourceLocation::at_offset(span.start());

                    let max_attr_len = prefix.as_str().len().max(local.as_str().len());
                    if max_attr_len > self.limits.max_token_length {
                        let msg = alloc::format!(
                            "XML parse error: length of entity {} exceeds maximum limit {}",
                            format_num_commas(max_attr_len),
                            format_num_commas(self.limits.max_token_length)
                        );
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &msg).with_location(loc));
                    }

                    let decoded = self.decode_attribute_value(value.as_str())?;

                    let state = match self.pending_start.as_mut() {
                        Some(s) => s,
                        None => continue,
                    };

                    let p_str = prefix.as_str();
                    let l_str = local.as_str();

                    if (p_str == "xmlns" && l_str == "xmlns")
                        || decoded.as_ref() == "http://www.w3.org/2000/xmlns/"
                    {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            "Schema Definition Error: The prefix \"xmlns\" cannot be bound to any namespace explicitly; neither can the namespace for \"xmlns\" be bound to any prefix explicitly",
                        ).with_location(loc));
                    }

                    if p_str.is_empty() && l_str == "xmlns" {
                        state.frame.default_ns = Some(String::from(decoded.as_ref()));
                    } else if p_str == "xmlns" {
                        try_push(
                            &mut state.frame.bindings,
                            (String::from(l_str), String::from(decoded.as_ref())),
                        )?;
                    } else {
                        let pref_opt = if p_str.is_empty() { None } else { Some(p_str) };
                        try_push(&mut state.raw_attrs, (pref_opt, l_str, decoded, loc))?;
                    }
                }
                Token::ElementEnd { end, span } => {
                    match end {
                        ElementEnd::Open => {
                            // Finish tag header, stay open
                            let (start_ev, _) = self.finalize_start_element(false)?;
                            return Ok(Some(start_ev));
                        }
                        ElementEnd::Empty => {
                            // Self-closing element `<foo/>`
                            let (start_ev, end_ev_opt) = self.finalize_start_element(true)?;
                            if let Some(end_ev) = end_ev_opt {
                                try_push(&mut self.pending_events, (end_ev, true))?;
                            }
                            return Ok(Some(start_ev));
                        }
                        ElementEnd::Close(prefix, local) => {
                            let loc = SourceLocation::at_offset(span.start());
                            let p_opt = if prefix.as_str().is_empty() {
                                None
                            } else {
                                Some(prefix.as_str())
                            };

                            let end_qname = self.resolve_qname(p_opt, local.as_str())?;
                            let expected = self.tag_stack.pop().ok_or_else(|| {
                                DFDLError::new(DFDLErrorKind::Parse, "Unexpected end element tag")
                                    .with_location(loc)
                            })?;

                            if expected != end_qname {
                                return Err(DFDLError::new(
                                    DFDLErrorKind::Parse,
                                    "Mismatched XML end tag",
                                )
                                .with_location(loc));
                            }

                            self.ns_stack.pop();

                            return Ok(Some(XmlEvent::EndElement {
                                name: end_qname,
                                location: loc,
                            }));
                        }
                    }
                }
                Token::Text { text } => {
                    let loc = SourceLocation::at_offset(text.start());
                    let decoded = self.decode_entities(text.as_str())?;
                    return Ok(Some(XmlEvent::Text {
                        content: decoded,
                        location: loc,
                    }));
                }
                Token::Cdata { text, span } => {
                    let loc = SourceLocation::at_offset(span.start());
                    return Ok(Some(XmlEvent::CData {
                        content: text.as_str(),
                        location: loc,
                    }));
                }
                Token::Comment { text, span } => {
                    let loc = SourceLocation::at_offset(span.start());
                    return Ok(Some(XmlEvent::Comment {
                        content: text.as_str(),
                        location: loc,
                    }));
                }
                Token::ProcessingInstruction {
                    target,
                    content,
                    span,
                } => {
                    let loc = SourceLocation::at_offset(span.start());
                    return Ok(Some(XmlEvent::ProcessingInstruction {
                        target: target.as_str(),
                        content: content.map(|c| c.as_str()),
                        location: loc,
                    }));
                }
                Token::DtdStart { span, .. } | Token::EntityDeclaration { span, .. } => {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "DTD declarations (<!DOCTYPE) are prohibited for security (anti-XXE)",
                    )
                    .with_location(SourceLocation::at_offset(span.start())));
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::limits::XmlReaderLimits;

    #[test]
    fn test_xml_reader_start_end_element() {
        let xml = "<root xmlns=\"http://example.com\"><item id=\"1\">Hello</item></root>";
        let mut reader = XmlReader::new(xml);

        let ev1 = reader.next_event().unwrap().unwrap();
        assert!(matches!(
            ev1,
            XmlEvent::StartElement { ref name, ref attributes, .. }
                if name.local_name == "root" && name.namespace.as_ref().map(|ns| ns.as_str()) == Some("http://example.com") && attributes.is_empty()
        ));

        let ev2 = reader.next_event().unwrap().unwrap();
        assert!(matches!(
            ev2,
            XmlEvent::StartElement { ref name, ref attributes, .. }
                if name.local_name == "item" && attributes.len() == 1 && attributes[0].name.local_name == "id" && attributes[0].value == "1"
        ));

        let ev3 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev3, XmlEvent::Text { ref content, .. } if content == "Hello"));
    }

    #[test]
    fn test_prohibit_dtd_declarations() {
        let xml = "<!DOCTYPE root [<!ENTITY xxe SYSTEM \"http://attacker.com\">]><root/>";
        let mut reader = XmlReader::new(xml);
        assert!(reader.next_event().is_err());
    }

    #[test]
    fn test_xml_reader_self_closing_tag_symmetry() {
        let xml = "<root><empty id=\"1\"/><item>value</item></root>";
        let mut reader = XmlReader::new(xml);

        let ev1 = reader.next_event().unwrap().unwrap();
        assert!(
            matches!(ev1, XmlEvent::StartElement { ref name, .. } if name.local_name == "root")
        );

        let ev2 = reader.next_event().unwrap().unwrap();
        assert!(
            matches!(ev2, XmlEvent::StartElement { ref name, .. } if name.local_name == "empty")
        );

        let ev3 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev3, XmlEvent::EndElement { ref name, .. } if name.local_name == "empty"));

        let ev4 = reader.next_event().unwrap().unwrap();
        assert!(
            matches!(ev4, XmlEvent::StartElement { ref name, .. } if name.local_name == "item")
        );

        let ev5 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev5, XmlEvent::Text { ref content, .. } if content == "value"));

        let ev6 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev6, XmlEvent::EndElement { ref name, .. } if name.local_name == "item"));

        let ev7 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev7, XmlEvent::EndElement { ref name, .. } if name.local_name == "root"));

        assert_eq!(reader.next_event().unwrap(), None);
    }

    #[test]
    fn test_xml_reader_in_scope_namespace_bindings() {
        let xml = "<root xmlns=\"http://default.com\" xmlns:ns1=\"http://ns1.com\"><child xmlns:ns2=\"http://ns2.com\">value</child></root>";
        let mut reader = XmlReader::new(xml);
        assert_eq!(reader.resolve_default_ns(), None);

        // Read <root>
        let ev1 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev1, XmlEvent::StartElement { .. }));
        assert_eq!(reader.resolve_default_ns(), Some("http://default.com"));

        let bindings = reader.in_scope_namespace_bindings();
        assert!(bindings.iter().any(|(p, u)| p == "ns1" && u == "http://ns1.com"));

        // Read <child>
        let ev2 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev2, XmlEvent::StartElement { .. }));
        let child_bindings = reader.in_scope_namespace_bindings();
        assert!(child_bindings.iter().any(|(p, u)| p == "ns1" && u == "http://ns1.com"));
        assert!(child_bindings.iter().any(|(p, u)| p == "ns2" && u == "http://ns2.com"));
    }

    #[test]
    fn test_xml_reader_current_element_namespace_bindings() {
        let xml = "<root xmlns=\"http://default.com\" xmlns:ns1=\"http://ns1.com\"><child xmlns:ns2=\"http://ns2.com\"><grandchild>val</grandchild></child></root>";
        let mut reader = XmlReader::new(xml);

        // Read <root>
        let ev1 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev1, XmlEvent::StartElement { .. }));
        let root_local = reader.current_element_namespace_bindings();
        assert!(root_local.iter().any(|(p, u)| p.is_empty() && u == "http://default.com"));
        assert!(root_local.iter().any(|(p, u)| p == "ns1" && u == "http://ns1.com"));
        assert!(!root_local.iter().any(|(p, _)| p == "ns2"));

        // Read <child>
        let ev2 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev2, XmlEvent::StartElement { .. }));
        let child_local = reader.current_element_namespace_bindings();
        assert!(child_local.iter().any(|(p, u)| p == "ns2" && u == "http://ns2.com"));
        // Parent ns1 is in scope but NOT declared on child
        assert!(!child_local.iter().any(|(p, _)| p == "ns1"));

        // Read <grandchild>
        let ev3 = reader.next_event().unwrap().unwrap();
        assert!(matches!(ev3, XmlEvent::StartElement { .. }));
        let grandchild_local = reader.current_element_namespace_bindings();
        assert!(grandchild_local.is_empty());
        // All ancestors are still in scope
        let grandchild_in_scope = reader.in_scope_namespace_bindings();
        assert!(grandchild_in_scope.iter().any(|(p, u)| p == "ns1" && u == "http://ns1.com"));
        assert!(grandchild_in_scope.iter().any(|(p, u)| p == "ns2" && u == "http://ns2.com"));
    }

    /// Verifies CDATA, comments, processing instructions, entity decoding, limits, and SDE namespace errors.
    #[test]
    fn test_xml_reader_comprehensive_coverage() {
        // CDATA, Comment, Processing Instruction
        let xml_special = "<?target content?><!-- my comment --><root><![CDATA[raw & unescaped <data>]]></root>";
        let mut r = XmlReader::new(xml_special);
        let pi = r.next_event().unwrap().unwrap();
        assert!(matches!(pi, XmlEvent::ProcessingInstruction { target: "target", content: Some("content"), .. }));
        let comment = r.next_event().unwrap().unwrap();
        assert!(matches!(comment, XmlEvent::Comment { content: " my comment ", .. }));
        let start = r.next_event().unwrap().unwrap();
        assert!(matches!(start, XmlEvent::StartElement { .. }));
        let cdata = r.next_event().unwrap().unwrap();
        assert!(matches!(cdata, XmlEvent::CData { content: "raw & unescaped <data>", .. }));
        let end = r.next_event().unwrap().unwrap();
        assert!(matches!(end, XmlEvent::EndElement { .. }));

        // Entity decoding: standard, hex, decimal
        let xml_entities = "<root val=\"&quot;&apos;&lt;&gt;&amp;&#65;&#x42;\">&lt;text&gt;</root>";
        let mut r_ent = XmlReader::new(xml_entities);
        let ev = r_ent.next_event().unwrap().unwrap();
        assert!(matches!(ev, XmlEvent::StartElement { attributes, .. } if attributes[0].value == "\"'<>&AB"));
        let txt = r_ent.next_event().unwrap().unwrap();
        assert!(matches!(txt, XmlEvent::Text { content, .. } if content == "<text>"));

        // Unknown entity returns error
        let mut r_bad_ent = XmlReader::new("<root>&unknown;</root>");
        let _ = r_bad_ent.next_event();
        assert!(r_bad_ent.next_event().is_err());

        // Invalid hex entity
        let mut r_bad_num = XmlReader::new("<root>&#xZZ;</root>");
        let _ = r_bad_num.next_event();
        assert!(r_bad_num.next_event().is_err());

        // Mismatched end tag
        let mut r_mismatch = XmlReader::new("<root><item></mismatch></root>");
        let _ = r_mismatch.next_event();
        let _ = r_mismatch.next_event();
        assert!(r_mismatch.next_event().is_err());

        // Unexpected end tag
        let mut r_orphan = XmlReader::new("</orphan>");
        assert!(r_orphan.next_event().is_err());

        // Duplicate attribute
        let mut r_dup = XmlReader::new("<root attr=\"1\" attr=\"2\"/>");
        assert!(r_dup.next_event().is_err());

        // Undeclared prefix on element
        let mut r_undef = XmlReader::new("<undef:root/>");
        assert!(r_undef.next_event().is_err());

        // Undeclared prefix on attribute
        let mut r_undef_attr = XmlReader::new("<root undef:attr=\"1\"/>");
        assert!(r_undef_attr.next_event().is_err());

        // Forbidden xmlns bindings
        let mut r_xmlns1 = XmlReader::new("<root xmlns:xmlns=\"http://example.com\"/>");
        assert!(r_xmlns1.next_event().is_err());
        let mut r_xmlns2 = XmlReader::new("<root xmlns:p=\"http://www.w3.org/2000/xmlns/\"/>");
        assert!(r_xmlns2.next_event().is_err());

        // Depth limit exceeded
        let limits_depth = XmlReaderLimits { max_depth: 1, ..Default::default() };
        let mut r_depth = XmlReader::with_limits("<root><child><grandchild/></child></root>", limits_depth);
        let _ = r_depth.next_event();
        assert!(r_depth.next_event().is_err());

        // Attribute limit exceeded
        let limits_attr = XmlReaderLimits { max_attributes: 1, ..Default::default() };
        let mut r_attrs = XmlReader::with_limits("<root a1=\"1\" a2=\"2\"/>", limits_attr);
        assert!(r_attrs.next_event().is_err());

        // Token length limit exceeded (triggers format_num_commas)
        let limits_tok = XmlReaderLimits { max_token_length: 4, ..Default::default() };
        let mut r_tok_elem = XmlReader::with_limits("<toolong/>", limits_tok);
        assert!(r_tok_elem.next_event().is_err());
        let mut r_tok_attr = XmlReader::with_limits("<root toolongattr=\"1\"/>", limits_tok);
        assert!(r_tok_attr.next_event().is_err());

        // UTF-8 BOM stripping
        let mut r_bom = XmlReader::new("\u{FEFF}<root/>");
        assert!(r_bom.next_event().unwrap().is_some());

        // add_namespace_binding, set_permissive_namespaces, push_back
        let mut r_custom = XmlReader::new("<root><item/></root>");
        r_custom.add_namespace_binding("pfx", "http://example.com");
        r_custom.set_permissive_namespaces(true);
        assert_eq!(r_custom.resolve_prefix("pfx"), Some("http://example.com"));
        assert_eq!(r_custom.resolve_prefix("xml"), Some("http://www.w3.org/XML/1998/namespace"));
        assert_eq!(r_custom.resolve_prefix("xmlns"), Some("http://www.w3.org/2000/xmlns/"));
        assert_eq!(r_custom.find_prefixes_for_uri("http://example.com"), &["pfx"]);

        let ev1 = r_custom.next_event().unwrap().unwrap();
        r_custom.push_back(ev1);
        let ev1_again = r_custom.next_event().unwrap().unwrap();
        assert!(matches!(ev1_again, XmlEvent::StartElement { .. }));

        // resolve_default_ns with empty xmlns
        let mut r_empty_ns = XmlReader::new("<root xmlns=\"http://default.com\"><child xmlns=\"\"><sub/></child></root>");
        let _ = r_empty_ns.next_event(); // <root>
        assert_eq!(r_empty_ns.resolve_default_ns(), Some("http://default.com"));
        let _ = r_empty_ns.next_event(); // <child>
        assert_eq!(r_empty_ns.resolve_default_ns(), None);

        // Invalid Unicode scalar entity (surrogate)
        let mut r_surrogate = XmlReader::new("<root>&#xD800;</root>");
        let _ = r_surrogate.next_event();
        assert!(r_surrogate.next_event().is_err());

        // Invalid decimal entity
        let mut r_bad_dec = XmlReader::new("<root>&#abc;</root>");
        let _ = r_bad_dec.next_event();
        assert!(r_bad_dec.next_event().is_err());

        // Unterminated entity reference
        let mut r_unterm = XmlReader::new("<root>&unterm</root>");
        let _ = r_unterm.next_event();
        assert!(r_unterm.next_event().is_err());

        // Entity declaration prohibited (Token::EntityDeclaration)
        let mut r_ent_decl = XmlReader::new("<!ENTITY foo \"bar\"><root/>");
        assert!(r_ent_decl.next_event().is_err());

        // Capital hex entity &#X41;
        let mut r_cap_hex = XmlReader::new("<root>&#X41;</root>");
        let _ = r_cap_hex.next_event();
        let ev_hex = r_cap_hex.next_event().unwrap().unwrap();
        assert!(matches!(ev_hex, XmlEvent::Text { content, .. } if content == "A"));

        // in_scope_namespace_bindings with initial bindings
        let mut r_init = XmlReader::new("<root><item/></root>");
        r_init.add_namespace_binding("init_pfx", "http://init.example.com");
        let _ = r_init.next_event(); // <root>
        let in_scope = r_init.in_scope_namespace_bindings();
        assert!(in_scope.iter().any(|(p, u)| p == "init_pfx" && u == "http://init.example.com"));

        // current_element_namespace_bindings without default_ns
        let mut r_nodef = XmlReader::new("<root xmlns:p=\"http://p.com\"/>");
        let _ = r_nodef.next_event();
        let cur_bindings = r_nodef.current_element_namespace_bindings();
        assert!(cur_bindings.iter().any(|(p, u)| p == "p" && u == "http://p.com"));

        // Unclosed tags error branch (lines 431-434)
        let mut r_unclosed = XmlReader::new("<root><item>");
        let _ = r_unclosed.next_event(); // <root>
        let _ = r_unclosed.next_event(); // <item>
        assert!(r_unclosed.next_event().is_err());

        // resolve_prefix with initial_bindings (line 133)
        let mut r_ib = XmlReader::new("<root/>");
        r_ib.add_namespace_binding("pfx_init", "http://init.example.org");
        assert_eq!(r_ib.resolve_prefix("pfx_init"), Some("http://init.example.org"));
        assert_eq!(r_ib.resolve_prefix("nonexistent"), None);

        // finalize_start_element when pending_start is None (lines 338-339)
        let mut r_no_pending = XmlReader::new("<root/>");
        assert!(r_no_pending.finalize_start_element(false).is_err());

        // Invalid numeric entity with empty digits &#; (lines 260-261)
        let mut r_empty_num = XmlReader::new("<root>&#;</root>");
        let _ = r_empty_num.next_event();
        assert!(r_empty_num.next_event().is_err());

        // Invalid Unicode scalar value entity (lines 274-278)
        let mut r_bad_scalar = XmlReader::new("<root>&#1114112;</root>");
        let _ = r_bad_scalar.next_event();
        assert!(r_bad_scalar.next_event().is_err());

        // Declaration with empty encoding attribute (line 445)
        let mut r_empty_enc = XmlReader::new("<?xml version=\"1.0\" encoding=\"\"?><root/>");
        let ev_doc = r_empty_enc.next_event().unwrap().unwrap();
        assert!(matches!(ev_doc, XmlEvent::StartDocument { encoding, .. } if encoding == "UTF-8"));

        // xmlns bound to xmlns namespace prohibited (lines 522-527)
        let mut r_xmlns_ns = XmlReader::new("<root xmlns=\"http://www.w3.org/2000/xmlns/\"/>");
        assert!(r_xmlns_ns.next_event().is_err());

        // Unexpected end tag when stack is empty (lines 566-569)
        let mut r_extra_end = XmlReader::new("</orphan>");
        assert!(r_extra_end.next_event().is_err());

        // Mismatched end tag (lines 571-576)
        let mut r_mismatch = XmlReader::new("<open></close>");
        let _ = r_mismatch.next_event();
        assert!(r_mismatch.next_event().is_err());

        // CData event parsing (lines 598-601)
        let mut r_cdata = XmlReader::new("<root><![CDATA[cdata payload]]></root>");
        let _ = r_cdata.next_event();
        assert!(matches!(r_cdata.next_event().unwrap().unwrap(), XmlEvent::CData { content, .. } if content == "cdata payload"));

        // Comment event parsing (lines 606-608)
        let mut r_comment = XmlReader::new("<!-- test comment --><root/>");
        assert!(matches!(r_comment.next_event().unwrap().unwrap(), XmlEvent::Comment { content, .. } if content == " test comment "));

        // ProcessingInstruction event parsing (lines 616-620)
        let mut r_pi = XmlReader::new("<?my-target some instructions?><root/>");
        assert!(matches!(r_pi.next_event().unwrap().unwrap(), XmlEvent::ProcessingInstruction { target, content: Some(c), .. } if target == "my-target" && c == "some instructions"));
    }

    /// Tests additional XML reader edge cases for entity decoding, token length limits,
    /// declaration attributes, and prohibited namespace bindings.
    ///
    /// Verifies that:
    /// 1. Token length limits correctly reject attribute values exceeding `max_token_length`.
    /// 2. Unterminated entity references trigger parse errors.
    /// 3. Invalid hex entity codes are rejected.
    /// 4. Uppercase hex entities (`&#X41;`) are decoded to their corresponding characters.
    /// 5. Prohibited `xmlns:xmlns` attribute bindings are rejected per XML Namespace specs.
    /// 6. XML declaration with `standalone` attribute is parsed successfully.
    /// 7. Prohibited custom DTD entity references trigger parse errors.
    /// 8. Standard predefined XML entities (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`) are decoded.
    #[test]
    fn test_xml_reader_extended_edge_cases() {
        // 1. max_token_length exceeded in attribute value (lines 500-506)
        let limits = XmlReaderLimits { max_token_length: 5, ..Default::default() };
        let mut r_tok_len = XmlReader::with_limits("<root verylongattributename=\"val\"/>", limits);
        assert!(r_tok_len.next_event().is_err());

        // 2. Unterminated entity reference (line 237)
        let mut r_unterminated = XmlReader::new("<root attr=\"&unterminated\"/>");
        assert!(r_unterminated.next_event().is_err());

        // 3. Invalid hex entity code (line 265)
        let mut r_bad_hex = XmlReader::new("<root>&#xZZ;</root>");
        let _ = r_bad_hex.next_event();
        assert!(r_bad_hex.next_event().is_err());

        // 4. Uppercase hex entity &#X41; (line 263)
        let mut r_hex_cap = XmlReader::new("<root>&#X41;</root>");
        let _ = r_hex_cap.next_event();
        let ev_text = r_hex_cap.next_event().unwrap().unwrap();
        assert!(matches!(ev_text, XmlEvent::Text { content, .. } if content == "A"));

        // 5. xmlns:xmlns attribute binding prohibited (line 521)
        let mut r_xmlns_xmlns = XmlReader::new("<root xmlns:xmlns=\"http://example.com\"/>");
        assert!(r_xmlns_xmlns.next_event().is_err());

        // 6. XML declaration standalone attribute (line 450)
        let mut r_standalone = XmlReader::new("<?xml version=\"1.0\" standalone=\"yes\"?><root/>");
        assert!(r_standalone.next_event().unwrap().is_some());

        // 7. Prohibited custom DTD entity reference (lines 282-285)
        let mut r_prohibited_ent = XmlReader::new("<root>&custom;</root>");
        let _ = r_prohibited_ent.next_event();
        assert!(r_prohibited_ent.next_event().is_err());

        // 8. Standard predefined XML entities (lines 253-257)
        let mut r_std_ents = XmlReader::new("<root attr=\"&amp;&lt;&gt;&quot;&apos;\"></root>");
        let ev = r_std_ents.next_event().unwrap().unwrap();
        assert!(matches!(ev, XmlEvent::StartElement { attributes, .. } if attributes[0].value == "&<>\"'"));

        // 9. Invalid decimal entity code (line 268)
        let mut r_bad_dec = XmlReader::new("<root>&#invalid;</root>");
        let _ = r_bad_dec.next_event();
        assert!(r_bad_dec.next_event().is_err());

        // 10. Numeric entity code pointing to invalid Unicode scalar (surrogate 0xD800, line 273)
        let mut r_surrogate = XmlReader::new("<root>&#xD800;</root>");
        let _ = r_surrogate.next_event();
        assert!(r_surrogate.next_event().is_err());

        // 11. Unexpected end element tag when stack is empty after closing root (lines 552-554)
        let mut r_unexpected_end = XmlReader::new("<root></root></extra>");
        let _ = r_unexpected_end.next_event();
        let _ = r_unexpected_end.next_event();
        assert!(r_unexpected_end.next_event().is_err());

        // 12. XML Comments and Processing Instructions (lines 604-620)
        let xml_nodes = "<root><!-- commentary --><?proc_inst data?></root>";
        let mut r_nodes = XmlReader::new(xml_nodes);
        let _ = r_nodes.next_event(); // Start root
        let ev_comment = r_nodes.next_event().unwrap().unwrap();
        assert!(matches!(ev_comment, XmlEvent::Comment { content, .. } if content == " commentary "));
        let ev_pi = r_nodes.next_event().unwrap().unwrap();
        assert!(matches!(ev_pi, XmlEvent::ProcessingInstruction { target, content: Some(c), .. } if target == "proc_inst" && c == "data"));

        // 13. W3C XML 1.0 §3.3.3 Attribute Value Normalization
        // Literal tabs and newlines must be normalized to space #x20,
        // but character references (&#x9;, &#xA;) must be preserved.
        let xml_attr_norm = "<root raw=\"line1\tline2\nline3\rline4\" char_ref=\"val&#x9;part&#xA;end\"/>";
        let mut r_attr_norm = XmlReader::new(xml_attr_norm);
        let ev_norm = r_attr_norm.next_event().unwrap().unwrap();
        if let XmlEvent::StartElement { attributes, .. } = ev_norm {
            assert_eq!(attributes.len(), 2);
            assert_eq!(attributes[0].value, "line1 line2 line3 line4");
            assert_eq!(attributes[1].value, "val\tpart\nend");
        } else {
            panic!("Expected StartElement");
        }
    }
}
