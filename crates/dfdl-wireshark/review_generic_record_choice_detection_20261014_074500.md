# Multi-Domain Expert Review: Elimination of Hardcoded Record Identifiers

**Date:** 2026-10-14 07:45:00 CEST  
**Artifact:** [`review_generic_record_choice_detection_20261014_074500.md`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/review_generic_record_choice_detection_20261014_074500.md)  
**Scope:** Elimination of `"AsterixRecord"`, `"Record"`, and `"RecordType"` heuristic string matching; implementation of pure XML Schema structural choice detection in [`schema_meta.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/schema_meta.rs).

---

## 1. Context & Motivation

During the initial implementation of hierarchical display filtering (`dfdl.<Record>.<Container>.<Field>`), heuristic string checks were used to detect top-level message variant boundaries:
```rust
// Previous heuristic approach (NON-GENERIC):
let is_inside_record = decl_stack
    .iter()
    .any(|d| d.name.ends_with("Record") && d.name != "AsterixRecord" || d.name.ends_with("RecordType"));
```

This violated the core architectural requirement that the DFDL Wireshark dissector must remain 100% format-agnostic. Protocols such as STANAG 5066, Link 16, CCSDS, or custom radar telemetry do not use names ending in `"Record"` or `"RecordType"` (e.g. `Heartbeat`, `TelemetryPacket`, `CommandFrame`, `TrackMessage`).

---

## 2. Generic Structural Solution

Rather than inspecting element or type names, message variant roots are now identified strictly through XML Schema / DFDL grammar semantics:
```rust
// Purely structural detection (100% GENERIC):
let is_top_choice_branch = choice_depth > 0
    && decl_stack.len() <= 2
    && decl_stack.first().map(|d| d.tag.as_str()) == Some("element");
```

### Structural Rationale:
1. **Polymorphic Protocols:** In DFDL schemas describing multi-message streams (e.g., ASTERIX data blocks, telemetry multiplexers), the global root element (`decl_stack.len() == 1`, tag `"element"`) contains an `<xs:choice>` representing the message dispatcher.
2. **Top-Level Choice Branches:** Any direct element declared or referenced inside that top-level choice (`choice_depth > 0 && decl_stack.len() <= 2`) is structurally a top-level message/record variant (e.g. `Cat048Record`, `Cat034Record`, `HeartbeatFrame`, `StatusMessage`).
3. **Inner Choices Isolated:** Choices nested within complex types or lower-level structures (`decl_stack.first().map(|d| d.tag.as_str()) == Some("complexType")` or `decl_stack.len() > 2`) are internal structural variations (such as variable-length extensions or sub-item alternatives) and are correctly treated as standard hierarchical components rather than top-level records.
4. **Monomorphic Schemas:** For schemas with no `<xs:choice>` dispatcher, the fallback to `registry.root_element` ensures single-record formats still generate complete hierarchical field paths (e.g. `dfdl.RadarReport.header.seq`).

---

## 3. Multi-Domain Verification

### 3.1 Software Engineering & Architecture
- **Zero String Pattern Dependencies:** Grep verification across `crates/dfdl-wireshark/src/` confirms 0 occurrences of `"AsterixRecord"`, `"Record"`, or `"RecordType"` string matching logic.
- **Clippy Cleanliness:** Passes `cargo clippy -p dfdl-wireshark --all-targets -- -D warnings` with zero warnings or lints.
- **Arithmetic Safety:** All stack depth calculations use saturating increments (`saturating_add`, `saturating_sub`).

### 3.2 Radar & Surveillance Domain
- **Hierarchical Path Integrity:**
  - `dfdl.Cat048Record.record.targetDescriptor.typ`
  - `dfdl.Cat048Record.record.rho`
  - `dfdl.Cat034Record.sectorCrossing.sectorNumber`
- **Container Filters:** `dfdl.Cat048Record` and `dfdl.Cat034Record` match target reports and service messages respectively without ambiguity.

### 3.3 Safety & Security (DO-178C / MISRA / Cyber)
- **Zero Panic Guarantee:** Stack transitions are symmetric and bounded (`decl_stack.pop()` strictly tied to opening tags; `choice_depth` checked against 0 before decrementing).
- **Infinite Recursion Protection:** `expand_hierarchy` enforces a depth limit of 32 and maintains a visited-path set to reject circular type references.

---

## 4. Test & Deployment Verification

1. **Unit Tests:** 10/10 passed (`cargo test -p dfdl-wireshark`).
2. **Wireshark Plugin Deployment:** `./scripts/deploy_wireshark.sh` completed with 0 errors.
3. **Live TShark Filters:**
   - `tshark -Y "dfdl.Cat048Record.record.targetDescriptor.typ == 5" -r crates/dfdl-example/data/cat_034_048.pcap`: **PASS**
   - `tshark -Y "dfdl.Cat034Record.sectorCrossing.sectorNumber" -r crates/dfdl-example/data/cat_034_048.pcap`: **PASS**
   - `tshark -Y "dfdl.Cat048Record" -r crates/dfdl-example/data/cat_034_048.pcap`: **PASS**
   - `tshark -Y "dfdl.typ == 5" -r crates/dfdl-example/data/cat_034_048.pcap`: **PASS** (flat fallback alias)
