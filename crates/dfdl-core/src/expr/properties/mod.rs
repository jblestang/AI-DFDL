//! DFDL Property Resolution, Scoping, and Inheritance Engine.
//!
//! Follows DFDL 1.0 Specification §6.3, §7, and §8.1.
//! Implements local annotation overriding, schema format defaults, and inheritance rules.
//!
//! # Architecture & Modularization
//!
//! The properties subsystem is organized into distinct domain modules following separation of concerns:
//! - [`entities`]: DFDL character and byte entity encoding, decoding, syntax validation, and regex checks (§23.4).
//! - [`keys`]: Strongly typed [`PropertyKey`] enum, dynamic [`PropertyKeyName`], and [`PropertyBinding`] (§6.3).
//! - [`scope`]: Lexical hierarchy and property delegation without deep copying ([`PropertyScope`], §13).
//! - [`store`]: Scoped property storage, namespaces, assertions, variables, and parent merging ([`PropertyStore`]).
//! - [`resolution`]: Property compilation into typed [`crate::schema::ir::ResolvedProperties`] with resource limits.

pub mod entities;
pub mod keys;
pub mod resolution;
pub mod scope;
pub mod store;

#[cfg(test)]
mod tests;

pub use entities::{decode_dfdl_character_entities, encode_dfdl_character_entities};
pub use keys::{PropertyBinding, PropertyKey, PropertyKeyName};
pub use resolution::parse_byte_order;
pub use scope::PropertyScope;
pub use store::PropertyStore;
