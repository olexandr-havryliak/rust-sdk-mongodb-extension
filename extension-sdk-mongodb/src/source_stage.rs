//! **Source** (generator) aggregation stage: parses `{$stageName: <args>}` and emits documents from
//! Rust without requiring an upstream executable stage (e.g. `aggregate: 1` with only this stage).
//!
//! When an upstream stage is present (`set_source` was called), this implementation **forwards**
//! `get_next` to that upstream stage unchanged (passthrough).
//!
//! Use [`export_source_stage!`](crate::export_source_stage) from the crate root.

use std::cell::Cell;
use std::ffi::c_void;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use bson::Document;

use crate::byte_buf;
use crate::error::ExtensionError;
use crate::expansion::Expansion;
use crate::stage_model::StagePlan;
use crate::stage_properties::StageProperties;
use crate::host;
use crate::panics::ffi_boundary;
use crate::stage_context::{CatalogContext, StageContext};
use crate::stage_output::Next;
use crate::status;
use crate::sys::{
    MongoExtension, MongoExtensionAggStageAstNode, MongoExtensionAggStageAstNodeVTable,
    MongoExtensionAggStageDescriptor, MongoExtensionAggStageDescriptorVTable,
    MongoExtensionAggStageParseNode, MongoExtensionAggStageParseNodeVTable, MongoExtensionAggStageNodeType,
    MongoExtensionByteContainer, MongoExtensionByteContainerBytes, MongoExtensionByteContainerType,
    MongoExtensionByteView, MongoExtensionCatalogContext, MongoExtensionClientType,
    MongoExtensionDistributedPlanLogic,
    MongoExtensionExecAggStage, MongoExtensionExecAggStageVTable, MongoExtensionExpandedArray,
    MongoExtensionExpandedArrayContainer, MongoExtensionExpandedArrayContainerVTable,
    MongoExtensionExpandedArrayElementUnion, MongoExtensionExplainVerbosity,
    MongoExtensionFirstStageViewApplicationPolicy, MongoExtensionGetNextResult,
    MongoExtensionGetNextResultCode, MongoExtensionLogicalAggStage, MongoExtensionLogicalAggStageVTable,
    MongoExtensionOperationMetrics, MongoExtensionPipelineDependencies,
    MongoExtensionPipelineRewriteContext, MongoExtensionQueryExecutionContext, MongoExtensionStatus,
    MongoExtensionStreamType, MongoExtensionVTable, MongoExtensionViewInfo,
};
use crate::version::{supports_selected_version, EXTENSION_API_VERSION};

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
    /// BSON for `get_properties` on the AST node (planner static properties).
    pub static_properties_doc: fn() -> Document,
    /// Parse-time expansion from the inner args document (after BSON decode from the parse node).
    pub expand_inner: fn(Document) -> crate::error::Result<Expansion>,
}

fn name_view(ops: &SourceOps) -> MongoExtensionByteView {
    let b = ops.name.as_bytes();
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

    /// Lower this stage to itself or to a linear pipeline (parse-time expansion).
    ///
    /// Default: no expansion ([`Expansion::SelfStage`]).
    fn expand(_args: &Self::Args) -> Expansion {
        Expansion::SelfStage
    }

    /// Static planner properties (`MongoExtensionStaticProperties` on the host).
    ///
    /// Default: [`StagePlan::source_default`](crate::stage_model::StagePlan::source_default) (streaming
    /// pull / generator, first in pipeline, document source when present).
    fn properties() -> StageProperties {
        StagePlan::source_default().properties
    }
}

// --- Descriptor ---

#[repr(C)]
struct DescriptorObj {
    base: MongoExtensionAggStageDescriptor,
    ops: &'static SourceOps,
}

unsafe extern "C" fn desc_get_name(d: *const MongoExtensionAggStageDescriptor) -> MongoExtensionByteView {
    let this = d.cast::<DescriptorObj>();
    name_view((*this).ops)
}

unsafe extern "C" fn desc_get_client_type(
    _: *const MongoExtensionAggStageDescriptor,
) -> MongoExtensionClientType {
    MongoExtensionClientType::kMongoExtensionClientTypeAny
}

unsafe extern "C" fn desc_parse(
    descriptor: *const MongoExtensionAggStageDescriptor,
    stage_bson: MongoExtensionByteView,
    out_parse: *mut *mut MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    *out_parse = std::ptr::null_mut();
    let parsed = ffi_boundary(|| -> crate::error::Result<*mut MongoExtensionAggStageParseNode> {
        let bytes = std::slice::from_raw_parts(stage_bson.data, stage_bson.len as usize);
        let doc = Document::from_reader(bytes)
            .map_err(|e| ExtensionError::FailedToParse(format!("parse bson: {e}")))?;
        let this = descriptor.cast::<DescriptorObj>();
        let g = (*this).ops;
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
        let p = Box::into_raw(Box::new(parse_alloc(arg_bytes, g))).cast::<MongoExtensionAggStageParseNode>();
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
    get_client_type: desc_get_client_type,
    parse: desc_parse,
};

// --- Parse node ---

#[repr(C)]
struct ParseObj {
    base: MongoExtensionAggStageParseNode,
    args: Vec<u8>,
    ops: &'static SourceOps,
}

fn parse_alloc(args: Vec<u8>, ops: &'static SourceOps) -> ParseObj {
    ParseObj {
        base: MongoExtensionAggStageParseNode {
            vtable: &PARSE_VTABLE,
        },
        args,
        ops,
    }
}

unsafe extern "C" fn parse_destroy(p: *mut MongoExtensionAggStageParseNode) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<ParseObj>()));
}

unsafe extern "C" fn parse_get_name(p: *const MongoExtensionAggStageParseNode) -> MongoExtensionByteView {
    let this = p.cast::<ParseObj>();
    name_view((*this).ops)
}

unsafe extern "C" fn parse_get_query_shape(
    p: *const MongoExtensionAggStageParseNode,
    _ctx: *const crate::sys::MongoExtensionHostQueryShapeOpts,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = std::ptr::null_mut();
    let r = ffi_boundary(|| -> crate::error::Result<*mut crate::sys::MongoExtensionByteBuf> {
        let this = p.cast::<ParseObj>();
        let g = (*this).ops;
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
        let args_bytes = (*this).args.clone();
        let args_doc = Document::from_reader(args_bytes.as_slice())
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
        let g = (*this).ops;
        let ex = (g.expand_inner)(args_doc)?;
        match ex {
            Expansion::SelfStage => {
                let ast =
                    Box::into_raw(Box::new(ast_alloc(args_bytes, g))).cast::<MongoExtensionAggStageAstNode>();
                let c = Box::new(expanded_single(ast));
                Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
            }
            Expansion::Pipeline(docs) => {
                let blobs = Expansion::pipeline_stage_arg_blobs(g.name, &docs)?;
                let mut asts: Vec<*mut MongoExtensionAggStageAstNode> =
                    Vec::with_capacity(blobs.len());
                for b in blobs {
                    asts.push(
                        Box::into_raw(Box::new(ast_alloc(b, g))).cast::<MongoExtensionAggStageAstNode>(),
                    );
                }
                let c = Box::new(expanded_multi(asts));
                Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
            }
            Expansion::WithHostIdLookup {
                extension_stage,
                id_lookup,
            } => {
                let blobs = Expansion::pipeline_stage_arg_blobs(g.name, &[extension_stage])?;
                let extension_args = blobs.into_iter().next().ok_or_else(|| {
                    ExtensionError::BadValue("missing extension stage".into())
                })?;
                // Ask the host before allocating the extension AST. A rejected lookup must not
                // leave that node allocated.
                let id_lookup_ast = host::create_id_lookup_ast(&id_lookup)?;
                let extension_ast = Box::into_raw(Box::new(ast_alloc(extension_args, g)))
                    .cast::<MongoExtensionAggStageAstNode>();
                let c = Box::new(expanded_multi(vec![extension_ast, id_lookup_ast]));
                Ok(Box::into_raw(c).cast::<MongoExtensionExpandedArrayContainer>())
            }
        }
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
    let c = Box::into_raw(Box::new(parse_alloc((*this).args.clone(), (*this).ops))).cast::<MongoExtensionAggStageParseNode>();
    *out = c;
    status::status_ok()
}

unsafe extern "C" fn parse_to_bson_for_log(
    p: *const MongoExtensionAggStageParseNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<ParseObj>();
    let g = (*this).ops;
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
        destroy_ast_via_vtable((*this).ast);
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
    ops: &'static SourceOps,
}

#[cfg(test)]
static LIVE_AST_NODES: AtomicUsize = AtomicUsize::new(0);

fn ast_alloc(args: Vec<u8>, ops: &'static SourceOps) -> AstObj {
    #[cfg(test)]
    LIVE_AST_NODES.fetch_add(1, Ordering::SeqCst);
    AstObj {
        base: MongoExtensionAggStageAstNode {
            vtable: &AST_VTABLE,
        },
        args,
        ops,
    }
}

unsafe fn ast_destroy(p: *mut MongoExtensionAggStageAstNode) {
    if p.is_null() {
        return;
    }
    #[cfg(test)]
    LIVE_AST_NODES.fetch_sub(1, Ordering::SeqCst);
    drop(Box::from_raw(p.cast::<AstObj>()));
}

unsafe fn destroy_ast_via_vtable(p: *mut MongoExtensionAggStageAstNode) {
    if p.is_null() {
        return;
    }
    let vt = (*p).vtable;
    ((*vt).destroy)(p);
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
                destroy_ast_via_vtable(*ast);
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
        return ExtensionError::BadValue("expanded array size mismatch".into()).into_raw_status();
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

unsafe extern "C" fn ast_get_name(p: *const MongoExtensionAggStageAstNode) -> MongoExtensionByteView {
    let this = p.cast::<AstObj>();
    name_view((*this).ops)
}

unsafe extern "C" fn ast_get_properties(
    p: *const MongoExtensionAggStageAstNode,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let doc = ((*this).ops.static_properties_doc)();
    match byte_buf::from_bson(&doc) {
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
    ctx: *const MongoExtensionCatalogContext,
    out: *mut *mut MongoExtensionLogicalAggStage,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let logical = Box::into_raw(Box::new(logical_alloc(
        (*this).args.clone(),
        catalog_context_from_raw(ctx),
        (*this).ops,
    )))
    .cast::<MongoExtensionLogicalAggStage>();
    *out = logical;
    status::status_ok()
}

unsafe extern "C" fn ast_clone(
    p: *const MongoExtensionAggStageAstNode,
    out: *mut *mut MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<AstObj>();
    let n = Box::into_raw(Box::new(ast_alloc((*this).args.clone(), (*this).ops))).cast::<MongoExtensionAggStageAstNode>();
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
    catalog: Option<CatalogContext>,
    ops: &'static SourceOps,
}

fn logical_alloc(args: Vec<u8>, catalog: Option<CatalogContext>, ops: &'static SourceOps) -> LogicalObj {
    LogicalObj {
        base: MongoExtensionLogicalAggStage {
            vtable: &LOGICAL_VTABLE,
        },
        args,
        catalog,
        ops,
    }
}

unsafe fn string_from_view(view: MongoExtensionByteView) -> Option<String> {
    if view.data.is_null() || view.len == 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(view.data, view.len as usize);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

unsafe fn catalog_context_from_raw(ctx: *const MongoExtensionCatalogContext) -> Option<CatalogContext> {
    if ctx.is_null() {
        return None;
    }
    let db = string_from_view((*ctx).namespace_string.database_name)?;
    let coll = string_from_view((*ctx).namespace_string.collection_name)?;
    Some(CatalogContext {
        database_name: db,
        collection_name: coll,
        uuid: string_from_view((*ctx).uuid_string),
        in_router: (*ctx).in_router != 0,
        verbosity: (*ctx).verbosity as u32,
    })
}

unsafe extern "C" fn log_destroy(p: *mut MongoExtensionLogicalAggStage) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<LogicalObj>()));
}

unsafe extern "C" fn log_get_name(p: *const MongoExtensionLogicalAggStage) -> MongoExtensionByteView {
    let this = p.cast::<LogicalObj>();
    name_view((*this).ops)
}

unsafe extern "C" fn log_serialize(
    p: *const MongoExtensionLogicalAggStage,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<LogicalObj>();
    let g = (*this).ops;
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
    let e = Box::into_raw(Box::new(exec_alloc(
        (*this).args.clone(),
        (*this).catalog.clone(),
        (*this).ops,
    )))
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
    let n = Box::into_raw(Box::new(logical_alloc(
        (*this).args.clone(),
        (*this).catalog.clone(),
        (*this).ops,
    )))
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
    catalog: Option<CatalogContext>,
    ops: &'static SourceOps,
    source: *mut MongoExtensionExecAggStage,
    state: *mut c_void,
    generator_done: Cell<bool>,
    /// See [`MODE_INIT`] / [`MODE_PASSTHROUGH`] / [`MODE_GENERATOR`].
    mode: Cell<u8>,
    saw_upstream_advanced: Cell<bool>,
    /// Host metrics object created in [`exec_create_metrics`](exec_create_metrics).
    metrics: Cell<*mut MongoExtensionOperationMetrics>,
}

fn exec_alloc(args: Vec<u8>, catalog: Option<CatalogContext>, ops: &'static SourceOps) -> ExecObj {
    ExecObj {
        base: MongoExtensionExecAggStage {
            vtable: &EXEC_VTABLE,
        },
        args,
        catalog,
        ops,
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
    let ops = (*this).ops;
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

    let ops = (*this).ops;
    let gen = ffi_boundary(|| -> crate::error::Result<()> {
        let args_doc = Document::from_reader(std::io::Cursor::new(&(*this).args))
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
        if (*this).state.is_null() {
            let mut sctx = StageContext::new();
            sctx.bind_catalog((*this).catalog.clone());
            let st = (ops.open_from_doc)(args_doc, &mut sctx)?;
            (*this).state = st;
        }
        let mut sctx = StageContext::new();
        let metrics = (*this).metrics.get();
        sctx.bind_catalog((*this).catalog.clone());
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

unsafe extern "C" fn exec_get_name(p: *const MongoExtensionExecAggStage) -> MongoExtensionByteView {
    let this = p.cast::<ExecObj>();
    name_view((*this).ops)
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
    descriptors: Vec<DescriptorObj>,
}

static EXTENSION_OBJ_ADDR: OnceLock<usize> = OnceLock::new();

unsafe extern "C" fn ext_init(
    _: *const MongoExtension,
    portal: *const crate::sys::MongoExtensionHostPortal,
) -> *mut MongoExtensionStatus {
    let r = ffi_boundary(|| -> crate::error::Result<()> {
        unsafe {
            host::cache_extension_options_from_portal(portal);
        }
        let ext = EXTENSION_OBJ_ADDR
            .get()
            .copied()
            .ok_or_else(|| ExtensionError::Runtime("extension object not installed".into()))?
            as *const ExtensionObj;
        for descriptor in &(*ext).descriptors {
            if let Some(init) = descriptor.ops.on_extension_initialized {
                unsafe { init(portal)? };
            }
            let st = host::register_stage_descriptor(
                portal,
                std::ptr::addr_of!(descriptor.base),
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
                return Err(ExtensionError::Runtime(format!(
                    "register_stage_descriptor failed for {}",
                    descriptor.ops.name
                )));
            }
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
    version: crate::sys::MongoExtensionAPIVersion,
    host_services: *const crate::sys::MongoExtensionHostServices,
    extension_out: *mut *const MongoExtension,
) -> *mut MongoExtensionStatus {
    get_multi_source_extension_impl(&[ops], version, host_services, extension_out)
}

/// Builds one MongoDB extension that registers every source stage in `ops`.
///
/// The `export_*` macros each define `get_mongodb_extension_versions` and
/// `get_mongodb_extension`, so a crate can use only one of those macros. Call this
/// function from a hand-written loader when one shared library must register more
/// than one source stage.
pub unsafe fn get_multi_source_extension_impl(
    ops: &[&'static SourceOps],
    version: crate::sys::MongoExtensionAPIVersion,
    host_services: *const crate::sys::MongoExtensionHostServices,
    extension_out: *mut *const MongoExtension,
) -> *mut MongoExtensionStatus {
    if host_services.is_null() || extension_out.is_null() {
        return status::new_error_status(-1, "null parameter to get_mongodb_extension");
    }
    if ops.is_empty() {
        return status::new_error_status(-1, "source extension must register at least one stage");
    }
    for (idx, lhs) in ops.iter().enumerate() {
        if lhs.name.is_empty() {
            return status::new_error_status(-1, "source stage name must not be empty");
        }
        for rhs in &ops[idx + 1..] {
            if lhs.name == rhs.name {
                return status::new_error_status(-1, format!("duplicate source stage name {}", lhs.name));
            }
        }
    }
    if !supports_selected_version(version) {
        return status::new_error_status(-1, "incompatible extension API version");
    }
    host::set_host_services(host_services);
    let addr = *EXTENSION_OBJ_ADDR.get_or_init(|| {
        let descriptors = ops
            .iter()
            .map(|ops| DescriptorObj {
                base: MongoExtensionAggStageDescriptor {
                    vtable: &DESCRIPTOR_VTABLE,
                },
                ops,
            })
            .collect();
        let p = Box::into_raw(Box::new(ExtensionObj {
            base: MongoExtension {
                vtable: &EXTENSION_VTABLE,
                version: EXTENSION_API_VERSION,
            },
            descriptors,
        }));
        p as usize
    });
    let obj = addr as *const ExtensionObj;
    unsafe {
        *extension_out = std::ptr::addr_of!((*obj).base);
    }
    status::status_ok()
}

#[cfg(test)]
mod id_lookup_expand_tests {
    use super::*;
    use crate::sys::{
        MongoExtensionAggStageParseNode, MongoExtensionByteView, MongoExtensionExpandedArrayContainer,
        MongoExtensionHostServices, MongoExtensionHostServicesVTable, MongoExtensionIdleThreadBlock,
        MongoExtensionLogger, MongoExtensionStatus, MONGO_EXTENSION_STATUS_OK,
    };

    fn open_unused(
        _doc: Document,
        _ctx: &mut StageContext,
    ) -> crate::error::Result<*mut c_void> {
        Ok(std::ptr::null_mut())
    }

    unsafe fn drop_unused(_ptr: *mut c_void) {}

    unsafe fn next_unused(
        _ptr: *mut c_void,
        _ctx: &mut StageContext,
    ) -> crate::error::Result<Next> {
        Ok(Next::Eof)
    }

    fn static_props() -> Document {
        StagePlan::source_default().static_properties_document()
    }

    fn expand_with_id_lookup(_doc: Document) -> crate::error::Result<Expansion> {
        Ok(Expansion::WithHostIdLookup {
            extension_stage: bson::doc! { "$lookupLeak": { "path": "description", "query": "boots" } },
            id_lookup: bson::doc! { "$_internalSearchIdLookup": { "limit": 1i32 } },
        })
    }

    static OPS: SourceOps = SourceOps {
        name: "$lookupLeak",
        expect_empty: false,
        open_from_doc: open_unused,
        drop_state: drop_unused,
        next: next_unused,
        on_extension_initialized: None,
        static_properties_doc: static_props,
        expand_inner: expand_with_id_lookup,
    };

    unsafe extern "C" fn null_logger() -> *mut MongoExtensionLogger {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn status_ok_view(
        _msg: MongoExtensionByteView,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn status_ok_idle(
        _out: *mut *mut MongoExtensionIdleThreadBlock,
        _name: *const std::ffi::c_char,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn unused_parse_node(
        _bson: MongoExtensionByteView,
        out: *mut *mut MongoExtensionAggStageParseNode,
    ) -> *mut MongoExtensionStatus {
        if !out.is_null() {
            unsafe { *out = std::ptr::null_mut() };
        }
        status::status_ok()
    }

    unsafe extern "C" fn reject_id_lookup(
        _bson: MongoExtensionByteView,
        out: *mut *mut MongoExtensionAggStageAstNode,
    ) -> *mut MongoExtensionStatus {
        if !out.is_null() {
            unsafe { *out = std::ptr::null_mut() };
        }
        ExtensionError::HostError {
            code: 17,
            reason: "rejected id lookup".into(),
        }
        .into_raw_status()
    }

    static SERVICES_VT: MongoExtensionHostServicesVTable = MongoExtensionHostServicesVTable {
        get_logger: null_logger,
        user_asserted: status_ok_view,
        tripwire_asserted: status_ok_view,
        mark_idle_thread_block: status_ok_idle,
        create_host_agg_stage_parse_node: unused_parse_node,
        create_id_lookup: reject_id_lookup,
    };

    #[test]
    fn failed_id_lookup_expansion_frees_extension_ast() {
        let services = MongoExtensionHostServices {
            vtable: &SERVICES_VT,
        };
        host::set_host_services(std::ptr::from_ref(&services));
        let before = LIVE_AST_NODES.load(Ordering::SeqCst);
        let args = {
            let mut bytes = Vec::new();
            Document::new().to_writer(&mut bytes).unwrap();
            bytes
        };
        let parsed = Box::into_raw(Box::new(parse_alloc(args, &OPS)))
            .cast::<MongoExtensionAggStageParseNode>();
        let mut expanded: *mut MongoExtensionExpandedArrayContainer = std::ptr::null_mut();
        let expand_status = unsafe { parse_expand(parsed, std::ptr::addr_of_mut!(expanded)) };
        assert!(!expand_status.is_null());
        unsafe {
            let code = ((*(*expand_status).vtable).get_code)(expand_status);
            assert_ne!(code, MONGO_EXTENSION_STATUS_OK);
            ((*(*expand_status).vtable).destroy)(expand_status);
            assert!(expanded.is_null());
            ((*(*parsed).vtable).destroy)(parsed);
        }
        assert_eq!(
            LIVE_AST_NODES.load(Ordering::SeqCst),
            before,
            "failed id lookup expansion leaked an AST node"
        );
    }
}
