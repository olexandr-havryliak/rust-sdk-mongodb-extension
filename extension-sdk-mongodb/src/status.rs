//! [`MongoExtensionStatus`](crate::sys::MongoExtensionStatus) helpers.

use std::boxed::Box;
use std::ffi::CString;
use std::sync::Once;

use crate::sys::{
    MongoExtensionByteView, MongoExtensionStatus, MongoExtensionStatusVTable,
    MONGO_EXTENSION_STATUS_OK,
};

#[repr(C)]
struct OwnedErrorStatus {
    base: MongoExtensionStatus,
    code: i32,
    reason: CString,
}

unsafe extern "C" fn err_destroy(p: *mut MongoExtensionStatus) {
    if p.is_null() {
        return;
    }
    drop(Box::from_raw(p.cast::<OwnedErrorStatus>()));
}

unsafe extern "C" fn err_get_code(p: *const MongoExtensionStatus) -> i32 {
    (*p.cast::<OwnedErrorStatus>()).code
}

unsafe extern "C" fn err_get_reason(p: *const MongoExtensionStatus) -> MongoExtensionByteView {
    let s = &(*p.cast::<OwnedErrorStatus>()).reason;
    MongoExtensionByteView {
        data: s.as_ptr().cast(),
        len: s.as_bytes().len() as u64,
    }
}

unsafe extern "C" fn err_set_code(_p: *mut MongoExtensionStatus, _c: i32) {}

unsafe extern "C" fn err_set_reason(
    p: *mut MongoExtensionStatus,
    _r: MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    p
}

unsafe extern "C" fn err_clone(
    _p: *const MongoExtensionStatus,
    _out: *mut *mut MongoExtensionStatus,
) -> *mut MongoExtensionStatus {
    std::ptr::null_mut()
}

static ERR_VTABLE: MongoExtensionStatusVTable = MongoExtensionStatusVTable {
    destroy: err_destroy,
    get_code: err_get_code,
    get_reason: err_get_reason,
    set_code: err_set_code,
    set_reason: err_set_reason,
    clone: err_clone,
};

/// Heap-allocated error status (host will call `destroy`).
pub fn new_error_status(code: i32, reason: impl Into<String>) -> *mut MongoExtensionStatus {
    let reason = CString::new(reason.into()).unwrap_or_else(|_| CString::new("extension error").unwrap());
    let b = Box::new(OwnedErrorStatus {
        base: MongoExtensionStatus {
            vtable: &ERR_VTABLE,
        },
        code,
        reason,
    });
    Box::into_raw(b).cast::<MongoExtensionStatus>()
}

unsafe extern "C" fn ok_destroy(_p: *mut MongoExtensionStatus) {
    // Singleton OK: no-op destroy (matches server `ExtensionStatusOK`).
}

unsafe extern "C" fn ok_get_code(_p: *const MongoExtensionStatus) -> i32 {
    MONGO_EXTENSION_STATUS_OK
}

unsafe extern "C" fn ok_get_reason(_p: *const MongoExtensionStatus) -> MongoExtensionByteView {
    MongoExtensionByteView {
        data: std::ptr::null(),
        len: 0,
    }
}

unsafe extern "C" fn ok_set_code(_p: *mut MongoExtensionStatus, _c: i32) {}

unsafe extern "C" fn ok_set_reason(
    p: *mut MongoExtensionStatus,
    _r: MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    p
}

unsafe extern "C" fn ok_clone(
    _p: *const MongoExtensionStatus,
    _out: *mut *mut MongoExtensionStatus,
) -> *mut MongoExtensionStatus {
    std::ptr::null_mut()
}

static OK_VTABLE: MongoExtensionStatusVTable = MongoExtensionStatusVTable {
    destroy: ok_destroy,
    get_code: ok_get_code,
    get_reason: ok_get_reason,
    set_code: ok_set_code,
    set_reason: ok_set_reason,
    clone: ok_clone,
};

static OK_INIT: Once = Once::new();
static mut OK_PTR: *mut MongoExtensionStatus = std::ptr::null_mut();

/// Pointer to the process-wide OK status object (`destroy` is a no-op).
pub fn status_ok() -> *mut MongoExtensionStatus {
    OK_INIT.call_once(|| unsafe {
        let b = Box::new(MongoExtensionStatus {
            vtable: &OK_VTABLE,
        });
        OK_PTR = Box::into_raw(b);
    });
    unsafe { OK_PTR }
}
