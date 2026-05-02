//! Host logger bridge (crate-internal).

use crate::host;
use crate::sys::{
    MongoExtensionLogAttributesArray, MongoExtensionLogMessage, MongoExtensionLogMessageSeverityOrLevel,
    MongoExtensionLogSeverity, MongoExtensionLogType,
};

fn log_with(
    severity: MongoExtensionLogSeverity,
    typ: MongoExtensionLogType,
    debug_level: i32,
    message: &str,
) {
    let Some(svcs) = host::host_services_vtable() else {
        return;
    };
    let logger = unsafe { ((*svcs).get_logger)() };
    if logger.is_null() {
        return;
    }
    let lvt = unsafe { (*logger).vtable };
    let mut allow = true;
    let st = unsafe { ((*lvt).should_log)(severity, typ, std::ptr::addr_of_mut!(allow)) };
    if !st.is_null() {
        unsafe {
            let svt = (*st).vtable;
            ((*svt).destroy)(st);
        }
    }
    if !allow {
        return;
    }
    let bytes = message.as_bytes();
    let msg = crate::sys::MongoExtensionByteView {
        data: bytes.as_ptr(),
        len: bytes.len() as u64,
    };
    let sev_or = match typ {
        MongoExtensionLogType::kDebug => MongoExtensionLogMessageSeverityOrLevel { level: debug_level },
        _ => MongoExtensionLogMessageSeverityOrLevel { severity },
    };
    let lm = MongoExtensionLogMessage {
        code: 0,
        message: msg,
        type_: typ,
        attributes: MongoExtensionLogAttributesArray {
            size: 0,
            elements: std::ptr::null_mut(),
        },
        severity_or_level: sev_or,
    };
    let out = unsafe { ((*lvt).log)(std::ptr::from_ref(&lm)) };
    if !out.is_null() {
        unsafe {
            let ovt = (*out).vtable;
            ((*ovt).destroy)(out);
        }
    }
}

/// `info` / `warning` / `error` use [`MongoExtensionLogType::kLog`](MongoExtensionLogType::kLog).
pub(crate) fn log_info(message: &str) {
    log_with(
        MongoExtensionLogSeverity::kInfo,
        MongoExtensionLogType::kLog,
        0,
        message,
    );
}

pub(crate) fn log_warn(message: &str) {
    log_with(
        MongoExtensionLogSeverity::kWarning,
        MongoExtensionLogType::kLog,
        0,
        message,
    );
}

pub(crate) fn log_error(message: &str) {
    log_with(
        MongoExtensionLogSeverity::kError,
        MongoExtensionLogType::kLog,
        0,
        message,
    );
}

/// Debug logs use [`MongoExtensionLogType::kDebug`](MongoExtensionLogType::kDebug); `level` is passed to the host.
pub(crate) fn log_debug(level: i32, message: &str) {
    log_with(
        MongoExtensionLogSeverity::kInfo,
        MongoExtensionLogType::kDebug,
        level,
        message,
    );
}
