//! Host / extension API version negotiation (same rules as `ExtensionLoader::assertVersionCompatibility`).

use crate::sys::{MongoExtensionAPIVersion, MongoExtensionAPIVersionVector};

/// Extension API version this SDK targets (must match the vendored `api.h`).
pub const EXTENSION_API_VERSION: MongoExtensionAPIVersion = MongoExtensionAPIVersion {
    major: crate::sys::MONGODB_EXTENSION_API_MAJOR_VERSION,
    minor: crate::sys::MONGODB_EXTENSION_API_MINOR_VERSION,
};

static SUPPORTED_EXTENSION_API_VERSIONS: [MongoExtensionAPIVersion; 1] = [EXTENSION_API_VERSION];

/// Writes the SDK-supported extension API versions for the 1.0 two-phase loader.
///
/// The returned pointer references static storage and remains valid for the process lifetime.
pub unsafe fn write_supported_versions(out: *mut MongoExtensionAPIVersionVector) {
    if out.is_null() {
        return;
    }
    (*out).len = SUPPORTED_EXTENSION_API_VERSIONS.len() as u64;
    (*out).versions = SUPPORTED_EXTENSION_API_VERSIONS.as_ptr().cast_mut();
}

/// Returns true if `selected_version` is one of the API versions this SDK can instantiate.
pub fn supports_selected_version(selected_version: MongoExtensionAPIVersion) -> bool {
    selected_version.major == EXTENSION_API_VERSION.major && selected_version.minor == EXTENSION_API_VERSION.minor
}

/// Returns true if `host_versions` contains a compatible slot for `extension_version`.
pub fn host_supports_extension(
    host_versions: &MongoExtensionAPIVersionVector,
    extension_version: MongoExtensionAPIVersion,
) -> bool {
    if host_versions.len == 0 || host_versions.versions.is_null() {
        return false;
    }
    let slice = unsafe { std::slice::from_raw_parts(host_versions.versions, host_versions.len as usize) };
    let mut found_major = false;
    let mut found_minor = false;
    for host in slice {
        if host.major == extension_version.major {
            found_major = true;
            if host.minor >= extension_version.minor {
                found_minor = true;
                break;
            }
        }
    }
    found_major && found_minor
}
