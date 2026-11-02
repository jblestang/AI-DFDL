# Architecture & Implementation Review: Compile-Time Encoding Resolution

**Timestamp:** 2026-10-30 08:55:00 UTC+01:00  
**Domain Expertise:** Systems Programming, Compiler Design, Binary Serialization, High-Performance Rust, DFDL 1.0 Specification Conformance  

---

## 1. Executive Summary

Previously, character set encoding resolution in DFDL properties was performed on-the-fly or relied on raw strings (`String` and `&str`) throughout `ResolvedProperties`, `InScopeDelimiter`, and parsing/unparsing loops. Runtime routines frequently performed heap allocations (`to_ascii_uppercase()`, string slicing, cloning) and case-insensitive string comparisons (`eq_ignore_ascii_case("UTF-8")`) on every element and delimiter check.

This refactoring completely eliminates runtime encoding string parsing for schemas:
1. **Compile-Time Resolution:**
   - Raw DFDL encoding strings are parsed into a strongly-typed [`EncodingKind`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/mod.rs) enum during schema compilation (`to_resolved_properties_with_limits` and `schema::builder`).
   - Essential encoding properties (`encoding_char_bits`, `encoding_unit_bits`, and `mandatory_alignment_bits`) are precomputed at schema lower/build time directly on [`ResolvedProperties`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs).
2. **Modular Encoding System:**
   - Modularized encoding implementations under [`crates/dfdl-core/src/encoding/`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/) with one file per encoding family ([`utf8.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/utf8.rs), [`ascii.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/ascii.rs), [`iso8859_1.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/iso8859_1.rs), [`iso8859_13.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/iso8859_13.rs), [`ebcdic.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/ebcdic.rs), [`utf16.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/utf16.rs), [`utf32.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/utf32.rs), [`sub_byte.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/sub_byte.rs), [`custom.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/custom.rs)).
   - Backed by the unified [`Encoding`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/traits.rs) trait.
3. **Zero-Allocation Runtime Execution:**
   - The parser and unparser engines now inspect `props.encoding` via enum matching (e.g. `props.encoding == EncodingKind::Utf8`), removing string allocations and case conversion overhead from critical hot paths.
   - `ParserEngine.delim_encoding` and `InScopeDelimiter.encoding` use [`EncodingKind`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding/mod.rs) directly.

---

## 2. Detailed Technical Review

### 2.1 Compiler & Lowering
- **Property Lowering:** In [`crates/dfdl-core/src/expr/properties/resolution.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties/resolution.rs), literal encoding values are parsed using `EncodingKind::parse_name(encoding_raw)`. For expressions (`{ ... }`), `DfdlProp::parse_with(..., EncodingKind::parse_name)` stores the pre-parsed AST and closure.
- **Bit Metrics:** `encoding_char_bits` (fixed bit width for sub-byte encodings) and `encoding_unit_bits` (code unit width: 1, 8, 16, or 32 bits) are precalculated and stored in `ResolvedProperties`.
- **Implicit Alignment:** In [`crates/dfdl-schema/src/compiler/lower/helpers.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-schema/src/compiler/lower/helpers.rs), `compute_implicit_alignment` now accepts `&EncodingKind` and retrieves `mandatory_alignment_bits()` in O(1) without string checks.

### 2.2 Parser & Unparser Hot Paths
- **Mandatory Text Alignment (MTA):** In [`crates/dfdl-core/src/kernel/parser/mod.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/mod.rs) and [`crates/dfdl-core/src/kernel/parser/text.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/text.rs), MTA checks take `&EncodingKind` and directly test `align_bits = encoding.mandatory_alignment_bits()`.
- **Delimiters & Scopes:** `delim_encoding` on `ParserEngine` is now `EncodingKind`. Peeking and pushing in-scope delimiters no longer clones heap strings on every element boundary.
- **Zoned Number Parsing:** Eliminated string uppercase checks for EBCDIC in `parse_zoned_number`; now performs a direct enum equality check `props.encoding == EncodingKind::Ebcdic`.

### 2.3 Verification & Safety
- **Compilation:** Clean compilation with 0 warnings across the entire workspace under `cargo check --all-targets`.
- **Unit & Integration Tests:** 100% test pass rate across `dfdl-core` (281 unit tests + 4 doc tests) and `dfdl-schema` (117 tests).
- **Official Conformance:** 100% pass rate on the Apache Daffodil TDML test suite (`test_apache_daffodil_official_tdml_suite`).
- **Graft Context Graph:** Deterministically refreshed via `graft build`.
