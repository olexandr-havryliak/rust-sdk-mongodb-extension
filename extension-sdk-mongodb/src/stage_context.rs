//! Execution context passed to stage hooks: logging, cached extension options, metrics, and host query controls.

use std::ptr::NonNull;

use crate::error::{ExtensionError, Result};
use crate::extension_log;
use crate::host;
use crate::operation_metrics::SdkOperationMetrics;
use crate::status;
use crate::sys::{MongoExtensionOperationMetrics, MongoExtensionQueryExecutionContext, MONGO_EXTENSION_STATUS_OK};

/// Context available during stage `open` / `next` / `transform` callbacks.
///
/// During **`get_next`**, the SDK binds the host query execution context and metrics pointer so
/// logging, [`StageContext::deadline_timestamp_ms`](StageContext::deadline_timestamp_ms),
/// [`StageContext::check_interrupt`](StageContext::check_interrupt), and
/// [`StageContext::metrics`](StageContext::metrics) can call into the host. Earlier phases (e.g.
/// parse-only paths) may leave these unset; calls degrade gracefully (logging no-ops without a
/// logger, etc.).
#[derive(Debug)]
pub struct StageContext {
    query_ctx: Option<NonNull<MongoExtensionQueryExecutionContext>>,
    metrics: Option<NonNull<MongoExtensionOperationMetrics>>,
}

impl StageContext {
    /// Constructs an empty context (no host execution binding yet).
    pub fn new() -> Self {
        Self {
            query_ctx: None,
            metrics: None,
        }
    }

    /// Crate-internal: bind host pointers for one `get_next` / generator invocation.
    pub(crate) fn bind_execution(
        &mut self,
        query_ctx: *mut MongoExtensionQueryExecutionContext,
        metrics: *mut MongoExtensionOperationMetrics,
    ) {
        self.query_ctx = NonNull::new(query_ctx);
        self.metrics = NonNull::new(metrics);
    }

    /// Clears execution binding (used after a generator step completes internal work).
    pub(crate) fn unbind_execution(&mut self) {
        self.query_ctx = None;
        self.metrics = None;
    }

    /// Extension options YAML / config blob captured during extension `initialize` (copy of host bytes).
    ///
    /// Returns `None` if the host did not provide options or the snapshot was not cached yet.
    pub fn extension_options_raw(&self) -> Option<Vec<u8>> {
        host::extension_options_snapshot()
    }

    /// Log at **info** severity (host logger; no-op if services or logger are unavailable).
    pub fn log_info(&mut self, message: &str) {
        extension_log::log_info(message);
    }

    /// Log at **debug** type with numeric `level` (host-defined verbosity).
    pub fn log_debug(&mut self, level: i32, message: &str) {
        extension_log::log_debug(level, message);
    }

    /// Log at **warning** severity.
    pub fn log_warn(&mut self, message: &str) {
        extension_log::log_warn(message);
    }

    /// Log at **error** severity.
    pub fn log_error(&mut self, message: &str) {
        extension_log::log_error(message);
    }

    /// Host deadline in milliseconds since epoch, if the query execution context exposes it.
    pub fn deadline_timestamp_ms(&self) -> Result<Option<i64>> {
        let Some(q) = self.query_ctx else {
            return Ok(None);
        };
        unsafe {
            let vt = q.as_ref().vtable;
            let mut ts: i64 = 0;
            let st = ((*vt).get_deadline_timestamp_ms)(
                q.as_ptr().cast::<MongoExtensionQueryExecutionContext>() as *const MongoExtensionQueryExecutionContext,
                std::ptr::addr_of_mut!(ts),
            );
            if st.is_null() {
                return Ok(Some(ts));
            }
            let svt = (*st).vtable;
            let code = ((*svt).get_code)(st);
            let reason = ((*svt).get_reason)(st);
            let msg = if reason.data.is_null() || reason.len == 0 {
                "get_deadline_timestamp_ms failed".into()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(reason.data, reason.len as usize)).into_owned()
            };
            ((*svt).destroy)(st);
            if code == MONGO_EXTENSION_STATUS_OK {
                return Ok(Some(ts));
            }
            Err(ExtensionError::HostError { code, reason: msg })
        }
    }

    /// Ask the host whether the operation should yield (interrupt / kill). Ok(()) means continue.
    pub fn check_interrupt(&mut self) -> Result<()> {
        let Some(q) = self.query_ctx else {
            return Ok(());
        };
        unsafe {
            let vt = q.as_ref().vtable;
            // Host populates this extension-owned status (see `mongodb_extension_api.h`).
            let qs = status::new_error_status(MONGO_EXTENSION_STATUS_OK, "");
            let ret = ((*vt).check_for_interrupt)(
                q.as_ptr().cast::<MongoExtensionQueryExecutionContext>() as *const MongoExtensionQueryExecutionContext,
                qs,
            );
            if !ret.is_null() {
                let rvt = (*ret).vtable;
                ((*rvt).destroy)(ret);
            }
            let svt = (*qs).vtable;
            let code = ((*svt).get_code)(qs);
            let reason = ((*svt).get_reason)(qs);
            let msg = if reason.data.is_null() || reason.len == 0 {
                "check_for_interrupt".into()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(reason.data, reason.len as usize)).into_owned()
            };
            ((*svt).destroy)(qs);
            if code == MONGO_EXTENSION_STATUS_OK {
                return Ok(());
            }
            Err(ExtensionError::HostError { code, reason: msg })
        }
    }

    /// Writable view of BSON metrics for this stage (serialized to the host on demand).
    ///
    /// No-op if metrics were not bound for this call.
    pub fn metrics(&mut self) -> OperationMetricsSink<'_> {
        OperationMetricsSink {
            metrics: self.metrics,
            _lt: std::marker::PhantomData,
        }
    }
}

impl Default for StageContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental updates to [`SdkOperationMetrics`](crate::operation_metrics::SdkOperationMetrics).
pub struct OperationMetricsSink<'a> {
    metrics: Option<NonNull<MongoExtensionOperationMetrics>>,
    _lt: std::marker::PhantomData<&'a mut ()>,
}

impl OperationMetricsSink<'_> {
    /// Adds `delta` to the signed integer stored at `key` (defaults missing keys to `0`).
    pub fn inc(&mut self, key: &str, delta: i64) {
        let Some(p) = self.metrics else {
            return;
        };
        let p = p.as_ptr().cast::<SdkOperationMetrics>();
        unsafe {
            if let Ok(mut g) = (*p).doc.lock() {
                let prev = g.get(key).and_then(|b| b.as_i64()).unwrap_or(0);
                g.insert(key, bson::Bson::Int64(prev.saturating_add(delta)));
            }
        }
    }

    /// Stores `elapsed_ns` for `key` (overwrites previous value).
    pub fn record_time(&mut self, key: &str, elapsed_ns: u64) {
        let Some(p) = self.metrics else {
            return;
        };
        let p = p.as_ptr().cast::<SdkOperationMetrics>();
        unsafe {
            if let Ok(mut g) = (*p).doc.lock() {
                g.insert(key, bson::Bson::Int64(elapsed_ns as i64));
            }
        }
    }
}
