//! `get_map_extension_impl` rejects null / incompatible API before installing globals.

mod common;

use bson::{doc, Document};
use extension_sdk_mongodb::default_map_stage_static_properties;
use extension_sdk_mongodb::map_transform::{get_map_extension_impl, MapStageGlobals};
use extension_sdk_mongodb::sys::{MongoExtension, MongoExtensionAPIVersion};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

fn tr(_row: &Document, _args: &Document) -> Result<Document, String> {
    Ok(doc! {})
}

fn globals() -> MapStageGlobals {
    MapStageGlobals {
        name: "$mapSdkEntryFail",
        expect_empty: false,
        transform: tr,
        on_eof_no_rows: None,
        on_extension_initialized: None,
        static_properties_doc: default_map_stage_static_properties,
        expand_from_args_doc: None,
    }
}

#[test]
fn get_map_extension_impl_rejects_null_host_services() {
    let g = globals();
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_map_extension_impl(
            g,
            EXTENSION_API_VERSION,
            std::ptr::null(),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let vt = (*st).vtable;
        assert_eq!(((*vt).get_code)(st), -1);
        ((*vt).destroy)(st);
    }
}

#[test]
fn get_map_extension_impl_rejects_incompatible_api_version() {
    let g = globals();
    let mut out: *const MongoExtension = std::ptr::null();
    let version = MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major + 1,
        minor: EXTENSION_API_VERSION.minor,
    };
    let host = common::MockHost::new(common::mock_register_ok);
    unsafe {
        let st = get_map_extension_impl(
            g,
            version,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let vt = (*st).vtable;
        assert_eq!(((*vt).get_code)(st), -1);
        ((*vt).destroy)(st);
    }
}
