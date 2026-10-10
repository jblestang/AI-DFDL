//! Core vocabulary types: QNames, Namespaces, SourceLocations, InfosetPaths, Offsets, and Internal IDs.
//!
//! Aligned with DFDL 1.0 §3.1 and Appendix E terminology.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};

/// Represents an XML namespace URI or identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace {
    uri: String,
}

impl Namespace {
    /// Creates a new namespace from a URI string slice.
    #[inline]
    pub fn new(uri: &str) -> Self {
        Self {
            uri: String::from(uri),
        }
    }

    /// Returns the URI string reference.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.uri
    }
}

impl core::ops::Deref for Namespace {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.uri
    }
}

impl AsRef<str> for Namespace {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.uri
    }
}

/// Qualified Name (QName) containing optional namespace, local name, and optional prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QName {
    /// Optional namespace URI.
    pub namespace: Option<Namespace>,
    /// Local part of the qualified name.
    pub local_name: String,
    /// Optional prefix used in XML declarations.
    pub prefix: Option<String>,
}

impl QName {
    /// Creates a QName with local name only and no namespace.
    #[inline]
    pub fn local(local_name: &str) -> Self {
        Self {
            namespace: None,
            local_name: String::from(local_name),
            prefix: None,
        }
    }

    /// Creates a QName with explicit namespace, local name, and optional prefix.
    #[inline]
    pub fn with_namespace(namespace: &str, local_name: &str, prefix: Option<&str>) -> Self {
        Self {
            namespace: Some(Namespace::new(namespace)),
            local_name: String::from(local_name),
            prefix: prefix.map(String::from),
        }
    }

    /// Parses a string into a QName, handling optional `prefix:local_name` formatting.
    pub fn parse_element_name(s: &str) -> Self {
        Self::parse(s)
    }

    /// Parses a string into a QName, handling Clark notation `{uri}local`, `prefix:local`, or unprefixed `local`.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if s.starts_with('{') {
            if let Some(end) = s.find('}') {
                let uri = &s[1..end];
                let local = &s[end.saturating_add(1)..];
                Self::with_namespace(uri, local, None)
            } else {
                Self::local(s)
            }
        } else if let Some((prefix, local)) = s.split_once(':') {
            Self {
                namespace: None,
                local_name: String::from(local),
                prefix: Some(String::from(prefix)),
            }
        } else {
            Self::local(s)
        }
    }

    /// Returns whether this QName matches another QName, matching on full namespace + local name if namespace is present,
    /// or matching local name if either QName has no namespace.
    #[must_use]
    pub fn matches(&self, other: &QName) -> bool {
        if self.local_name != other.local_name {
            return false;
        }
        match (&self.namespace, &other.namespace) {
            (Some(ns1), Some(ns2)) => ns1 == ns2,
            _ => true,
        }
    }

    /// Formats this QName in Clark notation: `{namespace}local_name` or `{}local_name`.
    #[inline]
    #[must_use]
    pub fn clark_notation(&self) -> String {
        match &self.namespace {
            Some(ns) => alloc::format!("{{{}}}{}", ns.as_str(), self.local_name),
            None => alloc::format!("{{}}{}", self.local_name),
        }
    }

    /// Formats this QName with its prefix if present: `prefix:local_name` or `local_name`.
    #[inline]
    #[must_use]
    pub fn prefixed_name(&self) -> String {
        if let Some(ref p) = self.prefix {
            alloc::format!("{}:{}", p, self.local_name)
        } else {
            self.local_name.clone()
        }
    }
}

impl fmt::Display for QName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{{{}}}:{}", ns.as_str(), self.local_name),
            None => write!(f, "{}", self.local_name),
        }
    }
}

/// Source location in a schema or input document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// 1-based line number if available.
    pub line: Option<usize>,
    /// 1-based column number if available.
    pub column: Option<usize>,
    /// 0-based byte offset in stream.
    pub byte_offset: usize,
}

impl SourceLocation {
    /// Creates a new [`SourceLocation`] with byte offset.
    #[inline]
    #[must_use]
    pub const fn at_offset(byte_offset: usize) -> Self {
        Self {
            line: None,
            column: None,
            byte_offset,
        }
    }

    /// Attaches 1-based line and column numbers.
    #[inline]
    #[must_use]
    pub const fn with_line_col(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

/// Path navigation axis for an XPath step (§23.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathAxis {
    /// Default child axis.
    #[default]
    Child,
    /// Parent axis (`parent::`).
    Parent,
    /// Self axis (`self::`).
    SelfAxis,
}

/// Target node in an XPath step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepTarget {
    /// Unprefixed element name: e.g. `item`.
    Unprefixed(String),
    /// Prefixed element name: e.g. `ex:item`.
    Prefixed {
        /// Element namespace prefix.
        prefix: String,
        /// Element local name.
        local: String,
        /// Full raw target string `prefix:local`.
        raw: String,
    },
    /// Clark notation element name: e.g. `{urn:test}item`.
    Clark {
        /// Target namespace URI.
        uri: String,
        /// Target local name.
        local: String,
        /// Full raw target string `{uri}local`.
        raw: String,
    },
    /// Wildcard `*`.
    Wildcard,
    /// Current element `.`.
    SelfNode,
    /// Parent element `..`.
    ParentNode,
}

impl StepTarget {
    /// Parses a raw target string into a structured [`StepTarget`].
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if s == "*" {
            Self::Wildcard
        } else if s == "." {
            Self::SelfNode
        } else if s == ".." {
            Self::ParentNode
        } else if s.starts_with('{') {
            if let Some(end) = s.find('}') {
                Self::Clark {
                    uri: alloc::string::ToString::to_string(&s[1..end]),
                    local: alloc::string::ToString::to_string(&s[end.saturating_add(1)..]),
                    raw: alloc::string::ToString::to_string(s),
                }
            } else {
                Self::Unprefixed(alloc::string::ToString::to_string(s))
            }
        } else if let Some((pfx, loc)) = s.split_once(':') {
            if !pfx.is_empty() && pfx != "." && pfx != ".." {
                Self::Prefixed {
                    prefix: alloc::string::ToString::to_string(pfx),
                    local: alloc::string::ToString::to_string(loc),
                    raw: alloc::string::ToString::to_string(s),
                }
            } else {
                Self::Unprefixed(alloc::string::ToString::to_string(s))
            }
        } else {
            Self::Unprefixed(alloc::string::ToString::to_string(s))
        }
    }
}

/// A structured step in an [`InfosetPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    /// Navigation axis (Child, Parent, SelfAxis).
    pub axis: PathAxis,
    /// Target node (Unprefixed, Prefixed, Clark, Wildcard, SelfNode, ParentNode).
    pub target: StepTarget,
    /// Optional index predicate (1-based index if static integer, e.g. `[1]`).
    pub index_predicate: Option<usize>,
    /// Optional dynamic predicate expression string if not a simple positive integer literal.
    pub predicate_expr: Option<String>,
}

impl PathStep {
    /// Creates a new `PathStep` with the specified axis, target, index predicate, and expression predicate.
    #[inline]
    #[must_use]
    pub fn new(
        axis: PathAxis,
        target: StepTarget,
        index_predicate: Option<usize>,
        predicate_expr: Option<String>,
    ) -> Self {
        Self {
            axis,
            target,
            index_predicate,
            predicate_expr,
        }
    }

    /// Creates a new named child step without predicates.
    #[inline]
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        let name_str = name.into();
        let target = StepTarget::parse(&name_str);
        Self {
            axis: PathAxis::Child,
            target,
            index_predicate: None,
            predicate_expr: None,
        }
    }

    /// Returns the raw step name / target as a string slice (e.g. "root", "ex:item", "*", ".", "..").
    #[inline]
    #[must_use]
    pub fn raw_target(&self) -> &str {
        match &self.target {
            StepTarget::Unprefixed(s) => s.as_str(),
            StepTarget::Prefixed { raw, .. } => raw.as_str(),
            StepTarget::Clark { raw, .. } => raw.as_str(),
            StepTarget::Wildcard => "*",
            StepTarget::SelfNode => ".",
            StepTarget::ParentNode => "..",
        }
    }

    /// Returns the local name without prefix or namespace (e.g. "item" from "ex:item" or "{uri}item").
    #[inline]
    #[must_use]
    pub fn local_name(&self) -> &str {
        match &self.target {
            StepTarget::Unprefixed(s) => s.as_str(),
            StepTarget::Prefixed { local, .. } => local.as_str(),
            StepTarget::Clark { local, .. } => local.as_str(),
            StepTarget::Wildcard => "*",
            StepTarget::SelfNode => ".",
            StepTarget::ParentNode => "..",
        }
    }

    /// Returns the prefix if present (e.g. "ex" from "ex:item").
    #[inline]
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        match &self.target {
            StepTarget::Prefixed { prefix, .. } => Some(prefix.as_str()),
            _ => None,
        }
    }

    /// Returns the Clark namespace URI if present (e.g. "urn:test" from "{urn:test}item").
    #[inline]
    #[must_use]
    pub fn clark_uri(&self) -> Option<&str> {
        match &self.target {
            StepTarget::Clark { uri, .. } => Some(uri.as_str()),
            _ => None,
        }
    }

    /// Returns whether this step represents navigating to the parent.
    #[inline]
    #[must_use]
    pub fn is_parent(&self) -> bool {
        self.axis == PathAxis::Parent || self.target == StepTarget::ParentNode
    }

    /// Returns whether this step represents navigating to self.
    #[inline]
    #[must_use]
    pub fn is_self(&self) -> bool {
        self.axis == PathAxis::SelfAxis || self.target == StepTarget::SelfNode
    }

    /// Parses a path segment string (such as `"item[1]"`, `".."` , `"."`, `"..(node)[1]"`) into a [`PathStep`].
    pub fn from_segment(seg: &str) -> Self {
        let (raw_step, pred_expr, index_pred) = if let (Some(b_open), Some(b_close)) = (seg.find('['), seg.rfind(']')) {
            if b_open < b_close {
                let s_head = &seg[..b_open];
                let s_pred = &seg[b_open.saturating_add(1)..b_close];
                let trimmed = s_pred.trim();
                let idx = trimmed.parse::<usize>().ok();
                (s_head, Some(alloc::string::ToString::to_string(trimmed)), idx)
            } else {
                (seg, None, None)
            }
        } else {
            (seg, None, None)
        };

        if raw_step == ".." {
            Self {
                axis: PathAxis::Parent,
                target: StepTarget::ParentNode,
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        } else if let Some(inner) = raw_step.strip_prefix("..(") {
            let target_name = inner.strip_suffix(')').unwrap_or(inner);
            Self {
                axis: PathAxis::Parent,
                target: StepTarget::parse(target_name),
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        } else if raw_step == "." {
            Self {
                axis: PathAxis::SelfAxis,
                target: StepTarget::SelfNode,
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        } else if let Some(inner) = raw_step.strip_prefix(".(") {
            let target_name = inner.strip_suffix(')').unwrap_or(inner);
            Self {
                axis: PathAxis::SelfAxis,
                target: StepTarget::parse(target_name),
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        } else if raw_step == "*" {
            Self {
                axis: PathAxis::Child,
                target: StepTarget::Wildcard,
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        } else {
            Self {
                axis: PathAxis::Child,
                target: StepTarget::parse(raw_step),
                index_predicate: index_pred,
                predicate_expr: pred_expr,
            }
        }
    }

    /// Formats this step into its canonical string segment.
    pub fn to_segment_string(&self) -> String {
        let pred_str = if let Some(ref p) = self.predicate_expr {
            alloc::format!("[{}]", p)
        } else if let Some(idx) = self.index_predicate {
            alloc::format!("[{}]", idx)
        } else {
            String::new()
        };
        let target_str = self.raw_target();
        match self.axis {
            PathAxis::Parent => {
                if target_str.is_empty() || target_str == ".." || target_str == "." {
                    alloc::format!("..{pred_str}")
                } else {
                    alloc::format!("..({target_str}){pred_str}")
                }
            }
            PathAxis::SelfAxis => {
                if target_str.is_empty() || target_str == "." {
                    alloc::format!(".{pred_str}")
                } else {
                    alloc::format!(".({target_str}){pred_str}")
                }
            }
            PathAxis::Child => {
                alloc::format!("{target_str}{pred_str}")
            }
        }
    }
}

/// Path navigating the DFDL Infoset structure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfosetPath {
    steps: Vec<PathStep>,
    segments: Vec<String>,
    is_absolute: bool,
}

impl InfosetPath {
    /// Creates a root Infoset path.
    #[inline]
    #[must_use]
    pub const fn root() -> Self {
        Self {
            steps: Vec::new(),
            segments: Vec::new(),
            is_absolute: true,
        }
    }

    /// Constructs an [`InfosetPath`] directly from segments and absolute flag.
    #[inline]
    #[must_use]
    pub fn from_parts(segments: Vec<String>, is_absolute: bool) -> Self {
        let steps = segments.iter().map(|s| PathStep::from_segment(s.as_str())).collect();
        Self {
            steps,
            segments,
            is_absolute,
        }
    }

    /// Constructs an [`InfosetPath`] directly from structured [`PathStep`]s and absolute flag.
    pub fn from_steps(steps: Vec<PathStep>, is_absolute: bool) -> Self {
        let segments = steps.iter().map(|s| s.to_segment_string()).collect();
        Self {
            steps,
            segments,
            is_absolute,
        }
    }

    /// Parses a path string (e.g. `/root/header/length` or `parent::ex:root/len`) into an [`InfosetPath`] using Pest grammar.
    pub fn parse(path_str: &str) -> Self {
        crate::expr::parser::parse_path(path_str)
            .unwrap_or_else(|_| Self::from_parts(Vec::new(), path_str.starts_with('/')))
    }

    /// Returns whether this path is an absolute path.
    #[inline]
    #[must_use]
    pub const fn is_absolute(&self) -> bool {
        self.is_absolute
    }

    /// Returns a slice of structured steps in the path.
    #[inline]
    #[must_use]
    pub fn steps(&self) -> &[PathStep] {
        &self.steps
    }

    /// Returns a slice of segments in the path.
    #[inline]
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Pushes a structured [`PathStep`] onto the path using fallible allocation.
    pub fn try_push_step(&mut self, step: PathStep) -> DFDLResult<()> {
        let seg = step.to_segment_string();
        self.steps.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.segments.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.steps.push(step);
        self.segments.push(seg);
        Ok(())
    }

    /// Pushes a segment onto the path using fallible allocation, stripping any QName prefix while preserving index predicates.
    pub fn try_push(&mut self, segment: &str) -> DFDLResult<()> {
        let clean = if let (Some(open), Some(close)) = (segment.find('['), segment.rfind(']')) {
            if open < close {
                let name_part = segment[..open].split(':').next_back().unwrap_or(&segment[..open]);
                let pred_part = &segment[open..=close];
                alloc::format!("{}{}", name_part, pred_part)
            } else {
                let clean_ns = segment.split(':').next_back().unwrap_or(segment);
                alloc::string::ToString::to_string(clean_ns)
            }
        } else {
            let clean_ns = segment.split(':').next_back().unwrap_or(segment);
            alloc::string::ToString::to_string(clean_ns)
        };
        let step = PathStep::from_segment(&clean);
        self.steps.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.segments.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.steps.push(step);
        self.segments.push(clean);
        Ok(())
    }

    /// Pushes a segment verbatim onto the path using fallible allocation, preserving QName prefixes.
    pub fn try_push_raw(&mut self, segment: &str) -> DFDLResult<()> {
        let step = PathStep::from_segment(segment);
        self.steps.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.segments.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.steps.push(step);
        self.segments.push(alloc::string::ToString::to_string(segment));
        Ok(())
    }

    /// Pops the last segment from the path.
    pub fn pop(&mut self) -> Option<String> {
        self.steps.pop();
        self.segments.pop()
    }

    /// Pops the last structured step from the path.
    pub fn pop_step(&mut self) -> Option<PathStep> {
        self.segments.pop();
        self.steps.pop()
    }

    /// Returns the last structured step in the path if present.
    #[inline]
    #[must_use]
    pub fn last_step(&self) -> Option<&PathStep> {
        self.steps.last()
    }

    /// Returns whether this path represents the current node only (empty or `.` step).
    #[inline]
    #[must_use]
    pub fn is_self_only(&self) -> bool {
        self.steps.is_empty() || (self.steps.len() == 1 && self.steps.first().is_some_and(|s| s.is_self()))
    }

    /// Returns a parent path by removing the last step, or `None` if already empty/root.
    #[must_use]
    pub fn parent_path(&self) -> Option<Self> {
        if self.steps.is_empty() {
            None
        } else {
            let mut p = self.clone();
            p.pop_step();
            Some(p)
        }
    }

    /// Normalizes this relative or absolute path against a base path using structured steps.
    #[must_use]
    pub fn normalized_against(&self, base: &InfosetPath) -> Self {
        let mut norm = if self.is_absolute {
            Self::root()
        } else {
            base.clone()
        };
        for step in &self.steps {
            if step.is_self() {
                continue;
            } else if step.is_parent() {
                if !norm.steps.is_empty() {
                    let _ = norm.pop_step();
                }
            } else {
                let _ = norm.try_push_step(step.clone());
            }
        }
        norm
    }
}

impl fmt::Display for InfosetPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            write!(f, "/")
        } else {
            for seg in &self.segments {
                write!(f, "/{}", seg)?;
            }
            Ok(())
        }
    }
}

/// Strongly-typed bit offset with panic-free checked arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BitOffset(pub usize);

impl BitOffset {
    /// Checked addition of bit offsets.
    #[inline]
    #[must_use]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(val) => Some(Self(val)),
            None => None,
        }
    }

    /// Checked subtraction of bit offsets.
    #[inline]
    #[must_use]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(val) => Some(Self(val)),
            None => None,
        }
    }

    /// Converts bit offset to whole byte offset if byte-aligned.
    #[inline]
    #[must_use]
    pub const fn to_byte_offset(self) -> Option<ByteOffset> {
        if self.0.is_multiple_of(8) {
            Some(ByteOffset(self.0 / 8))
        } else {
            None
        }
    }
}

/// Strongly-typed byte offset with panic-free checked arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteOffset(pub usize);

impl ByteOffset {
    /// Checked addition of byte offsets.
    #[inline]
    #[must_use]
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(val) => Some(Self(val)),
            None => None,
        }
    }

    /// Checked subtraction of byte offsets.
    #[inline]
    #[must_use]
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(val) => Some(Self(val)),
            None => None,
        }
    }

    /// Converts byte offset to bit offset.
    #[inline]
    #[must_use]
    pub const fn to_bit_offset(self) -> Option<BitOffset> {
        match self.0.checked_mul(8) {
            Some(bits) => Some(BitOffset(bits)),
            None => None,
        }
    }
}

/// Stable internal symbol identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub u32);

/// Policy for resolving unqualified path steps in DFDL expressions (§23).
///
/// In DFDL expressions, a path step may omit an XML namespace prefix.
/// This policy governs how unqualified step names are bound to elements
/// in the Infoset tree:
/// - `NoNamespace`: An unqualified step only matches elements that reside in no namespace.
/// - `DefaultNamespace`: An unqualified step matches elements in the default namespace (or no namespace if none declared).
/// - `PreferDefaultNamespace`: An unqualified step first attempts to match in the default namespace,
///   falling back to no namespace if not found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum UnqualifiedPathStepPolicy {
    /// Matches only elements defined without a namespace (default Daffodil policy).
    #[default]
    NoNamespace,
    /// Matches elements defined in the default namespace (`xmlns="..."`).
    DefaultNamespace,
    /// Matches elements in the default namespace, falling back to no namespace.
    PreferDefaultNamespace,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_qname_display() {
        let qn_local = QName::local("element");
        assert_eq!(qn_local.to_string(), "element");

        let qn_ns = QName::with_namespace("http://example.com", "element", Some("ex"));
        assert_eq!(qn_ns.to_string(), "{http://example.com}:element");
    }

    #[test]
    fn test_qname_clark_notation() {
        let qn_local = QName::local("elem");
        assert_eq!(qn_local.clark_notation(), "{}elem");

        let qn_ns = QName::with_namespace("http://example.com", "elem", Some("ex"));
        assert_eq!(qn_ns.clark_notation(), "{http://example.com}elem");
    }

    #[test]
    fn test_bit_byte_offset_conversions() {
        let byte_off = ByteOffset(10);
        let bit_off = byte_off.to_bit_offset().unwrap();
        assert_eq!(bit_off, BitOffset(80));
        assert_eq!(bit_off.to_byte_offset(), Some(ByteOffset(10)));

        let unaligned_bit = BitOffset(85);
        assert_eq!(unaligned_bit.to_byte_offset(), None);
    }

    #[test]
    fn test_infoset_path() {
        let mut path = InfosetPath::root();
        assert_eq!(path.to_string(), "/");
        path.try_push("root").unwrap();
        path.try_push("header").unwrap();
        assert_eq!(path.to_string(), "/root/header");
        assert_eq!(path.pop(), Some(String::from("header")));
        assert_eq!(path.to_string(), "/root");
    }

    #[test]
    fn test_unqualified_path_step_policy_defaults() {
        let default_policy = UnqualifiedPathStepPolicy::default();
        assert_eq!(default_policy, UnqualifiedPathStepPolicy::NoNamespace);
        assert_ne!(default_policy, UnqualifiedPathStepPolicy::DefaultNamespace);
        assert_ne!(default_policy, UnqualifiedPathStepPolicy::PreferDefaultNamespace);
    }

    /// Verifies QName parsing, SourceLocation, InfosetPath predicates, and offset checked math.
    #[test]
    fn test_types_extended_coverage() {
        // Namespace as_str
        let ns = Namespace::new("urn:test");
        assert_eq!(ns.as_str(), "urn:test");

        // QName parse_element_name and prefixed_name
        let qn_prefixed = QName::parse_element_name("pfx:elem");
        assert_eq!(qn_prefixed.local_name, "elem");
        assert_eq!(qn_prefixed.prefix.as_deref(), Some("pfx"));
        assert_eq!(qn_prefixed.prefixed_name(), "pfx:elem");

        let qn_noprefix = QName::parse_element_name("elemOnly");
        assert_eq!(qn_noprefix.prefix, None);
        assert_eq!(qn_noprefix.prefixed_name(), "elemOnly");

        // SourceLocation
        let loc = SourceLocation::at_offset(42).with_line_col(10, 5);
        assert_eq!(loc.byte_offset, 42);
        assert_eq!(loc.line, Some(10));
        assert_eq!(loc.column, Some(5));

        // InfosetPath root, pop, and Display formatting
        let root_path = InfosetPath::root();
        assert!(root_path.is_absolute());
        assert_eq!(alloc::format!("{}", root_path), "/");

        let mut path = InfosetPath::from_parts(alloc::vec!["root".into()], false);
        assert!(!path.is_absolute());
        assert_eq!(path.segments(), &["root"]);

        path.try_push("pfx:item[1]").unwrap();
        assert_eq!(path.segments()[1], "item[1]");

        path.try_push("pfx:item]reversed[").unwrap();
        assert_eq!(path.segments()[2], "item]reversed[");

        path.try_push_raw("raw:segment").unwrap();
        assert_eq!(path.segments()[3], "raw:segment");
        assert_eq!(alloc::format!("{}", path), "/root/item[1]/item]reversed[/raw:segment");

        let popped = path.pop();
        assert_eq!(popped.as_deref(), Some("raw:segment"));

        // InfosetPath parse and fallback on invalid syntax
        let parsed = InfosetPath::parse("/root/header");
        assert!(parsed.is_absolute());
        let bad_parsed = InfosetPath::parse("/[invalid]]path");
        assert!(bad_parsed.is_absolute());
        let rel_bad = InfosetPath::parse("[invalid]]rel");
        assert!(!rel_bad.is_absolute());

        // Policy variants
        let p1 = UnqualifiedPathStepPolicy::DefaultNamespace;
        let p2 = UnqualifiedPathStepPolicy::PreferDefaultNamespace;
        assert_ne!(p1, p2);

        // BitOffset and ByteOffset checked arithmetic
        let bo1 = BitOffset(10);
        let bo2 = BitOffset(20);
        assert_eq!(bo1.checked_add(bo2), Some(BitOffset(30)));
        assert_eq!(bo2.checked_sub(bo1), Some(BitOffset(10)));
        assert_eq!(bo1.checked_sub(bo2), None);
        assert_eq!(BitOffset(usize::MAX).checked_add(BitOffset(1)), None);

        let by1 = ByteOffset(5);
        let by2 = ByteOffset(15);
        assert_eq!(by1.checked_add(by2), Some(ByteOffset(20)));
        assert_eq!(by2.checked_sub(by1), Some(ByteOffset(10)));
        assert_eq!(by1.checked_sub(by2), None);
        assert_eq!(ByteOffset(usize::MAX).checked_add(ByteOffset(1)), None);
        assert_eq!(ByteOffset(usize::MAX).to_bit_offset(), None);

        // SymbolId
        let sym = SymbolId(42);
        assert_eq!(sym.0, 42);
    }

    #[test]
    fn test_path_step_structured_and_infoset_path() {
        // Test PathStep creation, methods and segment conversions
        let step = PathStep::new(
            PathAxis::Child,
            StepTarget::Prefixed {
                prefix: "ns".into(),
                local: "field".into(),
                raw: "ns:field".into(),
            },
            Some(2),
            None,
        );
        assert_eq!(step.axis, PathAxis::Child);
        assert_eq!(step.raw_target(), "ns:field");
        assert_eq!(step.prefix(), Some("ns"));
        assert_eq!(step.local_name(), "field");
        assert_eq!(step.index_predicate, Some(2));
        assert!(!step.is_self());
        assert!(!step.is_parent());
        assert_eq!(step.to_segment_string(), "ns:field[2]");

        // Roundtrip from segment
        let from_seg = PathStep::from_segment("ns:field[2]");
        assert_eq!(from_seg.raw_target(), "ns:field");
        assert_eq!(from_seg.index_predicate, Some(2));
        assert_eq!(from_seg.to_segment_string(), "ns:field[2]");

        // Clark notation target
        let clark_step = PathStep::from_segment("{urn:test}elem");
        assert_eq!(clark_step.clark_uri(), Some("urn:test"));
        assert_eq!(clark_step.local_name(), "elem");
        assert_eq!(clark_step.raw_target(), "{urn:test}elem");

        // Parent and Self steps
        let parent_step = PathStep::from_segment("..");
        assert!(parent_step.is_parent());
        assert_eq!(parent_step.axis, PathAxis::Parent);
        assert_eq!(parent_step.target, StepTarget::ParentNode);

        let self_step = PathStep::from_segment(".");
        assert!(self_step.is_self());
        assert_eq!(self_step.axis, PathAxis::SelfAxis);
        assert_eq!(self_step.target, StepTarget::SelfNode);

        // InfosetPath with steps
        let mut path = InfosetPath::from_steps(alloc::vec![step, parent_step], true);
        assert!(path.is_absolute());
        assert_eq!(path.steps().len(), 2);
        assert_eq!(path.segments(), &["ns:field[2]", ".."]);

        let popped_step = path.pop_step();
        assert!(popped_step.is_some());
        assert!(popped_step.unwrap().is_parent());
        assert_eq!(path.steps().len(), 1);

        path.try_push_step(self_step).unwrap();
        assert_eq!(path.steps().len(), 2);
        assert!(path.steps()[1].is_self());
    }
}

