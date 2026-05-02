//! Extension options snapshot cached at `initialize`.

use extension_sdk_mongodb::host;
use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::{
    MongoExtensionHostPortal, MongoExtensionHostPortalVTable, MongoExtensionHostServices,
    MongoExtensionHostServicesVTable, MongoExtensionStatus,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

unsafe extern "C" fn mock_register(
    _: *const MongoExtensionHostPortal,
    _: *const extension_sdk_mongodb::sys::MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_get_opts(_: *const MongoExtensionHostPortal) -> extension_sdk_mongodb::sys::MongoExtensionByteView {
    static OPTS: &[u8] = b"sharedLibraryPath: /tmp/lib.so\n";
    extension_sdk_mongodb::sys::MongoExtensionByteView {
        data: OPTS.as_ptr(),
        len: OPTS.len() as u64,
    }
}

unsafe extern "C" fn mock_get_logger() -> *mut extension_sdk_mongodb::sys::MongoExtensionLogger {
    std::ptr::null_mut()
}

unsafe extern "C" fn mock_user_asserted(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_tripwire(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_mark_idle(
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionIdleThreadBlock,
    _: *const std::ffi::c_char,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_parse(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_id(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

#[test]
fn cache_extension_options_from_portal_populates_snapshot() {
    host::reset_extension_options_snapshot_for_tests();
    host::set_host_services(std::ptr::null());

    static PORTAL_VT: MongoExtensionHostPortalVTable = MongoExtensionHostPortalVTable {
        register_stage_descriptor: mock_register,
        get_extension_options: mock_get_opts,
    };
    static SVCS_VT: MongoExtensionHostServicesVTable = MongoExtensionHostServicesVTable {
        get_logger: mock_get_logger,
        user_asserted: mock_user_asserted,
        tripwire_asserted: mock_tripwire,
        mark_idle_thread_block: mock_mark_idle,
        create_host_agg_stage_parse_node: mock_create_parse,
        create_id_lookup: mock_create_id,
    };
    let portal = MongoExtensionHostPortal {
        vtable: &PORTAL_VT,
        host_extensions_api_version: EXTENSION_API_VERSION,
        host_mongodb_max_wire_version: 0,
    };
    let svcs = MongoExtensionHostServices {
        vtable: &SVCS_VT,
    };
    host::set_host_services(std::ptr::from_ref(&svcs));

    unsafe {
        host::cache_extension_options_from_portal(std::ptr::from_ref(&portal));
    }

    let snap = host::extension_options_snapshot().expect("snapshot");
    assert!(snap.starts_with(b"sharedLibraryPath:"));
}
