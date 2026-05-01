//! Shared mocks for integration tests (`mod common;` from each `tests/*.rs` crate).

#![allow(dead_code)]

use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::{
    MongoExtensionAggStageAstNode, MongoExtensionAggStageDescriptor, MongoExtensionAggStageParseNode,
    MongoExtensionHostPortal, MongoExtensionHostPortalVTable, MongoExtensionHostServices,
    MongoExtensionHostServicesVTable, MongoExtensionLogger, MongoExtensionStatus,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

pub unsafe extern "C" fn mock_register_ok(
    _portal: *const MongoExtensionHostPortal,
    _descriptor: *const MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub unsafe extern "C" fn mock_get_extension_options(
    _portal: *const MongoExtensionHostPortal,
) -> extension_sdk_mongodb::sys::MongoExtensionByteView {
    static OPTS: &[u8] = b"sharedLibraryPath: /tmp/x.so\n";
    extension_sdk_mongodb::sys::MongoExtensionByteView {
        data: OPTS.as_ptr(),
        len: OPTS.len() as u64,
    }
}

pub unsafe extern "C" fn mock_get_logger() -> *mut MongoExtensionLogger {
    std::ptr::null_mut()
}

pub unsafe extern "C" fn mock_user_asserted(
    _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub unsafe extern "C" fn mock_tripwire_asserted(
    _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub unsafe extern "C" fn mock_mark_idle_thread_block(
    _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionIdleThreadBlock,
    _name: *const std::ffi::c_char,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub unsafe extern "C" fn mock_create_host_agg_stage_parse_node(
    _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _out: *mut *mut MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub unsafe extern "C" fn mock_create_id_lookup(
    _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _out: *mut *mut MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

pub fn leak_portal_and_services(
    register: unsafe extern "C" fn(
        *const MongoExtensionHostPortal,
        *const MongoExtensionAggStageDescriptor,
    ) -> *mut MongoExtensionStatus,
) -> (
    &'static MongoExtensionHostPortal,
    &'static MongoExtensionHostServices,
) {
    let portal_vt = Box::leak(Box::new(MongoExtensionHostPortalVTable {
        register_stage_descriptor: register,
        get_extension_options: mock_get_extension_options,
    }));
    let svcs_vt = Box::leak(Box::new(MongoExtensionHostServicesVTable {
        get_logger: mock_get_logger,
        user_asserted: mock_user_asserted,
        tripwire_asserted: mock_tripwire_asserted,
        mark_idle_thread_block: mock_mark_idle_thread_block,
        create_host_agg_stage_parse_node: mock_create_host_agg_stage_parse_node,
        create_id_lookup: mock_create_id_lookup,
    }));
    let portal = Box::leak(Box::new(MongoExtensionHostPortal {
        vtable: portal_vt,
        host_extensions_api_version: EXTENSION_API_VERSION,
        host_mongodb_max_wire_version: 0,
    }));
    let svcs = Box::leak(Box::new(MongoExtensionHostServices {
        vtable: svcs_vt,
    }));
    (portal, svcs)
}
