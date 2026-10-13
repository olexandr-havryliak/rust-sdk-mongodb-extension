//! ABI benchmark fixtures with matching raw and SDK source algorithms.

use bson::{doc, Document};
use extension_sdk_mongodb::{byte_buf, status, sys::*, Next, SourceStage, StageContext};
use sdk_performance_fixture::{Config, PerfSource, State};
use std::ptr;
use std::sync::{
    atomic::{AtomicPtr, Ordering},
    Once,
};

#[path = "../../../../extension-sdk-mongodb/tests/common/mod.rs"]
mod mock_host;

static INIT: Once = Once::new();
static DESCRIPTOR: AtomicPtr<MongoExtensionAggStageDescriptor> = AtomicPtr::new(ptr::null_mut());
static HOST_OWNER: AtomicPtr<mock_host::MockHost> = AtomicPtr::new(ptr::null_mut());

#[test]
fn host_owner_remains_rooted_for_process_lifetime() {
    initialize();
    assert!(!HOST_OWNER.load(Ordering::SeqCst).is_null());
}

unsafe extern "C" fn register(
    _: *const MongoExtensionHostPortal,
    descriptor: *const MongoExtensionAggStageDescriptor,
) -> *mut MongoExtensionStatus {
    DESCRIPTOR.store(descriptor.cast_mut(), Ordering::SeqCst);
    status::status_ok()
}

pub fn initialize() {
    INIT.call_once(|| unsafe {
        // The real host retains services for the library lifetime. Setup is excluded from timing.
        let host = Box::leak(mock_host::MockHost::new(register));
        HOST_OWNER.store(host, Ordering::SeqCst);
        let mut extension = ptr::null();
        check_status(sdk_performance_fixture::get_mongodb_extension(
            extension_sdk_mongodb::version::EXTENSION_API_VERSION,
            host.services(),
            &mut extension,
        ))
        .unwrap();
        check_status(((*(*extension).vtable).initialize)(
            extension,
            host.portal(),
        ))
        .unwrap();
    });
}

unsafe fn check_status(st: *mut MongoExtensionStatus) -> Result<(), String> {
    assert!(!st.is_null());
    let vt = (*st).vtable;
    let code = ((*vt).get_code)(st);
    let reason = ((*vt).get_reason)(st);
    let message = if reason.data.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(std::slice::from_raw_parts(reason.data, reason.len as usize))
            .into_owned()
    };
    ((*vt).destroy)(st);
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{code}: {message}"))
    }
}

fn view(bytes: &[u8]) -> MongoExtensionByteView {
    MongoExtensionByteView {
        data: bytes.as_ptr(),
        len: bytes.len() as u64,
    }
}

fn empty() -> MongoExtensionByteContainer {
    MongoExtensionByteContainer {
        type_: MongoExtensionByteContainerType::kByteView,
        bytes: MongoExtensionByteContainerBytes {
            view: MongoExtensionByteView {
                data: ptr::null(),
                len: 0,
            },
        },
    }
}

unsafe fn read_container(c: &MongoExtensionByteContainer) -> Option<Document> {
    let bytes = match c.type_ {
        MongoExtensionByteContainerType::kByteBuf => {
            ((*(*c.bytes.buf).vtable).get_view)(c.bytes.buf)
        }
        MongoExtensionByteContainerType::kByteView => c.bytes.view,
    };
    if bytes.len == 0 {
        None
    } else {
        Some(
            Document::from_reader(std::slice::from_raw_parts(bytes.data, bytes.len as usize))
                .unwrap(),
        )
    }
}

unsafe fn release(c: MongoExtensionByteContainer) -> u64 {
    if c.type_ == MongoExtensionByteContainerType::kByteBuf {
        let buffer = c.bytes.buf;
        let length = ((*(*buffer).vtable).get_view)(buffer).len;
        ((*(*buffer).vtable).destroy)(buffer);
        length
    } else {
        c.bytes.view.len
    }
}

unsafe fn next_result(
    exec: *mut MongoExtensionExecAggStage,
) -> Result<MongoExtensionGetNextResult, String> {
    let mut result = MongoExtensionGetNextResult {
        code: MongoExtensionGetNextResultCode::kEOF,
        result_document: empty(),
        result_metadata: empty(),
    };
    check_status(((*(*exec).vtable).get_next)(
        exec,
        ptr::null_mut(),
        &mut result,
    ))?;
    Ok(result)
}

pub type Row = (Document, Option<Document>);

unsafe fn pull(exec: *mut MongoExtensionExecAggStage) -> Result<Option<Row>, String> {
    let result = next_result(exec)?;
    let row = read_container(&result.result_document)
        .map(|document| (document, read_container(&result.result_metadata)));
    release(result.result_document);
    release(result.result_metadata);
    Ok(row)
}

unsafe fn drain(exec: *mut MongoExtensionExecAggStage) -> u64 {
    let mut bytes = 0;
    loop {
        let result = next_result(exec).unwrap();
        bytes += release(result.result_document) + release(result.result_metadata);
        if result.code == MongoExtensionGetNextResultCode::kEOF {
            return bytes;
        }
    }
}

pub struct SdkCursor(*mut MongoExtensionExecAggStage);

impl SdkCursor {
    pub fn new(args: Document) -> Self {
        initialize();
        unsafe {
            let descriptor = DESCRIPTOR.load(Ordering::SeqCst);
            let bytes = bson::to_vec(&doc! {"$sdkPerfSource": args}).unwrap();
            let mut parse = ptr::null_mut();
            check_status(((*(*descriptor).vtable).parse)(
                descriptor,
                view(&bytes),
                &mut parse,
            ))
            .unwrap();
            let mut expanded = ptr::null_mut();
            check_status(((*(*parse).vtable).expand)(parse, &mut expanded)).unwrap();
            let mut element = MongoExtensionExpandedArrayElement {
                type_: MongoExtensionAggStageNodeType::kAstNode,
                parse_or_ast: MongoExtensionExpandedArrayElementUnion {
                    ast: ptr::null_mut(),
                },
            };
            let mut array = MongoExtensionExpandedArray {
                size: 1,
                elements: &mut element,
            };
            check_status(((*(*expanded).vtable).transfer)(expanded, &mut array)).unwrap();
            ((*(*expanded).vtable).destroy)(expanded);
            ((*(*parse).vtable).destroy)(parse);
            let ast = element.parse_or_ast.ast;
            let mut logical = ptr::null_mut();
            check_status(((*(*ast).vtable).bind)(ast, ptr::null(), &mut logical)).unwrap();
            ((*(*ast).vtable).destroy)(ast);
            let mut exec = ptr::null_mut();
            check_status(((*(*logical).vtable).compile)(logical, &mut exec)).unwrap();
            ((*(*logical).vtable).destroy)(logical);
            check_status(((*(*exec).vtable).open)(exec)).unwrap();
            Self(exec)
        }
    }
    pub fn pull(&mut self) -> Result<Option<Row>, String> {
        unsafe { pull(self.0) }
    }
    pub fn drain_bytes(&mut self) -> u64 {
        unsafe { drain(self.0) }
    }
    pub fn close(&mut self) {
        unsafe {
            check_status(((*(*self.0).vtable).close)(self.0)).unwrap();
        }
    }
}

impl Drop for SdkCursor {
    fn drop(&mut self) {
        unsafe {
            ((*(*self.0).vtable).destroy)(self.0);
        }
    }
}

#[repr(C)]
struct RawExec {
    base: MongoExtensionExecAggStage,
    state: State,
}

unsafe extern "C" fn raw_destroy(exec: *mut MongoExtensionExecAggStage) {
    drop(Box::from_raw(exec.cast::<RawExec>()));
}
unsafe extern "C" fn raw_next(
    exec: *mut MongoExtensionExecAggStage,
    _: *mut MongoExtensionQueryExecutionContext,
    out: *mut MongoExtensionGetNextResult,
) -> *mut MongoExtensionStatus {
    let result = extension_sdk_mongodb::panics::ffi_boundary(|| {
        let next = PerfSource::next(
            &mut (*exec.cast::<RawExec>()).state,
            &mut StageContext::new(),
        )?;
        *out = MongoExtensionGetNextResult {
            code: MongoExtensionGetNextResultCode::kEOF,
            result_document: empty(),
            result_metadata: empty(),
        };
        if let Next::Advanced { document, metadata } = next {
            (*out).code = MongoExtensionGetNextResultCode::kAdvanced;
            (*out).result_document = MongoExtensionByteContainer {
                type_: MongoExtensionByteContainerType::kByteBuf,
                bytes: MongoExtensionByteContainerBytes {
                    buf: byte_buf::from_bson(&document).unwrap(),
                },
            };
            if let Some(metadata) = metadata {
                (*out).result_metadata = MongoExtensionByteContainer {
                    type_: MongoExtensionByteContainerType::kByteBuf,
                    bytes: MongoExtensionByteContainerBytes {
                        buf: byte_buf::from_bson(&metadata).unwrap(),
                    },
                };
            }
        }
        Ok::<(), extension_sdk_mongodb::ExtensionError>(())
    });
    match result {
        Some(Ok(())) => status::status_ok(),
        Some(Err(error)) => error.into_raw_status(),
        None => status::new_error_status(-1, "raw fixture panic"),
    }
}
unsafe extern "C" fn raw_name(_: *const MongoExtensionExecAggStage) -> MongoExtensionByteView {
    view(b"$rawPerfSource")
}
unsafe extern "C" fn raw_metrics(
    _: *const MongoExtensionExecAggStage,
    out: *mut *mut MongoExtensionOperationMetrics,
) -> *mut MongoExtensionStatus {
    *out = ptr::null_mut();
    status::new_error_status(-1, "not used by raw benchmark")
}
unsafe extern "C" fn raw_set_source(
    _: *mut MongoExtensionExecAggStage,
    _: *mut MongoExtensionExecAggStage,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}
unsafe extern "C" fn raw_open(_: *mut MongoExtensionExecAggStage) -> *mut MongoExtensionStatus {
    status::status_ok()
}
unsafe extern "C" fn raw_explain(
    _: *const MongoExtensionExecAggStage,
    _: *mut MongoExtensionQueryExecutionContext,
    _: MongoExtensionExplainVerbosity,
    out: *mut *mut MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    *out = ptr::null_mut();
    status::new_error_status(-1, "not used by raw benchmark")
}
static RAW: MongoExtensionExecAggStageVTable = MongoExtensionExecAggStageVTable {
    destroy: raw_destroy,
    get_next: raw_next,
    get_name: raw_name,
    create_metrics: raw_metrics,
    set_source: raw_set_source,
    open: raw_open,
    reopen: raw_open,
    close: raw_open,
    explain: raw_explain,
};

pub struct RawCursor(*mut MongoExtensionExecAggStage);
impl RawCursor {
    pub fn new(args: Document) -> Self {
        let config = Config::parse(args).unwrap();
        let state = PerfSource::open(config, &mut StageContext::new()).unwrap();
        Self(
            Box::into_raw(Box::new(RawExec {
                base: MongoExtensionExecAggStage { vtable: &RAW },
                state,
            }))
            .cast(),
        )
    }
    pub fn pull(&mut self) -> Result<Option<Row>, String> {
        unsafe { pull(self.0) }
    }
    pub fn drain_bytes(&mut self) -> u64 {
        unsafe { drain(self.0) }
    }
}
impl Drop for RawCursor {
    fn drop(&mut self) {
        unsafe { raw_destroy(self.0) }
    }
}

pub fn sdk_source(rows: i64, bytes: usize, metadata: bool) -> u64 {
    SdkCursor::new(doc! {"rows": rows, "payload_bytes": bytes as i64, "metadata": metadata})
        .drain_bytes()
}
pub fn raw_source(rows: i64, bytes: usize, metadata: bool) -> u64 {
    RawCursor::new(doc! {"rows": rows, "payload_bytes": bytes as i64, "metadata": metadata})
        .drain_bytes()
}
pub fn bson_roundtrip(bytes: usize) -> usize {
    let input = doc! {"_id": 1i64, "payload": "x".repeat(bytes)};
    let encoded = bson::to_vec(&input).unwrap();
    let decoded = Document::from_reader(encoded.as_slice()).unwrap();
    std::hint::black_box(decoded);
    encoded.len()
}
pub fn status_cycle() {
    unsafe {
        let st = status::new_error_status(11601, "operation was interrupted");
        ((*(*st).vtable).destroy)(st);
    }
}

pub fn metrics_roundtrip(rows: i64) -> Document {
    let mut cursor = SdkCursor::new(doc! {"rows": rows});
    unsafe {
        let mut metrics = ptr::null_mut();
        check_status(((*(*cursor.0).vtable).create_metrics)(
            cursor.0,
            &mut metrics,
        ))
        .unwrap();
        cursor.drain_bytes();
        drop(cursor);
        let mut buffer = ptr::null_mut();
        check_status(((*(*metrics).vtable).serialize)(metrics, &mut buffer)).unwrap();
        let container = MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteBuf,
            bytes: MongoExtensionByteContainerBytes { buf: buffer },
        };
        let document = read_container(&container).unwrap();
        release(container);
        ((*(*metrics).vtable).destroy)(metrics);
        document
    }
}
pub fn buffer_cycle(bytes: usize) -> u64 {
    let document = doc! {"payload": "x".repeat(bytes)};
    unsafe {
        release(MongoExtensionByteContainer {
            type_: MongoExtensionByteContainerType::kByteBuf,
            bytes: MongoExtensionByteContainerBytes {
                buf: byte_buf::from_bson(&document).unwrap(),
            },
        })
    }
}
