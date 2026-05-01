//! Host / extension API version negotiation (same rules as `ExtensionLoader::assertVersionCompatibility`).

use crate::sys::{MongoExtensionAPIVersion, MongoExtensionAPIVersionVector};

/// Extension API version this SDK targets (must match the vendored `api.h`).
pub const EXTENSION_API_VERSION: MongoExtensionAPIVersion = MongoExtensionAPIVersion {
    major: crate::sys::MONGODB_EXTENSION_API_MAJOR_VERSION,
    minor: crate::sys::MONGODB_EXTENSION_API_MINOR_VERSION,
};

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
