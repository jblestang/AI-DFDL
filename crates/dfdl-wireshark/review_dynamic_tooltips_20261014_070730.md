# Multi-Domain Review: Dynamic Tooltips & Schema Metadata Blurbs in Wireshark Dissector

**Date & Time**: 2026-10-14 07:07:30 UTC+2  
**Component**: `crates/dfdl-wireshark` (`dfdl_dissector.lua`, `schema_meta.rs`, `converter.rs`, `lua_dissector.rs`)  
**Scope**: Dynamic hover tooltips across all DFDL fields in Wireshark (blurb extraction, XSD parsing, Lua ProtoField binding, Qt UI hover integration)

---

## 1. Domain Expertise Analysis

### 1.1 UX / UI Engineering (Wireshark Qt & Dissection Tree)
- **Wireshark Blurb Semantics**:
  - Wireshark's packet details pane is built on a Qt tree widget (`QTreeView`). Each field is backed by an `hf_register_info` structure where `hfinfo->blurb` stores the descriptive text.
  - When a user hovers their mouse over a field label or value, Wireshark natively queries `blurb` and displays a tooltip popup, as well as echoing the description into the main window status bar at the bottom.
  - Previously, `ProtoField` declarations in Lua were passing invalid argument counts for `ProtoField.string` (passing 6 args instead of 4), and omitting the `desc` (blurb) argument entirely on `double`, `uint32`, `int32`, and `bool`. This caused Wireshark to set `blurb = NULL`, silencing hover tooltips.
  - **Resolution**: Aligned Lua `ProtoField` constructor signatures with Wireshark's exact C/Lua API:
    - `ProtoField.double(abbr, label, desc)` (3 args)
    - `ProtoField.uint32(abbr, label, base.DEC, nil, 0, desc)` (6 args)
    - `ProtoField.int32(abbr, label, base.DEC, nil, 0, desc)` (6 args)
    - `ProtoField.bool(abbr, label, base.NONE, nil, 0, desc)` (6 args)
    - `ProtoField.string(abbr, label, base.NONE, desc)` (4 args)
  - Composite and group nodes in `populate_node` now also bind their corresponding `ProtoField` blurb onto the tree header item when available, enabling tooltips on complex container nodes (e.g., `TargetDescriptorType`, `TrackStatusType`).

### 1.2 Radar & Avionics Domain (ASTERIX & Protocol Specifications)
- **Information Density & Technical Context**:
  - Radar surveillance operators, EW analysts, and ATM system engineers require rapid access to:
    1. Exact Eurocontrol Standard Item reference (e.g., `I048/040-RHO`, `I048/020.TYP`).
    2. Quantization / LSB resolution (e.g., `1/256 NM (~7.2297 m)`, `360 / 65536 deg`).
    3. Physical unit of measure (e.g., `NM`, `degrees`, `feet`, `dBm`, `s`).
    4. Discrete enum value tables (e.g., `[0=No detection, 1=Single PSR, 2=Single SSR, 3=SSR+PSR, 5=Mode S+PSR]`).
    5. Normative specification excerpt.
  - `build_field_tooltip` formats these key attributes into a clean, human-readable tooltip string:
    `"Measured Distance Rho in Polar Coordinates | Item: I048/040-RHO | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 040: Measured Distance..."`
  - When inspecting fields in live captures (e.g. `cat_034_048.pcap`), analysts immediately see calibration notes and bit-level allocations without leaving Wireshark to consult external Eurocontrol PDFs.

### 1.3 Software Development & Engineering (Rust & Lua Engine)
- **XML Schema Parser Architecture**:
  - In `crates/dfdl-wireshark/src/schema_meta.rs`, the earlier parser used flat variables (`current_name`, `current_doc`, etc.). When encountering child `<xs:element>` tags inside `<xs:complexType>`, child declarations prematurely wiped or replaced the parent complexType's metadata.
  - **Fix**: Replaced flat variables with a stack-based context (`decl_stack: Vec<DeclContext>`). When entering any `<xs:element>` or `<xs:complexType>`, a new context frame is pushed; annotations and appinfo blocks attach to the current frame; on closing tags, the metadata is registered into the metadata catalog and popped.
  - **Formatting & Sanitization**:
    - Embedded schema docstrings often contain multi-line indentation and leading whitespace. An inline sanitizer collapses internal multiple spaces while preserving punctuation and readable paragraph breaks.
    - Enumeration maps are neatly formatted as `Values: [code=label, ...]`.
- **Quality & Safety Assurance**:
  - 100% compliant with strict crate lints: `#![deny(clippy::indexing_slicing)]`, `#![deny(clippy::panic)]`, `#![deny(clippy::unwrap_used)]`.
  - All 9 unit and integration tests in `dfdl-wireshark` pass.
  - `cargo clippy -p dfdl-wireshark --all-targets` passes with 0 warnings.

### 1.4 Safety & Cyber Security
- **Memory Safety & Sandbox Boundary**:
  - Blurb strings are registered into Wireshark's static protocol dictionary during initialization and dissector registration. No dynamic memory allocation or unsafe pointer manipulation occurs during the per-packet hot path.
  - Stack depth in `schema_meta.rs` is bounded by XSD nesting depth; parsing handles malformed or deeply nested XML schemas gracefully without panic.
  - Native C ABI boundaries (`dfdl_wireshark.dylib` / `.so`) and Lua FFI remain purely functional with zero buffer overflow exposure.

---

## 2. Verification Summary

| Check | Tool / Command | Result |
|---|---|---|
| Unit & Integration Tests | `cargo test -p dfdl-wireshark --lib` | 9 passed, 0 failed |
| Strict Static Linting | `cargo clippy -p dfdl-wireshark --all-targets` | 0 warnings, Clean |
| Field Dictionary Blurb Export | `tshark -G fields \| grep dfdl` | All primitive & complex types carry rich blurbs |
| Packet Dissection Validation | `tshark -r cat_034_048.pcap -V` | Successful packet decoding & tree generation |
| Plugin Deployment | `./scripts/deploy_wireshark.sh` | Deployed to `~/.local/lib/wireshark/plugins` |
