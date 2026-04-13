//! FFI error codes and thread-local error message storage.

use std::cell::RefCell;
use std::ffi::{c_char, CString};

use crate::error::Error;

/// FFI error codes returned by all `weby_*` functions.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebyErrorCode {
    Ok = 0,
    InvalidInput = 1,
    DatabaseError = 2,
    CryptoError = 3,
    ServerError = 4,
    InsufficientFunds = 5,
    NetworkError = 6,
    AuthError = 7,
    NotSupported = 8,
    Unknown = -1,
}

impl From<&Error> for WebyErrorCode {
    fn from(e: &Error) -> Self {
        match e {
            Error::InvalidInput { .. } | Error::Parse { .. } => WebyErrorCode::InvalidInput,
            Error::Database(_) => WebyErrorCode::DatabaseError,
            Error::Crypto { .. } => WebyErrorCode::CryptoError,
            Error::Server { .. } => WebyErrorCode::ServerError,
            Error::InsufficientFunds { .. } => WebyErrorCode::InsufficientFunds,
            Error::Http(_) => WebyErrorCode::NetworkError,
            Error::Auth { .. } | Error::TermsNotAccepted => WebyErrorCode::AuthError,
            Error::NotSupported { .. } => WebyErrorCode::NotSupported,
            _ => WebyErrorCode::Unknown,
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Store an error message in thread-local storage.
pub(crate) fn set_last_error(msg: &str) {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok();
    });
}

/// Map a `Result<T>` to an FFI error code, storing the message on failure.
pub(crate) fn result_to_code<T>(result: &std::result::Result<T, Error>) -> i32 {
    match result {
        Ok(_) => WebyErrorCode::Ok as i32,
        Err(e) => {
            set_last_error(&e.to_string());
            WebyErrorCode::from(e) as i32
        }
    }
}

/// Get the last error message (returns NULL if no error).
///
/// # Safety
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do **not** free it — it is owned by the library.
#[no_mangle]
pub extern "C" fn weby_last_error_message() -> *const c_char {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(cstr) => cstr.as_ptr(),
            None => std::ptr::null(),
        }
    })
}
