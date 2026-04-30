//! Raw FFI bindings to the Qualcomm AI Engine Direct (QNN) SDK.
//!
//! This crate is generated at build time by `bindgen` against the headers in
//! `$QNN_SDK_ROOT/include/QNN`. If `QNN_SDK_ROOT` is unset at build time, a
//! stub is emitted so the rest of the hexrun workspace can compile, but the
//! real symbols will be unavailable.
//!
//! Prefer the safe wrapper in the `qnn` crate over using this crate directly.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
