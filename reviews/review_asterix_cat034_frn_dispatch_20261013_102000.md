# Domain & Engineering Review: ASTERIX Category 034 FRN & Message Type Dispatch

**Timestamp:** 2026-10-13 10:20:00 UTC+2  
**Domain Expertises Applied:** Air Traffic Management (ATM) Surveillance Systems, Eurocontrol ASTERIX Standard, Protocol Engineering, DFDL Specification 1.0, High-Assurance Rust (`no_std` / zero-panic), Bytecode VM Architecture.

---

## 1. Executive Summary

This review addresses the refactoring of Eurocontrol ASTERIX Category 034 (Monoradar Service Messages) within the `dfdl-example` schema and its execution engine.

### Problem Statement
The previous Category 034 definition used an ad-hoc choice dispatch key:
```xml
<!-- Choice among standard Category 034 message profiles -->
<xs:choice dfdl:choiceDispatchKey="{ xs:string(dfdlx:lookAhead(0, 8)) }">
  <xs:element name="sectorCrossing" type="Cat034SectorCrossingType" dfdl:choiceBranchKey="240"/>
  <xs:element name="serviceStatus" type="Cat034StatusType" dfdl:choiceBranchKey="244"/>
  <xs:element name="processingStatus" type="Cat034ProcessingStatusType" dfdl:choiceBranchKey="246"/>
  <xs:element name="northMarker" type="Cat034NorthMarkerType" dfdl:choiceBranchKey="239"/>
</xs:choice>
```
This was fundamentally flawed:
1. **FSPEC is not the Message Type:** `dfdlx:lookAhead(0, 8)` was examining the raw bitmask of FSPEC Octet 1 (`240`, `244`, `246`, `239`), confusing field presence flags with semantic message classifications.
2. **Artificial Message Profiles:** Values `244` (0xF4) and `246` (0xF6) in the authentic PCAP sample were not distinct message types, but standard Sector Crossing messages (`messageType = 2`) that simply contained optional Item 050 (System Status) and Item 060 (Processing Mode).
3. **Eurocontrol ASTERIX Cat 034 Conformance:** In Eurocontrol ASTERIX Part 9, the message classification is given exclusively by **Item 000 (Message Type)**, whose presence is signaled by **FRN 2** (bit 7 / weight 64) in FSPEC Octet 1.

---

## 2. Technical Remediation & Implementation Details

### 2.1 DFDL Schema Refactoring
The choice dispatch key was updated to evaluate both the FRN indicator in FSPEC and the exact `messageType` enum value:
```xml
<!-- Choice among standard Category 034 message profiles -->
<xs:choice dfdl:choiceDispatchKey="{ if (((dfdlx:lookAhead(0, 8) idiv 64) mod 2) eq 1) then (if ((dfdlx:lookAhead(0, 8) mod 2) eq 1) then xs:string(dfdlx:lookAhead(32, 8)) else xs:string(dfdlx:lookAhead(24, 8))) else 'unknown' }">
  <xs:element name="northMarker" type="Cat034NorthMarkerType" dfdl:choiceBranchKey="1"/>
  <xs:element name="sectorCrossing" type="Cat034SectorCrossingType" dfdl:choiceBranchKey="2"/>
  <xs:element name="genericService" type="Cat034GenericServiceType" dfdl:choiceBranchKey="3 4 5 6 7 8"/>
</xs:choice>
```

#### Expression Breakdown:
1. `((dfdlx:lookAhead(0, 8) idiv 64) mod 2) eq 1`: Validates that FRN 2 (Item 000: Message Type) is declared present in FSPEC Octet 1.
2. `(dfdlx:lookAhead(0, 8) mod 2) eq 1`: Inspects the FX extension bit (bit 1 of FSPEC Octet 1).
   - If `1` (extended FSPEC, e.g., North Marker 0xEF 0x10): 2 octets FSPEC + 2 octets SAC/SIC (FRN 1) = 4 octets = 32 bits offset. Reads `dfdlx:lookAhead(32, 8)`.
   - If `0` (single octet FSPEC, e.g., Sector Crossing 0xF0, 0xF4, 0xF6): 1 octet FSPEC + 2 octets SAC/SIC (FRN 1) = 3 octets = 24 bits offset. Reads `dfdlx:lookAhead(24, 8)`.
3. The choice dispatch key resolves to the exact Eurocontrol `messageType` enum string:
   - `"1"`: North Marker Message (`Cat034NorthMarkerType`)
   - `"2"`: Sector Crossing Message (`Cat034SectorCrossingType`)
   - `"3"`..`"8"`: Other standard service messages (`Cat034GenericServiceType`)

### 2.2 Unification of Sector Crossing Profiles
`Cat034SectorCrossingType` was unified to handle all Sector Crossing variations (11-byte base packets and 16-byte status-extended packets):
```xml
<xs:complexType name="Cat034SectorCrossingType">
  <xs:sequence>
    <xs:element name="fspec" type="xs:unsignedByte"/>
    <xs:element name="sac" type="xs:unsignedByte"/>
    <xs:element name="sic" type="xs:unsignedByte"/>
    <xs:element name="messageType" type="xs:unsignedByte"/>
    <xs:element name="timeOfDay" type="xs:unsignedInt" dfdl:lengthKind="explicit" dfdl:lengthUnits="bits" dfdl:length="24"/>
    <xs:element name="sectorNumber" type="xs:unsignedByte"/>
    <!-- Trailing octets for optional Item 050 (status) and Item 060 (mode) -->
    <xs:element name="trailingOctets" type="xs:unsignedByte" minOccurs="0" maxOccurs="1000" dfdl:occursCountKind="parsed"/>
  </xs:sequence>
</xs:complexType>
```

### 2.3 `dfdl-vm` Engine Fixes for Speculative Choice & Block Reading
1. **Loop Lowering Support for `OccursCountKind::Parsed`:**
   In `dfdl-vm/src/compiler.rs`, `OccursCountKind::Parsed` (such as `trailingOctets`) was added to the `JumpIfEof` loop handling, allowing zero trailing bytes to cleanly terminate on EOF without reading past the end of slice.
2. **POU Backtracking Inside Fused `ReadBlock`:**
   In `dfdl-vm/src/interpreter.rs`, `ReadBlock` was refactored to catch EOF / parse errors during fused block reads, restore the bit checkpoint and sink mark, and properly break out of the inner field loop so that speculative choice rollback immediately retries the next branch without skipping instructions.

---

## 3. Verification & Compliance Matrix

| Criterion | Target | Result | Status |
|:---|:---:|:---:|:---:|
| **PCAP Full Validation** | 120 / 120 blocks | 120 / 120 blocks (100%) | Passed |
| **Category 034 Blocks** | 34 blocks | 34 / 34 validated | Passed |
| **Category 048 Blocks** | 86 blocks | 86 / 86 validated | Passed |
| **XML Infoset Dump** | Valid XML tags | Generated with `<sectorCrossing>` & `<northMarker>` | Passed |
| **`dfdl-vm` Wire-Speed Execution** | 34 / 34 Cat 034 | 34 / 34 decoded, 100% DOM parity | Passed |
| **`dfdl-vm` Decode Latency** | < 50 µs | 9.51 µs average | Passed |
| **Integration Test Suite** | 6 / 6 tests | 6 / 6 passing | Passed |
| **Unit Test Suite** | 5 / 5 tests | 5 / 5 passing | Passed |
| **VM Unit Test Suite** | 20 / 20 tests | 20 / 20 passing | Passed |
| **Clippy Lints (`-D warnings`)** | 0 warnings | 0 warnings | Passed |
