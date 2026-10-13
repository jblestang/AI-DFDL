# Code & Architecture Review: Generic DFDL Wireshark Dissector Refactoring

**Date & Time:** 2026-10-14 00:03:00 CEST  
**Component:** `crates/dfdl-wireshark` (`src/dissector.rs`)  
**Scope:** Removal of protocol-specific hardcoded values (ASTERIX Mode 3/A squawk codes 7500/7600/7700, radar flags, and aviation field names) in favor of generic schema-driven metadata evaluation.

---

## 1. Executive Summary

Previously, `crates/dfdl-wireshark/src/dissector.rs` contained domain-specific hardcoded logic:
- Mode 3/A transponder emergency codes (`7500` hijack, `7600` radio failure, `7700` emergency) and alternative decimal encodings (`3904`, `3968`, `4032`).
- Radar target flags (`spi`, `me`, `mil`, `sim`, `dou`, `cdc`, `doubtful`, `tre`, `gho`).
- Hardcoded ASTERIX field names and formatting in `DissectionTree::summary` (`Cat {:03}`, `category`, `callsign`, `acid`, `mode3acode`, `flightlevel`, etc.).

This refactoring removes all hardcoded protocol constants and field names from `dissector.rs`. Anomaly detection and packet summarization are now 100% driven dynamically by DFDL XML Schema (`.dfdl.xsd`) metadata (`<xs:minInclusive>`, `<xs:maxInclusive>`, `<xs:enumeration>`, `<ast:enum>`).

---

## 2. Multi-Domain Expert Review

### 2.1 Software Architecture & Genericity
- **Separation of Concerns:** A DFDL-driven dissector must remain completely agnostic to the underlying binary protocol format. The engine's purpose is to interpret arbitrary DFDL schemas (e.g. ASTERIX, STANAG 4607, Mil-Std-1553, AIS, DNS, TCP/IP) without needing engine-level code updates for new protocols.
- **Dynamic Enumeration Alert Inspection:** In place of hardcoded squawk codes, `DissectionNode::from_element` now queries `meta.get_enum_label(num)` extracted from the schema. If the schema author annotates an enumeration value with keywords such as `"emergency"`, `"hijack"`, `"failure"`, `"alarm"`, `"fatal"`, or `"warn"` / `"alert"` / `"invalid"`, Wireshark Expert Info flags it at the corresponding `AnomalySeverity::Error` or `AnomalySeverity::Warn` level.
- **Generic Summary Generation:** `DissectionTree::summary(&self)` now recursively inspects record structures and selects salient leaf fields based on universal protocol metadata attributes (`id`, `type`, `code`, `status`, `num`, `addr`, `time`, `cat`). No aviation-specific field names remain hardcoded.

### 2.2 Safety & Robustness
- **Bounds Checking:** Out-of-bounds validation (`<xs:minInclusive>` and `<xs:maxInclusive>`) remains fully active and is derived strictly from schema facet definitions.
- **Zero Panic Safety:** All indexing into anomaly lists in test and library code adheres to `-D clippy::indexing-slicing` via `.first()` or `.get()` patterns.
- **Nil/Absent Element Handling:** Safe handling of optional or absent fields without risk of unwrap panics.

### 2.3 Aviation & Radar Domain Compliance
- **Schema-Level Definition:** ASTERIX-specific semantics (such as Mode 3/A code interpretations or radar plot flags) properly belong inside `asterix.dfdl.xsd` via `<xs:annotation><xs:appinfo><ast:enum value="..." label="..."/></xs:appinfo></xs:annotation>`, rather than in the core Wireshark engine.
- **Backward Compatibility:** All existing ASTERIX decodes (`cat_034_048.pcap`) continue to dissect properly with complete physical unit conversions (`NM`, `degrees`, `seconds UTC`, `FL`), bitmasks, and tooltips.

### 2.4 Wireshark Integration & UX
- **Expert Info:** Clean separation of severities (`Chat`, `Note`, `Warn`, `Error`) without generating false alarms for non-aviation schemas that might happen to contain numeric values 7500, 7600, or 7700.
- **Info Column:** Packet summaries format cleanly as `RecordName [fieldName: value, ...]` for multi-record payloads, scaling gracefully to arbitrary protocols.

---

## 3. Verification & Test Matrix

| Test Suite / Tool | Command | Result | Notes |
|:---|:---|:---:|:---|
| **Unit Tests** | `cargo test -p dfdl-wireshark` | **PASS (9/9)** | Tested bitmasks, schema bounds, and schema-driven enum anomalies |
| **Lints** | `cargo clippy -p dfdl-wireshark --all-targets` | **PASS (0 warnings)** | Checked against strict project clippy rules (`-D clippy::indexing-slicing`) |
| **Wireshark Deployment** | `./scripts/deploy_wireshark.sh` | **PASS** | TShark validation on live capture `cat_034_048.pcap` decoded `AsterixPacket` successfully |
