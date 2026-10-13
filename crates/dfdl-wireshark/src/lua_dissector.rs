//! Generic Wireshark Lua Dissector Plugin Generator.
//!
//! Generates a Wireshark Lua plugin script that integrates directly with Wireshark
//! GUI and TShark (`tshark -X lua_script:dfdl_dissector.lua -r capture.pcap`).
//!
//! Enables loading and switching DFDL schemas directly from within Wireshark via:
//! - Wireshark Preferences: `Edit -> Preferences -> Protocols -> DFDL`
//! - Field registration with overlay tooltips (`ProtoField` descriptions)
//! - Clean, compact tree formatting with zero cluttered inline documentation text.

use crate::schema_meta::{FieldMetadata, SchemaMetadataRegistry};

/// Configuration parameters for generating a generic Wireshark Lua dissector.
#[derive(Debug, Clone)]
pub struct LuaDissectorConfig {
    /// Protocol short identifier (e.g. `dfdl_asterix`, `dfdl`).
    pub protocol_id: String,
    /// Protocol descriptive title shown in Wireshark packet summary.
    pub protocol_title: String,
    /// Default path to the DFDL XML schema file (`.dfdl.xsd`).
    pub schema_path: String,
    /// List of default UDP ports to bind to in Wireshark.
    pub udp_ports: Vec<u16>,
    /// Optional path to the native `libdfdl_wireshark.dylib` / `.so` library.
    pub dylib_path: Option<String>,
    /// Optional path to the `dfdl-wireshark` CLI executable.
    pub cli_bin_path: Option<String>,
}

impl LuaDissectorConfig {
    /// Creates a default configuration from a protocol name and schema path.
    #[must_use]
    pub fn new(protocol_name: &str, schema_path: &str) -> Self {
        let clean_id = protocol_name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();

        let pid = if clean_id.is_empty() || clean_id == "dfdl" || clean_id.starts_with("asterix") {
            "dfdl".to_string()
        } else if clean_id.starts_with("dfdl") {
            clean_id
        } else {
            format!("dfdl_{clean_id}")
        };

        let p_title = if protocol_name.is_empty() || protocol_name == "dfdl" || protocol_name.to_lowercase().starts_with("asterix") {
            "DFDL (DFDL Engine)".to_string()
        } else {
            format!("{protocol_name} (DFDL Engine)")
        };

        Self {
            protocol_id: pid,
            protocol_title: p_title,
            schema_path: schema_path.to_string(),
            udp_ports: Vec::new(),
            dylib_path: None,
            cli_bin_path: None,
        }
    }
}

/// Generates a standalone, preference-driven Wireshark Lua dissector script with overlay tooltips.
#[must_use]
pub fn generate_lua_dissector(config: &LuaDissectorConfig) -> String {
    let ports_str = if config.udp_ports.is_empty() {
        "21111-22135".to_string()
    } else {
        config
            .udp_ports
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };

    let mut proto_fields_lua = String::new();
    let mut fields_list_lua = String::new();

    if let Ok(content) = std::fs::read_to_string(&config.schema_path) {
        let registry = SchemaMetadataRegistry::from_schema_xml(&content);
        for (_name, meta) in registry.iter() {
            let field_label = if !meta.description.is_empty() && meta.description != meta.name {
                format!("{} ({})", meta.description, meta.name)
            } else {
                meta.name.clone()
            };
            let tooltip = build_tooltip_escaped(meta);
            proto_fields_lua.push_str(&format!(
                "    [\"{}\"] = ProtoField.string(\"{}.{}\", \"{}\", base.NONE, nil, 0, \"{}\"),\n",
                meta.name,
                config.protocol_id,
                meta.name,
                escape_lua_string(&field_label),
                escape_lua_string(&tooltip)
            ));
            if !fields_list_lua.is_empty() {
                fields_list_lua.push_str(", ");
            }
            fields_list_lua.push_str(&format!("proto_fields[\"{}\"]", meta.name));
        }
    }

    let cli_bin_str = config
        .cli_bin_path
        .as_deref()
        .unwrap_or("/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark");

    let dylib_candidates_lua = if let Some(ref path) = config.dylib_path {
        format!(
            r#"    "{path}",
    "/Users/jean-baptiste/AI-DFDL/target/release/libdfdl_wireshark.dylib",
    "dfdl-wireshark.so",
    "libdfdl_wireshark.dylib",
    "libdfdl_wireshark.so""#
        )
    } else {
        r#"    "/Users/jean-baptiste/AI-DFDL/target/release/libdfdl_wireshark.dylib",
    "dfdl-wireshark.so",
    "libdfdl_wireshark.dylib",
    "libdfdl_wireshark.so""#
            .to_string()
    };

    format!(
        r#"-- ============================================================================
-- Generic DFDL Wireshark Dissector
-- Allows loading and changing DFDL schemas from within Wireshark GUI
-- Configure via: Edit -> Preferences -> Protocols -> {title}
-- ============================================================================

local proto = Proto("{proto_id}", "{title}")

-- User Preferences (Accessible inside Wireshark: Edit -> Preferences -> Protocols)
proto.prefs.schema_path = Pref.string("DFDL Schema Path (.dfdl.xsd)", "{schema}", "Absolute path to the DFDL XML schema file (.dfdl.xsd)")
proto.prefs.ports = Pref.range("UDP Port(s)", "{ports_str}", "UDP ports to dissect with this DFDL schema", 65535)
proto.prefs.cli_path = Pref.string("dfdl-wireshark Binary", "{cli_bin_str}", "Command name or full path to dfdl-wireshark executable")

-- Dynamically registered fields with overlay tooltips
local proto_fields = {{
{proto_fields_lua}}}
proto.fields = {{ {fields_list_lua} }}

local native_dfdl = nil
local dylib_candidates = {{
{dylib_candidates_lua}
}}
for _, cand in ipairs(dylib_candidates) do
    local dylib_loader = package.loadlib(cand, "luaopen_dfdl_wireshark")
    if dylib_loader then
        local ok, mod = pcall(dylib_loader)
        if ok and mod then
            native_dfdl = mod
            break
        end
    end
end

-- Lightweight pure Lua JSON decoder (zero external dependencies)
local function parse_json_value(str, idx)
    idx = string.find(str, "%S", idx) or idx
    local c = string.sub(str, idx, idx)
    if c == '"' then
        local j = idx + 1
        local out = {{}}
        while j <= #str do
            local ch = string.sub(str, j, j)
            if ch == '\\' then
                local esc = string.sub(str, j + 1, j + 1)
                if esc == '"' or esc == '\\' or esc == '/' then
                    table.insert(out, esc)
                    j = j + 2
                elseif esc == 'n' then
                    table.insert(out, '\n')
                    j = j + 2
                elseif esc == 'r' then
                    table.insert(out, '\r')
                    j = j + 2
                elseif esc == 't' then
                    table.insert(out, '\t')
                    j = j + 2
                elseif esc == 'u' then
                    local hex_str = string.sub(str, j + 2, j + 5)
                    local cp = tonumber(hex_str, 16)
                    if cp then
                        if cp < 128 then
                            table.insert(out, string.char(cp))
                        else
                            table.insert(out, utf8.char(cp))
                        end
                    end
                    j = j + 6
                else
                    table.insert(out, esc)
                    j = j + 2
                end
            elseif ch == '"' then
                return table.concat(out), j + 1
            else
                table.insert(out, ch)
                j = j + 1
            end
        end
        return table.concat(out), j
    elseif c == '{{' then
        local obj = {{}}
        idx = idx + 1
        while idx <= #str do
            idx = string.find(str, "%S", idx) or idx
            if string.sub(str, idx, idx) == '}}' then return obj, idx + 1 end
            local key, next_i = parse_json_value(str, idx)
            idx = string.find(str, ":", next_i) + 1
            local val, after_val = parse_json_value(str, idx)
            obj[key] = val
            idx = string.find(str, "%S", after_val) or after_val
            if string.sub(str, idx, idx) == ',' then
                idx = idx + 1
            elseif string.sub(str, idx, idx) == '}}' then
                return obj, idx + 1
            end
        end
        return obj, idx
    elseif c == '[' then
        local arr = {{}}
        idx = idx + 1
        while idx <= #str do
            idx = string.find(str, "%S", idx) or idx
            if string.sub(str, idx, idx) == ']' then return arr, idx + 1 end
            local val, next_i = parse_json_value(str, idx)
            table.insert(arr, val)
            idx = string.find(str, "%S", next_i) or next_i
            if string.sub(str, idx, idx) == ',' then
                idx = idx + 1
            elseif string.sub(str, idx, idx) == ']' then
                return arr, idx + 1
            end
        end
        return arr, idx
    elseif string.find(c, "[%d%-]") then
        local _, stop, num = string.find(str, "([%d%.%-%+eE]+)", idx)
        return tonumber(num) or num, stop + 1
    elseif string.sub(str, idx, idx + 3) == "true" then
        return true, idx + 4
    elseif string.sub(str, idx, idx + 4) == "false" then
        return false, idx + 5
    elseif string.sub(str, idx, idx + 3) == "null" then
        return nil, idx + 4
    end
    return nil, idx + 1
end

local function decode_json(s)
    if not s or s == "" then return nil end
    local val, _ = parse_json_value(s, 1)
    return val
end

-- Recursively populates Wireshark tree nodes with overlay tooltips
local function populate_node(tree_item, tvb, node)
    if not node then return end

    local sub_item
    local pf = proto_fields[node.name]

    if pf and (not node.children or #node.children == 0) then
        local display_val = node.converted
        if not display_val or display_val == "" then
            display_val = node.raw or ""
        end
        sub_item = tree_item:add(pf, tvb(0, 0), tostring(display_val))
        local item_label = node.label
        if not item_label or item_label == "" then
            local prefix = node.name
            if node.description and node.description ~= "" and node.description ~= node.name then
                prefix = string.format("%s (%s)", node.description, node.name)
            end
            item_label = string.format("%s: %s", prefix, tostring(display_val))
        end
        sub_item:set_text(item_label)
    else
        local label = node.label or node.name or "Item"
        if (not node.label or node.label == node.name) and node.description and node.description ~= "" and node.description ~= node.name then
            label = string.format("%s (%s)", node.description, node.name)
        end
        sub_item = tree_item:add(proto, tvb(), label)
    end

    if node.children and #node.children > 0 then
        for _, child in ipairs(node.children) do
            populate_node(sub_item, tvb, child)
        end
    end
end

-- Resolves the effective schema path from Preferences, environment, or hardcoded default
local function get_effective_schema()
    local p = proto.prefs.schema_path
    if p and p ~= "" then
        local f = io.open(p, "r")
        if f then
            f:close()
            return p
        end
    end
    local env_p = os.getenv("DFDL_SCHEMA")
    if env_p and env_p ~= "" then
        local f = io.open(env_p, "r")
        if f then
            f:close()
            return env_p
        end
    end
    return "{schema}"
end

-- Resolves the CLI binary executable path
local function find_cli_bin()
    local p = proto.prefs.cli_path
    if p and p ~= "" and p ~= "dfdl-wireshark" then
        local f = io.open(p, "r")
        if f then
            f:close()
            return p
        end
    end
    local candidates = {{
        "{cli_bin_str}",
        "/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark",
        "/usr/local/bin/dfdl-wireshark",
        "/opt/homebrew/bin/dfdl-wireshark",
        "/Users/jean-baptiste/AI-DFDL/target/debug/dfdl-wireshark",
    }}
    for _, cand in ipairs(candidates) do
        local f = io.open(cand, "r")
        if f then
            f:close()
            return cand
        end
    end
    return "dfdl-wireshark"
end

-- Main protocol dissector function
function proto.dissector(tvb, pinfo, tree)
    local length = tvb:len()
    if length == 0 then return end

    local payload_tvb = tvb
    if length > 42 then
        local b12 = tvb(12, 1):uint()
        local b13 = tvb(13, 1):uint()
        if b12 == 0x08 and b13 == 0x00 then
            local ip_proto = tvb(23, 1):uint()
            if ip_proto == 17 then
                local ihl = (tvb(14, 1):uint() % 16) * 4
                local udp_offset = 14 + ihl + 8
                if length >= udp_offset then
                    payload_tvb = tvb(udp_offset)
                end
            end
        end
    end

    pinfo.cols.protocol = "{proto_id}"
    local active_schema = get_effective_schema()
    if not active_schema then
        local root_tree = tree:add(proto, payload_tvb(), "{title}: [No Schema Loaded]")
        root_tree:add(proto, payload_tvb(), "Open Edit -> Preferences -> Protocols -> {title} to select a .dfdl.xsd schema file")
        return
    end

    local hex_data = tostring(payload_tvb:bytes():tohex())
    local json_str = nil

    if native_dfdl and native_dfdl.dissect_hex then
        json_str = native_dfdl.dissect_hex(active_schema, hex_data)
    else
        local cli = find_cli_bin()
        local cmd = string.format("%s dissect-hex --schema '%s' %s 2>/dev/null", cli, active_schema, hex_data)
        local handle = io.popen(cmd)
        if handle then
            json_str = handle:read("*a")
            handle:close()
        end
    end

    local tree_data = decode_json(json_str)
    if tree_data and tree_data.root then
        local root_tree = tree:add(proto, payload_tvb(), string.format("{title} (%d bytes)", payload_tvb:len()))
        populate_node(root_tree, payload_tvb, tree_data.root)
    end
end

-- Dynamic port table binding based on Preferences
local registered_ports = {{}}

local function update_port_bindings()
    local udp_table = DissectorTable.get("udp.port")
    for _, p in ipairs(registered_ports) do
        udp_table:remove(p, proto)
    end
    registered_ports = {{}}

    local range_str = tostring(proto.prefs.ports or "{ports_str}")
    for part in string.gmatch(range_str, "[^,]+") do
        local start_p, end_p = string.match(part, "(%d+)%s*-%s*(%d+)")
        if start_p and end_p then
            for p = tonumber(start_p), tonumber(end_p) do
                udp_table:add(p, proto)
                table.insert(registered_ports, p)
            end
        else
            local single = tonumber(string.match(part, "%d+"))
            if single then
                udp_table:add(single, proto)
                table.insert(registered_ports, single)
            end
        end
    end
end

function proto.prefs_changed()
    update_port_bindings()
end

update_port_bindings()
register_postdissector(proto)
"#,
        title = config.protocol_title,
        schema = config.schema_path,
        proto_id = config.protocol_id,
        ports_str = ports_str,
        proto_fields_lua = proto_fields_lua,
        fields_list_lua = fields_list_lua,
        dylib_candidates_lua = dylib_candidates_lua,
        cli_bin_str = cli_bin_str,
    )
}

/// Builds an escaped, single-line tooltip description from field metadata.
fn build_tooltip_escaped(meta: &FieldMetadata) -> String {
    let mut parts = Vec::new();
    if !meta.description.is_empty() {
        parts.push(meta.description.clone());
    }
    if !meta.item_ref.is_empty() {
        parts.push(format!("Item: {}", meta.item_ref));
    }
    if !meta.unit.is_empty() {
        parts.push(format!("Unit: {}", meta.unit));
    }
    if !meta.resolution_str.is_empty() {
        parts.push(format!("Resolution: {}", meta.resolution_str));
    }
    if !meta.lsb_str.is_empty() {
        parts.push(format!("LSB: {}", meta.lsb_str));
    }
    if !meta.documentation.is_empty() && meta.documentation != meta.description {
        let clean_doc = meta.documentation.replace(['\n', '\r'], " ");
        parts.push(clean_doc);
    }
    parts.join(" | ")
}

/// Escapes characters for safe inclusion inside a Lua double-quoted string literal.
fn escape_lua_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push(' '),
            '\r' => {},
            '\t' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}
