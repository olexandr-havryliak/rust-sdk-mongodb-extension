//! **Rust SDK for MongoDB Extensions** — build MongoDB server extensions (aggregation stages) as `cdylib` plugins.
//!
//! The server loads your `.so` via `dlopen` and resolves [`GET_MONGODB_EXTENSION_SYMBOL`]. This
//! crate provides:
//! - Re-exported [`sys`] FFI types
//! - [`export_transform_stage`] for a passthrough transform stage (documents unchanged)
//! - [`export_map_transform_stage!`] for a stage that maps each upstream row with a Rust function
//! - [`export_source_stage!`] for a **generator** stage (no upstream required, e.g. `aggregate: 1`)
//! - Utilities: [`byte_buf`], [`status`], [`version`], [`host`], [`StageContext`], [`Next`], [`error`], [`TransformStage`]
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
pub mod error;
pub(crate) mod extension_log;
pub mod host;
pub mod map_transform;
pub mod operation_metrics;
pub mod panics;
pub mod passthrough;
pub mod source_stage;
pub mod stage_context;
pub mod stage_output;
pub mod status;
pub mod transform_stage;
pub mod version;

pub use extension_sys_mongodb as sys;

pub use sys::GET_MONGODB_EXTENSION_SYMBOL;

pub use error::{parse_args, ExtensionError};
pub use error::Result as ExtensionResult;
pub use map_transform::{get_map_extension_impl, MapStageGlobals};
pub use passthrough::{get_extension_impl, StageGlobals};
pub use source_stage::{get_source_extension_impl, SourceOps, SourceStage};
pub use stage_context::{OperationMetricsSink, StageContext};
pub use stage_output::Next;
pub use transform_stage::TransformStage;

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

/// Defines `get_mongodb_extension` from a [`TransformStage`](crate::transform_stage::TransformStage) impl (wraps [`export_map_transform_stage!`]).
///
/// Pass the stage name literal (must match [`TransformStage::NAME`](TransformStage::NAME)), the same
/// `expect_empty` flag as [`export_transform_stage!`](export_transform_stage), and the implementing type.
///
/// ```ignore
/// struct DoubleX;
/// impl extension_sdk_mongodb::TransformStage for DoubleX {
///     const NAME: &'static str = "$doubleX";
///     type Args = bson::Document;
///     fn parse(args: bson::Document) -> extension_sdk_mongodb::ExtensionResult<Self::Args> { Ok(args) }
///     fn transform(input: bson::Document, _args: &Self::Args, _ctx: &mut extension_sdk_mongodb::StageContext) -> extension_sdk_mongodb::ExtensionResult<bson::Document> {
///         Ok(input)
///     }
/// }
/// extension_sdk_mongodb::export_transform_stage_type!("$doubleX", false, DoubleX);
/// ```
#[macro_export]
macro_rules! export_transform_stage_type {
    ($stage:literal, $expect_empty:expr, $t:ty $(,)? ) => {
        fn __typed_transform_row(
            row: &bson::Document,
            args: &bson::Document,
        ) -> std::result::Result<bson::Document, std::string::String> {
            let parsed = <$t as $crate::transform_stage::TransformStage>::parse(args.clone())
                .map_err(|e| e.to_string())?;
            let mut ctx = $crate::stage_context::StageContext::new();
            <$t as $crate::transform_stage::TransformStage>::transform(row.clone(), &parsed, &mut ctx)
                .map_err(|e| e.to_string())
        }
        $crate::export_map_transform_stage!($stage, $expect_empty, __typed_transform_row);
    };
}

/// Defines `get_mongodb_extension` exporting a [`SourceStage`](crate::source_stage::SourceStage)
/// generator stage (emits documents when there is no upstream executable stage).
///
/// Pass the implementing type (unit struct or zero-sized type). Only **one** `export_source_stage!`
/// may appear per crate (single `get_mongodb_extension` symbol).
///
/// ```ignore
/// struct MyGen;
/// impl extension_sdk_mongodb::SourceStage for MyGen {
///     const NAME: &'static str = "$myGen";
///     type Args = bson::Document;
///     type State = usize;
///     fn parse(args: bson::Document) -> extension_sdk_mongodb::ExtensionResult<Self::Args> { Ok(args) }
///     fn open(args: Self::Args, _ctx: &mut extension_sdk_mongodb::StageContext) -> extension_sdk_mongodb::ExtensionResult<Self::State> { Ok(0) }
///     fn next(state: &mut Self::State, _ctx: &mut extension_sdk_mongodb::StageContext) -> extension_sdk_mongodb::ExtensionResult<extension_sdk_mongodb::Next> {
///         if *state >= 3 { return Ok(extension_sdk_mongodb::Next::Eof); }
///         let d = bson::doc! { "i": *state as i32 };
///         *state += 1;
///         Ok(extension_sdk_mongodb::Next::Advanced { document: d, metadata: None })
///     }
/// }
/// extension_sdk_mongodb::export_source_stage!(MyGen);
/// ```
#[macro_export]
macro_rules! export_source_stage {
    ($t:ty $(,)? ) => {
        fn __sdk_source_open(
            d: bson::Document,
            ctx: &mut $crate::stage_context::StageContext,
        ) -> $crate::error::Result<*mut std::ffi::c_void> {
            let a = <$t as $crate::source_stage::SourceStage>::parse(d)?;
            let s = <$t as $crate::source_stage::SourceStage>::open(a, ctx)?;
            Ok(std::boxed::Box::into_raw(std::boxed::Box::new(s)) as *mut std::ffi::c_void)
        }
        unsafe fn __sdk_source_drop(p: *mut std::ffi::c_void) {
            if p.is_null() {
                return;
            }
            drop(std::boxed::Box::from_raw(
                p as *mut <$t as $crate::source_stage::SourceStage>::State,
            ));
        }
        unsafe fn __sdk_source_next(
            p: *mut std::ffi::c_void,
            ctx: &mut $crate::stage_context::StageContext,
        ) -> $crate::error::Result<$crate::stage_output::Next> {
            let s = &mut *(p as *mut <$t as $crate::source_stage::SourceStage>::State);
            <$t as $crate::source_stage::SourceStage>::next(s, ctx)
        }
        static __SDK_SOURCE_OPS: $crate::source_stage::SourceOps = $crate::source_stage::SourceOps {
            name: <$t as $crate::source_stage::SourceStage>::NAME,
            expect_empty: false,
            open_from_doc: __sdk_source_open,
            drop_state: __sdk_source_drop,
            next: __sdk_source_next,
            on_extension_initialized: None,
        };
        #[no_mangle]
        pub unsafe extern "C" fn get_mongodb_extension(
            host_versions: *const $crate::sys::MongoExtensionAPIVersionVector,
            extension_out: *mut *const $crate::sys::MongoExtension,
        ) -> *mut $crate::sys::MongoExtensionStatus {
            $crate::source_stage::get_source_extension_impl(
                &__SDK_SOURCE_OPS,
                host_versions,
                extension_out,
            )
        }
    };
}
