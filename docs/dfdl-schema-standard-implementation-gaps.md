# DFDL 1.0 (OGF GFD-R-P.240) vs `dfdl-schema` Compilation Gap Analysis

**Scope:** `/workspace/crates/dfdl-schema` only — XSD parse (`schema_doc.rs`, `xsd_ast.rs`), annotation ingest (`annotation.rs`, `annotations.rs`, `elements.rs`, `groups.rs`), and lowering to `CompiledSchema` (`compiler/lower.rs`). Property resolution runs through `dfdl-core` **`PropertyStore::to_resolved_properties()`** (`crates/dfdl-core/src/expr/properties.rs`); this document treats that call as part of the compile pipeline but does not review kernel parse/unparse.

**Evidence:** Fresh ripgrep over `crates/dfdl-schema/src` (~249 SDE-related hits) plus targeted reads of `to_resolved_properties` at compile call sites (`lower.rs:1806`, `2902`, `3131`; `schema_doc.rs:868`; `annotations.rs:235`).

**Out of scope:** Runtime behavior in `dfdl-core` kernel (see [dfdl-standard-implementation-gaps.md](dfdl-standard-implementation-gaps.md), [dfdl-ir-standard-implementation-gaps.md](dfdl-ir-standard-implementation-gaps.md)).

---

## Executive summary — top 10 compiler gaps

| Rank | Gap | Compile impact |
|------|-----|----------------|
| 1 | **`lengthKind='endOfParent'` SDE after property layer accepts it** | `to_resolved_properties` maps `endOfParent` — `properties.rs:1041`; element lower rejects — `lower.rs:1529-1540`. |
| 2 | **`dfdl:binaryFloatRep` never lowered** | Parsed on elements in tests — `tests.rs:3236+`; no field in `ResolvedProperties`; wire semantics not represented in IR. |
| 3 | **`textBidi='yes'` / `floating='yes'` hard SDE in property resolution** | `properties.rs:1012-1025`; tunables only force *declaration* of `no` — `lower.rs:1880-1897`. |
| 4 | **Nested `lengthKind='prefixed'` on prefix type** | SDE — `lower.rs:1007`; IR path for top-level prefixed still built — `lower.rs:892-949`, `1807-1808`. |
| 5 | **`textNumberRep='zoned'` compile filter** | Rejects float/double and many types — `lower.rs:2324-2377`; allows some integer/decimal paths with pattern rules. |
| 6 | **Calendar/binary compile subset** | `binaryMilliseconds` on date/time — `lower.rs:2752`; BCD pattern punctuation — `lower.rs:2665-2823`; >9 fractional pattern digits — `lower.rs:2652`. |
| 7 | **Expression / path validation split** | `parse_expr` at property resolution; query-style paths post-build — `lower.rs:490` → `ir.rs:1947+`; polymorphic group check incomplete — `lower.rs:74-308`. |
| 8 | **Depth ≥ 32 silent IR stub** | Replaces subtree with default string element — `lower.rs:503-516` (sequences/choices similar at `2857+`). |
| 9 | **Daffodil `dfdlx` / `daf` wired at compile** | Layer whitelist — `lower.rs:786-806`, `is_known_layer` — `3575-3597`; repType, choiceBranchKeyRanges, objectKind, tunables — `compiler.rs:46-66`, `lower.rs:1817+`. |
| 10 | **Invalid enum fallthrough in property layer** | Unknown `occursCountKind` → `Implicit` — `properties.rs:1159-1166` (element lower also SDEs invalid kinds — `lower.rs:1393-1399`). |

---

## Compile pipeline (where spec gaps appear)

```mermaid
flowchart LR
  XML[XSD XML] --> SD[schema_doc.rs parse]
  SD --> AST[xsd_ast + PropertyStore]
  AST --> ANN[annotations / elements / groups]
  ANN --> LOW[lower.rs per-term SDEs]
  LOW --> TRP[to_resolved_properties]
  TRP --> IR[SchemaBuilder → CompiledSchema]
  IR --> QSP[validate_query_style_paths]
```

| Stage | GFD clauses | Primary files |
|-------|-------------|---------------|
| XSD + DFDL ingest | §4, §5.2 (partial), XSD 1.0 | `schema_doc.rs`, `xsd_ast.rs`, `annotation.rs` |
| Annotation rules | §6, §7, §14-16 | `annotations.rs`, `groups.rs`, `elements.rs` |
| Effective properties | §6.3 scoping | `lower.rs` (`merge_parent`, `extend`, format ref) + `PropertyStore` in core |
| Property → IR | §6.3, §12-13, §23 | `to_resolved_properties` + element SDEs in `lower.rs` |
| Post-build paths | §23.2 | `lower.rs:490`, `CompiledSchema::validate_query_style_paths` — `ir.rs:1947+` |

---

## Gap table (clause ↔ compile behavior)

| Clause (GFD-R-P.240) | Requirement | Compiler behavior | Gap type |
|----------------------|-------------|-------------------|----------|
| §12.3 | `lengthKind` includes `endOfParent` | Property layer sets enum — `properties.rs:1041`; SDE on lower — `lower.rs:1536` | **Missing** |
| §12.3.2 | Nested `prefixed` on prefix length type | SDE — `lower.rs:1007` | **Missing** |
| §12.3 | `lengthKind='expression'` | Accepted in `to_resolved_properties` — `properties.rs:1038`; no extra lower SDE | **Subset** (IR emitted; kernel gap out of scope) |
| §12 | Complex + `prefixed` + `lengthUnits='characters'` + variable encoding | SDE — `lower.rs:1973-1977` | **Missing** |
| §12 | Binary + `lengthKind='delimited'` (non-packed) | SDE — `lower.rs:1460-1465` | **Conformant** |
| §12 | Text calendar + `lengthKind='implicit'` | SDE — `lower.rs:1478-1482` | **Conformant** |
| §12 | `floating='yes'`, `textBidi='yes'` | SDE in `to_resolved_properties` — `properties.rs:1012-1025` | **Missing** |
| §13.6 | `binaryFloatRep` | Stored in `PropertyStore` from XSD; **not** mapped in `to_resolved_properties` | **Missing** / **Divergent** |
| §13.7.2 | `textNumberRep='zoned'` | Type-specific SDEs — `lower.rs:2324-2377` | **Subset** |
| §13.11 | `binaryCalendarRep='binaryMilliseconds'` on date/time | SDE — `lower.rs:2752` | **Missing** |
| §13.11 | Calendar pattern limits (fractional, BCD) | SDEs — `lower.rs:2652`, `2665-2686`, `2814-2823` | **Subset** |
| §13.7 | Packed/BCD implicit length | SDE — `lower.rs:2204`, `2770-2787` | **Subset** |
| §7.2 / §7.7 | Variables, defaults, circular defs | Multi-pass defaults — `lower.rs:355-417`; namespace check — `schema_doc.rs:672`; circular `{expr}` SDE — `lower.rs:402-406` | **Mostly conformant** |
| §7.4 | Discriminator vs assert | `validate_term_assertions` — `annotations.rs:16-28`; duplicate in property layer — `properties.rs:1478-1480` | **Conformant** |
| §7.4 | Assert/discriminator compile | `parse_expr` / `DfdlRegex` — `properties.rs:1436-1496`, `2038-2068` | **Subset** (expr language limits in `dfdl-core` parser) |
| §15.1.4 | Array choice branch default | SDE `(subset) ... not implemented` — `groups.rs:297` | **Missing** |
| §15 / UPA | Duplicate choice branch names | SDE — `groups.rs:306` | **Conformant** |
| §14 | Hidden groups / optional in hidden choice | SDE rules — `groups.rs:530+`, `592+` | **Conformant** / **Stricter-than-spec** (hidden choice defaultability) |
| §5.2 / XSD | Reachable component constraints | Duplicate defs — `schema_doc.rs`, `xsd_ast.rs`; enum ⊆ base — `lower.rs:27-71`, `711`; **no** full XSD 1.0 component constraint validator | **Subset** |
| §5.1 / §5.2 | Built-in derived type facets | Injected at lower — `lower.rs:1901-1948` | **Conformant** |
| §6.3 | Property scoping / format ref | `merge_parent`, `extend`, `resolve_ref_formats` — `lower.rs:754-889` | **Conformant** (core resolution) |
| §23 / §23.2 | Path expressions | Query-style SDE post-build — `ir.rs:2119`; occursCount path heuristics — `lower.rs:1597-1624` | **Subset** |
| §23 | Polymorphic model group expressions | Partial check; error text `Feature not yet implemented` — `lower.rs:291-301` | **Missing** |
| §23 | Expression result coercion | Tunable `daf:allowExpressionResultCoercion` — `compiler.rs:65-66`, `lower.rs:243-244` | **Extension** / **Subset** |
| §4 / format | `inputValueCalc` on `dfdl:format` | SDE — `lower.rs:320-342` | **Conformant** |
| §12.3.4 | `prefixLengthType` with asserts | SDE — `lower.rs:942-947` | **Conformant** |
| — | `dfdlx:repType` / repValues / ranges | Validation — `elements.rs:637-831`, `schema_doc.rs:398+`; merge at lower — `lower.rs:1817-1874` | **Extension** |
| — | `dfdlx:layerTransform` / `daf:` alias | Whitelist SDE — `lower.rs:786-806`, `3575-3597` | **Extension** |
| — | `dfdlx:objectKind='bytes'` | Requires explicit/prefixed — `lower.rs:1557-1565` | **Extension** |
| — | `dfdlx:choiceBranchKeyRanges` | Conflict SDEs — `lower.rs:3186-3249` | **Extension** |
| — | `daf:invalidRestrictionPolicy=Ignore` | Strips non-string pattern facets — `lower.rs:1810-1815` | **Extension** / **Divergent** |
| — | Depth limit 32 | Silent stub element — `lower.rs:503-516` | **Divergent** |
| — | TDML/test hooks | `nest4` lengthKind strip — `lower.rs:878-880`; `ivc_26` global IVC — `lower.rs:465-471` | **Divergent** |

---

## 1. Property resolution & scoping (§6.3)

**Flow:** Per element, the compiler builds `effective_elem_props` (type chain, defaults §5.1, global format, parent merge) — `lower.rs:586-889`, then calls **`to_resolved_properties(Some(parent_props))`** — `lower.rs:1806`. Sequences and choices call the same API — `lower.rs:2902`, `3131`.

**Conformant patterns:**

- Format reference resolution — `resolve_ref_formats` (used throughout `lower.rs`).
- Sequence-only properties stripped from elements — `lower.rs:881-885`.
- Escape scheme ref resolution and delimiter conflict — `lower.rs:889-890`, `annotations.rs` escape helpers.
- `leadingSkip` / `trailingSkip` required when non-default format combinations demand it — `lower.rs:1764-1781`.
- Explicit `lengthKind` without `length` on element’s own annotation — `lower.rs:1795-1804` (DFDL-12-039R comment).

**Gaps:**

| Issue | Detail | Gap type |
|-------|--------|----------|
| **`endOfParent` ordering** | Property resolution succeeds; lower rejects later | **Missing** |
| **`binaryFloatRep` dropped** | Never read in `to_resolved_properties` (grep: no matches in `properties.rs`) | **Missing** |
| **`textBidi` / `floating`** | SDE if effective value is `yes`; tunables `requireTextBidi` / `requireFloating` only error when property absent — `lower.rs:1880-1897` | **Missing** + **Extension** (tunables) |
| **Invalid `occursCountKind`** | Silent `Implicit` in property layer — `properties.rs:1166`; arrays get explicit invalid SDE — `lower.rs:1393-1399` | **Divergent** |
| **Invalid `nilKind` / delimiter policy** | Silent defaults — `properties.rs:1178-1209` | **Subset** |

---

## 2. XSD constraints & annotation ingest (§4, §5.2)

**`annotation.rs`:** Accepts `dfdl:`, `dfdlx:`, and `daf:` prefixes into one flat property namespace — `annotation.rs:34-41`. Unknown attributes on XSD components → SDE — `annotation.rs:42-48`.

**`schema_doc.rs`:** Include/import resolution; failed include is fatal SDE — `schema_doc.rs:637-647`. Duplicate top-level names — `schema_doc.rs:96-100`. Standalone `escapeScheme` fragment validated via `to_resolved_properties` — `schema_doc.rs:867-868`.

**`elements.rs`:** Facet applicability SDEs; `dfdlx:repType` requires restriction + enumeration — `elements.rs:637-658`; rep value overlap SDEs — `741-831`.

**§5.2 reachable components:** Implemented partially as duplicate-definition checks, UPA on choices (`groups.rs:306`), and enumeration subset validation (`lower.rs:27-71`). There is **no** general XSD 1.0 schema component constraint validator for all reachable particles — **Subset**.

---

## 3. Expression compile & paths (§18, §23)

**At property resolution (`to_resolved_properties`):**

- Discriminator, choice dispatch key: `parse_expr` — `properties.rs:2038-2083`.
- Asserts: AST + regex — `properties.rs:1436-1496`.
- Namespace prefix requirements on expressions — `properties.rs:1920-1951` (also variable defaults — `schema_doc.rs:672`).

**At lower (element-specific):**

- Literal `inputValueCalc` vs simple type coercion SDEs — `lower.rs:1627-1678`.
- `inputValueCalc` / `outputValueCalc` placement rules — `lower.rs:1688-1714`.
- Global element `choiceBranchKey` / `choiceBranchKeyRanges` forbidden — `lower.rs:450-462`, `615-620`.

**Post-build (`lower.rs:490`):**

- `validate_query_style_paths` — array steps without predicates → SDE — `ir.rs:2119`.

**Gaps:**

- Polymorphic reusable group expression validation is stubbed (`Feature not yet implemented`) — `lower.rs:291-301`.
- `validate_expr_query_paths` ignores parse failures (`if let Ok(ast)` — `ir.rs:2016-2018`) — **Subset**.

---

## 4. Discriminators, asserts, initiators (§7.4, §9)

| Rule | Location |
|------|----------|
| Discriminator + assert mutual exclusion | `annotations.rs:20-28`, `properties.rs:1478-1480` |
| Empty pattern discriminator/assert | `annotations.rs:30-48` |
| `newVariableInstance` default + `setVariable` race | `annotations.rs:50-62` |
| Terminator `%ES;` with `lengthKind='delimited'` | `lower.rs:1543-1550` |
| Separator/terminator vs escape char | `annotations.rs:1013+` (grep in annotations) |

Assert/discriminator regex uses `DfdlRegex` / `is_valid_dfdl_regex` — same subset as length patterns (ICU/Java features not validated here beyond core `pattern` module).

---

## 5. Calendar, binary, text rules (§12, §13)

**Binary numbers:** Bit length ranges, 1-bit signed tunable — `lower.rs:2280-2313`, `compiler.rs:144-150`. `byteOrder` + `bitOrder` conflict — `lower.rs:2556`. `binaryDecimalVirtualPoint` tunable limits — `2569-2575`.

**Text numbers:** `textStandardBase` disallowed on float/double/decimal — `lower.rs:2705-2730`. Required reps for boolean/text — `lower.rs:2466-2520`.

**Zoned:** See table; integer family allowed with pattern constraints — `lower.rs:2332-2370`.

**Calendar:** `calendarPatternKind` vs `binaryCalendarRep` — `lower.rs:2604+`; implicit length rules for packed calendar — `2770-2787`.

---

## 6. `dfdlx` / `daf` extensions (compile)

| Mechanism | File:line |
|-----------|-----------|
| Tunables on `SchemaCompiler` | `compiler.rs:46-66`, `112-165` |
| `parseUnparsePolicy` alias | `lower.rs:770-783` |
| Layer transform whitelist | `lower.rs:786-806`, `3575-3597` |
| Rep type merge into `ResolvedProperties` | `lower.rs:1817-1874` |
| Choice branch integer ranges | `lower.rs:3186-3249` |
| Warning escalation (`daf:escalateWarningsToErrors`) | `lower.rs:562-583` |
| `invalidRestrictionPolicy::Ignore` | `lower.rs:1810-1815` |

Pure **OGF** schemas never use these keys; TDML compilation typically requires the **dfdlx/daf profile** (see appendix).

---

## 7. Errors vs silent defaults

| Behavior | Example | Location |
|----------|---------|----------|
| **Explicit SDE** | `lengthKind='endOfParent' is not implemented` | `lower.rs:1536` |
| **Explicit SDE** | `textBidi='yes' is not supported` | `properties.rs:1016` |
| **Silent default** | Unknown `occursCountKind` → `Implicit` | `properties.rs:1166` |
| **Silent default** | Unknown `nilKind` → `literalValue` | `properties.rs:1183` |
| **Silent stub** | Depth ≥ 32 → string element, default props | `lower.rs:503-516` |
| **Property dropped** | `binaryFloatRep` not in IR | `to_resolved_properties` |
| **Facet stripped** | `invalidRestrictionPolicy::Ignore` | `lower.rs:1810-1815` |

---

## Recommendations (compiler maintainers)

1. **Align `endOfParent`:** SDE in `to_resolved_properties` or implement before IR (avoid property layer / lower mismatch).
2. **Lower `binaryFloatRep`** into `ResolvedProperties` or SDE when value ≠ `ieee` if kernel stays IEEE-only.
3. **Strict enums:** Invalid `occursCountKind` / `nilKind` → SDE in `to_resolved_properties` (match `lower.rs` array rules).
4. **Replace depth-32 stub** with SDE or document as non-conformant compile cap.
5. **Finish polymorphic group validation** or narrow error message to spec-accurate SDE (remove test-only `../foo/bar` branch — `lower.rs:244-245`).
6. **Machine-generate SDE catalog** from `rg` over `crates/dfdl-schema` + property-layer strings invoked at compile time.
7. **Profile docs:** See [CONFORMANCE-PROFILES.md](CONFORMANCE-PROFILES.md).

---

## Appendix A: Representative compile-time SDE strings (`dfdl-schema` + compile-time `properties.rs`)

| Message (substring) | Location |
|---------------------|----------|
| `lengthKind='endOfParent' is not implemented` | `lower.rs:1536` |
| `Nested dfdl:lengthKind="prefixed" is not supported` | `lower.rs:1007` |
| `textNumberRep='zoned' is not allowed` | `lower.rs:2329`, `2373` |
| `binaryCalendarRep='binaryMilliseconds' is not allowed` | `lower.rs:2752` |
| `textBidi='yes' is not supported` | `properties.rs:1016` |
| `floating='yes' is not supported` | `properties.rs:1024` |
| `Unsupported layer transform` | `lower.rs:802` |
| `(subset) Default value on array choice branch` | `groups.rs:297` |
| `Query-style paths not supported` | `ir.rs:2119` (invoked from `lower.rs:490`) |
| `Feature not yet implemented: Expression` | `lower.rs:292` |
| `inputValueCalc/outputValueCalc is not allowed in dfdl:format` | `lower.rs:331` |

---

## Appendix B: Conformance profiles

Normative profile definitions live in [CONFORMANCE-PROFILES.md](CONFORMANCE-PROFILES.md) in this directory.

---

*Generated from workspace source review focused on `dfdl-schema` compilation. Property-resolution spans in `dfdl-core/src/expr/properties.rs` are cited only where invoked from this crate’s compile path.*
