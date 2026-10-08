//! Thin access to [`MongoExtensionHostPortal`] and [`MongoExtensionHostServices`] during init / execution.

use std::sync::{Mutex, OnceLock};

use bson::Document;

use crate::error::{ExtensionError, Result};
use crate::sys::{
    MongoExtensionAggStageAstNode, MongoExtensionByteView, MongoExtensionHostPortal,
    MongoExtensionHostServicesVTable, MONGO_EXTENSION_STATUS_OK,
};

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

/// Ask the host to create an AST node for a `$_internalSearchIdLookup` stage.
///
/// This is used by search-like extensions that emit `_id`/score candidate rows and then need the
/// MongoDB server to fetch the latest full documents from the owning collection.
pub fn create_id_lookup_ast(spec: &Document) -> Result<*mut MongoExtensionAggStageAstNode> {
    let vt = host_services_vtable()
        .ok_or_else(|| ExtensionError::Runtime("host services not initialized".into()))?;
    let mut raw = Vec::new();
    spec.to_writer(&mut raw)
        .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
    let view = MongoExtensionByteView {
        data: raw.as_ptr(),
        len: raw.len() as u64,
    };
    let mut out: *mut MongoExtensionAggStageAstNode = std::ptr::null_mut();
    let st = unsafe { (vt.create_id_lookup)(view, std::ptr::addr_of_mut!(out)) };
    if st.is_null() {
        return Err(ExtensionError::Runtime(
            "null status from create_id_lookup".into(),
        ));
    }
    let svt = unsafe { (*st).vtable };
    let code = unsafe { ((*svt).get_code)(st) };
    let reason = if code == MONGO_EXTENSION_STATUS_OK {
        None
    } else {
        let view = unsafe { ((*svt).get_reason)(st) };
        let message = if view.data.is_null() || view.len == 0 {
            "create_id_lookup failed".to_string()
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(view.data, view.len as usize) };
            String::from_utf8_lossy(bytes).into_owned()
        };
        Some(message)
    };
    unsafe {
        ((*svt).destroy)(st);
    }
    if let Some(reason) = reason {
        return Err(ExtensionError::HostError { code, reason });
    }
    if out.is_null() {
        return Err(ExtensionError::Runtime(
            "create_id_lookup returned null AST node".into(),
        ));
    }
    Ok(out)
}

pub(crate) fn create_host_parse_node(spec: &Document) -> Result<*mut crate::sys::MongoExtensionAggStageParseNode> {
    let vt = host_services_vtable()
        .ok_or_else(|| ExtensionError::Runtime("host services not initialized".into()))?;
    let raw = bson::to_vec(spec).map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
    let mut out = std::ptr::null_mut();
    unsafe {
        let st = (vt.create_host_agg_stage_parse_node)(MongoExtensionByteView {
            data: raw.as_ptr(), len: raw.len() as u64,
        }, &mut out);
        if st.is_null() {
            return Err(ExtensionError::Runtime("null status from create_host_agg_stage_parse_node".into()));
        }
        let svt = &*(*st).vtable;
        let code = (svt.get_code)(st);
        let reason = (svt.get_reason)(st);
        let message = if reason.data.is_null() || reason.len == 0 {
            "create_host_agg_stage_parse_node failed".into()
        } else {
            String::from_utf8_lossy(std::slice::from_raw_parts(reason.data, reason.len as usize)).into_owned()
        };
        (svt.destroy)(st);
        if code != MONGO_EXTENSION_STATUS_OK {
            return Err(ExtensionError::HostError { code, reason: message });
        }
    }
    if out.is_null() {
        return Err(ExtensionError::Runtime("host returned null parse node".into()));
    }
    Ok(out)
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
