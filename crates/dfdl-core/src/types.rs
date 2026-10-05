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
        if let Some((prefix, local)) = s.split_once(':') {
            Self {
                namespace: None,
                local_name: String::from(local),
                prefix: Some(String::from(prefix)),
            }
        } else {
            Self::local(s)
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

/// Path navigating the DFDL Infoset structure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfosetPath {
    segments: Vec<String>,
    is_absolute: bool,
}

impl InfosetPath {
    /// Creates a root Infoset path.
    #[inline]
    #[must_use]
    pub const fn root() -> Self {
        Self {
            segments: Vec::new(),
            is_absolute: true,
        }
    }

    /// Constructs an [`InfosetPath`] directly from segments and absolute flag.
    #[inline]
    #[must_use]
    pub const fn from_parts(segments: Vec<String>, is_absolute: bool) -> Self {
        Self {
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
    /// Returns a slice of segments in the path.
    #[inline]
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
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
        self.segments.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.segments.push(clean);
        Ok(())
    }

    /// Pushes a segment verbatim onto the path using fallible allocation, preserving QName prefixes.
    pub fn try_push_raw(&mut self, segment: &str) -> DFDLResult<()> {
        self.segments.try_reserve(1).map_err(|_| {
            DFDLError::new(
                DFDLErrorKind::ImplementationLimit,
                "InfosetPath growth allocation failed",
            )
        })?;
        self.segments.push(alloc::string::ToString::to_string(segment));
        Ok(())
    }

    /// Pops the last segment from the path.
    pub fn pop(&mut self) -> Option<String> {
        self.segments.pop()
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
#[allow(clippy::unwrap_used)]
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
}
