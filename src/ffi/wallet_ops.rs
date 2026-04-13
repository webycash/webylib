//! FFI exports for wallet operations.
//!
//! Every function returns an `i32` error code (0 = success).
//! On failure, call `weby_last_error_message()` for a human-readable description.
//! Strings returned via `out_*` pointers must be freed with `weby_free_string()`.

use std::ffi::c_char;

use super::error::{result_to_code, set_last_error, WebyErrorCode};
use super::types::{cstr_to_str, str_to_cstring};
use crate::wallet::Wallet;

/// Opaque wallet handle.
pub struct WebyWallet {
    wallet: Wallet,
    runtime: tokio::runtime::Runtime,
}

// ── Lifecycle ───────────────────────────────────────────────────────

/// Open or create a wallet at `path`.
///
/// On success, writes the wallet handle to `*out_wallet`.
/// The caller must eventually free it with `weby_wallet_free`.
///
/// # Safety
/// - `path` must be a valid null-terminated UTF-8 string.
/// - `out_wallet` must be a valid, non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_open(
    path: *const c_char,
    out_wallet: *mut *mut WebyWallet,
) -> i32 {
    if out_wallet.is_null() {
        set_last_error("out_wallet is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let path_str = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_last_error("path is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(&format!("Failed to create async runtime: {}", e));
            return WebyErrorCode::Unknown as i32;
        }
    };
    let result = rt.block_on(Wallet::open(path_str));
    let code = result_to_code(&result);
    if let Ok(wallet) = result {
        let handle = Box::new(WebyWallet { wallet, runtime: rt });
        unsafe { *out_wallet = Box::into_raw(handle) };
    }
    code
}

/// Open or create a wallet with a caller-provided 32-byte seed.
///
/// # Safety
/// - `path` must be a valid null-terminated UTF-8 string.
/// - `seed_ptr` must point to at least 32 bytes.
/// - `out_wallet` must be a valid, non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_open_with_seed(
    path: *const c_char,
    seed_ptr: *const u8,
    seed_len: usize,
    out_wallet: *mut *mut WebyWallet,
) -> i32 {
    if out_wallet.is_null() || seed_ptr.is_null() || seed_len != 32 {
        set_last_error("Invalid arguments: need non-null pointers and seed_len=32");
        return WebyErrorCode::InvalidInput as i32;
    }
    let path_str = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_last_error("path is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let mut seed = [0u8; 32];
    unsafe { std::ptr::copy_nonoverlapping(seed_ptr, seed.as_mut_ptr(), 32) };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            set_last_error(&format!("Failed to create async runtime: {}", e));
            return WebyErrorCode::Unknown as i32;
        }
    };
    let result = rt.block_on(Wallet::open_with_seed(path_str, &seed));
    let code = result_to_code(&result);
    if let Ok(wallet) = result {
        let handle = Box::new(WebyWallet { wallet, runtime: rt });
        unsafe { *out_wallet = Box::into_raw(handle) };
    }
    code
}

/// Free a wallet handle.
///
/// # Safety
/// `wallet` must have been returned by `weby_wallet_open*` or be NULL (no-op).
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_free(wallet: *mut WebyWallet) {
    if !wallet.is_null() {
        drop(unsafe { Box::from_raw(wallet) });
    }
}

// ── Operations ──────────────────────────────────────────────────────

/// Get the wallet balance as a decimal string (e.g., "1.50000000").
///
/// On success, writes a C string to `*out_balance`. Free with `weby_free_string`.
///
/// # Safety
/// - `wallet` must be a valid handle from `weby_wallet_open*`.
/// - `out_balance` must be a valid, non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_balance(
    wallet: *const WebyWallet,
    out_balance: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_balance.is_null() {
        set_last_error("wallet or out_balance is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let handle = unsafe { &*wallet };
    let result = handle.runtime.block_on(handle.wallet.balance());
    let code = result_to_code(&result);
    if let Ok(balance) = result {
        unsafe { *out_balance = str_to_cstring(&balance) };
    }
    code
}

/// Insert webcash into the wallet (ownership transfer via server).
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `webcash_str` must be a valid null-terminated webcash string (e.g., "e1:secret:abc...").
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_insert(
    wallet: *const WebyWallet,
    webcash_str: *const c_char,
) -> i32 {
    if wallet.is_null() {
        set_last_error("wallet is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let wc_str = match unsafe { cstr_to_str(webcash_str) } {
        Some(s) => s,
        None => {
            set_last_error("webcash_str is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let webcash = match crate::webcash::SecretWebcash::parse(wc_str) {
        Ok(wc) => wc,
        Err(e) => {
            set_last_error(&e.to_string());
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let handle = unsafe { &*wallet };
    let result = handle.runtime.block_on(handle.wallet.insert(webcash));
    result_to_code(&result)
}

/// Pay amount from the wallet. Returns payment webcash string for recipient.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `amount_str` must be a valid decimal amount string (e.g., "1.5").
/// - `memo` must be a valid null-terminated string (or NULL for empty).
/// - `out_webcash` must be a valid, non-null pointer. Free result with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_pay(
    wallet: *const WebyWallet,
    amount_str: *const c_char,
    memo: *const c_char,
    out_webcash: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_webcash.is_null() {
        set_last_error("wallet or out_webcash is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let amt = match unsafe { cstr_to_str(amount_str) } {
        Some(s) => match s.parse::<crate::Amount>() {
            Ok(a) => a,
            Err(e) => {
                set_last_error(&e.to_string());
                return WebyErrorCode::InvalidInput as i32;
            }
        },
        None => {
            set_last_error("amount_str is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let memo_str = unsafe { cstr_to_str(memo) }.unwrap_or("");
    let handle = unsafe { &*wallet };
    let result = handle.runtime.block_on(handle.wallet.pay(amt, memo_str));
    let code = result_to_code(&result);
    if let Ok(msg) = result {
        unsafe { *out_webcash = str_to_cstring(&msg) };
    }
    code
}

/// Check wallet status against server.
///
/// # Safety
/// `wallet` must be a valid handle.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_check(wallet: *const WebyWallet) -> i32 {
    if wallet.is_null() {
        set_last_error("wallet is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let handle = unsafe { &*wallet };
    let result = handle.runtime.block_on(handle.wallet.check());
    result_to_code(&result)
}

/// Merge small outputs to reduce fragmentation.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `out_result` must be a valid, non-null pointer. Free with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_merge(
    wallet: *const WebyWallet,
    max_outputs: u32,
    out_result: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_result.is_null() {
        set_last_error("wallet or out_result is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let handle = unsafe { &*wallet };
    let result = handle
        .runtime
        .block_on(handle.wallet.merge(max_outputs as usize));
    let code = result_to_code(&result);
    if let Ok(msg) = result {
        unsafe { *out_result = str_to_cstring(&msg) };
    }
    code
}

/// Recover wallet from master secret hex.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `master_secret_hex` must be a 64-character hex string.
/// - `out_result` must be a valid, non-null pointer. Free with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_recover(
    wallet: *const WebyWallet,
    master_secret_hex: *const c_char,
    gap_limit: u32,
    out_result: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_result.is_null() {
        set_last_error("wallet or out_result is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let secret = match unsafe { cstr_to_str(master_secret_hex) } {
        Some(s) => s,
        None => {
            set_last_error("master_secret_hex is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let handle = unsafe { &*wallet };
    let result = handle
        .runtime
        .block_on(handle.wallet.recover(secret, gap_limit as usize));
    let code = result_to_code(&result);
    if let Ok(r) = result {
        unsafe { *out_result = str_to_cstring(&r.to_string()) };
    }
    code
}

/// Get wallet statistics as JSON.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `out_json` must be a valid, non-null pointer. Free with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_stats(
    wallet: *const WebyWallet,
    out_json: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_json.is_null() {
        set_last_error("wallet or out_json is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let handle = unsafe { &*wallet };
    let result = handle.runtime.block_on(handle.wallet.stats());
    let code = result_to_code(&result);
    if let Ok(stats) = result {
        let json = format!(
            r#"{{"total_webcash":{},"unspent_webcash":{},"spent_webcash":{},"total_balance":"{}"}}"#,
            stats.total_webcash, stats.unspent_webcash, stats.spent_webcash, stats.total_balance
        );
        unsafe { *out_json = str_to_cstring(&json) };
    }
    code
}

/// Export wallet snapshot as JSON.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `out_json` must be a valid, non-null pointer. Free with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_export_snapshot(
    wallet: *const WebyWallet,
    out_json: *mut *mut c_char,
) -> i32 {
    if wallet.is_null() || out_json.is_null() {
        set_last_error("wallet or out_json is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let handle = unsafe { &*wallet };
    let result = handle.wallet.export_snapshot();
    let code = result_to_code(&result);
    if let Ok(snapshot) = result {
        match serde_json::to_string(&snapshot) {
            Ok(json) => unsafe { *out_json = str_to_cstring(&json) },
            Err(e) => {
                set_last_error(&e.to_string());
                return WebyErrorCode::Unknown as i32;
            }
        }
    }
    code
}

// ── Encryption ──────────────────────────────────────────────────────

/// Encrypt the wallet seed with a password.
///
/// # Safety
/// - `wallet` must be a valid handle.
/// - `password` must be a valid null-terminated string.
#[no_mangle]
pub unsafe extern "C" fn weby_wallet_encrypt_seed(
    wallet: *const WebyWallet,
    password: *const c_char,
) -> i32 {
    if wallet.is_null() {
        set_last_error("wallet is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let pw = match unsafe { cstr_to_str(password) } {
        Some(s) => s,
        None => {
            set_last_error("password is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    let handle = unsafe { &*wallet };
    let result = handle
        .runtime
        .block_on(handle.wallet.encrypt_database_with_password(pw));
    result_to_code(&result)
}

// ── Utilities ───────────────────────────────────────────────────────

/// Get the library version string.
///
/// The returned pointer is valid for the lifetime of the program — do **not** free it.
#[no_mangle]
pub extern "C" fn weby_version() -> *const c_char {
    // Use a static CStr to avoid per-call allocation
    static VERSION_CSTR: std::sync::LazyLock<std::ffi::CString> =
        std::sync::LazyLock::new(|| {
            std::ffi::CString::new(crate::protocol::VERSION).unwrap()
        });
    VERSION_CSTR.as_ptr()
}

/// Parse an amount string and return the value in wats (integer).
///
/// # Safety
/// - `amount_str` must be a valid null-terminated decimal string.
/// - `out_wats` must be a valid, non-null pointer.
#[no_mangle]
pub unsafe extern "C" fn weby_amount_parse(
    amount_str: *const c_char,
    out_wats: *mut i64,
) -> i32 {
    if out_wats.is_null() {
        set_last_error("out_wats is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let s = match unsafe { cstr_to_str(amount_str) } {
        Some(s) => s,
        None => {
            set_last_error("amount_str is null or invalid UTF-8");
            return WebyErrorCode::InvalidInput as i32;
        }
    };
    match s.parse::<crate::Amount>() {
        Ok(amount) => {
            unsafe { *out_wats = amount.wats };
            WebyErrorCode::Ok as i32
        }
        Err(e) => {
            set_last_error(&e.to_string());
            WebyErrorCode::InvalidInput as i32
        }
    }
}

/// Format wats as a decimal amount string.
///
/// # Safety
/// `out_str` must be a valid, non-null pointer. Free with `weby_free_string`.
#[no_mangle]
pub unsafe extern "C" fn weby_amount_format(
    wats: i64,
    out_str: *mut *mut c_char,
) -> i32 {
    if out_str.is_null() {
        set_last_error("out_str is null");
        return WebyErrorCode::InvalidInput as i32;
    }
    let amount = crate::Amount::from_wats(wats);
    unsafe { *out_str = str_to_cstring(&amount.to_string()) };
    WebyErrorCode::Ok as i32
}
