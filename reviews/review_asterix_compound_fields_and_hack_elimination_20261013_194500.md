# Domain & Engineering Review: ASTERIX Compound Fields & Eliminating the `trailingOctets` Anti-Pattern

**Timestamp:** 2026-10-13 19:45:00 UTC+2  
**Domain Areas:** Eurocontrol ASTERIX Standard (Category 034 Ed. 1.28 & Category 048 Ed. 1.21), Air Traffic Management (ATM) Surveillance Systems, DFDL Specification 1.0, High-Assurance Rust (`no_std` / zero-panic), Binary Protocol Engineering.

---

## 1. Critique & Root Cause Analysis

### 1.1 The Anti-Pattern
In previous iterations, the schema incorporated a catch-all element:
```xml
<xs:element name="trailingOctets" type="xs:unsignedByte" minOccurs="0" maxOccurs="1000" dfdl:occursCountKind="parsed"/>
```
accompanied by the post-hoc rationale:
> *"Trailing & Compound Fields: trailingOctets (minOccurs="0" maxOccurs="1000" parsed) cleanly absorbs variable compound subfields (e.g. I034/050 Status and I034/060 Mode)"*

This rationalization was an architectural masking technique (anti-pattern) hiding several specific modeling failures:
1. **Misunderstanding of Compound Data Items:** In Eurocontrol ASTERIX, Compound Data Items are not arbitrary variable blobs. They begin with an extensible Primary Subfield (PSF) indicator octet where each bit deterministically signals the presence or absence of a well-defined subfield (e.g., COM, PSR, SSR, MDS).
2. **Incorrect Bit Mask Calculations:** In Item `I034/060` (System Processing Mode), the schema was inspecting bit 7 (`idiv 64`, spare) instead of bit 5 (`idiv 16`) for PSR, and bit 6 (`idiv 32`, spare) instead of bit 4 (`idiv 8`) for SSR. When messages containing both Item 050 and Item 060 arrived (e.g. Sector Crossing blocks #21–#24), parsing failed or shifted offsets, which was masked by dumping leftover bytes into `trailingOctets`.
3. **Improper Category 048 Framing:** In Category 048, Data Blocks contain one or more target reports (`record` repeating), and several items are either compound (`I048/130` Radar Plot Characteristics), repetitive (`I048/250` Mode S MB Data with a 1-byte `REP` indicator), or extended via FX (`I048/170` Track Status). When flattened and fixed, bytes were left unconsumed, and `trailingOctets` was deployed to silence parse errors.

---

## 2. Standard-Compliant Modeling of Compound Items

### 2.1 Category 034: Item 050 (System Configuration & Status)
Under Eurocontrol Cat 034 Ed. 1.28 §5.2.5:
- **Primary Subfield Indicator (`statusFspec`):** 1 octet (present when FSPEC1 bit 3 / weight 4 = 1).
  - Bit 8 (weight 128): `COM` – Common Part (1 octet: `statusCom`)
  - Bit 7, 6: Spare (0)
  - Bit 5 (weight 16): `PSR` – Specific Status for PSR Sensor (1 octet: `statusPsr`)
  - Bit 4 (weight 8): `SSR` – Specific Status for SSR Sensor (1 octet: `statusSsr`)
  - Bit 3 (weight 4): `MDS` – Specific Status for Mode S Sensor (2 octets: `statusMds`)
  - Bit 2: Spare (0)
  - Bit 1: `FX` – Field Extension Indicator

```xml
<!-- FRN 6 / Item 050: System Configuration and Status -->
<xs:element name="statusFspec" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression" dfdl:occursCount="{ (../fspec1 idiv 4) mod 2 }"/>
<xs:element name="statusCom" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../statusFspec)) then ((../statusFspec idiv 128) mod 2) else 0 }"/>
<xs:element name="statusPsr" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../statusFspec)) then ((../statusFspec idiv 16) mod 2) else 0 }"/>
<xs:element name="statusSsr" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../statusFspec)) then ((../statusFspec idiv 8) mod 2) else 0 }"/>
<xs:element name="statusMds" type="xs:unsignedShort" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../statusFspec)) then ((../statusFspec idiv 4) mod 2) else 0 }"/>
```

### 2.2 Category 034: Item 060 (System Processing Mode)
Under Eurocontrol Cat 034 Ed. 1.28 §5.2.6:
- **Primary Subfield Indicator (`modeFspec`):** 1 octet (present when FSPEC1 bit 2 / weight 2 = 1).
  - Bit 8 (weight 128): `COM` – Common Part (1 octet: `modeCom`)
  - Bit 7, 6: Spare (0)
  - Bit 5 (weight 16): `PSR` – Specific Processing Mode for PSR (1 octet: `modePsr`)
  - Bit 4 (weight 8): `SSR` – Specific Processing Mode for SSR (1 octet: `modeSsr`)
  - Bit 3 (weight 4): `MDS` – Specific Processing Mode for Mode S (1 octet: `modeMds`)
  - Bit 2: Spare (0)
  - Bit 1: `FX` – Field Extension Indicator

```xml
<!-- FRN 7 / Item 060: System Processing Mode -->
<xs:element name="modeFspec" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression" dfdl:occursCount="{ (../fspec1 idiv 2) mod 2 }"/>
<xs:element name="modeCom" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../modeFspec)) then ((../modeFspec idiv 128) mod 2) else 0 }"/>
<xs:element name="modePsr" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../modeFspec)) then ((../modeFspec idiv 16) mod 2) else 0 }"/>
<xs:element name="modeSsr" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../modeFspec)) then ((../modeFspec idiv 8) mod 2) else 0 }"/>
<xs:element name="modeMds" type="xs:unsignedByte" minOccurs="0" maxOccurs="1"
            dfdl:occursCountKind="expression"
            dfdl:occursCount="{ if (fn:exists(../modeFspec)) then ((../modeFspec idiv 4) mod 2) else 0 }"/>
```

---

## 3. Verification Across Live Capture

| Category 034 Message | Block Examples | FSPEC & Bitmasks | Decoded Octet Stream | Parity Status |
|:---|:---|:---|:---|:---:|
| **North Marker** | Blocks #9, #10 | `0xEF 0x10` (FRN 1, 2, 3, 5, 6, 7, 11) | `statusFspec=0x84` (COM 1B + MDS 2B = 4B total); `modeFspec=0x84` (COM 1B + MDS 1B = 3B total) | 100% Bit Exact (0 trailing) |
| **Sector Crossing (Standard)** | Blocks #1–#8, #11–#16, #25–#34 | `0xF0` (FRN 1, 2, 3, 4) | SAC/SIC (2B) + MsgType (1B) + TimeOfDay (3B) + Sector (1B) = 11B total | 100% Bit Exact (0 trailing) |
| **Sector Crossing (Status Only)** | Blocks #17–#20 | `0xF4` (FRN 1, 2, 3, 4, 6) | Sector (1B) + `statusFspec=0x94` (COM 1B + PSR 1B + MDS 2B = 5B total) = 16B total | 100% Bit Exact (0 trailing) |
| **Sector Crossing (Status + Mode)** | Blocks #21–#24 | `0xF6` (FRN 1, 2, 3, 4, 6, 7) | Sector (1B) + `statusFspec=0x88` (COM 1B + SSR 1B = 3B) + `modeFspec=0x80` (COM 1B = 2B) = 16B total | 100% Bit Exact (0 trailing) |

Every single one of the 34 authentic Category 034 blocks in the live surveillance stream parses cleanly down to the exact bit with **zero trailing bytes**, **zero buffer overrun**, and **zero unmodeled octets**.

---

## 4. Category 048 Standard-Compliant Modeling & Verification

### 4.1 Changes Implemented
1. **Multi-Record Block Framing:** `Cat048RecordType` now models repeating target reports via `<xs:element name="record" type="Cat048RecordItemType" minOccurs="1" maxOccurs="unbounded" dfdl:occursCountKind="parsed"/>`.
2. **Compound Data Item `I048/130` (Radar Plot Characteristics):**
   - Modeled with Primary Subfield Indicator `plotCharFspec` (1B, conditioned on FRN 7: bit 1 of `fspec1`).
   - Dynamically evaluates `srl`, `srr`, `sam`, `prl`, `pam`, `rpd`, `apd` (each 1B) according to their respective bits (weights 128, 64, 32, 16, 8, 4, 2).
3. **Repetitive Data Item `I048/250` (Mode S MB Data):**
   - Reads 1-octet repetition factor `bdsRep`.
   - Iterates `bdsReport` (`BdsReportType`: 7 octets `bdsData` hexBinary + 1 octet `bdsReg`) exactly `bdsRep` times.
4. **Extended Data Item `I048/170` (Track Status):**
   - Reads base `trackStatus` (1B), followed by `trackStatusExt` (1B) if FX bit (`../trackStatus mod 2`) is 1.
5. **Fixed FRN 21 (`I048/230` Communications / ACAS Capability):**
   - Corrected formula from `(fspec3 idiv 4) mod 2` to `(fspec3 idiv 2) mod 2`.
6. **Elimination of `trailingOctets`:** Completely purged from `asterix.dfdl.xsd`.

### 4.2 Test Verification Results
- **120 / 120 Blocks (100% Pass):** All 86 Cat 048 blocks (128 individual target reports) + all 34 Cat 034 blocks pass schema validation with **0 unparsed bytes**.
- **Packet 13 Multi-Record Block:** 416-byte block decoded into all 9 individual target reports without trailing bytes.
- **Zero Clippy Warnings:** `cargo clippy --package dfdl-example --all-targets` passes with code 0.

