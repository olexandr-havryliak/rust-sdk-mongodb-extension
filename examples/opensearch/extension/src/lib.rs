use std::ffi::c_void;

use bson::{Bson, Document};
use extension_sdk_mongodb::source_stage::{get_multi_source_extension_impl, SourceOps};
use extension_sdk_mongodb::{ExtensionResult, Next, SourceStage, StageContext};
use opensearch_extension_core::{
    id_lookup_expansion, next_result, open_state, parse_vector_search_args, QueryArgs, SearchState,
};

struct OpenSearchVectorSearch;

impl SourceStage for OpenSearchVectorSearch {
    const NAME: &'static str = "$vectorSearch";
    type Args = QueryArgs;
    type State = SearchState;

    fn parse(args: Document) -> ExtensionResult<Self::Args> {
        parse_vector_search_args(args)
    }

    fn expand(args: &Self::Args) -> extension_sdk_mongodb::Expansion {
        id_lookup_expansion(Self::NAME, args)
    }

    fn properties() -> extension_sdk_mongodb::StageProperties {
        extension_sdk_mongodb::StageProperties {
            requires_input: false,
            ..extension_sdk_mongodb::StageProperties::source_stage_default()
        }
    }

    fn open(args: Self::Args, ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        open_state(args, ctx)
    }

    fn next(state: &mut Self::State, ctx: &mut StageContext) -> ExtensionResult<Next> {
        next_result(state, ctx)
    }
}

fn vector_open(d: Document, ctx: &mut StageContext) -> ExtensionResult<*mut c_void> {
    let args = OpenSearchVectorSearch::parse(d)?;
    let state = OpenSearchVectorSearch::open(args, ctx)?;
    Ok(Box::into_raw(Box::new(state)) as *mut c_void)
}

unsafe fn drop_search_state(ptr: *mut c_void) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr as *mut SearchState));
    }
}

unsafe fn vector_next(ptr: *mut c_void, ctx: &mut StageContext) -> ExtensionResult<Next> {
    OpenSearchVectorSearch::next(&mut *(ptr as *mut SearchState), ctx)
}

fn vector_static_properties() -> Document {
    let mut properties = OpenSearchVectorSearch::properties().to_document();
    properties.insert(
        "providedMetadataFields",
        Bson::Array(vec![Bson::String("vectorSearchScore".to_string())]),
    );
    properties
}

fn vector_expand_inner(d: Document) -> ExtensionResult<extension_sdk_mongodb::Expansion> {
    let args = OpenSearchVectorSearch::parse(d)?;
    Ok(OpenSearchVectorSearch::expand(&args))
}

static VECTOR_SEARCH_OPS: SourceOps = SourceOps {
    name: OpenSearchVectorSearch::NAME,
    expect_empty: false,
    open_from_doc: vector_open,
    drop_state: drop_search_state,
    next: vector_next,
    on_extension_initialized: None,
    static_properties_doc: vector_static_properties,
    expand_inner: vector_expand_inner,
    merging_pipeline: None,
};

#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension_versions(
    extension_versions: *mut extension_sdk_mongodb::sys::MongoExtensionAPIVersionVector,
) {
    extension_sdk_mongodb::version::write_supported_versions(extension_versions)
}

#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension(
    version: extension_sdk_mongodb::sys::MongoExtensionAPIVersion,
    host_services: *const extension_sdk_mongodb::sys::MongoExtensionHostServices,
    extension_out: *mut *const extension_sdk_mongodb::sys::MongoExtension,
) -> *mut extension_sdk_mongodb::sys::MongoExtensionStatus {
    get_multi_source_extension_impl(&[&VECTOR_SEARCH_OPS], version, host_services, extension_out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use std::sync::{Mutex, OnceLock};

    use extension_sdk_mongodb::status;
    use extension_sdk_mongodb::sys::{
        MongoExtensionAggStageDescriptor, MongoExtensionHostPortal, MongoExtensionHostPortalVTable,
        MongoExtensionHostServices, MongoExtensionHostServicesVTable, MongoExtensionStatus,
        MONGO_EXTENSION_STATUS_OK,
    };
    use extension_sdk_mongodb::version::EXTENSION_API_VERSION;
    use extension_sdk_mongodb::{
        GET_MONGODB_EXTENSION_SYMBOL, GET_MONGODB_EXTENSION_VERSIONS_SYMBOL,
    };

    static REGISTERED_NAMES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

    fn registered_names() -> &'static Mutex<Vec<String>> {
        REGISTERED_NAMES.get_or_init(|| Mutex::new(Vec::new()))
    }

    #[test]
    fn exports_standard_extension_symbols() {
        assert!(GET_MONGODB_EXTENSION_SYMBOL.ends_with(b"\0"));
        assert!(GET_MONGODB_EXTENSION_VERSIONS_SYMBOL.ends_with(b"\0"));
    }

    #[test]
    fn registers_vector_stage_name() {
        assert_eq!(VECTOR_SEARCH_OPS.name, "$vectorSearch");
    }

    #[test]
    fn vector_stage_generates_candidates_without_collection_scan() {
        for properties in [vector_static_properties()] {
            assert!(!properties.get_bool("requiresInputDocSource").unwrap());
            assert_eq!(properties.get_str("position").unwrap(), "first");
        }
    }

    #[test]
    fn static_properties_declare_stage_score_metadata() {
        assert_eq!(
            vector_static_properties()
                .get_array("providedMetadataFields")
                .unwrap(),
            &[Bson::String("vectorSearchScore".to_string())]
        );
    }

    #[test]
    fn exported_extension_registers_only_vector_search() {
        registered_names()
            .lock()
            .expect("registered names mutex")
            .clear();
        let portal_vtable = MongoExtensionHostPortalVTable {
            register_stage_descriptor: register_recording_name,
            get_extension_options,
            register_stage_rules,
        };
        let portal = MongoExtensionHostPortal {
            vtable: &portal_vtable,
            host_extensions_api_version: EXTENSION_API_VERSION,
            host_mongodb_max_wire_version: 0,
        };
        let services_vtable = MongoExtensionHostServicesVTable {
            get_logger,
            user_asserted,
            tripwire_asserted,
            mark_idle_thread_block,
            create_host_agg_stage_parse_node,
            create_id_lookup,
        };
        let services = MongoExtensionHostServices {
            vtable: &services_vtable,
        };
        let mut extension = std::ptr::null();
        unsafe {
            let status = get_mongodb_extension(
                EXTENSION_API_VERSION,
                std::ptr::from_ref(&services),
                std::ptr::addr_of_mut!(extension),
            );
            assert_eq!(
                ((*(*status).vtable).get_code)(status),
                MONGO_EXTENSION_STATUS_OK
            );
            ((*(*status).vtable).destroy)(status);
            assert!(!extension.is_null());
            let init_status =
                ((*(*extension).vtable).initialize)(extension, std::ptr::from_ref(&portal));
            assert_eq!(
                ((*(*init_status).vtable).get_code)(init_status),
                MONGO_EXTENSION_STATUS_OK
            );
            ((*(*init_status).vtable).destroy)(init_status);
        }
        assert_eq!(
            *registered_names().lock().expect("registered names mutex"),
            vec!["$vectorSearch".to_string()]
        );
    }

    unsafe extern "C" fn register_recording_name(
        _portal: *const MongoExtensionHostPortal,
        descriptor: *const MongoExtensionAggStageDescriptor,
    ) -> *mut MongoExtensionStatus {
        let name = ((*(*descriptor).vtable).get_name)(descriptor);
        let bytes = std::slice::from_raw_parts(name.data, name.len as usize);
        registered_names()
            .lock()
            .expect("registered names mutex")
            .push(String::from_utf8_lossy(bytes).into_owned());
        status::status_ok()
    }

    unsafe extern "C" fn get_extension_options(
        _portal: *const MongoExtensionHostPortal,
    ) -> extension_sdk_mongodb::sys::MongoExtensionByteView {
        static OPTIONS: &[u8] = b"endpoints: http://opensearch:9200\n";
        extension_sdk_mongodb::sys::MongoExtensionByteView {
            data: OPTIONS.as_ptr(),
            len: OPTIONS.len() as u64,
        }
    }

    unsafe extern "C" fn register_stage_rules(
        _portal: *const MongoExtensionHostPortal,
        _stage_name: extension_sdk_mongodb::sys::MongoExtensionByteView,
        _rules: *const extension_sdk_mongodb::sys::MongoExtensionPipelineRewriteRule,
        _num_rules: usize,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn get_logger() -> *mut extension_sdk_mongodb::sys::MongoExtensionLogger {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn user_asserted(
        _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn tripwire_asserted(
        _msg: extension_sdk_mongodb::sys::MongoExtensionByteView,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn mark_idle_thread_block(
        _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionIdleThreadBlock,
        _name: *const std::ffi::c_char,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn create_host_agg_stage_parse_node(
        _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
        _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageParseNode,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }

    unsafe extern "C" fn create_id_lookup(
        _bson: extension_sdk_mongodb::sys::MongoExtensionByteView,
        _out: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageAstNode,
    ) -> *mut MongoExtensionStatus {
        status::status_ok()
    }
}
