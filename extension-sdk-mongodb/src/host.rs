//! Thin access to [`MongoExtensionHostPortal`] and [`MongoExtensionHostServices`] during init / execution.

use std::sync::OnceLock;

use crate::sys::{MongoExtensionHostPortal, MongoExtensionHostServicesVTable};

static HOST_SERVICES_VTABLE_ADDR: OnceLock<usize> = OnceLock::new();

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
