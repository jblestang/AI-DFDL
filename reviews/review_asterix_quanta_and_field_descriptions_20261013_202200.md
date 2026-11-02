# Review: ASTERIX Schema Quanta and Field Descriptions Integration
**Date:** 2026-10-13 20:22:00 UTC+2  
**Scope:** `crates/dfdl-example/schemas/asterix.dfdl.xsd`, `crates/dfdl-example/src/lib.rs`  
**Domain:** Radar & Surveillance Processing (Eurocontrol ASTERIX Standard Categories 034, 048, 001, 002)

---

## 1. Executive Summary

This review documents the addition of comprehensive field descriptions, engineering measurement units, resolutions, and exact quanta (LSB) to all data items and numerical types in the Eurocontrol ASTERIX DFDL schema [`asterix.dfdl.xsd`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd).

The definitions adhere strictly to official Eurocontrol ASTERIX specifications:
- **Category 034 (Ed. 1.28):** Transmission of Monoradar Service Messages
- **Category 048 (Ed. 1.21):** Transmission of Monoradar Target Reports
- **Category 001 (Ed. 1.2):** Transmission of Monoradar Data (Plots and Tracks)
- **Category 002 (Ed. 1.0):** Transmission of Monoradar Service Messages

The schema incorporates both human-readable `<xs:documentation>` annotations and machine-readable `<xs:appinfo source="urn:eurocontrol:asterix">` metadata containing the following elements under the custom namespace `xmlns:ast="urn:eurocontrol:asterix"`:
- `<ast:item>`: Eurocontrol ASTERIX standard data item reference (e.g., `I048/040-RHO`, `I048/090`)
- `<ast:description>`: Detailed engineering description of the surveillance field
- `<ast:unit>`: Physical unit (`seconds`, `NM`, `degrees`, `FL`, `feet`, `dBm`, `replies`, `meters`, `hex`, `octal`)
- `<ast:resolution>`: Fractional resolution (e.g., `1/128`, `1/256`, `360/65536`, `1/4`, `1/16384`, `360/8192`)
- `<ast:lsb>`: Least Significant Bit value with physical scale
- `<ast:range min="..." max="..."/>`: Valid standard domain interval

All changes pass 100% of integration and unit tests, with zero compilation warnings and zero Clippy lints.

---

## 2. Table of Quanta & Field Annotations

| Simple Type | Item Reference | Physical Unit | Quantum / Resolution (LSB) | Valid Standard Domain |
|---|---|---|---|---|
| `DataCategoryType` | Category | Dimensionless | 1 | `[0, 255]` |
| `BlockLengthType` | Length | Bytes | 1 byte | `[3, 65535]` |
| `DataSourceIdType` | `I034/010`, `I048/010`, `I001/010`, `I002/010` (SAC/SIC) | Code | 1 | `[0, 255]` |
| `MessageTypeType` | `I034/000`, `I002/000` | Dimensionless ID | 1 | `[1, 255]` |
| `TimeOfDayType` | `I034/030`, `I048/140`, `I002/030` | Seconds (s) | `1/128 s` (~7.8125 ms) | `[0 s, 24 h)` = `[0, 86400 s)` |
| `TruncatedTimeOfDayType` | `I001/141` | Seconds (s) | `1/128 s` (~7.8125 ms) | `[0 s, 512 s)` |
| `SectorNumberType` | `I034/020`, `I002/020` | Degrees (°) | `360 / 256 deg` = 1.40625° | `[0°, 358.59375°]` |
| `AntennaRotationPeriodType` | `I034/041`, `I002/041` | Seconds (s) | `1/128 s` (~7.8125 ms) | `[0 s, 511.992 s]` |
| `DataSourceHeightType` | `I034/120` (Height) | Meters (m) | 6.25 m (or 1 m) | `[-32768, 32767]` |
| `Wgs84CoordinateType` | `I034/120` (Lat/Lon) | Degrees (°) | `180 / 2^23 deg` (~0.00002145767°) | Lat: `[-90°, +90°]`, Lon: `[-180°, +180°]` |
| `CollimationErrorType` | `I034/090` | Degrees (°) | `360 / 2^16 deg` (~0.005493°) | `[0°, 359.9945°]` |
| `MessageCountType` | `I034/070` | Messages | 1 message | `[0, 65535]` |
| `TargetDescriptorType` | `I048/020`, `I001/020` | Bitfield | Bit flags (TYP, SIM, RDP, SPI, RAB, FX) | `[0, 255]` |
| `TrackNumberType` | `I048/161`, `I001/161` | Dimensionless ID | 1 | `[0, 65535]` (standard: `[0, 4095]`) |
| `PolarDistanceType` | `I048/040`, `I034/100`, `I001/040` (Rho) | Nautical Miles (NM) | `1/256 NM` (~7.2297 m) | `[0 NM, 255.996 NM]` |
| `PolarAzimuthType` | `I048/040`, `I034/100`, `I001/040` (Theta) | Degrees (°) | `360 / 2^16 deg` (~0.005493164°) | `[0°, 359.9945°]` |
| `CartesianCoordinateType` | `I048/042`, `I001/042` (X, Y) | Nautical Miles (NM) | `1/64 NM` (~28.918 m) in Cat 048; `1/16 NM` in Cat 001 | `[-512.0 NM, +511.984 NM]` |
| `GroundSpeedType` | `I048/200`, `I001/200` | NM/s (knots eq.) | `2^-14 NM/s` (~0.2197 kt / ~0.113 m/s) | `[0, 3.9999 NM/s]` (~`[0, 14400 kt]`) |
| `HeadingType` | `I048/200`, `I001/200` | Degrees (°) | `360 / 2^16 deg` (~0.005493164°) | `[0°, 359.9945°]` |
| `Mode3ACodeType` | `I048/070`, `I001/070` | Octal code | 1 octal code (A4-A1, B4-B1, C4-C1, D4-D1) | `[0000, 7777]` with V, G, L bits |
| `FlightLevelType` | `I048/090`, `I001/090` | Flight Level (FL) | `1/4 FL` (= 25 ft ~ 7.62 m) | `[-12 FL, +1270 FL]` (`[-300 ft, +127000 ft]`) |
| `Height3dType` | `I048/110` | Feet (ft) | 25 ft (`1/4 FL`) | `[0 ft, 819100 ft]` |
| `AircraftAddressType` | `I048/220` | Hexadecimal | 1 | `[0x000000, 0xFFFFFF]` (24 bits) |
| `BdsDataType` | `I048/250` (MB Data) | Bytes | 1 bit (56 bits / 7 octets) | 7 octets hexBinary |
| `BdsRegisterType` | `I048/250` (BDS Register) | Hexadecimal | 1 (BDS 1 & BDS 2) | `[0x00, 0xFF]` (e.g. 0x40, 0x50, 0x60) |
| `TrackStatusType` | `I048/170` | Bitfield | Bit flags (CNF, RAD, DOU, MAH, CDM, FX) | `[0, 255]` |
| `CommCapabilityType` | `I048/230` | Bitfield | Bit flags (COM, STAT, SI, MSSC, ARC, AIC, B1A) | `[0, 65535]` (16 bits) |
| `SrlType` | `I048/130-SRL` | Degrees (°) | `360 / 2^13 deg` (~0.043945°) | `[0°, 11.206°]` |
| `SrrType` | `I048/130-SRR` | Replies | 1 reply | `[0, 255]` |
| `SamType` | `I048/130-SAM` | dBm | 1 dBm | `[0, 255 dBm]` |
| `PrlType` | `I048/130-PRL` | Degrees (°) | `360 / 2^13 deg` (~0.043945°) | `[0°, 11.206°]` |
| `PamType` | `I048/130-PAM` | dBm | 1 dBm | `[0, 255 dBm]` |
| `RpdType` | `I048/130-RPD` | Nautical Miles (NM) | `1/256 NM` (~7.2297 m) two's complement | `[-0.5 NM, +0.496 NM]` |
| `ApdType` | `I048/130-APD` | Degrees (°) | `360 / 2^14 deg` (~0.02197°) two's complement | `[-2.8125°, +2.7905°]` |

---

## 3. Structural & Semantic Verification

1. **Custom Annotation Handling in Compiler:**
   - Evaluated by `dfdl-schema`: non-DFDL appinfo namespaces (`source="urn:eurocontrol:asterix"`) are correctly distinguished from DFDL annotations and skipped during schema lowering without generating extraneous warnings.
2. **Schema Compilation & Facets:**
   - New simpleTypes for `I048/130` compound subfields (`SrlType`, `SrrType`, `SamType`, `PrlType`, `PamType`, `RpdType`, `ApdType`), `Height3dType`, `MessageCountType`, and `CollimationErrorType` enforce exact standard byte widths, min/max constraints, and metadata.
3. **Automated Test Coverage:**
   - Added unit test [`tests::test_asterix_quantum_and_documentation_metadata`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/src/lib.rs#L317-L377) verifying schema presence of custom annotations, units, resolutions, LSB values, and numeric facet constraints across all items.
   - All 15 tests in `dfdl-example` pass with 0 failures.
   - PCAP validation over all 120 blocks passes with 0 unparsed bytes and bit-exact parsing.
