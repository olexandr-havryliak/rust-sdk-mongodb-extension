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

unsafe extern "C" fn err_set_code(p: *mut MongoExtensionStatus, c: i32) {
    (*p.cast::<OwnedErrorStatus>()).code = c;
}

fn reason_cstring(reason: &str) -> CString {
    CString::new(reason.replace('\0', "\u{FFFD}")).expect("NUL bytes were replaced")
}

/// Returns the status of the update itself (OK), never `p`: the caller destroys the result.
unsafe extern "C" fn err_set_reason(
    p: *mut MongoExtensionStatus,
    r: MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    let text = if r.data.is_null() || r.len == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(std::slice::from_raw_parts(r.data, r.len as usize)).into_owned()
    };
    (*p.cast::<OwnedErrorStatus>()).reason = reason_cstring(&text);
    status_ok()
}

unsafe extern "C" fn err_clone(
    p: *const MongoExtensionStatus,
    out: *mut *mut MongoExtensionStatus,
) -> *mut MongoExtensionStatus {
    let this = &*p.cast::<OwnedErrorStatus>();
    *out = new_error_status(this.code, this.reason.to_string_lossy());
    status_ok()
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
    let reason = reason_cstring(&reason.into());
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
        let b = Box::new(MongoExtensionStatus { vtable: &OK_VTABLE });
        OK_PTR = Box::into_raw(b);
    });
    unsafe { OK_PTR }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(bytes: &[u8]) -> MongoExtensionByteView {
        MongoExtensionByteView {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        }
    }

    unsafe fn code(st: *const MongoExtensionStatus) -> i32 {
        ((*(*st).vtable).get_code)(st)
    }

    unsafe fn reason(st: *const MongoExtensionStatus) -> String {
        let v = ((*(*st).vtable).get_reason)(st);
        if v.data.is_null() {
            return String::new();
        }
        String::from_utf8(std::slice::from_raw_parts(v.data, v.len as usize).to_vec()).unwrap()
    }

    unsafe fn destroy(st: *mut MongoExtensionStatus) {
        ((*(*st).vtable).destroy)(st);
    }

    #[test]
    fn host_can_populate_extension_owned_status() {
        unsafe {
            let st = new_error_status(MONGO_EXTENSION_STATUS_OK, "");
            ((*(*st).vtable).set_code)(st, 11601);
            let ret = ((*(*st).vtable).set_reason)(st, view(b"operation was interrupted"));
            assert!(!ret.is_null());
            assert_ne!(
                ret, st,
                "set_reason must not hand ownership of the target back"
            );
            assert_eq!(code(ret), MONGO_EXTENSION_STATUS_OK);
            destroy(ret);
            assert_eq!(code(st), 11601);
            assert_eq!(reason(st), "operation was interrupted");
            destroy(st);
        }
    }

    #[test]
    fn set_reason_keeps_text_with_interior_nul() {
        unsafe {
            let st = new_error_status(1, "old");
            destroy(((*(*st).vtable).set_reason)(st, view(b"a\0b")));
            assert_eq!(reason(st), "a\u{FFFD}b");
            destroy(st);
        }
    }

    #[test]
    fn clone_produces_independent_copy() {
        unsafe {
            let st = new_error_status(42, "boom");
            let mut copy = std::ptr::null_mut();
            let ret = ((*(*st).vtable).clone)(st, &mut copy);
            assert_eq!(code(ret), MONGO_EXTENSION_STATUS_OK);
            destroy(ret);
            assert!(!copy.is_null());
            destroy(st);
            assert_eq!(code(copy), 42);
            assert_eq!(reason(copy), "boom");
            destroy(copy);
        }
    }
}
