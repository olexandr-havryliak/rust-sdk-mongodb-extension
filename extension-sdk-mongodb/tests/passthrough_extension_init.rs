//! Passthrough extension: `get_extension_impl` + mocked `initialize` registers the descriptor.

mod common;

use common::{mock_register_ok, MockHost};
use extension_sdk_mongodb::default_map_stage_static_properties;
use extension_sdk_mongodb::passthrough::{get_extension_impl, StageGlobals};
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAPIVersion, MongoExtensionAPIVersionVector, MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

#[test]
fn passthrough_initialize_succeeds_with_mock_host() {
    let host = MockHost::new(mock_register_ok);
    let mut slots = [MongoExtensionAPIVersion {
        major: EXTENSION_API_VERSION.major,
        minor: EXTENSION_API_VERSION.minor,
    }];
    let vec = MongoExtensionAPIVersionVector {
        len: 1,
        versions: slots.as_mut_ptr(),
    };
    let globals = StageGlobals {
        name: "$passSdkInitTest",
        expect_empty: false,
        static_properties_doc: default_map_stage_static_properties,
        expand_from_args_doc: None,
    };
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_extension_impl(globals, std::ptr::addr_of!(vec), std::ptr::addr_of_mut!(out));
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
        assert_eq!(((*iv).get_code)(init_st), MONGO_EXTENSION_STATUS_OK);
        ((*iv).destroy)(init_st);
    }
}
