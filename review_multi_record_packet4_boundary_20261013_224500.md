# Domain & Architecture Code Review: ASTERIX Multi-Record Packet 4 Boundary Resolution
**Date:** 2026-10-13 22:45:00 UTC
**Review Scope:** Eurocontrol ASTERIX multi-record UDP packet boundary enforcement, DFDL explicit length lookahead scoping, Wireshark tree dissection parity.

---

## 1. Context & Identified Defect
- **Observation:** In `cat_034_048.pcap`, Packet 4 (66 bytes total) was dissected by Wireshark DFDL as a single Category 048 packet containing 3 records.
- **Protocol Ground Truth (Eurocontrol ASTERIX Part 1 §3.1 & Part 4):**
  - Bytes 0..55: Category 48 Data Block (`category: 48`, `length: 55`), carrying 1 target report of 52 bytes (`callsign: THY9TX`).
  - Bytes 55..66: Category 34 Data Block (`category: 34`, `length: 11`), carrying 1 Sector Crossing Message (`messageType: 2`, `sectorNumber: 135°`).
- **Root Cause:**
  - In `crates/dfdl-example/schemas/asterix.dfdl.xsd`, `Cat048RecordType` had `record` configured with `dfdl:occursCountKind="parsed"`, but `AsterixRecord` and `Cat048Record` lacked explicit length bounds.
  - After parsing the authentic 52-byte target report, the occurrence loop continued into the next 11 bytes of Cat 034 data, misinterpreting `0x22` as `fspec1` and synthesizing two spurious records.
  - In `crates/dfdl-core/src/kernel/parser/element.rs`, `evaluate_length_property` lacked `.with_lookahead(...)` on `ExprContext`, which previously prevented runtime lookahead expressions such as `{ dfdlx:lookAhead(8, 16) }` during length property evaluation.

---

## 2. Engineering Solution Applied

### 2.1 Core DFDL Engine (`crates/dfdl-core`)
- Attached lookahead capability via `RefCell` bit lookahead callback in `evaluate_length_property` (`crates/dfdl-core/src/kernel/parser/element.rs`), allowing dynamic lookahead inspection of length headers.
- Bound complex element parsing to `bit_limit` computed from explicit length expressions. When the explicit length is reached, `reader.is_eof()` evaluates to `true`, naturally terminating unbounded `occursCountKind="parsed"` occurrence loops.

### 2.2 ASTERIX Schema (`crates/dfdl-example/schemas/asterix.dfdl.xsd`)
- Specified `dfdl:lengthKind="explicit"` `dfdl:length="{ dfdlx:lookAhead(8, 16) }"` `dfdl:lengthUnits="bytes"` on:
  - `AsterixRecord`: Bounds each polymorphic data block according to its 16-bit length indicator at byte offset 1 (bits 8..24).
  - Category root elements (`Cat048Record`, `Cat034Record`, `Cat001Record`, `Cat002Record`): Guarantees correct bounds whether parsed through `AsterixPacket` or directly as standalone roots.

---

## 3. Verification & Compliance Matrix

| Target | Test / Tool | Verification Result |
| :--- | :--- | :--- |
| **`dfdl-core`** | 295 unit tests | **PASS** (100%) |
| **`dfdl-example`** | 7 unit tests + 8 integration tests | **PASS** (100%, includes 120 authentic PCAP blocks) |
| **`dfdl-wireshark`** | 4 unit tests + CLI decode | **PASS** (100%) |
| **Packet #4 TShark** | `tshark -r cat_034_048.pcap -Y "frame.number == 4" -V` | **PASS** (Dissects Cat 048 [55B, 1 report] + Cat 034 [11B, Sector Crossing]) |
| **Clippy Linting** | `cargo clippy --all-targets -- -D warnings` | **PASS** (Zero warnings, clean workspace) |
| **Graft Knowledge** | `graft build` | **PASS** (1912 nodes, 3794 edges updated) |

---

## 4. Safety & Maintainability Remarks
1. **Generic Design:** Zero hardcoded category offsets or packet lengths. The solution relies purely on the official DFDL v1.0 standard and Eurocontrol ASTERIX length header specification.
2. **Backwards Compatibility:** All standalone category roots and multi-datagram packet parsers continue to function identically.
3. **Deployment:** Redeployed native dylib and Lua dissector into `/Users/jean-baptiste/.local/lib/wireshark/plugins`.
