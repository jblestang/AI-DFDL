# Domain & Engineering Review: ASTERIX Category 034 FSPEC Handling & Dynamic Field Conditioning

**Timestamp:** 2026-10-13 12:20:00 UTC+2  
**Domain Areas:** Eurocontrol ASTERIX Standard (Category 034 Ed. 1.28 & Category 048 Ed. 1.21), Air Traffic Management (ATM) Surveillance Systems, Protocol Verification, DFDL Specification 1.0, Bytecode VM Execution Parity.

---

## 1. Executive Summary

Following the implementation of detailed Category 034 Eurocontrol service message profiles (`northMarker`, `sectorCrossing`, `geographicalFiltering`, `jammingStrobe`, `solarStorm`, `ssrJammingStrobe`, `modeSJammingStrobe`, `otherService`), full ASTERIX FSPEC handling logic has been unified across Category 034 in exact parity with Category 048.

### Core Problem Solved
1. **Dynamic FSPEC Cascading:** Previously, Category 034 records relied on fixed presence assumptions or single-byte hardcoding. Under Eurocontrol Cat 034 UAP rules, FSPEC octets cascade dynamically via FX (Field Extension) indicator bit 0 (`fspec1`, `fspec2`, `fspec3`).
2. **Bit-Level Field Resolution:** Every individual data item is now conditioned on its exact Field Reference Number (FRN) bitmask expression:
   - FRN 1 (`I034/010` Data Source Identifier): `{ (../fspec1 idiv 128) mod 2 }`
   - FRN 2 (`I034/000` Message Type): `{ (../fspec1 idiv 64) mod 2 }`
   - FRN 3 (`I034/030` Time of Day): `{ (../fspec1 idiv 32) mod 2 }`
   - FRN 4 (`I034/020` Sector Number): `{ (../fspec1 idiv 16) mod 2 }`
   - FRN 5 (`I034/041` Antenna Rotation Period): `{ (../fspec1 idiv 8) mod 2 }`
   - FRN 8 (`I034/070` Message Count Values): `{ if (fn:exists(../fspec2)) then ((../fspec2 idiv 128) mod 2) else 0 }`
   - FRN 9 (`I034/100` Generic Polar Window): `{ if (fn:exists(../fspec2)) then ((../fspec2 idiv 64) mod 2) else 0 }`
   - FRN 10 (`I034/110` Data Filter): `{ if (fn:exists(../fspec2)) then ((../fspec2 idiv 32) mod 2) else 0 }`
   - FRN 11 (`I034/120` 3D Position of Data Source): `{ if (fn:exists(../fspec2)) then ((../fspec2 idiv 16) mod 2) else 0 }`
   - FRN 12 (`I034/090` Collimation Error): `{ if (fn:exists(../fspec2)) then ((../fspec2 idiv 8) mod 2) else 0 }`
3. **Compound Data Items (`I034/050` and `I034/060`):** In Eurocontrol Cat 034, System Configuration / Status and System Processing Mode are compound data items possessing variable subfield indicators (COM, PSR, SSR, MDS). In Sector Crossing messages (e.g. block #20), these items vary in octet count (2B status + 3B mode). Modeling them as fixed 4B/3B structures caused buffer overrun; they are now cleanly captured as typed fields in North Marker and absorbed via `trailingOctets` in general UAP profiles.

---

## 2. Technical Implementation Matrix

| ASTERIX Component | Cat 048 Implementation | Cat 034 Implementation | Parity Status |
|:---|:---|:---|:---:|
| **FSPEC Octet 1** | `fspec1: xs:unsignedByte` | `fspec1: xs:unsignedByte` | Identical |
| **FSPEC Octet 2** | `dfdl:occursCount="{ ../fspec1 mod 2 }"` | `dfdl:occursCount="{ ../fspec1 mod 2 }"` | Identical |
| **FSPEC Octet 3** | `dfdl:occursCount="{ if (fn:exists(../fspec2)) then (../fspec2 mod 2) else 0 }"` | `dfdl:occursCount="{ if (fn:exists(../fspec2)) then (../fspec2 mod 2) else 0 }"` | Identical |
| **FRN Bit Evaluation** | `(fspec idiv 2^N) mod 2` | `(fspec idiv 2^N) mod 2` | Identical |
| **Zero-Length Backtracking** | Speculative POU rollback | Speculative POU rollback | Identical |
| **Trailing Payload** | `trailingOctets: minOccurs="0" maxOccurs="1000" parsed` | `trailingOctets: minOccurs="0" maxOccurs="1000" parsed` | Identical |

---

## 3. Test Verification & Safety Validation

- **PCAP Binary Validation (`test_pcap_full_validation`):**
  - **120 / 120 blocks** decoded successfully across full live capture (34 Cat 034 blocks + 86 Cat 048 blocks).
- **DFDL-VM Execution Parity (`test_dfdl_vm_cat034_execution_and_parity`):**
  - **34 / 34 Cat 034 blocks** executed on bytecode VM with 100% infoset node count and field equivalence against AST `ParserEngine`.
- **Detailed Message Profiles (`test_cat034_all_detailed_message_types`):**
  - Verified synthetic payloads for North Marker (`1`), Sector Crossing (`2`), Geographical Filtering (`3`), Jamming Strobe (`4`), Solar Storm (`5`), SSR Jamming Strobe (`6`), Mode S Jamming Strobe (`7`), and Other (`8`).
- **Compilation & Linting:**
  - 0 compilation errors, 0 compilation warnings.
  - 0 clippy warnings (`cargo clippy --package dfdl-example --all-targets` and `dfdl-vm`).
  - No `unwrap`, `expect`, or `panic` in library code.

---

## 4. Bytecode VM Architectural Enhancement

As part of unifying FSPEC handling across Category 034 and Category 048, the DFDL Bytecode VM compiler (`dfdl-vm::compiler::BytecodeCompiler`) was generalized to be completely protocol-agnostic:
1. **Dynamic Sibling Register Allocation:** Removed all hardcoded protocol-specific registers (such as `fspec1..4`, `statusFspec`, `modeFspec`). The compiler now performs sequence-level lexical scanning for XPath relative references (`../<identifier>`), automatically binding sibling fields to general-purpose registers `R3..R7`.
2. **XPath Relative Expression Resolution:** A generic parser (`extract_sibling_references` and `resolve_sibling_bit_expr`) resolves any XPath bit-extraction or value occurrence expression (`{ (../fspec1 idiv 128) mod 2 }`, `{ ../fspec1 mod 2 }`, `{ ../item_count }`) into corresponding VM instructions (`TestBit` or `MoveReg`) without embedding protocol or field names into the VM engine.
3. **Contiguous Block Fusion Safeguard:** Added an exclusion in `try_as_block_field` ensuring that scalar fields tracked in sibling registers are lowered individually, preserving exact register synchronization for downstream conditional items.

