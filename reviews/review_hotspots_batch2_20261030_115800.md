# Code Review & Verification: Hotspot Optimization Batch 2

**Date & Time**: 2026-10-30 11:58:00 CET  
**Scope**: High-Frequency Hotspots in `dfdl-core` (`parser`, `unparser`, `numbers`, `ir`, `builder`, `resolution`)  
**Status**: Verified & Compliant (0 Compiler Warnings, 283/283 unit tests passing, 100% Apache Daffodil TDML suite passing).

---

## 1. Executive Summary & Goals

Following the profiling analysis (`profile_faketdl_v12.txt`), 4 major CPU and allocation hotspots were targeted and refactored across the DFDL core engine:

1. **Gated `current_path()` Behind `ast.needs_infoset_doc()`**:
   - `builder.current_path()` traversed the active XML infoset hierarchy to construct a full `InfosetPath` on every evaluation, even when the expression only inspected variables or literal constants.
   - Conditioned `current_path()` evaluation across:
     - `occurs_count_expr` in [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
     - `ivc` (InputValueCalc) in [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
     - `evaluate_discriminator` in [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
     - `evaluate_asserts` in [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
     - `evaluate_length_property` in [`crates/dfdl-core/src/kernel/parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)
     - `evaluate_property_ast` in [`crates/dfdl-core/src/kernel/parser/delimiters.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/delimiters.rs)
   - When `ast.needs_infoset_doc()` is `false`, `InfosetPath::root()` and `active_doc = None` are passed directly with zero DOM walks and zero allocations.

2. **Fast-Path ASCII Whitespace Check (`numbers.rs`)**:
   - Replaced multi-branch Unicode table checks `s.chars().next().map_or(false, |c| c.is_whitespace())` with an inlined `has_outer_whitespace` helper in [`crates/dfdl-core/src/kernel/parser/numbers.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/numbers.rs).
   - Checks leading and trailing ASCII bytes `b <= b' ' && matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)` in 1-2 CPU cycles before falling back to Unicode tables only if non-ASCII bytes are encountered.

3. **Zero-Allocation `eval_runtime_prop_str` with `Cow<'_, str>` (`suspensions.rs`)**:
   - Refactored `eval_runtime_prop_str` in [`crates/dfdl-core/src/kernel/unparser/suspensions.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/suspensions.rs) to return `alloc::borrow::Cow<'p, str>`.
   - If `!raw.starts_with('{')`, immediately returns `Cow::Borrowed(raw)` without heap allocation or cloning.
   - Refactored unparser lookups in `newVariableInstance` suspension resolution to inspect pre-compiled ASTs.

4. **Pre-Compiled Variable ASTs in Schema IR**:
   - Added `set_variables_compiled: Vec<(QName, Option<ExprAst>, String)>` and `new_variable_instances_compiled: Vec<(QName, Option<ExprAst>, Option<String>)>` to `CompiledProperties` in [`crates/dfdl-core/src/schema/ir.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs).
   - Pre-parsed during schema construction in [`crates/dfdl-core/src/expr/properties/resolution.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties/resolution.rs) and synchronized in [`crates/dfdl-core/src/schema/builder.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/builder.rs).
   - Executed directly at runtime in `parser/mod.rs` and `unparser/suspensions.rs`, eliminating runtime XPath/expression parser invocations.
   - Included robust fallback to dynamic parsing for hand-crafted unit tests directly instantiating `ResolvedProperties`.

---

## 2. Benchmark & Latency Measurements

Benchmarking run with `cargo run --release -p dfdl-benchmark -- --bench`:

| Operation | Previous Latency | Optimized Latency | Relative Improvement |
| :--- | :--- | :--- | :--- |
| **Raw DFDL parse (64B Track)** | 3.47 µs | **3.21 µs** | **-7.5% latency** |
| **Raw DFDL unparse (64B Track)** | 2.34 µs | **2.16 µs** | **-7.7% latency** |
| **Raw DFDL roundtrip (64B Track)** | 5.86 µs | **5.52 µs** | **-5.8% latency** |
| **Raw DFDLFile parse (192B Multi)** | 11.16 µs | **10.37 µs** | **-7.1% latency** |
| **Raw DFDLFile unparse (192B Multi)** | 7.23 µs | **6.59 µs** | **-8.9% latency** |
| **Raw DFDLFile roundtrip (192B Multi)** | 18.63 µs | **17.60 µs** | **-5.5% latency** |

---

## 3. Conformance & Safety Verification

1. **Compilation**:
   - `cargo check --all-targets` passed cleanly with **0 compiler warnings** across all crates (`dfdl-core`, `dfdl-schema`, `dfdl-vm`, `dfdl-tests`, `dfdl-xml`, `dfdl-benchmark`, etc.).
2. **Unit Tests**:
   - `cargo test -p dfdl-core --lib`: 283 passed, 0 failed, 0 ignored (0.03s).
3. **Apache Daffodil TDML Conformance**:
   - `cargo test -p dfdl-tests -- test_apache_daffodil_official_tdml_suite`: 3 passed, 0 failed, 100% compliance maintained.
4. **`#![no_std]` + `alloc` Compatibility**:
   - Preserved `alloc::borrow::Cow`, `alloc::vec::Vec`, `alloc::string::String` without pulling in `std`.
5. **Graft Repo Context**:
   - `graft build` refreshed: 2,439 nodes, 3,480 edges wired successfully.
