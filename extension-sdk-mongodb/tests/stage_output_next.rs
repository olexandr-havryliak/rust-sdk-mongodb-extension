//! `Next` / metadata-aware source output (milestone: generator stages with optional metadata).

use bson::doc;
use extension_sdk_mongodb::stage_output::Next;

#[test]
fn next_advanced_carries_optional_metadata() {
    let n = Next::Advanced {
        document: doc! { "x": 1 },
        metadata: Some(doc! { "score": 0.9 }),
    };
    let Next::Advanced { document, metadata } = n else {
        panic!("expected Advanced");
    };
    assert_eq!(document.get_i32("x").ok(), Some(1));
    assert_eq!(metadata.as_ref().and_then(|m| m.get_f64("score").ok()), Some(0.9));
}

#[test]
fn next_advanced_allows_absent_metadata() {
    let n = Next::Advanced {
        document: doc! { "a": "b" },
        metadata: None,
    };
    assert!(matches!(
        n,
        Next::Advanced {
            metadata: None,
            ..
        }
    ));
}

#[test]
fn next_eof_has_no_payload() {
    assert!(matches!(Next::Eof, Next::Eof));
}
