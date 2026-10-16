# DFDL 1.0 §18 / §23 — Expression language gap inventory

**Scope:** `crates/dfdl-core/src/expr/` only (`parser.rs`, `eval.rs`, `ast.rs`, `variables.rs`, `dfdl_expression.pest`, `mod.rs`). **Excludes** `dfdl-vm` and compile-time lowering in `dfdl-schema` (except where noted for namespace SDEs).

**Normative reference:** OGF GFD-R-P.240 §18 (expression syntax), §23 (paths, functions, standard variables). **Conformance corpus cross-check:** Apache Daffodil TDML under `crates/dfdl-tests/tests/daffodil/section23/` (function names used in tests, not pass/fail).

---

## Architecture

| Component | Role | Primary source |
|-----------|------|----------------|
| Pest grammar | Token/syntax | `dfdl_expression.pest` |
| `parse_expr` / `build_*` | AST + compile-time SDEs | `parser.rs` |
| `eval_expr` | Interpret AST; path-aware pre-dispatch for some `fn:*` / `dfdl:*` | `eval.rs` ~250–573 |
| `eval_fn_call` | Function dispatch by **`local_name`** (prefix mostly ignored except `trace` / `replace`) | `eval.rs` ~2063–3246 |
| `lookup_variable` | `$var` + §7.4 accessor variables | `eval.rs` ~611–636 |
| `validate_expression_namespaces` | §23.1 prefix declarations | `mod.rs` ~21–142 |

---

## Syntax & operators (§18)

### Supported (parse + eval)

| Category | Forms | Parser | Evaluator |
|----------|--------|--------|-----------|
| Conditional | `if (e) then e else e` | `dfdl_expression.pest:7`, `parser.rs:69` | `eval.rs` `IfThenElse` arm |
| Logical | `and`, `or`, unary `not` | `pest:9–13`, `parser.rs:70–71` | Short-circuit `BinaryOp::And/Or` ~264–317; strict bool required |
| Value equality | `eq`, `ne` | `pest:16`, `parser.rs:72–84` | `eval_binary` `Eq/Ne` ~1405–1406 (`==` on `DfdlValue`) |
| Value relational | `lt`, `le`, `gt`, `ge` | `pest:19`, `parser.rs:86–101` | `eval_cmp` ~1826–1868 |
| Arithmetic | `+`, `-`, `*`, `div`, `idiv`, `mod` | `pest:21–25`, `parser.rs:103–117` | `eval_add/sub/mul/div/idiv/mod` ~1429+ |
| Unary numeric | `+`, `-` | `pest:28`, `parser.rs:193–221` | `eval_unary` |
| Literals | decimal int, hex `0x`, float, `true`/`false`, quoted strings | `pest:32–40`, `parser.rs:235–289` | `Literal` arm |
| Grouping | `( … )` | `pest:30` | — |
| Variables | `$name`, `$prefix:name` | `pest:42`, `parser.rs:292–296` | `lookup_variable` ~611–636 |
| Function calls | `name(args…)` | `pest:44–45`, `parser.rs:298–315` | `FnCall` → `eval_fn_call` |
| Paths | `/`, `.`, `..`, `child::`, `self::`, `parent::`, `[predicate]` | `pest:47–57`, `parser.rs:317–423` | `resolve_path` / `normalize_infoset_path` ~915+ |

### Rejected at parse time (SDE) — general comparisons

Grammar **accepts** `=`, `!=`, `<`, `<=`, `>`, `>=`, but `build_binary_chain` **rejects** them with `DFDLErrorKind::SchemaDefinition`:

```72:101:crates/dfdl-core/src/expr/parser.rs
        Rule::equality => build_binary_chain(
            pair,
            |op_str| match op_str {
                "=" | "!=" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('eq' or 'ne') instead",
                        op_str
                    ),
                )),
                // ...
            },
        ),
        Rule::relational => build_binary_chain(
            pair,
            |op_str| match op_str {
                "<=" | ">=" | "<" | ">" => Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    &alloc::format!(
                        "Schema Definition Error: Unsupported operation: General comparison operator '{}' is not supported in DFDL expressions; use value comparison operator ('lt', 'le', 'gt', 'ge') instead",
                        op_str
                    ),
                )),
```

Tests lock this behavior: `parser.rs:592–598`.

### Comparison semantics (value ops) — subset / divergent

- **`eq` / `ne`:** structural equality on `DfdlValue` after numeric operands are coerced for arithmetic only — not XPath “value comparison” for all XSD types (`eval.rs` ~1396–1406).
- **`lt`/`le`/`gt`/`ge`:** `eval_cmp` ~1826–1868 — booleans and strings compared directly; floats/decimals via `value_to_f64`; **hexBinary comparisons SDE**; other types fall through to `value_to_i64` (lossy).
- **Logical ops on non-boolean:** `TypeError` (~276–287, ~1405–1416).

---

## Paths & wildcards (§23.2)

| Feature | Spec / TDML usage | Implementation | Gap |
|---------|-------------------|----------------|-----|
| Standalone `*` step | XPath wildcard | SDE at parse: `parser.rs:360–362` | **Missing** (intentional reject) |
| `prefix:*` step | NCName wildcard | Grammar allows `local_name = *` (`pest:56`) but no SDE; not same as `wildcard_step` | **Unclear / likely broken** (no wildcard navigation) |
| `fn:count` with `*` in path | TDML | Runtime only: if last step local name is `"*"`, counts all element children (`eval.rs:371–372`) | **Divergent** (wildcard works here but not in path syntax) |
| Numeric predicates `[n]` | Array index | Parsed; validated + evaluated ~1040–1049 | **Subset** (expression predicates on non-arrays → SDE ~1028–1034) |
| `@` attribute axis | XPath | `@invalid` → parse error (`parser.rs:571–572`) | **Missing** |

---

## `fn:*` inventory (XPath / XQuery subset in §18)

Dispatch is by **`name.local_name`** in `eval_fn_call` (`eval.rs:2136–3245`) unless `eval_expr` intercepts path arguments first (`eval.rs:326–558`).

### Implemented (match arms in `eval_fn_call`)

| Function | Notes |
|----------|--------|
| `error` | Raises `ExpressionError` ~2137–2143 |
| `concat`, `string-length`, `substring`, `substring-before`, `substring-after` | String ops ~2165–2285 |
| `contains`, `starts-with`, `ends-with`, `upper-case`, `lower-case` | **3-arg collation forms ignored** (extra args not validated) |
| `abs`, `ceiling`, `floor`, `round` | NaN → Parse Error on float/double ~2327–2435 |
| `round-half-to-even`, `roundHalfToEven` | Custom half-even ~3189–3227 |
| `matches` | **Divergent:** substring `contains`, not regex ~2437–2442 |
| `exists`, `count` | **Also** path-aware in `eval_expr` ~343–381; **fallback** in `eval_fn_call` is trivial (~2444–2452, 2826–2834) |
| `not`, `true`, `false`, `boolean` | ~2836–2885 |
| `compare` | Lexicographic on string form ~2662–2672 |
| `encodeDFDLEntities`, `decodeDFDLEntities` | DFDL entity codec ~2178–2186 |
| Date/time extractors | `year-from-date`, `month-from-date`, `day-from-date`, `year-from-dateTime`, … `seconds-from-time` ~2688–2770 |
| **Type constructors** (typically `xs:*`, parsed as local name) | `string`, `int`, `long`, `short`, `byte`, `unsigned*`, `float`, `double`, `decimal`, `integer`, `nonNegativeInteger`, `boolean`, `hexBinary`, `date`, `dateTime`, `time` ~2533–3188 |

### Path-special-cased in `eval_expr` (before `eval_fn_call`)

| Function | Behavior |
|----------|----------|
| `exists`, `empty` | Path argument → infoset existence ~343–351 |
| `count` | Path → child count (supports `*` target) ~353–381 |
| `valueLength`, `contentLength` | Path → full DFDL length logic ~383–538 |
| `checkConstraints` | Path / self + nil handling ~540–557 |
| `trace` | **`fn:trace` SDE** ~327–331; unprefixed `trace(path)` resolves path ~332–341 |

### Explicitly prohibited

| Function | Where |
|----------|--------|
| `fn:trace` | `eval.rs:327–330`, `3229–3232`; `mod.rs:66–70` (namespace validation) |

### Missing vs Daffodil §23 `Functions.tdml` corpus

These **`fn:` names appear in TDML** but have **no** `eval_fn_call` arm (default → `Unsupported function` ~3237–3244):

`adjust-date-to-timezone`, `exactly-one`, `local-name`, `namespace-uri`, `nilled`, `timezone-from-time`, and **`empty`** when not given a path argument (only path form handled in `eval_expr`).

Other common XPath 2 functions (non-exhaustive): `position`, `last`, `sum`, `avg`, `min`, `max`, `distinct-values`, `deep-equal`, `regex-*`, `QName`, `node-name`, etc. — **all missing**.

---

## `dfdl:*` inventory (§23.1 functions)

### Implemented (`eval_fn_call` local_name)

| Function | Gap notes |
|----------|-----------|
| `valueLength`, `contentLength` | Rich path logic in `eval_expr` ~383–538; value-only fallback ~2145–2163 (heuristic byte lengths) |
| `lookAhead` | Requires `ctx.lookahead_fn`; **512-bit cap** SDE ~2806–2811; else `0` ~2820–2821 |
| `currentPosition` | **Stub:** always `0` ~2824 |
| `occursIndex` | From `ctx.occurs_index` ~2825 |
| `checkConstraints` | Facet + range checks ~2454–2531 |
| `checkRangeInclusive`, `checkRangeExclusive` | Float comparison ~3100–3124 |
| `replace` | Literal substring replace; rejects unknown prefix ~2886–2905 |
| `encodeDFDLEntities`, `decodeDFDLEntities` | Same as `fn:*` names |

### Missing vs TDML `section23` function calls

No implementation in `dfdl-core` (grep over `expr/`):

`testBit`, `setBits`, `containsEntity`, `containsDFDLEntities`, `stringLiteralFromString`, `timeZoneFromDate`, `timeZoneFromDateTime`, `timeZoneFromTime`.

(Also **`dfdl:lookAhead` / `dfdl:currentPosition`** are present but stubbed/capped as above.)

### Daffodil / test extensions (implemented, not strict OGF)

| Names | `eval.rs` arms |
|-------|----------------|
| `bitAnd`, `bitOr`, `bitXor`, `bitNot`, `leftShift`, `rightShift` | ~2939–3067 (errors cite `dfdlx:`) |
| `sayHello`, `addBoxed`, `addPrimitive` | ~2907–2937 |
| `prim*Func`, `boxed*Func`, `javaBigDecimalFunc`, `javaBigIntegerFunc` | ~3068–3098 |
| Unprefixed `trace` | ~3229–3235 (pass-through) |

---

## Variables (§7 / §23.1)

| Kind | Support | Gap |
|------|---------|-----|
| User variables | `VariableMap` + `$name` (`variables.rs`, `lookup_variable` ~617–624) | Lifecycle §7.7 in `variables.rs`; expression engine reads map only |
| §7.4 accessors `$dfdl:encoding`, `$byteOrder`, `$binaryFloatRep`, `$outputNewLine` | Hardcoded defaults ~627–631 | **Divergent** — not read from effective properties |
| Inline `ctx.variables` slice | Match on `local_name` only ~612–615 | Prefix on `$dfdl:…` not distinguished in slice lookup |

---

## Namespace validation (§23.1)

`validate_expression_namespaces` (`mod.rs:21–44`) walks AST and requires declared prefixes (except `xml`, `xmlns`, `xs`, `xsd`). **`fn` prefix must be declared** or compile fails — TDML negative test expects this (`Functions-neg.tdml`). **`fn:trace`** always SDE during validation ~66–70.

---

## Error strings (quick index)

| Condition | Message pattern | Location |
|-----------|-----------------|----------|
| Unknown function | `Schema Definition Error: Unsupported function: {prefix}:{name}` | `eval.rs:3243` |
| General comparison | `Unsupported operation: General comparison operator` | `parser.rs:78`, `92` |
| Wildcard step | `Wildcard '*' is unsupported in DFDL Expression Syntax.` | `parser.rs:361` |
| `fn:trace` | `Unsupported function: fn:trace` | `eval.rs:329`, `3231`; `mod.rs:69` |
| Lookahead limit | `Look-ahead distance of {} bits exceeds … 512 bits` | `eval.rs:2807–2810` |
| Undefined `$var` | `Undefined expression variable` | `eval.rs:634` |

---

## Summary matrix

| Area | Status |
|------|--------|
| Core syntax (if, booleans, arithmetic, value compares) | **Mostly conformant** |
| General compares `=`, `<`, … | **Rejected (SDE)** |
| Wildcard paths | **Rejected (SDE)**; partial `*` only in `fn:count` |
| Core `fn:` string/numeric/boolean/date extractors | **Largely implemented** |
| `fn:matches`, `fn:count`/`exists` on non-path args | **Divergent / stub** |
| `fn:trace` | **Prohibited** |
| XPath node tests (`namespace-uri`, `local-name`, …) | **Missing** |
| Normative `dfdl:` bit/entity/timezone helpers | **Missing** (bitwise via **dfdlx** instead) |
| `dfdl:currentPosition`, `$dfdl:*` accessors | **Stub / hardcoded** |
| Namespace prefix declarations | **Enforced** |

---

*Generated from source inspection of `dfdl-core/src/expr/`.*
