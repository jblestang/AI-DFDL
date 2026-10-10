//! Owned in-memory DFDL Infoset tree representation (`alloc`).
//!
//! Provides [`InfosetDocument`], [`InfosetElement`], and [`InfosetNode`] structures for owned DOM traversal and verification.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::events::{InfosetEvent, InfosetEventSink, InfosetSource};
use crate::infoset::state::ElementState;
use crate::limits::ResourceLimits;
use crate::types::{InfosetPath, QName, StepTarget};
use crate::util::try_push;

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
        let steps = path.steps();
        if steps.is_empty() {
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
            if let Some(first_step) = steps.first() {
                if first_step.local_name() == clean_root_name {
                    idx = 1;
                }
            }
        }

        while idx < steps.len() {
            let step = match steps.get(idx) {
                Some(s) => s,
                None => break,
            };

            if step.is_self() {
                idx = idx.saturating_add(1);
                continue;
            }

            if step.is_parent() {
                if stack.len() > 1 {
                    let _ = stack.pop();
                }
                idx = idx.saturating_add(1);
                continue;
            }

            let curr = match stack.last() {
                Some(c) => *c,
                None => break,
            };

            let default_ns = in_scope_namespaces
                .iter()
                .find(|(p, _)| p.is_empty())
                .map(|(_, u)| u.as_str());

            let mut matches = Vec::new();
            let clean_step = step.local_name();

            for child in &curr.children {
                match child {
                    InfosetNode::Element(ref elem) => {
                        let clean_elem = elem
                            .name
                            .local_name
                            .split(':')
                            .next_back()
                            .unwrap_or(&elem.name.local_name);

                        match &step.target {
                            StepTarget::Wildcard => {
                                let _ = try_push(&mut matches, elem);
                            }
                            StepTarget::Clark { uri, local, .. } => {
                                if clean_elem == local.as_str()
                                    && elem.name.namespace.as_deref() == Some(uri.as_str())
                                {
                                    let _ = try_push(&mut matches, elem);
                                }
                            }
                            StepTarget::Prefixed { prefix, local, .. } => {
                                if clean_elem == local.as_str()
                                    && (elem.name.namespace.is_some() || elem.name.prefix.is_some())
                                {
                                    if let Some((_, uri)) = in_scope_namespaces.iter().find(|(p, _)| p == prefix) {
                                        if elem.name.namespace.as_deref() == Some(uri.as_str()) {
                                            let _ = try_push(&mut matches, elem);
                                        }
                                    } else if elem.name.prefix.as_deref() == Some(prefix.as_str()) {
                                        let _ = try_push(&mut matches, elem);
                                    }
                                }
                            }
                            StepTarget::Unprefixed(unpref_name) => {
                                if clean_elem == unpref_name.as_str() {
                                    // Unqualified path step: apply UnqualifiedPathStepPolicy
                                    match policy {
                                        crate::types::UnqualifiedPathStepPolicy::NoNamespace => {
                                            if elem.name.namespace.is_none() && elem.name.prefix.is_none() {
                                                let _ = try_push(&mut matches, elem);
                                            }
                                        }
                                        crate::types::UnqualifiedPathStepPolicy::DefaultNamespace => {
                                            if let Some(def_uri) = default_ns {
                                                if elem.name.namespace.as_deref() == Some(def_uri) {
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
                                                        c_name == unpref_name.as_str() && e.name.namespace.as_deref() == Some(def_uri)
                                                    }
                                                });
                                                if has_default_ns_child {
                                                    if elem.name.namespace.as_deref() == Some(def_uri) {
                                                        let _ = try_push(&mut matches, elem);
                                                    }
                                                } else {
                                                    let has_no_ns_child = curr.children.iter().any(|c| match c {
                                                        InfosetNode::Element(e) => {
                                                            let c_name = e.name.local_name.split(':').next_back().unwrap_or(&e.name.local_name);
                                                            c_name == unpref_name.as_str() && e.name.namespace.is_none() && e.name.prefix.is_none()
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
                                                        c_name == unpref_name.as_str() && e.name.namespace.is_none() && e.name.prefix.is_none()
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
                            StepTarget::SelfNode | StepTarget::ParentNode => {}
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
            let is_last_step = idx == steps.len().saturating_sub(1);
            let mut explicit_index = None;
            if let Some(idx_usize) = step.index_predicate {
                if idx_usize == 0 || idx_usize > matches.len() {
                    let msg = alloc::format!(
                        "Schema Definition Error: expression evaluation error: Value {} is out of range with length {}",
                        idx_usize,
                        matches.len()
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                explicit_index = Some(idx_usize);
            } else if let Some(ref pred_str) = step.predicate_expr {
                let trimmed = pred_str.trim();
                if let Ok(idx_i64) = trimmed.parse::<i64>() {
                    if idx_i64 <= 0 || (idx_i64 as usize) > matches.len() {
                        let msg = alloc::format!(
                            "Schema Definition Error: expression evaluation error: Value {} is out of range with length {}",
                            idx_i64,
                            matches.len()
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    explicit_index = Some(idx_i64 as usize);
                }
            }

            let is_self_instance = is_self_ref && is_last_step && occurs_index > 0;
            if matches.len() > 1 && explicit_index.is_none() && !is_self_instance {
                let qname_desc = if let Some(first_match) = matches.first() {
                    if let Some(ref ns) = first_match.name.namespace {
                        if let Some(ref pfx) = first_match.name.prefix {
                            alloc::format!("{}:{{{}}}{}", pfx, ns.as_str(), clean_step)
                        } else if let Some(pfx) = step.prefix() {
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
                    matches.get(exp_idx.saturating_sub(1)).copied().unwrap_or(curr)
                } else {
                    return Ok(None);
                }
            } else if is_last_step {
                if occurs_index > 0 && occurs_index <= matches.len() {
                    matches.get(occurs_index.saturating_sub(1)).copied().unwrap_or(curr)
                } else {
                    matches.first().copied().unwrap_or(curr)
                }
            } else {
                matches.last().copied().unwrap_or(curr)
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
        let Some(last) = self.stack.last() else {
            return InfosetDocument::new();
        };
        let mut curr = last.clone();
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
            let seg = if let Some(parent) = i.checked_sub(1).and_then(|idx| self.stack.get(idx)) {
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
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
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

    #[test]
    fn test_infoset_tree_extended_coverage() {
        use crate::types::UnqualifiedPathStepPolicy;

        // 1. strip_hidden
        let mut builder = InfosetBuilder::new();
        builder.push_event_with_hidden(InfosetEvent::StartElement {
            name: QName::local("root"),
            is_nil: false,
        }, false).unwrap();
        builder.push_event_with_hidden(InfosetEvent::SimpleValue {
            name: QName::local("visible"),
            value: DfdlValue::Int(1),
        }, false).unwrap();
        builder.push_event_with_hidden(InfosetEvent::SimpleValue {
            name: QName::local("secret"),
            value: DfdlValue::Int(99),
        }, true).unwrap();
        builder.push_event_with_hidden(InfosetEvent::EndElement {
            name: QName::local("root"),
        }, false).unwrap();

        let doc = builder.build().unwrap();
        assert_eq!(doc.total_nodes, 3);
        let stripped = doc.strip_hidden();
        assert_eq!(stripped.total_nodes, 2);
        assert!(stripped.find_element(&InfosetPath::parse("/root/visible")).is_some());
        assert!(stripped.find_element(&InfosetPath::parse("/root/secret")).is_none());

        // 2. checkpoint and rollback
        let mut b2 = InfosetBuilder::new();
        b2.push_event(InfosetEvent::StartElement {
            name: QName::local("root"),
            is_nil: false,
        }).unwrap();
        let cp = b2.checkpoint();
        b2.push_event(InfosetEvent::SimpleValue {
            name: QName::local("tmp"),
            value: DfdlValue::String("temp".into()),
        }).unwrap();
        assert_eq!(b2.current_child_count(), 1);
        b2.rollback(cp);
        assert_eq!(b2.current_child_count(), 0);

        // 3. current_path, count_child_occurrences, and reordering
        b2.push_event(InfosetEvent::SimpleValue {
            name: QName::local("c"),
            value: DfdlValue::Int(3),
        }).unwrap();
        b2.push_event(InfosetEvent::SimpleValue {
            name: QName::local("a"),
            value: DfdlValue::Int(1),
        }).unwrap();
        b2.push_event(InfosetEvent::SimpleValue {
            name: QName::local("b"),
            value: DfdlValue::Int(2),
        }).unwrap();

        assert_eq!(b2.count_child_occurrences("a"), 1);
        assert_eq!(b2.count_child_occurrences("c"), 1);
        assert_eq!(b2.count_child_occurrences("missing"), 0);
        assert_eq!(b2.current_child_count(), 3);

        b2.reorder_children_from(0, &["a", "b", "c"]);
        let root_elem = b2.stack.first().unwrap();
        let first_child_name = match &root_elem.children[0] {
            InfosetNode::Element(e) => &e.name.local_name,
        };
        assert_eq!(first_child_name, "a");

        // 4. Policy checked path search: wildcard, clark notation, qualified, and policies
        let mut b_policy = InfosetBuilder::new();
        b_policy.push_event(InfosetEvent::StartElement {
            name: QName::local("root"),
            is_nil: false,
        }).unwrap();
        b_policy.push_event(InfosetEvent::SimpleValue {
            name: QName::with_namespace("http://example.com/ns", "item", Some("ex")),
            value: DfdlValue::String("namespaced".into()),
        }).unwrap();
        b_policy.push_event(InfosetEvent::SimpleValue {
            name: QName::local("plain"),
            value: DfdlValue::String("plain".into()),
        }).unwrap();
        b_policy.push_event(InfosetEvent::NilValue {
            name: QName::local("nildata"),
        }).unwrap();
        b_policy.push_event(InfosetEvent::EndElement {
            name: QName::local("root"),
        }).unwrap();

        let doc_policy = b_policy.build().unwrap();

        // Wildcard
        let res_wild = doc_policy.find_element_with_policy_checked(
            &InfosetPath::parse("/root/*"),
            1,
            false,
            UnqualifiedPathStepPolicy::NoNamespace,
            &[],
        );
        assert!(res_wild.is_ok());

        // Clark notation
        let clark_path = InfosetPath::parse("/root/{http://example.com/ns}item");
        let res_clark = doc_policy.find_element_with_policy_checked(
            &clark_path,
            1,
            false,
            UnqualifiedPathStepPolicy::NoNamespace,
            &[],
        );
        assert!(res_clark.unwrap().is_some());

        // Qualified step with in_scope_namespaces
        let ns_pair = (alloc::string::String::from("ex"), alloc::string::String::from("http://example.com/ns"));
        let q_path = InfosetPath::parse("/root/ex:item");
        let res_q = doc_policy.find_element_with_policy_checked(
            &q_path,
            1,
            false,
            UnqualifiedPathStepPolicy::NoNamespace,
            &[ns_pair],
        );
        assert!(res_q.unwrap().is_some());

        // Policies: DefaultNamespace and PreferDefaultNamespace
        let plain_path = InfosetPath::parse("/root/plain");
        let res_def = doc_policy.find_element_with_policy_checked(
            &plain_path,
            1,
            false,
            UnqualifiedPathStepPolicy::DefaultNamespace,
            &[],
        );
        assert!(res_def.unwrap().is_some());

        let res_pref = doc_policy.find_element_with_policy_checked(
            &plain_path,
            1,
            false,
            UnqualifiedPathStepPolicy::PreferDefaultNamespace,
            &[],
        );
        assert!(res_pref.unwrap().is_some());

        // InfosetTreeSource flattening NilValue
        let mut src = InfosetTreeSource::from_document(&doc_policy).unwrap();
        let mut found_nil = false;
        while let Some(ev) = src.next_event().unwrap() {
            if matches!(ev, InfosetEvent::NilValue { .. }) {
                found_nil = true;
            }
        }
        assert!(found_nil);

        // Root EmptyValue and NilValue
        let mut b_empty_root = InfosetBuilder::new();
        b_empty_root.push_event(InfosetEvent::EmptyValue { name: QName::local("emptyRoot") }).unwrap();
        let doc_empty = b_empty_root.build().unwrap();
        assert_eq!(doc_empty.total_nodes, 1);

        let mut b_nil_root = InfosetBuilder::new();
        b_nil_root.push_event(InfosetEvent::NilValue { name: QName::local("nilRoot") }).unwrap();
        let doc_nil = b_nil_root.build().unwrap();
        assert_eq!(doc_nil.total_nodes, 1);

        // InfosetTreeSource flattening EmptyValue
        let mut src_empty = InfosetTreeSource::from_document(&doc_empty).unwrap();
        let mut found_empty = false;
        while let Some(ev) = src_empty.next_event().unwrap() {
            if matches!(ev, InfosetEvent::EmptyValue { .. }) {
                found_empty = true;
            }
        }
        assert!(found_empty);

        // 1. Clark notation and wildcard path lookup in find_element (lines 250-261)
        let mut root_elem = InfosetElement::complex(QName::local("root"));
        let child1 = InfosetElement::simple(QName::with_namespace("http://example.com", "child", Some("ex")), ElementState::Value(DfdlValue::String("c1".into())));
        let child2 = InfosetElement::simple(QName::with_namespace("http://example.com", "child", Some("ex")), ElementState::Value(DfdlValue::String("c2".into())));
        root_elem.try_add_child(InfosetNode::Element(child1)).unwrap();
        root_elem.try_add_child(InfosetNode::Element(child2)).unwrap();
        let doc_clark = InfosetDocument { root: Some(root_elem), total_nodes: 3 };

        let path_clark = InfosetPath::from_parts(alloc::vec!["root".into(), "{http://example.com}child[1]".into()], true);
        let found_clark = doc_clark.find_element_with_policy_checked(&path_clark, 1, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(found_clark.is_some());

        let path_wild = InfosetPath::from_parts(alloc::vec!["root".into(), "*[1]".into()], true);
        let found_wild = doc_clark.find_element_with_policy_checked(&path_wild, 1, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(found_wild.is_some());

        // 2. Ambiguous query-style path without predicate returns SchemaDefinition error (lines 371-394)
        let path_ambig = InfosetPath::from_parts(alloc::vec!["root".into(), "{http://example.com}child".into()], true);
        let err_ambig = doc_clark.find_element_with_policy_checked(&path_ambig, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap_err();
        assert_eq!(err_ambig.kind, DFDLErrorKind::SchemaDefinition);

        // 3. InfosetBuilder empty build and unclosed stack build (lines 662-668)
        let b_empty = InfosetBuilder::new();
        let doc_empty_b = b_empty.build().unwrap();
        assert!(doc_empty_b.root.is_none());

        let mut b_unclosed = InfosetBuilder::new();
        b_unclosed.push_event(InfosetEvent::StartElement { name: QName::local("unclosed"), is_nil: false }).unwrap();
        let doc_unclosed = b_unclosed.build().unwrap();
        assert!(doc_unclosed.root.is_some());

        // 4. EndElement error branches (empty stack & mismatched name) (lines 701-708)
        let mut b_orphan_end = InfosetBuilder::new();
        assert!(b_orphan_end.push_event(InfosetEvent::EndElement { name: QName::local("orphan") }).is_err());

        let mut b_mismatch_end = InfosetBuilder::new();
        b_mismatch_end.push_event(InfosetEvent::StartElement { name: QName::local("actual"), is_nil: false }).unwrap();
        assert!(b_mismatch_end.push_event(InfosetEvent::EndElement { name: QName::local("different") }).is_err());

        // 5. InfosetTreeSource flattening nested complex elements (lines 819-830)
        let mut src_nested = InfosetTreeSource::from_document(&doc_clark).unwrap();
        let mut ev_count = 0;
        while src_nested.next_event().unwrap().is_some() {
            ev_count += 1;
        }
        assert!(ev_count >= 5);

        // 6. Default implementations, active_doc, and builder inspection (lines 96-98, 472-474, 512-535)
        let default_doc = InfosetDocument::default();
        assert_eq!(default_doc.total_nodes, 0);

        let mut default_builder = InfosetBuilder::default();
        assert!(default_builder.document().root.is_none());
        assert_eq!(default_builder.active_doc().total_nodes, 0);

        default_builder.push_event(InfosetEvent::StartElement { name: QName::local("active_root"), is_nil: false }).unwrap();
        let active_doc = default_builder.active_doc();
        assert!(active_doc.root.is_some());

        // 7. current_child_count and reorder_children_from edge branches (lines 618-637)
        let mut b_count = InfosetBuilder::new();
        assert_eq!(b_count.current_child_count(), 0);
        b_count.reorder_children_from(5, &["a", "b"]);

        b_count.push_event(InfosetEvent::SimpleValue { name: QName::local("root_val"), value: DfdlValue::Int(1) }).unwrap();
        assert_eq!(b_count.current_child_count(), 0);
        b_count.reorder_children_from(0, &["root_val"]);

        // 8. Nesting limit exhaustion and EmptyValue in parent element (lines 684, 733)
        let mut b_limits = InfosetBuilder::with_limits(ResourceLimits { max_nesting_depth: 1, ..Default::default() });
        b_limits.push_event(InfosetEvent::StartElement { name: QName::local("l1"), is_nil: false }).unwrap();
        assert!(b_limits.push_event(InfosetEvent::StartElement { name: QName::local("l2"), is_nil: false }).is_err());

        let mut b_empty_val = InfosetBuilder::new();
        b_empty_val.push_event(InfosetEvent::StartElement { name: QName::local("parent"), is_nil: false }).unwrap();
        b_empty_val.push_event(InfosetEvent::EmptyValue { name: QName::local("empty_child") }).unwrap();
        b_empty_val.push_event(InfosetEvent::EndElement { name: QName::local("parent") }).unwrap();
        let doc_empty_val = b_empty_val.build().unwrap();
        assert!(doc_empty_val.root.is_some());

        // 9. InfosetTreeSource with EmptyValue and NilValue elements (lines 797-810)
        let mut doc_nil_empty = InfosetDocument::new();
        let mut root_nil = InfosetElement::complex(QName::local("root"));
        root_nil.try_add_child(InfosetNode::Element(InfosetElement::simple(QName::local("e1"), ElementState::Empty))).unwrap();
        root_nil.try_add_child(InfosetNode::Element(InfosetElement::simple(QName::local("e2"), ElementState::Nil))).unwrap();
        doc_nil_empty.root = Some(root_nil);
        let mut src_ne = InfosetTreeSource::from_document(&doc_nil_empty).unwrap();
        let mut ne_events = alloc::vec![];
        while let Some(ev) = src_ne.next_event().unwrap() {
            ne_events.push(ev);
        }
        assert!(ne_events.len() >= 6);

        // 10. Explicit index out of bounds in find_element (line 405)
        let path_oob = InfosetPath::from_parts(alloc::vec!["root".into(), "child[99]".into()], true);
        let res_oob = doc_clark.find_element_with_policy_checked(&path_oob, 1, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(res_oob.is_none());
    }

    /// Tests remaining edge cases in tree navigation, occurrence counting,
    /// ambiguous path error formatting, and child reordering.
    ///
    /// Verifies that:
    /// 1. `count_child_occurrences` returns 0 when the root name does not match the target.
    /// 2. `current_path` correctly appends index suffixes for multiple sibling elements.
    /// 3. Ambiguous path step errors properly format prefix and namespace variants.
    /// 4. Path traversal over nonexistent intermediate or leaf steps returns `Ok(None)`.
    /// 5. Child reordering safely returns when `start_index` exceeds child count.
    #[test]
    fn test_tree_extended_edge_coverage() {
        // 1. count_child_occurrences on doc root non-match (line 606)
        let mut b_occ = InfosetBuilder::new();
        b_occ.push_event(InfosetEvent::SimpleValue { name: QName::local("root_elem"), value: DfdlValue::Int(42) }).unwrap();
        assert_eq!(b_occ.count_child_occurrences("non_matching"), 0);
        assert_eq!(b_occ.count_child_occurrences("root_elem"), 1);

        // 2. current_path with multiple siblings in stack (line 559)
        let mut b_path = InfosetBuilder::new();
        b_path.push_event(InfosetEvent::StartElement { name: QName::local("root"), is_nil: false }).unwrap();
        b_path.push_event(InfosetEvent::SimpleValue { name: QName::local("item"), value: DfdlValue::Int(1) }).unwrap();
        b_path.push_event(InfosetEvent::StartElement { name: QName::local("item"), is_nil: false }).unwrap();
        let path = b_path.current_path();
        assert!(path.segments().iter().any(|s| s.contains("item[2]")));

        // 3. Ambiguous path errors with namespace and prefix combinations (lines 375-380)
        let mut doc_ambig = InfosetDocument::new();
        let mut root_ambig = InfosetElement::complex(QName::local("root"));
        // First match has prefix and namespace
        let e1 = InfosetElement::simple(QName::with_namespace("urn:test", "multi", Some("pfx")), ElementState::Value(DfdlValue::Int(1)));
        let e2 = InfosetElement::simple(QName::with_namespace("urn:test", "multi", Some("pfx")), ElementState::Value(DfdlValue::Int(2)));
        root_ambig.try_add_child(InfosetNode::Element(e1)).unwrap();
        root_ambig.try_add_child(InfosetNode::Element(e2)).unwrap();
        doc_ambig.root = Some(root_ambig);

        let p_ambig = InfosetPath::from_parts(alloc::vec!["root".into(), "{urn:test}multi".into()], true);
        let err_ambig1 = doc_ambig.find_element_with_policy_checked(&p_ambig, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap_err();
        assert!(err_ambig1.message.as_str().contains("pfx:{urn:test}"));

        // Match with namespace but no prefix (line 379)
        let mut doc_ambig2 = InfosetDocument::new();
        let mut root_ambig2 = InfosetElement::complex(QName::local("root"));
        let e3 = InfosetElement::simple(QName::with_namespace("urn:test2", "multi2", None), ElementState::Value(DfdlValue::Int(1)));
        let e4 = InfosetElement::simple(QName::with_namespace("urn:test2", "multi2", None), ElementState::Value(DfdlValue::Int(2)));
        root_ambig2.try_add_child(InfosetNode::Element(e3)).unwrap();
        root_ambig2.try_add_child(InfosetNode::Element(e4)).unwrap();
        doc_ambig2.root = Some(root_ambig2);

        let p_ambig2 = InfosetPath::from_parts(alloc::vec!["root".into(), "{urn:test2}multi2".into()], true);
        let err_ambig2 = doc_ambig2.find_element_with_policy_checked(&p_ambig2, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap_err();
        assert!(err_ambig2.message.as_str().contains("{urn:test2}"));

        // Match with prefix in step but no prefix on element (line 377)
        let p_ambig3 = InfosetPath::from_parts(alloc::vec!["root".into(), "ns:multi2".into()], true);
        let ns_scope = [(alloc::string::String::from("ns"), alloc::string::String::from("urn:test2"))];
        let err_ambig3 = doc_ambig2.find_element_with_policy_checked(&p_ambig3, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &ns_scope).unwrap_err();
        assert!(err_ambig3.message.as_str().contains("ns:{urn:test2}"));

        // 4. Missing intermediate and leaf step traversal (lines 418, 424)
        let p_nonexistent_leaf = InfosetPath::from_parts(alloc::vec!["root".into(), "does_not_exist".into()], true);
        let res_leaf = doc_ambig.find_element_with_policy_checked(&p_nonexistent_leaf, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(res_leaf.is_none());

        let p_nonexistent_mid = InfosetPath::from_parts(alloc::vec!["root".into(), "missing_parent".into(), "child".into()], true);
        let res_mid = doc_ambig.find_element_with_policy_checked(&p_nonexistent_mid, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(res_mid.is_none());

        // 5. reorder_children_from beyond child bounds (line 637)
        let mut b_reorder = InfosetBuilder::new();
        b_reorder.push_event(InfosetEvent::StartElement { name: QName::local("r"), is_nil: false }).unwrap();
        b_reorder.push_event(InfosetEvent::SimpleValue { name: QName::local("c"), value: DfdlValue::Int(1) }).unwrap();
        b_reorder.reorder_children_from(100, &["c"]);

        // 6. Explicit index within matches range (lines 397-404)
        // Verify indexing array elements by 1-based index returns the exact child
        let p_exp_idx = InfosetPath::from_parts(alloc::vec!["root".into(), "{urn:test}multi[1]".into()], true);
        let elem_exp = doc_ambig.find_element_with_policy_checked(&p_exp_idx, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(elem_exp.is_some());
        if let Some(el) = elem_exp {
            assert_eq!(el.state, ElementState::Value(DfdlValue::Int(1)));
        }

        // 7. Last step occurs_index resolution on multiple matches (lines 408-415)
        // Occurs index selects the matching occurrence in array context
        let p_self_ref = InfosetPath::from_parts(alloc::vec!["root".into(), "{urn:test}multi".into()], true);
        let elem_occurs = doc_ambig.find_element_with_policy_checked(&p_self_ref, 2, true, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(elem_occurs.is_some());
        if let Some(el) = elem_occurs {
            assert_eq!(el.state, ElementState::Value(DfdlValue::Int(2)));
        }

        // 8. EventStream flattening with Empty and Nil element states (lines 796-810)
        // Verify EventStream round-trips both empty element and nillable element variants
        let mut doc_states = InfosetDocument::new();
        let mut root_states = InfosetElement::complex(QName::local("r_states"));
        let e_empty = InfosetElement::simple(QName::local("e_empty"), ElementState::Empty);
        let e_nil = InfosetElement::simple(QName::local("e_nil"), ElementState::Nil);
        root_states.try_add_child(InfosetNode::Element(e_empty)).unwrap();
        root_states.try_add_child(InfosetNode::Element(e_nil)).unwrap();
        doc_states.root = Some(root_states);

        let mut stream = InfosetTreeSource::from_document(&doc_states).unwrap();
        let mut events = Vec::new();
        while let Some(ev) = stream.next_event().unwrap() {
            events.push(ev);
        }
        assert!(events.iter().any(|ev| matches!(ev, InfosetEvent::EmptyValue { .. })));
        assert!(events.iter().any(|ev| matches!(ev, InfosetEvent::NilValue { .. })));

        // 9. UnqualifiedPathStepPolicy coverage: DefaultNamespace and PreferDefaultNamespace
        // Exercises child resolution when default namespace is configured or absent
        let mut doc_policy = InfosetDocument::new();
        let mut root_policy = InfosetElement::complex(QName::local("r_pol"));
        let child_no_ns = InfosetElement::simple(QName::local("item"), ElementState::Value(DfdlValue::Int(10)));
        let child_with_ns = InfosetElement::simple(QName::with_namespace("urn:default", "item", None), ElementState::Value(DfdlValue::Int(20)));
        root_policy.try_add_child(InfosetNode::Element(child_no_ns)).unwrap();
        root_policy.try_add_child(InfosetNode::Element(child_with_ns)).unwrap();
        doc_policy.root = Some(root_policy);

        let p_item = InfosetPath::from_parts(alloc::vec!["r_pol".into(), "item".into()], true);
        // DefaultNamespace with no default uri configured matches child without namespace
        let found_no_ns = doc_policy.find_element_with_policy_checked(&p_item, 0, false, crate::types::UnqualifiedPathStepPolicy::DefaultNamespace, &[]).unwrap();
        assert!(found_no_ns.is_some());

        // PreferDefaultNamespace when default uri is present matches child with default namespace
        let ns_def = [(alloc::string::String::new(), alloc::string::String::from("urn:default"))];
        let found_def_ns = doc_policy.find_element_with_policy_checked(&p_item, 0, false, crate::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace, &ns_def).unwrap();
        assert!(found_def_ns.is_some());

        // 10. Clark notation path step: {urn:default}item
        let p_clark = InfosetPath::from_parts(alloc::vec!["r_pol".into(), "{urn:default}item".into()], true);
        let found_clark = doc_policy.find_element_with_policy_checked(&p_clark, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(found_clark.is_some());

        // 11. Qualified path step where prefix is on element itself (lines 270-274)
        let mut doc_pfx = InfosetDocument::new();
        let mut root_pfx = InfosetElement::complex(QName::local("r_pfx"));
        let child_pfx = InfosetElement::simple(QName::with_namespace("urn:my", "child", Some("my_pfx")), ElementState::Value(DfdlValue::Int(30)));
        root_pfx.try_add_child(InfosetNode::Element(child_pfx)).unwrap();
        doc_pfx.root = Some(root_pfx);
        let p_pfx = InfosetPath::from_parts(alloc::vec!["r_pfx".into(), "my_pfx:child".into()], true);
        let found_pfx = doc_pfx.find_element_with_policy_checked(&p_pfx, 0, false, crate::types::UnqualifiedPathStepPolicy::NoNamespace, &[]).unwrap();
        assert!(found_pfx.is_some());

        // 12. PreferDefaultNamespace fallback when no default_ns child and no no_ns child exists (lines 317-319)
        let mut doc_other = InfosetDocument::new();
        let mut root_other = InfosetElement::complex(QName::local("r_pol"));
        let child_other = InfosetElement::simple(QName::with_namespace("urn:other", "item", None), ElementState::Value(DfdlValue::Int(40)));
        root_other.try_add_child(InfosetNode::Element(child_other)).unwrap();
        doc_other.root = Some(root_other);
        let found_other = doc_other.find_element_with_policy_checked(&p_item, 0, false, crate::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace, &ns_def).unwrap();
        assert!(found_other.is_some());

        // 13. PreferDefaultNamespace without default uri fallback when child has namespace (lines 332-334)
        let found_other_no_def = doc_other.find_element_with_policy_checked(&p_item, 0, false, crate::types::UnqualifiedPathStepPolicy::PreferDefaultNamespace, &[]).unwrap();
        assert!(found_other_no_def.is_some());
    }
}
