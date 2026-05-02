//! Thin access to [`MongoExtensionHostPortal`] and [`MongoExtensionHostServices`] during init / execution.

use std::sync::{Mutex, OnceLock};

use crate::sys::{MongoExtensionHostPortal, MongoExtensionHostServicesVTable};

static HOST_SERVICES_VTABLE_ADDR: OnceLock<usize> = OnceLock::new();

static EXTENSION_OPTIONS_SNAPSHOT: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Save host services for the extension lifetime (call from `initialize` once).
pub fn set_host_services(services: *const crate::sys::MongoExtensionHostServices) {
    if services.is_null() {
        return;
    }
    let vt = unsafe { (*services).vtable as usize };
    let _ = HOST_SERVICES_VTABLE_ADDR.set(vt);
}

/// Resolved vtable for host services, if `set_host_services` already ran.
pub fn host_services_vtable() -> Option<&'static MongoExtensionHostServicesVTable> {
    HOST_SERVICES_VTABLE_ADDR
        .get()
        .copied()
        .map(|a| unsafe { &*(a as *const MongoExtensionHostServicesVTable) })
}

/// Call `register_stage_descriptor` on the portal.
pub unsafe fn register_stage_descriptor(
    portal: *const MongoExtensionHostPortal,
    descriptor: *const crate::sys::MongoExtensionAggStageDescriptor,
) -> *mut crate::sys::MongoExtensionStatus {
    let vt = (*portal).vtable;
    ((*vt).register_stage_descriptor)(portal, descriptor)
}

/// Read extension YAML options blob (valid only during `initialize`).
pub unsafe fn extension_options_raw(portal: *const MongoExtensionHostPortal) -> crate::sys::MongoExtensionByteView {
    let vt = (*portal).vtable;
    ((*vt).get_extension_options)(portal)
}

/// Copies extension options from the portal into an in-process snapshot for later reads from [`StageContext`](crate::stage_context::StageContext).
///
/// Safe to call once per extension `initialize` after [`set_host_services`](set_host_services).
pub unsafe fn cache_extension_options_from_portal(portal: *const MongoExtensionHostPortal) {
    if portal.is_null() {
        return;
    }
    let v = extension_options_raw(portal);
    let mut slot = EXTENSION_OPTIONS_SNAPSHOT.lock().expect("extension options mutex");
    if v.data.is_null() || v.len == 0 {
        *slot = Some(Vec::new());
        return;
    }
    let slice = std::slice::from_raw_parts(v.data, v.len as usize);
    *slot = Some(slice.to_vec());
}

/// Snapshot of extension options bytes (clone), if [`cache_extension_options_from_portal`](cache_extension_options_from_portal) ran.
pub fn extension_options_snapshot() -> Option<Vec<u8>> {
    EXTENSION_OPTIONS_SNAPSHOT.lock().ok().and_then(|g| g.clone())
}

/// Clears the cached extension options snapshot (for integration tests and harnesses only).
#[doc(hidden)]
pub fn reset_extension_options_snapshot_for_tests() {
    if let Ok(mut g) = EXTENSION_OPTIONS_SNAPSHOT.lock() {
        *g = None;
    }
}
