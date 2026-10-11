//! Execution context passed to stage hooks: logging, cached extension options, metrics, and host query controls.

use std::ptr::NonNull;

use crate::error::{ExtensionError, Result};
use crate::extension_log;
use crate::host;
use crate::operation_metrics::SharedMetrics;
use crate::status;
use crate::sys::{MongoExtensionQueryExecutionContext, MONGO_EXTENSION_STATUS_OK};

/// Collection metadata captured while binding an extension stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogContext {
    /// Database name for the aggregation namespace.
    pub database_name: String,
    /// Collection name for the aggregation namespace.
    pub collection_name: String,
    /// Collection UUID string, when the host provides one.
    pub uuid: Option<String>,
    /// Whether binding happened in router context.
    pub in_router: bool,
    /// Explain verbosity active during bind.
    pub verbosity: u32,
}

impl CatalogContext {
    /// Namespace in `db.collection` form.
    pub fn namespace(&self) -> String {
        format!("{}.{}", self.database_name, self.collection_name)
    }
}

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
    metrics: Option<SharedMetrics>,
    catalog: Option<CatalogContext>,
}

impl StageContext {
    /// Constructs an empty context (no host execution binding yet).
    pub fn new() -> Self {
        Self {
            query_ctx: None,
            metrics: None,
            catalog: None,
        }
    }

    /// Crate-internal: bind catalog metadata captured during AST bind.
    pub(crate) fn bind_catalog(&mut self, catalog: Option<CatalogContext>) {
        self.catalog = catalog;
    }

    /// Crate-internal: bind host pointers for one `get_next` / generator invocation.
    pub(crate) fn bind_execution(
        &mut self,
        query_ctx: *mut MongoExtensionQueryExecutionContext,
        metrics: Option<SharedMetrics>,
    ) {
        self.query_ctx = NonNull::new(query_ctx);
        self.metrics = metrics;
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

    /// Catalog metadata for the collection being aggregated, if the host supplied it.
    pub fn catalog(&self) -> Option<&CatalogContext> {
        self.catalog.as_ref()
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
                q.as_ptr().cast::<MongoExtensionQueryExecutionContext>()
                    as *const MongoExtensionQueryExecutionContext,
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
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    reason.data,
                    reason.len as usize,
                ))
                .into_owned()
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
                q.as_ptr().cast::<MongoExtensionQueryExecutionContext>()
                    as *const MongoExtensionQueryExecutionContext,
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
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    reason.data,
                    reason.len as usize,
                ))
                .into_owned()
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
            metrics: self.metrics.as_deref(),
        }
    }
}

impl Default for StageContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental updates to the stage's BSON metrics document.
pub struct OperationMetricsSink<'a> {
    metrics: Option<&'a std::sync::Mutex<bson::Document>>,
}

impl OperationMetricsSink<'_> {
    /// Adds `delta` to the signed integer stored at `key` (defaults missing keys to `0`).
    pub fn inc(&mut self, key: &str, delta: i64) {
        if let Some(Ok(mut g)) = self.metrics.map(|m| m.lock()) {
            let prev = g.get(key).and_then(|b| b.as_i64()).unwrap_or(0);
            g.insert(key, bson::Bson::Int64(prev.saturating_add(delta)));
        }
    }

    /// Stores `elapsed_ns` for `key` (overwrites previous value).
    pub fn record_time(&mut self, key: &str, elapsed_ns: u64) {
        if let Some(Ok(mut g)) = self.metrics.map(|m| m.lock()) {
            g.insert(key, bson::Bson::Int64(elapsed_ns as i64));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{
        MongoExtensionByteView, MongoExtensionExecAggStage, MongoExtensionOperationMetrics,
        MongoExtensionQueryExecutionContextVTable, MongoExtensionStatus,
    };

    const INTERRUPTED: i32 = 11601;

    unsafe extern "C" fn killed(
        _: *const MongoExtensionQueryExecutionContext,
        query_status: *mut MongoExtensionStatus,
    ) -> *mut MongoExtensionStatus {
        let vt = (*query_status).vtable;
        ((*vt).set_code)(query_status, INTERRUPTED);
        let reason = b"operation was interrupted";
        let ret = ((*vt).set_reason)(
            query_status,
            MongoExtensionByteView {
                data: reason.as_ptr(),
                len: reason.len() as u64,
            },
        );
        ((*(*ret).vtable).destroy)(ret);
        status::status_ok()
    }

    unsafe extern "C" fn alive(
        _: *const MongoExtensionQueryExecutionContext,
        _: *mut MongoExtensionStatus,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn no_metrics(
        _: *const MongoExtensionQueryExecutionContext,
        _: *mut MongoExtensionExecAggStage,
        _: *mut *mut MongoExtensionOperationMetrics,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn no_deadline(
        _: *const MongoExtensionQueryExecutionContext,
        out: *mut i64,
    ) -> *mut MongoExtensionStatus {
        *out = i64::MAX;
        status::status_ok()
    }

    fn check_with(
        check_for_interrupt: unsafe extern "C" fn(
            *const MongoExtensionQueryExecutionContext,
            *mut MongoExtensionStatus,
        ) -> *mut MongoExtensionStatus,
    ) -> Result<()> {
        let vtable = MongoExtensionQueryExecutionContextVTable {
            check_for_interrupt,
            get_metrics: no_metrics,
            get_deadline_timestamp_ms: no_deadline,
        };
        let mut query_ctx = MongoExtensionQueryExecutionContext { vtable: &vtable };
        let mut ctx = StageContext::new();
        ctx.bind_execution(&mut query_ctx, None);
        ctx.check_interrupt()
    }

    #[test]
    fn check_interrupt_reports_killed_operation() {
        assert_eq!(
            check_with(killed),
            Err(ExtensionError::HostError {
                code: INTERRUPTED,
                reason: "operation was interrupted".into(),
            })
        );
    }

    #[test]
    fn check_interrupt_continues_for_live_operation() {
        assert_eq!(check_with(alive), Ok(()));
    }
}
