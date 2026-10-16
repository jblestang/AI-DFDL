# DFDL 1.0 (OGF GFD-R-P.240) vs Implementation Gap Analysis

**Scope:** `/workspace` — `dfdl-schema` (compile-time SDEs), `dfdl-core` (parser/unparser kernel, expressions, patterns, encodings, limits, validation).  
**Related:** [dfdl-ir-standard-implementation-gaps.md](dfdl-ir-standard-implementation-gaps.md) (compiled IR), [dfdl-schema-standard-implementation-gaps.md](dfdl-schema-standard-implementation-gaps.md) (compiler-only).  
**Evidence:** Source and error strings only (not prior store reviews or stale conformance matrices).  
**TDML:** `rustup run stable cargo test -p dfdl-tests test_apache_daffodil_official_tdml_suite -- --ignored` — **passed** (1/1, ~10.8s). Official Apache Daffodil TDML corpus therefore matches this engine today; remaining gaps are **documented rejections**, **silent subsets**, and **spec features outside TDML coverage**.

---

## Executive summary — top 10 gaps (user impact)

| Rank | Gap | Impact |
|------|-----|--------|
| 1 | **`lengthKind='endOfParent'` compile-time reject** | Common “consume rest of parent” layouts fail at schema compile. |
| 2 | **`dfdl:binaryFloatRep` not wired to kernel** | Only IEEE bit patterns decoded; `vax` / `ibm` (§13.6) would parse wrong if accepted. |
| 3 | **`ValidationMode::Limited` / `Full` indistinguishable** | No XML Schema Part 1 “full” validation; §21 profile not honored. |
| 4 | **`ResourceLimits` mostly unenforced** | DoS limits documented in API but only infoset nesting depth applied; parser uses separate `WorkBudget`. |
| 5 | **Expression §18 subset** | Wildcards, general comparisons, `fn:trace`, many XPath 2 functions missing; `fn:matches` is substring not regex. |
| 6 | **Regex §7.3.1 / ICU** | Negative look-ahead, look-behind, `\U` rejected; positive `(?=)` rewritten with documented limitations. |
| 7 | **`textNumberRep='zoned'`** | COBOL-style zoned decimal rejected (compile or parse). |
| 8 | **Nested `lengthKind='prefixed'`** | Rejected at compile (schema) and unparse (runtime). |
| 9 | **`textBidi='yes'` / `floating='yes'`** | Compile-time SDE — bidirectional and floating alignment absent. |
| 10 | **Daffodil extensions (`dfdlx`/`daf`) vs pure OGF** | `repType`, layer transforms, bitwise ops, `stringAsXml` — required for TDML but not normative OGF 1.0. |

---

## Gap table

| Clause (GFD-R-P.240) | Requirement | Implementation | Gap type |
|----------------------|-------------|----------------|----------|
| §12.3 / Table 12 | `lengthKind` includes `endOfParent` | SDE: `lengthKind='endOfParent' is not implemented` — `lower.rs:1529-1540`; IR enum exists (`ir.rs:60-61`) but compile rejects | **Missing** |
| §12.3.2 | Nested `prefixed` length | SDE: `Nested dfdl:lengthKind="prefixed" is not supported` — `lower.rs:1007`; unparse: `unparser.rs:2028` | **Missing** |
| §12 / §11 | Variable-width encoding + `lengthUnits='characters'` on complex types | SDE at unparse `unparser.rs:1525`; runtime SDE parse `element.rs:2222` | **Subset** |
| §13.6 | `binaryFloatRep`: `ieee`, `vax`, `ibm` | Float/double always `f32/f64::from_bits` after byte swap — `binary.rs:536-553`; property not in `ResolvedProperties`; `dfdl:binaryFloatRep` accessor hardcodes `"ieee"` — `eval.rs:630` | **Divergent** / **Missing** |
| §13.7 | Packed decimal / BCD / IBM4690 | Implemented in `util.rs` + `binary.rs:208-249`; compile rejects implicit length on packed calendar/numbers where spec allows other combinations — `lower.rs:2204`, `2778-2784` | **Subset** |
| §13.7.1 | `textNumberRep='zoned'` | SDE: `textNumberRep="zoned" is not supported` — `text.rs:975`; compile also rejects — `lower.rs:2329`, `2373` | **Missing** |
| §13.11 | `binaryCalendarRep='binaryMilliseconds'` on date/time | SDE: `binaryCalendarRep='binaryMilliseconds' is not allowed on element ... date/time` — `lower.rs:2751-2755` | **Missing** (dateTime may differ) |
| §13.11 | Calendar pattern fractional seconds | SDE if >9 fractional digits — `lower.rs:2652` | **Subset** |
| §13.11 | BCD calendar patterns | Restricted character sets — `lower.rs:2665-2686`, `2818` | **Subset** |
| §7.4 | Standard accessor variables reflect effective properties | `encoding`→`UTF-8`, `byteOrder`→`bigEndian`, `binaryFloatRep`→`ieee`, `outputNewLine`→`\n` hardcoded in `lookup_variable` — `eval.rs:627-631` | **Divergent** |
| §18 / §23 | XPath subset; no `*` wildcard steps | SDE: `Wildcard '*' is unsupported` — `parser.rs:361` | **Subset** |
| §18 | Value comparisons only (`eq`, `ne`, …) | General `=`, `!=`, `<`, … SDE — `parser.rs:78`, `92` | **Subset** |
| §18 | Built-in functions (XPath / XQuery) | Large custom set in `eval.rs`; unknown → `Unsupported function` — `eval.rs:3243`; `fn:trace` prohibited — `329`, `3231` | **Subset** |
| §18 | `fn:matches` regex semantics | Uses `String::contains` on pattern literal — `eval.rs:2437-2442` | **Divergent** |
| §18 | `dfdl:currentPosition()` | Always returns `0` — `eval.rs:2824` | **Divergent** |
| §18 | `dfdl:lookAhead` distance | Cap 512 bits — `eval.rs:2806-2811` | **Subset** |
| §7.3.1 | ICU/Java regex for `testPattern`, `lengthPattern` | `pattern.rs`: only trailing `(?=)` rewrite; `(?! )`, look-behind, `\U` rejected — `pattern.rs:59`, `101-103`; engine is `regex` crate not ICU | **Subset** / **Divergent** |
| §21 | Validation modes off / limited / full | Enum `ValidationMode` — `parser/mod.rs:33-40`; kernel only checks `!= Off` — no `Limited` vs `Full` branch anywhere | **Subset** |
| §21 | Facet validation when validation on | `facets.validate_value_detailed` queued in `validation_errors` — `element.rs:1680-1686`, `3037-3048`; not full XSD component model | **Subset** |
| §15.1.4 | Array choice branch defaults | SDE: `(subset) Default value on array choice branch ... not implemented` — `groups.rs:297` | **Missing** |
| §9.2 / §14 | Points of uncertainty / backtracking | `pou_stack`, choice rollback — `parser/mod.rs:1233-1370`; `WorkBudgetExhausted` is fatal (no backtrack) — `error.rs:121-128`, `mod.rs:1394+` | **Subset** (resource interaction) |
| §12 | `floating='yes'` alignment | SDE — `properties.rs:1020-1025` | **Missing** |
| §12 | `textBidi='yes'` | SDE — `properties.rs:1012-1017` | **Missing** |
| §5 / XSD | Derived enumeration ⊆ base | Enforced — `lower.rs:27-57`, `711` | **Conformant** (stricter compile) |
| — | `dfdlx:layerTransform` / `daf:layerTransform` | Whitelist `is_known_layer` — `lower.rs:3575-3597`; unknown → SDE `802` | **Extension** |
| — | `dfdlx:repType`, `repValues`, `choiceBranchKeyRanges` | Compile + kernel rep mapping — `elements.rs`, `lower.rs:1817+` | **Extension** |
| — | `dfdlx:bitAnd/Or/Xor/Not`, shifts | `eval.rs:2939-3067` | **Extension** |
| — | `dfdlx:runtimeProperties stringAsXml` | Post-parse XML well-formed check — `element.rs:3032-3054` | **Extension** |

---

## Deep dive: Points of uncertainty (POUs)

**Spec (§9.2, §14.3):** Choices, optional occurrences, and initiated content create points where the processor may backtrack until data is committed.

**Implementation:**

- **POU stack:** `PointOfUncertainty` / `PouKind::Choice` / `Occurrence` — `parser/mod.rs:43-70`, pushed on choices — `1233-1238`.
- **Backtracking:** On branch failure, reader, infoset builder, variable map, and validation error list are rolled back — `1250-1269`, `1364-1369`.
- **Discriminator resolution:** `on_initiator_matched` marks POUs discriminated — `396-407`; discriminated POUs abort choice retry with aggregated error — `1349-1361`.
- **Work budget interaction:** `budget.consume(1)` per term — `mod.rs:411`; exhaustion yields `WorkBudgetExhausted`, classified **fatal** — `error.rs:121-128`. Choice parsing does not backtrack on budget exhaustion — tested in `kernel/mod.rs:1394-1468`.

**Gap:** Spec resource-limit behavior vs unbounded backtrack is not fully specified in this analysis, but coupling **fatal** budget errors with POU retry differs from engines that treat limits as independent of speculative parse attempts. Document as **Subset/Divergent** for §9.2 operational semantics under resource caps.

---

## Deep dive: Delimiters, terminators, escapes

**Spec (§12, §12.2, §12.3):** Separators, initiators, terminators, `%ES;`, escape schemes, in-scope delimiter stacks.

**Implementation:**

- **In-scope stacks:** `in_scope_delimiters` / `in_scope_terminators` — `parser/mod.rs:79-80`, pushed per sequence/element framing.
- **Encoding-aware matching:** `delim_encoding`, `delim_ignore_case` scoped per term — `parse_term` — `mod.rs:186-196`.
- **Compile rules:** `%ES;` in own terminator when `lengthKind='delimited'` → SDE — `lower.rs:1543-1548`.
- **Complex + characters + variable encoding:** Rejected (see table).

**Gaps:** Sub-byte and non-UTF-8 delimiter matching is partially handled (`delim_encoding` comment — `mod.rs:83-84`) but variable-width **character** lengths on complex types are explicitly unsupported. **Floating** alignment (delimited scanning without fixed anchor) rejected at property resolution.

---

## Deep dive: Expression language (§18)

**Parser (`expr/parser.rs`):**

- Rejects wildcard steps, general comparisons (requires `eq`/`ne`/`lt`/…).
- Builds AST for literals, paths, `if`, function calls.

**Evaluator (`expr/eval.rs`):**

- **Supported:** Core `fn:` string/numeric/boolean functions (see match arms ~2165-3228), DFDL `dfdl:lookAhead`, `occursIndex`, `checkConstraints`, type constructors, Daffodil test hooks (`sayHello`, `addBoxed`, …), **`dfdlx:` bitwise**.
- **Missing / wrong:** `fn:trace`; non-`fn`/`dfdl` prefixed `replace`; arbitrary XPath 2 library; **`fn:matches` ≠ regex**; **`currentPosition` stub**; **`count`/`exists` simplified** (e.g. `count` — `2826-2834`).
- **Namespace validation:** `validate_expression_namespaces` requires declared prefix for `fn:*` — `mod.rs:909-914` (may reject valid schemas if `fn` prefix not declared in expression in-scope namespaces).

**Property accessors (§7.4):** Several `$dfdl:*` reads ignore live property evaluation (hardcoded defaults).

---

## Deep dive: Calendar (§13.11)

**Supported:** Text calendar with patterns; binary seconds/milliseconds paths where compiled; BCD/packed with explicit lengths and pattern-kind rules; `calendarCheckPolicy` lax/strict in IR — `properties.rs:2268+`, kernel `calendar.rs`.

**Compile-time rejections (normative features absent):**

- `binaryMilliseconds` on `xs:date` / `xs:time` — `lower.rs:2748-2755`.
- Pattern punctuation in BCD date patterns (e.g. `-`) — tests `compiler/tests.rs:951`.
- More than nine fractional-second pattern symbols — `lower.rs:2652`.

**Gap type:** **Missing** for disallowed rep combinations; **Subset** for pattern expressiveness.

---

## Deep dive: Binary numbers and floats (§13)

**Integers / decimal:** Binary twos-complement with explicit bit lengths; packed/BCD/IBM4690 via `util.rs` and `binary.rs`.

**Float/double:** Always IEEE754 bitwise with `byteOrder` swap — `binary.rs:536-553`. No branch on `binaryFloatRep` property (property not resolved into `ResolvedProperties` in `expr/properties.rs`).

**Text integers:** `textStandardBase` restricted for float/double/decimal — `lower.rs:2705-2730`.

**Zoned decimal text:** Not implemented — `text.rs:975`.

---

## Resource limits vs specification

| Limit | `ResourceLimits` field | Enforced where |
|-------|------------------------|----------------|
| Nesting depth | `max_nesting_depth` | `InfosetBuilder::push_event` — `tree.rs:699` |
| Array occurrences | `max_array_occurrences` | **Not referenced** in kernel |
| Backtrack window | `max_backtrack_bytes` | **Not referenced** |
| Expression work | `max_expression_work` | **Not referenced** (expressions use `WorkBudget` passed ad hoc) |
| Infoset nodes | `max_infoset_nodes` | **Not referenced** |
| Schema size | `max_schema_bytes` | **Not referenced** in grep sweep |

**WorkBudget:** Parser/unparser decrement per operation; default budgets in tests only — callers supply `WorkBudget::new(n)`; no automatic wiring from `ResourceLimits::max_expression_work`.

**Comment drift:** `limits.rs:4` states all operations evaluate against `ResourceLimits`; implementation does not match.

---

## Properties in IR / property store but weak or absent in kernel

| Property | In IR / compile | Kernel behavior |
|----------|-----------------|-----------------|
| `binaryFloatRep` | Accepted in XSD/TDML fixtures | Ignored for decode; accessor hardcoded |
| `endOfParent` | Parsed in property layer (`properties.rs:1041`) | Compile reject in `lower.rs` |
| `ValidationMode::Limited/Full` | API surface | Treated same as each other |
| `ResourceLimits` (most fields) | Public API | Only nesting on infoset builder |

---

## Daffodil extensions vs pure OGF profile

Implementations required for Apache TDML pass but **outside strict OGF DFDL 1.0:**

- **`dfdlx:repType` / rep value maps** — enumeration wire format — `elements.rs`, `element.rs:2120+`.
- **`dfdlx:choiceBranchKeyRanges`** — dispatch — `lower.rs:3186+`, `parser/mod.rs:1180+`.
- **`dfdlx:layerTransform` / `daf:`** — gzip, base64, IPv4 checksum, boundary mark, etc. — `layer.rs`, `is_known_layer`.
- **`dfdlx:objectKind='bytes'`** — requires explicit/prefixed length — `lower.rs:1562+`.
- **`dfdlx:parseUnparsePolicy` / `daf:`** — aliased with `dfdl:` — `properties.rs:1860+`.
- **Test-only expression functions** — `sayHello`, `addBoxed`, … — `eval.rs:2907+`.

Pure OGF schemas using only standardized properties may still hit **Missing** rows above (endOfParent, zoned, vax floats, etc.).

---

## TDML run note

```
rustup run stable cargo test -p dfdl-tests test_apache_daffodil_official_tdml_suite -- --ignored
→ ok (1 passed)
```

The bundled corpus exercises Daffodil-aligned schemas (mostly `ieee`, no endOfParent in passing cases). **Passing TDML does not imply full GFD-R-P.240 coverage** — compile-time SDEs and silent subsets above remain.

Default workspace `cargo test` without stable toolchain **fails to compile** (`unsigned_is_multiple_of` unstable on non-stable); use stable for verification.

---

## Recommendations

### Conformance documentation

1. Publish a **normative gap appendix** keyed to this table (clause → behavior → error string).
2. Replace stale matrices with **machine-generated** lists from `rg 'not implemented|is not supported|Unsupported|subset)' crates/dfdl-schema crates/dfdl-core`.
3. State **validation profile explicitly:** “facet checks when `ValidationMode != Off`; no XSD Part 1 schema-validity assessment.”
4. Document **regex engine** as “Rust `regex` + limited `(?=)` rewrite”, not ICU.
5. Label **Daffodil TDML profile** vs **OGF-only profile** (extensions whitelist).

### Implementation fixes (priority)

1. **`endOfParent`** — implement in kernel or reject earlier in property resolution only (avoid IR drift).
2. **`binaryFloatRep`** — add to `ResolvedProperties`, branch in `binary.rs`, fix `$dfdl:binaryFloatRep` accessor.
3. **`ValidationMode::Full`** — either integrate XSD validation or remove enum variant.
4. **Wire `ResourceLimits`** to parser (`max_array_occurrences`, `max_infoset_nodes`, `max_expression_work` ↔ `WorkBudget`).
5. **`fn:matches`**, **`currentPosition`**, **`$dfdl:encoding`/`byteOrder`** — align with spec or document as permanent subsets.
6. **ICU or spec-accurate regex** for patterns if IBM/OGF test suites require look-behind.

---

## Appendix: Explicit compile/runtime rejection strings (source scan)

Representative SDE / error messages from `dfdl-schema` + `dfdl-core`:

- `lengthKind='endOfParent' is not implemented` — `lower.rs:1536`
- `Nested dfdl:lengthKind="prefixed" is not supported` — `lower.rs:1007`, `unparser.rs:2028`
- `textBidi='yes' is not supported` / `floating='yes' is not supported` — `properties.rs:1016`, `1024`
- `textNumberRep="zoned" is not supported` — `text.rs:975`
- `Wildcard '*' is unsupported in DFDL Expression Syntax` — `parser.rs:361`
- `General comparison operator ... is not supported` — `parser.rs:78`, `92`
- `Unsupported function: fn:trace` — `eval.rs:329`
- `Unsupported layer transform` — `lower.rs:802`
- `(subset) Default value on array choice branch ... not implemented` — `groups.rs:297`
- Pattern: `negative look-ahead is not supported` / `look-behind is not supported` — `pattern.rs:101-103`

---

*Generated from workspace source review. TDML executed on stable Rust 1.99.0 (2026-10-01).*
