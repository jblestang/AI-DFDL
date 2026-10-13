# Domain Review: Eurocontrol ASTERIX Cat 034 Fine-Grained Service Message Types

**Date & Time:** 2026-10-13T11:35:00+02:00  
**Context:** Eurocontrol ASTERIX DFDL Schema (`crates/dfdl-example/schemas/asterix.dfdl.xsd`)  
**Domain Areas:** Radar Surveillance (Eurocontrol ASTERIX Category 034: Transmission of Monoradar Service Messages), Air Traffic Management (ATM), DFDL XML Schema Direct Choice Dispatch

---

## 1. Background & Domain Specification

In Eurocontrol ASTERIX Category 034 (Standard for Radar Data Exchange Part 2b / Part 9, Edition 1.28), service messages broadcast operational and environmental status from monoradar sensor heads to Surveillance Data Processing (SDP) systems.

The transaction type is explicitly governed by Data Item **`I034/000` (Message Type)**:
- **Value 1 — North Marker Message:** Transmitted once per rotation when the beam crosses local magnetic or geographic North. Contains station orientation, antenna rotation period (`I034/041`), system configuration (`I034/050`), processing mode (`I034/060`), and antenna 3D coordinates (`I034/120`).
- **Value 2 — Sector Crossing Message:** Transmitted periodically as the antenna crosses azimuth sector boundaries (typically 32 or 256 sectors). Contains azimuth sector number (`I034/020`) and timestamp (`I034/030`).
- **Value 3 — Geographical Filtering Message:** Transmitted when selective geographical data filtering/blanking is modified. Contains polar window boundaries (`I034/100` Generic Polar Window: start/end range and start/end azimuth) and data filter flags (`I034/110`).
- **Value 4 — Jamming Strobe Message:** Signals detected radio-frequency interference or active jamming strobes from Primary Surveillance Radar (PSR). Contains strobe azimuth sector (`I034/020`) and strobe angular span.
- **Value 5 — Solar Storm Message:** Warns of solar flare or geomagnetic disturbance exceeding radar sensitivity thresholds, causing false plots or desensitization in specific azimuth sectors.
- **Value 6 — SSR Jamming Strobe Message:** Signals radio-frequency interference on Secondary Surveillance Radar (1030 MHz interrogation or 1090 MHz reply channels).
- **Value 7 — Mode S Jamming Strobe Message:** Signals RF interference, preamble corruption, or high reply rates affecting Mode S selective interrogators and roll-call periods.
- **Values 8 through 255 — Reserved / System-Specific Messages:** Standard reserved range (EUROCONTROL-SPEC-0149-2b) and proprietary vendor extensions (e.g., radar calibration, site test loops).

---

## 2. DFDL Modeling: From Generic Bucket to Specific Message Profiles

Previously, types 3 through 8 were grouped into a single broad branch:
```xml
<xs:element name="genericService" type="Cat034GenericServiceType" dfdl:choiceBranchKey="3 4 5 6 7 8"/>
```

This was replaced with dedicated, strongly-typed element definitions corresponding directly to the Eurocontrol specification:
1. `northMarker` (`Cat034NorthMarkerType`, `dfdl:choiceBranchKey="1"`)
2. `sectorCrossing` (`Cat034SectorCrossingType`, `dfdl:choiceBranchKey="2"`)
3. `geographicalFiltering` (`Cat034GeographicalFilteringType`, `dfdl:choiceBranchKey="3"`)
4. `jammingStrobe` (`Cat034JammingStrobeType`, `dfdl:choiceBranchKey="4"`)
5. `solarStorm` (`Cat034SolarStormType`, `dfdl:choiceBranchKey="5"`)
6. `ssrJammingStrobe` (`Cat034SsrJammingStrobeType`, `dfdl:choiceBranchKey="6"`)
7. `modeSJammingStrobe` (`Cat034ModeSJammingStrobeType`, `dfdl:choiceBranchKey="7"`)
8. `otherService` (`Cat034OtherServiceType`, `dfdlx:choiceBranchKeyRanges="8 255"`)

### Message Profile Comparison:

| Message Type (`I034/000`) | Element Name | Target ComplexType | Key Data Items |
|---|---|---|---|
| `1` | `northMarker` | `Cat034NorthMarkerType` | SAC/SIC, MsgType, TimeOfDay, Period (`I034/041`), Status (`I034/050`), Mode (`I034/060`), Height/Lat/Lon (`I034/120`) |
| `2` | `sectorCrossing` | `Cat034SectorCrossingType` | SAC/SIC, MsgType, TimeOfDay, SectorNumber (`I034/020`) |
| `3` | `geographicalFiltering` | `Cat034GeographicalFilteringType` | SAC/SIC, MsgType, TimeOfDay, Optional SectorNumber, Polar Window (`I034/100`: `rhoStart`, `rhoEnd`, `thetaStart`, `thetaEnd`) |
| `4` | `jammingStrobe` | `Cat034JammingStrobeType` | SAC/SIC, MsgType, TimeOfDay, SectorNumber, Strobe Angular Range (`strobeStartAzimuth`, `strobeEndAzimuth`) |
| `5` | `solarStorm` | `Cat034SolarStormType` | SAC/SIC, MsgType, TimeOfDay, SectorNumber, Solar Interference Intensity |
| `6` | `ssrJammingStrobe` | `Cat034SsrJammingStrobeType` | SAC/SIC, MsgType, TimeOfDay, SectorNumber, SSR Strobe Angular Range |
| `7` | `modeSJammingStrobe` | `Cat034ModeSJammingStrobeType` | SAC/SIC, MsgType, TimeOfDay, SectorNumber, Mode S Strobe Angular Range |
| `8..255` | `otherService` | `Cat034OtherServiceType` | SAC/SIC, MsgType, TimeOfDay, Trailing Payload / Vendor Extensions |

---

## 3. Direct Dispatch & Performance Analysis

The schema uses expression-based `dfdl:choiceDispatchKey`:
```xml
dfdl:choiceDispatchKey="{ if (((dfdlx:lookAhead(0, 8) idiv 64) mod 2) eq 1) then (if ((dfdlx:lookAhead(0, 8) mod 2) eq 1) then xs:string(dfdlx:lookAhead(32, 8)) else xs:string(dfdlx:lookAhead(24, 8))) else 'unknown' }"
```

- When FSPEC has no extension (`fspec1 mod 2 == 0`, 1 octet), FSPEC (8 bits) + SAC/SIC (16 bits) = 24 bits offset to Message Type byte.
- When FSPEC has extension (`fspec1 mod 2 == 1`, 2 octets), FSPEC (16 bits) + SAC/SIC (16 bits) = 32 bits offset to Message Type byte.
- The dispatch evaluates to the discrete string (`"1"`, `"2"`, `"3"`, `"4"`, `"5"`, `"6"`, `"7"`, or numerical range for `"8"`..`"255"`), achieving $O(1)$ deterministic branch selection without speculative backtracking.

---

## 4. Verification Results

1. **Synthetic Full Category 034 Suite (`test_cat034_all_detailed_message_types`):**
   - Verified that synthetic payloads for types 3, 4, 5, 6, 7, and 8 dispatch cleanly to `<geographicalFiltering>`, `<jammingStrobe>`, `<solarStorm>`, `<ssrJammingStrobe>`, `<modeSJammingStrobe>`, and `<otherService>`.
2. **PCAP Real-World Flight Data Parity:**
   - All 120 blocks in the CroatiaControl PCAP continue to pass 100% schema validation and VM execution.
3. **Clippy & Compiler:**
   - 0 compilation warnings, 0 clippy warnings (`cargo clippy --package dfdl-example --all-targets`).
