//! Typed DFDL diagnostics and error classification.
//!
//! Implements structured failure classifications defined by DFDL 1.0 §3.2 and Appendix F.

extern crate alloc;
use alloc::string::String;
use core::fmt;

use crate::types::SourceLocation;

/// DFDL error classification enumeration matching DFDL 1.0 §3.2 and Appendix F.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DFDLErrorKind {
    /// Schema Definition Error: invalid, malformed, or contradictory DFDL schema.
    SchemaDefinition,
    /// Parse Error: failure during parsing of physical data into DFDL Infoset.
    Parse,
    /// Unparse Error: failure during unparsing of DFDL Infoset into physical data.
    Unparse,
    /// Validation Error: optional XSD validation failure.
    Validation,
    /// Expression Error: evaluation or syntax failure in DFDL expression.
    ExpressionError,
    /// Type Error: runtime type mismatch in expression or property conversion.
    TypeError,
    /// Recoverable Error: non-fatal diagnostic.
    Recoverable,
    /// Implementation Limit Error: resource limit, memory, or recursion budget exceeded.
    ImplementationLimit,
    /// Work Budget Exhausted Error.
    WorkBudgetExhausted,
    /// External Callback Error: failure returned by user resolver or trace hook.
    ExternalCallback,
    /// Internal Invariant Error: unexpected state error in engine logic.
    InternalInvariant,
}

/// Message payload abstraction supporting allocated `String` or zero-allocation `&'static str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorMessage {
    /// Zero-allocation static string slice reference.
    Static(&'static str),
    /// Dynamically allocated string message.
    Allocated(String),
}

impl ErrorMessage {
    /// Returns a string slice reference to the message payload.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(s) => s,
            Self::Allocated(s) => s.as_str(),
        }
    }
}

impl fmt::Display for ErrorMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured DFDL Diagnostic record.
///
/// Contains error classification, static or allocated context message, and optional location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DFDLError {
    /// Classification category of this error.
    pub kind: DFDLErrorKind,
    /// Detailed diagnostic message payload.
    pub message: ErrorMessage,
    /// Optional source location details.
    pub location: Option<SourceLocation>,
}

impl DFDLError {
    /// Constructs a new [`DFDLError`] with allocated message string.
    #[inline]
    pub fn new(kind: DFDLErrorKind, message: &str) -> Self {
        Self {
            kind,
            message: ErrorMessage::Allocated(String::from(message)),
            location: None,
        }
    }

    /// Constructs a const-compatible [`DFDLError`] with static string slice.
    #[inline]
    #[must_use]
    pub const fn new_static(kind: DFDLErrorKind, message: &'static str) -> Self {
        Self {
            kind,
            message: ErrorMessage::Static(message),
            location: None,
        }
    }

    /// Attaches source location details to the error.
    #[inline]
    #[must_use]
    pub const fn with_location(mut self, location: SourceLocation) -> Self {
        self.location = Some(location);
        self
    }

    /// Constructs an arithmetic overflow error.
    #[inline]
    #[must_use]
    pub const fn arithmetic_overflow(message: &'static str) -> Self {
        Self::new_static(DFDLErrorKind::ExpressionError, message)
    }
}

impl fmt::Display for DFDLError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Some(loc) => write!(
                f,
                "[{:?}] at offset {}: {}",
                self.kind, loc.byte_offset, self.message
            ),
            None => write!(f, "[{:?}]: {}", self.kind, self.message),
        }
    }
}

/// Specialized Result alias for DFDL engine operations.
pub type DFDLResult<T> = Result<T, DFDLError>;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_error_creation_and_display() {
        let err = DFDLError::new(DFDLErrorKind::SchemaDefinition, "Invalid property value")
            .with_location(SourceLocation::at_offset(42));

        assert_eq!(err.kind, DFDLErrorKind::SchemaDefinition);
        assert_eq!(err.location, Some(SourceLocation::at_offset(42)));
        assert_eq!(
            err.to_string(),
            "[SchemaDefinition] at offset 42: Invalid property value"
        );
    }

    #[test]
    fn test_static_error_creation() {
        const ERR: DFDLError = DFDLError::new_static(DFDLErrorKind::Parse, "Static end of input");
        assert_eq!(ERR.kind, DFDLErrorKind::Parse);
        assert_eq!(ERR.message.as_str(), "Static end of input");
    }
}
