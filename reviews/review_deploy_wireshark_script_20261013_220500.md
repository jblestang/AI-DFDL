# Domain Review: Automated Wireshark Deployment Script & Out-of-the-Box Dissector

**Timestamp**: 2026-10-13T22:05:00+02:00  
**Context**: Added [`scripts/deploy_wireshark.sh`](file:///Users/jean-baptiste/AI-DFDL/scripts/deploy_wireshark.sh), hardcoded default canonical paths to the release binary and default ASTERIX DFDL schema, and verified out-of-the-box Wireshark / TShark dissection.

---

## 1. Domain & Tooling Expertise (Wireshark Dissectors & Air Traffic Radar Data)

### 1.1 Out-of-the-Box Experience
- **Zero-Configuration Startup**:
  - Wireshark automatically scans its Personal Lua Plugins directory (`~/.local/lib/wireshark/plugins/` on macOS/Linux) on launch and executes any `.lua` scripts found there.
  - Previously, opening Wireshark required the user to manually set the schema path in Preferences or specify `-X lua_script:...` flags.
  - With the updated generator, the Lua dissector script (`dfdl_dissector.lua`) now embeds the absolute canonical paths:
    1. Default schema: `/Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd`
    2. Release executable: `/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark`
    3. Native shared library: `/Users/jean-baptiste/AI-DFDL/target/release/libdfdl_wireshark.dylib` (and installed `dfdl-wireshark.so`)
  - If a user opens Wireshark and loads an ASTERIX packet capture on UDP ports 21111-22135, it immediately dissects all records without requiring any configuration.

### 1.2 Multi-Tier Execution Fallback
- The generated Lua script implements resilient fallback logic:
  - **Tier 1 (In-Process Native C ABI)**: Attempts to load `dfdl-wireshark.so` or `libdfdl_wireshark.dylib` via `package.loadlib` inside a `pcall`. If successful, dissection occurs with zero process-spawn overhead.
  - **Tier 2 (Out-of-Process CLI Engine)**: If native loading is unavailable (e.g. Wireshark sandbox restrictions or missing symbols), it invokes the compiled release binary (`dfdl-wireshark dissect-hex`) via `io.popen`.
  - **Tier 3 (Schema Preference & Dynamic Override)**: While the default ASTERIX schema is hardcoded for immediate out-of-the-box operation, the user can still navigate to `Wireshark -> Settings / Preferences -> Protocols -> DFDL (DFDL Engine)` and point to any other custom DFDL schema at runtime.

---

## 2. Deployment Script Architecture ([`scripts/deploy_wireshark.sh`](file:///Users/jean-baptiste/AI-DFDL/scripts/deploy_wireshark.sh))

1. **Deterministic Discovery**:
   - Computes repository root via `$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)`.
   - Locates `tshark` executable via `command -v tshark` or `/Applications/Wireshark.app/Contents/MacOS/tshark`.
   - Queries `tshark -G folders` to dynamically identify the user's `Personal Lua Plugins:` directory, falling back to `${HOME}/.local/lib/wireshark/plugins`.
2. **Automated Compilation & Library Installation**:
   - Runs `cargo build --release --package dfdl-wireshark`.
   - Installs the compiled shared library as both `dfdl-wireshark.so` and `libdfdl_wireshark.dylib` into the Wireshark plugins directory.
3. **Canonical Path Embedding**:
   - Calls `target/release/dfdl-wireshark generate-lua` with explicit `--schema`, `--cli-bin`, and `--dylib` arguments.
   - Installs the generated Lua file directly into the Wireshark plugin folder and syncs a copy to `crates/dfdl-wireshark/dfdl_dissector.lua`.
4. **Automated Smoke Test Verification**:
   - Executes `tshark -r cat_034_048.pcap -c 1 -V` without any special flags to confirm that Wireshark's auto-discovery dissects the packet cleanly out of the box.

---

## 3. Test & Verification Matrix

| Test Suite | Command | Result |
|:---|:---|:---|
| DFDL Core & Engine | `cargo test --package dfdl-core` | Passed (100%) |
| DFDL Example & ASTERIX PCAP | `cargo test --package dfdl-example` | Passed (15/15 tests) |
| DFDL Wireshark Dissector Engine | `cargo test --package dfdl-wireshark` | Passed (3/3 tests) |
| Clippy Strict Linter | `cargo clippy --package dfdl-core --package dfdl-wireshark --package dfdl-example --all-targets` | 0 warnings |
| Deployment Script | `./scripts/deploy_wireshark.sh` | Success (All 5 stages completed) |
| Default Out-of-the-Box TShark | `/Applications/Wireshark.app/Contents/MacOS/tshark -r cat_034_048.pcap -c 1 -V` | Verified automatic dissection with bitfields and calibrated units |
