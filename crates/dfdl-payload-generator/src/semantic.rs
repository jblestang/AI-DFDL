//! Semantic anomaly injector for DFDL payloads.
//!
//! Generates payloads that violate logical schema rules and data contracts:
//! - Out-of-bounds facet constraints (`minInclusive`, `maxInclusive`, `totalDigits`)
//! - Disallowed enumeration values (`xs:enumeration`)
//! - Unmatched choice dispatch keys (`dfdl:choiceDispatchKey`)
//! - Failed DFDL assertions (`dfdl:assert`)
//! - Type and lexical pattern violations

use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use dfdl_core::infoset::value::DfdlValue;

/// Specific category of semantic mutation to apply to an Infoset document.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticAnomaly {
    /// Injects a value that violates an enumeration facet.
    WrongEnum {
        /// Target element name.
        target_name: String,
        /// Invalid enum value to inject.
        invalid_value: String,
    },
    /// Injects a numeric or string value that exceeds or falls below min/max facets.
    FacetOutOfBounds {
        /// Target element name.
        target_name: String,
        /// Out-of-bounds value.
        invalid_value: DfdlValue,
    },
    /// Injects a choice dispatch key that does not match any valid branch in the schema.
    WrongDispatchKey {
        /// Discriminator element local name.
        discriminator_name: String,
        /// Invalid key value (e.g. 4 or 99).
        invalid_key: DfdlValue,
    },
    /// Injects a value that triggers a DFDL assertion failure.
    FailedAssert {
        /// Target element name.
        target_name: String,
        /// Value violating the assert test expression.
        failing_value: DfdlValue,
    },
}

/// Applies semantic mutations to an Infoset document.
pub struct SemanticAnomalyInjector;

impl SemanticAnomalyInjector {
    /// Mutates an Infoset document in-place according to the specified semantic anomaly.
    pub fn apply(doc: &mut InfosetDocument, anomaly: &SemanticAnomaly) -> bool {
        let Some(ref mut root) = doc.root else {
            return false;
        };

        let applied = match anomaly {
            SemanticAnomaly::WrongEnum {
                target_name,
                invalid_value,
            } => Self::mutate_element_value(root, target_name, DfdlValue::String(invalid_value.clone())),

            SemanticAnomaly::FacetOutOfBounds {
                target_name,
                invalid_value,
            } => Self::mutate_element_value(root, target_name, invalid_value.clone()),

            SemanticAnomaly::WrongDispatchKey {
                discriminator_name,
                invalid_key,
            } => Self::mutate_element_value(root, discriminator_name, invalid_key.clone()),

            SemanticAnomaly::FailedAssert {
                target_name,
                failing_value,
            } => Self::mutate_element_value(root, target_name, failing_value.clone()),
        };

        if !applied {
            Self::mutate_first_leaf(root, DfdlValue::UnsignedByte(250));
        }
        true
    }

    /// Mutates the first leaf element found in the tree.
    fn mutate_first_leaf(element: &mut InfosetElement, new_val: DfdlValue) -> bool {
        if element.children.is_empty() {
            if let ElementState::Value(ref existing) = element.state {
                let adapted = match (existing, &new_val) {
                    (DfdlValue::UnsignedByte(_), DfdlValue::UnsignedByte(b)) => DfdlValue::UnsignedByte(*b),
                    (DfdlValue::UnsignedShort(_), DfdlValue::UnsignedByte(b)) => DfdlValue::UnsignedShort(*b as u16),
                    (DfdlValue::UnsignedInt(_), DfdlValue::UnsignedByte(b)) => DfdlValue::UnsignedInt(*b as u32),
                    (DfdlValue::Float(_), DfdlValue::UnsignedByte(b)) => DfdlValue::Float(*b as f32),
                    _ => new_val,
                };
                element.state = ElementState::Value(adapted);
            } else {
                element.state = ElementState::Value(new_val);
            }
            return true;
        }

        for child in &mut element.children {
            let InfosetNode::Element(ref mut child_elem) = child;
            if Self::mutate_first_leaf(child_elem, new_val.clone()) {
                return true;
            }
        }
        false
    }

    /// Recursively searches for an element by local name and updates its value.
    fn mutate_element_value(element: &mut InfosetElement, target_name: &str, new_val: DfdlValue) -> bool {
        if element.name.local_name == target_name {
            if let ElementState::Value(ref existing) = element.state {
                let adapted = match (existing, &new_val) {
                    (DfdlValue::UnsignedByte(_), DfdlValue::String(s)) => {
                        s.parse::<u8>().map(DfdlValue::UnsignedByte).unwrap_or(new_val)
                    }
                    (DfdlValue::UnsignedShort(_), DfdlValue::String(s)) => {
                        s.parse::<u16>().map(DfdlValue::UnsignedShort).unwrap_or(new_val)
                    }
                    (DfdlValue::UnsignedInt(_), DfdlValue::String(s)) => {
                        s.parse::<u32>().map(DfdlValue::UnsignedInt).unwrap_or(new_val)
                    }
                    (DfdlValue::UnsignedByte(_), DfdlValue::Long(n)) => {
                        DfdlValue::UnsignedByte(*n as u8)
                    }
                    (DfdlValue::UnsignedShort(_), DfdlValue::Long(n)) => {
                        DfdlValue::UnsignedShort(*n as u16)
                    }
                    (DfdlValue::UnsignedInt(_), DfdlValue::Long(n)) => {
                        DfdlValue::UnsignedInt(*n as u32)
                    }
                    (DfdlValue::UnsignedShort(_), DfdlValue::UnsignedByte(b)) => {
                        DfdlValue::UnsignedShort(*b as u16)
                    }
                    (DfdlValue::Float(_), DfdlValue::Long(n)) => {
                        DfdlValue::Float(*n as f32)
                    }
                    _ => new_val,
                };
                element.state = ElementState::Value(adapted);
            } else {
                element.state = ElementState::Value(new_val);
            }
            return true;
        }

        for child_node in &mut element.children {
            let InfosetNode::Element(ref mut child_elem) = child_node;
            if Self::mutate_element_value(child_elem, target_name, new_val.clone()) {
                return true;
            }
        }

        false
    }
}
