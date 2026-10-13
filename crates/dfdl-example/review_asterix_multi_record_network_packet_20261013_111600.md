# Domain Review: Multi-Plot / Multi-Track Architecture in Eurocontrol ASTERIX Network Packets

**Timestamp:** 2026-10-13 11:16:00 UTC  
**Domain Areas:** Radar Surveillance (Eurocontrol ASTERIX), Networking (UDP/IP Datagrams), DFDL Schema Specification, Rust Safety  
**Audited Files:**
- [`crates/dfdl-example/schemas/asterix.dfdl.xsd`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd)
- [`crates/dfdl-example/src/lib.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/src/lib.rs)
- [`crates/dfdl-example/src/pcap.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/src/pcap.rs)
- [`crates/dfdl-example/tests/integration_tests.rs`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-example/tests/integration_tests.rs)

---

## 1. Domain Background & Problem Statement

In civil and military air traffic management (ATM) and surveillance networks, radar data transmission over IP networks (typically UDP) follows Eurocontrol ASTERIX specifications:

1. **Application Transmission Unit (Network Packet / UDP Datagram) - Eurocontrol Part 1 §3.3.1:**
   - A single UDP datagram payload can contain **one or more ASTERIX Data Blocks**.
   - These Data Blocks can belong to different categories (e.g. Category 048 target reports followed by Category 034 service messages) or multiple blocks of the same category.
2. **ASTERIX Data Block - Eurocontrol Part 1 §3.1:**
   - A Data Block consists of:
     - 1 octet Data Category (`CAT`)
     - 2 octets Length Indicator (`LEN`)
     - **One or more Records** of the specified category.
3. **Surveillance Target Reports (Plots / Tracks):**
   - In surveillance categories such as **Category 001** (Monoradar Data) and **Category 048** (Monoradar Target Reports), each Record within the Data Block represents an individual radar plot or track report.
   - Therefore, a single network packet can contain multiple plots/tracks at two levels:
     - **Intra-Block Multiplexing:** Multiple records (plots/tracks) sequenced back-to-back inside a single Data Block under one Category and Length header.
     - **Inter-Block Multiplexing:** Multiple Data Blocks bundled inside a single UDP network packet.

---

## 2. Identified Schema Gaps & Fixes

| Component | Previous State | Fixed State | Standard Reference |
| :--- | :--- | :--- | :--- |
| **Top-Level Network Packet** | Only `AsterixRecord` root existed (single Data Block). No schema element could parse an entire UDP packet with multiple blocks. | Added `AsterixPacket` and `AsterixDatagram` root elements with repeating `AsterixRecord` (`dfdl:occursCountKind="parsed"`). | Eurocontrol Part 1 §3.3.1 |
| **Category 001 Data Block** | `Cat001RecordType` flattened fields of a single plot/track under the Data Block header, preventing multiple records per block. | Refactored `Cat001RecordType` to hold repeating `record` of type `Cat001RecordItemType` (`dfdl:occursCountKind="parsed"`). | Eurocontrol Part 2a §3.1 |
| **Multi-Record Captures** | PCAP blocks with length up to 416 bytes (e.g. Packet 13 containing ~10 plots/tracks in Cat 048 + Cat 034) were previously treated as monolithic single-record blocks. | Validated multi-record extraction in integration tests, ensuring both intra-block and inter-block plot/track decoding are supported. | Eurocontrol Part 4 §3.1 |

---

## 3. Verification & Compliance Evidence

- **Unit Tests:** `cargo test --package dfdl-example --lib` (5/5 passed).
- **Integration Tests:** `cargo test --package dfdl-example --test integration_tests` (7/7 passed).
  - `test_cat001_synthetic_record`: Validated decoding of multiple plot/track records (Track 1042 and Track 1043) in a single Cat 001 Data Block with XML Infoset and VM parity.
  - `test_multiple_plots_tracks_in_network_packet`: Validated extraction and schema parsing of Packet 13 (427-byte UDP payload containing a 416-byte multi-track Cat 048 block and an 11-byte Cat 034 sector crossing message).
  - `test_pcap_full_validation`: 100% pass across all 120 authentic CroatiaControl blocks.
- **Static Analysis & Linting:** `cargo clippy --package dfdl-example --all-targets` clean (0 warnings, 0 errors).
- **Code Quality:** Zero `unwrap`, zero `expect`, zero `panic` in library code; zero compilation warnings.
