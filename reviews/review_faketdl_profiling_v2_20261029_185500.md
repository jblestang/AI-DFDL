# FakeTDL Benchmark Performance Profiling v2 — Post-Optimization Analysis

**Date:** 2026-10-29 18:55:00 CET  
**Scope:** Re-profiling after `InfosetCheckpoint` optimization in [`crates/dfdl-core/src/infoset/tree.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs)  
**Binary:** `dfdl-benchmark` (release mode, CPU sampling 1 kHz, 100,000 iterations)  
**Previous Reference:** [`review_faketdl_profiling_20261029_145500.md`](file:///Users/jean-baptiste/AI-DFDL/review_faketdl_profiling_20261029_145500.md)

---

## 1. Executive Summary: What Changed After Hotspot #1 Fix

The elimination of whole-tree cloning (`InfosetBuilder::checkpoint` replacing `self.clone()` with `InfosetCheckpoint` marker stack) yielded dramatic performance improvements:

| Benchmark Operation | Baseline Latency (v1) | Optimized Latency (v2) | Latency Delta | Throughput Gain |
| :--- | :--- | :--- | :--- | :--- |
| **Raw DFDL parse (64B Track)** | 16.68 µs | **9.00 µs** | **-46.0%** | **+83.2%** (111,103 ops/s) |
| **Raw DFDL unparse (64B Track)** | 18.23 µs | **17.92 µs** | -1.7% | +1.7% (55,803 ops/s) |
| **Raw DFDL roundtrip (64B Track)** | 35.86 µs | **27.60 µs** | **-23.0%** | **+29.9%** (36,231 ops/s) |
| **Raw DFDLFile parse (192B Multi)** | 69.18 µs | **27.06 µs** | **-60.9%** | **+152.1%** (36,954 ops/s) |
| **Raw DFDLFile unparse (192B Multi)**| 27.67 µs | **28.32 µs** | +2.3% | -2.3% (35,310 ops/s) |
| **Raw DFDLFile roundtrip (192B Multi)**| 96.25 µs | **57.65 µs** | **-40.1%** | **+66.2%** (17,346 ops/s) |

- In the v1 profile, `InfosetBuilder::checkpoint` was the single largest bottleneck with **1,188 samples** (~35-40% of parse CPU).
- In this v2 profile, `InfosetBuilder::checkpoint` has **disappeared completely** (< 5 samples), achieving the targeted zero-alloc rollback bookmark.

---

## 2. Top-of-Stack Leaf Analysis (Where CPU Cycles Go Now)

Out of ~10,000 total profile samples, the top leaf functions are:

| Rank | Function / Symbol | Samples | Module / Subsystem | Primary Root Cause |
| :---: | :--- | :---: | :--- | :--- |
| **1** | `_xzm_free` / `_free` / `___rdl_dealloc` | **1,486** | Memory Allocator | Allocations of transient Strings, Vecs, and Pest tokens |
| **2** | `_malloc_zone_malloc` / `_xzm_xzone_malloc` / `___rdl_alloc` | **1,080** | Memory Allocator | Heap requests for encoding checks, ASTs, and vector growth |
| **3** | `Pattern::is_contained_in` | **424** | `core::str::pattern` | Substring searches in `encoding_char_bits` / `encoding_mandatory_alignment_bits` |
| **4** | `_platform_memcmp` / `memcmp` | **485** | Platform / libc | String comparisons in Pest grammar parsing & encoding checks |
| **5** | `_platform_memmove` / `memcpy` | **415** | Platform / libc | String copying and Vec growth in buffers and AST nodes |
| **6** | `memchr::memrchr` | **350** | `core::slice::memchr` | Suffix splitting in [`count_child_occurrences`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L615) (`split(':').next_back()`) |
| **7** | `SplitInternal::next_back` | **192** | `core::str::iter` | Suffix splitting in `count_child_occurrences` |
| **8** | `parse_element_with_separators` | **157** | `kernel::parser::element` | Element occurrence dispatch, alignment, delimiter matching |
| **9** | `String::clone` | **141** | `alloc::string` | `QName` cloning and path string construction |
| **10**| `unparse_element_with_seg_internals` | **115** | `kernel::unparser` | Dynamic OVC evaluation and recursive term navigation |
| **11**| `parse_single_element_occurrence_inner` | **113** | `kernel::parser::element` | Scalar decoding, validation, assert evaluations |
| **12**| `unparse_term_internal` | **108** | `kernel::unparser` | Unparser term routing |
| **13**| `BitReader::read_bits` | **100** | `io::bitstream` | Bit-level stream reading |
| **14**| `write_evaluated_delimiter` | **96** | `kernel::unparser::framing` | Delimiter unparsing & formatting |
| **15**| `resolve_output_new_line` | **76** | `kernel::unparser::framing` | Output newline determination |
| **16**| `count_child_occurrences` | **75** | `infoset::tree` | Linear scanning & splitting over sibling nodes |
| **17**| `CompiledSchema::term_has_representation_impl` | **72** | `schema::ir` | Dynamic recursion & `Vec<NodeId>` heap allocation |
| **18**| `RawVecInner::finish_grow` / `realloc` | **160** | `alloc::raw_vec` | Vector reallocations (AST tokens, child elements, buffer sinks) |

---

## 3. The 5 Main Remaining Performance Bottlenecks

### Hotspot #1: Transient String Allocations in `encoding.rs` (~15-20% of Parser CPU)
- **Location:** [`crates/dfdl-core/src/encoding.rs:456-530`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L456-L530)
- **Mechanism:**
  Every time a text element is parsed or aligned, the engine invokes:
  1. [`encoding_mandatory_alignment_bits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L520)
  2. [`encoding_char_bits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L456)
  3. [`encoding_unit_bits`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L485)
  Each of these functions executes:
  ```rust
  let enc_upper = encoding.trim().to_ascii_uppercase();
  ```
  Followed by 10 to 15 `.contains("...")` calls.
- **Cost:**
  - `to_ascii_uppercase()` allocates a fresh heap `String` on every scalar field.
  - `.contains()` uses `Pattern::is_contained_in` (424 samples, #3 in whole benchmark) and `platform_memcmp` (485 samples).
  - The heap string is immediately dropped (`free`).
- **Optimization Potential:**
  - Schema properties are immutable. The encoding string can be parsed once during schema compilation into an `enum EncodingKind { Utf8, Ascii, Iso8859_1, ... }` or matched zero-alloc using case-insensitive ASCII byte checks without allocating a `String`.

---

### Hotspot #2: Pest Expression Parsing at Unparse Runtime (`outputValueCalc`) (~12-15% of Unparser CPU)
- **Location:** [`crates/dfdl-core/src/kernel/unparser/mod.rs:1236`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs#L1236)
- **Mechanism:**
  In FakeTDL, fields like checksums or message headers use `dfdl:outputValueCalc`.
  In `unparse_element_with_seg_internals`:
  ```rust
  if let Some(ref ovc_expr) = props.output_value_calc {
      let ast = crate::expr::parse_expr(ovc_expr)?;
      let mut ctx = self.make_expr_context();
      let val = crate::expr::eval_expr(&ast, &mut ctx)?;
      ...
      self.ovc_values.insert(alloc::format!("{}", self.current_path), val.clone());
  ```
- **Cost:**
  - `parse_expr` invokes Pest (`DfdlPestParser`) at runtime for every message:
    - 269 samples in `parse_expr`.
    - Hundreds of sub-rule recursions (`logic_or`, `equality`, `additive`, `qname_step`, etc.).
    - Allocates Pest pairs, token queues, and AST nodes (`ExprAst`), evaluates them, and frees them.
  - `alloc::format!("{}", self.current_path)` allocates a `String` on each insertion.
- **Optimization Potential:**
  - Pre-parse expression strings during schema compilation into `CompiledProperties::output_value_calc_ast: Option<ExprAst>`.
  - Avoid parsing grammar at wire runtime.

---

### Hotspot #3: `count_child_occurrences` and Repeated Suffix Splitting (~10-13% of Parser CPU)
- **Location:** [`crates/dfdl-core/src/infoset/tree.rs:615-632`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L615-L632)
- **Mechanism:**
  ```rust
  pub fn count_child_occurrences(&self, local_name: &str) -> usize {
      let clean_target = local_name.split(':').next_back().unwrap_or(local_name);
      if let Some(parent) = self.stack.last() {
          parent.children.iter().filter(|child| match child {
              InfosetNode::Element(elem) => {
                  let clean_elem = elem.name.local_name.split(':').next_back().unwrap_or(&elem.name.local_name);
                  clean_elem == clean_target
              }
          }).count()
  ```
- **Cost:**
  - `split(':').next_back()` calls `memchr::memrchr` (350 leaf samples, #6 in profile) and `SplitInternal::next_back` (192 leaf samples, #7 in profile).
  - Executed for every child in `parent.children` on every element occurrence loop.
  - However, `elem.name.local_name` is *already* stripped of any namespace prefix by definition!
- **Optimization Potential:**
  - Direct string slice comparison `elem.name.local_name == clean_target`.
  - Or maintaining occurrence counters directly in `ElementBuilder` or tracking the last sibling index.

---

### Hotspot #4: `CompiledSchema::term_has_representation` Heap Allocations (~5% of runtime)
- **Location:** [`crates/dfdl-core/src/schema/ir.rs:1787-1801`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs#L1787-L1801)
- **Mechanism:**
  ```rust
  pub fn term_has_representation(&self, term_id: NodeId) -> bool {
      let mut visited = alloc::vec::Vec::new();
      self.term_has_representation_impl(term_id, &mut visited)
  }
  ```
- **Cost:**
  - Called multiple times per term during parsing and unparsing.
  - Every call creates `Vec::new()`, pushes `term_id` (triggering `finish_grow` / `malloc`), then drops it (`free`).
  - Accounted for 72 leaf samples in `term_has_representation_impl` + associated allocator traffic.
- **Optimization Potential:**
  - Precompute `has_representation: bool` directly on `CompiledTerm` at schema compilation time.
  - Schema terms are fixed; their representation status never changes at runtime.

---

### Hotspot #5: Infoset Node Allocations & QName Cloning (~8-10% of runtime)
- **Location:** [`crates/dfdl-core/src/infoset/tree.rs:590-610`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L590-L610)
- **Mechanism:**
  1. `InfosetElement::try_add_child`: `children.push(child)` starts with capacity 0 and repeatedly calls `realloc` as children are added (160 samples in `finish_grow`).
  2. `elem.name.clone()`: Deep-clones `QName` (`prefix: Option<String>`, `local_name: String`, `namespace: Option<String>`).
  3. `parse_calendar_from_digits`: Uses `format!` to construct formatted date/time strings.
- **Optimization Potential:**
  - SmallVec or initial capacity hint for `children`.
  - String interning or `Arc<str>` / `&'static str` for schema element names in infoset nodes.

---

## 4. Summary of Impact & Recommended Next Actions

| Target Area | Estimated Latency Saving | Complexity | Priority |
| :--- | :--- | :--- | :--- |
| **1. Zero-alloc / pre-parsed `encoding` properties** | -1.5 µs to -2.0 µs parse | Very Low | High |
| **2. Precomputed `term_has_representation` on `CompiledTerm`** | -0.5 µs to -1.0 µs parse/unparse | Low | High |
| **3. Clean `count_child_occurrences` without `memrchr`** | -1.0 µs parse | Very Low | High |
| **4. Pre-compiled AST for `outputValueCalc` (Unparser)** | -3.0 µs to -4.0 µs unparse | Medium | High |
| **5. Infoset vector pre-sizing & `QName` sharing** | -1.0 µs to -1.5 µs parse | Medium | Medium |

With hotspots #1 through #4 addressed, interpreted AST DFDL parsing for 64B Track is projected to drop from **9.0 µs down to ~4.5–5.5 µs** (approaching the ~1.5 µs baseline of the DFDL-VM engine), and file parsing from **27 µs down to ~12–15 µs**.
