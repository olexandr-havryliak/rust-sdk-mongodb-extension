//! Extension-owned [`MongoExtensionByteBuf`](crate::sys::MongoExtensionByteBuf) helpers.

use std::boxed::Box;

use crate::sys::{
    MongoExtensionByteBuf, MongoExtensionByteBufVTable, MongoExtensionByteView, MongoExtensionStatus,
};

/// Heap object: C-visible `MongoExtensionByteBuf` plus owned bytes.
#[repr(C)]
pub struct OwnedByteBuf {
    /// Public ABI header (must be first).
    pub base: MongoExtensionByteBuf,
    /// Owned BSON or other payload.
    pub data: Vec<u8>,
}

unsafe extern "C" fn byte_buf_destroy(p: *mut MongoExtensionByteBuf) {
    if p.is_null() {
        return;
    }
    let this = p.cast::<OwnedByteBuf>();
    drop(Box::from_raw(this));
}

unsafe extern "C" fn byte_buf_get_view(p: *const MongoExtensionByteBuf) -> MongoExtensionByteView {
    let this = p.cast::<OwnedByteBuf>();
    let s = &(*this).data;
    MongoExtensionByteView {
        data: s.as_ptr(),
        len: s.len() as u64,
    }
}

static BYTE_BUF_VTABLE: MongoExtensionByteBufVTable = MongoExtensionByteBufVTable {
    destroy: byte_buf_destroy,
    get_view: byte_buf_get_view,
};

/// Build a host-owned buffer (caller receives ownership per API docs).
pub fn into_raw_byte_buf(bytes: Vec<u8>) -> *mut MongoExtensionByteBuf {
    let b = Box::new(OwnedByteBuf {
        base: MongoExtensionByteBuf {
            vtable: &BYTE_BUF_VTABLE,
        },
        data: bytes,
    });
    Box::into_raw(b).cast::<OwnedByteBuf>().cast::<MongoExtensionByteBuf>()
}

/// Convenience: BSON document to raw byte buffer for the host.
pub fn from_bson(doc: &bson::Document) -> Result<*mut MongoExtensionByteBuf, bson::ser::Error> {
    let mut v = Vec::new();
    doc.to_writer(&mut v)?;
    Ok(into_raw_byte_buf(v))
}

/// Wrap `Ok(buf_ptr)` or map BSON encode error to a fresh error [`MongoExtensionStatus`](crate::sys::MongoExtensionStatus).
pub fn from_bson_status(
    doc: &bson::Document,
) -> Result<*mut MongoExtensionByteBuf, *mut MongoExtensionStatus> {
    match from_bson(doc) {
        Ok(p) => Ok(p),
        Err(e) => Err(crate::status::new_error_status(-1, format!("{e}"))),
    }
}
