//! Milestone 2: [`extension_sdk_mongodb::error::ExtensionError`], [`extension_sdk_mongodb::parse_args`],
//! and status mapping at the FFI boundary.

use bson::doc;
use extension_sdk_mongodb::error::{parse_args, ExtensionError};
use extension_sdk_mongodb::status;
use extension_sdk_mongodb::sys::MONGO_EXTENSION_STATUS_OK;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SampleArgs {
    n: u64,
}

#[test]
fn parse_args_deserializes_document() {
    let got: SampleArgs = parse_args(doc! { "n": 42 }).expect("parse_args");
    assert_eq!(got, SampleArgs { n: 42 });
}

#[test]
fn parse_args_wrong_type_is_failed_to_parse() {
    let err = parse_args::<SampleArgs>(doc! { "n": "not-a-number" }).unwrap_err();
    assert!(matches!(err, ExtensionError::FailedToParse(_)));
}

#[test]
fn extension_error_bad_value_display() {
    let e = ExtensionError::BadValue("bad".into());
    assert!(format!("{e}").contains("bad"));
}

#[test]
fn into_raw_status_host_error_preserves_code() {
    let e = ExtensionError::HostError {
        code: 7,
        reason: "from host".into(),
    };
    let p = e.into_raw_status();
    assert!(!p.is_null());
    unsafe {
        let vt = (*p).vtable;
        assert_eq!(((*vt).get_code)(p), 7);
        let reason = ((*vt).get_reason)(p);
        let s = std::str::from_utf8(std::slice::from_raw_parts(reason.data, reason.len as usize))
            .expect("utf8");
        assert!(s.contains("from host"));
        ((*vt).destroy)(p);
    }
}

#[test]
fn into_raw_status_runtime_uses_default_code() {
    let e = ExtensionError::Runtime("boom".into());
    let p = e.into_raw_status();
    assert!(!p.is_null());
    unsafe {
        let vt = (*p).vtable;
        assert_eq!(((*vt).get_code)(p), extension_sdk_mongodb::sys::MONGO_EXTENSION_STATUS_RUNTIME_ERROR);
        ((*vt).destroy)(p);
    }
}

#[test]
fn into_raw_status_bad_value_uses_runtime_code() {
    let e = ExtensionError::BadValue("nope".into());
    let p = e.into_raw_status();
    assert!(!p.is_null());
    unsafe {
        let vt = (*p).vtable;
        assert_eq!(
            ((*vt).get_code)(p),
            extension_sdk_mongodb::sys::MONGO_EXTENSION_STATUS_RUNTIME_ERROR
        );
        let reason = ((*vt).get_reason)(p);
        let s = std::str::from_utf8(std::slice::from_raw_parts(reason.data, reason.len as usize))
            .expect("utf8");
        assert!(s.contains("bad value"));
        ((*vt).destroy)(p);
    }
}

#[test]
fn into_raw_status_failed_to_parse_reason_prefix() {
    let e = ExtensionError::FailedToParse("not bson".into());
    let p = e.into_raw_status();
    assert!(!p.is_null());
    unsafe {
        let vt = (*p).vtable;
        let reason = ((*vt).get_reason)(p);
        let s = std::str::from_utf8(std::slice::from_raw_parts(reason.data, reason.len as usize))
            .expect("utf8");
        assert!(s.contains("failed to parse"));
        ((*vt).destroy)(p);
    }
}

#[test]
fn status_ok_singleton_still_ok() {
    let p = status::status_ok();
    unsafe {
        let vt = (*p).vtable;
        assert_eq!(((*vt).get_code)(p), MONGO_EXTENSION_STATUS_OK);
    }
}
