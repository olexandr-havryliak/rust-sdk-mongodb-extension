//! `get_map_extension_impl` rejects null / incompatible API before installing globals.

use bson::{doc, Document};
use extension_sdk_mongodb::map_transform::{get_map_extension_impl, MapStageGlobals};
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAPIVersion, MongoExtensionAPIVersionVector,
};
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
    }
}

#[test]
fn get_map_extension_impl_rejects_null_version_vector() {
    let g = globals();
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_map_extension_impl(g, std::ptr::null(), std::ptr::addr_of_mut!(out));
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
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor.saturating_sub(1),
    }];
    let vec = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    unsafe {
        let st = get_map_extension_impl(g, std::ptr::addr_of!(vec), std::ptr::addr_of_mut!(out));
        assert!(!st.is_null());
        let vt = (*st).vtable;
        assert_eq!(((*vt).get_code)(st), -1);
        ((*vt).destroy)(st);
    }
}
