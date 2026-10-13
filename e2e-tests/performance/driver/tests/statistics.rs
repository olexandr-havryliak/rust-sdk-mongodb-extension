use sdk_load_test::Measurements;
use std::time::Duration;

#[test]
fn operation_probe_observes_the_local_router_not_remote_shards() {
    use bson::doc;
    assert_eq!(
        sdk_load_test::operation_pipeline("my-query"),
        vec![
            doc! {"$currentOp": {"allUsers": true, "localOps": true}},
            doc! {"$match": {"command.comment": "my-query"}},
        ]
    );
}

#[test]
fn cursor_count_requires_a_nonnegative_server_metric() {
    use bson::doc;
    use sdk_load_test::open_cursor_count;
    assert_eq!(
        open_cursor_count(&doc! {"metrics": {"cursor": {"open": {"total": 3i64}}}}).unwrap(),
        3
    );
    for status in [
        doc! {},
        doc! {"metrics": {"cursor": {"open": {"total": -1i64}}}},
    ] {
        assert!(open_cursor_count(&status).is_err());
    }
}

#[test]
fn failures_are_not_hidden_in_throughput_or_document_counts() {
    let mut measurements = Measurements::default();
    measurements.record(Duration::from_micros(100), 10, true);
    measurements.record(Duration::from_micros(200), 0, false);
    let summary = measurements.summary(Duration::from_secs(2)).unwrap();
    assert_eq!(
        (summary.successful, summary.failed, summary.documents),
        (1, 1, 10)
    );
    assert_eq!(summary.queries_per_second, 0.5);
    assert_eq!(summary.documents_per_second, 5.0);
    assert_eq!(summary.p99_us, 200);
}

#[test]
fn quantiles_use_nearest_rank_and_include_the_tail() {
    let mut measurements = Measurements::default();
    for us in (1..=100).rev() {
        measurements.record(Duration::from_micros(us), 1, true);
    }
    let summary = measurements.summary(Duration::from_secs(1)).unwrap();
    assert_eq!(
        (summary.p50_us, summary.p95_us, summary.p99_us),
        (50, 95, 99)
    );
}

#[test]
fn empty_or_zero_duration_measurements_cannot_establish_success() {
    assert!(Measurements::default()
        .summary(Duration::from_secs(1))
        .is_err());
    let mut measurements = Measurements::default();
    measurements.record(Duration::from_micros(1), 1, true);
    assert!(measurements.summary(Duration::ZERO).is_err());
}
