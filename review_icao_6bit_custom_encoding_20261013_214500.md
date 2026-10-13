# Domain & Technical Review: ICAO 6-Bit Custom Character Encoding in DFDL Engine

**Date:** 2026-10-13 21:45:00 UTC  
**Scope:** Elimination of ad-hoc `try_decode_icao_6bit` in `dfdl-wireshark` and integration of standard `X-DFDL-ICAO-6BIT` sub-byte character encoding in `dfdl-core`.  
**Reviewers:** Domain Experts (Air Traffic Management / Radar Surveillance / ASTERIX, DFDL Standards Architecture, Wireshark Protocol Engineering, Safety & Verification).

---

## 1. Domain Expertise Analysis (Aviation, Radar & ASTERIX)

### 1.1 Specification Compliance (ICAO Annex 10 & Eurocontrol ASTERIX Cat 048 Item 240)
- **Standard:** ICAO Annex 10, Volume IV, §3.1.2.9 and Eurocontrol ASTERIX Part 4 (Category 048) Data Item `I048/240` define Aircraft Identification as 8 characters coded on 6 bits each (total 48 bits = 6 octets).
- **Code Mapping:**
  - Codes `1..=26` (`0x01..0x1A`) map to characters `'A'..='Z'`.
  - Codes `48..=57` (`0x30..0x39`) map to numeric characters `'0'..='9'`.
  - Codes `32` (`0x20`) and `0` (`0x00`) represent space padding `' '`.
- **Finding:** The native DFDL custom sub-byte decoder accurately parses callsign bits into Unicode strings directly at the parser engine layer without requiring external transformations.

---

## 2. DFDL Standards & Architectural Review

### 2.1 First-Class Sub-Byte Encoding
- **DFDL v1.0 §12 (Character Set Encoding):** Custom encodings prefixed with `X-DFDL-` (such as `X-DFDL-ICAO-6BIT`) are the standard DFDL mechanism for non-octet-aligned and domain-specific character sets.
- **Unit Width & Alignment:**
  - `encoding_unit_bits("X-DFDL-ICAO-6BIT") = 1`
  - `encoding_char_bits("X-DFDL-ICAO-6BIT") = 6`
  - `encoding_mandatory_alignment_bits("X-DFDL-ICAO-6BIT") = 1`
- **Length Units Handling:**
  - In `asterix.dfdl.xsd`, `aircraftId` is defined with `dfdl:lengthKind="explicit" dfdl:length="8" dfdl:lengthUnits="characters" dfdl:encoding="X-DFDL-ICAO-6BIT"`.
  - The binary string parser computes `num_chars = 8`, consuming exactly `8 * 6 = 48` bits (6 bytes).
  - Unparser support is automatically available via `unparse_sub_byte_text` and `strict_sub_byte_code`.

---

## 3. Wireshark Dissector & Generic Architecture Review

### 3.1 Total Elimination of Ad-Hoc Logic
- **Elimination:** `try_decode_icao_6bit` and intermediate raw byte conversion vector have been completely removed from [converter.rs](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/converter.rs).
- **Generic Engine:** `dfdl-wireshark` now receives the already-decoded string (e.g. `"DLH65A  "`) from `dfdl-core`.
- **UI Tree Formatting:** Clean field display `- aircraftId: "DLH65A"` with full schema metadata (Item, description) housed in hover tooltips.

---

## 4. Verification & Safety Criteria

1. **Unit Tests:**
   - Added round-trip codec unit tests `test_icao_6bit_roundtrip` in `crates/dfdl-core/src/encoding.rs`.
   - Verified that all 295 core tests and 3 Wireshark tests pass (`0 failures`).
2. **Clippy & Compiler Warnings:**
   - Workspace compiled under strict clippy settings (`unwrap_used = "deny"`, `arithmetic_side_effects = "deny"`).
   - Zero warnings across all targets (`cargo clippy --package dfdl-core --package dfdl-wireshark --all-targets`).
3. **PCAP End-to-End Validation:**
   - Validated on `crates/dfdl-example/data/cat_034_048.pcap` packet 1: `aircraftId: "DLH65A"` verified in `tshark` and `dfdl-wireshark decode`.
