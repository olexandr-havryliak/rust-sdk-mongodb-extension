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
//! The shared image loads **`$search`**, **`$vectorSearch`**, **`$fibonacci`**, **`$readLocalJsonl`**,
//! and **`$rustSdkE2e`**. Search stages exercise connection errors when OpenSearch is absent.
//! The local stages exercise successful parse and execution.

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

fn random_search_like_inner(rng: &mut StdRng) -> Document {
    let paths = ["name", "description", "category", "nested.text"];
    let queries = ["waterproof", "rain shell", "day pack", "boots", ""];
    let mut inner = doc! {
        "path": paths[rng.gen_range(0..paths.len())],
        "query": queries[rng.gen_range(0..queries.len())],
    };
    if rng.gen_bool(0.45) {
        let limit = match rng.gen_range(0u8..8) {
            0 => 0i64,
            1 => -1i64,
            2 => 1001i64,
            _ => rng.gen_range(1i64..=25),
        };
        inner.insert("limit", limit);
    }
    if rng.gen_bool(0.22) {
        inner.insert("filter", doc! { "term": { "category": "bags" } });
    }
    if rng.gen_bool(0.10) {
        inner.insert(random_ascii(rng, 10), random_bson_value(rng, 1));
    }
    inner
}

const JSONL_FIXTURES: &[&str] = &[
    "sample.ndjson",
    "events.jsonl",
    "nested/stage_params.jsonl",
];

fn random_fibonacci_stage(rng: &mut StdRng) -> Document {
    let inner = if rng.gen_bool(0.8) {
        doc! { "n": rng.gen_range(0i32..=12) }
    } else {
        match rng.gen_range(0u8..3) {
            0 => Document::new(),
            1 => doc! { "n": "x" },
            _ => doc! { "n": -1i32 },
        }
    };
    doc! { "$fibonacci": inner }
}

fn random_jsonl_stage(rng: &mut StdRng) -> Document {
    let mut inner = if rng.gen_bool(0.75) {
        doc! { "path": JSONL_FIXTURES[rng.gen_range(0..JSONL_FIXTURES.len())] }
    } else {
        doc! { "path": random_ascii(rng, 12) }
    };
    if rng.gen_bool(0.35) {
        inner.insert("maxDocuments", rng.gen_range(0i32..=5));
    }
    doc! { "$readLocalJsonl": inner }
}

fn random_e2e_stage(rng: &mut StdRng) -> Document {
    let inner = if rng.gen_bool(0.5) {
        Document::new()
    } else {
        doc! { "probe": rng.gen_range(0i32..4), "tag": random_ascii(rng, 8) }
    };
    doc! { "$rustSdkE2e": inner }
}

fn random_search_stage(rng: &mut StdRng) -> Document {
    doc! { "$search": random_search_like_inner(rng) }
}

fn random_vector_search_stage(rng: &mut StdRng) -> Document {
    doc! { "$vectorSearch": random_search_like_inner(rng) }
}

/// One or more stages: weighted mix of extension stages; optional trailing `$match` / `$project`.
fn build_random_pipeline(rng: &mut StdRng) -> Vec<Document> {
    let first = match rng.gen_range(0u8..5) {
        0 => random_search_stage(rng),
        1 => random_vector_search_stage(rng),
        2 => random_fibonacci_stage(rng),
        3 => random_jsonl_stage(rng),
        _ => random_e2e_stage(rng),
    };
    let mut pipe = vec![first];
    if rng.gen_bool(0.14) {
        pipe.push(doc! { "$match": {} });
    }
    if rng.gen_bool(0.08) {
        pipe.push(doc! { "$limit": rng.gen_range(1i32..=5) });
    }
    if rng.gen_bool(0.05) {
        pipe.push(doc! { "$project": { "_id": 1i32 } });
    }
    pipe
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
        "mongo_extension_fuzz uri={uri} iterations={iterations} seed={seed} stages=$search|$vectorSearch|$fibonacci|$readLocalJsonl|$rustSdkE2e max_time_ms={per_ms}"
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
        let pipeline = build_random_pipeline(&mut rng);
        let r = timeout(wall_timeout, drain_aggregate(coll, pipeline, per_ms)).await;
        match r {
            Err(_) => timeouts += 1,
            Ok(Ok(())) => ok += 1,
            Ok(Err(_)) => err += 1,
        }
    }

    eprintln!("done ok={ok} err={err} timeouts={timeouts} (driver/server errors are expected; rising timeouts may indicate stalls)");
    let code = fuzz_exit_code(ok);
    if code != ExitCode::SUCCESS {
        eprintln!("fuzz produced no successful aggregations");
    }
    code
}

fn fuzz_exit_code(ok: u64) -> ExitCode {
    if ok == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_stage_has_operator_path_and_query() {
        let mut rng = StdRng::seed_from_u64(6);
        let d = random_search_stage(&mut rng);
        let inner = d.get_document("$search").expect("inner");
        assert!(inner.contains_key("path"));
        assert!(inner.contains_key("query"));
    }

    #[test]
    fn vector_search_stage_has_operator_path_and_query() {
        let mut rng = StdRng::seed_from_u64(7);
        let d = random_vector_search_stage(&mut rng);
        let inner = d.get_document("$vectorSearch").expect("inner");
        assert!(inner.contains_key("path"));
        assert!(inner.contains_key("query"));
    }

    #[test]
    fn pipelines_cover_every_loaded_extension() {
        let mut rng = StdRng::seed_from_u64(11);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..400 {
            let pipeline = build_random_pipeline(&mut rng);
            seen.insert(pipeline[0].keys().next().expect("one key").to_string());
        }
        for stage in [
            "$search",
            "$vectorSearch",
            "$fibonacci",
            "$readLocalJsonl",
            "$rustSdkE2e",
        ] {
            assert!(seen.contains(stage), "missing {stage} in {seen:?}");
        }
    }

    #[test]
    fn local_stages_include_executable_arguments() {
        let mut rng = StdRng::seed_from_u64(12);
        let mut fib_ok = false;
        let mut jsonl_ok = false;
        for _ in 0..400 {
            let pipeline = build_random_pipeline(&mut rng);
            let (name, inner) = pipeline[0].iter().next().expect("stage");
            let inner = inner.as_document().expect("inner document");
            if name == "$fibonacci" {
                if let Ok(n) = inner.get_i32("n") {
                    if (0..=12).contains(&n) {
                        fib_ok = true;
                    }
                }
            }
            if name == "$readLocalJsonl" {
                if let Ok(path) = inner.get_str("path") {
                    if matches!(
                        path,
                        "sample.ndjson" | "events.jsonl" | "nested/stage_params.jsonl"
                    ) {
                        jsonl_ok = true;
                    }
                }
            }
        }
        assert!(fib_ok, "no executable $fibonacci stage");
        assert!(jsonl_ok, "no executable $readLocalJsonl stage");
    }

    #[test]
    fn fuzz_run_fails_when_no_iteration_succeeds() {
        assert_eq!(fuzz_exit_code(0), ExitCode::from(1));
        assert_eq!(fuzz_exit_code(1), ExitCode::SUCCESS);
    }

    #[test]
    fn pipeline_first_stage_is_loaded_extension() {
        let mut rng = StdRng::seed_from_u64(5);
        for _ in 0..200 {
            let p = build_random_pipeline(&mut rng);
            let k = p[0].keys().next().expect("one key");
            assert!(
                matches!(
                    k.as_str(),
                    "$search" | "$vectorSearch" | "$fibonacci" | "$readLocalJsonl" | "$rustSdkE2e"
                ),
                "unexpected stage {k}"
            );
        }
    }

    #[test]
    fn pipeline_is_non_empty() {
        let mut rng = StdRng::seed_from_u64(4);
        for _ in 0..50 {
            let p = build_random_pipeline(&mut rng);
            assert!(!p.is_empty(), "pipeline must have at least one stage");
        }
    }
}
