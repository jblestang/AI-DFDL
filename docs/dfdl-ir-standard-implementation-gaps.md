# Compiled Schema IR — DFDL 1.0 (GFD-R-P.240) Implementation Gap Analysis

**Subject:** There is **no** separate `dfdl-ir` crate in `/workspace`. The executable **compiled schema IR** lives in `dfdl-core` (`crates/dfdl-core/src/schema/ir.rs`, `builder.rs`, `mod.rs`). XSD/DFDL annotations are lowered by `dfdl-schema` (`compiler/lower.rs`) through `PropertyStore` → `to_resolved_properties()` (`crates/dfdl-core/src/expr/properties.rs`) into `CompiledSchema` / `ResolvedProperties` / `DfdlProp<T>`.

**Scope:** IR data model fidelity (§5–8), pre-compilation property resolution, compiler-only vs IR vs kernel consumption, Daffodil extension slots, and `SchemaBuilder` vs production compile path.

**Evidence:** Source only (fresh ripgrep over `crates/dfdl-core/src/schema`, `crates/dfdl-core/src/expr/properties.rs`, `crates/dfdl-schema/src/compiler`).

**Related:** [dfdl-standard-implementation-gaps.md](dfdl-standard-implementation-gaps.md) (kernel/runtime), [dfdl-schema-standard-implementation-gaps.md](dfdl-schema-standard-implementation-gaps.md) (XSD lowering).

---

## Executive summary — top 10 IR-specific gaps

| Rank | Gap | IR / impact |
|------|-----|-------------|
| 1 | **`LengthKind::EndOfParent` in IR, never emitted by compiler** | Enum variant `ir.rs:60-61`; `to_resolved_properties` accepts `endOfParent` — `properties.rs:1041`; production `lower.rs` SDE before `add_term_with_props` — `lower.rs:1529-1540`. Hand-built IR can set it; kernel has **no** `EndOfParent` branch (only enum + property parse). |
| 2 | **`LengthKind::Expression` is dead in the kernel** | Parsed into IR — `properties.rs:1038`; **zero** kernel matches on `LengthKind::Expression`. `evaluate_length_property` returns `None` unless `Explicit` (or special `Implicit` text case) — `element.rs:2857-2864`. |
| 3 | **`binaryFloatRep` absent from `ResolvedProperties`** | §13.6 property never lowered; floats use IEEE `from_bits` only. Accessor hardcodes `"ieee"` — `eval.rs:630`. IR cannot express VAX/IBM wire formats. |
| 4 | **`floating` / `textBidi` never reach IR** | SDE in `to_resolved_properties` when `yes` — `properties.rs:1012-1025`. No IR fields; spec §12 alignment/bidi semantics unavailable even in test IR via property layer. |
| 5 | **Compiler reject vs IR support mismatch (zoned, prefixed, calendar)** | `TextNumberRep::Zoned` in IR — `ir.rs:532-537`; kernel implements zoned parse — `text.rs:898-922`; compiler rejects many type combos — `lower.rs:2324-2377`. Nested `prefixed` rejected at compile — `lower.rs:1007` — while `PrefixLengthDescriptor` / `LengthKind::Prefixed` are first-class IR. |
| 6 | **Dual property representation (`*_expr` strings + `DfdlProp` + mirrors)** | `ResolvedProperties` carries parallel `encoding` / `encoding_prop`, `length` / `length_expr` / `length_prop`, etc. — `ir.rs:949-1198`. Kernel hot paths prefer `*_prop` — `element.rs:2867+`; legacy strings still validated in `CompiledSchema::validate_query_style_paths` — `ir.rs:1957-1991`. Risk of drift if `SchemaBuilder` fills only one side. |
| 7 | **`SchemaBuilder` path skips production-only validation** | `build()` → `validate_schema_ir` only — `builder.rs:265-284`. Production adds tunables + `validate_query_style_paths()` after build — `lower.rs:485-490`, `ir.rs:1947-2006`. Test/hand-built schemas can omit query-path SDEs the XML compiler enforces. |
| 8 | **`hidden_group_ref` stored but not consumed at runtime** | Lowering sets `is_hidden` from ref — `properties.rs:1418-1429`; IR keeps both fields — `ir.rs:1055-1058`. Kernel/unparser use **`is_hidden` only** — e.g. `element.rs:1626`, `unparser.rs:280`. |
| 9 | **Extension fields share the normative property bag** | `rep_type`, `rep_simple_type`, `layer`, `choice_branch_key_ranges`, `string_as_xml`, `facets.rep_values` — `ir.rs:1187-1196`, `1264-1265`. Required for TDML/Daffodil; blurs OGF §5–8 vs `dfdlx` in one struct. |
| 10 | **`term_has_representation` IR helper is a subset of §9.2** | Complex types with `Delimited` / `Expression` / `EndOfParent` length kinds not treated like explicit/prefixed/pattern — `ir.rs:1829-1836`. Affects IR-level reasoning, not wire parse directly. |

---

## Gap table (clause ↔ IR ↔ compiler ↔ kernel)

| Clause (GFD-R-P.240) | IR representation | Compiler (`dfdl-schema`) | Kernel consumer (`dfdl-core`) | Gap |
|------------------------|-------------------|---------------------------|--------------------------------|-----|
| §5–8 property model | `ResolvedProperties` (~123 fields), `DfdlProp<T>` — `ir.rs:797-1198` | `lower_*_to_ir` → `to_resolved_properties` — `lower.rs:1806`, `2902`, `3131` | Parser/unparser read `term.properties.*` | **Subset** (missing props never lowered) |
| §12.3 `lengthKind` | `LengthKind` enum incl. `EndOfParent`, `Expression` — `ir.rs:46-61` | Rejects `endOfParent` — `lower.rs:1529-1540`; nested `prefixed` — `lower.rs:1007` | `Prefixed`/`Explicit`/… in `evaluate_length_property` — `element.rs:2652+`; no `EndOfParent`/`Expression` | **Missing** / **Dead** |
| §12.3 `length` expressions | `length_prop: Option<DfdlProp<usize>>`, `length_expr` — `ir.rs:965-967` | Pre-parse via `DfdlProp::parse_with` — `properties.rs:1963-1986` | Eval when `length_kind == Explicit` — `element.rs:2857-2915` | **Subset** (`Expression` kind unused) |
| §12.1 alignment | `alignment`, `alignment_units`, `alignment_kind` — `ir.rs:976-981` | Resolved in `to_resolved_properties` — `properties.rs:1090-1114` | Automatic alignment skips — `element.rs:922+`, `unparser.rs:232+` | **Conformant** (manual kind honored) |
| §12 `floating` | *(no IR field)* | N/A — SDE if `yes` in property layer — `properties.rs:1020-1025`; TDML tunable requires property defined — `lower.rs:1880-1897` | N/A | **Missing** (compiler-only gate) |
| §12 `textBidi` | *(no IR field)* | SDE if `yes` — `properties.rs:1012-1017` | N/A | **Missing** |
| §13.6 `binaryFloatRep` | *(not in `ResolvedProperties`)* | Accepted in XSD tests — `dfdl-schema/.../tests.rs:3236+` | Always IEEE bits — `binary.rs` (see [dfdl-standard-implementation-gaps.md](dfdl-standard-implementation-gaps.md)) | **Divergent** / **Missing in IR** |
| §13.7.2 `textNumberRep` zoned | `text_number_rep`, `text_zoned_sign_style` — `ir.rs:1173-1176` | Type-specific SDEs — `lower.rs:2320-2377` | Zoned parse path — `text.rs:898-922` | **Subset** (compile filter ≠ IR enum) |
| §13.2 justification | `text_*_justification` — `ir.rs:1077-1084`; helper `text_justification_for_value` — `ir.rs:1205-1223` | Lowered — `properties.rs:1273-1283` | Unparser uses helper — `unparser.rs:3198+`; parse path mostly number/string fields directly | **Subset** (boolean/calendar justification largely unparse-only) |
| §14.2–14.3 sequences | `CompiledSequence`, `separator_*`, `sequence_kind`, `SeparatorSuppressionPolicy` — `ir.rs:64-88`, `998-999` | Group lowering — `lower.rs:2851+` | Unordered scan — `parser/mod.rs:1530+` | **Conformant** (core paths exist) |
| §15 choices | `CompiledChoice`, `choice_length_kind`, dispatch ASTs — `ir.rs:1006-1009`, `1121-1127` | Branch keys / `dfdlx:choiceBranchKeyRanges` — `lower.rs:3186+` (see grep) | Dispatch — `parser/mod.rs:1107+`, `1166+` | **Extension** for ranges |
| §16 occurs count | `OccursCountKind`, `occurs_count_prop` — `ir.rs:114-125`, `1014-1019` | Array rules — `lower.rs:1368-1402` | `element.rs:647+`, `759+` | **Subset** (invalid kind → `Implicit` in property layer — `properties.rs:1166`) |
| §7 variables | `CompiledSchema.variable_map`, per-term `set_variables` / `new_variable_instances` — `ir.rs:1059-1062`, `1736` | Variable defs in `lower.rs:346-379` | `parser/mod.rs:255+`, `unparser.rs:1083+` | **Conformant** (wired) |
| §7.4 assertions | `CompiledAssert`, `asserts: Vec<_>` — `ir.rs:492-507`, `1053-1054` | XSD `dfdl:assert` → property store — `annotations.rs:458-489` | Parse/unparse assert checks — `element.rs:2412+`, `2637+` | **Conformant** (IR carries AST/regex) |
| §13.2.1 escapes | `CompiledEscapeScheme` — `ir.rs:255-272` | `resolve_escape_scheme_ref` — `lower.rs:889` | Delimited/unparse escape — kernel delimiter modules | **Conformant** (IR type used) |
| §12.3.4 prefix length | `PrefixLengthDescriptor` — `ir.rs:610-632` | Built at lower time — `lower.rs:1807-1808` | Chained prefix read — `element.rs:2659-2677` | **Subset** (nested compile reject) |
| §9.2 representation | `term_has_representation()` — `ir.rs:1777-1856` | Used internally / tests | Not invoked on every parse step | **Subset** (IR metadata) |
| §23 expressions | `in_scope_namespaces`, precompiled `ExprAst` on props | Namespace + query-path validation post-build — `lower.rs:490`, `ir.rs:1947+` | `eval_expr` with `ExprContext` | **Subset** (expression language gaps in eval, not IR shape) |
| — `dfdlx:repType` / `repValues` | `rep_type`, `rep_simple_type`, `facets.rep_values` | Merge from rep type — `lower.rs:1817-1874` | Logical↔rep mapping — `element.rs:2099+` | **Extension** |
| — `dfdlx:layer` | `layer: Option<String>` — `ir.rs:1191-1192` | Whitelist at compile — `lower.rs:3575+` | `kernel/layer.rs` | **Extension** |
| — `stringAsXml` | `string_as_xml: bool` — `ir.rs:1195-1196` | Runtime property parsing in lower | Post-parse XML check — `element.rs:3032-3054` | **Extension** |

---

## 1. IR data model fidelity to DFDL §5–8

**Graph shape (§5–8, groups, choices):** `CompiledSchema` holds a flat `Vec<CompiledTerm>` indexed by `NodeId` — `ir.rs:1728-1734`. Terms are `Element` (`CompiledElement` + `CompiledType::Simple|Complex`), `Sequence`, `Choice`, or `GroupRef` — `ir.rs:1662-1713`. This matches the spec’s logical term tree, with group refs resolved to shared subgraphs at lower time.

**Property association:** Every term carries one `ResolvedProperties` snapshot (effective properties after format/ref resolution), not a live property stack. Inheritance is **frozen at compile time** via `PropertyStore::resolve_property` in `to_resolved_properties` — `properties.rs:981+`.

**Types and facets:** Simple types use `DfdlSimpleType` on elements; facets live in `SimpleTypeFacets` — `ir.rs:1239-1266`. Validation at runtime uses `validate_value_detailed` — `ir.rs:1275+` (regex crate, not ICU).

**Gaps:**

- **Normative properties with no IR slot:** `binaryFloatRep`, `floating`, `textBidi` (see table).
- **IR enums broader than supported behavior:** `LengthKind::EndOfParent`, `LengthKind::Expression`; full `TextNumberRep` / `BinaryCalendarRep` vs compile-time SDEs.
- **`CompiledElement` occurrence model:** `min_occurs` / `max_occurs` on element — `ir.rs:1676-1681`; array semantics also driven by `occurs_count_kind` on properties — potential dual source (spec-correct but easy to mis-set in hand-built IR).

---

## 2. Pre-compilation: `DfdlProp`, AST, regex on terms

**`DfdlProp<T>`** — `ir.rs:797-914`: At compile time, `{ expr }` values become `Expression { raw, ast }` via `parse_expr`; constants use typed parsers. This is the intended elimination of runtime `{` scanning on hot paths.

**Property lowering pipeline:**

1. XSD → `PropertyStore` (annotations, formats, ref resolution) — `dfdl-schema/src/compiler/annotations.rs`, `schema_doc.rs`.
2. `PropertyStore::to_resolved_properties(parent)` — `properties.rs:981-2200+`: enum mapping, SDEs for unsupported combos, precompilation of `DfdlProp` fields, discriminator/choice ASTs — `properties.rs:2038-2078`.
3. Element-specific SDEs after resolution — `lower.rs:1412+` (lengthKind rules, zoned, calendar, etc.).

**Regex / pattern pre-checks:** `lengthKind='pattern'` requires `is_valid_dfdl_regex` — `properties.rs:1059-1065`. Discriminator patterns compile to `DfdlRegex` — `properties.rs:2052-2068`.

**Spec divergences in the property layer:**

- Invalid `occursCountKind` strings fall through to **`Implicit`** — `properties.rs:1159-1166` (**Divergent** vs strict SDE).
- `textBidi` / `floating` rejected here, so they never appear in IR even though TDML may require the properties to be **declared** (`require_text_bidi` / `require_floating` in `lower.rs:1880-1897`).

---

## 3. Properties present in IR but weak or absent in kernel

| IR field / enum | Notes | Kernel |
|-----------------|-------|--------|
| `LengthKind::EndOfParent` | Enum + property parse | No consumer |
| `LengthKind::Expression` | In enum | `evaluate_length_property` ignores — `element.rs:2857-2864` |
| `hidden_group_ref` | Set at compile | Unused; `is_hidden` used instead |
| `byte_order_expr` | Mirror of dynamic byte order | Superseded by `byte_order_prop`; expr string used only in `validate_query_style_paths` — `ir.rs:1966-1967` |
| `encoding_error_policy_defined` | Compile/tunable metadata | Only `encoding_error_policy_error` affects encode/decode — `text.rs:525`, `unparser.rs:3146` |
| `text_boolean_justification`, `text_calendar_justification` | In `text_justification_for_value` | Unparser path; limited parse-side padding use |
| `choice_branch_key` (OGF) vs `choice_branch_key_ranges` (dfdlx) | Both in IR | Kernel uses both in choice dispatch — `parser/mod.rs:1166-1173` |

**Positive:** Most `ResolvedProperties` fields (123 total) have at least one non-test kernel reference; the list above is the main **Dead / metadata-only** set from an automated field scan over `crates/dfdl-core/src/kernel`.

---

## 4. Properties enforced only in compiler, never in IR

These are rejected or stripped **before** or **during** `to_resolved_properties` / `lower_element_to_ir`, so the IR cannot represent the spec feature even for fuzz/property testing via `SchemaBuilder`:

| Property / rule | Mechanism | Location |
|-----------------|-----------|----------|
| `lengthKind='endOfParent'` | SDE on element lower | `lower.rs:1529-1540` |
| Nested `lengthKind='prefixed'` on prefix type | SDE | `lower.rs:1007` |
| `floating='yes'`, `textBidi='yes'` | SDE in property resolution | `properties.rs:1012-1025` |
| Many `textNumberRep='zoned'` type combos | SDE | `lower.rs:2324-2377` |
| `binaryFloatRep` | Never mapped to IR | *(no field)* |
| Global `inputValueCalc` / `outputValueCalc` on format | SDE | `lower.rs:320-342` |
| `invalidRestrictionPolicy::Ignore` | Strips pattern facets before IR | `lower.rs:1810-1815` |

**IR implication:** Fuzzers using `SchemaBuilder` + manual `ResolvedProperties` can still set **`EndOfParent`**, **`Zoned`**, or **`Expression`** without hitting compiler SDEs — exposing kernel holes not reachable from XSD compile.

---

## 5. Extension fields (`dfdlx` / Daffodil) in IR

Extension data is **not** modeled as a separate sidecar; it occupies the same `ResolvedProperties` and `SimpleTypeFacets` structures as normative properties:

| Extension | IR fields | Lowering | Kernel |
|-----------|-----------|----------|--------|
| `dfdlx:repType` | `rep_type`, `rep_simple_type` — `ir.rs:1187-1190` | Merge rep props — `lower.rs:1817-1874` | Rep value parse/unparse — `element.rs:2099+` |
| `dfdlx:repValues` | `facets.rep_values` — `ir.rs:1264-1265` | `properties.rs:1740+` | `element.rs:2114+`, `unparser.rs:1767+` |
| `dfdlx:choiceBranchKeyRanges` | `choice_branch_key_ranges` — `ir.rs:1127-1128` | Choice lower | `parser/mod.rs:1173+` |
| `dfdlx:layer` / transforms | `layer` — `ir.rs:1191-1192` | Whitelist SDE | `kernel/layer.rs` |
| `dfdlx:runtimeProperties` (`stringAsXml`) | `string_as_xml` — `ir.rs:1195-1196` | Lower | `element.rs:3032-3054` |
| `dfdlx:alignmentKind` | `AlignmentKind` — `ir.rs:176-184` | `properties.rs:1110-1114` | Parser/unparser alignment |

**Gap type:** **Extension** for TDML parity; **Divergent** for a strict OGF-only IR profile (same struct mixes normative and extension slots).

---

## 6. `SchemaBuilder` test IR vs production compiler path

| Stage | `SchemaBuilder` (tests, benchmarks) | `SchemaCompiler::lower_schema_to_ir_with_root` |
|-------|--------------------------------------|-----------------------------------------------|
| Graph construction | `add_term` / `add_term_with_props` — `builder.rs:74-241` | `lower_element_to_ir_bounded` — `lower.rs:495+` |
| Property precompile | Partial: `add_term_with_props` backfills some `DfdlProp` from mirror strings — `builder.rs:98-226` | Full `to_resolved_properties` + element SDEs |
| Validation | `validate_schema_ir` (refs, cycles, pattern/complex rule) — `builder.rs:289-355` | Same + **`validate_query_style_paths`** — `lower.rs:490` |
| Tunables on `CompiledSchema` | Defaults: `disallow_signed_integer_length_1bit: false`, `max_occurs_bounds: None` — `builder.rs:277-278` | Patched from compiler — `lower.rs:486-489` |
| Variables | Manual `define_variable*` | Full default resolution passes — `lower.rs:346-379` |
| Depth limit | No 32-level soft stub | Inserts stub string element at depth ≥ 32 — `lower.rs:503-509` |

**Risk:** Kernel unit tests that build IR manually can pass `validate_schema_ir` while violating constraints the XSD path would reject (query-style paths, endOfParent, inconsistent `length_kind` vs `length_prop`, missing `encoding_prop` precompile).

---

## Deep dive: `DfdlProp` and length/occurs semantics

**Length:** For `lengthKind='explicit'`, dynamic lengths use `length_prop` AST evaluation — `element.rs:2867-2915`. For `lengthKind='expression'` (spec distinct from explicit-with-expression), IR is populated but kernel treats like non-explicit and skips evaluation — **spec §12.3 gap**.

**Occurs:** `occurs_count_prop` parallels `occurs_count_expr` — `ir.rs:1017-1019`. Kernel evaluates prop/expression in array drivers — `element.rs:647+`. Compiler requires `occursCount` when kind is `expression` on arrays — `lower.rs:1383-1390`.

**Discriminator / dispatch:** IR stores both raw strings and precompiled `discriminator_ast` / `discriminator_regex` / `choice_dispatch_key_ast` — `ir.rs:1020-1027`, `1123-1124`. Kernel prefers AST when present — `element.rs:2461`, `parser/mod.rs:1111`.

---

## Deep dive: Groups, choices, hidden terms

- **Choice explicit length:** `choice_length_kind`, `choice_length` — `ir.rs:1006-1009`; parser reads explicit choice bit budget — `parser/mod.rs:1243+`.
- **Initiated content:** `initiated_content: bool` on choice/sequence properties — `properties.rs:1783+`; drives POU — `parser/mod.rs:1232-1238`.
- **Hidden groups:** Compile lowers ref to `is_hidden`; `hidden_group_ref` retained for diagnostics only — kernel gates infoset emission on `is_hidden` — `element.rs:1626`, `unparser.rs:1252+`.

---

## Recommendations for IR maintainers

### Model hygiene

1. **Split or tag extension fields** — e.g. `ResolvedPropertiesExtensions` or `#[cfg(feature = "dfdlx")]` block for `rep_type`, `layer`, `choice_branch_key_ranges`, `string_as_xml` to clarify OGF vs TDML profiles.
2. **Remove or gate dead enum variants** — Either implement `LengthKind::EndOfParent` / `Expression` in kernel, or reject them in `validate_schema_ir` when set on built schemas (close `SchemaBuilder` loophole).
3. **Add missing normative fields** — `binary_float_rep: BinaryFloatRep` on `ResolvedProperties`, lowered from `dfdl:binaryFloatRep`, consumed in `kernel/parser/binary.rs`.
4. **Drop redundant mirrors** — Plan deprecation of `*_expr` parallel strings once all paths use `DfdlProp` (keep one round-trip for debug if needed).

### Compiler / IR alignment

5. **Unify validation entry points** — Call `CompiledSchema::validate_query_style_paths()` from `SchemaBuilder::build()` (optional strict flag) so test IR matches production.
6. **Strict enum parsing** — Invalid `occursCountKind` → SDE instead of default `Implicit` — `properties.rs:1166`.
7. **Document compiler-only properties** — Maintain a machine-readable list of properties that never appear in IR (`floating`, `textBidi`, …).

### Pre-compilation

8. **Centralize regex validation** — `is_valid_dfdl_regex` vs `DfdlRegex::new` vs facet `regex::Regex` use three engines; document in IR module docs (`ir.rs` header).
9. **`DfdlProp::parse_with` fold** — Comment claims const-fold to `Constant(T)` at compile time; verify/finish for length/occurs where spec allows static evaluation.

### Testing

10. **IR conformance tests** — For each `LengthKind` / `OccursCountKind` variant, assert: (a) XSD compile result, (b) kernel branch exists, or (c) explicit SDE — avoid silent `Ok(None)` for length expression kind.

---

## Appendix: Key IR source locations

| Artifact | Path |
|----------|------|
| IR types & `ResolvedProperties` | `crates/dfdl-core/src/schema/ir.rs` |
| `SchemaBuilder`, `validate_schema_ir` | `crates/dfdl-core/src/schema/builder.rs` |
| Property → IR lowering | `crates/dfdl-core/src/expr/properties.rs` (`to_resolved_properties`) |
| XSD → IR graph | `crates/dfdl-schema/src/compiler/lower.rs` |
| Post-build path validation | `CompiledSchema::validate_query_style_paths` — `ir.rs:1947+` |

---

*Generated from workspace source review focused on compiled schema IR. No separate `dfdl-ir` crate exists; this document names that explicitly.*
