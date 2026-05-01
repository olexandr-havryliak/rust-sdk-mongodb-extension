//! Mock-host coverage: `get_map_extension_impl` + `initialize` runs `on_extension_initialized` before register.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use bson::{doc, Document};
use common::{leak_portal_and_services, mock_register_ok};
use extension_sdk_mongodb::map_transform::{get_map_extension_impl, MapStageGlobals};
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAPIVersion, MongoExtensionAPIVersionVector, MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

static INIT_HOOK_RAN: AtomicBool = AtomicBool::new(false);

unsafe fn on_init(_portal: *const extension_sdk_mongodb::sys::MongoExtensionHostPortal) -> Result<(), String> {
    INIT_HOOK_RAN.store(true, Ordering::SeqCst);
    Ok(())
}

fn tr(_row: &Document, args: &Document) -> Result<Document, String> {
    Ok(args.clone())
}

fn eof(args: &Document) -> Result<Document, String> {
    Ok(doc! { "eof": true, "n": args.get("n").cloned().unwrap_or(bson::Bson::Null) })
}

fn compatible_vec() -> (MongoExtensionAPIVersionVector, [MongoExtensionAPIVersion; 1]) {
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor,
    }];
    let v = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    (v, slots)
}

#[test]
fn map_initialize_runs_on_extension_initialized_then_register() {
    INIT_HOOK_RAN.store(false, Ordering::SeqCst);
    let (portal, svcs) = leak_portal_and_services(mock_register_ok);
    let (vec, _slots) = compatible_vec();
    let globals = MapStageGlobals {
        name: "$mapSdkInitTest",
        expect_empty: false,
        transform: tr,
        on_eof_no_rows: Some(eof),
        on_extension_initialized: Some(on_init),
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
        assert!(!out.is_null(), "extension pointer");

        let ev = (*out).vtable;
        let init_st = ((*ev).initialize)(out, std::ptr::from_ref(portal), std::ptr::from_ref(svcs));
        assert!(!init_st.is_null());
        let iv = (*init_st).vtable;
        assert_eq!(((*iv).get_code)(init_st), MONGO_EXTENSION_STATUS_OK);
        ((*iv).destroy)(init_st);
    }
    assert!(
        INIT_HOOK_RAN.load(Ordering::SeqCst),
        "on_extension_initialized should run during initialize"
    );
}
