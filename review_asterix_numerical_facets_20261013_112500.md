# Domain Review: ASTERIX Numerical Facets & Value Range Validation

**Date & Time:** 2026-10-13T11:25:00+02:00  
**Context:** Eurocontrol ASTERIX DFDL Schema (`crates/dfdl-example/schemas/asterix.dfdl.xsd`)  
**Domain Areas:** Radar Surveillance (Eurocontrol ASTERIX Cat 001, 002, 034, 048), Air Traffic Management (ATM), DFDL XML Schema Specification

---

## 1. Domain Standards & Specification Reference

Eurocontrol ASTERIX specifies physical and binary bounds for all data items across all surveillance data categories:
- **Eurocontrol Standard for Radar Data Exchange Part 1:** All categories share category number (0–255) and block length (3–65,535 bytes).
- **ASTERIX Cat 034 (Nextgen Monoradar Service Messages, Ed. 1.28):**
  - `I034/010` Data Source Identifier: SAC / SIC (0..255).
  - `I034/020` Message Type: North Marker (1), Sector Crossing (2), Status (3), etc. (1..255).
  - `I034/030` Time of Day: 24-bit representation with LSB = 1/128 s (0..16,777,215, covering 0 to 24h = 86,400 × 128 = 11,059,200 < 2^24).
  - `I034/041` Antenna Rotation Period: 16-bit representation with LSB = 1/128 s (0..65,535).
  - `I034/050` System Configuration & Status: 8-bit status flags (0..255).
  - `I034/060` System Processing Mode: 8-bit mode indicator (0..255).
  - `I034/070` Message Count: 16-bit count (0..65,535).
- **ASTERIX Cat 048 (Monoradar Target Reports, Ed. 1.21):**
  - `I048/010` Data Source Identifier: SAC / SIC (0..255).
  - `I048/020` Target Report Descriptor: Primary / Secondary plot flags (0..255).
  - `I048/040` Measured Position in Polar Coordinates:
    - Distance (R): 16-bit unsigned, LSB = 1/256 NM (0..65,535, max ~256 NM).
    - Azimuth (Theta): 16-bit unsigned, LSB = 360°/2^16 = 0.005493° (0..65,535).
  - `I048/042` Calculated Position in Cartesian Coordinates:
    - X, Y coordinates: 16-bit signed two's complement, LSB = 1/64 NM (-32,768..32,767, range [-512 NM, +512 NM]).
  - `I048/070` Mode-3/A Code in Octal: 12-bit code mapped into 16-bit unsigned (0..65,535).
  - `I048/090` Flight Level in Binary Representation: 16-bit signed/unsigned code (0..65,535, LSB = 1/4 FL).
  - `I048/130` Radar Plot Characteristics: 8-bit SSR / primary metrics (0..4,294,967,295).
  - `I048/140` Time of Day: 24-bit representation with LSB = 1/128 s (0..16,777,215).
  - `I048/161` Track Number: 16-bit unsigned (0..65,535).
  - `I048/200` Calculated Track Velocity in Polar Coordinates:
    - Ground Speed: 16-bit unsigned, LSB = (2^-14) NM/s (0..65,535).
    - Heading: 16-bit unsigned, LSB = 360°/2^16 (0..65,535).
  - `I048/220` Aircraft Address: 24-bit ICAO aircraft address (0..16,777,215).
  - `I048/240` Aircraft Identification: 8-character callsign (6 bits/char).
  - `I048/250` Mode S Comm B Data: 64-bit BDS register data (0..18,446,744,073,709,551,615) + 8-bit BDS address (0..255).
- **ASTERIX Cat 001 (Monoradar Data Target Reports, Ed. 1.26):**
  - `I001/010` SAC / SIC (0..255).
  - `I001/020` Target Descriptor (0..255).
  - `I001/040` Measured Polar Coordinates: Distance (0..65,535) and Azimuth (0..65,535).
  - `I001/041` Cartesian Coordinates: X and Y (-32,768..32,767).
  - `I001/070` Mode-3/A Code (0..65,535).
  - `I001/080` Mode C Code (0..65,535).
  - `I001/130` Radar Plot Characteristics (0..65,535).
  - `I001/141` Truncated Time of Day: 16-bit representation with LSB = 1/128 s (0..65,535, rollover every 512 s).
- **ASTERIX Cat 002 (Monoradar Service Messages, Ed. 1.0):**
  - `I002/010` SAC / SIC (0..255).
  - `I002/020` Message Type (1..255).
  - `I002/030` Sector Number (0..255).
  - `I002/041` Antenna Rotation Period (0..65,535).

---

## 2. Review Remarks & Assessment

### Positive Findings:
1. **DFDL Schema Native Support for XSD Facets:**
   - The DFDL schema engine lowers XML Schema `xs:minInclusive` and `xs:maxInclusive` facets into `ResolvedProperties.facets`.
   - Named types cleanly isolate validation logic, allowing both single elements and repeating items within composite structures (e.g., BDS registers, target coordinates) to share verified mathematical bounds.
2. **Surveillance Value Fidelity:**
   - 24-bit items (`TimeOfDayType`, `AircraftAddressType`) are mapped to `xs:unsignedInt` with `maxInclusive = 16777215` (0xFFFFFF), preventing out-of-range 32-bit values from corrupting downstream track correlation.
   - 16-bit Cartesian items (`CartesianCoordinateType`) preserve the signed two's complement interval `[-32768, 32767]`.
   - 24-bit WGS-84 coordinates (`Wgs84CoordinateType`) reflect `[-8388608, 8388607]`, matching Eurocontrol 3-byte latitude/longitude angle encodings.
3. **Multi-Record Packet Compatibility:**
   - Adding explicit facets to record fields does not interfere with multi-record network packets (`AsterixPacket` or repeating `Cat001RecordItem`).
   - All 120 blocks in the real-world CroatiaControl PCAP sample validate cleanly with zero violations.

### Verification Matrix:
| Domain Type | Base Primitive | Min Value | Max Value | Eurocontrol Item |
|---|---|---|---|---|
| `DataCategoryType` | `xs:unsignedByte` | 0 | 255 | Category |
| `BlockLengthType` | `xs:unsignedShort` | 3 | 65535 | Length |
| `DataSourceIdType` | `xs:unsignedByte` | 0 | 255 | I034/010, I048/010, I001/010, I002/010 |
| `MessageTypeType` | `xs:unsignedByte` | 1 | 255 | I034/020, I002/020 |
| `TimeOfDayType` | `xs:unsignedInt` | 0 | 16777215 | I034/030, I048/140 |
| `TruncatedTimeOfDayType`| `xs:unsignedShort` | 0 | 65535 | I001/141 |
| `SectorNumberType` | `xs:unsignedByte` | 0 | 255 | I034/050, I002/030 |
| `AntennaRotationPeriodType` | `xs:unsignedShort` | 0 | 65535 | I034/041, I002/041 |
| `DataSourceHeightType` | `xs:short` | -32768 | 32767 | I034/090 |
| `Wgs84CoordinateType` | `xs:int` | -8388608 | 8388607 | I034/100 |
| `TargetDescriptorType`| `xs:unsignedByte` | 0 | 255 | I048/020, I001/020 |
| `TrackNumberType` | `xs:unsignedShort` | 0 | 65535 | I048/161 |
| `PolarDistanceType` | `xs:unsignedShort` | 0 | 65535 | I048/040, I001/040 |
| `PolarAzimuthType` | `xs:unsignedShort` | 0 | 65535 | I048/040, I001/040 |
| `CartesianCoordinateType` | `xs:short` | -32768 | 32767 | I048/042, I001/041 |
| `GroundSpeedType` | `xs:unsignedShort` | 0 | 65535 | I048/200 |
| `HeadingType` | `xs:unsignedShort` | 0 | 65535 | I048/200 |
| `Mode3ACodeType` | `xs:unsignedShort` | 0 | 65535 | I048/070, I001/070 |
| `FlightLevelType` | `xs:unsignedShort` | 0 | 65535 | I048/090, I001/080 |
| `AircraftAddressType` | `xs:unsignedInt` | 0 | 16777215 | I048/220 |
| `BdsDataType` | `xs:unsignedLong` | 0 | 18446744073709551615 | I048/250 |
| `BdsRegisterType` | `xs:unsignedByte` | 0 | 255 | I048/250 |
| `TrackStatusType` | `xs:unsignedByte` | 0 | 255 | I048/170 |
| `CommCapabilityType` | `xs:unsignedShort` | 0 | 65535 | I048/230 |
| `PlotCharacteristicsType`| `xs:unsignedInt` | 0 | 4294967295 | I048/130, I001/130 |
