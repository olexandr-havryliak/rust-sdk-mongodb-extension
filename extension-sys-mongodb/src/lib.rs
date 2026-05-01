//! Raw FFI types for the MongoDB Extensions API (`api.h`).
//!
//! Layout matches the vendored header in `../include/mongodb_extension_api.h`.

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

mod abi;

pub use abi::*;
