# Expert Domain Review: Clean Tree Formatting and Overlay Tooltips in Wireshark

**Date:** 2026-10-13 21:24:00 CEST  
**Component:** `crates/dfdl-wireshark` & Wireshark Lua Dissector Engine  
**Objective:** Transform cluttered tree rows with inline field descriptions into clean, standard Wireshark tree nodes with rich, overlay tooltips (`ProtoField` descriptions).

---

## 1. UX / UI Domain Expertise (Wireshark Qt & Tree Pane Ergonomics)

### Problem Addressed
Previously, tree nodes were formatted with parenthetical descriptions and raw values in the item label, e.g.:
```text
timeOfDay (Time of Day (UTC since midnight)): 27354.602 s (07:35:54.602 UTC) [3501389]
    [Doc: Time of Day (UTC since midnight) | Item: I034/030, I048/140 | Unit: seconds ...]
category: 48 dimensionless
sac: 25 code
sic: 201 code
trackNumber: 3563 track_id
targetDescriptor: 160 bitfield
```
This severely degraded visual ergonomics in the Wireshark GUI:
- Row height and line wraps in the Packet Details tree made browsing difficult.
- Pseudo-units such as `code`, `dimensionless`, `bitfield`, `track_id` polluted the numeric values.
- Hover tooltips on element nodes (`timeOfDay`, `rho`, `theta`, etc.) were empty because element declarations were decoupled from type annotations during Lua field generation.

### Solution & Improvements
1. **Clean Tree Presentation:**
   - Leaf items are formatted strictly as: `<name>: <formatted_value>` (e.g. `timeOfDay: 27354.602 s (07:35:54.602 UTC)`, `rho: 197.684 NM (366.11 km)`, `sac: 25`, `category: 48`).
   - Group/parent nodes are formatted cleanly by structural name (`AsterixPacket`, `AsterixRecord`, `Cat048Record`, `record`).
   - Pseudo-units (`dimensionless`, `code`, `bitfield`, `track_id`, etc.) are filtered out of the tree values and retained exclusively in metadata.
2. **Native Overlay Tooltips (`ProtoField` Descriptions):**
   - The type annotations (`description`, `item_ref`, `unit`, `resolution_str`, `lsb_str`, and schema documentation) are automatically propagated from `simpleType` definitions to the linked `<xs:element>` instances.
   - Every `ProtoField` is generated with its `desc` parameter containing:
     `Description | Item: <Ref> | Unit: <Unit> | Resolution: <Res> | LSB: <LSB> | <Doc>`
   - In Wireshark Qt GUI, hovering over any row in the tree displays this overlay tooltip popup, and clicking the row displays it in the bottom status bar.

---

## 2. Signal Processing & Radar Domain Expertise (ASTERIX / Target Reports)

1. **6-bit ICAO Callsign Unpacking (Item 240 / `aircraftId`):**
   - Radar plot/track aircraft identification in ASTERIX Category 048 Item 240 encodes 8 characters into 6 octets using 6-bit IA-5 character packing.
   - The converter now automatically unpacks 48-bit payloads into human-readable callsigns (`DLH65A`), preventing raw non-printable byte sequences (`\x10Â6Ô\x18`) from appearing in the tree.
2. **Physical Engineering Conversions Preserved:**
   - Polar coordinates: `NM` (with metric `km`), `degrees` (with `°`).
   - Ground velocity: `kt` (knots) with `NM/s` and `m/s`.
   - Time of Day: seconds with elapsed UTC time `(HH:MM:SS.mmm UTC)`.
   - Flight levels: `FL` with altitude in feet.
   - Mode 3/A: 4-digit octal reply representation.

---

## 3. Cyber & System Safety Expertise

1. **Memory & Slicing Safety:**
   - Packed callsign decoding uses exact array pattern destructuring (`<&[u8; 6]>::try_from(bytes)`) and precomputed shift tables `[42, 36, 30, 24, 18, 12, 6, 0]`.
   - All arithmetic uses saturating operations (`saturating_add`, `saturating_sub`), strictly avoiding panics, out-of-bounds indexing, and arithmetic overflows.
2. **Unicode Escape Injection Resistance:**
   - The embedded Lua JSON decoder safely handles `\uXXXX` unicode sequences, mapping codepoints `< 128` to `string.char` and multi-byte codepoints to `utf8.char`, preventing parser corruption on non-ASCII telemetry.

---

## 4. Development & Compliance Status

- `cargo clippy --package dfdl-wireshark --all-targets`: **0 warnings** under strict denial rules (`unwrap_used = "deny"`, `indexing_slicing = "deny"`, `arithmetic_side_effects = "deny"`).
- `cargo test --release --package dfdl-wireshark`: **All 3 tests passed** (0 failures).
- Real PCAP packet verification via `tshark`: **100% verified** against live ASTERIX capture `cat_034_048.pcap`.
