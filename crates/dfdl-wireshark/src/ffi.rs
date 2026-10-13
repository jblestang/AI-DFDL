//! C-ABI Foreign Function Interface (FFI) for Wireshark and Lua plugins.
//!
//! Exposes thread-safe, panic-free C functions for compiling DFDL schemas,
//! dissecting binary payloads, and returning JSON trees for Wireshark GUI display.

use crate::engine::DfdlDissectorEngine;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::Mutex;

static CACHED_ENGINES: Mutex<Option<HashMap<String, DfdlDissectorEngine>>> = Mutex::new(None);

fn get_or_compile_engine(schema_path: &str) -> Result<DfdlDissectorEngine, String> {
    let mut guard = CACHED_ENGINES
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;

    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(engine) = map.get(schema_path) {
        return Ok(engine.clone());
    }

    let engine = DfdlDissectorEngine::from_schema_file(schema_path, None)?;
    map.insert(schema_path.to_string(), engine.clone());
    Ok(engine)
}

/// Dissects a hex string payload using the specified DFDL schema.
///
/// Returns a freshly allocated C string containing the dissection tree in JSON format.
/// The caller must free this string by calling [`dfdl_free_string`].
///
/// # Safety
/// Both `schema_path` and `hex_str` must be valid, non-null, null-terminated UTF-8 C strings.
#[no_mangle]
pub unsafe extern "C" fn dfdl_dissect_hex(
    schema_path: *const c_char,
    hex_str: *const c_char,
) -> *mut c_char {
    if schema_path.is_null() || hex_str.is_null() {
        return std::ptr::null_mut();
    }

    let schema_cstr = match CStr::from_ptr(schema_path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let hex_slice = match CStr::from_ptr(hex_str).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let bytes = match decode_hex(hex_slice) {
        Some(b) => b,
        None => return std::ptr::null_mut(),
    };

    let engine = match get_or_compile_engine(schema_cstr) {
        Ok(e) => e,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            return CString::new(err_json).map_or(std::ptr::null_mut(), CString::into_raw);
        }
    };

    let json = match engine.dissect_payload_to_json(&bytes) {
        Ok(j) => j,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            return CString::new(err_json).map_or(std::ptr::null_mut(), CString::into_raw);
        }
    };

    CString::new(json).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Dissects a raw byte buffer using the specified DFDL schema.
///
/// Returns a freshly allocated C string containing the dissection tree in JSON format.
/// The caller must free this string by calling [`dfdl_free_string`].
///
/// # Safety
/// `schema_path` must be a valid, null-terminated UTF-8 C string.
/// `data_ptr` must point to at least `data_len` valid bytes.
#[no_mangle]
pub unsafe extern "C" fn dfdl_dissect_bytes(
    schema_path: *const c_char,
    data_ptr: *const u8,
    data_len: usize,
) -> *mut c_char {
    if schema_path.is_null() || data_ptr.is_null() {
        return std::ptr::null_mut();
    }

    let schema_cstr = match CStr::from_ptr(schema_path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let bytes = std::slice::from_raw_parts(data_ptr, data_len);

    let engine = match get_or_compile_engine(schema_cstr) {
        Ok(e) => e,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            return CString::new(err_json).map_or(std::ptr::null_mut(), CString::into_raw);
        }
    };

    let json = match engine.dissect_payload_to_json(bytes) {
        Ok(j) => j,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            return CString::new(err_json).map_or(std::ptr::null_mut(), CString::into_raw);
        }
    };

    CString::new(json).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Frees a C string allocated by [`dfdl_dissect_hex`] or [`dfdl_dissect_bytes`].
///
/// # Safety
/// `s` must be a pointer returned by an FFI function from this library or null.
#[no_mangle]
pub unsafe extern "C" fn dfdl_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Returns a freshly allocated C string containing registered schema fields in JSON format.
/// The caller must free this string by calling [`dfdl_free_string`].
///
/// # Safety
/// `schema_path` must be a valid, null-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn dfdl_get_schema_fields(schema_path: *const c_char) -> *mut c_char {
    if schema_path.is_null() {
        return std::ptr::null_mut();
    }
    let schema_cstr = match CStr::from_ptr(schema_path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let content = match std::fs::read_to_string(schema_cstr) {
        Ok(s) => s,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e);
            return CString::new(err_json).map_or(std::ptr::null_mut(), CString::into_raw);
        }
    };
    let registry = crate::schema_meta::SchemaMetadataRegistry::from_schema_xml(&content);
    let json = registry.to_fields_json();
    CString::new(json).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Decodes an ASCII hex string into raw bytes.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    let clean = hex.trim();
    if !clean.len().is_multiple_of(2) {
        return None;
    }

    let mut bytes = Vec::with_capacity(clean.len() / 2);
    let chars: Vec<char> = clean.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        let hi = chars.get(i)?.to_digit(16)? as u8;
        let lo = chars.get(i.checked_add(1)?)?.to_digit(16)? as u8;
        bytes.push((hi << 4) | lo);
        i = i.checked_add(2)?;
    }

    Some(bytes)
}

type LuaState = *mut std::ffi::c_void;
type LuaCFunction = unsafe extern "C" fn(LuaState) -> std::os::raw::c_int;

#[cfg(target_os = "macos")]
const RTLD_DEFAULT: *mut std::ffi::c_void = -2isize as *mut std::ffi::c_void;
#[cfg(not(target_os = "macos"))]
const RTLD_DEFAULT: *mut std::ffi::c_void = std::ptr::null_mut();

extern "C" {
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::os::raw::c_char,
    ) -> *mut std::ffi::c_void;
}

struct LuaApi {
    createtable: unsafe extern "C" fn(LuaState, std::os::raw::c_int, std::os::raw::c_int),
    pushcclosure: unsafe extern "C" fn(LuaState, LuaCFunction, std::os::raw::c_int),
    setfield: unsafe extern "C" fn(LuaState, std::os::raw::c_int, *const std::os::raw::c_char),
    tolstring: unsafe extern "C" fn(
        LuaState,
        std::os::raw::c_int,
        *mut usize,
    ) -> *const std::os::raw::c_char,
    pushlstring: unsafe extern "C" fn(
        LuaState,
        *const std::os::raw::c_char,
        usize,
    ) -> *const std::os::raw::c_char,
    pushnil: unsafe extern "C" fn(LuaState),
}

static LUA_API: std::sync::OnceLock<Option<LuaApi>> = std::sync::OnceLock::new();

fn get_lua_api() -> Option<&'static LuaApi> {
    LUA_API
        .get_or_init(|| unsafe {
            let p_createtable = dlsym(RTLD_DEFAULT, c"lua_createtable".as_ptr());
            let p_pushcclosure = dlsym(RTLD_DEFAULT, c"lua_pushcclosure".as_ptr());
            let p_setfield = dlsym(RTLD_DEFAULT, c"lua_setfield".as_ptr());
            let p_tolstring = dlsym(RTLD_DEFAULT, c"lua_tolstring".as_ptr());
            let p_pushlstring = dlsym(RTLD_DEFAULT, c"lua_pushlstring".as_ptr());
            let p_pushnil = dlsym(RTLD_DEFAULT, c"lua_pushnil".as_ptr());

            if p_createtable.is_null()
                || p_pushcclosure.is_null()
                || p_setfield.is_null()
                || p_tolstring.is_null()
                || p_pushlstring.is_null()
                || p_pushnil.is_null()
            {
                return None;
            }

            Some(LuaApi {
                createtable: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(LuaState, std::os::raw::c_int, std::os::raw::c_int),
                >(p_createtable),
                pushcclosure: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(LuaState, LuaCFunction, std::os::raw::c_int),
                >(p_pushcclosure),
                setfield: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(
                        LuaState,
                        std::os::raw::c_int,
                        *const std::os::raw::c_char,
                    ),
                >(p_setfield),
                tolstring: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(
                        LuaState,
                        std::os::raw::c_int,
                        *mut usize,
                    ) -> *const std::os::raw::c_char,
                >(p_tolstring),
                pushlstring: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(
                        LuaState,
                        *const std::os::raw::c_char,
                        usize,
                    ) -> *const std::os::raw::c_char,
                >(p_pushlstring),
                pushnil: std::mem::transmute::<
                    *mut std::ffi::c_void,
                    unsafe extern "C" fn(LuaState),
                >(p_pushnil),
            })
        })
        .as_ref()
}

unsafe extern "C" fn lua_dissect_hex(state: LuaState) -> std::os::raw::c_int {
    let Some(api) = get_lua_api() else {
        return 0;
    };
    let mut schema_len = 0;
    let schema_ptr = (api.tolstring)(state, 1, &mut schema_len);
    let mut hex_len = 0;
    let hex_ptr = (api.tolstring)(state, 2, &mut hex_len);

    if schema_ptr.is_null() || hex_ptr.is_null() {
        (api.pushnil)(state);
        return 1;
    }

    let schema_str = match std::str::from_utf8(std::slice::from_raw_parts(
        schema_ptr as *const u8,
        schema_len,
    )) {
        Ok(s) => s,
        Err(_) => {
            (api.pushnil)(state);
            return 1;
        }
    };
    let hex_str =
        match std::str::from_utf8(std::slice::from_raw_parts(hex_ptr as *const u8, hex_len)) {
            Ok(s) => s,
            Err(_) => {
                (api.pushnil)(state);
                return 1;
            }
        };

    let bytes = match decode_hex(hex_str) {
        Some(b) => b,
        None => {
            (api.pushnil)(state);
            return 1;
        }
    };

    let engine = match get_or_compile_engine(schema_str) {
        Ok(e) => e,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            (api.pushlstring)(
                state,
                err_json.as_ptr() as *const std::os::raw::c_char,
                err_json.len(),
            );
            return 1;
        }
    };

    let json = match engine.dissect_payload_to_json(&bytes) {
        Ok(j) => j,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            (api.pushlstring)(
                state,
                err_json.as_ptr() as *const std::os::raw::c_char,
                err_json.len(),
            );
            return 1;
        }
    };

    (api.pushlstring)(
        state,
        json.as_ptr() as *const std::os::raw::c_char,
        json.len(),
    );
    1
}

unsafe extern "C" fn lua_dissect_bytes(state: LuaState) -> std::os::raw::c_int {
    let Some(api) = get_lua_api() else {
        return 0;
    };
    let mut schema_len = 0;
    let schema_ptr = (api.tolstring)(state, 1, &mut schema_len);
    let mut bytes_len = 0;
    let bytes_ptr = (api.tolstring)(state, 2, &mut bytes_len);

    if schema_ptr.is_null() || bytes_ptr.is_null() {
        (api.pushnil)(state);
        return 1;
    }

    let schema_str = match std::str::from_utf8(std::slice::from_raw_parts(
        schema_ptr as *const u8,
        schema_len,
    )) {
        Ok(s) => s,
        Err(_) => {
            (api.pushnil)(state);
            return 1;
        }
    };
    let bytes = std::slice::from_raw_parts(bytes_ptr as *const u8, bytes_len);

    let engine = match get_or_compile_engine(schema_str) {
        Ok(e) => e,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            (api.pushlstring)(
                state,
                err_json.as_ptr() as *const std::os::raw::c_char,
                err_json.len(),
            );
            return 1;
        }
    };

    let json = match engine.dissect_payload_to_json(bytes) {
        Ok(j) => j,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\""));
            (api.pushlstring)(
                state,
                err_json.as_ptr() as *const std::os::raw::c_char,
                err_json.len(),
            );
            return 1;
        }
    };

    (api.pushlstring)(
        state,
        json.as_ptr() as *const std::os::raw::c_char,
        json.len(),
    );
    1
}

unsafe extern "C" fn lua_get_schema_fields(state: LuaState) -> std::os::raw::c_int {
    let Some(api) = get_lua_api() else {
        return 0;
    };
    let mut schema_len = 0;
    let schema_ptr = (api.tolstring)(state, 1, &mut schema_len);

    if schema_ptr.is_null() {
        (api.pushnil)(state);
        return 1;
    }

    let schema_str = match std::str::from_utf8(std::slice::from_raw_parts(
        schema_ptr as *const u8,
        schema_len,
    )) {
        Ok(s) => s,
        Err(_) => {
            (api.pushnil)(state);
            return 1;
        }
    };

    let content = match std::fs::read_to_string(schema_str) {
        Ok(s) => s,
        Err(e) => {
            let err_json = format!("{{\"error\": \"{}\"}}", e);
            (api.pushlstring)(
                state,
                err_json.as_ptr() as *const std::os::raw::c_char,
                err_json.len(),
            );
            return 1;
        }
    };

    let registry = crate::schema_meta::SchemaMetadataRegistry::from_schema_xml(&content);
    let json = registry.to_fields_json();
    (api.pushlstring)(
        state,
        json.as_ptr() as *const std::os::raw::c_char,
        json.len(),
    );
    1
}

/// Lua module entry point for `require("dfdl_wireshark")` or `package.loadlib`.
///
/// Registers `dissect_hex`, `dissect_bytes`, and `get_schema_fields` in the returned module table.
///
/// # Safety
/// `state` must be a valid, non-null pointer to a Lua state.
#[no_mangle]
pub unsafe extern "C" fn luaopen_dfdl_wireshark(state: LuaState) -> std::os::raw::c_int {
    let Some(api) = get_lua_api() else {
        return 0;
    };

    (api.createtable)(state, 0, 3);

    (api.pushcclosure)(state, lua_dissect_hex, 0);
    (api.setfield)(state, -2, c"dissect_hex".as_ptr());

    (api.pushcclosure)(state, lua_dissect_bytes, 0);
    (api.setfield)(state, -2, c"dissect_bytes".as_ptr());

    (api.pushcclosure)(state, lua_get_schema_fields, 0);
    (api.setfield)(state, -2, c"get_schema_fields".as_ptr());

    1
}
