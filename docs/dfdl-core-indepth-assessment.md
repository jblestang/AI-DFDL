# dfdl-core In-Depth Assessment

Independent review of `/workspace/crates/dfdl-core` based on source, unit tests, and build verification (October 2026). Conclusions are **not** taken from prior review markdown in the Project store or repository `review_*.md` files.

---

## Executive summary

`dfdl-core` is a **`#![no_std]` + `alloc` DFDL 1.0 execution stack**: vocabulary types, property/expression machinery, compiled schema IR, bit-addressable I/O, infoset model, and a full **tree-walking parse/unparse kernel**. Roughly **~50k lines** of Rust across **39** `.rs` files, with **295** in-crate unit tests and **4** doc-tests—all passing on **Rust stable 1.99.0** (`rustup run stable cargo test -p dfdl-core`).

The crate is **spec-aware and test-heavy** for core physical parsing (binary/text scalars, delimiters, choices, arrays, discriminators, POU backtracking, variables, assertions). The **`ResolvedProperties` / `DfdlProp<T>` IR** is the architectural anchor: it pre-compiles dynamic properties and discriminators so runtime work is evaluation, not re-parsing XSD.

Gaps are structural rather than cosmetic: **`ResourceLimits` is mostly declarative** (nesting on the infoset builder only; `max_backtrack_bytes` unused in the kernel), **XML schema compilation lives in sibling `dfdl-schema`**, **no Daffodil/MSRC TDML harness inside this crate**, and **hot-path choice backtracking clones delimiter scope vectors**. **`dfdl-vm`** reimplements a subset of the same IR for performance, creating a **dual-engine maintenance burden**.

**Toolchain:** Default VM image **Rust 1.83.0 fails to compile** the crate (`unsigned_is_multiple_of` stability and, in default `no_std`, `f64::abs` in expression rounding). **Rust stable ≥ 1.99** builds and tests cleanly. The workspace does not pin `rust-toolchain.toml`; CI/maintainers should document MSRV explicitly.

---

## Verification record

| Check | Result |
|--------|--------|
| `cargo test -p dfdl-core` (rustc **1.83.0**) | **FAIL** — 9 errors (`is_multiple_of`, `f64::abs`) |
| `rustup run stable cargo test -p dfdl-core` (**1.99.0**) | **PASS** — 295 tests, 4 doc-tests |
| `cargo doc -p dfdl-core --no-deps` | Not run (optional); public surface is small in `lib.rs` |
| Line count (`src/**/*.rs`) | **~43.5k** under `src/` subdirs; top-level modules add ~4k more |

---

## Module-by-module deep dive

### Public API (`lib.rs`)

The exported surface is intentionally narrow: diagnostics, engines, limits, spec metadata, core types, and panic-free utilities (`lib.rs:10-31`). Everything else is reached through submodules (`schema`, `kernel`, `expr`, etc.). **`ParserEngine` / `UnparserEngine`** are the primary runtime entry points; **`CompiledSchema`** is built via `schema::builder::SchemaBuilder` in tests or via **`dfdl-schema`** in full applications.

### `error`

`DFDLErrorKind` mirrors DFDL §3.2 / Appendix F classifications (`error.rs:11-36`). **`ErrorMessage::Static`** supports const-friendly errors (`error.rs:38-97`). **`is_fatal()`** gates POU backtracking: schema definition, work budget, implementation limit, and internal invariant errors must not trigger speculative rollback (`error.rs:115-128`). Parse/unparse/expression errors are generally backtrack-eligible at choice/occurrence POUs (see `kernel/parser/element.rs` uses of `is_fatal()`).

### `spec`

Pins **OGF GFD-R-P.240 DFDL 1.0** with errata flag (`spec.rs:21-27`). **`ConformanceLevel`** and **`FeatureProfile`** are documentation-oriented flags today—the kernel does not branch on `FeatureProfile::extended()` at runtime. Useful for product packaging, not enforcement.

### `types`

QName (Clark notation, prefixes), `InfosetPath`, bit/byte offsets, and **`UnqualifiedPathStepPolicy`** for expression paths (`types.rs` — §23 alignment). **`BitOffset`** helpers use alignment checks (`types.rs:713` area). Heavy use of owned `String` in QNames—appropriate for infoset fidelity, allocation-heavy on hot paths.

### `limits`

`ResourceLimits` defines nesting, array occurrences, backtrack window, expression work, infoset nodes, schema bytes (`limits.rs:12-39`). **`WorkBudget`** is actively consumed by the parser (`kernel/parser/mod.rs` `consume(1)` per term) and expression evaluator (`expr/eval.rs:198`). **`ResourceLimits::check_*`** is only wired into **`InfosetBuilder`** nesting in tests (`infoset/tree.rs:1233` pattern)—not into `ParserEngine`.

### `util`

Checked indexing, fallible `Vec` growth, integer byte reordering, **packed decimal / BCD / IBM4690** codecs (`util.rs` — shared by binary parser and unparser). Establishes the “panic-free contract” cited in module docs.

### `schema/` — IR and builder

**`ir.rs` (~2510 lines)** is the semantic heart. It encodes DFDL constructs as **`TermKind`** (element, sequence, choice, group ref, etc.) with a large **`ResolvedProperties`** struct covering representation, length/delimiter/framing, occurs, nil, text/binary number and calendar facets, escape schemes, dfdlx extensions (`layer`, `repType`, `stringAsXml`), and namespace bindings (`schema/ir.rs:947-1198`).

**`DfdlProp<T>`** distinguishes compile-time constants from pre-parsed expression ASTs (`schema/ir.rs:797-807`)—this is the right model for §6–8 property evaluation at runtime without re-lexing `{...}` strings.

**`builder.rs`** builds IR **programmatically** (no XML). It validates graph integrity (`validate_schema_ir`), variable definitions, and houses tunables like `max_hex_binary_length_in_bytes`. **Full XSD+DFDL compilation is not in this crate**; `dfdl-schema` lowers XML into the same `CompiledSchema`.

**Spec alignment:** IR field names and enums track normative property names closely (length kinds, separator suppression, choice dispatch keys, etc.). **Approximations:** dfdlx properties are first-class in IR; strict “DFDL 1.0 only” profiles would need explicit stripping or gating.

### `io/` — bitstream, sources, sinks

**`BitReader` / `BitWriter`** implement MSBF/LSBF, endianness for multi-byte reads, bit limits, peek/lookahead, and **transactional rollback** via `Checkpoint { bit_position, sequence }` (`io/bitstream.rs:275-296`, `io/traits.rs:28-35`). Rollback rewinds the byte source and replays partial bits—critical for POU correctness.

**`SliceByteSource`**, bounded sources, **`VecByteSink`**, and fixed-slice sinks provide embedded-friendly I/O. **`peek_slice`** on sources enables delimiter scanning optimizations when byte-aligned.

### `expr/` — parser, eval, properties, variables

**Parser (`parser.rs`, pest grammar `dfdl_expression.pest`):** DFDL expression syntax subset; tests reject unsupported constructs (e.g. general comparisons in `expr/mod.rs` tests).

**`eval.rs` (~5708 lines):** Large builtin library (DFDL functions, constructors, path navigation, lookahead, decimal/float rounding). **`MAX_EVAL_DEPTH = 64`** and **`WorkBudget`** consumption guard recursion/CPU (`expr/eval.rs:21`, `:198-200`). Depth and budget tests exist.

**`properties.rs` (~3033 lines):** Property store inheritance, literal validation (no stray whitespace in string properties), byte order parsing, facet hooks—feeds **`dfdl-schema`** and manual IR builds.

**`variables.rs`:** `VariableMap` with define/set/newInstance, direction, runtime external overrides—integrated with parser checkpoints (`element.rs` rollback of `variable_map`).

**Spec alignment:** Strong on §7 variables, §18 expression forms used in schemas, §23 namespace checks (`expr/mod.rs:21-44`). **Gap:** XPath 2.0 full semantics and general comparisons are not implemented (explicitly tested as rejected). Float rounding uses host `f64` (`expr/eval.rs:3216`)—acceptable with `std`/modern libm, fragile on bare `no_std` older toolchains.

### `pattern.rs`

Bridges DFDL’s ICU-style **`(?=…)` lookahead** to the `regex` crate by rewriting to empty capture groups (`pattern.rs:1-21`, `:94-98`). Documents limitations (positive lookahead only, must be trailing). Used for `lengthPattern`, discriminators, test patterns—**pragmatic spec approximation**, not a Java regex engine.

### `encoding.rs` (~442 lines in file header region, ~1k total)

§11 character encodings: UTF-8/16, ASCII, Latin-1, EBCDIC tables, **sub-byte and packed encodings** (5/6/7-bit, ICAO) with strict/replacement policies (`encoding.rs:453-692` region). Central to text parsing and delimiter matching in non-byte encodings (`ParserEngine.delim_encoding`).

### `infoset/`

**`value.rs`:** `DfdlValue` / `DfdlSimpleType`, coercions, EBV rules for expressions.

**`state.rs`:** Value vs nil vs absent vs default.

**`tree.rs` (~1420 lines):** DOM-style **`InfosetBuilder`** with checkpoint/rollback (mirrors bitstream for POU), event round-trip, **`ResourceLimits`** on nesting.

**`events.rs`:** Streaming event model for alternative consumers.

Aligned with §4–5 infoset modeling; validation mode can accumulate facet errors on the parser/unparser before failing the document.

### `kernel/` — parser, unparser, layer

**Architecture:** Monolithic **`ParserEngine`** in `parser/mod.rs` (~1696 lines) dispatches **`element.rs`** (~3300 lines), plus focused submodules:

| Submodule | Lines (approx.) | Role |
|-----------|-----------------|------|
| `binary.rs` | 872 | Binary scalars, floats, decimals |
| `text.rs` | 1456 | Text scalars, charset alignment |
| `numbers.rs` | 1891 | Text number parsing/formatting |
| `calendar.rs` | 2180 | Text/binary calendar |
| `delimiters.rs` | 1717 | Initiator/terminator/separator scanning |
| `rounding.rs` | 285 | Text number rounding modes |
| `tests.rs` | 3583 | Kernel parser integration tests |

**POU model** (`parser/mod.rs:43-70`, `:1233-1388`): Choice and occurrence POUs; **`is_discriminated`** freezes backtracking after initiator match or discriminator success. Extensively documented test for nested choice + sequential discriminators (`parser/tests.rs:1435-1457`).

**Delimiter scoping:** `in_scope_delimiters` / `in_scope_terminators` cloned on each choice branch attempt (`parser/mod.rs:1254` area)—correctness-first, allocation-heavy.

**`ValidationMode`** (`parser/mod.rs:31-41`): Off (default), Limited, Full—facet/assert handling in `element.rs`; not a complete §21 XSD validator.

**`unparser.rs` (~6246 lines):** Symmetric unparsing—binary/text, padding, escape schemes, layers, nil handling. Same property IR drives both directions.

**`layer.rs`:** dfdlx layer transforms (IPv4 checksum, check digit, RFC 5322 unfolding, compression)—extension territory beyond baseline §20.

**Correctness mechanisms (summary):**

- Triple checkpoint: **bit reader + infoset builder + variable map** on speculative paths.
- Fatal errors suppress backtrack (`error.rs:121-128`, `element.rs:1058`).
- Work budget exhaustion treated as fatal for backtracking (`kernel/mod.rs` `test_choice_budget_exhaustion_does_not_backtrack`).
- Choice explicit length enforcement interacts with discriminated POU (`parser/mod.rs:1286-1297`).

### `kernel/parser/element.rs` (cross-cutting)

Single largest module—element occurrence loops, nil/empty policies, input/output value calc, hidden groups, assertions, discriminator evaluation, separator suppression. **Maintainability risk:** any spec fix in element lifecycle requires navigating thousands of lines.

---

## Cross-cutting themes

### 1. Spec alignment (DFDL 1.0 §§3–20, 23)

| Area | Assessment |
|------|------------|
| Physical properties (§12–16) | **Strong** — encoded in IR and exercised in kernel tests |
| Data syntax / POU (§9) | **Strong** — explicit stack, discriminator tests, initiated content |
| Properties & expressions (§6–8, §18) | **Good** — inheritance, variables, large builtin set; not full XPath 2.0 |
| Infoset (§4–5) | **Good** — DOM builder + events |
| Validation (§21) | **Partial** — modes exist; “Full” is not a complete XSD validator |
| Namespaces (§23) | **Good** — path policy + expression namespace validation |
| Regex (§12.3.2, §7.3.1) | **Approximated** via `regex` + lookahead rewrite |
| Calendar/numbers | **Substantial custom logic** — not ICU/JDK; behavior proven by unit tests, not MSRC vectors |

### 2. Architecture and maintainability

Clear **layering**: `io` → `kernel` → `infoset`, with `schema::ir` and `expr` as shared compile-time/runtime bridges. **`dfdl-schema`** and **`dfdl-vm`** sit adjacent—core remains the semantic reference implementation.

**Pain points:** God-modules (`element.rs`, `unparser.rs`, `eval.rs`), duplicate engine logic in VM, IR struct growth (250+ lines of fields in `ResolvedProperties`).

### 3. Correctness mechanisms

POU + checkpoint trilogy is **sound engineering** and **test-documented**. Delimiter rollback restores cloned vectors—correct but **O(branch × delimiter depth)** allocations on choice failure.

### 4. Safety and embedded readiness

`no_std` + fallible allocation patterns are consistent. **`WorkBudget`** is real; **`ResourceLimits`** mostly aspirational in the kernel. No `#![deny(unused)]` panics observed; **`unwrap`/`expect`** confined to tests per module attrs.

### 5. Performance

Tree-walking with **clone-on-backtrack** (delimiters, sometimes full parser state in `with_sub_engine` — `parser/mod.rs:1485-1491`). **Pre-compiled `DfdlProp` ASTs** avoid parse overhead. **`dfdl-vm`** exists precisely to avoid this profile for binary subsets.

### 6. Test coverage quality

**295** focused unit tests—excellent **branch and error-cluster** coverage in `kernel/parser/tests.rs` and `expr/eval.rs` (42 tests). **Proven:** POU, discriminators, separator suppression, rounding, encodings, variables, escape/unparse, layers.

**Not proven in this crate:** End-to-end **Daffodil TDML** suites (live in `dfdl-tests` with `dfdl-core` **`std`** feature). Property combination explosion, fuzz regressions (`dfdl-fuzz`), and VM/compiler parity are **out of crate scope**.

### 7. Technical debt and dfdl-vm duplication

`dfdl-vm` consumes **`CompiledSchema`** and documents itself as replacing tree-walking for line-rate decode (`dfdl-vm/src/lib.rs:8-23`). Compiler coverage is **binary-biased**; text, calendar, and rich delimiter stories remain kernel-only. **Risk:** spec fixes must be applied twice until VM reaches parity or kernel shrinks to reference profile.

---

## Strengths and weaknesses

| Strengths | Weaknesses |
|-----------|------------|
| Rich, spec-labeled **`ResolvedProperties` + `DfdlProp<T>`** IR | **`ResourceLimits` not enforced** on parse backtrack/array growth |
| **POU + triple checkpoint** with explicit fatal-error policy | **Large monolith files** hinder review and safe change |
| **295** in-crate tests covering edge cases many engines skip | **No TDML/MSRC** in-crate; integration tests elsewhere |
| **`no_std`** I/O and codecs suitable for embedded | **Toolchain fragility** on 1.83; MSRV undocumented |
| **Sub-byte encodings + pattern lookahead** handled deliberately | **Regex/calendar/number** semantics approximated vs reference JDK/ICU |
| Symmetric **unparser** sharing IR | **Dual engine** (`kernel` vs `dfdl-vm`) duplication |
| Panic-free utilities and typed errors | Choice backtrack **clones delimiter stacks** |

---

## Risk register

| ID | Pri | Risk | Evidence / impact |
|----|-----|------|-------------------|
| R1 | **P0** | Undocumented MSRV; **1.83 fails to build** | CI/env snapshot default; `is_multiple_of`, `f64::abs` |
| R2 | **P1** | **`max_backtrack_bytes` unused** — unbounded speculative read on hostile input | `limits.rs:18`; no kernel references |
| R3 | **P1** | **Kernel vs VM behavioral drift** as schemas evolve | Separate execution paths; VM skips validation |
| R4 | **P1** | **God-module regressions** in `element.rs` / `unparser.rs` | ~3300 / ~6246 lines |
| R5 | **P2** | **Regex lookahead rewrite** mismatch vs Java ICU on exotic patterns | `pattern.rs:19-21` documented limits |
| R6 | **P2** | **`ValidationMode::Full`** oversold vs real XSD validation | `parser/mod.rs:31-41` |
| R7 | **P2** | **`no_std` float expr** without `std` feature on older rustc | `expr/eval.rs:3216` |
| R8 | **P3** | **`FeatureProfile` / `ConformanceLevel` non-enforcing** | `spec.rs:29-79` |
| R9 | **P3** | Performance: delimiter vector clone per choice branch | `parser/mod.rs:1254+` |

---

## Recommendations (prioritized for dfdl-core maintainers)

1. **Pin MSRV** in workspace `rust-toolchain.toml` (≥ version that stabilizes `is_multiple_of` and compiles expr float helpers under crate features). Verify `cargo test -p dfdl-core` on that pin in CI.

2. **Wire `ResourceLimits` into `ParserEngine`**: track bytes read since last committed POU; enforce `max_backtrack_bytes`, `max_array_occurrences`, and `max_infoset_nodes` at builder append sites.

3. **Extract POU/backtrack orchestration** from `element.rs` into a dedicated module (~500 lines) with table-driven tests—reduce regression risk before VM parity work.

4. **Document engine selection**: kernel = reference + full spec surface; VM = fast path subset—publish matrix in crate docs (which `LengthKind`/representations VM supports).

5. **Share delimiter scope via copy-on-write or stack depth indices** instead of cloning `Vec` on every choice trial.

6. **Add a thin integration test** in `dfdl-core` (behind `std` dev-dependency) that runs one TDML case from `dfdl-tests`—optional but catches crate-only drift.

7. **Clarify `ValidationMode`** in public docs: what Limited/Full actually check vs §21.

8. **Split `ResolvedProperties`** into sub-structs (framing, text, binary, calendar) generated or composed—lowers merge conflict rate.

9. **For `no_std` builds**, gate float rounding in `eval.rs` on `feature = "std"` or use integer-only decimal paths for embedded.

10. **After large IR/kernel changes**, run `graft build` per repo policy and extend `kernel/parser/tests.rs` clusters for any new §9.2 edge case.

---

## Appendix: complexity signals

| File | Lines |
|------|------:|
| `kernel/unparser.rs` | 6246 |
| `expr/eval.rs` | 5708 |
| `kernel/parser/element.rs` | 3300 |
| `expr/properties.rs` | 3033 |
| `schema/ir.rs` | 2510 |
| `kernel/parser/calendar.rs` | 2180 |
| `kernel/parser/delimiters.rs` | 1717 |
| `kernel/parser/mod.rs` | 1696 |
| `kernel/parser/tests.rs` | 3583 |
| `encoding.rs` | (top-level ~442 + tables) |

Public re-exports: ```23:31:/workspace/crates/dfdl-core/src/lib.rs```

POU frame definition: ```59:70:/workspace/crates/dfdl-core/src/kernel/parser/mod.rs```

Fatal error policy: ```115:128:/workspace/crates/dfdl-core/src/error.rs```

Bitstream rollback: ```275:296:/workspace/crates/dfdl-core/src/io/bitstream.rs```

Compiled property model (excerpt): ```797:807:/workspace/crates/dfdl-core/src/schema/ir.rs```, ```947:976:/workspace/crates/dfdl-core/src/schema/ir.rs```

---

*Assessment author: Cloud Agent worker (source-derived). Toolchain: Rust 1.99.0 stable for verification; 1.83.0 documented failure.*
