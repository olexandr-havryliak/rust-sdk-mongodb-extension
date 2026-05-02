//! Mock-host coverage: `get_map_extension_impl` + `initialize` runs `on_extension_initialized` before register.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use bson::{doc, Document};
use common::{mock_register_ok, MockHost};
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

/// Build a host version vector pointing at `slots`. `slots` must outlive the returned struct
/// (do not build the vector inside a helper that then moves `slots` — the raw pointer would dangle).
fn compatible_vec(slots: &mut [MongoExtensionAPIVersion; 1]) -> MongoExtensionAPIVersionVector {
    MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    }
}

#[test]
fn map_initialize_runs_on_extension_initialized_then_register() {
    INIT_HOOK_RAN.store(false, Ordering::SeqCst);
    let host = MockHost::new(mock_register_ok);
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor,
    }];
    let vec = compatible_vec(&mut slots);
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
        let init_st = ((*ev).initialize)(
            out,
            std::ptr::from_ref(host.portal()),
            std::ptr::from_ref(host.services()),
        );
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
