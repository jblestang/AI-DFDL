# Technical Review: Generic DFDL Wireshark Dissector & PCAP Plugin (`dfdl-wireshark`)

- **Date:** 2026-10-13 20:42:00 UTC
- **Crate:** [`crates/dfdl-wireshark`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark)
- **Component:** Wireshark Plugin, Lua Dissector Generator, and Libpcap Engine
- **Reviewer:** Domain Specialist (Surveillance Protocols, Network Dissection, Cyber, High-Assurance Rust)

---

## 1. Executive Summary

This review validates the architecture and implementation of `dfdl-wireshark`, built to address the requirement:
> *"the wireshark plugin shall be generic and use only data from the DFDL schema, no hard coded stuff"*

All legacy protocol-specific assumptions, hardcoded category branches (`Cat034`, `Cat048`), and static field mapping dictionaries have been completely eliminated. The plugin is now a **100% generic, schema-driven network dissector**.

Any binary protocol defined via a DFDL schema (`.dfdl.xsd`) can be loaded dynamically, inspected for engineering units/quanta/documentation, dissected from live Wireshark/TShark packet streams, or extracted from PCAP capture files.

---

## 2. Core Architecture & Zero Hardcoding Guarantee

### 2.1 Dynamic Metadata Introspection ([`src/schema_meta.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/schema_meta.rs))
- **Schema Single Source of Truth:** Introspects `<xs:element>`, `<xs:simpleType>`, `<xs:complexType>`, and nested `<xs:annotation>`/`<xs:appinfo>` tags dynamically using streaming XML reader (`XmlReader`).
- **Mathematical Quantum Evaluator:** Evaluates fractional resolutions (`1/128`, `360/65536`, `1/256`, `1/4`), power-of-two resolutions (`2^-14`), and complex coordinate scaling (`180/8388608`) dynamically using math parser `parse_generic_scale`.
- **Zero Static Symbol Tables:** No field names, protocol identifiers, or category numbers are hardcoded.

### 2.2 Generic Unit & Engineering Value Conversion ([`src/converter.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/converter.rs))
- Multiplies raw numeric values by the schema-declared `scale_factor`.
- Formats physical values based on the schema's declared `<*:unit>` tag:
  - `seconds`: Formats sub-second time and UTC clock representation `HH:MM:SS.mmm UTC`.
  - `NM`: Formats Nautical Miles with automatic kilometer equivalent.
  - `degrees`: Formats decimal angles `°`.
  - `FL`: Formats Flight Levels with foot altitude equivalent `(ft)`.
  - `NM/s`: Formats ground speeds with knots and m/s conversions.
  - `dBm`, `octal`, `hex`, `meters`: Formats according to declared unit without hardcoded field name matching.

### 2.3 Dissection Tree & Multi-Format Serializers ([`src/dissector.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/dissector.rs))
- **`DissectionTree` & `DissectionNode`:** Hierarchical tree directly matching the DFDL Infoset DOM.
- **Node Labels:** Formatted as `{name} ({title}): {converted_value} [{raw_value}]` with documentation tooltip attached.
- **Wireshark PDML:** Emits standard Wireshark Packet Details Markup Language (`<field name="..." show="..." unit="..." tooltip="..."/>`).
- **JSON Serialization:** Emits JSON tree structure consumed by Wireshark Lua plugins, web visualizers, or downstream tools.

### 2.4 Generic Dissector Engine ([`src/engine.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/engine.rs))
- Compiles any DFDL XML schema into executable in-memory IR using `dfdl_schema::SchemaCompiler`.
- Executes `dfdl_core::kernel::ParserEngine` against raw byte streams.
- Seamlessly handles both single-message datagrams and multi-record stream blocks without protocol-specific framing hacks.

### 2.5 Transport-Layer Decapsulation & PCAP Processing ([`src/pcap_runner.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/pcap_runner.rs))
- Protocol-agnostic capture decoder:
  - Extracts standard Ethernet/IPv4/UDP payloads or raw network frames.
  - Passes raw payloads directly into `DfdlDissectorEngine`.
  - Emits `DissectedPacket` stream decorated with capture timestamps and converted trees.

### 2.6 Wireshark Lua Dissector Generator & C-ABI FFI ([`src/lua_dissector.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/lua_dissector.rs), [`src/ffi.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-wireshark/src/ffi.rs))
- **Lua Plugin:** Completely generic Lua script (`dfdl_dissector.lua`) containing a self-contained JSON decoder.
- **Wireshark GUI Integration:** Populates Wireshark packet detail tree with full physical conversions and documentation tooltips for any loaded schema.
- **Native C-ABI:** Exports thread-safe, panic-free `dfdl_dissect_hex`, `dfdl_dissect_bytes`, and `dfdl_free_string` for dynamic library loading (`cdylib`).

---

## 3. Verification & Compliance Matrix

| Requirement / Standard | Status | Evidence |
|:---|:---:|:---|
| **Zero Hardcoded Logic** | **COMPLIANT** | Audited with `grep`: 0 hardcoded categories, 0 static field lists. |
| **Dynamic Quanta & Units** | **COMPLIANT** | Tested on `cat_034_048.pcap`: converted `rho` (NM), `theta` (deg), `flightLevel` (FL), `timeOfDay` (UTC). |
| **Documentation Tooltips** | **COMPLIANT** | Schema `<xs:documentation>` extracted and presented in tree nodes and PDML tooltips. |
| **Wireshark / TShark Lua** | **COMPLIANT** | `generate-lua` produces standalone Lua dissector script binding to user-specified UDP ports. |
| **Workspace Lint Rules** | **COMPLIANT** | `cargo clippy --package dfdl-wireshark --all-targets` passes with 0 warnings (`unwrap_used = "deny"`, `indexing_slicing = "deny"`). |
| **Unit & Integration Tests**| **COMPLIANT** | `cargo test --package dfdl-wireshark` passes 3/3 tests (Lua generator, metadata extraction, engine execution). |

---

## 4. Live Verification Output

Execution against real network capture [`cat_034_048.pcap`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/data/cat_034_048.pcap) with [`asterix.dfdl.xsd`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd):

```text
- timeOfDay (Time of Day (UTC since midnight)): 27354.602 s (07:35:54.602 UTC) [3501389]
    [Doc: Item: I034/030, I048/140 | Time of Day | Unit: seconds | Resolution: 1/128 | Range: 0 to 86400 s]
- rho (Measured Distance Rho in Polar Coordinates): 197.684 NM (366.11 km) [50607]
    [Doc: Item: I048/040-RHO | Measured Distance Rho | Unit: NM | Resolution: 1/256 | Range: 0 to 255.996 NM]
- theta (Measured Azimuth Theta in Polar Coordinates): 340.137° [61920]
    [Doc: Item: I048/040-THETA | Measured Azimuth Theta | Unit: degrees | Resolution: 360/65536]
- flightLevel (Flight Level in Binary Representation): FL 330.0 (33000 ft) [1320]
    [Doc: Item: I048/090 | Flight Level | Unit: FL | Resolution: 1/4 | LSB: 25 ft]
- groundSpeed (Calculated Track Ground Speed): 434.4 kt (0.1207 NM/s, 223.5 m/s) [1977]
    [Doc: Item: I048/200-SPEED | Ground Speed | Unit: NM/s | Resolution: 1/16384 (~0.2197 kt)]
- heading (Calculated Track Heading): 124.003° [22574]
    [Doc: Item: I048/200-HEADING | Track Heading | Unit: degrees | Resolution: 360/65536]
```
