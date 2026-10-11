/// # Safety
/// The host must provide a writable API version vector.
#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension_versions(
    out: *mut extension_sdk_mongodb::sys::MongoExtensionAPIVersionVector,
) {
    opensearch_extension_core::mongodb::get_mongodb_extension_versions(out)
}

/// # Safety
/// Host service pointers must remain valid for the library lifetime.
/// `out` must point to writable storage owned by the MongoDB host.
#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension(
    version: extension_sdk_mongodb::sys::MongoExtensionAPIVersion,
    services: *const extension_sdk_mongodb::sys::MongoExtensionHostServices,
    out: *mut *const extension_sdk_mongodb::sys::MongoExtension,
) -> *mut extension_sdk_mongodb::sys::MongoExtensionStatus {
    opensearch_extension_core::mongodb::get_mongodb_extension(version, services, out)
}
