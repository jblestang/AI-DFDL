//! DFDL Element state representations: Value, Empty, Nil, NoValue, and Absent.
//!
//! Aligned with DFDL 1.0 §4.2 Information Item properties and §9.4 empty/nil semantics.

use crate::infoset::value::DfdlValue;

/// Represents the precise semantic value state of a DFDL Element Information Item.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementState {
    /// Element is present in the Infoset and contains a scalar value.
    Value(DfdlValue),
    /// Element is present with empty value (empty representation parsed / default empty policy).
    Empty,
    /// Element is nilled (`xsi:nil="true"` or `dfdl:nilValue`).
    Nil,
    /// Element container is present in tree but has no scalar value (e.g. complex element).
    NoValue,
    /// Element is absent / missing from the Infoset.
    Absent,
}

impl ElementState {
    /// Returns `true` if the element state is [`ElementState::Value`].
    #[must_use]
    pub const fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    /// Returns `true` if the element is nilled ([`ElementState::Nil`]).
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    /// Returns `true` if the element is empty ([`ElementState::Empty`]).
    #[must_use]
    pub const fn is_empty_value(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns `true` if the element is absent ([`ElementState::Absent`]).
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_element_state_distinctions() {
        let st_val = ElementState::Value(DfdlValue::Int(10));
        let st_empty = ElementState::Empty;
        let st_nil = ElementState::Nil;
        let st_absent = ElementState::Absent;
        let st_noval = ElementState::NoValue;

        assert!(st_val.is_value());
        assert!(!st_val.is_nil());

        assert!(st_empty.is_empty_value());
        assert!(st_nil.is_nil());
        assert!(st_absent.is_absent());
        assert!(!st_noval.is_value());
    }
}
