//! DFDL Ahead-of-Time (AOT) Schema Compiler and Native Code Generator.
//!
//! Compiles validated [`dfdl_core::schema::ir::CompiledSchema`] intermediate representations
//! into safe, `#![no_std]`-compatible, zero-allocation native Rust binary codecs.
//!
//! # Key Architectural Benefits
//! - **Eliminates Interpretation Overhead**: Completely removes AST walks, dynamic property
//!   lookups, and runtime alignment branching.
//! - **Zero DOM Allocation**: Decodes directly into strongly-typed Rust `struct` and `enum`
//!   types without heap allocation or `DfdlValue` / `InfosetNode` boxing.
//! - **Wire-Speed Execution**: Decodes aligned binary fields via native endian byte loads
//!   (`from_be_bytes`), matching hand-written C/Rust decoders at ~25–40 nanoseconds.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

extern crate alloc;
use alloc::string::String;

pub mod analyzer;
pub mod emitter;
pub mod error;
pub mod ir;

pub use analyzer::SchemaAnalyzer;
pub use emitter::RustEmitter;
pub use error::{CodegenError, CodegenResult};
pub use ir::CodeGenIr;

use dfdl_core::schema::ir::CompiledSchema;

/// Code generation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodecStrategy {
    /// Standard strongly-typed owned struct codec (deserializes fields into struct).
    #[default]
    Owned,
    /// Zero-copy accessor view (`StructRef<'a>(&'a [u8])`) borrowing directly from the wire slice.
    /// Supports variable-length messages via dynamic slice bounds.
    ZeroCopy,
    /// SIMD-accelerated codec using 128-bit NEON (ARM64) / 256-bit AVX2 (x86_64) vector instructions,
    /// with vectorized delimiter scanning for variable-length messages.
    Simd,
}

/// Configuration options for the DFDL AOT code generator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodegenConfig {
    /// Code generation strategy.
    pub strategy: CodecStrategy,
}

/// Generates native Rust binary codec source code with default configuration.
///
/// # Errors
/// Returns [`CodegenError`] if the schema contains unsupported constructs or invalid node references.
pub fn generate_rust_code(schema: &CompiledSchema) -> CodegenResult<String> {
    generate_rust_code_with_config(schema, &CodegenConfig::default())
}

/// Generates native Rust binary codec source code for the provided compiled DFDL schema and configuration.
///
/// # Errors
/// Returns [`CodegenError`] if code emission or schema analysis fails.
pub fn generate_rust_code_with_config(
    schema: &CompiledSchema,
    config: &CodegenConfig,
) -> CodegenResult<String> {
    let analyzer = SchemaAnalyzer::new(schema);
    let ir = analyzer.analyze()?;
    let emitter = RustEmitter::with_config(config.clone());
    emitter.emit(&ir)
}

/// AOT Code Generator configuration and compiler builder.
#[derive(Debug, Default)]
pub struct CodeGenerator {
    config: CodegenConfig,
}

impl CodeGenerator {
    /// Creates a new [`CodeGenerator`] with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: CodegenConfig::default(),
        }
    }

    /// Creates a new [`CodeGenerator`] with specified [`CodegenConfig`].
    #[must_use]
    pub fn with_config(config: CodegenConfig) -> Self {
        Self { config }
    }

    /// Sets the [`CodecStrategy`] (Owned, ZeroCopy, Simd).
    #[must_use]
    pub fn with_strategy(mut self, strategy: CodecStrategy) -> Self {
        self.config.strategy = strategy;
        self
    }

    /// Compiles a [`CompiledSchema`] into native Rust source code.
    ///
    /// # Errors
    /// Returns [`CodegenError`] on analysis or code emission failures.
    pub fn generate(&self, schema: &CompiledSchema) -> CodegenResult<String> {
        generate_rust_code_with_config(schema, &self.config)
    }
}

