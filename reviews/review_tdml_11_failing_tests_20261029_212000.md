# Test Suite & TDML Conformance Review: Remediation of 11 Failing Tests
**Timestamp**: 2026-10-29 21:20:00 UTC  
**Scope**: `dfdl-core`, `dfdl-schema`, `dfdl-tests`  
**Status**: 100% Passing (0 failures in TDML test suite runner, 284/284 unit tests passing, clippy clean).

---

## 1. Problem Identification & Root Cause

The user reported:
> "fix TDML suites with 11 failing tests"

Running `cargo test -p dfdl-tests -- test_apache_daffodil_official_tdml_suite` had failed at assertion:
```text
assertion `left == right` failed: No missed errors allowed
  left: 11
 right: 0
```

The 11 failing tests reported by the diagnostic engine were:
1. `InputValueCalc_11` (`inputValueCalc.tdml`) — syntax error in `dfdl:inputValueCalc="{ ..\abcde }"`.
2. `asterisk_01` (`expressions.tdml`) — unsupported wildcard in `dfdl:inputValueCalc="{ /ex:e1/ex:nest/* }"`.
3. `asterisk_02` (`expressions.tdml`) — unsupported wildcard and double-slash in `dfdl:inputValueCalc="{ //* }"`.
4. `ancestor_axis_01` (`expressions.tdml`) — unsupported axis in `dfdl:inputValueCalc="{ ancestor::ex:e4/ex:c1 }"`.
5. `ancestor_or_self_axis_01` (`expressions.tdml`) — unsupported axis in `dfdl:inputValueCalc="{ ancestor-or-self::ex:c2/ex:d1 }"`.
6. `namespace_axis_01` (`expressions.tdml`) — unsupported axis in `dfdl:inputValueCalc="{ namespace::* }"`.
7. `diagnostics_01` (`expressions.tdml`) — invalid expression in `dfdl:inputValueCalc="{ /invalid! }"`.
8. `diagnostics_02` (`expressions.tdml`) — invalid expression in `dfdl:inputValueCalc="{ ! }"`.
9. `diagnostics_03` (`expressions.tdml`) — invalid expression in `dfdl:inputValueCalc="{ ././.\\/ }"`.
10. `var_end_path` (`variables.tdml`) — variable at path end in `dfdl:inputValueCalc="{ /ex:r/$var }"`.
11. `var_in_path` (`variables.tdml`) — variable in path in `dfdl:inputValueCalc="{ $var2/ex:e1 }"`.

### Root Cause
All 11 tests involved an invalid or unsupported DFDL expression attached via `dfdl:inputValueCalc`.
1. **Error Swallowing**: During property resolution ([`resolution.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties/resolution.rs)), schema compilation ([`builder.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/builder.rs)), and runtime element execution ([`element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)), calls to `parse_expr` converted failures to `None` using `.ok()` or `if let Ok(...)`.
2. **Fallthrough to Bitstream**: Because `input_value_calc_prop` was silently set to `None`, the parser fell through to reading the element directly from the bitstream. For tests with empty or matching inputs, parsing succeeded, resulting in "expected parse error but succeeded".
3. **Pest Grammar Limitations**:
   - `axis_specifier` only matched `"parent::" | "self::" | "child::"`, causing unsupported axes (e.g. `ancestor::`, `attribute::`, `namespace::`) to fail pest parsing with generic syntax errors instead of spec-conforming `Schema Definition Error: Unsupported axis '<name>'`.
   - `path` did not support double-slash prefixes (`//`) or lookahead for `@` separators (`&"@"`).
   - `step_node` did not match variable references (`$var`), preventing specific diagnosis of variable usage within paths.

---

## 2. Technical Remediation

### A. Pest Expression Grammar Updates ([`dfdl_expression.pest`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/dfdl_expression.pest))
- Expanded `axis_specifier` to `@ | (ASCII_ALPHA | "-")+ ~ "::"`.
- Expanded `path` to `("//" | "/")? ~ path_step ~ (path_sep ~ path_step)* | "//" | "/"`, where `path_sep = _{ "//" | "/" | &"@" }`.
- Added `var_step = @{ "$" ~ (qname_ident ~ ":")? ~ qname_ident }` to `step_node`.

### B. Path Expression Parsing ([`crates/dfdl-core/src/expr/parser.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/parser.rs))
- In `parse_path_from_pair`:
  - `axis_specifier`: maps `"@" | "attribute::"` to `Schema Definition Error: Unsupported axis 'attribute' in path expression.`.
  - Maps any other non-standard axis to `Schema Definition Error: Unsupported axis '<name>' in path expression.`.
  - `Rule::var_step`: returns `Schema Definition Error: Variables cannot be used in path expressions.`.

### C. Strict Error Propagation on `inputValueCalc` & `outputValueCalc`
- **[`resolution.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/properties/resolution.rs)**: Replaced `.ok()` in `to_resolved_properties_with_limits` with `?` and mapped expression errors to `Schema Definition Error: Invalid expression '<expr>': Unable to parse expression: <err>`.
- **[`builder.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/schema/builder.rs)**: Replaced `if let Ok(ast)` in `add_term_with_props` with strict `?` propagation.
- **[`element.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs)**: In `parse_simple_element_scalar`, replaced `.ok()` fallback with strict `?` error propagation when `input_value_calc` is present.

---

## 3. Verification Matrix

| Test Suite / Command | Scope | Result | Details |
| :--- | :--- | :--- | :--- |
| `cargo test -p dfdl-tests -- test_apache_daffodil_official_tdml_suite` | Apache Daffodil Official TDML Suite | **PASS (100%)** | 0 missed errors, 0 unexpected parse errors, 0 unparse errors, 0 infoset mismatches. |
| `cargo test -p dfdl-core` | Core library unit & integration tests | **PASS (284/284 + 4 doc tests)** | All tests passing. |
| `cargo clippy -p dfdl-core -p dfdl-benchmark -p dfdl-tests -- -D warnings` | Clippy static analysis | **PASS (0 warnings)** | Clean across all packages. |
| `cargo run --release -p dfdl-benchmark` | FakeTDL end-to-end performance suite | **PASS** | Unparse latency: 7.02 µs (8.65 MB/s), File unparse: 17.63 µs (10.37 MB/s). |
