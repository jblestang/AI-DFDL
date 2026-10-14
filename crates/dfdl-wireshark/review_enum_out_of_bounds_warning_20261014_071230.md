# Multi-Domain Review: Out-of-Bounds Enum Validation & Warning System

**Date & Time**: 2026-10-14 07:12:30 UTC+2  
**Component**: `crates/dfdl-wireshark` (`dissector.rs`, `converter.rs`, `lib.rs`)  
**Scope**: Schema-driven validation of enumeration values, Wireshark Expert Info warning generation, and tree node rendering for unmapped values.

---

## 1. Domain Expertise Analysis

### 1.1 UX / UI Engineering (Wireshark Qt & Expert Information Tree)
- **Visual Status Hierarchy in Wireshark**:
  - Wireshark users rely heavily on colored visual cues in the packet list and packet details tree. Assigning an `AnomalySeverity::Warn` level attaches a yellow badge to the specific tree item and registers the anomaly under `Analyze -> Expert Information -> Warnings`.
  - When an unexpected enumeration value is received:
    - **Previous Behavior**: The field fell back to displaying the raw integer without context, with zero warning or indication that the value violated the schema's defined enumeration list.
    - **New Behavior**:
      1. Display value is formatted clearly as `Unknown (<value>)` (e.g., `Target Report Type (typ): Unknown (9)`).
      2. An anomaly of severity `Warn` is attached to the item: `[Expert Info (Warning/Protocol): Field 'typ' value 9 is outside expected enum values]`.
      3. Hover tooltips continue to show the full list of valid schema enum values under `Values: [0=..., 1=...]` so the user can immediately compare the invalid value against allowed values.

### 1.2 Radar & Avionics Domain (ASTERIX & Protocol Specifications)
- **Protocol Integrity & Reserved Codes**:
  - Surveillance standards (Eurocontrol ASTERIX Categories 001, 002, 034, 048, 062, etc.) allocate fixed-width bitfields for status and type indicators (e.g., `TYP` 3 bits [0..7], `SIM` 1 bit [0..1], `TRE` 1 bit [0..1], `COM` 3 bits [0..5]).
  - Bit patterns that are not assigned in the specification (or marked as spare/reserved) represent potential sensor firmware bugs, corrupted transmission frames, or out-of-spec radar extractor outputs.
  - Generating a non-intrusive `Warning` (rather than corrupting the dissection or marking the entire packet frame unparsable) allows air traffic and radar engineers to inspect the remaining payload while immediately flagging the protocol anomaly.

### 1.3 Software Development & Engineering (Rust & DFDL Architecture)
- **Surgical, Resilient Implementation**:
  - **Numeric & Hex Parsing**: Integer parsing supports both decimal strings and hexadecimal values (`0x...` / `0X...`) when querying `meta.enum_map`.
  - **Separation of Concerns**:
    - [`converter.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/converter.rs) handles textual presentation (`Unknown (val)`).
    - [`dissector.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/dissector.rs) handles anomaly detection and Wireshark Expert Info generation.
  - **Zero-Warning & Strict Safety**:
    - Complies with strict `#![deny(clippy::indexing_slicing)]`, `#![deny(clippy::panic)]`, `#![deny(clippy::unwrap_used)]`.
    - Unit tests in `dissector::tests` and `lib::tests` verify nominal values, status warnings/emergencies, and out-of-bounds enum detection.
    - `cargo clippy -p dfdl-wireshark --all-targets` passes with 0 warnings.

### 1.4 Safety & Cyber Security
- **Defense in Depth**:
  - Validating all field values against schema-declared enumeration bounds prevents malformed or spoofed packets from quietly masquerading as valid data.
  - Automated anomaly logging provides immediate situational awareness for cyber security analysts inspecting suspicious network telemetry.

---

## 2. Verification Summary

| Check | Tool / Command | Result |
|---|---|---|
| Unit & Integration Tests | `cargo test -p dfdl-wireshark --lib` | 9 passed, 0 failed |
| Strict Static Linting | `cargo clippy -p dfdl-wireshark --all-targets` | 0 warnings, Clean |
| Sample PCAP Dissection | `tshark -r cat_034_048.pcap -V` | Successful packet decoding & tree generation |
| Plugin Deployment | `./scripts/deploy_wireshark.sh` | Deployed to `~/.local/lib/wireshark/plugins` |
