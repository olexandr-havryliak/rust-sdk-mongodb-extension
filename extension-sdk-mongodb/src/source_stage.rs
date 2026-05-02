//! **Source** (generator) aggregation stage: parses `{$stageName: <args>}` and emits documents from
//! Rust without requiring an upstream executable stage (e.g. `aggregate: 1` with only this stage).
//!
//! When an upstream stage is present (`set_source` was called), this implementation **forwards**
//! `get_next` to that upstream stage unchanged (passthrough).
//!
//! Use [`export_source_stage!`](crate::export_source_stage) from the crate root.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::OnceLock;

use bson::Document;

use crate::byte_buf;
use crate::error::ExtensionError;
use crate::host;
use crate::panics::ffi_boundary;
use crate::stage_context::StageContext;
use crate::stage_output::Next;
use crate::status;
use crate::sys::{
    MongoExtension, MongoExtensionAggStageAstNode, MongoExtensionAggStageAstNodeVTable,
    MongoExtensionAggStageDescriptor, MongoExtensionAggStageDescriptorVTable,
    MongoExtensionAggStageParseNode, MongoExtensionAggStageParseNodeVTable, MongoExtensionAggStageNodeType,
    MongoExtensionByteContainer, MongoExtensionByteContainerBytes, MongoExtensionByteContainerType,
    MongoExtensionByteView, MongoExtensionCatalogContext, MongoExtensionDistributedPlanLogic,
    MongoExtensionExecAggStage, MongoExtensionExecAggStageVTable, MongoExtensionExpandedArray,
    MongoExtensionExpandedArrayContainer, MongoExtensionExpandedArrayContainerVTable,
    MongoExtensionExpandedArrayElementUnion, MongoExtensionExplainVerbosity,
    MongoExtensionFirstStageViewApplicationPolicy, MongoExtensionGetNextResult,
    MongoExtensionGetNextResultCode, MongoExtensionLogicalAggStage, MongoExtensionLogicalAggStageVTable,
    MongoExtensionOperationMetrics,
    MongoExtensionQueryExecutionContext, MongoExtensionStatus, MongoExtensionVTable, MongoExtensionViewInfo,
};
use crate::version::{host_supports_extension, EXTENSION_API_VERSION};

/// Erased hooks for a concrete [`SourceStage`], installed once per extension via
/// [`export_source_stage!`](crate::export_source_stage).
pub struct SourceOps {
    /// Stage key including `$`, e.g. `"$fibonacci"`.
    pub name: &'static str,
    /// When true, the inner args object must be `{}`.
    pub expect_empty: bool,
    /// Parses args from BSON and allocates opaque state (`Box<State>` as `*mut c_void`).
    pub open_from_doc: fn(Document, &mut StageContext) -> crate::error::Result<*mut c_void>,
    /// Drops state allocated by [`SourceOps::open_from_doc`](SourceOps::open_from_doc).
    pub drop_state: unsafe fn(*mut c_void),
    /// Produces the next output row ([`Next::Advanced`](Next::Advanced)) or end-of-stream ([`Next::Eof`](Next::Eof)).
    pub next: unsafe fn(*mut c_void, &mut StageContext) -> crate::error::Result<Next>,
    /// Optional hook during extension `initialize` (portal valid for extension options).
    pub on_extension_initialized:
        Option<unsafe fn(*const crate::sys::MongoExtensionHostPortal) -> crate::error::Result<()>>,
}

static ACTIVE_OPS: OnceLock<&'static SourceOps> = OnceLock::new();

fn active_ops() -> &'static SourceOps {
    ACTIVE_OPS.get().expect("source stage ops not installed")
}

fn name_view() -> MongoExtensionByteView {
    let b = active_ops().name.as_bytes();
    MongoExtensionByteView {
        data: b.as_ptr(),
        len: b.len() as u64,
    }
}

/// Implement this trait for a generator stage, then export it with [`export_source_stage!`](crate::export_source_stage).
pub trait SourceStage: Sized + Send + 'static {
    /// Stage key including leading `$`, must match BSON (`{ $name: <args> }`).
    const NAME: &'static str;
    /// Parsed stage arguments (from the inner object of `{ Self::NAME: <args> }`).
    type Args;
    /// Per-cursor mutable state between [`SourceStage::next`](SourceStage::next) calls.
    type State;

    /// Validates and decodes `args` into [`SourceStage::Args`](SourceStage::Args).
    fn parse(args: Document) -> crate::error::Result<Self::Args>;
    /// Called once before the first [`SourceStage::next`](SourceStage::next).
    fn open(args: Self::Args, ctx: &mut StageContext) -> crate::error::Result<Self::State>;
    /// Returns the next output row or end-of-stream.
    fn next(state: &mut Self::State, ctx: &mut StageContext) -> crate::error::Result<Next>;
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
    let parsed = ffi_boundary(|| -> crate::error::Result<*mut MongoExtensionAggStageParseNode> {
        let bytes = std::slice::from_raw_parts(stage_bson.data, stage_bson.len as usize);
        let doc = Document::from_reader(bytes)
            .map_err(|e| ExtensionError::FailedToParse(format!("parse bson: {e}")))?;
        let g = active_ops();
        let key = doc.keys().next().ok_or_else(|| {
            ExtensionError::BadValue("stage document must have one field".into())
        })?;
        if key != g.name {
            return Err(ExtensionError::BadValue(format!(
                "expected stage {}, got {key}",
                g.name
            )));
        }
        let args = doc
            .get_document(key)
            .map_err(|e| ExtensionError::BadValue(e.to_string()))?;
        if g.expect_empty && !args.is_empty() {
            return Err(ExtensionError::BadValue(
                "stage definition must be an empty object".into(),
            ));
        }
        let mut arg_bytes = Vec::new();
        args.to_writer(&mut arg_bytes)
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
        let p = Box::into_raw(Box::new(parse_alloc(arg_bytes))).cast::<MongoExtensionAggStageParseNode>();
        Ok(p)
    });
    match parsed {
        None => ExtensionError::Runtime("extension panic during parse".into()).into_raw_status(),
        Some(Err(e)) => e.into_raw_status(),
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
    let r = ffi_boundary(|| -> crate::error::Result<*mut crate::sys::MongoExtensionByteBuf> {
        let this = p.cast::<ParseObj>();
        let g = active_ops();
        let args_bytes: &[u8] = unsafe { &(*this).args };
        let args = Document::from_reader(args_bytes)
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
        let d = bson::doc! { g.name: args };
        byte_buf::from_bson(&d).map_err(|e| ExtensionError::FailedToParse(e.to_string()))
    });
    match r {
        None => ExtensionError::Runtime("panic during get_query_shape".into()).into_raw_status(),
        Some(Err(e)) => e.into_raw_status(),
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
    let r = ffi_boundary(|| -> crate::error::Result<*mut MongoExtensionExpandedArrayContainer> {
        let this = p.cast::<ParseObj>();
        let args = (*this).args.clone();
        let ast = Box::into_raw(Box::new(ast_alloc(args))).cast::<MongoExtensionAggStageAstNode>();
        let c = Box::new(expanded_single(ast));
        Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
    });
    match r {
        None => ExtensionError::Runtime("panic during expand".into()).into_raw_status(),
        Some(Err(e)) => e.into_raw_status(),
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
    let g = active_ops();
    let args_bytes: &[u8] = unsafe { &(*this).args };
    let args = match Document::from_reader(args_bytes) {
        Ok(d) => d,
        Err(_) => {
            *out = std::ptr::null_mut();
            return ExtensionError::FailedToParse("log bson".into()).into_raw_status();
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
            ExtensionError::FailedToParse(e.to_string()).into_raw_status()
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

// --- Expanded array ---

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
        return ExtensionError::BadValue("expanded array size mismatch".into()).into_raw_status();
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
            ExtensionError::FailedToParse(e.to_string()).into_raw_status()
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
    let g = active_ops();
    let args_bytes: &[u8] = unsafe { &(*this).args };
    let args = match Document::from_reader(args_bytes) {
        Ok(d) => d,
        Err(_) => {
            *out = std::ptr::null_mut();
            return ExtensionError::FailedToParse("serialize".into()).into_raw_status();
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
            ExtensionError::FailedToParse(e.to_string()).into_raw_status()
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

// --- Exec ---

fn empty_view() -> MongoExtensionByteView {
    MongoExtensionByteView {
        data: std::ptr::null(),
        len: 0,
    }
}

fn set_eof_empty(res: *mut MongoExtensionGetNextResult) {
    unsafe {
        (*res).code = MongoExtensionGetNextResultCode::kEOF;
        (*res).result_document = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteView,
            bytes: MongoExtensionByteContainerBytes { view: empty_view() },
        };
        (*res).result_metadata = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteView,
            bytes: MongoExtensionByteContainerBytes { view: empty_view() },
        };
    }
}

/// `0` = not yet decided; `1` = passthrough upstream only; `2` = generator (`SourceStage`).
const MODE_INIT: u8 = 0;
const MODE_PASSTHROUGH: u8 = 1;
const MODE_GENERATOR: u8 = 2;

#[repr(C)]
struct ExecObj {
    base: MongoExtensionExecAggStage,
    args: Vec<u8>,
    source: *mut MongoExtensionExecAggStage,
    state: *mut c_void,
    generator_done: Cell<bool>,
    /// See [`MODE_INIT`] / [`MODE_PASSTHROUGH`] / [`MODE_GENERATOR`].
    mode: Cell<u8>,
    saw_upstream_advanced: Cell<bool>,
    /// Host metrics object created in [`exec_create_metrics`](exec_create_metrics).
    metrics: Cell<*mut MongoExtensionOperationMetrics>,
}

fn exec_alloc(args: Vec<u8>) -> ExecObj {
    ExecObj {
        base: MongoExtensionExecAggStage {
            vtable: &EXEC_VTABLE,
        },
        args,
        source: std::ptr::null_mut(),
        state: std::ptr::null_mut(),
        generator_done: Cell::new(false),
        mode: Cell::new(MODE_INIT),
        saw_upstream_advanced: Cell::new(false),
        metrics: Cell::new(std::ptr::null_mut()),
    }
}

unsafe extern "C" fn exec_destroy(p: *mut MongoExtensionExecAggStage) {
    if p.is_null() {
        return;
    }
    let this = p.cast::<ExecObj>();
    let ops = active_ops();
    if !(*this).state.is_null() {
        (ops.drop_state)((*this).state);
    }
    let m = (*this).metrics.get();
    if !m.is_null() {
        unsafe {
            let vt = (*m).vtable;
            ((*vt).destroy)(m);
        }
        (*this).metrics.set(std::ptr::null_mut());
    }
    drop(Box::from_raw(this));
}

unsafe extern "C" fn exec_get_next(
    p: *mut MongoExtensionExecAggStage,
    ctx: *mut MongoExtensionQueryExecutionContext,
    res: *mut MongoExtensionGetNextResult,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ExecObj>();
    let src = (*this).source;

    // Passthrough: upstream produced at least one row — keep forwarding.
    if (*this).mode.get() == MODE_PASSTHROUGH && !src.is_null() {
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
        return status::status_ok();
    }

    if (*this).mode.get() == MODE_INIT && !src.is_null() {
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
            (*this).mode.set(MODE_PASSTHROUGH);
            return status::status_ok();
        }
        if (*res).code == MongoExtensionGetNextResultCode::kEOF && !(*this).saw_upstream_advanced.get() {
            (*this).mode.set(MODE_GENERATOR);
            // Fall through to generator using stage args (empty collection / no rows).
        } else {
            return status::status_ok();
        }
    }

    if (*this).mode.get() == MODE_INIT && src.is_null() {
        (*this).mode.set(MODE_GENERATOR);
    }

    if (*this).mode.get() != MODE_GENERATOR {
        return status::status_ok();
    }

    if (*this).generator_done.get() {
        set_eof_empty(res);
        return status::status_ok();
    }

    let ops = active_ops();
    let gen = ffi_boundary(|| -> crate::error::Result<()> {
        let args_doc = Document::from_reader(std::io::Cursor::new(&(*this).args))
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
        if (*this).state.is_null() {
            let mut sctx = StageContext::new();
            let st = (ops.open_from_doc)(args_doc, &mut sctx)?;
            (*this).state = st;
        }
        let mut sctx = StageContext::new();
        let metrics = (*this).metrics.get();
        sctx.bind_execution(ctx, metrics);
        let out = (ops.next)((*this).state, &mut sctx)?;
        sctx.unbind_execution();
        match out {
            Next::Eof => {
                (ops.drop_state)((*this).state);
                (*this).state = std::ptr::null_mut();
                (*this).generator_done.set(true);
                unsafe {
                    (*res).code = MongoExtensionGetNextResultCode::kEOF;
                    (*res).result_document = MongoExtensionByteContainer {
                        type_: MongoExtensionByteContainerType::kByteView,
                        bytes: MongoExtensionByteContainerBytes { view: empty_view() },
                    };
                    (*res).result_metadata = MongoExtensionByteContainer {
                        type_: MongoExtensionByteContainerType::kByteView,
                        bytes: MongoExtensionByteContainerBytes { view: empty_view() },
                    };
                }
            }
            Next::Advanced { document, metadata } => {
                let raw = byte_buf::from_bson(&document)
                    .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
                let meta_container = match metadata {
                    None => MongoExtensionByteContainer {
                        type_: MongoExtensionByteContainerType::kByteView,
                        bytes: MongoExtensionByteContainerBytes { view: empty_view() },
                    },
                    Some(meta_doc) => {
                        let mbuf = byte_buf::from_bson(&meta_doc)
                            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
                        MongoExtensionByteContainer {
                            type_: MongoExtensionByteContainerType::kByteBuf,
                            bytes: MongoExtensionByteContainerBytes { buf: mbuf },
                        }
                    }
                };
                unsafe {
                    (*res).code = MongoExtensionGetNextResultCode::kAdvanced;
                    (*res).result_document = MongoExtensionByteContainer {
                        type_: MongoExtensionByteContainerType::kByteBuf,
                        bytes: MongoExtensionByteContainerBytes { buf: raw },
                    };
                    (*res).result_metadata = meta_container;
                }
            }
        }
        Ok(())
    });
    match gen {
        None => ExtensionError::Runtime("panic during source stage get_next".into()).into_raw_status(),
        Some(Err(e)) => e.into_raw_status(),
        Some(Ok(())) => status::status_ok(),
    }
}

unsafe extern "C" fn exec_get_name(_: *const MongoExtensionExecAggStage) -> MongoExtensionByteView {
    name_view()
}

unsafe extern "C" fn exec_create_metrics(
    exec: *const MongoExtensionExecAggStage,
    out: *mut *mut MongoExtensionOperationMetrics,
) -> *mut MongoExtensionStatus {
    let this = exec.cast::<ExecObj>();
    let m = crate::operation_metrics::alloc_sdk_operation_metrics();
    (*this).metrics.set(m);
    *out = m;
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
            ExtensionError::FailedToParse(e.to_string()).into_raw_status()
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
    let r = ffi_boundary(|| -> crate::error::Result<()> {
        host::set_host_services(services);
        unsafe {
            host::cache_extension_options_from_portal(portal);
        }
        let ops = active_ops();
        if let Some(init) = ops.on_extension_initialized {
            unsafe { init(portal)? };
        }
        let ext = EXTENSION_OBJ_ADDR
            .get()
            .copied()
            .ok_or_else(|| ExtensionError::Runtime("extension object not installed".into()))?
            as *const ExtensionObj;
        let st = host::register_stage_descriptor(
            portal,
            std::ptr::addr_of!((*ext).descriptor.base),
        );
        if st.is_null() {
            return Err(ExtensionError::Runtime(
                "null status from register_stage_descriptor".into(),
            ));
        }
        let vt = (*st).vtable;
        let code = ((*vt).get_code)(st);
        ((*vt).destroy)(st);
        if code != crate::sys::MONGO_EXTENSION_STATUS_OK {
            return Err(ExtensionError::Runtime(
                "register_stage_descriptor failed".into(),
            ));
        }
        Ok(())
    });
    match r {
        None => ExtensionError::Runtime("panic during extension initialize".into()).into_raw_status(),
        Some(Err(e)) => e.into_raw_status(),
        Some(Ok(())) => status::status_ok(),
    }
}

static EXTENSION_VTABLE: MongoExtensionVTable = MongoExtensionVTable {
    initialize: ext_init,
};

/// Called from `export_source_stage!` with a static [`SourceOps`] table for the concrete stage.
pub unsafe fn get_source_extension_impl(
    ops: &'static SourceOps,
    host_versions: *const crate::sys::MongoExtensionAPIVersionVector,
    extension_out: *mut *const MongoExtension,
) -> *mut MongoExtensionStatus {
    if host_versions.is_null() || extension_out.is_null() {
        return status::new_error_status(-1, "null parameter to get_mongodb_extension");
    }
    let hv = unsafe { &*host_versions };
    if !host_supports_extension(hv, EXTENSION_API_VERSION) {
        return status::new_error_status(-1, "incompatible extension API version");
    }
    let _ = ACTIVE_OPS.get_or_init(|| ops);
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
    unsafe {
        *extension_out = std::ptr::addr_of!((*obj).base);
    }
    status::status_ok()
}
