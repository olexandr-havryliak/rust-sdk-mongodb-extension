//! Performance statistics independent of MongoDB and the load generator.

use serde::Serialize;
use std::time::Duration;

pub fn operation_pipeline(tag: &str) -> Vec<bson::Document> {
    vec![
        bson::doc! {"$currentOp": {"allUsers": true, "localOps": true}},
        bson::doc! {"$match": {"command.comment": tag}},
    ]
}

pub fn open_cursor_count(status: &bson::Document) -> Result<u64, &'static str> {
    let value = status
        .get_document("metrics")
        .ok()
        .and_then(|metrics| metrics.get_document("cursor").ok())
        .and_then(|cursor| cursor.get_document("open").ok())
        .and_then(|open| open.get("total"));
    match value {
        Some(bson::Bson::Int32(value)) if *value >= 0 => Ok(*value as u64),
        Some(bson::Bson::Int64(value)) if *value >= 0 => Ok(*value as u64),
        _ => Err("missing or invalid metrics.cursor.open.total"),
    }
}

#[derive(Default)]
pub struct Measurements {
    latencies_us: Vec<u64>,
    successful: u64,
    failed: u64,
    documents: u64,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub successful: u64,
    pub failed: u64,
    pub documents: u64,
    pub seconds: f64,
    pub queries_per_second: f64,
    pub documents_per_second: f64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

impl Measurements {
    pub fn record(&mut self, elapsed: Duration, documents: u64, passed: bool) {
        self.latencies_us
            .push(elapsed.as_micros().min(u64::MAX as u128) as u64);
        if passed {
            self.successful += 1;
            self.documents += documents;
        } else {
            self.failed += 1;
        }
    }

    pub fn merge(&mut self, mut other: Self) {
        self.successful += other.successful;
        self.failed += other.failed;
        self.documents += other.documents;
        self.latencies_us.append(&mut other.latencies_us);
    }

    pub fn summary(mut self, elapsed: Duration) -> Result<Summary, &'static str> {
        if self.latencies_us.is_empty() || elapsed.is_zero() {
            return Err("empty or zero-duration measurement");
        }
        self.latencies_us.sort_unstable();
        let rank = |percentage: usize| {
            self.latencies_us[(self.latencies_us.len() * percentage).div_ceil(100) - 1]
        };
        Ok(Summary {
            successful: self.successful,
            failed: self.failed,
            documents: self.documents,
            seconds: elapsed.as_secs_f64(),
            queries_per_second: self.successful as f64 / elapsed.as_secs_f64(),
            documents_per_second: self.documents as f64 / elapsed.as_secs_f64(),
            p50_us: rank(50),
            p95_us: rank(95),
            p99_us: rank(99),
            max_us: *self.latencies_us.last().unwrap(),
        })
    }
}
