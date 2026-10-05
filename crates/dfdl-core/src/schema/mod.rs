//! Compiled Schema Intermediate Representation (IR) module.
//!
//! Aligned with DFDL 1.0 §§5–8 semantic model.

pub mod builder;
pub mod ir;

pub use builder::{validate_schema_ir, SchemaBuilder};
pub use ir::{
    CompiledChoice, CompiledElement, CompiledSchema, CompiledSequence, CompiledTerm, CompiledType,
    EmptyElementParsePolicy, LengthKind, NodeId, Representation, ResolvedProperties, TermKind,
    AlignmentKind,
};
