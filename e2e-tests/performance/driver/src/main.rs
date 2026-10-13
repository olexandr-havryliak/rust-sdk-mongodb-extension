use std::{
    env, fs,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

use bson::{doc, Document};
use futures::TryStreamExt;
use mongodb::{error::ErrorKind, Client, Collection};
use sdk_load_test::{open_cursor_count, operation_pipeline, Measurements, Summary};
use serde::Serialize;
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinSet,
};

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Serialize)]
struct Case {
    name: String,
    passed: bool,
    detail: String,
    summary: Option<Summary>,
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    topology: String,
    rust_profile: &'static str,
    target_rps: u64,
    cases: Vec<Case>,
    server_before: Document,
    server_after: Document,
}

#[derive(Clone, Copy)]
enum Workload {
    Source,
    Metadata,
    Transform,
    Passthrough,
    Native,
}

#[derive(Clone)]
struct Request {
    kind: Workload,
    rows: i64,
    bytes: usize,
    batch: u32,
}

impl Request {
    fn pipeline(&self) -> Vec<Document> {
        match self.kind {
            Workload::Source => vec![
                doc! {"$sdkPerfSource": {"rows": self.rows, "payload_bytes": self.bytes as i64}},
            ],
            Workload::Metadata => vec![
                doc! {"$sdkPerfSource": {"rows": self.rows, "payload_bytes": self.bytes as i64, "metadata": true}},
                doc! {"$set": {"score": {"$meta": "vectorSearchScore"}}},
            ],
            Workload::Transform | Workload::Passthrough => vec![
                doc! {"$sort": {"_id": 1}},
                doc! {"$sdkPerfTransform": {"passthrough": matches!(self.kind, Workload::Passthrough)}},
            ],
            Workload::Native => vec![doc! {"$sort": {"_id": 1}}, doc! {"$set": {"mapped": true}}],
        }
    }

    async fn execute(&self, collection: &Collection<Document>) -> TestResult<u64> {
        let mut cursor = collection
            .aggregate(self.pipeline())
            .batch_size(self.batch)
            .max_time(Duration::from_secs(15))
            .comment("sdk-performance-load")
            .await?;
        let mut count = 0;
        while let Some(actual) = cursor.try_next().await? {
            let mut expected = doc! {"_id": count as i64, "payload": "x".repeat(self.bytes)};
            match self.kind {
                Workload::Transform | Workload::Native => {
                    expected.insert("mapped", true);
                }
                Workload::Metadata => {
                    expected.insert("score", 0.5);
                }
                _ => {}
            }
            if actual != expected {
                return Err(format!("unexpected document at row {count}: {actual}").into());
            }
            count += 1;
        }
        if count != self.rows as u64 {
            return Err(format!("expected {} rows, received {count}", self.rows).into());
        }
        Ok(count)
    }
}

fn integer(name: &str, default: u64, min: u64, max: u64) -> TestResult<u64> {
    let raw = env::var(name).unwrap_or_else(|_| default.to_string());
    let value: u64 = raw
        .parse()
        .map_err(|_| format!("{name} must be an integer"))?;
    if !(min..=max).contains(&value) {
        return Err(format!("{name} must be {min}..={max}").into());
    }
    Ok(value)
}

async fn phase(
    collection: Collection<Document>,
    request: Request,
    concurrency: usize,
    total: usize,
    rate: u64,
) -> TestResult<Summary> {
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel::<Instant>(concurrency * 2);
    let receiver = std::sync::Arc::new(Mutex::new(receiver));
    let mut workers = JoinSet::new();
    for _ in 0..concurrency {
        let (collection, request, receiver) =
            (collection.clone(), request.clone(), receiver.clone());
        workers.spawn(async move {
            let mut measurements = Measurements::default();
            loop {
                let planned = receiver.lock().await.recv().await;
                let Some(planned) = planned else {
                    break;
                };
                let result = request.execute(&collection).await;
                if let Err(error) = &result {
                    eprintln!("request failed: {error}");
                }
                measurements.record(
                    planned.elapsed(),
                    result.as_ref().copied().unwrap_or(0),
                    result.is_ok(),
                );
            }
            measurements
        });
    }
    for index in 0..total {
        let planned = if rate == 0 {
            Instant::now()
        } else {
            started + Duration::from_secs_f64(index as f64 / rate as f64)
        };
        if rate > 0 {
            tokio::time::sleep_until(planned.into()).await;
        }
        sender.send(planned).await?;
    }
    drop(sender);
    let mut measurements = Measurements::default();
    while let Some(result) = workers.join_next().await {
        measurements.merge(result?);
    }
    Ok(measurements.summary(started.elapsed())?)
}

fn command_code(error: &mongodb::error::Error) -> Option<i32> {
    match error.kind.as_ref() {
        ErrorKind::Command(command) => Some(command.code),
        _ => None,
    }
}

async fn active_operations(client: &Client, tag: &str) -> TestResult<Vec<Document>> {
    Ok(client
        .database("admin")
        .aggregate(operation_pipeline(tag))
        .await?
        .try_collect()
        .await?)
}

async fn wait_idle(client: &Client, tag: &str) -> TestResult<()> {
    for _ in 0..100 {
        if active_operations(client, tag).await?.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(format!("operations tagged {tag} did not recover").into())
}

async fn cursor_count(client: &Client) -> TestResult<u64> {
    let status = client
        .database("admin")
        .run_command(doc! {"serverStatus": 1})
        .await?;
    Ok(open_cursor_count(&status)?)
}

async fn wait_cursors(client: &Client, baseline: u64) -> TestResult<()> {
    for _ in 0..100 {
        if cursor_count(client).await? <= baseline {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err("open server cursors did not return to baseline".into())
}

async fn controls(client: &Client, collection: &Collection<Document>, cases: &mut Vec<Case>) {
    async fn check<T>(name: &str, result: TestResult<T>, cases: &mut Vec<Case>) {
        cases.push(Case {
            name: name.into(),
            passed: result.is_ok(),
            detail: result
                .err()
                .map_or_else(|| "verified".into(), |error| error.to_string()),
            summary: None,
        });
    }
    let early_close: TestResult<()> = async {
        let baseline = cursor_count(client).await?;
        for _ in 0..20 {
            let mut cursor = collection
                .aggregate(vec![
                    doc! {"$sdkPerfSource": {"rows": 100_000, "metadata": true}},
                ])
                .batch_size(2)
                .comment("sdk-performance-close")
                .await?;
            if cursor.try_next().await?.is_none() {
                return Err("early close returned no document".into());
            }
            drop(cursor);
        }
        wait_idle(client, "sdk-performance-close").await?;
        wait_cursors(client, baseline).await
    }
    .await;
    check("early cursor close", early_close, cases).await;

    let error_paths: TestResult<()> = async {
        for _ in 0..10 {
            for (field, message) in [("fail_at", "fixture failure"), ("panic_at", "panic")] {
                let mut args = doc! {"rows": 10, "metadata": true};
                args.insert(field, 0);
                let result = collection
                    .aggregate(vec![doc! {"$sdkPerfSource": args}])
                    .await;
                match result {
                    Err(error) if error.to_string().contains(message) => {}
                    other => return Err(format!("expected {message}, received {other:?}").into()),
                }
            }
            for (field, message) in [("fail_at", "fixture failure"), ("panic_at", "panic")] {
                let mut args = doc! {};
                args.insert(field, 0);
                let result = collection
                    .aggregate(vec![
                        doc! {"$sort": {"_id": 1}},
                        doc! {"$sdkPerfTransform": args},
                    ])
                    .await;
                match result {
                    Err(error) if error.to_string().contains(message) => {}
                    other => {
                        return Err(
                            format!("expected transform {message}, received {other:?}").into()
                        )
                    }
                }
            }
        }
        Request {
            kind: Workload::Source,
            rows: 1,
            bytes: 256,
            batch: 1,
        }
        .execute(collection)
        .await?;
        Ok(())
    }
    .await;
    check("error panic cleanup and recovery", error_paths, cases).await;

    let deadline: TestResult<()> = async {
        let started = Instant::now();
        let result = collection
            .aggregate(vec![doc! {"$sdkPerfSource": {"rows": 1, "delay_ms": 1000}}])
            .max_time(Duration::from_millis(100))
            .comment("sdk-performance-deadline")
            .await;
        match result {
            Err(error)
                if command_code(&error) == Some(50)
                    && started.elapsed() < Duration::from_secs(3) => {}
            other => {
                return Err(format!("expected bounded MaxTimeMSExpired, received {other:?}").into())
            }
        }
        wait_idle(client, "sdk-performance-deadline").await
    }
    .await;
    check("maxTimeMS code and recovery", deadline, cases).await;

    let kill: TestResult<()> = async {
        let collection = collection.clone();
        let task = tokio::spawn(async move {
            collection
                .aggregate(vec![doc! {"$sdkPerfSource": {"rows": 1, "delay_ms": 5000}}])
                .max_time(Duration::from_secs(10))
                .comment("sdk-performance-kill")
                .await
        });
        let mut op = None;
        for _ in 0..100 {
            op = active_operations(client, "sdk-performance-kill")
                .await?
                .into_iter()
                .find_map(|document| document.get("opid").cloned());
            if op.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let Some(op) = op else {
            task.abort();
            return Err("could not observe operation for killOp".into());
        };
        let started = Instant::now();
        client
            .database("admin")
            .run_command(doc! {"killOp": 1, "op": op})
            .await?;
        let result = tokio::time::timeout(Duration::from_secs(3), task).await??;
        match result {
            Err(error)
                if command_code(&error) == Some(11601)
                    && started.elapsed() < Duration::from_secs(3) => {}
            other => return Err(format!("expected Interrupted, received {other:?}").into()),
        }
        wait_idle(client, "sdk-performance-kill").await
    }
    .await;
    check("killOp code and recovery", kill, cases).await;

    let abandon: TestResult<()> = async {
        let pending = collection
            .aggregate(vec![doc! {"$sdkPerfSource": {"rows": 1, "delay_ms": 1000}}])
            .max_time(Duration::from_millis(300))
            .comment("sdk-performance-abandon");
        if tokio::time::timeout(Duration::from_millis(50), pending)
            .await
            .is_ok()
        {
            return Err("abandoned request unexpectedly completed before client timeout".into());
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        wait_idle(client, "sdk-performance-abandon").await
    }
    .await;
    check(
        "client future cancellation and bounded server recovery",
        abandon,
        cases,
    )
    .await;
}

async fn run() -> TestResult<bool> {
    let uri = env::var("MONGODB_URI")?;
    let topology = env::var("LOAD_TOPOLOGY").unwrap_or_else(|_| "replica-set".into());
    let repetitions = integer("LOAD_REQUESTS", 8, 1, 1000)? as usize;
    let warmup = integer("LOAD_WARMUP", 2, 1, 100)?;
    let rate = integer("LOAD_TARGET_RPS", 0, 0, 10_000)?;
    let soak_seconds = integer("LOAD_SOAK_SECONDS", 0, 0, 7200)?;
    let concurrency: Vec<usize> = env::var("LOAD_CONCURRENCY")
        .unwrap_or_else(|_| "1,4,16,64".into())
        .split(',')
        .map(|value| value.parse::<usize>())
        .collect::<Result<_, _>>()?;
    if concurrency.is_empty()
        || concurrency.len() > 8
        || concurrency.iter().any(|value| !(1..=128).contains(value))
    {
        return Err("LOAD_CONCURRENCY needs 1-8 values between 1 and 128".into());
    }
    let client = Client::with_uri_str(uri).await?;
    let database = client.database("rust_sdk_performance");
    let collection = database.collection::<Document>("input");
    collection.delete_many(doc! {}).await?;
    collection
        .insert_many((0..128i64).map(|id| doc! {"_id": id, "payload": "x".repeat(256)}))
        .await?;
    let server_before = client
        .database("admin")
        .run_command(doc! {"serverStatus": 1})
        .await?;
    let mut cases = Vec::new();
    for (name, kind, rows, bytes) in [
        ("source-short", Workload::Source, 1, 256),
        ("source-cursor", Workload::Source, 128, 4096),
        ("source-large", Workload::Source, 32, 65536),
        ("source-1MiB", Workload::Source, 1, 1048576),
        ("metadata", Workload::Metadata, 128, 256),
        ("transform", Workload::Transform, 128, 256),
        ("passthrough-map", Workload::Passthrough, 128, 256),
        ("native-map-reference", Workload::Native, 128, 256),
    ] {
        for &workers in &concurrency {
            let request = Request {
                kind,
                rows,
                bytes,
                batch: 32,
            };
            for _ in 0..warmup {
                request.execute(&collection).await?;
            }
            let summary = phase(
                collection.clone(),
                request,
                workers,
                workers * repetitions,
                rate,
            )
            .await?;
            cases.push(Case {
                name: format!("{name}/c{workers}"),
                passed: summary.failed == 0,
                detail: "latency includes admission/queue delay; every row and EOF checked".into(),
                summary: Some(summary),
            });
        }
    }
    controls(&client, &collection, &mut cases).await;
    if soak_seconds > 0 {
        let deadline = Instant::now() + Duration::from_secs(soak_seconds);
        let request = Request {
            kind: Workload::Metadata,
            rows: 128,
            bytes: 256,
            batch: 2,
        };
        let mut round = 0;
        while Instant::now() < deadline {
            let summary = phase(collection.clone(), request.clone(), 4, 40, rate).await?;
            cases.push(Case {
                name: format!("soak/{round}"),
                passed: summary.failed == 0,
                detail: "bounded round; repeated cursor and metadata lifecycle".into(),
                summary: Some(summary),
            });
            round += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let server_after = client
        .database("admin")
        .run_command(doc! {"serverStatus": 1})
        .await?;
    let report = Report {
        schema_version: 1,
        topology,
        rust_profile: "release",
        target_rps: rate,
        cases,
        server_before,
        server_after,
    };
    let passed = report.cases.iter().all(|case| case.passed);
    let directory = PathBuf::from(env::var("REPORT_DIR").unwrap_or_else(|_| "/reports".into()));
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("load.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    client.shutdown().await;
    Ok(passed)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    match run().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("load setup/execution failed: {error}");
            ExitCode::FAILURE
        }
    }
}
