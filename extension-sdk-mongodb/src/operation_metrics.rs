//! BSON-backed [`MongoExtensionOperationMetrics`](crate::sys::MongoExtensionOperationMetrics) for stages.

use std::sync::Mutex;

use bson::Document;

use crate::byte_buf;
use crate::status;
use crate::sys::{
    MongoExtensionByteView, MongoExtensionOperationMetrics, MongoExtensionOperationMetricsVTable,
    MongoExtensionStatus,
};

#[repr(C)]
pub(crate) struct SdkOperationMetrics {
    pub base: MongoExtensionOperationMetrics,
    pub(crate) doc: Mutex<Document>,
}

unsafe extern "C" fn sdk_met_destroy(p: *mut MongoExtensionOperationMetrics) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<SdkOperationMetrics>()));
}

unsafe extern "C" fn sdk_met_serialize(
    p: *const MongoExtensionOperationMetrics,
    out: *mut *mut crate::sys::MongoExtensionByteBuf,
) -> *mut MongoExtensionStatus {
    let this = p.cast::<SdkOperationMetrics>();
    let d = (*this).doc.lock().map(|g| g.clone()).unwrap_or_default();
    match byte_buf::from_bson(&d) {
        Ok(b) => {
            *out = b;
            status::status_ok()
        }
        Err(e) => {
            *out = std::ptr::null_mut();
            crate::error::ExtensionError::FailedToParse(e.to_string()).into_raw_status()
        }
    }
}

unsafe extern "C" fn sdk_met_update(p: *mut MongoExtensionOperationMetrics, patch: MongoExtensionByteView) -> *mut MongoExtensionStatus {
    let this = p.cast::<SdkOperationMetrics>();
    if patch.data.is_null() || patch.len == 0 {
        return status::status_ok();
    }
    let bytes = std::slice::from_raw_parts(patch.data, patch.len as usize);
    let Ok(extra) = Document::from_reader(bytes) else {
        return status::status_ok();
    };
    if let Ok(mut g) = (*this).doc.lock() {
        for (k, v) in extra {
            g.insert(k, v);
        }
    }
    status::status_ok()
}

static SDK_METRICS_VTABLE: MongoExtensionOperationMetricsVTable = MongoExtensionOperationMetricsVTable {
    destroy: sdk_met_destroy,
    serialize: sdk_met_serialize,
    update: sdk_met_update,
};

pub(crate) fn alloc_sdk_operation_metrics() -> *mut MongoExtensionOperationMetrics {
    let m = Box::new(SdkOperationMetrics {
        base: MongoExtensionOperationMetrics {
            vtable: &SDK_METRICS_VTABLE,
        },
        doc: Mutex::new(Document::new()),
    });
    Box::into_raw(m).cast::<MongoExtensionOperationMetrics>()
}
