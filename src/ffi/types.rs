//! FFI-safe type conversions and helpers.

use std::ffi::{c_char, CStr, CString};

/// Convert a C string pointer to a Rust `&str`.
///
/// # Safety
/// `ptr` must be a valid, null-terminated C string or NULL.
pub(crate) unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Allocate a C string from a Rust string.
/// The caller must free the result with `weby_free_string`.
pub(crate) fn str_to_cstring(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cstr) => cstr.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string previously returned by webylib FFI functions.
///
/// # Safety
/// `ptr` must have been allocated by this library (e.g., from `weby_wallet_balance`)
/// or be NULL (no-op).
#[no_mangle]
pub unsafe extern "C" fn weby_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}
