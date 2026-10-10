//! XML token event definitions and attribute representations.
//!
//! Represents streaming XML events produced by [`crate::reader::XmlReader`].

extern crate alloc;
use alloc::borrow::Cow;
use alloc::vec::Vec;

use dfdl_core::types::{QName, SourceLocation};

/// XML Attribute representation with resolved QName and string value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute<'a> {
    /// Resolved Qualified Name of the attribute.
    pub name: QName,
    /// Raw or unescaped attribute value string.
    pub value: Cow<'a, str>,
    /// Source location where the attribute occurred.
    pub location: SourceLocation,
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
        let attr = Attribute {
            name: QName::local("attr"),
            value: Cow::Borrowed("val"),
            location: loc,
        };
        assert_eq!(attr.name.local_name, "attr");
        assert_eq!(attr.value, "val");

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
