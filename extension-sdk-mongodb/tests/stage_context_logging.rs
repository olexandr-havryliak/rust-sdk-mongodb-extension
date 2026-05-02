//! `StageContext` logging forwards to the host logger when configured.

use std::sync::Mutex;

use extension_sdk_mongodb::host;
use extension_sdk_mongodb::stage_context::StageContext;
use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::{
    MongoExtensionHostServices, MongoExtensionHostServicesVTable, MongoExtensionLogMessage,
    MongoExtensionLogger, MongoExtensionLoggerVTable, MongoExtensionLogSeverity, MongoExtensionLogType,
    MongoExtensionStatus,
};

static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "C" fn mock_should_log(
    _: MongoExtensionLogSeverity,
    _: MongoExtensionLogType,
    allow: *mut bool,
) -> *mut MongoExtensionStatus {
    if !allow.is_null() {
        *allow = true;
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn mock_log(msg: *const MongoExtensionLogMessage) -> *mut MongoExtensionStatus {
    let m = &*msg;
    let slice = std::slice::from_raw_parts(m.message.data, m.message.len as usize);
    let s = String::from_utf8_lossy(slice).into_owned();
    CAPTURED.lock().expect("cap").push(s);
    std::ptr::null_mut()
}

static LOGGER_VT: MongoExtensionLoggerVTable = MongoExtensionLoggerVTable {
    log: mock_log,
    should_log: mock_should_log,
};

#[repr(C)]
struct MockLogger {
    base: MongoExtensionLogger,
}

static mut MOCK_LOGGER: MockLogger = MockLogger {
    base: MongoExtensionLogger {
        vtable: &LOGGER_VT,
    },
};

unsafe extern "C" fn mock_get_logger() -> *mut MongoExtensionLogger {
    std::ptr::addr_of_mut!(MOCK_LOGGER).cast::<MongoExtensionLogger>()
}

unsafe extern "C" fn mock_user_asserted(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_tripwire(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_mark_idle(
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionIdleThreadBlock,
    _: *const std::ffi::c_char,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_parse(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageParseNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

unsafe extern "C" fn mock_create_id(
    _: extension_sdk_mongodb::sys::MongoExtensionByteView,
    _: *mut *mut extension_sdk_mongodb::sys::MongoExtensionAggStageAstNode,
) -> *mut MongoExtensionStatus {
    status::status_ok()
}

#[test]
fn stage_context_log_info_forwards_to_host_logger() {
    CAPTURED.lock().expect("cap").clear();
    host::set_host_services(std::ptr::null());

    static SVCS_VT: MongoExtensionHostServicesVTable = MongoExtensionHostServicesVTable {
        get_logger: mock_get_logger,
        user_asserted: mock_user_asserted,
        tripwire_asserted: mock_tripwire,
        mark_idle_thread_block: mock_mark_idle,
        create_host_agg_stage_parse_node: mock_create_parse,
        create_id_lookup: mock_create_id,
    };
    let svcs = MongoExtensionHostServices {
        vtable: &SVCS_VT,
    };
    host::set_host_services(std::ptr::from_ref(&svcs));

    let mut ctx = StageContext::new();
    ctx.log_info("hello-sdk");

    let v = CAPTURED.lock().expect("cap");
    assert_eq!(v.as_slice(), &["hello-sdk".to_string()]);
}
