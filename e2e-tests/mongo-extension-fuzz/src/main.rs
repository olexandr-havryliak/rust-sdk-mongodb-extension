//! Random aggregation pipelines against a **running** MongoDB with extensions loaded.
//!
//! Environment:
//! - **`MONGODB_URI`** — default `mongodb://127.0.0.1:27017` (use `mongodb://mongo:27017` from the fuzz Compose service).
//! - **`ITERATIONS`** — default `5000`.
//! - **`SEED`** — optional `u64` for reproducibility.
//! - **`PER_ITER_TIMEOUT_MS`** — server-side `maxTimeMS` for the aggregate cursor (default `8000`).
//! - **`FUZZ_DATABASE`** — database name for scratch collections (default `mongo_extension_fuzz`).
//!
//! This is **not** LLVM libFuzzer; it is a bounded random driver to stress server + extension parse/exec paths.
//! The fuzz target is **`$rustSdkE2e`** (matches the e2e Docker image).

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use bson::{doc, Bson, Document};
use futures::stream::TryStreamExt;
use mongodb::Client;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::time::timeout;

const DEFAULT_DB: &str = "mongo_extension_fuzz";

fn random_ascii(rng: &mut StdRng, max_len: usize) -> String {
    let len = rng.gen_range(0..=max_len.max(1));
    (0..len)
        .map(|_| rng.gen_range(b'a'..=b'z') as char)
        .collect()
}

fn random_bson_leaf(rng: &mut StdRng) -> Bson {
    match rng.gen_range(0u8..9) {
        0 => Bson::Null,
        1 => Bson::Boolean(rng.gen()),
        2 => Bson::Int32(rng.gen()),
        3 => Bson::Int64(rng.gen()),
        4 => Bson::Double(rng.gen::<f64>()),
        5 => Bson::String(random_ascii(rng, 48)),
        6 => Bson::RegularExpression(bson::Regex {
            pattern: random_ascii(rng, 8),
            options: "i".into(),
        }),
        7 => Bson::DateTime(bson::DateTime::now()),
        8 => Bson::Timestamp(bson::Timestamp {
            time: rng.gen(),
            increment: rng.gen(),
        }),
        _ => Bson::ObjectId(bson::oid::ObjectId::new()),
    }
}

fn random_bson_value(rng: &mut StdRng, depth: u8) -> Bson {
    if depth == 0 {
        return random_bson_leaf(rng);
    }
    match rng.gen_range(0u8..10) {
        0..=5 => {
            let n = rng.gen_range(0..=10usize);
            let mut d = Document::new();
            for i in 0..n {
                d.insert(format!("k{i}"), random_bson_value(rng, depth - 1));
            }
            Bson::Document(d)
        }
        6..=8 => {
            let n = rng.gen_range(0..=8usize);
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(random_bson_value(rng, depth - 1));
            }
            Bson::Array(v)
        }
        _ => random_bson_leaf(rng),
    }
}

fn random_stage_args_rust_sdk_e2e(rng: &mut StdRng) -> Document {
    random_bson_value(rng, 3)
        .as_document()
        .cloned()
        .unwrap_or_else(|| doc! { "x": 1 })
}

fn pipeline_for_stage(rng: &mut StdRng) -> Vec<Document> {
    let args = random_stage_args_rust_sdk_e2e(rng);
    let mut stage0 = Document::new();
    stage0.insert("$rustSdkE2e", Bson::Document(args));
    vec![stage0]
}

async fn drain_aggregate(
    coll: &mongodb::Collection<Document>,
    pipeline: Vec<Document>,
    max_time_ms: u64,
) -> Result<(), String> {
    let mut cursor = coll
        .aggregate(pipeline)
        .max_time(Duration::from_millis(max_time_ms))
        .await
        .map_err(|e| e.to_string())?;
    while let Some(_doc) = cursor
        .try_next()
        .await
        .map_err(|e| e.to_string())?
    {}
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let uri = env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://127.0.0.1:27017".into());
    let iterations: u64 = env::var("ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000);
    let seed: u64 = env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
    });
    let per_ms: u64 = env::var("PER_ITER_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8_000);

    eprintln!(
        "mongo_extension_fuzz uri={uri} iterations={iterations} seed={seed} stage=$rustSdkE2e max_time_ms={per_ms}"
    );

    let client = match Client::with_uri_str(&uri).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connect failed: {e}");
            return ExitCode::from(1);
        }
    };

    let db = client.database(
        &env::var("FUZZ_DATABASE").unwrap_or_else(|_| DEFAULT_DB.to_string()),
    );
    let _ = db.drop().await;

    let coll_t = db.collection::<Document>("fuzz_t");
    let coll_empty = db.collection::<Document>("fuzz_empty");
    if let Err(e) = db.create_collection("fuzz_t").await {
        eprintln!("create_collection fuzz_t: {e}");
    }
    if let Err(e) = coll_t.insert_one(doc! { "_id": 1i32, "n": 1i32 }).await {
        eprintln!("seed fuzz_t: {e}");
        return ExitCode::from(1);
    }
    if let Err(e) = db.create_collection("fuzz_empty").await {
        eprintln!("create_collection fuzz_empty: {e}");
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let mut ok = 0u64;
    let mut err = 0u64;
    let mut timeouts = 0u64;

    let wall_timeout = Duration::from_millis(per_ms.saturating_add(2500));

    for _ in 0..iterations {
        let coll = if rng.gen_bool(0.28) {
            &coll_empty
        } else {
            &coll_t
        };
        let pipeline = pipeline_for_stage(&mut rng);
        let r = timeout(wall_timeout, drain_aggregate(coll, pipeline, per_ms)).await;
        match r {
            Err(_) => timeouts += 1,
            Ok(Ok(())) => ok += 1,
            Ok(Err(_)) => err += 1,
        }
    }

    eprintln!("done ok={ok} err={err} timeouts={timeouts} (driver/server errors are expected; rising timeouts may indicate stalls)");
    ExitCode::SUCCESS
}
