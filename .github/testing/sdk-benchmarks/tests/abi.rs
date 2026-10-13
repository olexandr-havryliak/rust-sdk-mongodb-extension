use bson::doc;
use sdk_benchmarks::{initialize, metrics_roundtrip, RawCursor, SdkCursor};

#[test]
fn metrics_survive_cursor_destruction_and_count_exact_rows() {
    assert_eq!(metrics_roundtrip(3), doc! {"rows": 3i64});
}

#[test]
fn raw_and_sdk_emit_identical_documents_metadata_and_repeated_eof() {
    initialize();
    for metadata in [false, true] {
        let args = doc! {"rows": 3, "payload_bytes": 256, "metadata": metadata};
        let mut sdk = SdkCursor::new(args.clone());
        let mut raw = RawCursor::new(args);
        for id in 0..3i64 {
            let actual = sdk.pull().unwrap().unwrap();
            assert_eq!(actual.0, doc! {"_id": id, "payload": "x".repeat(256)});
            assert_eq!(Some(actual), raw.pull().unwrap());
        }
        for _ in 0..3 {
            assert_eq!(sdk.pull().unwrap(), None);
            assert_eq!(raw.pull().unwrap(), None);
        }
    }
}

#[test]
fn ffi_error_and_panic_paths_are_statuses_and_survive_cleanup() {
    initialize();
    for args in [doc! {"fail_at": 0}, doc! {"panic_at": 0}] {
        let mut cursor = SdkCursor::new(args);
        assert!(cursor.pull().is_err());
        drop(cursor);
    }
    assert!(SdkCursor::new(doc! {"rows": 1}).pull().unwrap().is_some());
}
