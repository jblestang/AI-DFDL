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
