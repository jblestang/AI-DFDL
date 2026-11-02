# Profiling Campaign v4 Review — FakeTDL Benchmark

**Date**: 2026-10-29 19:58:00 CET  
**Scope**: Post-Hotspots 1, 2, 4, 5 optimization profile (`profile_faketdl_v4.txt`)  
**Methodology**: macOS `sample` at 1 kHz with 100,000 iterations (`BENCH_ITERS=100000`).

---

## 1. Executive Summary & Progression

After implementing optimizations for Hotspots #1, #2, #4, and #5, the DFDL parser latency dropped from 16.68 µs to **5.77 µs** (-65.5%), and multi-message parse latency dropped from 69.18 µs to **20.90 µs** (-72.7%).

The v4 sampling profile reveals that:
1. **Unparser now dominates execution time**: In roundtrip and file operations, unparsing takes ~65% of the total CPU time.
2. **Delimiter resolution and formatting** in the unparser creates significant string and vector churn (`split_delimiter_alternatives`, `resolve_output_new_line`, `decode_unparse_delimiter`, `encode_text_string`).
3. **`BitWriter` lacks a `write_bytes` bulk fast-path**: Text and delimiter bytes are written bit-by-bit via `write_bits(b as u64, 8)`.
4. **`VariableMap` clone on engine creation**: Every parse and unparse clones 4 predefined DFDL variables (`encoding`, `byteOrder`, `binaryFloatRep`, `outputNewLine`).
5. **`encoding_char_bits` dynamic substring scanning**: Called on every element and delimiter check, executing up to 8 case-insensitive substring searches.

---

## 2. Top Hotspots Identified (Profile v4)

### Hotspot #1 — Unparser Delimiter Resolution Churn (`framing.rs`)
- **Profile Impact**: ~560 samples across `split_delimiter_alternatives` (117), `resolve_output_new_line` (124), `write_evaluated_delimiter` (127), and `extract_delimiter_tokens` (74).
- **Location**: [`crates/dfdl-core/src/kernel/unparser/framing.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/framing.rs#L137-L232)
- **Root Cause**:
  1. `split_delimiter_alternatives(&eval_str)` splits all alternatives into a `Vec<String>`, only for the caller to take `.first()`.
  2. `resolve_output_new_line(props)` is called unconditionally even when the delimiter contains no `%NL;` entity token.
  3. `decode_unparse_delimiter` loops character-by-character and builds a new `String` even when the delimiter has no `%` entity token (e.g. `","` or `"\n"`).
  4. For UTF-8/ASCII encodings, `encode_text_string(&decoded, &props.encoding)` allocates a `Vec<u8>` every time, even though `decoded.as_bytes()` is already the encoded byte slice.
- **Proposed Solution**:
  - Implement zero-alloc `first_delimiter_alternative(&str) -> &str` avoiding `Vec<String>`.
  - Fast-path `decode_unparse_delimiter`: if `!val.contains('%')`, return `val` directly without allocation (`Cow<'_, str>`).
  - Only call `resolve_output_new_line` if `first_alt.contains("%NL;")`.
  - For UTF-8 / ASCII, write `decoded.as_bytes()` directly without calling `encode_text_string`.

---

### Hotspot #2 — `BitWriter` Missing `write_bytes` Bulk Fast-Path (`bitstream.rs`)
- **Profile Impact**: High cumulative overhead in `BitWriter::write_bits` called byte-by-byte across all text and delimiter writes.
- **Location**: [`crates/dfdl-core/src/io/bitstream.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/io/bitstream.rs#L391-L430)
- **Root Cause**:
  - `BitReader` has a dedicated `read_bytes(&mut [u8])` method that bypasses bit manipulation when byte-aligned (`self.bits_in_buf == 0`), delegating directly to `ByteSource::read_bytes`.
  - `BitWriter` has no `write_bytes(&[u8])`. Every byte of text, number string, or delimiter is sent through `write_bits(b as u64, 8)`, running shift and mask loops.
- **Proposed Solution**:
  - Add `pub fn write_bytes(&mut self, buf: &[u8]) -> DFDLResult<()>` to `BitWriter`.
  - When `self.bits_in_buf == 0`, delegate directly to `self.sink.write_bytes(buf)`. (For `VecByteSink`, this is an instant `extend_from_slice` `memcpy`).
  - When unaligned, fall back to writing 8-bit chunks.

---

### Hotspot #3 — Redundant `VariableMap` Cloning on Engine Instantiation
- **Profile Impact**: 63 samples in `Vec<DfdlVariable>::clone` + 21 samples in `drop_glue(Vec<DfdlVariable>)` + 40 samples in engine setup.
- **Location**: [`crates/dfdl-core/src/kernel/parser/mod.rs:99`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/mod.rs#L99) and [`crates/dfdl-core/src/kernel/unparser/mod.rs:62`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser/mod.rs#L62)
- **Root Cause**:
  - `ParserEngine::new(schema)` and `UnparserEngine::new(schema)` execute `variable_map: schema.variable_map.clone()`.
  - `VariableMap::new()` pre-populates 4 standard DFDL variables (`encoding`, `byteOrder`, `binaryFloatRep`, `outputNewLine`), each holding a `QName` and `String`.
  - When schemas don't define variables or when variables are never mutated, cloning these 4 items per message allocation is pure overhead.
- **Proposed Solution**:
  - Use Copy-on-Write (`Cow`) or lazy cloning for `VariableMap`, or share immutable predefined variables via reference / `Arc` so that `clone()` does not reallocate strings and vectors.

---

### Hotspot #4 — `encoding_char_bits` Dynamic Substring Search
- **Profile Impact**: 193 samples directly in `encoding_char_bits` + 63 in `encoding_mandatory_alignment_bits`.
- **Location**: [`crates/dfdl-core/src/encoding.rs:533`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/encoding.rs#L533)
- **Root Cause**:
  - `encoding_char_bits(&props.encoding)` is called repeatedly across parser and unparser loops.
  - Each invocation executes `.trim()` and up to 8 case-insensitive string searches (`contains_ascii_ci(enc, "BITS")`, etc.). Standard encodings like `"UTF-8"` or `"US-ASCII"` execute all 8 negative searches.
- **Proposed Solution**:
  - Add fast-path checks for `"UTF-8"`, `"ASCII"`, `"US-ASCII"`, `"ISO-8859-1"` returning `None` immediately in $O(1)$.
  - Precompute `encoding_char_bits: Option<usize>` in `ResolvedProperties` at schema compilation time.

---

### Hotspot #5 — Unicode Trimming in Delimiter and Text Scanning
- **Profile Impact**: ~575 samples across `CharSearcher::next_match` (240), `trim_matches(is_whitespace)` (234), and `next_match_back` (101).
- **Location**: [`crates/dfdl-core/src/kernel/parser/text.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/text.rs) and [`crates/dfdl-core/src/kernel/parser/delimiters.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/delimiters.rs)
- **Root Cause**:
  - `trim_matches(char::is_whitespace)` decodes UTF-8 codepoints and tests large Unicode character tables.
  - When delimiters or texts are ASCII, or when trimming is unnecessary because no surrounding whitespace exists, this incurs heavy pattern-search overhead.
- **Proposed Solution**:
  - Use ASCII byte trimming (`trim_ascii()`) for ASCII-encoded contexts.
  - Check `is_ascii()` or test endpoints before engaging full Unicode character trimming.

---

## 3. Summary of Prioritization

| # | Optimization | Complexity | Projected Impact | Primary Target |
|---|---|---|---|---|
| **1** | Zero-alloc unparser delimiter resolution & entity decoding | Low | High (Unparser) | `framing.rs` |
| **2** | Add `write_bytes` bulk fast-path to `BitWriter` | Low | High (Unparser & Codecs) | `bitstream.rs` |
| **3** | Lazy / zero-alloc `VariableMap` cloning | Medium | Medium-High (Parse & Unparse) | `variables.rs`, `parser/mod.rs`, `unparser/mod.rs` |
| **4** | Precompute `encoding_char_bits` / fast-path standard encodings | Low | Medium (Parse & Unparse) | `encoding.rs`, `ir.rs` |
| **5** | ASCII-optimized trimming for delimiters and text values | Low | Medium (Parser) | `text.rs`, `delimiters.rs` |
