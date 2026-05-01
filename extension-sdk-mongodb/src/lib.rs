//! Rust SDK for building MongoDB server extensions (aggregation stages) as `cdylib` plugins.
//!
//! The server loads your `.so` via `dlopen` and resolves [`GET_MONGODB_EXTENSION_SYMBOL`]. This
//! crate provides:
//! - Re-exported [`sys`] FFI types
//! - [`export_transform_stage`] for a passthrough transform stage (documents unchanged)
//! - [`export_map_transform_stage!`] for a stage that maps each upstream row with a Rust function
//! - Utilities: [`byte_buf`], [`status`], [`version`], [`host`]
//!
//! ## Example
//!
//! ```ignore
//! use extension_sdk_mongodb::export_transform_stage;
//! export_transform_stage!("$myRustPass", true);
//! ```
//!
//! Build with `crate-type = ["cdylib"]` and link only [`GET_MONGODB_EXTENSION_SYMBOL`].

#![warn(missing_docs)]

pub mod byte_buf;
pub mod host;
pub mod map_transform;
pub mod panics;
pub mod passthrough;
pub mod status;
pub mod version;

pub use extension_sys_mongodb as sys;

pub use sys::GET_MONGODB_EXTENSION_SYMBOL;

pub use map_transform::{get_map_extension_impl, MapStageGlobals};
pub use passthrough::{get_extension_impl, StageGlobals};

/// Defines `get_mongodb_extension` exporting a single passthrough transform stage.
///
/// ```ignore
/// extension_sdk_mongodb::export_transform_stage!("$myRustPass", true);
/// ```
#[macro_export]
macro_rules! export_transform_stage {
    ($stage:literal, $expect_empty:expr $(,)? ) => {
        #[no_mangle]
        pub unsafe extern "C" fn get_mongodb_extension(
            host_versions: *const $crate::sys::MongoExtensionAPIVersionVector,
            extension_out: *mut *const $crate::sys::MongoExtension,
        ) -> *mut $crate::sys::MongoExtensionStatus {
            let globals = $crate::passthrough::StageGlobals {
                name: $stage,
                expect_empty: $expect_empty,
            };
            $crate::passthrough::get_extension_impl(globals, host_versions, extension_out)
        }
    };
}

/// Defines `get_mongodb_extension` exporting a map transform stage (`transform` is `fn(&Document, &Document) -> Result<Document, String>`).
///
/// Optional fourth argument `on_eof` is `fn(&Document) -> Result<Document, String>`: invoked once
/// when upstream hits EOF before any row (e.g. empty collection), using only stage arguments.
///
/// Optional fifth argument `on_init` is `unsafe fn(*const MongoExtensionHostPortal) -> Result<(), String>`:
/// runs during extension `initialize` (parse extension YAML via [`host::extension_options_raw`](crate::host::extension_options_raw)).
///
/// ```ignore
/// fn my_map(row: &bson::Document, args: &bson::Document) -> Result<bson::Document, String> { Ok(row.clone()) }
/// extension_sdk_mongodb::export_map_transform_stage!("$myMap", false, my_map);
/// extension_sdk_mongodb::export_map_transform_stage!("$myGen", false, my_map, my_on_eof);
/// unsafe fn my_init(p: *const extension_sdk_mongodb::sys::MongoExtensionHostPortal) -> Result<(), String> { Ok(()) }
/// extension_sdk_mongodb::export_map_transform_stage!("$x", false, my_map, my_on_eof, my_init);
/// ```
#[macro_export]
macro_rules! export_map_transform_stage {
    (
        $stage:literal,
        $expect_empty:expr,
        $transform:path,
        $on_eof:path,
        $on_init:path $(,)?
    ) => {
        #[no_mangle]
        pub unsafe extern "C" fn get_mongodb_extension(
            host_versions: *const $crate::sys::MongoExtensionAPIVersionVector,
            extension_out: *mut *const $crate::sys::MongoExtension,
        ) -> *mut $crate::sys::MongoExtensionStatus {
            let globals = $crate::map_transform::MapStageGlobals {
                name: $stage,
                expect_empty: $expect_empty,
                transform: $transform,
                on_eof_no_rows: Some($on_eof),
                on_extension_initialized: Some($on_init),
            };
            $crate::map_transform::get_map_extension_impl(globals, host_versions, extension_out)
        }
    };
    ($stage:literal, $expect_empty:expr, $transform:path, $on_eof:path $(,)? ) => {
        #[no_mangle]
        pub unsafe extern "C" fn get_mongodb_extension(
            host_versions: *const $crate::sys::MongoExtensionAPIVersionVector,
            extension_out: *mut *const $crate::sys::MongoExtension,
        ) -> *mut $crate::sys::MongoExtensionStatus {
            let globals = $crate::map_transform::MapStageGlobals {
                name: $stage,
                expect_empty: $expect_empty,
                transform: $transform,
                on_eof_no_rows: Some($on_eof),
                on_extension_initialized: None,
            };
            $crate::map_transform::get_map_extension_impl(globals, host_versions, extension_out)
        }
    };
    ($stage:literal, $expect_empty:expr, $transform:path $(,)? ) => {
        #[no_mangle]
        pub unsafe extern "C" fn get_mongodb_extension(
            host_versions: *const $crate::sys::MongoExtensionAPIVersionVector,
            extension_out: *mut *const $crate::sys::MongoExtension,
        ) -> *mut $crate::sys::MongoExtensionStatus {
            let globals = $crate::map_transform::MapStageGlobals {
                name: $stage,
                expect_empty: $expect_empty,
                transform: $transform,
                on_eof_no_rows: None,
                on_extension_initialized: None,
            };
            $crate::map_transform::get_map_extension_impl(globals, host_versions, extension_out)
        }
    };
}
