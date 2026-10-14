# DFDL conformance profiles

This implementation supports two documented schema profiles. Schemas compiled with
`dfdl-schema` are always checked against **Core compile rules**; additional properties
activate the **Daffodil alignment profile**.

## 1. OGF DFDL 1.0 core profile

**Normative reference:** OGF GFD-R-P.240 (DFDL 1.0).

**Intent:** Schemas using only standardized DFDL 1.0 properties and OGF namespace
`http://www.ogf.org/dfdl/dfdl-1.0/`.

**Compile-time guarantees (partial — see gap analyses):**

- Property scoping via `PropertyStore` / effective properties (GFD §6.3).
- Structural SDEs for groups, choices, hidden groups, delimiter/escape conflicts.
- Facet and lengthKind combination checks in `compiler/lower.rs`.

**Known compile-time rejections (non-exhaustive):**

- `dfdl:lengthKind='endOfParent'`
- `dfdl:textBidi='yes'`, `dfdl:floating='yes'`
- Many `textNumberRep='zoned'` type combinations
- Selected calendar/binary combinations (see Project conformance gap analyses: dfdl-schema, dfdl-core, IR)

**Not in this profile:** Any property in the `dfdlx` or `daf` namespaces (see below).

## 2. Daffodil alignment profile (dfdlx + daf)

**Intent:** Compatibility with Apache Daffodil TDML and extension schemas.

**Additional namespaces:**

- `http://www.ogf.org/dfdl/dfdl-1.0/extensions` (`dfdlx:`)
- Daffodil attribute prefix `daf:` (accepted as alias in `annotation.rs`)

**Extension properties compiled by this codebase (whitelist where noted):**

| Property / feature | Compile behavior |
|--------------------|------------------|
| `dfdlx:repType`, `repValues`, `repValueRanges` | Validated in `elements.rs`; merged in `lower.rs` |
| `dfdlx:choiceBranchKeyRanges` | Range parse + conflict SDEs in `lower.rs` |
| `dfdlx:objectKind='bytes'` | Requires explicit/prefixed length |
| `dfdlx:layerTransform` / `daf:layerTransform` | Whitelist `is_known_layer` in `lower.rs` |
| `dfdlx:parseUnparsePolicy` / `daf:parseUnparsePolicy` | Parsed like `parseUnparsePolicy` |
| `daf:escalateWarningsToErrors` | `SchemaCompiler` tunable |
| `daf:invalidRestrictionPolicy` | `Ignore` strips pattern facets at lower |
| `daf:allowExpressionResultCoercion` | Polymorphic group check tunable |
| TDML tunables | `requireTextBidi`, `requireFloating`, `requireEncodingErrorPolicy`, etc. |

**Layer whitelist:** Documented in `is_known_layer` (`lower.rs`); unknown layers → SDE.

## 3. How to select a profile

| Goal | Action |
|------|--------|
| Strict OGF-only | Reject schemas containing `dfdlx:` / `daf:` attributes (not enforced automatically today). |
| TDML / Daffodil | Default compiler settings + extension properties as required by corpus. |
| Document gaps | See Project conformance gap analyses (dfdl-schema, dfdl-core, IR). |

## 4. Relationship to runtime

Compilation success under the Daffodil profile **does not** imply full GFD runtime
semantics. Kernel gaps (e.g. `binaryFloatRep`, `ValidationMode::Full`) are documented
separately in Project conformance gap analyses (dfdl-schema, dfdl-core, IR).

## 5. Maintenance

- Regenerate SDE tables: `rg 'Schema Definition Error|not implemented|not supported' crates/dfdl-schema`
- After compiler changes: `graft build` (if graph indexed) and update Project conformance gap analyses.
