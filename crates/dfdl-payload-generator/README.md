# dfdl-payload-generator

Schema-driven DFDL binary payload synthesizer and PCAP capture generator.

`dfdl-payload-generator` generates authentic binary test payloads and standard Libpcap captures (v2.4) conforming to or violating any compiled DFDL 1.0 schema. It is **100% schema-agnostic**, dynamically inspecting schema definitions to discover choice discriminators, bounds, enumerations, and length prefixes without hardcoded assumptions.

---

## Features

- **Valid Payload Generation**:
  - Nominal valid payloads and sequences.
  - Exhaustive branch coverage (every `dfdl:choiceBranchKey` discovered in the schema).
  - Facet boundary extremes (min/max inclusive/exclusive limits).
  - Variable array occurrence counts.
- **Syntactic Anomaly Injection**:
  - Truncation (`cut_bytes`).
  - Trailing garbage and buffer overflow (`garbage_bytes`).
  - Dynamic declared header/prefix length mismatches (`field_offset`, `field_width`, `delta`).
  - Bitstream misalignment ($1 \dots 7$ extra bits).
  - Unmatched choice dispatch key (`WrongDispatchKey` causing syntactic branch resolution failure).
- **Semantic Anomaly Injection**:
  - Facet boundary violations (values exceeding min/max ranges).
  - Enumeration violations (values outside `xs:enumeration` restrictions).
  - Wrong constant / fixed field values (`WrongConstantValue` violating fixed values or schema defaults).
- **Network Encapsulation & PCAP Export**:
  - Standard Libpcap file format (v2.4, microsecond precision, magic `0xa1b2c3d4`).
  - IEEE 802.3 Ethernet II + IETF RFC 791 IPv4 + RFC 768 UDP framing with valid checksums.
  - Raw unencapsulated framing (`TransportProtocol::Raw`).
- **Configurable Budgets & Limits**:
  - Maximum message count (`max_messages`).
  - Strict total generated capture size budget in bytes (`max_total_size`).
  - Maximum individual payload size (`max_payload_size`).
  - Execution metrics via `GenerationReport`.

---

## Installation

Add `dfdl-payload-generator` to your `Cargo.toml`:

```toml
[dependencies]
dfdl-payload-generator = { path = "crates/dfdl-payload-generator" }
dfdl-core = { path = "crates/dfdl-core" }
```

---

## Quick Start

### 1. Generating an Exclusively Valid PCAP File

To generate a PCAP file containing only valid, schema-compliant packets:

```rust
use std::fs::File;
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, TransportProtocol,
};

fn generate_valid_capture(schema: &dfdl_core::schema::ir::CompiledSchema) -> Result<(), Box<dyn std::error::Error>> {
    let seed = 42;
    let mut generator = PayloadGenerator::new(schema, seed);

    // Network configuration (Ethernet + IPv4)
    let net_cfg = NetworkConfig {
        src_mac: [0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E],
        dst_mac: [0x00, 0x50, 0x56, 0xC0, 0x00, 0x01],
        src_ip: [192, 168, 1, 10],
        dst_ip: [192, 168, 1, 20],
    };

    // Transport layer: UDP port 5602
    let transport = TransportProtocol::Udp {
        src_port: 5602,
        dst_port: 5602,
    };

    // Option A: Write an exhaustive valid suite (all choice branches and facet extremes)
    let mut file_suite = File::create("valid_suite.pcap")?;
    generator.write_valid_pcap(&mut file_suite, 0, &net_cfg, &transport)?;

    // Option B: Write a sequential stream of 100 valid packets
    let mut file_stream = File::create("valid_stream.pcap")?;
    let report = generator.write_valid_pcap(&mut file_stream, 100, &net_cfg, &transport)?;
    println!("Packets written: {}, Total bytes: {}", report.message_count, report.total_bytes);

    Ok(())
}
```

---

### 2. Generating an Invalid PCAP File (Test Suite with Anomalies)

To generate test PCAP captures containing syntactic and semantic anomalies for fuzzing or dissector verification:

```rust
use std::fs::File;
use dfdl_payload_generator::{
    NetworkConfig, PayloadGenerator, SyntaxAnomaly, SemanticAnomaly, TransportProtocol,
};
use dfdl_core::infoset::value::DfdlValue;

fn generate_anomaly_capture(schema: &dfdl_core::schema::ir::CompiledSchema) -> Result<(), Box<dyn std::error::Error>> {
    let mut generator = PayloadGenerator::new(schema, 42);
    let net_cfg = NetworkConfig::default();
    let transport = TransportProtocol::default();

    // Option A: Generate the full exhaustive test suite
    // Includes choice branches, facet bounds, truncations, length mismatches, bit slips, invalid enums, and unmatched keys.
    let suite = generator.generate_exhaustive_suite()?;
    let mut file = File::create("anomalies_exhaustive.pcap")?;
    generator.write_pcap(&mut file, &suite, &net_cfg, &transport)?;

    // Option B: Generate individual targeted anomalies
    let mut targeted_payloads = Vec::new();

    // 1. Truncated packet (-4 bytes)
    targeted_payloads.push(
        generator.generate_syntax_anomaly(SyntaxAnomaly::Truncated { cut_bytes: 4 })?
    );

    // 2. Trailing garbage (+16 bytes)
    targeted_payloads.push(
        generator.generate_syntax_anomaly(SyntaxAnomaly::TrailingGarbage { garbage_bytes: 16 })?
    );

    // 3. Bit misaligned (+3 extra bits)
    targeted_payloads.push(
        generator.generate_syntax_anomaly(SyntaxAnomaly::BitMisaligned { extra_bits: 3 })?
    );

    // 4. Semantic: out-of-bounds facet value
    targeted_payloads.push(
        generator.generate_semantic_anomaly(SemanticAnomaly::FacetOutOfBounds {
            target_name: "version".to_string(),
            invalid_value: DfdlValue::Long(9999),
        })?
    );

    // Write targeted anomalies to PCAP
    let mut custom_file = File::create("anomalies_targeted.pcap")?;
    generator.write_pcap(&mut custom_file, &targeted_payloads, &net_cfg, &transport)?;

    Ok(())
}
```

---

## Configuration Options

### Generation Limits (`GenerationLimits`)

You can control generation volume, cap message counts, and enforce strict byte size limits:

```rust
use dfdl_payload_generator::{GenerationLimits, PayloadGenerator};

// Using fluent builder on limits
let limits = GenerationLimits::new()
    .max_messages(10)               // Cap at 10 packets max
    .max_total_size(100 * 1024)     // Strict budget of 100 KB total PCAP size
    .max_payload_size(4096);        // Individual payloads capped at 4 KB

// Configure on generator directly
let mut generator = PayloadGenerator::new(&schema, 123)
    .with_limits(limits);

// Or via mutators
generator.set_max_messages(Some(5));
generator.set_max_total_size(Some(60_000));
```

### Generation Report (`GenerationReport`)

`write_pcap` and `write_valid_pcap` return a `GenerationReport` detailing the outcome:

```rust
let report = generator.write_valid_pcap(&mut file, 50, &net_cfg, &transport)?;

println!("Packets emitted: {}", report.message_count);
println!("Total PCAP bytes: {}", report.total_bytes);
println!("Hit message limit: {}", report.reached_message_limit);
println!("Hit size budget limit: {}", report.reached_size_limit);
```

### Network Configuration (`NetworkConfig`)

```rust
use dfdl_payload_generator::NetworkConfig;

let net_cfg = NetworkConfig {
    src_mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
    dst_mac: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
    src_ip: [10, 0, 0, 1],
    dst_ip: [10, 0, 0, 2],
};
```

### Transport Protocol (`TransportProtocol`)

```rust
use dfdl_payload_generator::TransportProtocol;

// UDP encapsulation (adds Ethernet 14B + IPv4 20B + UDP 8B framing)
let udp = TransportProtocol::Udp {
    src_port: 5602,
    dst_port: 5602,
};

// Raw link-layer / direct payload
let raw = TransportProtocol::Raw;
```

---

## Command-Line Interface (`dfdl-pcap-gen`)

The crate provides a standalone CLI binary, `dfdl-pcap-gen`, allowing you to generate and customize PCAP captures directly from the terminal.

### Usage

```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- [OPTIONS]
```

### CLI Arguments Reference

| Option | Short | Argument | Default | Description |
| :--- | :---: | :--- | :--- | :--- |
| `--schema` | | `<PATH>` | Built-in STANAG 5602 | Path to a DFDL XML Schema file (`.dfdl.xsd` or `.xsd`). |
| `--root` | | `<NAME>` | Root in schema | Target root element name to compile (e.g. `SimplePacket`, `SimpleControlPayload`). |
| `--output` | `-o` | `<PATH>` | `capture.pcap` | Output destination path for the generated Libpcap file. |
| `--mode` | | `<MODE>` | `valid` | Generation mode: `valid`, `exhaustive`, `syntax`, or `semantic`. |
| `--count` | `-n` | `<COUNT>` | `10` (`0` for full suite) | Number of messages/packets to generate. In `exhaustive`/`syntax`/`semantic` modes with `-n`, cycles through the test suite up to COUNT. If omitted, generates the complete suite once. |
| `--max-size` | | `<BYTES>` | Unlimited | Strict maximum total generated PCAP file size in bytes. Generation stops before exceeding this budget. |
| `--max-payload-size` | | `<BYTES>` | Unlimited | Maximum allowed size in bytes for an individual payload (payloads exceeding this are capped). |
| `--transport` | | `<PROTO>` | `udp` | Transport layer framing: `udp` (Ethernet II + IPv4 + UDP) or `raw` (raw link layer). |
| `--port` | | `<PORT>` | `5602` | UDP source and destination transport port. |
| `--src-ip` | | `<IPV4>` | `192.168.1.10` | Source IPv4 address. |
| `--dst-ip` | | `<IPV4>` | `192.168.1.20` | Destination IPv4 address. |
| `--src-mac` | | `<MAC>` | `00:1A:2B:3C:4D:5E` | Source MAC address (colon or hyphen-separated hex). |
| `--dst-mac` | | `<MAC>` | `00:50:56:C0:00:01` | Destination MAC address (colon or hyphen-separated hex). |
| `--seed` | | `<U64>` | `42` | PRNG seed for deterministic test reproducibility. |
| `--help` | `-h` | | | Display CLI help documentation and exit. |

---

### CLI Customization Examples

#### 1. Generate an Exclusively Valid PCAP with Custom Count
Generate 25 purely valid packets:
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    -o valid_capture.pcap \
    -n 25
```

#### 2. Generate the Exhaustive Valid Branch Matrix
Cover every choice branch key, facet extreme, and array occurrence count (`-n 0`):
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --mode valid \
    -n 0 \
    -o valid_exhaustive.pcap
```

#### 3. Enforce a Strict Capture File Size Budget
Generate packets while ensuring the PCAP file never exceeds 50 KB (51,200 bytes):
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    -o budgeted.pcap \
    -n 50 \
    --max-size 51200
```

#### 4. Stream 5000 Exhaustive Test Packets under a Size Budget
Cycle through the exhaustive test suite to produce up to 5,000 packets capped at 100 KB total capture size:
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --mode exhaustive \
    -o budgeted.pcap \
    -n 5000 \
    --max-size 100000
```

#### 5. Restrict Individual Payload Size and Total Capture Size
Cap each payload to 200 bytes and the total capture file to 1,200 bytes:
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --mode exhaustive \
    -o budgeted.pcap \
    -n 5000 \
    --max-payload-size 200 \
    --max-size 1200
```

#### 6. Generate an Exhaustive Anomaly Test Suite (Wireshark / Fuzzing)
Generate a capture containing the complete test matrix (valid nominals, choice branches, syntax corruptions, and semantic anomalies):
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --mode exhaustive \
    -o test_matrix.pcap
```

#### 7. Generate from a Custom External DFDL Schema
Compile a user DFDL schema and generate PCAP packets for root element `RadarTrackMessage`:
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --schema /path/to/radar_protocol.dfdl.xsd \
    --root RadarTrackMessage \
    -o radar_tracks.pcap \
    -n 15
```

#### 8. Customize Network Addressing and Ports
Configure specific MAC addresses, IP addresses, and UDP port 50000:
```bash
cargo run -p dfdl-payload-generator --bin dfdl-pcap-gen -- \
    --src-mac 00:11:22:33:44:55 \
    --dst-mac AA:BB:CC:DD:EE:FF \
    --src-ip 10.0.0.1 \
    --dst-ip 10.0.0.2 \
    --port 50000 \
    -o custom_network.pcap
```

---

## Running Examples & Verification

To run the NATO STANAG 5602 (SIMPLE) PCAP generator example:

```bash
cargo run -p dfdl-payload-generator --example generate_stanag5602_pcap
```

This generates:
1. `stanag5602_valid.pcap` — 10 purely valid STANAG 5602 packets.
2. `stanag5602_test_suite.pcap` — 28 packets covering valid branches, syntax corruptions, and semantic anomalies.
3. `stanag5602_count_capped.pcap` — Message-capped capture (max 5 packets).
4. `stanag5602_budget_constrained.pcap` — Size-budgeted capture ($\le$ 60,000 bytes).

To run the complete test suite:

```bash
cargo run -p dfdl-payload-generator --example run_generator_tests
```

All 19 integration and unit tests will execute and verify payload parsing, checksums, and PCAP format integrity.

