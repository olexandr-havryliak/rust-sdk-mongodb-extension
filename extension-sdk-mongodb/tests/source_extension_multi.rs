//! Mock-host coverage for one source extension registering multiple stage descriptors.

mod common;

use std::sync::{Mutex, OnceLock};

use bson::{doc, Document};
use common::MockHost;
use extension_sdk_mongodb::source_stage::{get_multi_source_extension_impl, SourceOps};
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAggStageDescriptor, MongoExtensionAggStageParseNode,
    MongoExtensionByteView, MongoExtensionHostPortal, MongoExtensionStatus,
    MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;

static REGISTERED_NAMES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static REGISTERED_DESCRIPTORS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn registered_names() -> &'static Mutex<Vec<String>> {
    REGISTERED_NAMES.get_or_init(|| Mutex::new(Vec::new()))
}

fn registered_descriptors() -> &'static Mutex<Vec<usize>> {
    REGISTERED_DESCRIPTORS.get_or_init(|| Mutex::new(Vec::new()))
}

fn test_lock() -> &'static Mutex<()> {
    TEST_LOCK.get_or_init(|| Mutex::new(()))
}

unsafe extern "C" fn register_recording_name(
    _portal: *const MongoExtensionHostPortal,
    descriptor: *const MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    let vt = (*descriptor).vtable;
    let name = ((*vt).get_name)(descriptor);
    let bytes = std::slice::from_raw_parts(name.data, name.len as usize);
    registered_names()
        .lock()
        .expect("registered names mutex")
        .push(String::from_utf8_lossy(bytes).into_owned());
    registered_descriptors()
        .lock()
        .expect("registered descriptors mutex")
        .push(descriptor as usize);
    extension_sdk_mongodb::status::status_ok()
}

fn open_a(
    _doc: Document,
    _ctx: &mut extension_sdk_mongodb::StageContext,
) -> extension_sdk_mongodb::ExtensionResult<*mut std::ffi::c_void> {
    Ok(Box::into_raw(Box::new(0usize)) as *mut std::ffi::c_void)
}

fn open_b(
    _doc: Document,
    _ctx: &mut extension_sdk_mongodb::StageContext,
) -> extension_sdk_mongodb::ExtensionResult<*mut std::ffi::c_void> {
    Ok(Box::into_raw(Box::new(1usize)) as *mut std::ffi::c_void)
}

unsafe fn drop_usize(ptr: *mut std::ffi::c_void) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr as *mut usize));
    }
}

unsafe fn next_eof(
    _ptr: *mut std::ffi::c_void,
    _ctx: &mut extension_sdk_mongodb::StageContext,
) -> extension_sdk_mongodb::ExtensionResult<extension_sdk_mongodb::Next> {
    Ok(extension_sdk_mongodb::Next::Eof)
}

fn static_props() -> Document {
    extension_sdk_mongodb::StagePlan::source_default().static_properties_document()
}

fn expand_self(_doc: Document) -> extension_sdk_mongodb::ExtensionResult<extension_sdk_mongodb::Expansion> {
    Ok(extension_sdk_mongodb::Expansion::SelfStage)
}

static OPS_A: SourceOps = SourceOps {
    name: "$multiA",
    expect_empty: false,
    open_from_doc: open_a,
    drop_state: drop_usize,
    next: next_eof,
    on_extension_initialized: None,
    static_properties_doc: static_props,
    expand_inner: expand_self,
    merging_pipeline: None,
};

static OPS_B: SourceOps = SourceOps {
    name: "$multiB",
    expect_empty: false,
    open_from_doc: open_b,
    drop_state: drop_usize,
    next: next_eof,
    on_extension_initialized: None,
    static_properties_doc: static_props,
    expand_inner: expand_self,
    merging_pipeline: None,
};

#[test]
fn initialize_registers_every_source_descriptor() {
    let _guard = test_lock().lock().expect("test mutex");
    registered_names().lock().expect("registered names mutex").clear();
    registered_descriptors()
        .lock()
        .expect("registered descriptors mutex")
        .clear();
    let host = MockHost::new(register_recording_name);
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_multi_source_extension_impl(
            &[&OPS_A, &OPS_B],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let svt = (*st).vtable;
        assert_eq!(((*svt).get_code)(st), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(st);
        assert!(!out.is_null(), "extension pointer");

        let ev = (*out).vtable;
        let init_st = ((*ev).initialize)(out, std::ptr::from_ref(host.portal()));
        assert!(!init_st.is_null());
        let iv = (*init_st).vtable;
        assert_eq!(((*iv).get_code)(init_st), MONGO_EXTENSION_STATUS_OK);
        ((*iv).destroy)(init_st);
    }

    assert_eq!(
        *registered_names().lock().expect("registered names mutex"),
        vec!["$multiA".to_string(), "$multiB".to_string()]
    );
}

#[test]
fn each_descriptor_parses_and_names_its_own_stage() {
    let _guard = test_lock().lock().expect("test mutex");
    registered_names().lock().expect("registered names mutex").clear();
    registered_descriptors()
        .lock()
        .expect("registered descriptors mutex")
        .clear();
    let host = MockHost::new(register_recording_name);
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_multi_source_extension_impl(
            &[&OPS_A, &OPS_B],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        ((*(*st).vtable).destroy)(st);
        let ev = (*out).vtable;
        let init_st = ((*ev).initialize)(out, std::ptr::from_ref(host.portal()));
        ((*(*init_st).vtable).destroy)(init_st);

        let descriptors = registered_descriptors()
            .lock()
            .expect("registered descriptors mutex")
            .clone();
        assert_eq!(descriptors.len(), 2);

        let parsed_a = parse_with_descriptor(
            descriptors[0] as *const MongoExtensionAggStageDescriptor,
            doc! { "$multiA": { "a": 1i32 } },
        );
        let parsed_b = parse_with_descriptor(
            descriptors[1] as *const MongoExtensionAggStageDescriptor,
            doc! { "$multiB": { "b": 2i32 } },
        );
        assert_eq!(parse_node_name(parsed_a), "$multiA");
        assert_eq!(parse_node_name(parsed_b), "$multiB");
        ((*(*parsed_a).vtable).destroy)(parsed_a);
        ((*(*parsed_b).vtable).destroy)(parsed_b);
    }
}

#[test]
fn descriptor_rejects_wrong_stage_name() {
    let _guard = test_lock().lock().expect("test mutex");
    registered_descriptors()
        .lock()
        .expect("registered descriptors mutex")
        .clear();
    let host = MockHost::new(register_recording_name);
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_multi_source_extension_impl(
            &[&OPS_A, &OPS_B],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        ((*(*st).vtable).destroy)(st);
        let ev = (*out).vtable;
        let init_st = ((*ev).initialize)(out, std::ptr::from_ref(host.portal()));
        ((*(*init_st).vtable).destroy)(init_st);

        let descriptor = registered_descriptors()
            .lock()
            .expect("registered descriptors mutex")[0]
            as *const MongoExtensionAggStageDescriptor;
        let mut bytes = Vec::new();
        doc! { "$multiB": {} }.to_writer(&mut bytes).unwrap();
        let mut parsed: *mut MongoExtensionAggStageParseNode = std::ptr::null_mut();
        let vt = (*descriptor).vtable;
        let status = ((*vt).parse)(
            descriptor,
            MongoExtensionByteView {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            std::ptr::addr_of_mut!(parsed),
        );
        assert!(!status.is_null());
        assert!(parsed.is_null());
        let svt = (*status).vtable;
        assert_ne!(((*svt).get_code)(status), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(status);
    }
}

unsafe fn parse_with_descriptor(
    descriptor: *const MongoExtensionAggStageDescriptor,
    stage: Document,
) -> *mut MongoExtensionAggStageParseNode {
    let mut bytes = Vec::new();
    stage.to_writer(&mut bytes).unwrap();
    let mut parsed: *mut MongoExtensionAggStageParseNode = std::ptr::null_mut();
    let vt = (*descriptor).vtable;
    let status = ((*vt).parse)(
        descriptor,
        MongoExtensionByteView {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        std::ptr::addr_of_mut!(parsed),
    );
    assert!(!status.is_null());
    let svt = (*status).vtable;
    assert_eq!(((*svt).get_code)(status), MONGO_EXTENSION_STATUS_OK);
    ((*svt).destroy)(status);
    assert!(!parsed.is_null());
    parsed
}

unsafe fn parse_node_name(parsed: *const MongoExtensionAggStageParseNode) -> String {
    let vt = (*parsed).vtable;
    let name = ((*vt).get_name)(parsed);
    let bytes = std::slice::from_raw_parts(name.data, name.len as usize);
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn rejects_empty_stage_list() {
    let _guard = test_lock().lock().expect("test mutex");
    let host = MockHost::new(register_recording_name);
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_multi_source_extension_impl(
            &[],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let svt = (*st).vtable;
        assert_ne!(((*svt).get_code)(st), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(st);
    }
}

#[test]
fn rejects_duplicate_stage_names() {
    let _guard = test_lock().lock().expect("test mutex");
    let host = MockHost::new(register_recording_name);
    let mut out: *const MongoExtension = std::ptr::null();
    unsafe {
        let st = get_multi_source_extension_impl(
            &[&OPS_A, &OPS_A],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(out),
        );
        assert!(!st.is_null());
        let svt = (*st).vtable;
        assert_ne!(((*svt).get_code)(st), MONGO_EXTENSION_STATUS_OK);
        ((*svt).destroy)(st);
    }
}
