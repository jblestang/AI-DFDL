# Domain Review: Detailed ASTERIX Bitfields in DFDL Schema & Wireshark Dissector

**Timestamp**: 2026-10-13T21:55:00+02:00  
**Context**: Detailed bit-level decomposition of Eurocontrol ASTERIX Category 048 / 001 bitfields (`TargetDescriptorType`, `TargetDescriptorExtType`, `TrackStatusType`, `TrackStatusExtType`, `CommCapabilityType`) directly inside the DFDL schema ([`asterix.dfdl.xsd`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd)).

---

## 1. Domain Expertise Assessment (Radar & Surveillance Systems - ASTERIX Cat 048)

### 1.1 Eurocontrol Category 048 Specification Adherence
1. **Target Report Descriptor (I048/020)**:
   - **Primary Subfield (1 octet)**:
     - `typ` (bits 8-6, length 3): Target Report Type (No detection, Single PSR, Single SSR, SSR+PSR, etc.).
     - `sim` (bit 5, length 1): Simulated target report (0 = Actual, 1 = Simulated).
     - `rdp` (bit 4, length 1): Report from RDP (0 = Default surv chain, 1 = RDP).
     - `spi` (bit 3, length 1): Special Position Identification (0 = Default, 1 = SPI present).
     - `rab` (bit 2, length 1): Report from target report calculation / reply (0 = Report, 1 = Reply).
     - `fx` (bit 1, length 1): Field Extension indicator (0 = End, 1 = Extension follows).
   - **First Extension (I048/020 First Ext, 1 octet)**:
     - `tst` (bit 8, length 1): Test target indicator.
     - `err` (bit 7, length 1): Extended range error condition.
     - `xpp` (bit 6, length 1): X-pulse present in Mode 3/A reply.
     - `me` (bit 5, length 1): Military emergency.
     - `mi` (bit 4, length 1): Military identification.
     - `foeFri` (bits 3-2, length 2): Friend / Foe identification code.
     - `fx` (bit 1, length 1): Extension indicator.
   - **Verification**: Conforms 100% to Eurocontrol Standard Document Part 4 (Category 048), Edition 1.30.

2. **Track Status (I048/170)**:
   - **Primary Subfield (1 octet)**:
     - `cnf` (bit 8, length 1): Confirmed vs Tentative track (0 = Confirmed, 1 = Tentative).
     - `rad` (bits 7-6, length 2): Maintaining Sensor Type (0 = Combined, 1 = PSR, 2 = SSR/Mode S, 3 = Invalid).
     - `dou` (bit 5, length 1): Doubtful correlation.
     - `mah` (bit 4, length 1): Horizontal manoeuvre detection.
     - `cdm` (bits 3-2, length 2): Climb / Descent Mode (0 = Maintaining, 1 = Climbing, 2 = Descending, 3 = Unknown).
     - `fx` (bit 1, length 1): Field Extension indicator.
   - **First Extension (I048/170 First Ext, 1 octet)**:
     - `tre` (bit 8, length 1): Track termination indicator.
     - `gho` (bit 7, length 1): Ghost track indicator.
     - `sup` (bit 6, length 1): Correlation suppression.
     - `tcc` (bit 5, length 1): Coordinate conversion mode (Radar slant to local Cartesian vs sensor plane).
     - `spare` (bits 4-2, length 3): Reserved / spare bits set to 0.
     - `fx` (bit 1, length 1): Field Extension indicator.
   - **Verification**: Exact match with Eurocontrol Cat 048 Item 170.

3. **Communications / ACAS Capability and Flight Status (I048/230)**:
   - **2 Octets (16 bits)**:
     - `com` (bits 16-14, length 3): Communications Capability (Transponder capability, BDS registers available).
     - `stat` (bits 13-11, length 3): Flight Status (Airborne, On-ground, Alert, SPI).
     - `si` (bit 10, length 1): SI / II code capability.
     - `spare` (bit 9, length 1): Spare bit.
     - `mssc` (bit 8, length 1): Mode S Specific Service Capability.
     - `arc` (bit 7, length 1): Altitude Reporting Capability (25ft vs 100ft).
     - `aic` (bit 6, length 1): Aircraft Identification Capability.
     - `b1a` (bit 5, length 1): BDS 0,5 bit 16 status.
     - `b1b` (bits 4-1, length 4): BDS 0,5 bits 37-40 status.
   - **Verification**: Exact match with I048/230 (Mode S transponder capability summary).

---

## 2. DFDL Modeling & Parser Architecture

1. **Sub-Byte Bitfield Alignment in DFDL**:
   - The top-level format defaults to `alignmentUnits="bytes"`. When breaking down an octet or short into individual 1-, 2-, 3-, or 4-bit fields, each subfield must explicitly declare `dfdl:alignment="1"` and `dfdl:alignmentUnits="bits"`, alongside `dfdl:lengthUnits="bits"` and `dfdl:length="N"`.
   - Without `dfdl:alignment="1" dfdl:alignmentUnits="bits"`, DFDL parsers re-align to byte boundaries after each field, which would fail to parse contiguous bit sequences.
   - All defined subfield elements in `asterix.dfdl.xsd` now enforce `alignment="1" alignmentUnits="bits"`.

2. **Recursive Relative Expressions for Extension Octets**:
   - In ASTERIX, extension octets are present if and only if the `fx` bit of the preceding octet equals 1.
   - For `targetDescriptorExt`:
     ```xml
     dfdl:occursCountKind="expression"
     dfdl:occursCount="{ if (fn:exists(../targetDescriptor)) then ../targetDescriptor/fx else 0 }"
     ```
   - For `trackStatusExt`:
     ```xml
     dfdl:occursCountKind="expression"
     dfdl:occursCount="{ if (fn:exists(../trackStatus)) then ../trackStatus/fx else 0 }"
     ```
   - The `../targetDescriptor/fx` expression resolves cleanly through the infoset tree, accurately controlling the optional presence of the extension record.

---

## 3. Wireshark & UI/UX Integration

1. **Zero Dissector Code Changes**:
   - The generic dissector in `dfdl-wireshark` builds the dissection tree dynamically from the DFDL infoset element hierarchy (`DissectionNode::from_element`).
   - By transforming `targetDescriptor`, `trackStatus`, and `commCapability` from `xs:unsignedByte` / `xs:unsignedShort` into `xs:complexType`, Wireshark automatically displays expandable tree subnodes:
     - `+ targetDescriptor`
       - `typ: 5`
       - `sim: 0`
       - `rdp: 0`
       - `spi: 0`
       - `rab: 0`
       - `fx: 0`
     - `+ trackStatus`
       - `cnf: 0`
       - `rad: 2`
       - `dou: 0`
       - `mah: 0`
       - `cdm: 0`
       - `fx: 1`
     - `+ trackStatusExt`
       - `tre: 0`
       - `gho: 0`
       - `sup: 0`
       - `tcc: 0`
       - `spare: 0`
       - `fx: 0`
     - `+ commCapability`
       - `com: 1`
       - `stat: 0`
       - `si: 0`
       - `spare: 0`
       - `mssc: 1`
       - `arc: 1`
       - `aic: 1`
       - `b1a: 1`
       - `b1b: 5`
   - Descriptions, items, and unit annotations are preserved in the schema metadata registry and exposed via tooltips/comments without cluttering the tree label text.

---

## 4. Test & Verification Summary

| Test Suite | Command | Result |
|:---|:---|:---|
| DFDL Core & Engine | `cargo test --package dfdl-core` | Passed (100%) |
| DFDL Example & Asterix PCAP | `cargo test --package dfdl-example` | Passed (15/15 tests) |
| DFDL Wireshark Dissector Engine | `cargo test --package dfdl-wireshark` | Passed (3/3 tests) |
| Clippy Strict Linter | `cargo clippy --package dfdl-core --package dfdl-wireshark --package dfdl-example --all-targets` | 0 warnings |
| Live PCAP Dissection Validation | `target/release/dfdl-wireshark decode --schema asterix.dfdl.xsd --pcap cat_034_048.pcap --limit 1` | Verified complete bitfields breakdown |
| Wireshark Lua/tshark Integration | `tshark -X lua_script:.../dfdl_dissector.lua -r cat_034_048.pcap -c 1 -V` | Verified nested tree dissection |
