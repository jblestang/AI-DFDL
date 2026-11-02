# DFDL Engine — Independent Expert Reassessment

*Assessment date: 2026-10-14. Opinion formed from source inspection and commands run in this environment only. Prior `review_*.md` files and `CONFORMANCE.md` §3 were not used as evidence.*

---

## Executive summary

This workspace implements a **serious, spec-minded DFDL 1.0 engine** in Rust: a layered `no_std` core (`dfdl-xml` → `dfdl-schema` → `dfdl-core`), a substantial tree-walking parse/unparse kernel with explicit Points of Uncertainty (POU), a full TDML runner wired to **198 bundled Apache Daffodil TDML files**, and an emerging bytecode VM (`dfdl-vm`) for throughput-oriented decoding. Measured against Daffodil’s official TDML corpus, the kernel path is **~99.8% aligned** (4,339 / 4,349 cases passing in ~12s on this VM).

What holds it back from “production product” is not kernel depth but **host integration and release engineering**: the default toolchain in this environment **does not compile** the tree; `dfdl-cli` is a one-line stub; the full TDML run is **`#[ignore]` and does not fail CI on the ten remaining mismatches**; and `dfdl-std` is mostly a re-export shim. For embedded / library use by DFDL-literate integrators, the core is **credible and unusually disciplined** (panic-free lints, resource limits, structured errors). For drop-in Daffodil replacement or enterprise rollout, gaps in tooling, strict conformance gating, and operational defaults remain material.

**Bottom line:** Strong **engine R&D / library** quality with Daffodil-interoperable behavior on the official vectors; **not yet a shippable CLI/runtime product** without toolchain pinning, a strict TDML gate, and real host adapters.

---

## What I looked at

### Commands run (actual outcomes)

| Step | Command | Outcome |
|------|---------|---------|
| Rust version (default) | `rustc --version` | **1.83.0** (2024-11-26) |
| Check (default) | `cargo check --workspace` | **FAILED** — 8× `E0658` on `unsigned_is_multiple_of` / `.is_multiple_of()` in `dfdl-core` |
| Rust (stable) | `rustup run stable rustc --version` | **1.99.0** (2026-09-28) |
| Check (stable) | `rustup run stable cargo check --workspace` | **OK** — all workspace members |
| Tests (stable) | `rustup run stable cargo test --workspace` | **OK** — unit/integration tests green (see counts below) |
| TDML (ignored) | `rustup run stable cargo test -p dfdl-tests test_apache_daffodil_official_tdml_suite -- --ignored --nocapture` | **Test fn passed**; harness reported **4,349 cases, 4,339 passed, 10 failed** |
| `no_std` spot check | `cargo check -p dfdl-core -p dfdl-xml -p dfdl-schema --target thumbv7m-none-eabi` (stable, after `rust-std` for target) | **OK** for all three core crates |

**Per-crate test counts** (`cargo test --workspace`, stable): `dfdl-core` 295; `dfdl-schema` 114; `dfdl-tests` 150 (+1 ignored); `dfdl-vm` 20; `dfdl-xml` 7; `dfdl-benchmark` 18; others single-digit — **all executed tests passed**.

### Orientation (not verdict)

- Skimmed `ARCHITECTURE.md` §1–3 for crate boundaries and `no_std`/panic-free rules only.

### Source samples (direct reads / targeted search)

| Area | Location | Why |
|------|----------|-----|
| Error taxonomy | `crates/dfdl-core/src/error.rs` | §3.2 / Appendix F alignment, fatal vs backtrackable |
| IR model | `crates/dfdl-core/src/schema/ir.rs` (opening + choice/discriminator fields) | Executable schema graph, length/delimiter/choice metadata |
| POU / parser kernel | `crates/dfdl-core/src/kernel/parser/mod.rs` | `PointOfUncertainty`, `pou_stack`, choice backtracking |
| Discriminators | `crates/dfdl-core/src/kernel/parser/element.rs` (`evaluate_discriminator`) | Expression vs pattern tests |
| Delimiters | `crates/dfdl-core/src/kernel/parser/delimiters.rs` | Token model, `%` entities, WSP\*, encoding-aware matching |
| Patterns | `crates/dfdl-core/src/pattern.rs` | Lookahead rewrite for `regex` crate vs ICU/Java |
| Limits | `crates/dfdl-core/src/limits.rs` | Nesting, backtrack window, work budget |
| Expressions | `crates/dfdl-core/src/expr/mod.rs`, `eval.rs` (via tests in suite) | Pest parser, namespace validation §23 |
| Schema compiler | `crates/dfdl-schema/src/lib.rs`, `compiler/lower.rs`, `elements.rs` (grep) | XSD→IR, `dfdlx:` extensions |
| TDML runner | `crates/dfdl-tests/src/tdml.rs`, `lib.rs` (`run_apache_daffodil_official_tdml_suite_impl`) | Parser, schema cache, reporting |
| CLI | `crates/dfdl-cli/src/main.rs` | Host entrypoint |
| VM | `crates/dfdl-vm/src/lib.rs` | Bytecode path, validation off by design |
| Host std | `crates/dfdl-std/src/lib.rs` | Adapter maturity |
| Wireshark | `crates/dfdl-wireshark/src/lib.rs` | Dissector / PCAP story |

**Scale:** ~80k lines of Rust under `crates/`; `dfdl-core` kernel alone is tens of thousands of lines (parser modules + large `unparser.rs`).

---

## Findings (expert voice)

### Spec alignment and Daffodil interop

Running the **full ignored** Apache TDML suite against the bundled tree (`198` `.tdml` files) is the strongest objective signal in this repo. The harness executed **4,349** cases across **194** suites and reported **4,339 passes (99.77%)** and **10 failures (0.23%)**, with **zero infoset mismatches** on successes.

Failure clustering (from harness stderr) is informative:

- **7** unexpected parse outcomes — mostly **`conformance_reporter_test.tdml`** (synthetic negative tests: leftover data, delimiter mismatch, choice failure, invalid int/date text) plus **`Facets.tdml`** `largeNumRows` hitting **`WorkBudgetExhausted`**.
- **2** “expected error not raised” — same reporter TDML (unparse/parse should fail but succeeded).
- **1** schema compilation rejection — undefined `fn:` prefix for `fn:unsupportedFunction()` (arguably correct SDE behavior).

So the remaining gap vs Daffodil is **not** broad infoset drift; it is **error-classification edge cases**, **reporter TDML semantics**, and **resource budget tuning** on stress cases. That is a much healthier profile than a partial property matrix would suggest — but only if you treat TDML as the oracle (which I did here).

The IR and compiler clearly target **DFDL 1.0 §§5–16** semantics: rich `LengthKind`, separator policies, occurs kinds, nil kinds, choice dispatch keys, discriminators (expression + pattern), in-scope delimiters, etc. The schema compiler also accepts **`dfdlx:`** Daffodil-style extensions (`repType`, `choiceBranchKeyRanges`, layers, etc.), which matters for real-world schemas and Daffodil TDML.

### Architecture quality

**Crate boundaries match how DFDL practitioners think:** XML streaming without XSD knowledge (`dfdl-xml`), compilation and property resolution (`dfdl-schema`), execution (`dfdl-core`). No cycles; `dfdl-core` does not pull XML. Workspace **Clippy denies** `unwrap`, indexing, `todo`, `unimplemented`, etc. — rare in open-source parsers and appropriate for binary/text formats.

The **compiled schema IR** is the right center of gravity: immutable graph, precompiled expression ASTs and regex objects on terms, explicit validation facets on types. That is how you avoid re-parsing `{...}` on every element at runtime.

The **parser kernel** exposes an explicit **POU stack** with discriminated resolution — the heart of DFDL correctness for choices, optional content, and initiated content. Delimiter handling is split into a dedicated module with a token algebra (including DFDL `%` character entities and WSP\* / WSP+), which reflects real spec pain points rather than naive string splits.

**Pattern support** shows practitioner awareness: DFDL’s trailing positive lookahead is rewritten for the `regex` crate with documented limitations (no negative lookahead/behind). That is a pragmatic tradeoff with a clear failure surface.

**Resource governance** (`ResourceLimits`, `WorkBudget`) is spec-adjacent but essential for untrusted inputs — distinguishes this from academic parsers.

### Secondary execution path (`dfdl-vm`)

`dfdl-vm` is a coherent **performance bet**: linear bytecode, arena sinks, O(1) choice rollback via truncation, register machine. Documentation states **validation/facet checks are skipped** in VM mode (equivalent to `ValidationMode::Off`). Benchmark tests compile real schemas to VM and round-trip — good — but this is a **second full engine** to keep in sync with the tree-walking kernel. Today it is **not** wired through CLI or TDML.

### Host layer and operability

**`dfdl-cli`** currently prints a banner only — no compile, parse, unparse, or TDML driver. For operators comparing to `daffodil parse` / `daffodil test`, this is a **complete product gap**.

**`dfdl-std`** re-exports crates and advertises “adapters ready” without substantive `Read`/`Write`/filesystem resolver implementations in the snippet reviewed — integrators must use `dfdl-core` APIs directly.

**`dfdl-wireshark`** is a surprising plus for practitioners (dissector tree, PCAP runner, Lua generator, FFI hooks) — suggests domain users (ASTERIX-style metadata) were in mind.

### Toolchain / MSRV

There is **no `rust-toolchain.toml` or `rust-version` in the workspace manifest** I inspected. On **Rust 1.83**, the project **does not build** because of stabilized-in-later-Rust `is_multiple_of`. On **1.99 stable**, it builds cleanly. **`no_std` cross-check to `thumbv7m-none-eabi`** succeeded for `dfdl-core`, `dfdl-xml`, and `dfdl-schema` after installing target std — matching architectural claims in `ARCHITECTURE.md`.

### Test harness honesty

The official TDML test **`test_apache_daffodil_official_tdml_suite`** is **`#[ignore]`** and, when run, **asserts only** `suites_executed > 0`, `total_cases > 0`, and `total_passed > 0`. It does **not** require 100% pass or fail on the ten mismatches. CI default `cargo test --workspace` therefore **does not gate** Daffodil parity. The 150 `dfdl-tests` unit tests include smoke subsets and many hand-written regressions — valuable, but not a substitute for a hard TDML threshold.

### What would impress a DFDL practitioner

- Bundled **full Daffodil TDML tree** and a runner that understands **`defineConfig`**, tunables, encodings, and schema caching keys aligned with Daffodil policy flags.
- Explicit **POU** modeling and nested-choice discriminator tests in `kernel/parser/tests.rs`.
- **Sub-byte / encoding** work (encoding module tests, delimiter encoding on parser engine).
- **Leftover data** detection after root parse (Wireshark engine also surfaces trailing bytes).
- **Structured `DFDLErrorKind`** including `WorkBudgetExhausted` and `ImplementationLimit` — maps to spec diagnostic classes.
- **Panic-free discipline** enforced workspace-wide.

### Honest gap vs Apache Daffodil

| Dimension | This engine | Daffodil (typical) |
|-----------|-------------|-------------------|
| TDML on official vectors | ~99.8% pass (this run); soft CI gate | CI is TDML-first; releases tied to suite |
| CLI / IDE ergonomics | Stub CLI | Mature CLI, UDF, debugging |
| Ecosystem | Rust library + Wireshark experiment | JVM, XML stack, commercial support paths |
| Expression / ICU regex | Pest + `regex` + lookahead rewrite | Full Java/ICU alignment |
| Unparse + validation | Kernel supports validation mode; VM skips facets | Full symmetric pipeline in one runtime |
| Schema editing / diagnostics UX | Compiler errors exist; no user-facing formatter in CLI | Rich SDE reporting |

Interop for **already-validated Daffodil schemas and test vectors** is **very high**. Replacing Daffodil **operationally** (tools, strict regression policy, JVM ecosystem, extension corner cases) is **not** there yet.

---

## Severity-ranked issues

| Sev | Issue | Evidence |
|-----|--------|----------|
| **P0** | Default Rust **1.83 fails to compile**; no pinned MSRV in repo | `cargo check --workspace` → 8× `is_multiple_of` errors |
| **P1** | **Full TDML not a hard gate** — 10/4349 failures still yield green ignored test | `lib.rs` asserts `total_passed > 0` only; run output |
| **P1** | **`dfdl-cli` non-functional** for parse/compile/unparse | `main.rs` is banner-only |
| **P2** | **`WorkBudgetExhausted`** on `Facets.tdml` `largeNumRows` | TDML run stderr |
| **P2** | **Dual runtimes** (tree kernel vs VM) with different validation semantics | `dfdl-vm` docs; benchmarks use VM |
| **P2** | **Regex lookahead rewrite** has documented structural limits | `pattern.rs` module docs |
| **P3** | **`dfdl-std` thin** — limited host I/O story | `dfdl-std/src/lib.rs` |
| **P3** | Reporter TDML cases: **expected-failure tests** disagree on whether parse/unparse should fail | 2× “expected error not raised” in TDML run |

---

## Verdict

**Engine core (compile → IR → parse/unparse → TDML): A− / production-leaning for library embedders** who control toolchain (≥ stable with `is_multiple_of`), budgets, and integration.

**Product readiness (CLI, strict conformance CI, host std): C / pre-release** — impressive internals, incomplete operator surface.

**Spec alignment:** **High** on Daffodil’s bundled official TDML for data and infoset outcomes; **residual risk** concentrated in error reporting semantics, synthetic reporter tests, and work-limit defaults — not in mass infoset divergence.

**Biggest technical risks going forward:** (1) maintaining **two execution engines**; (2) **regex/ICU semantic gaps** on exotic patterns; (3) **POU/backtracking complexity** in the monolithic parser modules as schemas grow; (4) **release discipline** without MSRV + strict TDML threshold.

**Recommendation to stakeholders:** Pin Rust (e.g. `rust-toolchain.toml`), make the full TDML run a **required** check with an explicit pass-rate budget (ideally 100% minus documented waivers), finish **`dfdl-cli`** atop `dfdl-std`, and treat **`dfdl-vm`** as either the primary path with parity tests or a clearly scoped fast subset until validation catches up.

---

## Related documentation

- [dfdl-test-summary.md](dfdl-test-summary.md) — commands and TDML statistics from this run  
- [dfdl-core-indepth-assessment.md](dfdl-core-indepth-assessment.md) — `dfdl-core` module review  
- [dfdl-standard-implementation-gaps.md](dfdl-standard-implementation-gaps.md) — clause-level runtime gaps  
- [CONFORMANCE-PROFILES.md](CONFORMANCE-PROFILES.md) — OGF vs Daffodil compile profiles  
