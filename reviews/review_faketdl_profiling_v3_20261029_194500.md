# FakeTDL Benchmark Performance Profiling v3 — Post-Hotspots 1-3 Profiling Report

**Date:** 2026-10-29 19:45:00 CET  
**Scope:** CPU Sampling & Hotspot Analysis after Hotspots 1, 2, and 3 Optimizations  
**Binary:** `dfdl-benchmark` (release mode, CPU sampling 1 kHz, 100,000 iterations per op)  
**Previous References:**
- [`review_faketdl_profiling_20261029_145500.md`](file:///Users/jean-baptiste/AI-DFDL/review_faketdl_profiling_20261029_145500.md) (Profiling v1)
- [`review_faketdl_profiling_v2_20261029_185500.md`](file:///Users/jean-baptiste/AI-DFDL/review_faketdl_profiling_v2_20261029_185500.md) (Profiling v2)
- [`review_hotspots_1_2_3_20261029_191500.md`](file:///Users/jean-baptiste/AI-DFDL/review_hotspots_1_2_3_20261029_191500.md) (Hotspots 1, 2, 3 Review)

---

## 1. Executive Summary & Progression

Following the implementation of:
1. Zero-allocation ASCII case-insensitive helpers and fast paths for `UTF-8` / `ASCII` / `ISO-8859-1` in [`crates/dfdl-core/src/encoding.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs).
2. Schema-time pre-compilation of `outputValueCalc` and `inputValueCalc` `DfdlProp` ASTs, eliminating runtime Pest grammar parsing in [`unparser/mod.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs) and [`parser/element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs).
3. Fast-path string comparison in [`count_child_occurrences`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/infoset/tree.rs#L570).

A new profiling campaign was conducted with `/usr/bin/sample` on `dfdl-benchmark` running 100,000 iterations per scenario.

### Comparative Evolution Across Profiling Iterations

| Benchmark Operation | Baseline (v1) | Post-Checkpoint (v2) | Post-Hotspots 1-3 (v3) | Total Latency Reduction | Total Throughput Gain |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Raw DFDL parse (64B Track)** | 16.68 µs | 9.00 µs | **7.47 µs** (min **6.29 µs**) | **-55.2%** | **+115%** (59.0k $\rightarrow$ 129.1k ops/s) |
| **Raw DFDL unparse (64B Track)** | 18.23 µs | 17.92 µs | **11.48 µs** (min **9.42 µs**) | **-37.0%** | **+58%** (54.8k $\rightarrow$ 86.8k ops/s) |
| **Raw DFDL roundtrip (64B Track)** | 35.86 µs | 27.60 µs | **19.64 µs** (min **16.38 µs**) | **-45.2%** | **+80%** (27.9k $\rightarrow$ 50.1k ops/s) |
| **Raw DFDLFile parse (192B Multi)** | 69.18 µs | 27.06 µs | **24.47 µs** (min **21.12 µs**) | **-64.6%** | **+177%** (14.3k $\rightarrow$ 39.7k ops/s) |
| **Raw DFDLFile unparse (192B Multi)**| 27.67 µs | 28.32 µs | **27.35 µs** (min **22.88 µs**) | **-1.2%** | +1.2% |
| **Raw DFDLFile roundtrip (192B Multi)**| 96.25 µs | 57.65 µs | **53.14 µs** (min **45.67 µs**) | **-44.8%** | **+79%** (10.4k $\rightarrow$ 18.6k ops/s) |

---

## 2. Top-of-Stack Leaf Analysis (Profile v3)

Top leaf functions sorted by sample count from `profile_faketdl_v3.txt`:

| Rank | Samples | Function / Symbol | Subsystem | Analysis / Bottleneck |
| :---: | :---: | :--- | :--- | :--- |
| **1** | 1,837 | `_xzm_free` + `_free` | Memory Allocator | Deallocation of vectors (`in_scope_delimiters`, child vectors, tokens) |
| **2** | 1,391 | `_malloc_zone_malloc` + `_xzm_xzone_malloc` + `__rdl_alloc` | Memory Allocator | Transient heap allocations during traversal and delimiter handling |
| **3** | 484 | `_platform_memmove` | libc / memory | Vector resizing, string buffer formatting |
| **4** | 248 | `InfosetBuilder::count_child_occurrences` | `infoset::tree` | Linear scanning over siblings for `current_occurs_index` |
| **5** | 229 | `CharSearcher::next_match` | `core::str` | Substring and delimiter character scans |
| **6** | 209 | `String::clone` | `alloc::string` | `InScopeDelimiter` clones, path names, unparse formatting |
| **7** | 197 | `_platform_memcmp` | libc / memory | String equality checks in schema term matching |
| **8** | 195 | `encoding::encoding_char_bits` | `encoding` | Scanning 15 multi-character patterns for every text element |
| **9** | 180 | `unparse_element_with_seg_internal` | `kernel::unparser` | Recursive unparser term handling |
| **10**| 176 | `parse_single_element_occurrence` | `kernel::parser` | Element parser entrypoint |
| **11**| 147 | `parse_element_with_separators` | `kernel::parser` | Element loop, delimiter checks, occurrences index |
| **12**| 142 | `unparse_term_internal` | `kernel::unparser` | Sequence/choice unparser routing |
| **13**| 136 | `write_evaluated_delimiter` | `kernel::unparser::framing` | Delimiter evaluation and entity unescaping |
| **14**| 133 | `Vec<InfosetNode>::clone` | `infoset::tree` | Infoset subtree cloning during expression evaluation |
| **15**| 130 | `BitReader::read_bits` | `io::bitstream` | Core I/O bit decoding |
| **16**| 118 | `CompiledSchema::term_has_representation_impl` | `schema::ir` | Dynamic recursion & `Vec<NodeId>` allocation per check |
| **17**| 88 | `split_delimiter_alternatives` | `kernel::parser::delimiters` | Splitting delimiter strings into `Vec<String>` on every write |
| **18**| 87 | `resolve_output_new_line` | `kernel::unparser::framing` | Dynamic newline convention resolution per delimiter |
| **19**| 82 | `sequence_member_has_output` | `kernel::unparser` | Quadratic $O(M^2 \times C)$ search over subsequent sequence members |
| **20**| 65 | `encoding_mandatory_alignment_bits` | `encoding` | Re-evaluating alignment bits per text field |

---

## 3. The 5 Main Remaining Bottlenecks Identified

### Next Hotspot #1: Dynamic Recursion & Heap Allocation in `CompiledSchema::term_has_representation`
- **Location:** [`crates/dfdl-core/src/schema/ir.rs:1793-1806`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/ir.rs#L1793-L1806)
- **Samples:** 118 direct samples + ~150 in `Vec<NodeId>::grow_one` / `malloc` / `free`.
- **Mechanism:**
  ```rust
  pub fn term_has_representation(&self, term_id: NodeId) -> bool {
      let mut visited = alloc::vec::Vec::new();
      self.term_has_representation_impl(term_id, &mut visited)
  }
  ```
  Called by `parse_sequence_members`, `parse_single_element_occurrence`, and `parse_element_with_separators` for every member of every sequence. Every call allocates a `Vec<NodeId>` and recursively walks the schema tree to determine representation.
- **Root Cause:** A term's physical representation is completely static and invariant once compiled.
- **Solution:**
  Precompute representation status during `SchemaBuilder::build()` into a pre-sized lookup table (e.g. `has_representation: Vec<bool>` indexed by `NodeId.0` or a `bool` field on `Term`). Transforms recursive graph search with heap allocation into an $O(1)$ zero-alloc array lookup.

---

### Next Hotspot #2: Whole-Vector Cloning of `in_scope_delimiters` and `in_scope_terminators`
- **Locations:**
  - [`crates/dfdl-core/src/kernel/parser/mod.rs:933-934`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/mod.rs#L933-L934)
  - [`crates/dfdl-core/src/kernel/parser/element.rs:878`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L878)
  - [`crates/dfdl-core/src/kernel/parser/delimiters.rs:866, 880, 898, 904`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/delimiters.rs#L866)
- **Samples:** ~180 samples in `Vec<InScopeDelimiter>::clone`, `_malloc_zone_malloc`, `_xzm_free`.
- **Mechanism:**
  1. On every choice alternative and element occurrence checkpoint:
     ```rust
     let delims_cp = self.in_scope_delimiters.clone();
     let terms_cp = self.in_scope_terminators.clone();
     ```
     Cloning the vectors clones every `InScopeDelimiter` struct, duplicating multiple `String`s.
  2. In `is_in_scope_terminator_longer` and `peek_any_in_scope_delimiter`, the entire delimiter stack is cloned solely to avoid borrow conflicts during iteration.
- **Solution:**
  1. Replace vector clone checkpoints with length snapshots (`let cp = self.in_scope_delimiters.len()`) and rollback via `truncate(cp)` ($O(1)$, zero-alloc).
  2. Replace cloning in `delimiters.rs` with indexed loops `for i in 0..len` or inspecting slices directly without copying.

---

### Next Hotspot #3: Delimiter Splitting, Newline Resolution, and Entity Decoding per Unparsed Delimiter
- **Location:** [`crates/dfdl-core/src/kernel/unparser/framing.rs:178-210, 320-332`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/framing.rs#L178-L210)
- **Samples:** 136 samples in `write_evaluated_delimiter`, 87 samples in `resolve_output_new_line`, 64 samples in `extract_delimiter_tokens`, 52 samples in `decode_unparse_delimiter`.
- **Mechanism:**
  Every time a delimiter is written:
  1. `split_delimiter_alternatives(&eval_str)` splits by whitespace into `Vec<String>`.
  2. `resolve_output_new_line(props)` allocates a `String` clone.
  3. `decode_unparse_delimiter` allocates a `String` and re-scans for DFDL character entities (`%NL;`, `%SP;`).
- **Solution:**
  When delimiters and newline conventions are constant (i.e. `DfdlProp::Constant`, which is true for >95% of schemas including FakeTDL), pre-decode and cache the unparse delimiter strings on `ResolvedProperties` at schema compilation time. Unparsing then outputs the pre-decoded string directly without runtime splitting, entity decoding, or heap allocations.

---

### Next Hotspot #4: Redundant Child Occurrence Counting on Scalar Elements (`maxOccurs == 1`)
- **Locations:**
  - [`crates/dfdl-core/src/kernel/parser/element.rs:806`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L806)
  - [`crates/dfdl-core/src/kernel/parser/element.rs:1399`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L1399)
  - [`crates/dfdl-core/src/kernel/parser/element.rs:1581`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L1581)
- **Samples:** 248 samples in `InfosetBuilder::count_child_occurrences`.
- **Mechanism:**
  Every parsed element invokes `count_child_occurrences` to calculate `self.current_occurs_index = occurrences + 1`. For scalar elements (`max_occurs == Some(1)`), an unparsed element has occurred 0 times by definition. Scanning all existing siblings in the parent element on every scalar field creates an $O(N^2)$ traversal over the document.
- **Solution:**
  - If `elem.max_occurs == Some(1)`, set `current_occurs_index = 1` immediately without calling `count_child_occurrences`.
  - For array elements (`max_occurs != Some(1)`), query `count_child_occurrences` once before entering the loop and increment a local occurrence counter each iteration.

---

### Next Hotspot #5: Quadratic Member Output Checking in Unparser Sequences (`sequence_member_has_output`)
- **Location:** [`crates/dfdl-core/src/kernel/unparser/mod.rs:423-431, 880-912`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs#L423-L431)
- **Samples:** 82 samples in `sequence_member_has_output` + 47 samples in `find_child_elements`.
- **Mechanism:**
  ```rust
  for (m_idx, &member_id) in seq.members.iter().enumerate() {
      let has_subsequent_output = seq.members.get(m_idx + 1..).is_some_and(|subsequent| {
          subsequent.iter().any(|&sub_m| self.sequence_member_has_output(parent_elem, sub_m))
      });
  ```
  `has_subsequent_output` is computed unconditionally for every sequence member. For a sequence of $M$ members, it performs $O(M^2)$ checks, each walking all children of `parent_elem` with string comparisons.
- **Solution:**
  1. Only evaluate `has_subsequent_output` when separators are active (`sep_opt.is_some()`) AND the separator suppression policy requires trailing empty detection (`TrailingEmptyStrict` or `TrailingEmpty`).
  2. Optimize `find_child_elements` to avoid allocating a `Vec` for 0 or 1 matching elements.
