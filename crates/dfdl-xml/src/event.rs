//! XML token event definitions and attribute representations.
//!
//! Represents streaming XML events produced by [`crate::reader::XmlReader`].

extern crate alloc;
use alloc::borrow::Cow;
use alloc::vec::Vec;

use core::fmt;

use dfdl_core::types::{QName, SourceLocation};

/// Borrowed Qualified Name referencing slices of an input XML document without heap allocation.
///
/// Designed per RUST-XML-01 to eliminate micro-allocations when tokenizing XML attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QNameRef<'a> {
    /// Optional namespace URI.
    pub namespace: Option<&'a str>,
    /// Local part of the qualified name.
    pub local_name: &'a str,
    /// Optional prefix used in XML declarations.
    pub prefix: Option<&'a str>,
}

impl<'a> QNameRef<'a> {
    /// Creates a borrowed QName with local name only and no namespace.
    #[inline]
    #[must_use]
    pub const fn local(local_name: &'a str) -> Self {
        Self {
            namespace: None,
            local_name,
            prefix: None,
        }
    }

    /// Creates a borrowed QName with explicit namespace, local name, and optional prefix.
    #[inline]
    #[must_use]
    pub const fn with_namespace(
        namespace: &'a str,
        local_name: &'a str,
        prefix: Option<&'a str>,
    ) -> Self {
        Self {
            namespace: Some(namespace),
            local_name,
            prefix,
        }
    }

    /// Converts this borrowed qualified name into an owned [`QName`].
    #[must_use]
    pub fn to_qname(&self) -> QName {
        match self.namespace {
            Some(ns) => QName::with_namespace(ns, self.local_name, self.prefix),
            None => {
                if let Some(pref) = self.prefix {
                    QName {
                        namespace: None,
                        local_name: alloc::string::String::from(self.local_name),
                        prefix: Some(alloc::string::String::from(pref)),
                    }
                } else {
                    QName::local(self.local_name)
                }
            }
        }
    }

    /// Returns whether this borrowed QName matches an owned [`QName`].
    #[must_use]
    pub fn matches_qname(&self, other: &QName) -> bool {
        if self.local_name != other.local_name.as_str() {
            return false;
        }
        match (self.namespace, &other.namespace) {
            (Some(ns1), Some(ns2)) => ns1 == ns2.as_str(),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<'a> PartialEq<QName> for QNameRef<'a> {
    #[inline]
    fn eq(&self, other: &QName) -> bool {
        self.local_name == other.local_name.as_str()
            && self.prefix == other.prefix.as_deref()
            && self.namespace == other.namespace.as_ref().map(|ns| ns.as_str())
    }
}

impl<'a> PartialEq<QNameRef<'a>> for QName {
    #[inline]
    fn eq(&self, other: &QNameRef<'a>) -> bool {
        other == self
    }
}

impl<'a> From<QNameRef<'a>> for QName {
    #[inline]
    fn from(r: QNameRef<'a>) -> Self {
        r.to_qname()
    }
}

impl<'a> fmt::Display for QNameRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.namespace {
            Some(ns) => write!(f, "{{{}}}:{}", ns, self.local_name),
            None => write!(f, "{}", self.local_name),
        }
    }
}

/// XML Attribute representation with resolved borrowed QName and string value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute<'a> {
    /// Resolved Qualified Name reference of the attribute.
    pub name: QNameRef<'a>,
    /// Raw or unescaped attribute value string.
    pub value: Cow<'a, str>,
    /// Source location where the attribute occurred.
    pub location: SourceLocation,
}

impl<'a> Attribute<'a> {
    /// Returns the resolved qualified name as an owned [`QName`].
    #[inline]
    #[must_use]
    pub fn qname(&self) -> QName {
        self.name.to_qname()
    }
}

/// Streaming XML event enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent<'a> {
    /// XML declaration / start document (e.g. `<?xml version="1.0" encoding="UTF-8"?>`).
    StartDocument {
        /// Document character encoding string (e.g. "UTF-8").
        encoding: &'a str,
        /// Source location of the declaration.
        location: SourceLocation,
    },
    /// Start element tag event with resolved QName and attributes.
    StartElement {
        /// Resolved element QName.
        name: QName,
        /// List of attributes present on the element.
        attributes: Vec<Attribute<'a>>,
        /// Source location of the element start.
        location: SourceLocation,
    },
    /// End element tag event.
    EndElement {
        /// Resolved element QName.
        name: QName,
        /// Source location of the element end.
        location: SourceLocation,
    },
    /// Text character data content.
    Text {
        /// Unescaped text content string.
        content: Cow<'a, str>,
        /// Source location of the text content.
        location: SourceLocation,
    },
    /// CDATA section content (e.g. `<![CDATA[...]]>`).
    CData {
        /// Raw CDATA content string.
        content: &'a str,
        /// Source location of the CDATA section.
        location: SourceLocation,
    },
    /// XML Comment (e.g. `<!-- comment -->`).
    Comment {
        /// Comment text content.
        content: &'a str,
        /// Source location of the comment.
        location: SourceLocation,
    },
    /// Processing Instruction (e.g. `<?target content?>`).
    ProcessingInstruction {
        /// Processing instruction target name.
        target: &'a str,
        /// Optional processing instruction content.
        content: Option<&'a str>,
        /// Source location of the processing instruction.
        location: SourceLocation,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_event_construction() {
        let name = QName::local("root");
        let loc = SourceLocation::at_offset(0);
        let qref = QNameRef::local("attr");
        let attr = Attribute {
            name: qref,
            value: Cow::Borrowed("val"),
            location: loc,
        };
        assert_eq!(attr.name.local_name, "attr");
        assert_eq!(attr.value, "val");
        assert_eq!(attr.qname(), QName::local("attr"));
        assert_eq!(attr.name, QName::local("attr"));
        assert_eq!(QName::local("attr"), attr.name);

        let qref_ns = QNameRef::with_namespace("http://example.com", "elem", Some("p"));
        assert_eq!(qref_ns.to_qname(), QName::with_namespace("http://example.com", "elem", Some("p")));
        assert_eq!(alloc::format!("{}", qref), "attr");
        assert_eq!(alloc::format!("{}", qref_ns), "{http://example.com}:elem");
        assert!(qref_ns.matches_qname(&QName::with_namespace("http://example.com", "elem", None)));

        let events = [
            XmlEvent::StartDocument {
                encoding: "UTF-8",
                location: loc,
            },
            XmlEvent::StartElement {
                name: name.clone(),
                attributes: alloc::vec![attr],
                location: loc,
            },
            XmlEvent::Text {
                content: Cow::Borrowed("hello"),
                location: loc,
            },
            XmlEvent::CData {
                content: "cdata_data",
                location: loc,
            },
            XmlEvent::Comment {
                content: "comment_text",
                location: loc,
            },
            XmlEvent::ProcessingInstruction {
                target: "target",
                content: Some("data"),
                location: loc,
            },
            XmlEvent::EndElement {
                name,
                location: loc,
            },
        ];

        assert_eq!(events.len(), 7);
    }
}
