# DFDL 1.0 Appendix F (TDML) — Harness Gap Audit (Item 4)

**Scope:** `crates/dfdl-tests/src/tdml.rs` (parser + runner), `crates/dfdl-tests/src/lib.rs` (Apache Daffodil suite driver, reporter). **Excluded:** `dfdl-vm`.  
**Reference baselines:** OGF GFD-R-P.240 Appendix F (TDML format); Apache Daffodil TDML runner semantics (Daffodil test corpus under `crates/dfdl-tests/tests/daffodil/`).  
**Evidence:** Source inspection only (this audit does not re-run the 4k+ ignored benchmark).

---

## Executive summary

The in-tree TDML harness is a **pragmatic subset** of Appendix F plus Daffodil extensions. It drives `dfdl-schema` / `dfdl-core` directly, maps a **fixed set of Daffodil tunables** into compiler/parser flags, and classifies outcomes via **`TdmlReport`** string patterns. Several paths **inflate pass counts** relative to a strict TDML runner (soft infoset checks, hard-coded upstream waivers, and a **parse-error matching bug**). The full Apache corpus runs only under an **`#[ignore]`** test; CI smoke runs one file plus `conformance_reporter_test.tdml`, and the benchmark **does not assert `failed == 0`**.

---

## Harness vs OGF Appendix F / TDML XML

| TDML construct | Spec / Daffodil intent | This harness | Gap |
|----------------|------------------------|--------------|-----|
| `testSuite` | Suite metadata, default config | Parses `suiteName`, `defaultConfig`, `defaultImplementations`; captures namespace bindings for schema wrap | No `description` / version fields |
| `defineSchema` | Embed or reference XSD | Inline wrap to synthetic `xs:schema` (`targetNamespace=http://example.com`, injects `dfdlx`/`daf` URIs) | **Divergent** namespace story vs files that ship full schemas |
| `defineConfig` + `tunables` | Processor tuning | Parses inline configs; external `*.xml` configs via second XML pass | Only **9 tunable names** wired (see below) |
| `externalVariableBindings` | Runtime variables | Parsed; applied on parse/unparse | Bindings not merged when both inline + file config |
| `parserTestCase` / `unparserTestCase` | Single test | `name`, `root`, `model`, `validation`, `config` | **`roundTrip` not read** (~1k+ corpus uses it); no `dependsOn` / `description` |
| `implementations` | Vendor filter | Drops cases unless token **`daffodil`** (case-insensitive) in suite or case attr | IBM-only / other vendor cases **silently omitted** at parse time |
| `document` / `documentPart` | Physical data | `text`, `byte`, `hex`, `bits`, `file`; DFDL entities; sub-byte text encodings | **`document@bitOrder` stored but BitReader stays MSBF/BE**; mixed part orders only error if no document-level order |
| `infoset` / `dfdlInfoset` | Expected logical XML | Inline XML rebuilt with namespaces; path heuristic loads `.xml` **only if infoset text does not start with `<`** | **`dfdlInfoset@type="file"` not honored** (filename becomes inline XML text) |
| `errors` | Expected processing errors | Substring match on `Debug` + `message` | No ordering / multi-error completeness |
| `validationErrors` | Post-parse validation | Substring match when `Err(Validation)` | **`validation` defaults to `Off`** if attribute absent |
| `warnings` | Expected warnings | **Not parsed or asserted** | **Missing** (e.g. `SepTests.tdml`, `parseUnparseModeTest.tdml`, `expressions.tdml`) |

### Tunables mapped today (`tdml.rs` → compiler/parser)

| Tunable | Effect |
|---------|--------|
| `allowSignedIntegerLength1Bit` | `SchemaCompiler::with_allow_signed_integer_length_1bit` (default allow unless `false`) |
| `maxOccursBounds` | Caps parsed `maxOccurs` overflow |
| `requireEncodingErrorPolicyProperty` | Strict compile |
| `requireTextBidiProperty` | Strict compile |
| `requireFloatingProperty` | Strict compile |
| `escalateWarningsToErrors` | Compiler + engine |
| `allowExpressionResultCoercion` | Engine (forced `false` for three polymorphic expr tests by name) |
| `unqualifiedPathStepPolicy` | `noNamespace` / `defaultNamespace` / `preferDefaultNamespace` |
| `maxHexBinaryLengthInBytes` | Limit |
| `invalidRestrictionPolicy` | `error` / `ignore` / default validate |

All other Daffodil tunables in corpus configs are **ignored** (no effect on pass/fail).

---

## Harness vs Daffodil runner behavior expectations

| Topic | Daffodil expectation | This harness |
|-------|---------------------|--------------|
| Round trip | `roundTrip` onePass / twoPass / false controls parse↔unparse checks | Unparser always byte-compares output; **optional re-parse infoset match** on mismatch only (not gated by `roundTrip`) |
| Infoset compare | Canonical DFDL infoset equivalence | Strict tree compare first; then **soft substring fallbacks** (see below) |
| Error tests | Each `<tdml:error>` should appear in diagnostic stream | Any **one** substring match passes; schema-time errors with `<errors>` pass compile failure without message check |
| Warnings | `<tdml:warnings>` when warnings expected | **Ignored** — success even if warnings differ |
| Missing fixtures | Fail or skip visibly | **`indexLimit`**: missing `csv_1.6m` → **counted pass** |
| Upstream bad TDML | Runner fails or documents waiver | **`predicate_02`**, **`alignmentPacked7BitASCII_02`**: hard-coded **pass** on specific engine errors |
| Delimiter encoding check | Tunable / mode dependent | **`checkDelimiterEncoding` disabled for all unparser cases** (parser only) |
| GeneralPurposeFormat | Classpath IBM format | Bundled constant `IBM_GENERAL_PURPOSE_FORMAT_XSD` |

---

## Soft assertions and pass inflation

1. **`verify_infoset` fallback** (`tdml.rs`): if strict `compare_infoset_elements` fails, pass if expected XML **contains root local name**, root text value, or **any child local name** — can mask real mismatches.
2. **Parse error branch bug** (`tdml.rs` ~1341–1346):

   ```rust
   if matches_expected || !tc.expected_errors.is_empty() {
       report.passed += 1;
   }
   ```

   When `<errors>` is non-empty, **every failed parse passes**, even when no expected substring matches — **`tc_error_msg_mismatch` in reporter suite never clusters correctly**.

3. **Schema compile + `<errors>`**: compile error counts as pass without verifying error text.
4. **Unparser without infoset**: **automatic pass** (`~1512`).
5. **Unparser infoset XML parse error + expected errors**: pass without checking load error text (`~1506–1507`).
6. **Unparser expected error on success path**: uses synthetic `TDML Error: ...` strings and **`all()`** substring match on expected list (differs from parse-side `any()`).

---

## Pass / fail classification (`TdmlReport`)

| Outcome | Condition |
|---------|-----------|
| **Pass** | Infoset match (strict or soft); empty expected errors and success; expected error substring match (parse load path); validation error match; waivers above; unparser byte match or round-trip infoset salvage |
| **Fail** | Messages tagged: `failed schema compilation`, `unexpectedly failed parse`, `infoset mismatch`, `expected parse/unparse error but succeeded`, `unparse failed`, `unparse output bytes mismatch`, `error message mismatch`, `validation error mismatch`, document load failures |

`TdmlReport::total` = parsed test cases (after `implementations` filter). **`passed + failed` can exceed meaningful “strict TDML” passes** because of soft checks and waivers.

---

## `lib.rs` — Apache suite runner and reporter

| Mechanism | Behavior |
|-----------|----------|
| **`test_apache_daffodil_official_tdml_suite`** | `#[ignore]` — full tree under `tests/daffodil` (~4,337 cases cited in attribute) |
| **CI smoke** | `test_apache_daffodil_official_tdml_suite_smoke` → `section06/namespaces/multiFile.tdml` only |
| **Reporter test** | `test_apache_daffodil_official_tdml_suite_reporter_coverage` → `conformance_reporter_test.tdml` (13 intentional failure modes) |
| **Benchmark assertions** | Requires `suites_executed > 0`, `total_cases > 0`, **`passed > 0` only** — **not** `failed == 0` |
| **Env filters** | `TDML_FILTER` (case name), `TDML_FILE_FILTER`, `TDML_DEBUG`, `TDML_DEBUG_COMPARE`, `TDML_DEBUG_NO_TOP` |
| **Failure reporter** | Substring buckets on failure messages (schema comp, parse, infoset, expected-not-raised, unparse, error-msg mismatch, other); prints cluster histograms to **stderr** |
| **Strict suites in CI** | `official_conformance.tdml`, `delimiters_conformance.tdml` assert **`report.failed == 0`** |
| **Targeted regressions** | Many `lib.rs` tests filter one TDML file to named cases and assert zero failures |

`conformance_reporter_test.tdml` is the **contract** for reporter strings; bucket **“Error Message Substring Diff”** is undermined by the parse-error logic bug above.

---

## Gap table (harness / TDML)

| ID | Area | Gap type | Severity |
|----|------|----------|----------|
| H1 | `roundTrip` attribute | **Missing** — corpus attribute ignored | High for unparser/twoPass semantics |
| H2 | `warnings` element | **Missing** | Medium |
| H3 | `dfdlInfoset type="file"` | **Missing** — file infoset refs mis-handled | High for file-heavy suites |
| H4 | `document@bitOrder` / byte order | **Partial** — not applied to `BitReader`/`BitWriter` | Medium |
| H5 | Infoset verification | **Soft / divergent** substring fallbacks | High (masks failures) |
| H6 | Expected parse errors | **Bug** — non-empty `<errors>` always passes | Critical |
| H7 | Daffodil tunables | **Subset** — 9 names only | Medium |
| H8 | `implementations` filter | **Divergent** — non-`daffodil` cases dropped at parse | Low (intentional) |
| H9 | Upstream waivers | **Divergent** — 3 named cases + missing fixture pass | Medium |
| H10 | Benchmark gate | **Process** — ignored full suite; no zero-fail assert | High for CI truthfulness |

---

## Recommended fixes (harness-only, ordered)

1. Fix expected parse-error branch: require `matches_expected` only; emit `error message mismatch` when substrings fail.
2. Remove or gate soft `verify_infoset` fallbacks behind `TDML_STRICT=1` (default strict for new runs).
3. Parse `roundTrip` and implement Daffodil onePass/twoPass/false behavior on unparser and parser cases.
4. Load infoset when `dfdlInfoset@type="file"` or body is a bare filename.
5. Parse and assert `<warnings>` (collect engine warnings; compare substrings).
6. Apply `document@bitOrder` / per-part orders to bit stream reader/writer.
7. Document or implement remaining Daffodil tunables as no-ops with warnings when present in config.
8. Replace hard-coded waivers with `tdml:skip` or explicit `#[ignore]` case lists in test metadata.
9. Split Apache benchmark: assert `failed == 0` in release CI or publish pass/fail counts separately from waivers.
10. Extend `TdmlTestCase` with `expected_warnings`, `round_trip`, `implementations` for reporting skipped cases in `TdmlReport`.

---

## File map

| File | Role |
|------|------|
| `crates/dfdl-tests/src/tdml.rs` | TDML XML parse, document assembly, suite execution, infoset compare |
| `crates/dfdl-tests/src/lib.rs` | Integration tests, `run_apache_daffodil_official_tdml_suite_impl`, reporter |
| `crates/dfdl-tests/tests/official_conformance.tdml` | Small strict harness self-test |
| `crates/dfdl-tests/tests/daffodil/conformance_reporter_test.tdml` | Reporter category fixtures |
