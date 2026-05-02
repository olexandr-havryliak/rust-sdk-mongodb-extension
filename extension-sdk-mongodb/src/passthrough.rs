//! Minimal **passthrough transform** stage: parses `{$stageName: <args>}`, forwards documents from the
//! upstream executable stage unchanged.
//!
//! Use [`export_transform_stage`] from the crate root.

use std::cell::Cell;
use std::sync::OnceLock;

use bson::Document;

use crate::byte_buf;
use crate::host;
use crate::panics::ffi_boundary;
use crate::status;
use crate::sys::{
    MongoExtension, MongoExtensionAggStageAstNode, MongoExtensionAggStageAstNodeVTable,
    MongoExtensionAggStageDescriptor, MongoExtensionAggStageDescriptorVTable,
    MongoExtensionAggStageParseNode, MongoExtensionAggStageParseNodeVTable,
    MongoExtensionByteView, MongoExtensionCatalogContext, MongoExtensionDistributedPlanLogic,
    MongoExtensionExecAggStage, MongoExtensionExecAggStageVTable,     MongoExtensionExpandedArray,
    MongoExtensionExpandedArrayContainer, MongoExtensionExpandedArrayContainerVTable,
    MongoExtensionExpandedArrayElementUnion,
    MongoExtensionExplainVerbosity, MongoExtensionFirstStageViewApplicationPolicy,
    MongoExtensionGetNextResult, MongoExtensionLogicalAggStage, MongoExtensionLogicalAggStageVTable,
    MongoExtensionOperationMetrics, MongoExtensionOperationMetricsVTable,
    MongoExtensionQueryExecutionContext, MongoExtensionStatus, MongoExtensionVTable,
    MongoExtensionViewInfo,     MongoExtensionAggStageNodeType, MongoExtensionByteContainer, MongoExtensionByteContainerType,
    MongoExtensionGetNextResultCode,
};
use crate::version::{host_supports_extension, EXTENSION_API_VERSION};

/// Stage name and parse options shared by generated `get_mongodb_extension`.
#[derive(Clone, Copy)]
pub struct StageGlobals {
    /// Stage key including leading `$`, e.g. `"$myRustPass"`.
    pub name: &'static str,
    /// If true, the inner document must be empty (like `$testFoo: {}`).
    pub expect_empty: bool,
}

static ACTIVE_STAGE: OnceLock<StageGlobals> = OnceLock::new();

fn globals() -> StageGlobals {
    *ACTIVE_STAGE.get().expect("extension globals not installed")
}

fn name_bytes() -> &'static [u8] {
    globals().name.as_bytes()
}

fn name_view() -> MongoExtensionByteView {
    let b = name_bytes();
    MongoExtensionByteView {
        data: b.as_ptr(),
        len: b.len() as u64,
    }
}

// --- Descriptor ---

#[repr(C)]
struct DescriptorObj {
    base: MongoExtensionAggStageDescriptor,
}

unsafe extern "C" fn desc_get_name(_: *const MongoExtensionAggStageDescriptor) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn desc_parse(
    _: *const MongoExtensionAggStageDescriptor,
    stage_bson: MongoExtensionByteView,
    out_parse: *mut *mut MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    *out_parse = std::ptr::null_mut();
    let parsed = ffi_boundary(|| -> Result<*mut MongoExtensionAggStageParseNode, String> {
        let bytes = std::slice::from_raw_parts(stage_bson.data, stage_bson.len as usize);
        let doc = Document::from_reader(bytes).map_err(|e| format!("parse bson: {e}"))?;
        let g = globals();
        let key = doc
            .keys()
            .next()
            .ok_or_else(|| "stage document must have one field".to_string())?;
        if key != g.name {
            return Err(format!("expected stage {}, got {key}", g.name));
        }
        let args = doc.get_document(key).map_err(|e| e.to_string())?;
        if g.expect_empty && !args.is_empty() {
            return Err("stage definition must be an empty object".into());
        }
        let mut arg_bytes = Vec::new();
        args.to_writer(&mut arg_bytes).map_err(|e| e.to_string())?;
        let p = Box::into_raw(Box::new(parse_alloc(arg_bytes)));
        Ok(p.cast::<MongoExtensionAggStageParseNode>())
    });
    match parsed {
        None => status::new_error_status(-1, "extension panic during parse"),
        Some(Err(e)) => status::new_error_status(-1, e),
        Some(Ok(p)) => {
            *out_parse = p;
            status::status_ok()
        }
    }
}

static DESCRIPTOR_VTABLE: MongoExtensionAggStageDescriptorVTable = MongoExtensionAggStageDescriptorVTable {
    get_name: desc_get_name,
    parse: desc_parse,
};

// --- Parse node ---

#[repr(C)]
struct ParseObj {
    base: MongoExtensionAggStageParseNode,
    args: Vec<u8>,
}

fn parse_alloc(args: Vec<u8>) -> ParseObj {
    ParseObj {
        base: MongoExtensionAggStageParseNode {
            vtable: &PARSE_VTABLE,
        },
        args,
    }
}

unsafe extern "C" fn parse_destroy(p: *mut MongoExtensionAggStageParseNode) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<ParseObj>()));
}

unsafe extern "C" fn parse_get_name(_: *const MongoExtensionAggStageParseNode) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn parse_get_query_shape(
    p: *const MongoExtensionAggStageParseNode,
    _ctx: *const crate::sys::MongoExtensionHostQueryShapeOpts,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let r = ffi_boundary(|| -> Result<*mut crate::sys::MongoExtensionByteBuf, String> {
        let this = p.cast::<ParseObj>();
        let g = globals();
        let args_bytes: &[u8] = unsafe { &(*this).args };
        let args = Document::from_reader(args_bytes).map_err(|e| e.to_string())?;
        let d = bson::doc! { g.name: args };
        byte_buf::from_bson(&d).map_err(|e| e.to_string())
    });
    match r {
        None => status::new_error_status(-1, "panic during get_query_shape"),
        Some(Err(e)) => status::new_error_status(-1, e),
        Some(Ok(b)) => {
            *out = b;
            status::status_ok()
        }
    }
}

unsafe extern "C" fn parse_expand(
    p: *const MongoExtensionAggStageParseNode,
    out: *mut *mut MongoExtensionExpandedArrayContainer,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let r = ffi_boundary(|| -> Result<*mut MongoExtensionExpandedArrayContainer, String> {
        let this = p.cast::<ParseObj>();
        let args = (*this).args.clone();
        let ast = Box::into_raw(Box::new(ast_alloc(args))).cast::<MongoExtensionAggStageAstNode>();
        let c = Box::new(expanded_single(ast));
        Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
    });
    match r {
        None => status::new_error_status(-1, "panic during expand"),
        Some(Err(e)) => status::new_error_status(-1, e),
        Some(Ok(c)) => {
            *out = c;
            status::status_ok()
        }
    }
}

unsafe extern "C" fn parse_clone(
    p: *const MongoExtensionAggStageParseNode,
    out: *mut *mut MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ParseObj>();
    let c = Box::into_raw(Box::new(parse_alloc((*this).args.clone()))).cast::<MongoExtensionAggStageParseNode>();
    *out = c;
    status::status_ok()
}

unsafe extern "C" fn parse_to_bson_for_log(
    p: *const MongoExtensionAggStageParseNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ParseObj>();
    let g = globals();
    let args_bytes: &[u8] = unsafe { &(*this).args };
    let args = match Document::from_reader(args_bytes) {
        Ok(d) => d,
        Err(_) => {
            *out = std::ptr::null_mut();
            return status::new_error_status(-1, "log bson");
        }
    };
    let d = bson::doc! { g.name: args };
    match byte_buf::from_bson(&d) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            status::new_error_status(-1, e.to_string())
        }
    }
}

static PARSE_VTABLE: MongoExtensionAggStageParseNodeVTable = MongoExtensionAggStageParseNodeVTable {
    destroy: parse_destroy,
    get_name: parse_get_name,
    get_query_shape: parse_get_query_shape,
    expand: parse_expand,
    clone: parse_clone,
    to_bson_for_log: parse_to_bson_for_log,
};

// --- Expanded array container (single AST) ---

#[repr(C)]
struct ExpandedSingle {
    base: MongoExtensionExpandedArrayContainer,
    ast: *mut MongoExtensionAggStageAstNode,
    transferred: Cell<bool>,
}

fn expanded_single(ast: *mut MongoExtensionAggStageAstNode) -> ExpandedSingle {
    ExpandedSingle {
        base: MongoExtensionExpandedArrayContainer {
            vtable: &EXPANDED_VTABLE,
        },
        ast,
        transferred: Cell::new(false),
    }
}

unsafe extern "C" fn exp_destroy(p: *mut MongoExtensionExpandedArrayContainer) {
    if p.is_null() {
        return;
    }
    let this = p.cast::<ExpandedSingle>();
    if !(*this).transferred.get() && !(*this).ast.is_null() {
        ast_destroy((*this).ast);
    }
    drop(Box::from_raw(this));
}

unsafe extern "C" fn exp_size(_: *const MongoExtensionExpandedArrayContainer) -> usize {
    1
}

unsafe extern "C" fn exp_transfer(
    p: *mut MongoExtensionExpandedArrayContainer,
    arr: *mut MongoExtensionExpandedArray,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ExpandedSingle>();
    if (*arr).size != 1 {
        return status::new_error_status(-1, "expanded array size mismatch");
    }
    let el = (*arr).elements;
    (*el).type_ = MongoExtensionAggStageNodeType::kAstNode;
    (*el).parse_or_ast = MongoExtensionExpandedArrayElementUnion {
        ast: (*this).ast,
    };
    (*this).transferred.set(true);
    status::status_ok()
}

static EXPANDED_VTABLE: MongoExtensionExpandedArrayContainerVTable =
    MongoExtensionExpandedArrayContainerVTable {
        destroy: exp_destroy,
        size: exp_size,
        transfer: exp_transfer,
    };

// --- AST ---

#[repr(C)]
struct AstObj {
    base: MongoExtensionAggStageAstNode,
    args: Vec<u8>,
}

fn ast_alloc(args: Vec<u8>) -> AstObj {
    AstObj {
        base: MongoExtensionAggStageAstNode {
            vtable: &AST_VTABLE,
        },
        args,
    }
}

unsafe fn ast_destroy(p: *mut MongoExtensionAggStageAstNode) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<AstObj>()));
}

unsafe extern "C" fn ast_ext_destroy(p: *mut MongoExtensionAggStageAstNode) {
    ast_destroy(p);
}

unsafe extern "C" fn ast_get_name(_: *const MongoExtensionAggStageAstNode) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn ast_get_properties(
    _: *const MongoExtensionAggStageAstNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let empty = bson::Document::new();
    match byte_buf::from_bson(&empty) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            status::new_error_status(-1, e.to_string())
        }
    }
}

unsafe extern "C" fn ast_bind(
    p: *const MongoExtensionAggStageAstNode,
    _ctx: *const MongoExtensionCatalogContext,
    out: *mut *mut MongoExtensionLogicalAggStage,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let logical = Box::into_raw(Box::new(logical_alloc((*this).args.clone()))).cast::<MongoExtensionLogicalAggStage>();
    *out = logical;
    status::status_ok()
}

unsafe extern "C" fn ast_clone(
    p: *const MongoExtensionAggStageAstNode,
    out: *mut *mut MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let n = Box::into_raw(Box::new(ast_alloc((*this).args.clone()))).cast::<MongoExtensionAggStageAstNode>();
    *out = n;
    status::status_ok()
}

unsafe extern "C" fn ast_view_policy(
    _: *const MongoExtensionAggStageAstNode,
    out: *mut MongoExtensionFirstStageViewApplicationPolicy,
) -> *mut MongoExtensionStatus {
    *out = MongoExtensionFirstStageViewApplicationPolicy::kDefaultPrepend;
    status::status_ok()
}

unsafe extern "C" fn ast_bind_view(
    _: *mut MongoExtensionAggStageAstNode,
    _: *const MongoExtensionViewInfo,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

static AST_VTABLE: MongoExtensionAggStageAstNodeVTable = MongoExtensionAggStageAstNodeVTable {
    destroy: ast_ext_destroy,
    get_name: ast_get_name,
    get_properties: ast_get_properties,
    bind: ast_bind,
    clone: ast_clone,
    get_first_stage_view_application_policy: ast_view_policy,
    bind_view_info: ast_bind_view,
};

// --- Logical ---

#[repr(C)]
struct LogicalObj {
    base: MongoExtensionLogicalAggStage,
    args: Vec<u8>,
}

fn logical_alloc(args: Vec<u8>) -> LogicalObj {
    LogicalObj {
        base: MongoExtensionLogicalAggStage {
            vtable: &LOGICAL_VTABLE,
        },
        args,
    }
}

unsafe extern "C" fn log_destroy(p: *mut MongoExtensionLogicalAggStage) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<LogicalObj>()));
}

unsafe extern "C" fn log_get_name(_: *const MongoExtensionLogicalAggStage) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn log_serialize(
    p: *const MongoExtensionLogicalAggStage,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<LogicalObj>();
    let g = globals();
    let args_bytes: &[u8] = unsafe { &(*this).args };
    let args = match Document::from_reader(args_bytes) {
        Ok(d) => d,
        Err(_) => {
            *out = std::ptr::null_mut();
            return status::new_error_status(-1, "serialize");
        }
    };
    let d = bson::doc! { g.name: args };
    match byte_buf::from_bson(&d) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            status::new_error_status(-1, e.to_string())
        }
    }
}

unsafe extern "C" fn log_explain(
    p: *const MongoExtensionLogicalAggStage,
    _v: MongoExtensionExplainVerbosity,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    log_serialize(p, out)
}

unsafe extern "C" fn log_compile(
    p: *const MongoExtensionLogicalAggStage,
    out: *mut *mut MongoExtensionExecAggStage,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<LogicalObj>();
    let e = Box::into_raw(Box::new(exec_alloc((*this).args.clone()))).cast::<MongoExtensionExecAggStage>();
    *out = e;
    status::status_ok()
}

unsafe extern "C" fn log_dpl(
    _: *const MongoExtensionLogicalAggStage,
    out: *mut *mut MongoExtensionDistributedPlanLogic,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    status::status_ok()
}

unsafe extern "C" fn log_clone(
    p: *const MongoExtensionLogicalAggStage,
    out: *mut *mut MongoExtensionLogicalAggStage,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<LogicalObj>();
    let n = Box::into_raw(Box::new(logical_alloc((*this).args.clone()))).cast::<MongoExtensionLogicalAggStage>();
    *out = n;
    status::status_ok()
}

unsafe extern "C" fn log_vec_score(
    _: *const MongoExtensionLogicalAggStage,
    o: *mut bool,
) -> *mut MongoExtensionStatus {
    *o = false;
    status::status_ok()
}

unsafe extern "C" fn log_vec_limit(
    _: *mut MongoExtensionLogicalAggStage,
    _: *mut i64,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

static LOGICAL_VTABLE: MongoExtensionLogicalAggStageVTable = MongoExtensionLogicalAggStageVTable {
    destroy: log_destroy,
    get_name: log_get_name,
    serialize: log_serialize,
    explain: log_explain,
    compile: log_compile,
    get_distributed_plan_logic: log_dpl,
    clone: log_clone,
    is_stage_sorted_by_vector_search_score: log_vec_score,
    set_vector_search_limit_for_optimization: log_vec_limit,
};

// --- Exec (passthrough) ---

#[repr(C)]
struct ExecObj {
    base: MongoExtensionExecAggStage,
    args: Vec<u8>,
    source: *mut MongoExtensionExecAggStage,
}

fn exec_alloc(args: Vec<u8>) -> ExecObj {
    ExecObj {
        base: MongoExtensionExecAggStage {
            vtable: &EXEC_VTABLE,
        },
        args,
        source: std::ptr::null_mut(),
    }
}

unsafe extern "C" fn exec_destroy(p: *mut MongoExtensionExecAggStage) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<ExecObj>()));
}

unsafe extern "C" fn exec_get_next(
    p: *mut MongoExtensionExecAggStage,
    ctx: *mut MongoExtensionQueryExecutionContext,
    res: *mut MongoExtensionGetNextResult,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ExecObj>();
    let src = (*this).source;
    if src.is_null() {
        (*res).code = MongoExtensionGetNextResultCode::kEOF;
        let empty_view = MongoExtensionByteView {
            data: std::ptr::null(),
            len: 0,
        };
        (*res).result_document = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteView,
            bytes: crate::sys::MongoExtensionByteContainerBytes {
                view: empty_view,
            },
        };
        (*res).result_metadata = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteView,
            bytes: crate::sys::MongoExtensionByteContainerBytes {
                view: empty_view,
            },
        };
        return status::status_ok();
    }
    let vt = (*src).vtable;
    ((*vt).get_next)(src, ctx, res)
}

unsafe extern "C" fn exec_get_name(_: *const MongoExtensionExecAggStage) -> MongoExtensionByteView {
    name_view()
}

#[repr(C)]
struct EmptyMetrics {
    base: MongoExtensionOperationMetrics,
}

unsafe extern "C" fn met_destroy(p: *mut MongoExtensionOperationMetrics) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<EmptyMetrics>()));
}

unsafe extern "C" fn met_serialize(
    _: *const MongoExtensionOperationMetrics,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let empty = bson::Document::new();
    match byte_buf::from_bson(&empty) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            status::new_error_status(-1, e.to_string())
        }
    }
}

unsafe extern "C" fn met_update(
    _: *mut MongoExtensionOperationMetrics,
    _: MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

static METRICS_VTABLE: MongoExtensionOperationMetricsVTable = MongoExtensionOperationMetricsVTable {
    destroy: met_destroy,
    serialize: met_serialize,
    update: met_update,
};

unsafe extern "C" fn exec_create_metrics(
    _: *const MongoExtensionExecAggStage,
    out: *mut *mut MongoExtensionOperationMetrics,
) -> *mut MongoExtensionStatus {
    let m = Box::new(EmptyMetrics {
        base: MongoExtensionOperationMetrics {
            vtable: &METRICS_VTABLE,
        },
    });
    *out = Box::into_raw(m).cast::<MongoExtensionOperationMetrics>();
    status::status_ok()
}

unsafe extern "C" fn exec_set_source(
    p: *mut MongoExtensionExecAggStage,
    src: *mut MongoExtensionExecAggStage,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ExecObj>();
    (*this).source = src;
    status::status_ok()
}

unsafe extern "C" fn exec_open(_: *mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn exec_reopen(_: *mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn exec_close(_: *mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn exec_explain(
    _: *const MongoExtensionExecAggStage,
    _: MongoExtensionExplainVerbosity,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let empty = bson::Document::new();
    match byte_buf::from_bson(&empty) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            status::new_error_status(-1, e.to_string())
        }
    }
}

static EXEC_VTABLE: MongoExtensionExecAggStageVTable = MongoExtensionExecAggStageVTable {
    destroy: exec_destroy,
    get_next: exec_get_next,
    get_name: exec_get_name,
    create_metrics: exec_create_metrics,
    set_source: exec_set_source,
    open: exec_open,
    reopen: exec_reopen,
    close: exec_close,
    explain: exec_explain,
};

// --- Root extension ---

#[repr(C)]
struct ExtensionObj {
    base: MongoExtension,
    descriptor: DescriptorObj,
}

static EXTENSION_OBJ_ADDR: OnceLock<usize> = OnceLock::new();

unsafe extern "C" fn ext_init(
    _: *const MongoExtension,
    portal: *const crate::sys::MongoExtensionHostPortal,
    services: *const crate::sys::MongoExtensionHostServices,
) -> *mut MongoExtensionStatus {
    let r = ffi_boundary(|| -> Result<(), String> {
        host::set_host_services(services);
        unsafe {
            host::cache_extension_options_from_portal(portal);
        }
        let ext = EXTENSION_OBJ_ADDR
            .get()
            .copied()
            .ok_or_else(|| "extension object not installed".to_string())? as *const ExtensionObj;
        let st = host::register_stage_descriptor(
            portal,
            std::ptr::addr_of!((*ext).descriptor.base),
        );
        if st.is_null() {
            return Err("null status from register_stage_descriptor".into());
        }
        let vt = (*st).vtable;
        let code = ((*vt).get_code)(st);
        ((*vt).destroy)(st);
        if code != crate::sys::MONGO_EXTENSION_STATUS_OK {
            return Err("register_stage_descriptor failed".into());
        }
        Ok(())
    });
    match r {
        None => status::new_error_status(-1, "panic during extension initialize"),
        Some(Err(e)) => status::new_error_status(-1, e),
        Some(Ok(())) => status::status_ok(),
    }
}

static EXTENSION_VTABLE: MongoExtensionVTable = MongoExtensionVTable {
    initialize: ext_init,
};

/// Shared implementation for the `export_transform_stage!` macro.
pub unsafe fn get_extension_impl(
    globals: StageGlobals,
    host_versions: *const crate::sys::MongoExtensionAPIVersionVector,
    extension_out: *mut *const MongoExtension,
) -> *mut MongoExtensionStatus {
    if host_versions.is_null() || extension_out.is_null() {
        return status::new_error_status(-1, "null parameter to get_mongodb_extension");
    }
    let hv = &*host_versions;
    if !host_supports_extension(hv, EXTENSION_API_VERSION) {
        return status::new_error_status(-1, "incompatible extension API version");
    }
    let _ = ACTIVE_STAGE.get_or_init(|| globals);
    let addr = *EXTENSION_OBJ_ADDR.get_or_init(|| {
        let p = Box::into_raw(Box::new(ExtensionObj {
            base: MongoExtension {
                vtable: &EXTENSION_VTABLE,
                version: EXTENSION_API_VERSION,
            },
            descriptor: DescriptorObj {
                base: MongoExtensionAggStageDescriptor {
                    vtable: &DESCRIPTOR_VTABLE,
                },
            },
        }));
        p as usize
    });
    let obj = addr as *const ExtensionObj;
    *extension_out = std::ptr::addr_of!((*obj).base);
    status::status_ok()
}
