//! **Map transform** stage: parses `{$stageName: <args>}`, pulls documents from the upstream
//! executable stage, then applies a Rust `fn(&Document, &Document) -> Result<Document, String>` where
//! the second document is the stage argument object.
//!
//! Optionally, `MapStageGlobals.on_eof_no_rows` supplies a `fn(&Document) -> Result<Document, String>`
//! invoked once when upstream hits EOF before any row (empty collection), so the stage can emit
//! output from arguments alone.
//!
//! Use [`export_map_transform_stage!`](crate::export_map_transform_stage) from the crate root.

use std::cell::Cell;
use std::sync::OnceLock;

use bson::Document;

use crate::byte_buf;
use crate::expansion::Expansion;
use crate::host;
use crate::panics::ffi_boundary;
use crate::status;
use crate::sys::{
    MongoExtension, MongoExtensionAggStageAstNode, MongoExtensionAggStageAstNodeVTable,
    MongoExtensionAggStageDescriptor, MongoExtensionAggStageDescriptorVTable,
    MongoExtensionAggStageNodeType, MongoExtensionAggStageParseNode,
    MongoExtensionAggStageParseNodeVTable, MongoExtensionByteContainer,
    MongoExtensionByteContainerBytes, MongoExtensionByteContainerType, MongoExtensionByteView,
    MongoExtensionCatalogContext, MongoExtensionClientType, MongoExtensionDistributedPlanLogic,
    MongoExtensionExecAggStage, MongoExtensionExecAggStageVTable, MongoExtensionExpandedArray,
    MongoExtensionExpandedArrayContainer, MongoExtensionExpandedArrayContainerVTable,
    MongoExtensionExpandedArrayElementUnion, MongoExtensionExplainVerbosity,
    MongoExtensionFirstStageViewApplicationPolicy, MongoExtensionGetNextResult,
    MongoExtensionGetNextResultCode, MongoExtensionLogicalAggStage,
    MongoExtensionLogicalAggStageVTable, MongoExtensionOperationMetrics,
    MongoExtensionOperationMetricsVTable, MongoExtensionPipelineDependencies,
    MongoExtensionPipelineRewriteContext, MongoExtensionQueryExecutionContext,
    MongoExtensionStatus, MongoExtensionStreamType, MongoExtensionVTable, MongoExtensionViewInfo,
};
use crate::version::{supports_selected_version, EXTENSION_API_VERSION};

/// Transform used when the upstream produces no documents.
pub type EmptyInputTransform = fn(&Document) -> Result<Document, String>;
/// Initialization callback; the host portal is valid only during the call.
pub type ExtensionInitializer =
    unsafe fn(*const crate::sys::MongoExtensionHostPortal) -> Result<(), String>;

/// Stage name, parse options, and per-document map callback for `export_map_transform_stage!`.
#[derive(Clone, Copy)]
pub struct MapStageGlobals {
    /// Stage key including leading `$`, e.g. `"$myMap"`.
    pub name: &'static str,
    /// If true, the inner document must be empty (like `$testFoo: {}`).
    pub expect_empty: bool,
    /// Transform each advanced document: input row and stage args (from `{$name: args}`).
    pub transform: fn(&Document, &Document) -> Result<Document, String>,
    /// If set, called **once** when upstream reaches `kEOF` before any `kAdvanced` (e.g. empty
    /// collection). Output is built from stage args only—useful for generator-style stages.
    pub on_eof_no_rows: Option<EmptyInputTransform>,
    /// If set, invoked during extension `initialize` while `portal` is valid (e.g. parse
    /// [`host::extension_options_raw`](crate::host::extension_options_raw) YAML).
    pub on_extension_initialized: Option<ExtensionInitializer>,
    /// BSON for `get_properties` on the AST node (planner static properties).
    pub static_properties_doc: fn() -> Document,
    /// Optional parse-time expansion for typed transforms (`export_transform_stage_type!`); map-only
    /// macros pass `None` (always [`Expansion::SelfStage`]).
    pub expand_from_args_doc: Option<fn(Document) -> Result<Expansion, String>>,
}

static MAP_ACTIVE_STAGE: OnceLock<MapStageGlobals> = OnceLock::new();

fn map_globals() -> MapStageGlobals {
    *MAP_ACTIVE_STAGE
        .get()
        .expect("map extension globals not installed")
}

fn name_bytes() -> &'static [u8] {
    map_globals().name.as_bytes()
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

unsafe extern "C" fn desc_get_name(
    _: *const MongoExtensionAggStageDescriptor,
) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn desc_get_client_type(
    _: *const MongoExtensionAggStageDescriptor,
) -> MongoExtensionClientType {
    MongoExtensionClientType::kMongoExtensionClientTypeAny
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
        let g = map_globals();
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

static DESCRIPTOR_VTABLE: MongoExtensionAggStageDescriptorVTable =
    MongoExtensionAggStageDescriptorVTable {
        get_name: desc_get_name,
        get_client_type: desc_get_client_type,
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

unsafe extern "C" fn parse_get_name(
    _: *const MongoExtensionAggStageParseNode,
) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn parse_get_query_shape(
    p: *const MongoExtensionAggStageParseNode,
    _ctx: *const crate::sys::MongoExtensionHostQueryShapeOpts,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let r = ffi_boundary(
        || -> Result<*mut crate::sys::MongoExtensionByteBuf, String> {
            let this = p.cast::<ParseObj>();
            let g = map_globals();
            let args_bytes: &[u8] = unsafe { &(*this).args };
            let args = Document::from_reader(args_bytes).map_err(|e| e.to_string())?;
            let d = bson::doc! { g.name: args };
            byte_buf::from_bson(&d).map_err(|e| e.to_string())
        },
    );
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
    let r = ffi_boundary(
        || -> Result<*mut MongoExtensionExpandedArrayContainer, String> {
            let this = p.cast::<ParseObj>();
            let args_bytes = (*this).args.clone();
            let args_doc =
                Document::from_reader(args_bytes.as_slice()).map_err(|e| e.to_string())?;
            let g = map_globals();
            if let Some(expand_fn) = g.expand_from_args_doc {
                match expand_fn(args_doc)? {
                    Expansion::SelfStage => {
                        let ast = Box::into_raw(Box::new(ast_alloc(args_bytes)))
                            .cast::<MongoExtensionAggStageAstNode>();
                        let c = Box::new(expanded_single(ast));
                        Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
                    }
                    Expansion::Pipeline(docs) => {
                        let blobs = Expansion::pipeline_stage_arg_blobs(g.name, &docs)
                            .map_err(|e| e.to_string())?;
                        let mut asts: Vec<*mut MongoExtensionAggStageAstNode> =
                            Vec::with_capacity(blobs.len());
                        for b in blobs {
                            asts.push(
                                Box::into_raw(Box::new(ast_alloc(b)))
                                    .cast::<MongoExtensionAggStageAstNode>(),
                            );
                        }
                        let c = Box::new(expanded_multi(asts));
                        Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
                    }
                    Expansion::WithHostIdLookup { .. } => Err(
                        "WithHostIdLookup expansion is only supported by source stages".to_string(),
                    ),
                }
            } else {
                let ast = Box::into_raw(Box::new(ast_alloc(args_bytes)))
                    .cast::<MongoExtensionAggStageAstNode>();
                let c = Box::new(expanded_single(ast));
                Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
            }
        },
    );
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
    let c = Box::into_raw(Box::new(parse_alloc((*this).args.clone())))
        .cast::<MongoExtensionAggStageParseNode>();
    *out = c;
    status::status_ok()
}

unsafe extern "C" fn parse_to_bson_for_log(
    p: *const MongoExtensionAggStageParseNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ParseObj>();
    let g = map_globals();
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

static PARSE_VTABLE: MongoExtensionAggStageParseNodeVTable =
    MongoExtensionAggStageParseNodeVTable {
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
    (*el).parse_or_ast = MongoExtensionExpandedArrayElementUnion { ast: (*this).ast };
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

// --- Expanded array (multiple AST nodes; pipeline expansion) ---

#[repr(C)]
struct ExpandedMulti {
    base: MongoExtensionExpandedArrayContainer,
    asts: Vec<*mut MongoExtensionAggStageAstNode>,
    transferred: Cell<bool>,
}

unsafe extern "C" fn exp_multi_destroy(p: *mut MongoExtensionExpandedArrayContainer) {
    if p.is_null() {
        return;
    }
    let this = p.cast::<ExpandedMulti>();
    if !(*this).transferred.get() {
        for ast in &(*this).asts {
            if !ast.is_null() {
                ast_destroy(*ast);
            }
        }
    }
    drop(Box::from_raw(this));
}

unsafe extern "C" fn exp_multi_size(p: *const MongoExtensionExpandedArrayContainer) -> usize {
    let this = p.cast::<ExpandedMulti>();
    (*this).asts.len()
}

unsafe extern "C" fn exp_multi_transfer(
    p: *mut MongoExtensionExpandedArrayContainer,
    arr: *mut MongoExtensionExpandedArray,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ExpandedMulti>();
    let n = (*this).asts.len();
    if (*arr).size != n {
        return status::new_error_status(-1, "expanded array size mismatch");
    }
    let el = (*arr).elements;
    for (i, ast) in (*this).asts.iter().enumerate() {
        let slot = el.add(i);
        (*slot).type_ = MongoExtensionAggStageNodeType::kAstNode;
        (*slot).parse_or_ast = MongoExtensionExpandedArrayElementUnion { ast: *ast };
    }
    (*this).transferred.set(true);
    status::status_ok()
}

static EXPANDED_MULTI_VTABLE: MongoExtensionExpandedArrayContainerVTable =
    MongoExtensionExpandedArrayContainerVTable {
        destroy: exp_multi_destroy,
        size: exp_multi_size,
        transfer: exp_multi_transfer,
    };

fn expanded_multi(asts: Vec<*mut MongoExtensionAggStageAstNode>) -> ExpandedMulti {
    ExpandedMulti {
        base: MongoExtensionExpandedArrayContainer {
            vtable: &EXPANDED_MULTI_VTABLE,
        },
        asts,
        transferred: Cell::new(false),
    }
}

unsafe extern "C" fn ast_ext_destroy(p: *mut MongoExtensionAggStageAstNode) {
    ast_destroy(p);
}

unsafe extern "C" fn ast_get_name(
    _: *const MongoExtensionAggStageAstNode,
) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn ast_get_properties(
    _: *const MongoExtensionAggStageAstNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let doc = (map_globals().static_properties_doc)();
    match byte_buf::from_bson(&doc) {
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
    let logical = Box::into_raw(Box::new(logical_alloc((*this).args.clone())))
        .cast::<MongoExtensionLogicalAggStage>();
    *out = logical;
    status::status_ok()
}

unsafe extern "C" fn ast_clone(
    p: *const MongoExtensionAggStageAstNode,
    out: *mut *mut MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let n = Box::into_raw(Box::new(ast_alloc((*this).args.clone())))
        .cast::<MongoExtensionAggStageAstNode>();
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

unsafe extern "C" fn log_get_name(
    _: *const MongoExtensionLogicalAggStage,
) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn log_serialize(
    p: *const MongoExtensionLogicalAggStage,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<LogicalObj>();
    let g = map_globals();
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
    _ctx: *mut MongoExtensionQueryExecutionContext,
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
    let e = Box::into_raw(Box::new(exec_alloc((*this).args.clone())))
        .cast::<MongoExtensionExecAggStage>();
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
    let n = Box::into_raw(Box::new(logical_alloc((*this).args.clone())))
        .cast::<MongoExtensionLogicalAggStage>();
    *out = n;
    status::status_ok()
}

unsafe extern "C" fn log_vec_limit(
    _: *mut MongoExtensionLogicalAggStage,
    _: *mut i64,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn log_rewrite_precondition(
    _: *const MongoExtensionLogicalAggStage,
    _: MongoExtensionByteView,
    _: *const MongoExtensionPipelineRewriteContext,
    result: *mut bool,
) -> *mut MongoExtensionStatus {
    *result = false;
    status::status_ok()
}

unsafe extern "C" fn log_rewrite_transform(
    _: *mut MongoExtensionLogicalAggStage,
    _: MongoExtensionByteView,
    _: *mut MongoExtensionPipelineRewriteContext,
    result: *mut bool,
) -> *mut MongoExtensionStatus {
    *result = false;
    status::status_ok()
}

unsafe extern "C" fn log_null_byte_buf(
    _: *const MongoExtensionLogicalAggStage,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    status::status_ok()
}

unsafe extern "C" fn log_apply_suffix_deps(
    _: *mut MongoExtensionLogicalAggStage,
    _: *const MongoExtensionPipelineDependencies,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn log_skip_stream(
    _: *mut MongoExtensionLogicalAggStage,
    _: MongoExtensionStreamType,
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
    set_vector_search_limit_for_optimization_deprecated: log_vec_limit,
    evaluate_pipeline_rewrite_rule_precondition: log_rewrite_precondition,
    evaluate_pipeline_rewrite_rule_transform: log_rewrite_transform,
    get_filter: log_null_byte_buf,
    apply_pipeline_suffix_dependencies: log_apply_suffix_deps,
    get_sort_pattern: log_null_byte_buf,
    skip_stream: log_skip_stream,
    get_docs_needed_bounds: log_null_byte_buf,
};

// --- Exec (map over upstream) ---

fn document_from_byte_container(c: &MongoExtensionByteContainer) -> Result<Document, String> {
    let bytes: &[u8] = match c.type_ {
        MongoExtensionByteContainerType::kByteView => unsafe {
            let v = c.bytes.view;
            if v.len == 0 || v.data.is_null() {
                &[]
            } else {
                std::slice::from_raw_parts(v.data, v.len as usize)
            }
        },
        MongoExtensionByteContainerType::kByteBuf => unsafe {
            let buf = c.bytes.buf;
            if buf.is_null() {
                return Err("null bytebuf in result_document".into());
            }
            let bvt = (*buf).vtable;
            let view = ((*bvt).get_view)(buf);
            std::slice::from_raw_parts(view.data, view.len as usize)
        },
    };
    Document::from_reader(bytes).map_err(|e| format!("decode result_document: {e}"))
}

fn empty_byte_container() -> MongoExtensionByteContainer {
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

/// Destroys a `kByteBuf` whose ownership the upstream stage transferred to this stage.
fn release_byte_container(c: MongoExtensionByteContainer) {
    if c.type_ == MongoExtensionByteContainerType::kByteBuf {
        unsafe {
            let buf = c.bytes.buf;
            if !buf.is_null() {
                ((*(*buf).vtable).destroy)(buf);
            }
        }
    }
}

/// Destroys upstream-owned `kByteBuf` metadata and clears it; borrowed views stay untouched.
unsafe fn release_owned_metadata(res: *mut MongoExtensionGetNextResult) {
    if (*res).result_metadata.type_ == MongoExtensionByteContainerType::kByteBuf {
        release_byte_container(std::mem::replace(
            &mut (*res).result_metadata,
            empty_byte_container(),
        ));
    }
}

#[repr(C)]
struct ExecObj {
    base: MongoExtensionExecAggStage,
    args: Vec<u8>,
    source: *mut MongoExtensionExecAggStage,
    saw_upstream_advanced: Cell<bool>,
    emitted_eof_synthetic: Cell<bool>,
}

fn exec_alloc(args: Vec<u8>) -> ExecObj {
    ExecObj {
        base: MongoExtensionExecAggStage {
            vtable: &EXEC_VTABLE,
        },
        args,
        source: std::ptr::null_mut(),
        saw_upstream_advanced: Cell::new(false),
        emitted_eof_synthetic: Cell::new(false),
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
            bytes: MongoExtensionByteContainerBytes { view: empty_view },
        };
        (*res).result_metadata = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteView,
            bytes: MongoExtensionByteContainerBytes { view: empty_view },
        };
        return status::status_ok();
    }
    let vt = (*src).vtable;
    let st = ((*vt).get_next)(src, ctx, res);
    if !st.is_null() {
        let svt = (*st).vtable;
        let code = ((*svt).get_code)(st);
        if code != crate::sys::MONGO_EXTENSION_STATUS_OK {
            return st;
        }
        ((*svt).destroy)(st);
    }

    if (*res).code == MongoExtensionGetNextResultCode::kAdvanced {
        (*this).saw_upstream_advanced.set(true);
        let upstream_doc = std::mem::replace(&mut (*res).result_document, empty_byte_container());
        let row = document_from_byte_container(&upstream_doc);
        release_byte_container(upstream_doc);
        let mapped = ffi_boundary(|| -> Result<(), String> {
            let row = row?;
            let args_bytes: &[u8] = &(*this).args;
            let args = Document::from_reader(args_bytes).map_err(|e| e.to_string())?;
            let mg = map_globals();
            let out = (mg.transform)(&row, &args)?;
            let raw = byte_buf::from_bson(&out).map_err(|e| e.to_string())?;
            (*res).result_document = MongoExtensionByteContainer {
                type_: MongoExtensionByteContainerType::kByteBuf,
                bytes: MongoExtensionByteContainerBytes { buf: raw },
            };
            Ok(())
        });
        if !matches!(mapped, Some(Ok(()))) {
            release_owned_metadata(res);
        }
        return match mapped {
            None => status::new_error_status(-1, "panic during map transform"),
            Some(Err(e)) => status::new_error_status(-1, e),
            Some(Ok(())) => status::status_ok(),
        };
    }

    if (*res).code == MongoExtensionGetNextResultCode::kEOF
        && !(*this).saw_upstream_advanced.get()
        && !(*this).emitted_eof_synthetic.get()
    {
        if let Some(on_eof) = map_globals().on_eof_no_rows {
            let synth = ffi_boundary(
                || -> Result<*mut crate::sys::MongoExtensionByteBuf, String> {
                    let args_bytes: &[u8] = &(*this).args;
                    let args = Document::from_reader(args_bytes).map_err(|e| e.to_string())?;
                    let out = on_eof(&args)?;
                    byte_buf::from_bson(&out).map_err(|e| e.to_string())
                },
            );
            release_owned_metadata(res);
            return match synth {
                None => status::new_error_status(-1, "panic during on_eof_no_rows"),
                Some(Err(e)) => status::new_error_status(-1, e),
                Some(Ok(raw)) => {
                    (*this).emitted_eof_synthetic.set(true);
                    (*res).code = MongoExtensionGetNextResultCode::kAdvanced;
                    (*res).result_document = MongoExtensionByteContainer {
                        type_: MongoExtensionByteContainerType::kByteBuf,
                        bytes: MongoExtensionByteContainerBytes { buf: raw },
                    };
                    (*res).result_metadata = empty_byte_container();
                    status::status_ok()
                }
            };
        }
    }

    status::status_ok()
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

static METRICS_VTABLE: MongoExtensionOperationMetricsVTable =
    MongoExtensionOperationMetricsVTable {
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
    _: *mut MongoExtensionQueryExecutionContext,
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
) -> *mut MongoExtensionStatus {
    let r = ffi_boundary(|| -> Result<(), String> {
        unsafe {
            host::cache_extension_options_from_portal(portal);
        }
        let mg = map_globals();
        if let Some(init) = mg.on_extension_initialized {
            unsafe { init(portal)? };
        }
        let ext = EXTENSION_OBJ_ADDR
            .get()
            .copied()
            .ok_or_else(|| "extension object not installed".to_string())?
            as *const ExtensionObj;
        let st =
            host::register_stage_descriptor(portal, std::ptr::addr_of!((*ext).descriptor.base));
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

/// Shared implementation for the `export_map_transform_stage!` macro.
///
/// # Safety
/// Non-null host services must satisfy the MongoDB lifetime/callback contract;
/// non-null `extension_out` must be aligned and writable for one pointer.
pub unsafe fn get_map_extension_impl(
    globals: MapStageGlobals,
    version: crate::sys::MongoExtensionAPIVersion,
    host_services: *const crate::sys::MongoExtensionHostServices,
    extension_out: *mut *const MongoExtension,
) -> *mut MongoExtensionStatus {
    if host_services.is_null() || extension_out.is_null() {
        return status::new_error_status(-1, "null parameter to get_mongodb_extension");
    }
    if !supports_selected_version(version) {
        return status::new_error_status(-1, "incompatible extension API version");
    }
    host::set_host_services(host_services);
    let _ = MAP_ACTIVE_STAGE.get_or_init(|| globals);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{
        MongoExtensionByteBuf, MongoExtensionByteBufVTable, MONGO_EXTENSION_STATUS_OK,
    };
    use bson::doc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn tag_row(row: &Document, _: &Document) -> Result<Document, String> {
        if row.get_bool("fail").unwrap_or(false) {
            return Err("transform rejected row".into());
        }
        if row.get_bool("panic").unwrap_or(false) {
            panic!("transform panicked");
        }
        let mut out = row.clone();
        out.insert("mapped", true);
        Ok(out)
    }

    fn generate_from_args(args: &Document) -> Result<Document, String> {
        if args.get_bool("fail").unwrap_or(false) {
            return Err("generator rejected args".into());
        }
        Ok(doc! { "generated": true })
    }

    fn install_globals() {
        let _ = MAP_ACTIVE_STAGE.get_or_init(|| MapStageGlobals {
            name: "$leakProbe",
            expect_empty: false,
            transform: tag_row,
            on_eof_no_rows: Some(generate_from_args),
            on_extension_initialized: None,
            static_properties_doc: crate::stage_properties::default_map_stage_static_properties,
            expand_from_args_doc: None,
        });
    }

    #[repr(C)]
    struct CountedBuf {
        base: MongoExtensionByteBuf,
        data: Vec<u8>,
        destroyed: Arc<AtomicUsize>,
    }

    unsafe extern "C" fn counted_destroy(p: *mut MongoExtensionByteBuf) {
        let b = Box::from_raw(p.cast::<CountedBuf>());
        b.destroyed.fetch_add(1, Ordering::SeqCst);
    }

    unsafe extern "C" fn counted_view(p: *const MongoExtensionByteBuf) -> MongoExtensionByteView {
        let d = &(*p.cast::<CountedBuf>()).data;
        MongoExtensionByteView {
            data: d.as_ptr(),
            len: d.len() as u64,
        }
    }

    static COUNTED_VTABLE: MongoExtensionByteBufVTable = MongoExtensionByteBufVTable {
        destroy: counted_destroy,
        get_view: counted_view,
    };

    fn counted_buf(data: Vec<u8>, destroyed: &Arc<AtomicUsize>) -> *mut MongoExtensionByteBuf {
        Box::into_raw(Box::new(CountedBuf {
            base: MongoExtensionByteBuf {
                vtable: &COUNTED_VTABLE,
            },
            data,
            destroyed: Arc::clone(destroyed),
        }))
        .cast()
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Meta {
        Owned,
        Borrowed,
    }

    /// Upstream that emits one result: `row` bytes as an owned `kByteBuf` (or `kEOF` when
    /// `None`), plus `$vectorSearchScore` metadata that is either owned or borrowed.
    #[repr(C)]
    struct FakeUpstream {
        base: MongoExtensionExecAggStage,
        row: Option<Vec<u8>>,
        meta: Meta,
        meta_bytes: Vec<u8>,
        doc_frees: Arc<AtomicUsize>,
        meta_frees: Arc<AtomicUsize>,
    }

    fn meta_doc() -> Document {
        doc! { "$vectorSearchScore": 0.5 }
    }

    unsafe extern "C" fn upstream_get_next(
        p: *mut MongoExtensionExecAggStage,
        _: *mut MongoExtensionQueryExecutionContext,
        res: *mut MongoExtensionGetNextResult,
    ) -> *mut MongoExtensionStatus {
        let this = &*p.cast::<FakeUpstream>();
        match &this.row {
            Some(bytes) => {
                (*res).code = MongoExtensionGetNextResultCode::kAdvanced;
                (*res).result_document = MongoExtensionByteContainer {
                    type_: MongoExtensionByteContainerType::kByteBuf,
                    bytes: MongoExtensionByteContainerBytes {
                        buf: counted_buf(bytes.clone(), &this.doc_frees),
                    },
                };
            }
            None => {
                (*res).code = MongoExtensionGetNextResultCode::kEOF;
                (*res).result_document = empty_byte_container();
            }
        }
        (*res).result_metadata = match this.meta {
            Meta::Owned => MongoExtensionByteContainer {
                type_: MongoExtensionByteContainerType::kByteBuf,
                bytes: MongoExtensionByteContainerBytes {
                    buf: counted_buf(this.meta_bytes.clone(), &this.meta_frees),
                },
            },
            Meta::Borrowed => MongoExtensionByteContainer {
                type_: MongoExtensionByteContainerType::kByteView,
                bytes: MongoExtensionByteContainerBytes {
                    view: MongoExtensionByteView {
                        data: this.meta_bytes.as_ptr(),
                        len: this.meta_bytes.len() as u64,
                    },
                },
            },
        };
        status::status_ok()
    }

    static UPSTREAM_VTABLE: MongoExtensionExecAggStageVTable = MongoExtensionExecAggStageVTable {
        get_next: upstream_get_next,
        ..EXEC_VTABLE
    };

    struct Pulled {
        code: i32,
        document: Option<Document>,
        metadata: Option<Document>,
        doc_frees: usize,
        /// Metadata buffers destroyed by the stage, before the caller releases what it received.
        meta_frees: usize,
    }

    /// Reads the container as a caller would, destroying it when the caller owns it.
    unsafe fn take_container(c: &MongoExtensionByteContainer) -> Option<Document> {
        let doc = match c.type_ {
            MongoExtensionByteContainerType::kByteBuf => {
                let buf = c.bytes.buf;
                let view = ((*(*buf).vtable).get_view)(buf);
                let d =
                    Document::from_reader(std::slice::from_raw_parts(view.data, view.len as usize))
                        .unwrap();
                ((*(*buf).vtable).destroy)(buf);
                d
            }
            MongoExtensionByteContainerType::kByteView => {
                let view = c.bytes.view;
                if view.data.is_null() || view.len == 0 {
                    return None;
                }
                Document::from_reader(std::slice::from_raw_parts(view.data, view.len as usize))
                    .unwrap()
            }
        };
        Some(doc)
    }

    /// Pulls one result through the map stage built with `args` on top of a fake upstream.
    unsafe fn pull(row: Option<Vec<u8>>, meta: Meta, args: Document) -> Pulled {
        install_globals();
        let doc_frees = Arc::new(AtomicUsize::new(0));
        let meta_frees = Arc::new(AtomicUsize::new(0));
        let mut upstream = FakeUpstream {
            base: MongoExtensionExecAggStage {
                vtable: &UPSTREAM_VTABLE,
            },
            row,
            meta,
            meta_bytes: bson::to_vec(&meta_doc()).unwrap(),
            doc_frees: Arc::clone(&doc_frees),
            meta_frees: Arc::clone(&meta_frees),
        };
        let exec = Box::into_raw(Box::new(exec_alloc(bson::to_vec(&args).unwrap())))
            .cast::<MongoExtensionExecAggStage>();
        let st = exec_set_source(exec, std::ptr::addr_of_mut!(upstream).cast());
        ((*(*st).vtable).destroy)(st);
        let mut res = std::mem::MaybeUninit::<MongoExtensionGetNextResult>::uninit();
        let st = exec_get_next(exec, std::ptr::null_mut(), res.as_mut_ptr());
        let code = ((*(*st).vtable).get_code)(st);
        ((*(*st).vtable).destroy)(st);
        let res = res.assume_init();
        let stage_meta_frees = meta_frees.load(Ordering::SeqCst);
        let document = if code == MONGO_EXTENSION_STATUS_OK {
            take_container(&res.result_document)
        } else {
            None
        };
        let metadata = take_container(&res.result_metadata);
        exec_destroy(exec);
        Pulled {
            code,
            document,
            metadata,
            doc_frees: doc_frees.load(Ordering::SeqCst),
            meta_frees: stage_meta_frees,
        }
    }

    unsafe fn pull_row(row: Document, meta: Meta) -> Pulled {
        pull(Some(bson::to_vec(&row).unwrap()), meta, doc! {})
    }

    #[test]
    fn mapped_row_releases_upstream_owned_buffer() {
        let r = unsafe { pull_row(doc! { "_id": 1 }, Meta::Borrowed) };
        assert_eq!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.document, Some(doc! { "_id": 1, "mapped": true }));
        assert_eq!(
            r.doc_frees, 1,
            "upstream kByteBuf must be destroyed exactly once"
        );
    }

    #[test]
    fn failed_transform_releases_upstream_owned_buffer() {
        let r = unsafe { pull_row(doc! { "_id": 2, "fail": true }, Meta::Borrowed) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.document, None);
        assert_eq!(
            r.doc_frees, 1,
            "upstream kByteBuf must be destroyed on errors too"
        );
    }

    #[test]
    fn mapped_row_hands_owned_metadata_to_caller() {
        let r = unsafe { pull_row(doc! { "_id": 3 }, Meta::Owned) };
        assert_eq!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.meta_frees, 0, "stage must not free metadata it passes on");
        assert_eq!(r.metadata, Some(meta_doc()));
    }

    #[test]
    fn failed_transform_releases_owned_metadata() {
        let r = unsafe { pull_row(doc! { "_id": 4, "fail": true }, Meta::Owned) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.meta_frees, 1);
        assert_eq!(
            r.metadata, None,
            "released metadata must not be left in the result"
        );
    }

    #[test]
    fn undecodable_row_releases_owned_metadata() {
        let r = unsafe { pull(Some(vec![1, 2, 3]), Meta::Owned, doc! {}) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.meta_frees, 1);
        assert_eq!(r.metadata, None);
    }

    #[test]
    fn panicking_transform_releases_owned_metadata() {
        let r = unsafe { pull_row(doc! { "_id": 5, "panic": true }, Meta::Owned) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.meta_frees, 1);
        assert_eq!(r.metadata, None);
    }

    #[test]
    fn failed_transform_leaves_borrowed_metadata_alone() {
        let r = unsafe { pull_row(doc! { "_id": 6, "fail": true }, Meta::Borrowed) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.metadata, Some(meta_doc()));
    }

    #[test]
    fn synthetic_eof_row_releases_owned_metadata() {
        let r = unsafe { pull(None, Meta::Owned, doc! {}) };
        assert_eq!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.document, Some(doc! { "generated": true }));
        assert_eq!(r.meta_frees, 1);
        assert_eq!(r.metadata, None);
    }

    #[test]
    fn failed_synthetic_eof_releases_owned_metadata() {
        let r = unsafe { pull(None, Meta::Owned, doc! { "fail": true }) };
        assert_ne!(r.code, MONGO_EXTENSION_STATUS_OK);
        assert_eq!(r.meta_frees, 1);
        assert_eq!(r.metadata, None);
    }
}
