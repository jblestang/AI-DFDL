# DFDL workspace test summary

Generated: 2026-10-14 (Cloud Agent, `/workspace`)

## Environment

| Item | Value |
|------|--------|
| OS | Linux 6.12.94+ x86_64 |
| Default toolchain | `rustc 1.83.0` (active via rustup) |
| Stable toolchain | `rustc 1.99.0` (installed during this run; was not preinstalled) |
| Cargo (stable) | `cargo 1.99.0` |
| `CARGO_HOME` | `/usr/local/cargo` |
| Workspace | `/workspace` (12 member crates) |

## Toolchain note (default vs stable)

**Default Rust 1.83.0 does not compile the workspace.** `cargo test -p dfdl-core` failed with 9 errors in `dfdl-core`:

- **E0658** (8×): `is_multiple_of` on unsigned integers — unstable on 1.83 (`crates/dfdl-core/src/kernel/parser/text.rs`, `mod.rs`, `unparser.rs`, `types.rs`, `util.rs`).
- **E0599** (1×): no method `abs` on `f64` in `crates/dfdl-core/src/expr/eval.rs:3216` (likely `no_std` / import path interaction on older compiler).

All test commands below used **`rustup run stable cargo …`**. No test failures were observed on stable; only compile failures on default.

## Per-crate results (`cargo test --workspace` on stable)

| Crate | Unit / integration tests | Passed | Failed | Ignored |
|-------|--------------------------|--------|--------|---------|
| `dfdl-benchmark` | lib | 18 | 0 | 0 |
| `dfdl-cli` | main | 1 | 0 | 0 |
| `dfdl-core` | lib | 295 | 0 | 0 |
| `dfdl-example` | lib | 7 | 0 | 0 |
| `dfdl-example` | `tests/integration_tests.rs` | 8 | 0 | 0 |
| `dfdl-fuzz` | lib | 1 | 0 | 0 |
| `dfdl-schema` | lib | 114 | 0 | 0 |
| `dfdl-std` | lib | 1 | 0 | 0 |
| `dfdl-tests` | lib | 150 | 0 | 1 |
| `dfdl-vm` | lib | 20 | 0 | 0 |
| `dfdl-wireshark` | lib | 9 | 0 | 0 |
| `dfdl-xml` | lib | 9 | 0 | 0 |
| **Subtotal (executable)** | | **633** | **0** | **1** |
| Doc-tests | `dfdl-core` | 4 | 0 | 0 |
| Doc-tests | other crates | 0 | — | — |

**Workspace exit code:** 0. **633** Rust tests executed and passed; **1** ignored (`test_apache_daffodil_official_tdml_suite`).

Ignored test (not run in default workspace test):

```text
#[ignore = "Full Apache Daffodil official TDML benchmark suite (4,337 test cases) - run explicitly with -- --ignored"]
fn test_apache_daffodil_official_tdml_suite()
```

## `dfdl-core` only (stable)

```bash
rustup run stable cargo test -p dfdl-core
```

- **295** lib tests passed; **4** doc-tests passed; **0** failed.

## Apache Daffodil official TDML suite (`--ignored`)

```bash
rustup run stable cargo test -p dfdl-tests test_apache_daffodil_official_tdml_suite -- --ignored --nocapture
```

- **Harness:** 1 test passed (the runner completes and prints a conformance report; TDML case failures are reported inside the test output, not as `cargo test` failures).
- **Duration:** ~12.5s (debug profile).

### TDML statistics (from runner report)

| Metric | Count |
|--------|------:|
| TDML files discovered | 198 |
| Suites executed | 194 |
| Total test cases | 4349 |
| Passed test cases | 4339 (99.8%) |
| Failed test cases | 10 (0.2%) |

**Failure cluster breakdown (10 cases):**

| Cluster | Count |
|---------|------:|
| Schema compilation rejections | 1 |
| Parse execution failures | 7 |
| Expected error not raised | 2 |
| Infoset mismatch | 0 |
| Unparse failures | 0 |

Notable samples (many from `conformance_reporter_test.tdml` — intentional reporter fixtures):

- Schema: unsupported `fn:unsupportedFunction()` namespace.
- Parse: delimiter mismatch, EOF/length, leftover bits, choice branch, invalid date text, work budget (`largeNumRows` in `Facets.tdml`).
- Expected errors: `tc_expected_error_succeeded`, `tc_expected_unparse_err_succeeded`.

## How to reproduce

```bash
cd /workspace

# Verify toolchains
rustc --version
rustup toolchain install stable   # if stable missing
rustup run stable rustc --version

# Core only
rustup run stable cargo test -p dfdl-core

# Full workspace (skips ignored TDML harness by default)
rustup run stable cargo test --workspace

# Full TDML conformance report (slow-ish; large embedded suite)
rustup run stable cargo test -p dfdl-tests test_apache_daffodil_official_tdml_suite -- --ignored --nocapture
```

Captured logs (this run): `/tmp/dfdl-core-test.log` (default, compile fail), `/tmp/dfdl-core-test-stable.log`, `/tmp/dfdl-workspace-test-stable.log`, `/tmp/dfdl-tdml-ignored.log`.

## Recommendations

1. **Pin MSRV** — Add `rust-version = "1.87"` (or newer; verify with `cargo +1.87 check`) to the workspace `Cargo.toml` because the codebase uses stabilized `is_multiple_of` on integers. Document that **1.83 is below MSRV** (matches default VM image failure).
2. **CI** — No `.github/` workflows were present in this checkout. Add a job that runs `rustup run stable cargo test --workspace` and optionally a scheduled or manual job for the ignored TDML harness.
3. **Environment images** — Bump default Rust in dev/CI images to **stable ≥ MSRV** or install `rustup toolchain install stable` in `environment.json` install steps.
4. **TDML gate** — If product goal is 100% official suite pass rate, triage the 10 failing cases (1 real conformance hit: `largeNumRows` work budget; remainder largely reporter/scaffold TDML).
