//! Exercises public helpers (version, status, byte_buf, host, panics) and mocked host entrypoints.
//!
//! See also: `tests/common/mod.rs` (`MockHost`), `map_extension_*.rs`, `passthrough_extension_init.rs`,
//! `proptest_byte_buf_roundtrip.rs`. Map/passthrough **initialize** paths run here with mocks; full
//! pipelines against `mongod` stay in Docker e2e. **Miri** (undefined-behavior checks on `unsafe`):
//! `./e2e-tests/run-miri-docker.sh` (skips proptest binary).

use bson::doc;
use extension_sdk_mongodb::byte_buf;
use extension_sdk_mongodb::host;
use extension_sdk_mongodb::panics::ffi_boundary;
use extension_sdk_mongodb::passthrough::{get_extension_impl, StageGlobals};
use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::{
    MongoExtensionAPIVersion, MongoExtensionAPIVersionVector, MongoExtensionHostPortal,
    MongoExtensionHostPortalVTable, MongoExtensionHostServices, MongoExtensionHostServicesVTable,
    MongoExtensionStatus, MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::{host_supports_extension, EXTENSION_API_VERSION};

// --- crate root / sys re-exports ---

#[test]
fn get_mongodb_extension_export_symbol_bytes() {
    assert_eq!(
        extension_sdk_mongodb::GET_MONGODB_EXTENSION_SYMBOL,
        extension_sdk_mongodb::sys::GET_MONGODB_EXTENSION_SYMBOL,
    );
    assert!(extension_sdk_mongodb::GET_MONGODB_EXTENSION_SYMBOL.ends_with(b"\0"));
}

// --- version ---

#[test]
fn extension_api_version_matches_sys_constants() {
    assert_eq!(EXTENSION_API_VERSION.major, extension_sdk_mongodb::sys::MONGODB_EXTENSION_API_MAJOR_VERSION);
    assert_eq!(EXTENSION_API_VERSION.minor, extension_sdk_mongodb::sys::MONGODB_EXTENSION_API_MINOR_VERSION);
}

#[test]
fn host_supports_extension_rejects_empty_or_null_versions() {
    let empty = MongoExtensionAPIVersionVector { len: 0, versions: std::ptr::null_mut() };
    assert!(!host_supports_extension(&empty, EXTENSION_API_VERSION));

    let null_ptr = MongoExtensionAPIVersionVector {
        len: 1,
        versions: std::ptr::null_mut(),
    };
    assert!(!host_supports_extension(&null_ptr, EXTENSION_API_VERSION));
}

#[test]
fn host_supports_extension_accepts_compatible_slot() {
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor,
    }];
    let v = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    assert!(host_supports_extension(&v, EXTENSION_API_VERSION));
}

#[test]
fn host_supports_extension_minor_must_meet_extension_minor() {
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor.saturating_sub(1),
    }];
    let v = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    assert!(!host_supports_extension(&v, EXTENSION_API_VERSION));
}

#[test]
fn host_supports_extension_wrong_major() {
    let mut slots = [MongoExtensionAPIVersion { major: 99, minor: 99 }];
    let v = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    assert!(!host_supports_extension(&v, EXTENSION_API_VERSION));
}

// --- status ---

#[test]
fn status_ok_singleton() {
    let a = status::status_ok();
    let b = status::status_ok();
    assert_eq!(a, b);
    unsafe {
        let vt = (*a).vtable;
        assert_eq!(((*vt).get_code)(a), MONGO_EXTENSION_STATUS_OK);
        let r = ((*vt).get_reason)(a);
        assert!(r.data.is_null() && r.len == 0);
    }
}

#[test]
fn new_error_status_roundtrip_via_vtable() {
    let p = status::new_error_status(42, "hello reason");
    unsafe {
        let vt = (*p).vtable;
        assert_eq!(((*vt).get_code)(p), 42);
        let r = ((*vt).get_reason)(p);
        let bytes = std::slice::from_raw_parts(r.data.cast::<u8>(), r.len as usize);
        assert_eq!(bytes, b"hello reason");
        ((*vt).destroy)(p);
    }
}

// --- byte_buf ---

#[test]
fn into_raw_byte_buf_view_and_destroy() {
    let raw = byte_buf::into_raw_byte_buf(vec![1, 2, 3, 4]);
    unsafe {
        let vt = (*raw).vtable;
        let v = ((*vt).get_view)(raw);
        assert_eq!(std::slice::from_raw_parts(v.data, v.len as usize), &[1, 2, 3, 4]);
        ((*vt).destroy)(raw);
    }
}

#[test]
fn from_bson_roundtrip_view() {
    let d = doc! { "x": 1i32, "s": "hi" };
    let raw = byte_buf::from_bson(&d).expect("encode");
    unsafe {
        let vt = (*raw).vtable;
        let v = ((*vt).get_view)(raw);
        let encoded = std::slice::from_raw_parts(v.data, v.len as usize);
        let round = bson::Document::from_reader(encoded).expect("decode");
        assert_eq!(round, d);
        ((*vt).destroy)(raw);
    }
}

#[test]
fn from_bson_status_ok_destroys_output_buffer() {
    let d = doc! { "ok": true };
    let raw = byte_buf::from_bson_status(&d).expect("ok arm");
    unsafe {
        let vt = (*raw).vtable;
        let v = ((*vt).get_view)(raw);
        assert!(!v.data.is_null());
        ((*vt).destroy)(raw);
    }
}

// `from_bson_status` maps BSON encode errors to `new_error_status` (same object shape as
// `new_error_status_roundtrip_via_vtable`). The `bson` crate often accepts documents larger than
// the 16 MiB wire cap, so we do not rely on `to_writer` failing here.

// --- panics ---

#[test]
fn ffi_boundary_returns_some_on_success() {
    assert_eq!(ffi_boundary(|| 7u8), Some(7u8));
}

#[test]
fn ffi_boundary_returns_none_on_panic() {
    assert!(ffi_boundary(|| -> u8 { panic!("expected") }).is_none());
}

// --- host (mock portal / services; keep in one test — static OnceLock in host module) ---

unsafe extern "C" fn mock_register_stage_descriptor(
    _portal: *const MongoExtensionHostPortal,
    _descriptor: *const extension_sdk_mongodb::sys::MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_get_extension_options(
    _portal: *const MongoExtensionHostPortal,
) -> extension_sdk_mongodb::sys::MongoExtensionByteView {
    static OPTS: &[u8] = b"sharedLibraryPath: /tmp\n";
    extension_sdk_mongodb::sys::MongoExtensionByteView {
        data: OPTS.as_ptr(),
        len: OPTS.len() as u64,
    }
}

unsafe extern "C" fn mock_get_logger() -> *mut extension_sdk_mongodb::sys::MongoExtensionLogger {
    std::ptr::null_mut()
}

unsafe extern "C" fn mock_user_asserted(
    _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_tripwire_asserted(
    _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_mark_idle_thread_block(
    _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionIdleThreadBlock,
    _name: *const std::ffi::c_char,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_host_agg_stage_parse_node(
    _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_id_lookup(
    _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

#[test]
fn host_set_services_vtable_register_and_extension_options() {
    assert!(host::host_services_vtable().is_none());
    host::set_host_services(std::ptr::null());

    static HOST_PORTAL_VTABLE: MongoExtensionHostPortalVTable = MongoExtensionHostPortalVTable {
        register_stage_descriptor: mock_register_stage_descriptor,
        get_extension_options: mock_get_extension_options,
    };

    static HOST_SVCS_VTABLE: MongoExtensionHostServicesVTable = MongoExtensionHostServicesVTable {
        get_logger: mock_get_logger,
        user_asserted: mock_user_asserted,
        tripwire_asserted: mock_tripwire_asserted,
        mark_idle_thread_block: mock_mark_idle_thread_block,
        create_host_agg_stage_parse_node: mock_create_host_agg_stage_parse_node,
        create_id_lookup: mock_create_id_lookup,
    };

    let portal = MongoExtensionHostPortal {
        vtable: &HOST_PORTAL_VTABLE,
        host_extensions_api_version: EXTENSION_API_VERSION,
        host_mongodb_max_wire_version: 0,
    };
    let svcs = MongoExtensionHostServices {
        vtable: &HOST_SVCS_VTABLE,
    };

    host::set_host_services(std::ptr::from_ref(&svcs));
    let vt = host::host_services_vtable().expect("vtable after set");
    assert_eq!(vt as *const _, std::ptr::from_ref(&HOST_SVCS_VTABLE));

    unsafe {
        let view = host::extension_options_raw(std::ptr::from_ref(&portal));
        let slice = std::slice::from_raw_parts(view.data, view.len as usize);
        assert!(slice.starts_with(b"sharedLibraryPath:"));

        let st = host::register_stage_descriptor(std::ptr::from_ref(&portal), std::ptr::null());
        assert!(!st.is_null());
        let svt = (*st).vtable;
        assert_eq!(((*svt).get_code)(st), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(st);
    }
}

// --- get_extension_impl (failure paths only; success needs real host and loads globals once) ---

#[test]
fn get_extension_impl_rejects_null_pointers() {
    let globals = StageGlobals {
        name: "$x",
        expect_empty: true,
    };
    let mut out: *const extension_sdk_mongodb::sys::MongoExtension = std::ptr::null();
    unsafe {
        let st = get_extension_impl(globals, std::ptr::null(), std::ptr::addr_of_mut!(out));
        assert!(!st.is_null());
        let vt = (*st).vtable;
        assert_eq!(((*vt).get_code)(st), -1);
        ((*vt).destroy)(st);
    }
}

#[test]
fn get_extension_impl_rejects_incompatible_api_version() {
    let globals = StageGlobals {
        name: "$y",
        expect_empty: true,
    };
    let mut out: *const extension_sdk_mongodb::sys::MongoExtension = std::ptr::null();
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor.saturating_sub(1),
    }];
    let vec = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    unsafe {
        let st = get_extension_impl(globals, std::ptr::addr_of!(vec), std::ptr::addr_of_mut!(out));
        assert!(!st.is_null());
        let vt = (*st).vtable;
        assert_eq!(((*vt).get_code)(st), -1);
        ((*vt).destroy)(st);
    }
}
