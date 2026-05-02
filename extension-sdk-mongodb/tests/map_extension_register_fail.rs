//! If `register_stage_descriptor` fails, `initialize` returns an error (hook still ran first).

mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use bson::{doc, Document};
use common::MockHost;
use extension_sdk_mongodb::default_map_stage_static_properties;
use extension_sdk_mongodb::map_transform::{get_map_extension_impl, MapStageGlobals};
use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAPIVersion, MongoExtensionAPIVersionVector,
    MongoExtensionAggStageDescriptor, MongoExtensionHostPortal, MongoExtensionStatus,
    MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

static INIT_HOOK_RAN: AtomicBool = AtomicBool::new(false);

unsafe fn on_init(_portal: *const extension_sdk_mongodb::sys::MongoExtensionHostPortal) -> Result<(), String> {
    INIT_HOOK_RAN.store(true, Ordering::SeqCst);
    Ok(())
}

unsafe extern "C" fn mock_register_fail(
    _portal: *const MongoExtensionHostPortal,
    _descriptor: *const MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    status::new_error_status(99, "register_stage_descriptor failed (mock)")
}

fn tr(_row: &Document, _args: &Document) -> Result<Document, String> {
    Ok(doc! {})
}

fn eof(_args: &Document) -> Result<Document, String> {
    Ok(doc! {})
}

#[test]
fn map_initialize_fails_when_register_fails_after_hook() {
    INIT_HOOK_RAN.store(false, Ordering::SeqCst);
    let host = MockHost::new(mock_register_fail);
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor,
    }];
    let vec = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    let globals = MapStageGlobals {
        name: "$mapSdkRegFail",
        expect_empty: false,
        transform: tr,
        on_eof_no_rows: Some(eof),
        on_extension_initialized: Some(on_init),
        static_properties_doc: default_map_stage_static_properties,
        expand_from_args_doc: None,
    };
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_map_extension_impl(
            globals,
            std::ptr::addr_of!(vec),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let svt = (*st).vtable;
        assert_eq!(((*svt).get_code)(st), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(st);
        assert!(!out.is_null());

        let ev = (*out).vtable;
        let init_st = ((*ev).initialize)(
            out,
            std::ptr::from_ref(host.portal()),
            std::ptr::from_ref(host.services()),
        );
        assert!(!init_st.is_null());
        let iv = (*init_st).vtable;
        assert_ne!(((*iv).get_code)(init_st), MONGO_EXTENSION_STATUS_OK);
        ((*iv).destroy)(init_st);
    }
    assert!(INIT_HOOK_RAN.load(Ordering::SeqCst));
}
