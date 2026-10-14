//! Generic Wireshark Lua Dissector Plugin Generator.
//!
//! Generates a 100% schema-agnostic Wireshark Lua plugin script that integrates
//! directly with Wireshark GUI and TShark (`tshark -X lua_script:dfdl_dissector.lua -r capture.pcap`).
//!
//! Enables loading and switching DFDL schemas directly from within Wireshark via:
//! - Wireshark Preferences: `Edit -> Preferences -> Protocols -> DFDL`
//! - Dynamic field registration from the loaded XSD: zero hardcoded fields or protocol rules.
//! - Interactive overlay tooltips (`ProtoField` descriptions) and clean, uncluttered tree items.

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

/// Generates a standalone, preference-driven Wireshark Lua dissector script with dynamic schema introspection.
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
--
-- Completely schema-agnostic: zero hardcoded fields or protocol rules.
-- Field names, labels, descriptions, and tooltips are retrieved dynamically
-- from the configured DFDL XML Schema (.dfdl.xsd).
-- ============================================================================

local proto = Proto("{proto_id}", "{title}")

-- User Preferences (Accessible inside Wireshark: Edit -> Preferences -> Protocols)
proto.prefs.schema_path = Pref.string("DFDL Schema Path (.dfdl.xsd)", "{schema}", "Absolute path to the DFDL XML schema file (.dfdl.xsd)")
proto.prefs.ports = Pref.range("UDP Port(s)", "{ports_str}", "UDP ports to dissect with this DFDL schema", 65535)
proto.prefs.cli_path = Pref.string("dfdl-wireshark Binary", "{cli_bin_str}", "Command name or full path to dfdl-wireshark executable")

-- Dynamically registered fields table (populated from the loaded XSD at runtime)
local proto_fields = {{}}
local proto_field_types = {{}}
local fields_list = {{}}

-- Enhancement 5: Wireshark Expert Info declarations for anomalies and operational alerts
local expert_error = ProtoExpert.new("{proto_id}.expert.error", "DFDL Error", expert.group.MALFORMED, expert.severity.ERROR)
local expert_warn = ProtoExpert.new("{proto_id}.expert.warn", "DFDL Warning", expert.group.PROTOCOL, expert.severity.WARN)
local expert_note = ProtoExpert.new("{proto_id}.expert.note", "DFDL Note", expert.group.SEQUENCE, expert.severity.NOTE)
local expert_chat = ProtoExpert.new("{proto_id}.expert.chat", "DFDL Info", expert.group.PROTOCOL, expert.severity.CHAT)
proto.experts = {{ expert_error, expert_warn, expert_note, expert_chat }}

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

-- Resolves the effective schema path from Preferences, environment, or default
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

-- Dynamically registers ProtoFields by querying the schema metadata from the loaded XSD
local function register_fields_from_schema(schema_file)
    if not schema_file or schema_file == "" then return end

    local fields_json = nil
    if native_dfdl and native_dfdl.get_schema_fields then
        fields_json = native_dfdl.get_schema_fields(schema_file)
    end

    if not fields_json or fields_json == "" then
        local cli = find_cli_bin()
        local cmd = string.format("%s schema-fields --schema '%s' 2>/dev/null", cli, schema_file)
        local handle = io.popen(cmd)
        if handle then
            fields_json = handle:read("*a")
            handle:close()
        end
    end

    if not fields_json or fields_json == "" then
        -- Fast pure Lua fallback: scan element names directly from the XSD file
        local f = io.open(schema_file, "r")
        if f then
            local xsd_text = f:read("*a")
            f:close()
            for elem_name in string.gmatch(xsd_text, '<xs:element%s+name="([%w_]+)"') do
                if not proto_fields[elem_name] then
                    proto_fields[elem_name] = ProtoField.string(string.format("%s.%s", "{proto_id}", elem_name), elem_name, base.NONE, elem_name)
                    table.insert(fields_list, proto_fields[elem_name])
                end
            end
        end
        return
    end

    local fields_data = decode_json(fields_json)
    if fields_data and type(fields_data) == "table" then
        for _, item in ipairs(fields_data) do
            local name = item.name
            local path = item.path or name
            local label = item.label or name
            local tooltip = item.tooltip or ""
            local desc = tooltip
            if not desc or desc == "" then
                desc = item.description or label or name
            end
            local ptype = item.proto_type or "string"
            local filter_abbr = string.format("%s.%s", "{proto_id}", path)
            if name and not proto_fields[filter_abbr] then
                proto_field_types[path] = ptype
                proto_field_types[string.lower(path)] = ptype
                local field
                if ptype == "double" then
                    field = ProtoField.double(filter_abbr, label, desc)
                elseif ptype == "uint32" or ptype == "uint" then
                    field = ProtoField.uint32(filter_abbr, label, base.DEC, nil, 0, desc)
                elseif ptype == "int32" or ptype == "int" then
                    field = ProtoField.int32(filter_abbr, label, base.DEC, nil, 0, desc)
                elseif ptype == "bool" then
                    field = ProtoField.bool(filter_abbr, label, base.NONE, nil, 0, desc)
                else
                    field = ProtoField.string(filter_abbr, label, base.NONE, desc)
                end
                proto_fields[filter_abbr] = field
                proto_fields[string.lower(filter_abbr)] = field
                proto_fields[path] = field
                proto_fields[string.lower(path)] = field
                if not proto_fields[name] then
                    proto_fields[name] = field
                    proto_fields[string.lower(name)] = field
                    proto_field_types[name] = ptype
                    proto_field_types[string.lower(name)] = ptype
                end
                table.insert(fields_list, field)
            end
        end
    end
end

-- Dynamically load and register fields from the effective XSD at startup
register_fields_from_schema(get_effective_schema())
proto.fields = fields_list

-- Recursively populates Wireshark tree nodes with overlay tooltips and wire highlights
local function populate_node(tree_item, tvb, node)
    if not node then return end

    local item_tvb
    local tvb_len = tvb:len()
    if node.length and node.length > 0 and node.offset and (node.offset + node.length <= tvb_len) then
        item_tvb = tvb(node.offset, node.length)
    elseif node.offset and node.offset < tvb_len then
        item_tvb = tvb(node.offset, 1)
    else
        item_tvb = tvb(0, 0)
    end

    local sub_item
    local path = node.path or node.name
    local pf = proto_fields[path] or proto_fields[string.lower(path)] or proto_fields[node.name] or proto_fields[string.lower(node.name)]
    local flat_pf = proto_fields[node.name] or proto_fields[string.lower(node.name)]
    local ptype = proto_field_types[path] or proto_field_types[string.lower(path)] or proto_field_types[node.name] or proto_field_types[string.lower(node.name)] or "string"

    if pf and (not node.children or #node.children == 0) then
        local num_val = nil
        if ptype == "double" then
            if node.converted and node.converted ~= "" then
                num_val = tonumber(string.match(node.converted, "([%-%+]?%d+%.?%d*)"))
            end
            if not num_val then
                num_val = tonumber(node.raw)
            end
        elseif ptype == "uint32" or ptype == "int32" then
            if node.converted and node.converted ~= "" then
                num_val = tonumber(string.match(node.converted, "([%-%+]?%d+)"))
            end
            if not num_val then
                num_val = tonumber(node.raw)
            end
        end

        local display_val = node.converted
        if not display_val or display_val == "" then
            display_val = node.raw or ""
        end

        if num_val and (ptype == "uint32" or ptype == "int32" or ptype == "double") then
            sub_item = tree_item:add(pf, item_tvb, num_val)
            if flat_pf and flat_pf ~= pf then
                local h = tree_item:add(flat_pf, item_tvb, num_val)
                h:set_hidden()
            end
        else
            sub_item = tree_item:add(pf, item_tvb, tostring(display_val))
            if flat_pf and flat_pf ~= pf then
                local h = tree_item:add(flat_pf, item_tvb, tostring(display_val))
                h:set_hidden()
            end
        end

        local item_label = node.label
        if not item_label or item_label == "" then
            local prefix = node.name
            if node.description and node.description ~= "" and node.description ~= node.name then
                prefix = string.format("%s (%s)", node.description, node.name)
            end
            item_label = string.format("%s: %s", prefix, tostring(node.converted or node.raw or ""))
        end
        -- Enhancement 6: Sub-byte bitfield dotted mask breakdown (e.g. ".... .1.. = ...")
        if node.bitmask_dots and node.bitmask_dots ~= "" then
            item_label = string.format("%s = %s", node.bitmask_dots, item_label)
        end
        sub_item:set_text(item_label)
    else
        local label = node.label or node.name or "Item"
        if (not node.label or node.label == node.name) and node.description and node.description ~= "" and node.description ~= node.name then
            label = string.format("%s (%s)", node.description, node.name)
        end
        -- Enhancement 6: Sub-byte bitfield dotted mask breakdown for generic items
        if node.bitmask_dots and node.bitmask_dots ~= "" then
            label = string.format("%s = %s", node.bitmask_dots, label)
        end
        if pf then
            sub_item = tree_item:add(pf, item_tvb, label)
        else
            sub_item = tree_item:add(proto, item_tvb, label)
        end
    end

    -- Enhancement 5: Attach expert info / anomalies to tree item
    if node.anomalies and #node.anomalies > 0 then
        for _, an in ipairs(node.anomalies) do
            local ef = expert_warn
            if an.severity == "Error" then
                ef = expert_error
            elseif an.severity == "Note" then
                ef = expert_note
            elseif an.severity == "Chat" then
                ef = expert_chat
            end
            sub_item:add_proto_expert_info(ef, an.message)
        end
    end

    if node.children and #node.children > 0 then
        for _, child in ipairs(node.children) do
            populate_node(sub_item, tvb, child)
        end
    end
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

    local json_str = nil

    if native_dfdl and native_dfdl.dissect_bytes then
        local raw_bytes = payload_tvb:raw()
        json_str = native_dfdl.dissect_bytes(active_schema, raw_bytes)
    elseif native_dfdl and native_dfdl.dissect_hex then
        local hex_data = tostring(payload_tvb:bytes():tohex())
        json_str = native_dfdl.dissect_hex(active_schema, hex_data)
    else
        local hex_data = tostring(payload_tvb:bytes():tohex())
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
        if tree_data.protocol and tree_data.protocol ~= "" then
            pinfo.cols.protocol = tree_data.protocol
        else
            pinfo.cols.protocol = "{proto_id}"
        end

        if tree_data.summary and tree_data.summary ~= "" then
            pinfo.cols.info = tree_data.summary
        end

        local root_tree = tree:add(proto, payload_tvb(), string.format("%s (%d bytes)", tree_data.protocol or "{title}", payload_tvb:len()))
        if tree_data.anomalies and #tree_data.anomalies > 0 then
            for _, an in ipairs(tree_data.anomalies) do
                local ef = expert_warn
                if an.severity == "Error" then
                    ef = expert_error
                elseif an.severity == "Note" then
                    ef = expert_note
                elseif an.severity == "Chat" then
                    ef = expert_chat
                end
                root_tree:add_proto_expert_info(ef, an.message)
            end
        end
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
        dylib_candidates_lua = dylib_candidates_lua,
        cli_bin_str = cli_bin_str,
    )
}
