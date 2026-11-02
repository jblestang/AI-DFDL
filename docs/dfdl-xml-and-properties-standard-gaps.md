# DFDL 1.0 (OGF GFD-R-P.240) vs `dfdl-xml` + §6.3 Property Resolution

**Scope:** `/workspace/crates/dfdl-xml` (XML 1.0 + Namespaces in XML 1.0 for schema ingest) and `dfdl-core/src/expr/properties.rs` (`PropertyStore::to_resolved_properties`, §6.3 / §7 property syntax). **Excluded:** `dfdl-vm`, kernel parse/unparse (see `docs/dfdl-standard-implementation-gaps.md`), and compile-only SDEs in `dfdl-schema` except where they consume XML or call `to_resolved_properties`.

**Evidence:** Source and ripgrep only (no TDML run, no prior matrices).

**Consumers:** `dfdl-schema` drives `XmlReader` — `compiler.rs:244-245`; property stores call `to_resolved_properties` — `lower.rs:1806`, `2902`, `3131`; entity validation — `validate_property_entities()` — `lower.rs:344`, `767`, etc.

---

## Executive summary — top 10 gaps (user impact)

| Rank | Gap | Impact |
|------|-----|--------|
| 1 | **Schema compile enables permissive namespaces** | Undeclared prefixes on **elements** resolve to the prefix string as URI — `compiler.rs:245`, `reader.rs:393-400`; violates Namespaces in XML 1.0 (strict mode exists but is not used for XSD). |
| 2 | **`dfdl:binaryFloatRep` never resolved into IR** | XSD may carry the attribute; `to_resolved_properties` never reads it (no `binaryFloatRep` in `properties.rs`); accessor hardcodes `ieee` — `eval.rs:630`. |
| 3 | **`floating='yes'` / `textBidi='yes'` rejected at resolution** | Hard SDE in `to_resolved_properties` — `properties.rs:1012-1025`; no IR fields; §12 alignment/bidi unavailable. |
| 4 | **`lengthKind='endOfParent'` accepted in property layer** | Enum set at resolution — `properties.rs:1041`; compiler/kernel reject later — `lower.rs:1529-1540` (IR drift). |
| 5 | **`textNumberRep='zoned'` resolved but not fully implemented** | Mapped to `TextNumberRep::Zoned` — `properties.rs:2258-2260`; parse path SDE elsewhere; pattern rules only when `zoned` — `properties.rs:1594-1608`. |
| 6 | **Invalid property enumerations silently defaulted** | Unknown `occursCountKind` → `Implicit` — `properties.rs:1166`; unknown `lengthUnits` → `Bytes` — `1087`; unknown `nilKind` → `literalValue` — `1183`. |
| 7 | **UTF-8 `&str` input; declaration encoding not applied** | `XmlReader` only accepts UTF-8 Rust strings — `reader.rs:77`; `StartDocument.encoding` is informational — `reader.rs:522-527`, `event.rs:25-30`. |
| 8 | **DTD / external entities prohibited** | `Token::DtdStart` / `EntityDeclaration` → SDE — `reader.rs:698-703`; general entity refs other than predefined/numeric → error — `reader.rs:272-276`, `362-366`. |
| 9 | **`max_entity_expansion_bytes` limit unused** | Field and default documented — `limits.rs:16-30`; never checked in `decode_entities` / `decode_attribute_value` — `reader.rs:219-386`. |
| 10 | **Term-scoped vs inherited properties split** | Initiator/terminator/separator/skips local-only — `properties.rs:1117-1157`, `1212-1236`; `merge_parent` blocklist — `766-813`; matches much of §6.3 but differs from `resolve_property` for format-wide props. |

---

## Gap table (clause ↔ code)

| Clause | Requirement | Implementation | Gap type |
|--------|-------------|----------------|----------|
| **Namespaces §2 / NS §4** | Undeclared namespace prefix on element is error | Strict path: SDE — `reader.rs:394-397`; **compiler sets permissive** — `compiler.rs:245` | **Divergent** |
| **Namespaces §2** | Undeclared prefix on attribute is error | Always strict — `reader.rs:437-443` | **Conformant** |
| **Namespaces §4** | `xmlns` / `xmlns:xmlns` binding rules | SDE — `reader.rs:597-603`, `698-703` | **Conformant** |
| **XML §3.3.3** | Attribute value normalization (literal WS → space; char refs preserved) | `decode_attribute_value` — `reader.rs:297-386`; tests — `reader.rs:1115-1127` | **Conformant** |
| **XML §4.1** | Predefined + numeric character references in content | `decode_entities` — `reader.rs:245-270`; rejects others — `272-276` | **Subset** (no DTD entities) |
| **XML §2.8 / §4.2** | DOCTYPE / internal subset | Rejected (anti-XXE) — `reader.rs:698-703` | **Subset** (documented security profile) |
| **XML §4.7** | CDATA sections | Raw slice, no entity expansion — `reader.rs:672-677` | **Conformant** |
| **XML §2.10 / §4.3** | Well-formedness (matching tags, single root) | Tag stack + SDE — `reader.rs:511-515`, `647-652` | **Conformant** |
| **XML §4.3.2** | Duplicate attributes | SDE — `reader.rs:449-454` | **Conformant** |
| **XML §4.3.1** | `StartDocument` version / standalone / encoding | Only `encoding` string exposed; empty encoding → `UTF-8` — `reader.rs:522-527`; no `version`/`standalone` on event — `event.rs:25-30` | **Subset** |
| **XML (encoding)** | Declared encoding drives input decoding | Input is always `&str` (UTF-8); BOM stripped — `reader.rs:78-82` | **Divergent** vs multi-byte legacy encodings |
| **Implementation limits** | Bounded entity expansion | `max_entity_expansion_bytes` defined — `limits.rs:16-30`; **not enforced** in decoder | **Missing** |
| **Implementation limits** | Depth / attribute / token size | `check_depth`, `check_attribute_count`, `max_token_length` on names — `reader.rs:427-433`, `537-545`, `577-585` | **Conformant** (engine limits; not W3C minimums) |
| **§6.3** | Effective property values from local + inheritable parent | `resolve_property` — `properties.rs:963-977`; `merge_parent` — `766-819` | **Mostly conformant** |
| **§6.3** | Non-inheritable term properties (separator, initiator, …) | Excluded from `merge_parent` — `766-813`; local `get_property` in resolution — `1117-1157` | **Conformant** |
| **§6.3 / §12.3** | `lengthKind` includes `endOfParent` | Accepted — `properties.rs:1041`; compile SDE later — `lower.rs:1536` | **Missing** (downstream) |
| **§12** | `floating='yes'` | SDE — `properties.rs:1020-1025` | **Missing** |
| **§12** | `textBidi='yes'` | SDE — `properties.rs:1012-1017` | **Missing** |
| **§13.6** | `binaryFloatRep` | Not in `to_resolved_properties`; not in `ResolvedProperties` — `ir.rs:949+` | **Missing** |
| **§13.7.2** | `textNumberRep='zoned'` | Resolved — `properties.rs:2258-2260`; kernel/text SDE outside this file | **Subset** |
| **§7 / §7.4** | Property literal syntax (entities, whitespace) | `validate_dfdl_property_entities` — `181-331`; invoked via `validate_property_entities` — `881-885` (not inside `to_resolved_properties`) | **Conformant** (split API) |
| **§7.4** | Discriminator and assert both present | SDE — `properties.rs:1478-1480` | **Conformant** |
| **§18 / §23** | Expression namespace prefixes declared | `validate_expression_namespaces` during resolution — `1919-1951` | **Subset** (strict prefix rules) |
| **§6.3** | Invalid enumeration tokens | Several silently map to defaults (see executive summary) | **Divergent** |
| **§6.3** | Invalid `alignment` numeric | `parse` failure → `1` via `unwrap_or(1)` — `properties.rs:1107` | **Divergent** |
| **§6.3** | `nillable` | Local binding only — `properties.rs:1191` | **Subset** (no parent inherit via this field) |
| **—** | Daffodil `dfdlx` / `daf` at resolution | `rep_type`, `layer`, `string_as_xml`, extended `parseUnparsePolicy` — `1859-1911`, `2374-2398` | **Extension** |

---

## Deep dive: `dfdl-xml` (XML 1.0 + Namespaces)

**Architecture:** Streaming tokenizer via `xmlparser` — `reader.rs:16`; events in `event.rs`; limits in `limits.rs`. `#![no_std]` + `alloc` — `lib.rs:5-8`.

**Namespace resolution:** Stack per element — `reader.rs:36-42`, `422`; `xml` / `xmlns` fixed URIs — `116-120`; default namespace empty `xmlns=""` clears default — `144-148`, tests `936-940`. Helper APIs for compiler (`in_scope_namespace_bindings`, `current_element_namespace_bindings`) — `180-216`.

**Security / profile choices:**

- DTD and entity declarations rejected at first token — `698-703`.
- Only five predefined entities plus numeric refs — `245-250`, `334-339`.
- Surrogate scalar values rejected — `264-268`, `353-357`.

**Compiler integration gap:** `SchemaCompiler::compile_str_with_resolver_and_root` always calls `set_permissive_namespaces(true)` — `compiler.rs:245`. In that mode, missing element prefixes become `QName::with_namespace(pref, …)` using the prefix as namespace URI — `reader.rs:398-403`. TDML/tests sometimes use strict limits — `dfdl-tests/src/lib.rs:182-189`, `compiler/tests.rs:2045+`.

**Dead / incomplete limits:** `max_entity_expansion_bytes` is never read in `reader.rs` (grep: only `limits.rs`, tests). Decoded entity output can grow without bound aside from `max_token_length` on **names**, not attribute values or text bodies.

**Ignored tokens:** `next_event` matches `_ => {}` — `705`; behavior depends on `xmlparser` token coverage.

---

## Deep dive: §6.3 `to_resolved_properties`

**Entry:** `PropertyStore::to_resolved_properties(parent)` — `properties.rs:981-2410`. Builds frozen `ResolvedProperties` — `2086-2409` (`ir.rs:949+`).

**Resolution patterns:**

| Mechanism | Use |
|-----------|-----|
| `resolve_property(parent, key, default)` | Format-wide inheritable properties (encoding, byteOrder, lengthKind, …) — `963-977`, e.g. `985-1010`, `1083+` |
| `get_property` + optional `parent` | Term-local or explicitly merged props (length, nilValue, calendarPattern, …) — `1068-1070`, `1186-1189`, `1799-1803` |
| `merge_parent` before compile | Copies inheritable parent bindings, skips blocklisted keys — `766-819` |

**Explicit compile-time rejections in this function:**

- `textBidi='yes'`, `floating='yes'` — `1012-1025`
- Empty `nilValue` when nillable literal — `1193-1200`
- `lengthKind='pattern'` without pattern or invalid regex — `1048-1065`
- Discriminator + assert — `1478-1480`
- Many `fillByte` / text number / parseUnparsePolicy rules — `1323-1410`, `1551-1709`, `1886-1910`

**Properties accepted into IR but incomplete downstream:**

- `LengthKind::EndOfParent` — `1041`
- `TextNumberRep::Zoned` — `2258-2260`
- `LengthKind::Expression` — `1038` (kernel gap documented elsewhere)

**Properties stored on `PropertyStore` from XSD but not mapped here:**

- `binaryFloatRep` (grep: zero reads in `properties.rs`; XSD tests set attribute — `dfdl-schema/src/compiler/tests.rs:3236+`)

**Validation split:** Entity/whitespace rules run via `validate_property_entities()` — `881-885`, called from compiler before/ alongside resolution — `lower.rs:767`. `to_resolved_properties` does not call it internally.

**Extensions in resolution:** `dfdlx:` / `daf:` parseUnparsePolicy — `1859-1868`; `rep_type`, layer transforms — `2374-2386`; `string_as_xml` from runtimeProperties — `2388-2398`.

---

## Recommendations

### Documentation

1. Publish an **XSD ingest profile**: UTF-8 input, DTD forbidden, predefined entities only, permissive namespaces during compile.
2. Cross-link **property resolution** vs **lower.rs SDEs** so `endOfParent` / `zoned` / calendar rules are one matrix (this doc + `dfdl-schema-standard-implementation-gaps.md`).
3. Document **`validate_property_entities` must run** before trusting a bare `PropertyStore` (hand-built IR tests).

### `dfdl-xml`

4. Default **strict namespaces** for schema compile, or reject QNames whose URI equals an undeclared prefix when permissive mode was used.
5. **Enforce `max_entity_expansion_bytes`** (and optionally cap decoded attribute/text length) in `decode_entities` / `decode_attribute_value`.
6. Expose **`version` / `standalone`** on `StartDocument` if XSD validation against standalone declarations matters.

### `to_resolved_properties` (§6.3)

7. Map **`binaryFloatRep`** into `ResolvedProperties` and thread to kernel/accessors.
8. **SDE on unknown** `occursCountKind`, `lengthUnits`, `nilKind`, and invalid `alignment` instead of silent defaults — `1166`, `1087`, `1183`, `1107`.
9. Align **`endOfParent`**: reject in property layer or implement in kernel (avoid enum without behavior).
10. Either implement **`floating` / `textBidi`** or keep SDE but document as permanent OGF subset.

---

## Appendix: Representative error strings

| Message | Location |
|---------|----------|
| `Undeclared XML namespace prefix reference` | `reader.rs:395-397` |
| `Undeclared XML attribute namespace prefix reference` | `reader.rs:438-441` |
| `DTD declarations (<!DOCTYPE) are prohibited for security` | `reader.rs:699-701` |
| `Unsupported or custom DTD entity reference prohibited` | `reader.rs:273-276`, `363-366` |
| `Schema Definition Error: textBidi='yes' is not supported` | `properties.rs:1016` |
| `Schema Definition Error: floating='yes' is not supported` | `properties.rs:1024` |
| `dfdl:lengthPattern property missing when lengthKind='pattern'` | `properties.rs:1054-1057` |
| `Schema Definition Error: A component cannot have both a discriminator and an assert statement.` | `properties.rs:1479` |
| `Schema Definition Error: Incompatible parseUnparsePolicy` | `properties.rs:1905-1909` |
| `Invalid lengthKind property` / `Invalid representation property` | `properties.rs:1043-1044`, `990-991` |

---

*Generated from workspace source review. Scope: `crates/dfdl-xml` and `dfdl-core/src/expr/properties.rs` only.*
