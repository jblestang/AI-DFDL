//! Error types for the DFDL Ahead-of-Time (AOT) code generator.
//!
//! Provides structured diagnostic errors for unsupported schema constructs,
//! unresolved type references, and code generation failures.

use core::fmt;

/// Result type alias for code generation operations.
pub type CodegenResult<T> = Result<T, CodegenError>;

/// Errors that can occur during schema analysis and Rust code generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenError {
    /// A required schema node was not found in the compiled graph.
    NodeNotFound {
        /// Identifier of the missing node.
        node_id: u32,
    },
    /// A schema construct is not supported by the AOT code generator MVP.
    UnsupportedConstruct {
        /// Description of the unsupported feature.
        feature: &'static str,
        /// Contextual details or element name.
        details: alloc::string::String,
    },
    /// Schema validation or structural inconsistency detected.
    InvalidSchema {
        /// Explanation of the inconsistency.
        message: alloc::string::String,
    },
    /// Choice branch without a deterministic initiator or branch key.
    AmbiguousChoiceBranch {
        /// Name of the choice or branch element.
        branch_name: alloc::string::String,
    },
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeNotFound { node_id } => {
                write!(f, "schema node ID {node_id} not found in compiled graph")
            }
            Self::UnsupportedConstruct { feature, details } => {
                write!(f, "unsupported schema construct '{feature}': {details}")
            }
            Self::InvalidSchema { message } => {
                write!(f, "invalid schema definition: {message}")
            }
            Self::AmbiguousChoiceBranch { branch_name } => {
                write!(
                    f,
                    "choice branch '{branch_name}' lacks a deterministic initiator or branch key"
                )
            }
        }
    }
}

impl core::error::Error for CodegenError {}
