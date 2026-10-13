use bson::doc;
use extension_sdk_mongodb::{Next, SourceStage, StageContext};
use sdk_performance_fixture::{map_row, source_properties, Config, PerfSource};

#[test]
fn source_declares_score_metadata_to_the_planner() {
    assert_eq!(
        source_properties()
            .get_array("providedMetadataFields")
            .unwrap(),
        &[bson::Bson::String("vectorSearchScore".into())]
    );
    assert!(!source_properties()
        .get_bool("requiresInputDocSource")
        .unwrap());
}

#[test]
fn source_emits_exact_sequence_payload_and_metadata_then_repeated_eof() {
    let config = Config::parse(doc! {"rows": 3, "payload_bytes": 256, "metadata": true}).unwrap();
    let mut ctx = StageContext::new();
    let mut state = PerfSource::open(config, &mut ctx).unwrap();
    for id in 0..3i64 {
        assert_eq!(
            PerfSource::next(&mut state, &mut ctx).unwrap(),
            Next::Advanced {
                document: doc! {"_id": id, "payload": "x".repeat(256)},
                metadata: Some(doc! {"$vectorSearchScore": 0.5}),
            }
        );
    }
    for _ in 0..3 {
        assert_eq!(PerfSource::next(&mut state, &mut ctx).unwrap(), Next::Eof);
    }
}

#[test]
fn invalid_or_unbounded_configurations_are_rejected() {
    for args in [
        doc! {"rows": -1},
        doc! {"rows": 1.5},
        doc! {"rows": 1_000_001},
        doc! {"payload_bytes": 1_048_577},
        doc! {"delay_ms": -1},
        doc! {"metadata": 1},
        doc! {"fail_at": -1},
        doc! {"unknown": true},
    ] {
        assert!(Config::parse(args.clone()).is_err(), "{args}");
    }
}

#[test]
fn failure_occurs_at_the_requested_row() {
    let mut ctx = StageContext::new();
    let mut state = PerfSource::open(
        Config::parse(doc! {"rows": 3, "fail_at": 1}).unwrap(),
        &mut ctx,
    )
    .unwrap();
    assert!(matches!(
        PerfSource::next(&mut state, &mut ctx).unwrap(),
        Next::Advanced { .. }
    ));
    assert!(PerfSource::next(&mut state, &mut ctx)
        .unwrap_err()
        .to_string()
        .contains("fixture failure"));
}

#[test]
fn transform_changes_only_the_expected_field() {
    assert_eq!(
        map_row(&doc! {"_id": 7, "payload": "abc"}, &doc! {}).unwrap(),
        doc! {"_id": 7, "payload": "abc", "mapped": true}
    );
    assert!(map_row(&doc! {"_id": 7}, &doc! {"fail_at": 7}).is_err());
}
