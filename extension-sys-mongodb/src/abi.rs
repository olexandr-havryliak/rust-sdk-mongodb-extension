//! Manual `#[repr(C)]` layout matching `mongodb_extension_api.h` (MongoDB Extensions public API).

/// Must match `MONGODB_EXTENSION_API_MAJOR_VERSION` in `api.h`.
pub const MONGODB_EXTENSION_API_MAJOR_VERSION: u32 = 0;
/// Must match `MONGODB_EXTENSION_API_MINOR_VERSION` in `api.h`.
pub const MONGODB_EXTENSION_API_MINOR_VERSION: u32 = 1;

pub const MONGO_EXTENSION_STATUS_RUNTIME_ERROR: i32 = -1;
pub const MONGO_EXTENSION_STATUS_OK: i32 = 0;

pub const GET_MONGODB_EXTENSION_SYMBOL: &[u8] = b"get_mongodb_extension\0";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionAPIVersion {
    pub major: u32,
    pub minor: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionAPIVersionVector {
    pub len: u64,
    pub versions: *mut MongoExtensionAPIVersion,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionByteView {
    pub data: *const u8,
    pub len: u64,
}

#[repr(C)]
pub struct MongoExtensionByteBuf {
    pub vtable: *const MongoExtensionByteBufVTable,
}

#[repr(C)]
pub struct MongoExtensionByteBufVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionByteBuf),
    pub get_view: unsafe extern "C" fn(*const MongoExtensionByteBuf) -> MongoExtensionByteView,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MongoExtensionByteContainerType {
    kByteView = 0,
    kByteBuf = 1,
}

#[repr(C)]
pub union MongoExtensionByteContainerBytes {
    pub view: MongoExtensionByteView,
    pub buf: *mut MongoExtensionByteBuf,
}

#[repr(C)]
pub struct MongoExtensionByteContainer {
    pub type_: MongoExtensionByteContainerType,
    pub bytes: MongoExtensionByteContainerBytes,
}

#[repr(C)]
pub struct MongoExtensionStatus {
    pub vtable: *const MongoExtensionStatusVTable,
}

#[repr(C)]
pub struct MongoExtensionStatusVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionStatus),
    pub get_code: unsafe extern "C" fn(*const MongoExtensionStatus) -> i32,
    pub get_reason: unsafe extern "C" fn(*const MongoExtensionStatus) -> MongoExtensionByteView,
    pub set_code: unsafe extern "C" fn(*mut MongoExtensionStatus, i32),
    pub set_reason:
        unsafe extern "C" fn(*mut MongoExtensionStatus, MongoExtensionByteView) -> *mut MongoExtensionStatus,
    pub clone: unsafe extern "C" fn(*const MongoExtensionStatus, *mut *mut MongoExtensionStatus) -> *mut MongoExtensionStatus,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionLogSeverity {
    kError = 0,
    kWarning = 1,
    kInfo = 2,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionLogType {
    kLog = 0,
    kDebug = 1,
}

#[repr(C)]
pub struct MongoExtensionOperationMetrics {
    pub vtable: *const MongoExtensionOperationMetricsVTable,
}

#[repr(C)]
pub struct MongoExtensionOperationMetricsVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionOperationMetrics),
    pub serialize: unsafe extern "C" fn(
        *const MongoExtensionOperationMetrics,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub update: unsafe extern "C" fn(
        *mut MongoExtensionOperationMetrics,
        MongoExtensionByteView,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionLogAttribute {
    pub name: MongoExtensionByteView,
    pub value: MongoExtensionByteView,
}

#[repr(C)]
pub struct MongoExtensionLogAttributesArray {
    pub size: u64,
    pub elements: *mut MongoExtensionLogAttribute,
}

#[repr(C)]
pub union MongoExtensionLogMessageSeverityOrLevel {
    pub severity: MongoExtensionLogSeverity,
    pub level: i32,
}

#[repr(C)]
pub struct MongoExtensionLogMessage {
    pub code: u32,
    pub message: MongoExtensionByteView,
    pub type_: MongoExtensionLogType,
    pub attributes: MongoExtensionLogAttributesArray,
    pub severity_or_level: MongoExtensionLogMessageSeverityOrLevel,
}

#[repr(C)]
pub struct MongoExtensionLogger {
    pub vtable: *const MongoExtensionLoggerVTable,
}

#[repr(C)]
pub struct MongoExtensionLoggerVTable {
    pub log: unsafe extern "C" fn(*const MongoExtensionLogMessage) -> *mut MongoExtensionStatus,
    pub should_log: unsafe extern "C" fn(
        MongoExtensionLogSeverity,
        MongoExtensionLogType,
        *mut bool,
    ) -> *mut MongoExtensionStatus,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionExplainVerbosity {
    kNotExplain = 0,
    kQueryPlanner = 1,
    kExecStats = 2,
    kExecAllPlans = 3,
}

#[repr(C)]
pub struct MongoExtensionHostQueryShapeOpts {
    pub vtable: *const MongoExtensionHostQueryShapeOptsVTable,
}

#[repr(C)]
pub struct MongoExtensionHostQueryShapeOptsVTable {
    pub serialize_identifier: unsafe extern "C" fn(
        *const MongoExtensionHostQueryShapeOpts,
        MongoExtensionByteView,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub serialize_field_path: unsafe extern "C" fn(
        *const MongoExtensionHostQueryShapeOpts,
        MongoExtensionByteView,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub serialize_literal: unsafe extern "C" fn(
        *const MongoExtensionHostQueryShapeOpts,
        MongoExtensionByteView,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionNamespaceString {
    pub database_name: MongoExtensionByteView,
    pub collection_name: MongoExtensionByteView,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionViewInfo {
    pub view_namespace: MongoExtensionNamespaceString,
    pub view_pipeline_len: usize,
    pub view_pipeline: *const MongoExtensionByteView,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MongoExtensionCatalogContext {
    pub namespace_string: MongoExtensionNamespaceString,
    pub uuid_string: MongoExtensionByteView,
    pub in_router: u8,
    pub verbosity: MongoExtensionExplainVerbosity,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionAggStageNodeType {
    kParseNode = 0,
    kAstNode = 1,
}

#[repr(C)]
pub struct MongoExtensionAggStageDescriptor {
    pub vtable: *const MongoExtensionAggStageDescriptorVTable,
}

#[repr(C)]
pub struct MongoExtensionAggStageDescriptorVTable {
    pub get_name: unsafe extern "C" fn(*const MongoExtensionAggStageDescriptor) -> MongoExtensionByteView,
    pub parse: unsafe extern "C" fn(
        *const MongoExtensionAggStageDescriptor,
        MongoExtensionByteView,
        *mut *mut MongoExtensionAggStageParseNode,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionAggStageParseNode {
    pub vtable: *const MongoExtensionAggStageParseNodeVTable,
}

#[repr(C)]
pub struct MongoExtensionAggStageAstNode {
    pub vtable: *const MongoExtensionAggStageAstNodeVTable,
}

#[repr(C)]
pub union MongoExtensionExpandedArrayElementUnion {
    pub parse: *mut MongoExtensionAggStageParseNode,
    pub ast: *mut MongoExtensionAggStageAstNode,
}

#[repr(C)]
pub struct MongoExtensionExpandedArrayElement {
    pub type_: MongoExtensionAggStageNodeType,
    pub parse_or_ast: MongoExtensionExpandedArrayElementUnion,
}

#[repr(C)]
pub struct MongoExtensionExpandedArray {
    pub size: usize,
    pub elements: *mut MongoExtensionExpandedArrayElement,
}

#[repr(C)]
pub struct MongoExtensionExpandedArrayContainer {
    pub vtable: *const MongoExtensionExpandedArrayContainerVTable,
}

#[repr(C)]
pub struct MongoExtensionExpandedArrayContainerVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionExpandedArrayContainer),
    pub size: unsafe extern "C" fn(*const MongoExtensionExpandedArrayContainer) -> usize,
    pub transfer: unsafe extern "C" fn(
        *mut MongoExtensionExpandedArrayContainer,
        *mut MongoExtensionExpandedArray,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionAggStageParseNodeVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionAggStageParseNode),
    pub get_name: unsafe extern "C" fn(*const MongoExtensionAggStageParseNode) -> MongoExtensionByteView,
    pub get_query_shape: unsafe extern "C" fn(
        *const MongoExtensionAggStageParseNode,
        *const MongoExtensionHostQueryShapeOpts,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub expand: unsafe extern "C" fn(
        *const MongoExtensionAggStageParseNode,
        *mut *mut MongoExtensionExpandedArrayContainer,
    ) -> *mut MongoExtensionStatus,
    pub clone: unsafe extern "C" fn(
        *const MongoExtensionAggStageParseNode,
        *mut *mut MongoExtensionAggStageParseNode,
    ) -> *mut MongoExtensionStatus,
    pub to_bson_for_log: unsafe extern "C" fn(
        *const MongoExtensionAggStageParseNode,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionFirstStageViewApplicationPolicy {
    kDefaultPrepend = 0,
    kDoNothing = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MongoExtensionGetNextResultCode {
    kAdvanced = 0,
    kEOF = 1,
    kPauseExecution = 2,
}

#[repr(C)]
pub struct MongoExtensionGetNextResult {
    pub code: MongoExtensionGetNextResultCode,
    pub result_document: MongoExtensionByteContainer,
    pub result_metadata: MongoExtensionByteContainer,
}

#[repr(C)]
pub struct MongoExtensionExecAggStage {
    pub vtable: *const MongoExtensionExecAggStageVTable,
}

#[repr(C)]
pub struct MongoExtensionQueryExecutionContext {
    pub vtable: *const MongoExtensionQueryExecutionContextVTable,
}

#[repr(C)]
pub struct MongoExtensionExecAggStageVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionExecAggStage),
    pub get_next: unsafe extern "C" fn(
        *mut MongoExtensionExecAggStage,
        *mut MongoExtensionQueryExecutionContext,
        *mut MongoExtensionGetNextResult,
    ) -> *mut MongoExtensionStatus,
    pub get_name: unsafe extern "C" fn(*const MongoExtensionExecAggStage) -> MongoExtensionByteView,
    pub create_metrics: unsafe extern "C" fn(
        *const MongoExtensionExecAggStage,
        *mut *mut MongoExtensionOperationMetrics,
    ) -> *mut MongoExtensionStatus,
    pub set_source: unsafe extern "C" fn(
        *mut MongoExtensionExecAggStage,
        *mut MongoExtensionExecAggStage,
    ) -> *mut MongoExtensionStatus,
    pub open: unsafe extern "C" fn(*mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus,
    pub reopen: unsafe extern "C" fn(*mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus,
    pub close: unsafe extern "C" fn(*mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus,
    pub explain: unsafe extern "C" fn(
        *const MongoExtensionExecAggStage,
        MongoExtensionExplainVerbosity,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionQueryExecutionContextVTable {
    pub check_for_interrupt: unsafe extern "C" fn(
        *const MongoExtensionQueryExecutionContext,
        *mut MongoExtensionStatus,
    ) -> *mut MongoExtensionStatus,
    pub get_metrics: unsafe extern "C" fn(
        *const MongoExtensionQueryExecutionContext,
        *mut MongoExtensionExecAggStage,
        *mut *mut MongoExtensionOperationMetrics,
    ) -> *mut MongoExtensionStatus,
    pub get_deadline_timestamp_ms: unsafe extern "C" fn(
        *const MongoExtensionQueryExecutionContext,
        *mut i64,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionLogicalAggStage {
    pub vtable: *const MongoExtensionLogicalAggStageVTable,
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum MongoExtensionDPLArrayElementType {
    kParse = 0,
    kLogical = 1,
}

#[repr(C)]
pub union MongoExtensionDPLArrayElementUnion {
    pub parse_node: *mut MongoExtensionAggStageParseNode,
    pub logical_stage: *mut MongoExtensionLogicalAggStage,
}

#[repr(C)]
pub struct MongoExtensionDPLArrayElement {
    pub type_: MongoExtensionDPLArrayElementType,
    pub element: MongoExtensionDPLArrayElementUnion,
}

#[repr(C)]
pub struct MongoExtensionDPLArray {
    pub size: usize,
    pub elements: *mut MongoExtensionDPLArrayElement,
}

#[repr(C)]
pub struct MongoExtensionDPLArrayContainer {
    pub vtable: *const MongoExtensionDPLArrayContainerVTable,
}

#[repr(C)]
pub struct MongoExtensionDPLArrayContainerVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionDPLArrayContainer),
    pub size: unsafe extern "C" fn(*const MongoExtensionDPLArrayContainer) -> usize,
    pub transfer: unsafe extern "C" fn(
        *mut MongoExtensionDPLArrayContainer,
        *mut MongoExtensionDPLArray,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionDistributedPlanLogic {
    pub vtable: *const MongoExtensionDistributedPlanLogicVTable,
}

#[repr(C)]
pub struct MongoExtensionDistributedPlanLogicVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionDistributedPlanLogic),
    pub extract_shards_pipeline: unsafe extern "C" fn(
        *mut MongoExtensionDistributedPlanLogic,
        *mut *mut MongoExtensionDPLArrayContainer,
    ) -> *mut MongoExtensionStatus,
    pub extract_merging_pipeline: unsafe extern "C" fn(
        *mut MongoExtensionDistributedPlanLogic,
        *mut *mut MongoExtensionDPLArrayContainer,
    ) -> *mut MongoExtensionStatus,
    pub get_sort_pattern: unsafe extern "C" fn(
        *const MongoExtensionDistributedPlanLogic,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionLogicalAggStageVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionLogicalAggStage),
    pub get_name: unsafe extern "C" fn(*const MongoExtensionLogicalAggStage) -> MongoExtensionByteView,
    pub serialize: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub explain: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        MongoExtensionExplainVerbosity,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub compile: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        *mut *mut MongoExtensionExecAggStage,
    ) -> *mut MongoExtensionStatus,
    pub get_distributed_plan_logic: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        *mut *mut MongoExtensionDistributedPlanLogic,
    ) -> *mut MongoExtensionStatus,
    pub clone: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        *mut *mut MongoExtensionLogicalAggStage,
    ) -> *mut MongoExtensionStatus,
    pub is_stage_sorted_by_vector_search_score: unsafe extern "C" fn(
        *const MongoExtensionLogicalAggStage,
        *mut bool,
    ) -> *mut MongoExtensionStatus,
    pub set_vector_search_limit_for_optimization: unsafe extern "C" fn(
        *mut MongoExtensionLogicalAggStage,
        *mut i64,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionAggStageAstNodeVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionAggStageAstNode),
    pub get_name: unsafe extern "C" fn(*const MongoExtensionAggStageAstNode) -> MongoExtensionByteView,
    pub get_properties: unsafe extern "C" fn(
        *const MongoExtensionAggStageAstNode,
        *mut *mut MongoExtensionByteBuf,
    ) -> *mut MongoExtensionStatus,
    pub bind: unsafe extern "C" fn(
        *const MongoExtensionAggStageAstNode,
        *const MongoExtensionCatalogContext,
        *mut *mut MongoExtensionLogicalAggStage,
    ) -> *mut MongoExtensionStatus,
    pub clone: unsafe extern "C" fn(
        *const MongoExtensionAggStageAstNode,
        *mut *mut MongoExtensionAggStageAstNode,
    ) -> *mut MongoExtensionStatus,
    pub get_first_stage_view_application_policy: unsafe extern "C" fn(
        *const MongoExtensionAggStageAstNode,
        *mut MongoExtensionFirstStageViewApplicationPolicy,
    ) -> *mut MongoExtensionStatus,
    pub bind_view_info: unsafe extern "C" fn(
        *mut MongoExtensionAggStageAstNode,
        *const MongoExtensionViewInfo,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionIdleThreadBlock {
    pub vtable: *const MongoExtensionIdleThreadBlockVTable,
}

#[repr(C)]
pub struct MongoExtensionIdleThreadBlockVTable {
    pub destroy: unsafe extern "C" fn(*mut MongoExtensionIdleThreadBlock),
}

#[repr(C)]
pub struct MongoExtension {
    pub vtable: *const MongoExtensionVTable,
    pub version: MongoExtensionAPIVersion,
}

#[repr(C)]
pub struct MongoExtensionHostPortal {
    pub vtable: *const MongoExtensionHostPortalVTable,
    pub host_extensions_api_version: MongoExtensionAPIVersion,
    pub host_mongodb_max_wire_version: i32,
}

#[repr(C)]
pub struct MongoExtensionHostPortalVTable {
    pub register_stage_descriptor: unsafe extern "C" fn(
        *const MongoExtensionHostPortal,
        *const MongoExtensionAggStageDescriptor,
    ) -> *mut MongoExtensionStatus,
    pub get_extension_options: unsafe extern "C" fn(*const MongoExtensionHostPortal) -> MongoExtensionByteView,
}

#[repr(C)]
pub struct MongoExtensionHostServices {
    pub vtable: *const MongoExtensionHostServicesVTable,
}

#[repr(C)]
pub struct MongoExtensionHostServicesVTable {
    pub get_logger: unsafe extern "C" fn() -> *mut MongoExtensionLogger,
    pub user_asserted: unsafe extern "C" fn(MongoExtensionByteView) -> *mut MongoExtensionStatus,
    pub tripwire_asserted: unsafe extern "C" fn(MongoExtensionByteView) -> *mut MongoExtensionStatus,
    pub mark_idle_thread_block: unsafe extern "C" fn(
        *mut *mut MongoExtensionIdleThreadBlock,
        *const std::ffi::c_char,
    ) -> *mut MongoExtensionStatus,
    pub create_host_agg_stage_parse_node: unsafe extern "C" fn(
        MongoExtensionByteView,
        *mut *mut MongoExtensionAggStageParseNode,
    ) -> *mut MongoExtensionStatus,
    pub create_id_lookup: unsafe extern "C" fn(
        MongoExtensionByteView,
        *mut *mut MongoExtensionAggStageAstNode,
    ) -> *mut MongoExtensionStatus,
}

#[repr(C)]
pub struct MongoExtensionVTable {
    pub initialize: unsafe extern "C" fn(
        *const MongoExtension,
        *const MongoExtensionHostPortal,
        *const MongoExtensionHostServices,
    ) -> *mut MongoExtensionStatus,
}

pub type get_mongo_extension_t = unsafe extern "C" fn(
    *const MongoExtensionAPIVersionVector,
    *mut *const MongoExtension,
) -> *mut MongoExtensionStatus;
