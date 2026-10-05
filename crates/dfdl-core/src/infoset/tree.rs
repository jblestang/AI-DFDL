//! Owned in-memory DFDL Infoset tree representation (`alloc`).
//!
//! Provides [`InfosetDocument`], [`InfosetElement`], and [`InfosetNode`] structures for owned DOM traversal and verification.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::events::{InfosetEvent, InfosetEventSink, InfosetSource};
use crate::infoset::state::ElementState;
use crate::limits::ResourceLimits;
use crate::types::{InfosetPath, QName};
use crate::util::{get_checked, try_push};

/// Individual node in an owned DFDL Infoset tree.
#[derive(Debug, Clone, PartialEq)]
pub enum InfosetNode {
    /// Element Information Item node.
    Element(InfosetElement),
}

/// Element Information Item in an owned Infoset tree.
#[derive(Debug, Clone, PartialEq)]
pub struct InfosetElement {
    /// Qualified Name of the element item.
    pub name: QName,
    /// Value state (Value, Empty, Nil, NoValue, Absent).
    pub state: ElementState,
    /// Children nodes if complex element container.
    pub children: Vec<InfosetNode>,
    /// Indicates if element is inside a hidden group (§14.2).
    pub is_hidden: bool,
    /// Explicit xsi:nil attribute value if present in infoset.
    pub nil_attribute: Option<bool>,
}

impl InfosetElement {
    /// Constructs a new simple element with a scalar value state.
    #[inline]
    #[must_use]
    pub const fn simple(name: QName, state: ElementState) -> Self {
        Self {
            name,
            state,
            children: Vec::new(),
            is_hidden: false,
            nil_attribute: None,
        }
    }

    /// Constructs a new complex element container.
    #[inline]
    #[must_use]
    pub const fn complex(name: QName) -> Self {
        Self {
            name,
            state: ElementState::NoValue,
            children: Vec::new(),
            is_hidden: false,
            nil_attribute: None,
        }
    }

    /// Sets the visibility flag for this element.
    #[inline]
    #[must_use]
    pub const fn with_hidden(mut self, is_hidden: bool) -> Self {
        self.is_hidden = is_hidden;
        self
    }

    /// Sets the explicit xsi:nil attribute state for this element.
    #[inline]
    #[must_use]
    pub const fn with_nil_attribute(mut self, nil: Option<bool>) -> Self {
        self.nil_attribute = nil;
        self
    }

    /// Adds a child node to a complex element using fallible allocation.
    pub fn try_add_child(&mut self, child: InfosetNode) -> DFDLResult<()> {
        try_push(&mut self.children, child)
    }
}

/// Root Document Information Item containing the top-level Infoset element.
#[derive(Debug, Clone, PartialEq)]
pub struct InfosetDocument {
    /// Distinguished root element of the document.
    pub root: Option<InfosetElement>,
    /// Total node count tracking.
    pub total_nodes: usize,
}

impl Default for InfosetDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl InfosetDocument {
    /// Constructs a new empty [`InfosetDocument`].
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            root: None,
            total_nodes: 0,
        }
    }

    /// Builds an [`InfosetDocument`] from an owned root element.
    #[inline]
    pub fn with_root(root: InfosetElement) -> Self {
        let count = Self::count_nodes_in_elem(&root);
        Self {
            root: Some(root),
            total_nodes: count,
        }
    }

    fn count_nodes_in_elem(elem: &InfosetElement) -> usize {
        let mut count: usize = 1;
        for child in &elem.children {
            match child {
                InfosetNode::Element(ref sub) => {
                    count = count.saturating_add(Self::count_nodes_in_elem(sub));
                }
            }
        }
        count
    }

    /// Finds an [`InfosetElement`] by path navigation.
    pub fn find_element<'a>(&'a self, path: &InfosetPath) -> Option<&'a InfosetElement> {
        self.find_element_with_context(path, 0)
    }

    /// Finds an [`InfosetElement`] by path navigation using `occurs_index` for array elements.
    pub fn find_element_with_context<'a>(
        &'a self,
        path: &InfosetPath,
        occurs_index: usize,
    ) -> Option<&'a InfosetElement> {
        self.find_element_with_context_checked(path, occurs_index, true)
            .ok()
            .flatten()
    }

    /// Finds an [`InfosetElement`] by path navigation using `occurs_index` for array elements,
    /// returning a [`DFDLError`] if a query-style path returns multiple elements without an index predicate.
    pub fn find_element_with_context_checked<'a>(
        &'a self,
        path: &InfosetPath,
        occurs_index: usize,
        is_self_ref: bool,
    ) -> DFDLResult<Option<&'a InfosetElement>> {
        self.find_element_with_policy_checked(
            path,
            occurs_index,
            is_self_ref,
            crate::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace,
            &[],
        )
    }

    /// Finds an [`InfosetElement`] by path navigation, matching namespaces and respecting
    /// the [`UnqualifiedPathStepPolicy`] for unqualified path steps.
    pub fn find_element_with_policy_checked<'a>(
        &'a self,
        path: &InfosetPath,
        occurs_index: usize,
        is_self_ref: bool,
        policy: crate::types::UnqualifiedPathStepPolicy,
        in_scope_namespaces: &[(alloc::string::String, alloc::string::String)],
    ) -> DFDLResult<Option<&'a InfosetElement>> {
        let root = match self.root.as_ref() {
            Some(r) => r,
            None => return Ok(None),
        };
        let segs = path.segments();
        if segs.is_empty() {
            return Ok(Some(root));
        }

        let mut stack: Vec<&'a InfosetElement> = Vec::new();
        let _ = try_push(&mut stack, root);

        let clean_root_name = root
            .name
            .local_name
            .split(':')
            .next_back()
            .unwrap_or(&root.name.local_name);

        let mut idx: usize = 0;
        if path.is_absolute() {
            if let Some(first_seg) = segs.first() {
                let raw_first = first_seg.split('[').next().unwrap_or(first_seg);
                let clean_first = raw_first.split(':').next_back().unwrap_or(raw_first);
                if clean_first == clean_root_name {
                    idx = 1;
                }
            }
        }

        while idx < segs.len() {
            let seg = match get_checked(segs, idx) {
                Ok(s) => s,
                Err(_) => return Ok(None),
            };
            let raw_step = seg.split('[').next().unwrap_or(seg);
            let clean_seg = seg.split(':').next_back().unwrap_or(seg);
            let clean_step = clean_seg.split('[').next().unwrap_or(clean_seg);

            if clean_step == "." || clean_step.starts_with(".(") {
                idx = idx.saturating_add(1);
                continue;
            }

            if clean_step == ".." || clean_step.starts_with("..(") {
                if stack.len() > 1 {
                    let _ = stack.pop();
                }
                idx = idx.saturating_add(1);
                continue;
            }

            let curr = match stack.last() {
                Some(c) => *c,
                None => return Ok(None),
            };

            let default_ns = in_scope_namespaces
                .iter()
                .find(|(p, _)| p.is_empty())
                .map(|(_, u)| u.as_str());

            let mut matches = Vec::new();
            for child in &curr.children {
                match child {
                    InfosetNode::Element(ref elem) => {
                        let clean_elem = elem
                            .name
                            .local_name
                            .split(':')
                            .next_back()
                            .unwrap_or(&elem.name.local_name);

                        if raw_step == "*" {
                            let _ = try_push(&mut matches, elem);
                        } else if raw_step.starts_with('{') {
                            if let Some(end_brace) = raw_step.find('}') {
                                let uri = &raw_step[1..end_brace];
                                let local = &raw_step[end_brace.saturating_add(1)..];
                                if clean_elem == local
                                    && elem.name.namespace.as_ref().map(|n| n.as_str()) == Some(uri)
                                {
                                    let _ = try_push(&mut matches, elem);
                                }
                            }
                        } else if let Some((pfx, local)) = raw_step.split_once(':') {
                            if !pfx.is_empty() && pfx != "." && pfx != ".." && clean_elem == local {
                                // Qualified path step: element MUST have a namespace or prefix.
                                if elem.name.namespace.is_some() || elem.name.prefix.is_some() {
                                    if let Some((_, uri)) = in_scope_namespaces.iter().find(|(p, _)| p == pfx) {
                                        if elem.name.namespace.as_ref().map(|n| n.as_str()) == Some(uri.as_str()) {
                                            let _ = try_push(&mut matches, elem);
                                        }
                                    } else if let Some(ref elem_pfx) = elem.name.prefix {
                                        if elem_pfx == pfx {
                                            let _ = try_push(&mut matches, elem);
                                        }
                                    }
                                }
                            }
                        } else if clean_elem == raw_step {
                            // Unqualified path step: apply UnqualifiedPathStepPolicy
                            match policy {
                                crate::types::UnqualifiedPathStepPolicy::NoNamespace => {
                                    if elem.name.namespace.is_none() && elem.name.prefix.is_none() {
                                        let _ = try_push(&mut matches, elem);
                                    }
                                }
                                crate::types::UnqualifiedPathStepPolicy::DefaultNamespace => {
                                    if let Some(def_uri) = default_ns {
                                        if elem.name.namespace.as_ref().map(|n| n.as_str()) == Some(def_uri) {
                                            let _ = try_push(&mut matches, elem);
                                        }
                                    } else if elem.name.namespace.is_none() && elem.name.prefix.is_none() {
                                        let _ = try_push(&mut matches, elem);
                                    }
                                }
                                crate::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace => {
                                    if let Some(def_uri) = default_ns {
                                        let has_default_ns_child = curr.children.iter().any(|c| match c {
                                            InfosetNode::Element(e) => {
                                                let c_name = e.name.local_name.split(':').next_back().unwrap_or(&e.name.local_name);
                                                c_name == raw_step && e.name.namespace.as_ref().map(|n| n.as_str()) == Some(def_uri)
                                            }
                                        });
                                        if has_default_ns_child {
                                            if elem.name.namespace.as_ref().map(|n| n.as_str()) == Some(def_uri) {
                                                let _ = try_push(&mut matches, elem);
                                            }
                                        } else {
                                            let has_no_ns_child = curr.children.iter().any(|c| match c {
                                                InfosetNode::Element(e) => {
                                                    let c_name = e.name.local_name.split(':').next_back().unwrap_or(&e.name.local_name);
                                                    c_name == raw_step && e.name.namespace.is_none() && e.name.prefix.is_none()
                                                }
                                            });
                                            if has_no_ns_child {
                                                if elem.name.namespace.is_none() && elem.name.prefix.is_none() {
                                                    let _ = try_push(&mut matches, elem);
                                                }
                                            } else {
                                                let _ = try_push(&mut matches, elem);
                                            }
                                        }
                                    } else {
                                        let has_no_ns_child = curr.children.iter().any(|c| match c {
                                            InfosetNode::Element(e) => {
                                                let c_name = e.name.local_name.split(':').next_back().unwrap_or(&e.name.local_name);
                                                c_name == raw_step && e.name.namespace.is_none() && e.name.prefix.is_none()
                                            }
                                        });
                                        if has_no_ns_child {
                                            if elem.name.namespace.is_none() && elem.name.prefix.is_none() {
                                                let _ = try_push(&mut matches, elem);
                                            }
                                        } else {
                                            let _ = try_push(&mut matches, elem);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if matches.is_empty() {
                if stack.len() == 1 && clean_step == clean_root_name {
                    idx = idx.saturating_add(1);
                    continue;
                }
                return Ok(None);
            }
            let is_last_step = idx == segs.len().saturating_sub(1);
            let explicit_index =
                if let (Some(b_open), Some(b_close)) = (clean_seg.find('['), clean_seg.find(']')) {
                    if b_open < b_close {
                        clean_seg
                            .get(b_open.saturating_add(1)..b_close)
                            .and_then(|s| s.parse::<usize>().ok())
                    } else {
                        None
                    }
                } else {
                    None
                };

            let is_self_instance = is_self_ref && is_last_step && occurs_index > 0;
            if matches.len() > 1 && explicit_index.is_none() && !is_self_instance {
                let qname_desc = if let Some(first_match) = matches.first() {
                    if let Some(ref ns) = first_match.name.namespace {
                        if let Some(ref pfx) = first_match.name.prefix {
                            alloc::format!("{}:{{{}}}{}", pfx, ns.as_str(), clean_step)
                        } else if let Some(pfx) = seg.split(':').next().filter(|_| seg.contains(':')) {
                            alloc::format!("{}:{{{}}}{}", pfx, ns.as_str(), clean_step)
                        } else {
                            alloc::format!("{{{}}}{}", ns.as_str(), clean_step)
                        }
                    } else {
                        alloc::string::ToString::to_string(clean_step)
                    }
                } else {
                    alloc::string::ToString::to_string(clean_step)
                };
                let msg = alloc::format!(
                    "Schema Definition Error: query-style path expression: Path step '{}' ('{}') ambiguous: matched {} elements but has no index predicate",
                    qname_desc,
                    clean_step,
                    matches.len()
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }

            let chosen_elem = if let Some(exp_idx) = explicit_index {
                if exp_idx > 0 && exp_idx <= matches.len() {
                    *get_checked(&matches, exp_idx.saturating_sub(1)).map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::ExpressionError,
                            &alloc::format!("{}", e),
                        )
                    })?
                } else {
                    return Ok(None);
                }
            } else if is_last_step {
                if occurs_index > 0 && occurs_index <= matches.len() {
                    *get_checked(&matches, occurs_index.saturating_sub(1)).map_err(|e| {
                        DFDLError::new(
                            DFDLErrorKind::ExpressionError,
                            &alloc::format!("{}", e),
                        )
                    })?
                } else {
                    match matches.first() {
                        Some(&f) => f,
                        None => return Ok(None),
                    }
                }
            } else {
                match matches.last() {
                    Some(&l) => l,
                    None => return Ok(None),
                }
            };
            let _ = try_push(&mut stack, chosen_elem);
            idx = idx.saturating_add(1);
        }

        Ok(stack.pop())
    }

    /// Produces a public [`InfosetDocument`] by stripping all elements marked as hidden (§14.2).
    #[must_use]
    pub fn strip_hidden(&self) -> Self {
        let root = self.root.as_ref().and_then(Self::strip_hidden_from_elem);
        let count = root.as_ref().map_or(0, Self::count_nodes_in_elem);
        Self {
            root,
            total_nodes: count,
        }
    }

    fn strip_hidden_from_elem(elem: &InfosetElement) -> Option<InfosetElement> {
        if elem.is_hidden {
            return None;
        }
        let mut new_elem = elem.clone();
        new_elem.children = elem
            .children
            .iter()
            .filter_map(|child| match child {
                InfosetNode::Element(ref sub) => {
                    Self::strip_hidden_from_elem(sub).map(InfosetNode::Element)
                }
            })
            .collect();
        Some(new_elem)
    }
}

/// Owned Infoset builder acting as an [`InfosetEventSink`].
#[derive(Debug, Clone)]
pub struct InfosetBuilder {
    limits: ResourceLimits,
    doc: InfosetDocument,
    stack: Vec<InfosetElement>,
}

impl Default for InfosetBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl InfosetBuilder {
    /// Creates a new [`InfosetBuilder`] with default resource limits.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(ResourceLimits::default())
    }

    /// Creates a new [`InfosetBuilder`] with configured resource limits.
    #[inline]
    #[must_use]
    pub const fn with_limits(limits: ResourceLimits) -> Self {
        Self {
            limits,
            doc: InfosetDocument::new(),
            stack: Vec::new(),
        }
    }

    /// Creates a checkpoint marker of current builder state for backtracking.
    #[inline]
    #[must_use]
    pub fn checkpoint(&self) -> Self {
        self.clone()
    }

    /// Restores builder state to a previous checkpoint.
    #[inline]
    pub fn rollback(&mut self, cp: Self) {
        *self = cp;
    }

    /// Returns a reference to the constructed [`InfosetDocument`].
    #[inline]
    #[must_use]
    pub const fn document(&self) -> &InfosetDocument {
        &self.doc
    }

    /// Returns active [`InfosetDocument`] representation, incorporating active element stack.
    #[must_use]
    pub fn active_doc(&self) -> InfosetDocument {
        if let Some(ref _root) = self.doc.root {
            return self.doc.clone();
        }
        if self.stack.is_empty() {
            return InfosetDocument::new();
        }
        let mut curr = match self.stack.last() {
            Some(elem) => elem.clone(),
            None => return InfosetDocument::new(),
        };
        for elem in self.stack.iter().rev().skip(1) {
            let mut parent = elem.clone();
            let _ = parent.try_add_child(InfosetNode::Element(curr));
            curr = parent;
        }
        let count = InfosetDocument::count_nodes_in_elem(&curr);
        InfosetDocument {
            root: Some(curr),
            total_nodes: count,
        }
    }

    /// Returns current Infoset path corresponding to active element stack.
    #[must_use]
    pub fn current_path(&self) -> InfosetPath {
        let mut path = InfosetPath::root();
        for (i, elem) in self.stack.iter().enumerate() {
            let seg = if i > 0 {
                let parent = match self.stack.get(i.saturating_sub(1)) {
                    Some(p) => p,
                    None => continue,
                };
                let prev_count = parent
                    .children
                    .iter()
                    .filter(|child| match child {
                        InfosetNode::Element(c) => c.name.local_name == elem.name.local_name,
                    })
                    .count();
                if prev_count > 0 {
                    alloc::format!("{}[{}]", elem.name.local_name, prev_count.saturating_add(1))
                } else {
                    elem.name.local_name.clone()
                }
            } else {
                elem.name.local_name.clone()
            };
            let _ = path.try_push(&seg);
        }
        if path.segments().is_empty() {
            if let Some(ref root) = self.doc.root {
                let _ = path.try_push(&root.name.local_name);
            }
        }
        path
    }

    /// Returns the number of occurrences of a child element with `local_name` added to current active parent.
    #[must_use]
    pub fn count_child_occurrences(&self, local_name: &str) -> usize {
        let clean_target = local_name.split(':').next_back().unwrap_or(local_name);
        if let Some(parent) = self.stack.last() {
            parent
                .children
                .iter()
                .filter(|child| match child {
                    InfosetNode::Element(elem) => {
                        let clean_elem = elem
                            .name
                            .local_name
                            .split(':')
                            .next_back()
                            .unwrap_or(&elem.name.local_name);
                        clean_elem == clean_target
                    }
                })
                .count()
        } else if let Some(ref root) = self.doc.root {
            let clean_root = root
                .name
                .local_name
                .split(':')
                .next_back()
                .unwrap_or(&root.name.local_name);
            if clean_root == clean_target {
                1
            } else {
                0
            }
        } else {
            0
        }
    }

    /// Returns the number of children in the currently active parent element.
    #[must_use]
    pub fn current_child_count(&self) -> usize {
        if let Some(parent) = self.stack.last() {
            parent.children.len()
        } else if let Some(ref root) = self.doc.root {
            root.children.len()
        } else {
            0
        }
    }

    /// Stably reorders children in the currently active parent added from `start_index`
    /// onwards according to the given schema element names (§14.3).
    pub fn reorder_children_from(&mut self, start_index: usize, schema_names: &[&str]) {
        let children = if let Some(parent) = self.stack.last_mut() {
            &mut parent.children
        } else if let Some(ref mut root) = self.doc.root {
            &mut root.children
        } else {
            return;
        };

        let Some(slice) = children.get_mut(start_index..) else {
            return;
        };
        slice.sort_by_key(|child| {
            let name = match child {
                InfosetNode::Element(elem) => elem
                    .name
                    .local_name
                    .split(':')
                    .next_back()
                    .unwrap_or(&elem.name.local_name),
            };
            schema_names
                .iter()
                .position(|&sn| {
                    let clean_sn = sn.split(':').next_back().unwrap_or(sn);
                    clean_sn == name
                })
                .unwrap_or(usize::MAX)
        });
    }

    /// Consumes the builder and returns the completed [`InfosetDocument`].
    pub fn build(self) -> DFDLResult<InfosetDocument> {
        if self.doc.root.is_some() {
            Ok(self.doc)
        } else if let Some(first) = self.stack.first() {
            Ok(InfosetDocument {
                root: Some(first.clone()),
                total_nodes: InfosetDocument::count_nodes_in_elem(first),
            })
        } else {
            Ok(InfosetDocument::new())
        }
    }

    /// Receives a streaming [`InfosetEvent`] with explicit visibility status (§14.2).
    pub fn push_event_with_hidden(
        &mut self,
        event: InfosetEvent,
        is_hidden: bool,
    ) -> DFDLResult<()> {
        match event {
            InfosetEvent::StartDocument => Ok(()),
            InfosetEvent::EndDocument => Ok(()),
            InfosetEvent::StartElement { name, is_nil } => {
                let current_depth = self.stack.len().saturating_add(1);
                if !self.limits.check_nesting(current_depth) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::ImplementationLimit,
                        "Infoset nesting depth limit exceeded",
                    ));
                }

                let state = if is_nil {
                    ElementState::Nil
                } else {
                    ElementState::NoValue
                };

                let elem = InfosetElement::simple(name, state).with_hidden(is_hidden);
                try_push(&mut self.stack, elem)
            }
            InfosetEvent::EndElement { name } => {
                let elem = self.stack.pop().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unexpected EndElement event")
                })?;

                if elem.name != name {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        "Mismatched EndElement event QName",
                    ));
                }

                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.doc.total_nodes = InfosetDocument::count_nodes_in_elem(&elem);
                    self.doc.root = Some(elem);
                }
                Ok(())
            }
            InfosetEvent::SimpleValue { name, value } => {
                let elem =
                    InfosetElement::simple(name, ElementState::Value(value)).with_hidden(is_hidden);
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))
                } else {
                    self.doc.total_nodes = 1;
                    self.doc.root = Some(elem);
                    Ok(())
                }
            }
            InfosetEvent::EmptyValue { name } => {
                let elem = InfosetElement::simple(name, ElementState::Empty).with_hidden(is_hidden);
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))
                } else {
                    self.doc.total_nodes = 1;
                    self.doc.root = Some(elem);
                    Ok(())
                }
            }
            InfosetEvent::NilValue { name } => {
                let elem = InfosetElement::simple(name, ElementState::Nil).with_hidden(is_hidden);
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))
                } else {
                    self.doc.total_nodes = 1;
                    self.doc.root = Some(elem);
                    Ok(())
                }
            }
        }
    }
}

impl InfosetEventSink for InfosetBuilder {
    fn push_event(&mut self, event: InfosetEvent) -> DFDLResult<()> {
        self.push_event_with_hidden(event, false)
    }
}

/// Trait implementation for converting an [`InfosetDocument`] into an [`InfosetSource`] stream.
#[derive(Debug)]
pub struct InfosetTreeSource {
    events: Vec<InfosetEvent>,
    position: usize,
}

impl InfosetTreeSource {
    /// Creates a streaming [`InfosetSource`] from an owned [`InfosetDocument`].
    pub fn from_document(doc: &InfosetDocument) -> DFDLResult<Self> {
        let mut events = Vec::new();
        try_push(&mut events, InfosetEvent::StartDocument)?;

        if let Some(ref root) = doc.root {
            Self::flatten_element(root, &mut events)?;
        }

        try_push(&mut events, InfosetEvent::EndDocument)?;

        Ok(Self {
            events,
            position: 0,
        })
    }

    fn flatten_element(elem: &InfosetElement, events: &mut Vec<InfosetEvent>) -> DFDLResult<()> {
        match elem.state {
            ElementState::Value(ref val) => {
                try_push(
                    events,
                    InfosetEvent::SimpleValue {
                        name: elem.name.clone(),
                        value: val.clone(),
                    },
                )?;
            }
            ElementState::Empty => {
                try_push(
                    events,
                    InfosetEvent::EmptyValue {
                        name: elem.name.clone(),
                    },
                )?;
            }
            ElementState::Nil => {
                try_push(
                    events,
                    InfosetEvent::NilValue {
                        name: elem.name.clone(),
                    },
                )?;
            }
            ElementState::NoValue | ElementState::Absent => {
                try_push(
                    events,
                    InfosetEvent::StartElement {
                        name: elem.name.clone(),
                        is_nil: false,
                    },
                )?;
                for child in &elem.children {
                    match child {
                        InfosetNode::Element(ref sub) => Self::flatten_element(sub, events)?,
                    }
                }
                try_push(
                    events,
                    InfosetEvent::EndElement {
                        name: elem.name.clone(),
                    },
                )?;
            }
        }
        Ok(())
    }
}

impl InfosetSource for InfosetTreeSource {
    fn next_event(&mut self) -> DFDLResult<Option<InfosetEvent>> {
        if self.position < self.events.len() {
            let ev = self.events.get(self.position).cloned().ok_or_else(|| {
                DFDLError::new(DFDLErrorKind::Parse, "Infoset event index out of bounds")
            })?;
            self.position = self.position.saturating_add(1);
            Ok(Some(ev))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infoset::value::DfdlValue;

    #[test]
    fn test_infoset_tree_building_and_event_roundtrip() {
        let qn_root = QName::local("root");
        let qn_item = QName::local("item");

        let mut builder = InfosetBuilder::with_limits(ResourceLimits::default());
        builder.push_event(InfosetEvent::StartDocument).unwrap();
        builder
            .push_event(InfosetEvent::StartElement {
                name: qn_root.clone(),
                is_nil: false,
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::SimpleValue {
                name: qn_item.clone(),
                value: DfdlValue::Int(42),
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::EndElement {
                name: qn_root.clone(),
            })
            .unwrap();
        builder.push_event(InfosetEvent::EndDocument).unwrap();

        let doc = builder.build().unwrap();
        assert_eq!(doc.total_nodes, 2);

        let mut source = InfosetTreeSource::from_document(&doc).unwrap();
        assert_eq!(
            source.next_event().unwrap(),
            Some(InfosetEvent::StartDocument)
        );
        assert!(matches!(
            source.next_event().unwrap(),
            Some(InfosetEvent::StartElement { .. })
        ));
        assert!(matches!(
            source.next_event().unwrap(),
            Some(InfosetEvent::SimpleValue { .. })
        ));
        assert!(matches!(
            source.next_event().unwrap(),
            Some(InfosetEvent::EndElement { .. })
        ));
        assert_eq!(
            source.next_event().unwrap(),
            Some(InfosetEvent::EndDocument)
        );
    }

    #[test]
    fn test_active_doc_stack_hierarchy_resolution() {
        use crate::infoset::events::InfosetEventSink;
        let qn_root = QName::local("root");
        let qn_table = QName::local("table");
        let qn_x = QName::local("_x");

        let mut builder = InfosetBuilder::new();
        builder
            .push_event(InfosetEvent::StartElement {
                name: qn_root.clone(),
                is_nil: false,
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::StartElement {
                name: qn_table.clone(),
                is_nil: false,
            })
            .unwrap();
        builder
            .push_event(InfosetEvent::SimpleValue {
                name: qn_x.clone(),
                value: DfdlValue::Int(100),
            })
            .unwrap();

        // While root and table are still open on stack, active_doc must reflect full hierarchy
        let active_doc = builder.active_doc();
        let path = InfosetPath::parse("/root/table/_x");
        let found = active_doc.find_element(&path);
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().state,
            crate::infoset::state::ElementState::Value(DfdlValue::Int(100))
        );

        let path_dot = InfosetPath::parse("/./root/table/_x");
        let found_dot = active_doc.find_element(&path_dot);
        assert!(found_dot.is_some());

        let path_dotdot = InfosetPath::parse("/../root/table/_x");
        let found_dotdot = active_doc.find_element(&path_dotdot);
        assert!(found_dotdot.is_some());
    }
}
