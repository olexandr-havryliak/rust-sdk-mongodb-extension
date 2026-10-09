//! CI/local test-only C ABI fixture for the Rust SDK for MongoDB Extensions.

use extension_sdk_mongodb::{byte_buf, status, sys};
use std::ffi::{c_char, CStr};

extension_sdk_mongodb::export_transform_stage!("$abiHarness", true);

include!(concat!(env!("OUT_DIR"), "/layouts.rs"));

/// Look up a Rust layout or constant for comparison by the C compiler.
///
/// # Safety
/// `key` must reference a readable, NUL-terminated string for this call.
#[no_mangle]
pub unsafe extern "C" fn sdk_abi_value(key: *const c_char) -> u64 {
    if key.is_null() {
        return u64::MAX;
    }
    match CStr::from_ptr(key).to_str() {
        Ok(key) => layout_value(key),
        Err(_) => u64::MAX,
    }
}

/// Return a Rust-owned byte buffer; C must destroy it through its vtable.
#[no_mangle]
pub extern "C" fn sdk_abi_buffer() -> *mut sys::MongoExtensionByteBuf {
    byte_buf::into_raw_byte_buf(vec![1, 2, 3, 4])
}

/// Return a Rust-owned status; C must destroy it through its vtable.
#[no_mangle]
pub extern "C" fn sdk_abi_error() -> *mut sys::MongoExtensionStatus {
    status::new_error_status(42, "C ABI fixture error")
}

/// Return the process-lifetime success status.
#[no_mangle]
pub extern "C" fn sdk_abi_ok() -> *mut sys::MongoExtensionStatus {
    status::status_ok()
}

/// Deliberately violate memory safety only in the ASan negative-control process.
///
/// # Safety
/// Call only in an isolated ASan-instrumented process expected to fail.
#[no_mangle]
pub unsafe extern "C" fn sdk_abi_asan_probe() -> u8 {
    let pointer = Box::into_raw(Box::new(7u8));
    drop(Box::from_raw(pointer));
    std::ptr::read_volatile(pointer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_fields_do_not_collide_with_type_size() {
        assert_eq!(
            layout_value("MongoExtensionExpandedArray.size"),
            std::mem::offset_of!(sys::MongoExtensionExpandedArray, size) as u64
        );
        assert_eq!(
            layout_value("MongoExtensionExpandedArray.sizeof"),
            std::mem::size_of::<sys::MongoExtensionExpandedArray>() as u64
        );
    }

    #[test]
    fn layout_table_has_version_and_late_vtable_slots() {
        assert_eq!(layout_value("MONGODB_EXTENSION_API_MAJOR_VERSION"), 1);
        assert_eq!(layout_value("MONGODB_EXTENSION_API_MINOR_VERSION"), 0);
        assert_ne!(
            layout_value("MongoExtensionLogicalAggStageVTable.get_docs_needed_bounds"),
            u64::MAX
        );
        assert_ne!(layout_value("MongoExtensionByteContainer.bytes"), u64::MAX);
        assert_eq!(layout_value("unknown"), u64::MAX);
    }
}
