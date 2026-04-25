//! End-to-end test of the `#[ffi_export]` macro.
//!
//! Calls the generated `extern "C"` symbols directly (we're in the same
//! crate as the cdylib, so they're locally accessible). This is the
//! same path that a C consumer would hit through the `.dylib`/`.so`.

//! Each generated `extern "C"` symbol is also reachable via the Rust path
//! `webylib_ffi::weby_xxx` (because the macro emits the shim alongside the
//! original function in the same crate). C consumers link the cdylib;
//! these tests link the rlib in-process.

use std::ffi::CString;

#[test]
fn sync_scalar_roundtrip() {
    let r = unsafe { webylib_ffi::weby_add(2, 3) };
    assert_eq!(r, 5);
}

#[test]
fn sync_i64_identity() {
    let r = unsafe { webylib_ffi::weby_identity_i64(42_000_000_000) };
    assert_eq!(r, 42_000_000_000);
}

#[test]
fn sync_str_len_via_cstr() {
    let s = CString::new("hello world").unwrap();
    let r = unsafe { webylib_ffi::weby_str_len(s.as_ptr()) };
    assert_eq!(r, 11);
}

#[test]
fn sync_str_len_null_returns_neg_one() {
    let r = unsafe { webylib_ffi::weby_str_len(std::ptr::null()) };
    assert_eq!(r, -1, "null pointer must early-return -1");
}

#[test]
fn async_result_scalar_marshaling() {
    let mut out: i64 = 0;
    let code = unsafe { webylib_ffi::weby_double_eventually(21, &mut out as *mut i64) };
    assert_eq!(code, 0, "Ok must return 0");
    assert_eq!(out, 42);
}

#[test]
fn async_result_overflow_error_code() {
    let mut out: i64 = 0;
    let code = unsafe { webylib_ffi::weby_double_eventually(i64::MAX, &mut out as *mut i64) };
    assert_eq!(code, -100, "Err must return -100");
}

#[test]
fn async_result_unit_marshaling() {
    let ok = unsafe { webylib_ffi::weby_validate_positive(7) };
    assert_eq!(ok, 0);
    let err = unsafe { webylib_ffi::weby_validate_positive(-1) };
    assert_eq!(err, -100);
}
