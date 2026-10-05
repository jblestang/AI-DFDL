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
                let remaining = bytes.get(idx..).ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Invalid entity byte range")
                })?;

                let semi_pos = remaining.iter().position(|&c| c == b';').ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unterminated entity reference")
                })?;

                let entity_end = idx.checked_add(semi_pos).ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::ImplementationLimit, "Entity index overflow")
                })?;

                let entity_start = idx.checked_add(1).ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::ImplementationLimit, "Entity index overflow")
                })?;

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
                        let code_str = entity_str.get(1..).ok_or_else(|| {
                            DFDLError::new(DFDLErrorKind::Parse, "Invalid numeric entity")
                        })?;

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

                idx = entity_end.checked_add(1).ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::ImplementationLimit, "Index overflow")
                })?;
            } else {
                let end_idx = idx.checked_add(1).ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::ImplementationLimit, "Index overflow")
                })?;
                let ch = text
                    .get(idx..end_idx)
                    .ok_or_else(|| DFDLError::new(DFDLErrorKind::Parse, "Invalid char slice"))?;
                out.push_str(ch);
                idx = end_idx;
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

                    let current_depth = self.ns_stack.len().checked_add(1).ok_or_else(|| {
                        DFDLError::new(DFDLErrorKind::ImplementationLimit, "Depth overflow")
                    })?;

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
                    let decoded = self.decode_entities(value.as_str())?;

                    let state = self.pending_start.as_mut().ok_or_else(|| {
                        DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Attribute occurred outside element start tag",
                        )
                        .with_location(loc)
                    })?;

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

    #[test]
    fn test_xml_reader_start_end_element() {
        let xml = "<root xmlns=\"http://example.com\"><item id=\"1\">Hello</item></root>";
        let mut reader = XmlReader::new(xml);

        let ev1 = reader.next_event().unwrap().unwrap();
        if let XmlEvent::StartElement {
            name, attributes, ..
        } = ev1
        {
            assert_eq!(name.local_name, "root");
            assert_eq!(name.namespace.unwrap().as_str(), "http://example.com");
            assert!(attributes.is_empty());
        } else {
            panic!("Expected StartElement root");
        }

        let ev2 = reader.next_event().unwrap().unwrap();
        if let XmlEvent::StartElement {
            name, attributes, ..
        } = ev2
        {
            assert_eq!(name.local_name, "item");
            assert_eq!(attributes.len(), 1);
            assert_eq!(attributes[0].name.local_name, "id");
            assert_eq!(attributes[0].value, "1");
        } else {
            panic!("Expected StartElement item");
        }

        let ev3 = reader.next_event().unwrap().unwrap();
        if let XmlEvent::Text { content, .. } = ev3 {
            assert_eq!(content, "Hello");
        } else {
            panic!("Expected Text");
        }
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
}
