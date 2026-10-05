//! Low-allocation streaming Infoset events and I/O pipeline interfaces.
//!
//! Provides [`InfosetEvent`], [`InfosetEventSink`], and [`InfosetSource`] traits for DFDL parse and unparse execution.

use crate::error::DFDLResult;
use crate::infoset::value::DfdlValue;
use crate::types::QName;

/// Low-allocation streaming event representation for DFDL Infoset streams.
#[derive(Debug, Clone, PartialEq)]
pub enum InfosetEvent {
    /// Start of Infoset document.
    StartDocument,
    /// End of Infoset document.
    EndDocument,
    /// Entry into a complex element container or element item.
    StartElement {
        /// Element Qualified Name.
        name: QName,
        /// Indicates if the element is nilled.
        is_nil: bool,
    },
    /// Exit from a complex element container.
    EndElement {
        /// Element Qualified Name.
        name: QName,
    },
    /// Scalar simple element value item.
    SimpleValue {
        /// Element Qualified Name.
        name: QName,
        /// Typed scalar value payload.
        value: DfdlValue,
    },
    /// Present element with empty value.
    EmptyValue {
        /// Element Qualified Name.
        name: QName,
    },
    /// Present element with nil value.
    NilValue {
        /// Element Qualified Name.
        name: QName,
    },
}

/// Abstract event sink interface for receiving streaming Infoset events during parsing.
pub trait InfosetEventSink {
    /// Receives and processes a streaming [`InfosetEvent`].
    fn push_event(&mut self, event: InfosetEvent) -> DFDLResult<()>;
}

/// Abstract event source interface for producing streaming Infoset events during unparsing.
pub trait InfosetSource {
    /// Advances and yields the next streaming [`InfosetEvent`].
    fn next_event(&mut self) -> DFDLResult<Option<InfosetEvent>>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_infoset_event_construction() {
        let qn = QName::local("root");
        let ev = InfosetEvent::StartElement {
            name: qn.clone(),
            is_nil: false,
        };

        if let InfosetEvent::StartElement { name, is_nil } = ev {
            assert_eq!(name.local_name, "root");
            assert!(!is_nil);
        } else {
            panic!("Expected StartElement");
        }
    }
}
