# Domain & Architecture Code Review: Dynamic XSD Field Retrieval in Wireshark Lua Dissector
**Date:** 2026-10-13 23:05:00 UTC
**Review Scope:** Elimination of hardcoded schema fields in `dfdl_dissector.lua`; dynamic retrieval of element annotations, titles, units, and tooltips from the loaded DFDL XSD schema.

---

## 1. Context & Architectural Observation
- **Observation:** `dfdl_dissector.lua` previously contained hundreds of lines of static `ProtoField` definitions baked in at script generation time (e.g. `["AircraftIdType"] = ProtoField.string(...)`, `["trackNumber"] = ...`).
- **Defect / Limitation:** 
  1. The Lua dissector was tightly coupled to whatever schema was used at generation time rather than being generic.
  2. If a user switched the active DFDL schema in Wireshark preferences (`proto.prefs.schema_path`), the dissector retained the old hardcoded fields.
  3. Violates the architectural requirement that the Wireshark plugin be 100% schema-agnostic and retrieve all protocol metadata dynamically from the loaded XSD.

---

## 2. Engineering Solution Applied

### 2.1 Dynamic Schema Metadata Extraction (`dfdl-wireshark`)
- Added `SchemaMetadataRegistry::to_fields_json(&self) -> String` in `crates/dfdl-wireshark/src/schema_meta.rs`:
  - Dynamically formats each schema element's name, display label, description, unit, and comprehensive tooltip into a clean JSON array.
- Added `schema-fields` subcommand to the CLI (`crates/dfdl-wireshark/src/main.rs`):
  - `dfdl-wireshark schema-fields --schema <path>` outputs the JSON array directly for external tools and Lua scripts.
- Added `dfdl_get_schema_fields` C-ABI function to the native FFI library (`crates/dfdl-wireshark/src/ffi.rs`).

### 2.2 100% Schema-Agnostic Lua Dissector (`dfdl_dissector.lua`)
- Completely removed all static `proto_fields = { ... }` code from `generate_lua_dissector` (`crates/dfdl-wireshark/src/lua_dissector.rs`).
- Replaced it with runtime introspection:
  1. At script load time, the Lua dissector retrieves the active schema path (`get_effective_schema()`).
  2. It queries `dfdl_get_schema_fields` via native FFI, or executes `dfdl-wireshark schema-fields --schema <path>` via `io.popen`.
  3. Includes a fast pure-Lua XSD regex scanner as an additional fallback if neither native FFI nor CLI binary is accessible.
  4. Dynamically registers all returned fields into `proto.fields`.
- If the schema is switched at runtime, `populate_node` seamlessly renders any newly introduced fields using `tree_item:add(proto, tvb(), label)` where `label` carries the full descriptive title, unit, and value returned from the DFDL parser engine.

---

## 3. Verification & Compliance Matrix

| Target | Test / Verification | Result |
| :--- | :--- | :--- |
| **`dfdl_dissector.lua` inspection** | Inspect Lua script source code | **PASS** (Zero hardcoded schema fields; 100% generic) |
| **`dfdl-wireshark` unit tests** | `cargo test --package dfdl-wireshark --lib` | **PASS** (4 passed, 0 failed) |
| **`schema-fields` CLI** | `dfdl-wireshark schema-fields --schema asterix.dfdl.xsd` | **PASS** (Outputs valid JSON array of fields with tooltips) |
| **Wireshark / TShark dissection** | `tshark -r cat_034_048.pcap -Y "frame.number == 4" -O dfdl` | **PASS** (Dissects Cat 048 and Cat 034 with complete dynamic fields) |
| **Clippy Linting** | `cargo clippy --package dfdl-wireshark --all-targets -- -D warnings` | **PASS** (Zero warnings, clean build) |
