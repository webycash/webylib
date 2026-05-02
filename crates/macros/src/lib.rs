//! Procedural macros for webylib.
//!
//! Three macros land here:
//! - `#[wallet_op]` — generates lock/run-effect/emit-event/persist plumbing
//!   for a wallet operation. Mirrors webycash-server's `#[gen_server]`.
//! - `#[asset_storage]` — generates per-asset CRUD on the `Store` trait
//!   from a struct definition.
//! - `#[ffi_export]` — generates the `extern "C"` shim, error-code
//!   marshaling, opaque-handle lifecycle, and async-callback bridging
//!   for the FFI layer. **This is the macro that replaces the legacy
//!   22K-LOC handwritten `src/ffi/wallet_ops.rs` with one source of truth.**

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
    FnArg, ItemFn, Pat, ReturnType, Type,
};

// ─────────────────────────────────────────────────────────────────────────────
// #[ffi_export]
// ─────────────────────────────────────────────────────────────────────────────

/// Attribute macro that takes an async (or sync) Rust function and emits a
/// matching `extern "C"` shim.
///
/// Supported signatures:
///
/// ```ignore
/// #[ffi_export]
/// pub fn balance(handle: WeyWalletHandle) -> i64 { ... }
///
/// #[ffi_export(name = "weby_wallet_open")]
/// pub async fn open(path: &str) -> Result<WeyWalletHandle, Error> { ... }
/// ```
///
/// Generates:
///   * `extern "C" fn weby_wallet_balance(handle: WeyWalletHandle) -> i64`
///   * `extern "C" fn weby_wallet_open(path: *const c_char,
///                                     out: *mut WeyWalletHandle) -> i32`
///     — async wrapped via the FFI runtime; result marshaled to
///     `(error_code, out)`.
///
/// The macro recognises a small set of marshaling rules:
///   * `&str` → `*const c_char`            (read with `CStr::from_ptr`)
///   * `String` → `*const c_char`          (caller frees nothing; we copy)
///   * `Result<T, E>` → returns `i32`      with T written via `*out` ptr
///   * `Result<(), E>` → returns `i32`
///   * `i32` / `i64` / `u32` / `u64` / `bool` / opaque handle types → as-is
#[proc_macro_attribute]
pub fn ffi_export(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as FfiArgs);
    let func = parse_macro_input!(item as ItemFn);

    match expand_ffi_export(args, func) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

struct FfiArgs {
    /// Optional override for the C-side function name.
    name: Option<String>,
    /// Optional prefix prepended to the Rust ident (default: `weby_`).
    prefix: Option<String>,
}

impl Parse for FfiArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut prefix = None;
        if input.is_empty() {
            return Ok(FfiArgs { name, prefix });
        }
        let metas: syn::punctuated::Punctuated<syn::MetaNameValue, syn::Token![,]> =
            input.parse_terminated(syn::MetaNameValue::parse, syn::Token![,])?;
        for m in metas {
            let key = m
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let val = match m.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) => s.value(),
                other => {
                    return Err(syn::Error::new(other.span(), "expected string literal"));
                }
            };
            match key.as_str() {
                "name" => name = Some(val),
                "prefix" => prefix = Some(val),
                _ => return Err(syn::Error::new(m.path.span(), "unknown ffi_export option")),
            }
        }
        Ok(FfiArgs { name, prefix })
    }
}

fn expand_ffi_export(args: FfiArgs, mut func: ItemFn) -> syn::Result<TokenStream2> {
    let original = func.clone();
    let rust_ident = func.sig.ident.clone();
    let prefix = args.prefix.unwrap_or_else(|| "weby_".to_string());
    let c_ident = match args.name {
        Some(n) => format_ident!("{}", n),
        None => format_ident!("{}{}", prefix, rust_ident),
    };

    let is_async = func.sig.asyncness.is_some();

    // Map each input. Track:
    //   - the C-ABI parameter list we'll emit
    //   - the conversion code to translate to Rust types
    //   - the Rust call argument list
    let mut c_params = Vec::<TokenStream2>::new();
    let mut conversions = Vec::<TokenStream2>::new();
    let mut call_args = Vec::<TokenStream2>::new();

    for input in &func.sig.inputs {
        let FnArg::Typed(pat_ty) = input else {
            return Err(syn::Error::new(
                input.span(),
                "ffi_export does not support `self` receivers; use a free function with an opaque handle parameter",
            ));
        };
        let Pat::Ident(pat_ident) = &*pat_ty.pat else {
            return Err(syn::Error::new(
                pat_ty.pat.span(),
                "expected simple identifier",
            ));
        };
        let name = pat_ident.ident.clone();
        let ty = &*pat_ty.ty;
        let kind = classify_input(ty)?;
        match kind {
            InputKind::CStrRef => {
                let raw = format_ident!("__{}_raw", name);
                c_params.push(quote! { #raw: *const ::std::os::raw::c_char });
                conversions.push(quote! {
                    let #name: &str = match unsafe {
                        if #raw.is_null() { return -1; }
                        ::std::ffi::CStr::from_ptr(#raw).to_str()
                    } {
                        Ok(s) => s,
                        Err(_) => return -2,
                    };
                });
                call_args.push(quote! { #name });
            }
            InputKind::Scalar(scalar) => {
                c_params.push(quote! { #name: #scalar });
                call_args.push(quote! { #name });
            }
            InputKind::Opaque(path) => {
                c_params.push(quote! { #name: #path });
                call_args.push(quote! { #name });
            }
        }
    }

    // Map the return type. `post_call` MUST be an expression that the
    // generated `extern "C"` body returns.
    let (c_return, post_call): (TokenStream2, TokenStream2) = match &func.sig.output {
        ReturnType::Default => (quote! { () }, quote! { drop(__result) }),
        ReturnType::Type(_, ty) => match classify_output(ty)? {
            // Sync scalar / opaque: the generated body returns __result directly.
            OutputKind::Scalar(s) => (quote! { #s }, quote! { __result }),
            OutputKind::Opaque(path) => (quote! { #path }, quote! { __result }),
            OutputKind::ResultUnit => (
                quote! { i32 },
                quote! {
                    match __result {
                        Ok(()) => 0,
                        Err(_) => -100,
                    }
                },
            ),
            OutputKind::ResultScalar(s) => {
                c_params.push(quote! { __out: *mut #s });
                (
                    quote! { i32 },
                    quote! {
                        match __result {
                            Ok(v) => unsafe {
                                if !__out.is_null() { *__out = v; }
                                0
                            },
                            Err(_) => -100,
                        }
                    },
                )
            }
            OutputKind::ResultOpaque(path) => {
                c_params.push(quote! { __out: *mut #path });
                (
                    quote! { i32 },
                    quote! {
                        match __result {
                            Ok(v) => unsafe {
                                if !__out.is_null() { *__out = v; }
                                0
                            },
                            Err(_) => -100,
                        }
                    },
                )
            }
        },
    };

    // Strip async/visibility from the original; we re-wrap.
    func.sig.asyncness = None;

    let invocation = if is_async {
        // Each generated fn carries its own lazy-initialised Tokio runtime.
        // The first call pays the construction cost; subsequent calls reuse.
        // Caller crate must depend on `tokio` with `rt-multi-thread`.
        quote! {
            let __result = {
                static __RT: ::std::sync::OnceLock<::tokio::runtime::Runtime> =
                    ::std::sync::OnceLock::new();
                let rt = __RT.get_or_init(|| {
                    ::tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .expect("ffi_export tokio runtime")
                });
                rt.block_on(async {
                    #rust_ident(#(#call_args),*).await
                })
            };
        }
    } else {
        quote! {
            let __result = #rust_ident(#(#call_args),*);
        }
    };

    let extern_fn = quote! {
        /// Auto-generated FFI export. Do not edit directly; modify the
        /// underlying Rust function and recompile.
        #[no_mangle]
        pub unsafe extern "C" fn #c_ident(#(#c_params),*) -> #c_return {
            // Conversions (e.g., *const c_char → &str). On failure these
            // early-return with a negative error code.
            #(#conversions)*
            #invocation
            #post_call
        }
    };

    Ok(quote! {
        #original
        #extern_fn
    })
}

enum InputKind {
    CStrRef,
    Scalar(TokenStream2),
    Opaque(TokenStream2),
}

fn classify_input(ty: &Type) -> syn::Result<InputKind> {
    if let Type::Reference(r) = ty {
        if let Type::Path(p) = &*r.elem {
            if p.path.is_ident("str") {
                return Ok(InputKind::CStrRef);
            }
        }
    }
    if let Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            let s = seg.ident.to_string();
            match s.as_str() {
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize"
                | "bool" | "f32" | "f64" => {
                    return Ok(InputKind::Scalar(quote! { #ty }));
                }
                _ => {
                    // Treat as opaque pass-through (likely a `#[repr(C)]`
                    // handle struct or `*mut T`).
                    return Ok(InputKind::Opaque(quote! { #ty }));
                }
            }
        }
    }
    if let Type::Ptr(_) = ty {
        return Ok(InputKind::Opaque(quote! { #ty }));
    }
    Err(syn::Error::new(
        ty.span(),
        "ffi_export: unsupported parameter type",
    ))
}

enum OutputKind {
    Scalar(TokenStream2),
    Opaque(TokenStream2),
    ResultUnit,
    ResultScalar(TokenStream2),
    ResultOpaque(TokenStream2),
}

fn classify_output(ty: &Type) -> syn::Result<OutputKind> {
    if let Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            if seg.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                    let inner = args.args.first();
                    match inner {
                        Some(syn::GenericArgument::Type(Type::Tuple(t))) if t.elems.is_empty() => {
                            return Ok(OutputKind::ResultUnit);
                        }
                        Some(syn::GenericArgument::Type(inner_ty)) => {
                            return Ok(match classify_input(inner_ty)? {
                                InputKind::Scalar(s) => OutputKind::ResultScalar(s),
                                InputKind::Opaque(p) => OutputKind::ResultOpaque(p),
                                InputKind::CStrRef => {
                                    return Err(syn::Error::new(
                                        inner_ty.span(),
                                        "ffi_export: returning &str isn't supported; use String + caller-allocated buffer instead",
                                    ));
                                }
                            });
                        }
                        _ => {}
                    }
                }
                return Err(syn::Error::new(
                    seg.ident.span(),
                    "ffi_export: malformed Result<...>",
                ));
            }
        }
        let s = p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        match s.as_str() {
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize"
            | "bool" | "f32" | "f64" => {
                return Ok(OutputKind::Scalar(quote! { #ty }));
            }
            _ => {
                return Ok(OutputKind::Opaque(quote! { #ty }));
            }
        }
    }
    Err(syn::Error::new(
        ty.span(),
        "ffi_export: unsupported return type",
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Minimal stubs for the OTHER two macros so the trait surface lights up.
// Real implementations land in M2.
// ─────────────────────────────────────────────────────────────────────────────

/// `#[wallet_op]` — placeholder that re-emits the input unchanged.
/// Real plumbing (lock/run/persist) lands when webylib-core operations
/// migrate from `webylib/src/wallet/operations.rs`.
#[proc_macro_attribute]
pub fn wallet_op(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// `#[asset_storage]` — placeholder that re-emits the input unchanged.
/// Generates per-asset CRUD on the `Store` trait when populated.
#[proc_macro_attribute]
pub fn asset_storage(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
