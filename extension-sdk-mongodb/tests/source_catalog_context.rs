//! Catalog metadata captured at bind must be visible on every generator `next` call.

mod common;

use std::sync::{Mutex, OnceLock};

use bson::doc;
use common::MockHost;
use extension_sdk_mongodb::source_stage::{get_multi_source_extension_impl, SourceOps};
use extension_sdk_mongodb::sys::{
    MongoExtension, MongoExtensionAggStageAstNode, MongoExtensionAggStageDescriptor,
    MongoExtensionAggStageNodeType, MongoExtensionAggStageParseNode, MongoExtensionByteContainer,
    MongoExtensionByteContainerBytes, MongoExtensionByteContainerType, MongoExtensionByteView,
    MongoExtensionCatalogContext, MongoExtensionExecAggStage, MongoExtensionExpandedArray,
    MongoExtensionExpandedArrayContainer, MongoExtensionExpandedArrayElement,
    MongoExtensionExpandedArrayElementUnion, MongoExtensionExplainVerbosity,
    MongoExtensionGetNextResult, MongoExtensionGetNextResultCode, MongoExtensionHostPortal,
    MongoExtensionLogicalAggStage, MongoExtensionNamespaceString, MongoExtensionStatus,
    MONGO_EXTENSION_STATUS_OK,
};
use extension_sdk_mongodb::version::EXTENSION_API_VERSION;
use extension_sdk_mongodb::StageContext;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedCatalog {
    namespace: String,
    uuid: Option<String>,
    in_router: bool,
}

static OBSERVED: OnceLock<Mutex<Vec<Option<ObservedCatalog>>>> = OnceLock::new();
static DESCRIPTOR: OnceLock<Mutex<usize>> = OnceLock::new();

fn observed() -> &'static Mutex<Vec<Option<ObservedCatalog>>> {
    OBSERVED.get_or_init(|| Mutex::new(Vec::new()))
}

fn descriptor_slot() -> &'static Mutex<usize> {
    DESCRIPTOR.get_or_init(|| Mutex::new(0))
}

fn record_catalog(ctx: &StageContext) {
    let snap = ctx.catalog().map(|catalog| ObservedCatalog {
        namespace: catalog.namespace(),
        uuid: catalog.uuid.clone(),
        in_router: catalog.in_router,
    });
    observed().lock().expect("observed catalog").push(snap);
}

fn open_recording(
    _doc: bson::Document,
    ctx: &mut StageContext,
) -> extension_sdk_mongodb::ExtensionResult<*mut std::ffi::c_void> {
    record_catalog(ctx);
    Ok(Box::into_raw(Box::new(0usize)) as *mut std::ffi::c_void)
}

unsafe fn drop_usize(ptr: *mut std::ffi::c_void) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr as *mut usize));
    }
}

unsafe fn next_recording(
    ptr: *mut std::ffi::c_void,
    ctx: &mut StageContext,
) -> extension_sdk_mongodb::ExtensionResult<extension_sdk_mongodb::Next> {
    record_catalog(ctx);
    let state = &mut *(ptr as *mut usize);
    if *state == 0 {
        *state = 1;
        return Ok(extension_sdk_mongodb::Next::Advanced {
            document: doc! { "n": 1i32 },
            metadata: None,
        });
    }
    Ok(extension_sdk_mongodb::Next::Eof)
}

fn static_props() -> bson::Document {
    extension_sdk_mongodb::StagePlan::source_default().static_properties_document()
}

fn expand_self(
    _doc: bson::Document,
) -> extension_sdk_mongodb::ExtensionResult<extension_sdk_mongodb::Expansion> {
    Ok(extension_sdk_mongodb::Expansion::SelfStage)
}

static OPS: SourceOps = SourceOps {
    name: "$catalogProbe",
    expect_empty: false,
    open_from_doc: open_recording,
    drop_state: drop_usize,
    next: next_recording,
    on_extension_initialized: None,
    static_properties_doc: static_props,
    expand_inner: expand_self,
};

unsafe extern "C" fn register_keep_descriptor(
    _portal: *const MongoExtensionHostPortal,
    descriptor: *const MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    *descriptor_slot().lock().expect("descriptor slot") = descriptor as usize;
    extension_sdk_mongodb::status::status_ok()
}

unsafe fn expect_ok(status: *mut MongoExtensionStatus) {
    assert!(!status.is_null(), "missing status");
    let vt = (*status).vtable;
    assert_eq!(((*vt).get_code)(status), MONGO_EXTENSION_STATUS_OK);
    ((*vt).destroy)(status);
}

fn byte_view(bytes: &[u8]) -> MongoExtensionByteView {
    MongoExtensionByteView {
        data: bytes.as_ptr(),
        len: bytes.len() as u64,
    }
}

fn empty_container() -> MongoExtensionByteContainer {
    MongoExtensionByteContainer {
        type_: MongoExtensionByteContainerType::kByteView,
        bytes: MongoExtensionByteContainerBytes {
            view: MongoExtensionByteView {
                data: std::ptr::null(),
                len: 0,
            },
        },
    }
}

unsafe fn release_container(container: &MongoExtensionByteContainer) {
    if container.type_ == MongoExtensionByteContainerType::kByteBuf {
        let buf = container.bytes.buf;
        if !buf.is_null() {
            let vt = (*buf).vtable;
            ((*vt).destroy)(buf);
        }
    }
}

#[test]
fn generator_next_sees_catalog_bound_at_ast_bind() {
    observed().lock().expect("observed catalog").clear();
    let host = MockHost::new(register_keep_descriptor);
    let mut extension: *const MongoExtension = std::ptr::null();
    let expected = ObservedCatalog {
        namespace: "search_demo.products".to_string(),
        uuid: Some("11111111-1111-1111-1111-111111111111".to_string()),
        in_router: false,
    };

    unsafe {
        let status = get_multi_source_extension_impl(
            &[&OPS],
            EXTENSION_API_VERSION,
            std::ptr::from_ref(host.services()),
            std::ptr::addr_of_mut!(extension),
        );
        expect_ok(status);
        let init = ((*(*extension).vtable).initialize)(extension, std::ptr::from_ref(host.portal()));
        expect_ok(init);

        let descriptor = descriptor_slot().lock().expect("descriptor slot").clone()
            as *const MongoExtensionAggStageDescriptor;
        assert!(!descriptor.is_null());

        let mut bytes = Vec::new();
        doc! { "$catalogProbe": {} }.to_writer(&mut bytes).unwrap();
        let mut parsed: *mut MongoExtensionAggStageParseNode = std::ptr::null_mut();
        let parse_status = ((*(*descriptor).vtable).parse)(
            descriptor,
            byte_view(&bytes),
            std::ptr::addr_of_mut!(parsed),
        );
        expect_ok(parse_status);

        let mut expanded: *mut MongoExtensionExpandedArrayContainer = std::ptr::null_mut();
        let expand_status = ((*(*parsed).vtable).expand)(parsed, std::ptr::addr_of_mut!(expanded));
        expect_ok(expand_status);
        assert_eq!(((*(*expanded).vtable).size)(expanded), 1);

        let mut element = MongoExtensionExpandedArrayElement {
            type_: MongoExtensionAggStageNodeType::kParseNode,
            parse_or_ast: MongoExtensionExpandedArrayElementUnion {
                parse: std::ptr::null_mut(),
            },
        };
        let mut expanded_array = MongoExtensionExpandedArray {
            size: 1,
            elements: std::ptr::addr_of_mut!(element),
        };
        let transfer_status =
            ((*(*expanded).vtable).transfer)(expanded, std::ptr::addr_of_mut!(expanded_array));
        expect_ok(transfer_status);
        ((*(*expanded).vtable).destroy)(expanded);
        assert_eq!(
            element.type_ as u32,
            MongoExtensionAggStageNodeType::kAstNode as u32
        );
        let ast: *mut MongoExtensionAggStageAstNode = element.parse_or_ast.ast;

        let db = b"search_demo";
        let coll = b"products";
        let uuid = b"11111111-1111-1111-1111-111111111111";
        let catalog = MongoExtensionCatalogContext {
            namespace_string: MongoExtensionNamespaceString {
                database_name: byte_view(db),
                collection_name: byte_view(coll),
            },
            uuid_string: byte_view(uuid),
            in_router: 0,
            verbosity: MongoExtensionExplainVerbosity::kNotExplain,
        };
        let mut logical: *mut MongoExtensionLogicalAggStage = std::ptr::null_mut();
        let bind_status = ((*(*ast).vtable).bind)(
            ast,
            std::ptr::from_ref(&catalog),
            std::ptr::addr_of_mut!(logical),
        );
        expect_ok(bind_status);
        ((*(*ast).vtable).destroy)(ast);

        let mut exec: *mut MongoExtensionExecAggStage = std::ptr::null_mut();
        let compile_status = ((*(*logical).vtable).compile)(logical, std::ptr::addr_of_mut!(exec));
        expect_ok(compile_status);
        ((*(*logical).vtable).destroy)(logical);

        for _ in 0..2 {
            let mut result = MongoExtensionGetNextResult {
                code: MongoExtensionGetNextResultCode::kEOF,
                result_document: empty_container(),
                result_metadata: empty_container(),
            };
            let next_status = ((*(*exec).vtable).get_next)(
                exec,
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(result),
            );
            expect_ok(next_status);
            release_container(&result.result_document);
            release_container(&result.result_metadata);
        }
        ((*(*exec).vtable).destroy)(exec);
        ((*(*parsed).vtable).destroy)(parsed);
    }

    let seen = observed().lock().expect("observed catalog").clone();
    assert_eq!(
        seen,
        vec![Some(expected.clone()), Some(expected.clone()), Some(expected)],
        "open and every next call must see the catalog captured at bind"
    );
}
