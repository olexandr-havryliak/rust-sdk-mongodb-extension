use std::collections::VecDeque;
use std::time::Duration;

use bson::{doc, Bson, Document};
use extension_sdk_mongodb::{ExtensionError, ExtensionResult, Next, StageContext};
use serde_json::{json, Value};

pub mod mongodb;
pub mod mongos;
pub mod transport;

const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 1000;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MAX_FILTER_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSearchConfig {
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryArgs {
    pub path: String,
    pub query: String,
    pub limit: i64,
    pub filter: Option<Document>,
}

#[derive(Debug)]
pub struct SearchState {
    args: QueryArgs,
    index: String,
    config: OpenSearchConfig,
    timeout: Duration,
    pending: VecDeque<Document>,
    loaded: bool,
}

fn bson_string(args: &Document, key: &str) -> ExtensionResult<String> {
    let value = args
        .get_str(key)
        .map_err(|_| ExtensionError::BadValue(format!("{key} must be a string")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ExtensionError::BadValue(format!("{key} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn bson_i64(args: &Document, key: &str, default: i64, min: i64, max: i64) -> ExtensionResult<i64> {
    let Some(value) = args.get(key) else {
        return Ok(default);
    };
    let parsed = match value {
        Bson::Int32(v) => *v as i64,
        Bson::Int64(v) => *v,
        Bson::Double(v) if v.is_finite() => *v as i64,
        _ => {
            return Err(ExtensionError::BadValue(format!(
                "{key} must be a finite number"
            )))
        }
    };
    if parsed < min || parsed > max {
        return Err(ExtensionError::BadValue(format!(
            "{key} must be between {min} and {max}"
        )));
    }
    Ok(parsed)
}

pub fn parse_vector_search_args(args: Document) -> ExtensionResult<QueryArgs> {
    let filter = match args.get("filter") {
        None => None,
        Some(Bson::Document(filter)) => {
            let mut clauses = 0;
            validate_filter_clause(filter, 1, &mut clauses)?;
            Some(filter.clone())
        }
        Some(_) => return Err(bad_filter("filter must be a document")),
    };
    Ok(QueryArgs {
        path: bson_string(&args, "path")?,
        query: bson_string(&args, "query")?,
        limit: bson_i64(&args, "limit", DEFAULT_LIMIT, 1, MAX_LIMIT)?,
        filter,
    })
}

const MAX_FILTER_CLAUSES: usize = 256;
const MAX_FILTER_TERMS: usize = 1024;

fn bad_filter(message: impl std::fmt::Display) -> ExtensionError {
    ExtensionError::BadValue(format!("filter: {message}"))
}

/// The filter is forwarded to OpenSearch as query DSL, so only a closed set of cheap,
/// single-index clauses is accepted (no scripts, lookups, or pattern queries).
fn validate_filter_clause(
    clause: &Document,
    depth: usize,
    clauses: &mut usize,
) -> ExtensionResult<()> {
    *clauses += 1;
    if depth > MAX_FILTER_DEPTH || *clauses > MAX_FILTER_CLAUSES {
        return Err(bad_filter("too deeply nested or too many clauses"));
    }
    let (kind, body) = single_entry(clause, "each clause must have exactly one query type")?;
    let body = body
        .as_document()
        .ok_or_else(|| bad_filter(format!("{kind} must be a document")))?;
    match kind.as_str() {
        "bool" => validate_bool(body, depth, clauses),
        "term" => {
            let value = single_field(body, "term")?;
            match value {
                Bson::Document(d) => {
                    let (key, inner) = single_entry(d, "term accepts only {value: <scalar>}")?;
                    if key != "value" {
                        return Err(bad_filter("term accepts only {value: <scalar>}"));
                    }
                    require_scalar(inner, "term")
                }
                other => require_scalar(other, "term"),
            }
        }
        "terms" => {
            let values = single_field(body, "terms")?
                .as_array()
                .ok_or_else(|| bad_filter("terms must map one field to an array of values"))?;
            if values.len() > MAX_FILTER_TERMS {
                return Err(bad_filter("terms has too many values"));
            }
            values.iter().try_for_each(|v| require_scalar(v, "terms"))
        }
        "range" => {
            let bounds = single_field(body, "range")?
                .as_document()
                .ok_or_else(|| bad_filter("range must map one field to bounds"))?;
            for (op, value) in bounds {
                match op.as_str() {
                    "gt" | "gte" | "lt" | "lte" => require_scalar(value, "range")?,
                    "format" if matches!(value, Bson::String(_)) => {}
                    _ => return Err(bad_filter(format!("unsupported range option {op}"))),
                }
            }
            Ok(())
        }
        "exists" => {
            let (key, field) = single_entry(body, "exists accepts only {field: <name>}")?;
            match (key.as_str(), field) {
                ("field", Bson::String(name)) => validate_field_name(name),
                _ => Err(bad_filter("exists accepts only {field: <name>}")),
            }
        }
        "ids" => {
            let (key, values) = single_entry(body, "ids accepts only {values: [<string>]}")?;
            let values = values
                .as_array()
                .filter(|_| key == "values")
                .ok_or_else(|| bad_filter("ids accepts only {values: [<string>]}"))?;
            if values.len() > MAX_FILTER_TERMS || !values.iter().all(|v| v.as_str().is_some()) {
                return Err(bad_filter("ids accepts only {values: [<string>]}"));
            }
            Ok(())
        }
        other => Err(bad_filter(format!("unsupported query type {other}"))),
    }
}

fn validate_bool(body: &Document, depth: usize, clauses: &mut usize) -> ExtensionResult<()> {
    for (occur, value) in body {
        match occur.as_str() {
            "must" | "filter" | "should" | "must_not" => match value {
                Bson::Document(d) => validate_filter_clause(d, depth + 1, clauses)?,
                Bson::Array(items) => {
                    for item in items {
                        let d = item.as_document().ok_or_else(|| {
                            bad_filter(format!("bool.{occur} items must be clauses"))
                        })?;
                        validate_filter_clause(d, depth + 1, clauses)?;
                    }
                }
                _ => {
                    return Err(bad_filter(format!(
                        "bool.{occur} must be a clause or array"
                    )))
                }
            },
            "minimum_should_match"
                if matches!(value, Bson::Int32(_) | Bson::Int64(_) | Bson::String(_)) => {}
            _ => return Err(bad_filter(format!("unsupported bool option {occur}"))),
        }
    }
    Ok(())
}

fn single_entry<'a>(d: &'a Document, message: &str) -> ExtensionResult<(&'a String, &'a Bson)> {
    let mut entries = d.iter();
    match (entries.next(), entries.next()) {
        (Some(entry), None) => Ok(entry),
        _ => Err(bad_filter(message)),
    }
}

fn single_field<'a>(body: &'a Document, kind: &str) -> ExtensionResult<&'a Bson> {
    let (field, value) = single_entry(body, &format!("{kind} must target exactly one field"))?;
    validate_field_name(field)?;
    Ok(value)
}

fn validate_field_name(name: &str) -> ExtensionResult<()> {
    if name.is_empty() || name.starts_with('_') {
        return Err(bad_filter(format!("field {name:?} is not allowed")));
    }
    Ok(())
}

fn require_scalar(value: &Bson, kind: &str) -> ExtensionResult<()> {
    match value {
        Bson::String(_) | Bson::Boolean(_) | Bson::Int32(_) | Bson::Int64(_) => Ok(()),
        Bson::Double(v) if v.is_finite() => Ok(()),
        _ => Err(bad_filter(format!(
            "{kind} values must be strings, numbers or booleans"
        ))),
    }
}

pub fn parse_config(raw: Option<Vec<u8>>) -> ExtensionResult<OpenSearchConfig> {
    let Some(raw) = raw else {
        return Err(ExtensionError::BadValue(
            "OpenSearch extension config must define endpoints".into(),
        ));
    };
    let text = String::from_utf8(raw)
        .map_err(|e| ExtensionError::FailedToParse(format!("extension config utf8: {e}")))?;
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|e| ExtensionError::FailedToParse(format!("extension config json: {e}")))?;
        let endpoints = value
            .get("endpoints")
            .and_then(Value::as_array)
            .ok_or_else(|| ExtensionError::BadValue("endpoints must be an array".into()))?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        ExtensionError::BadValue("endpoints must contain strings".into())
                    })
            })
            .collect::<ExtensionResult<Vec<_>>>()?;
        return config_from_endpoints(endpoints);
    }

    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("endpoints:") {
            let endpoints = rest
                .split(',')
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return config_from_endpoints(endpoints);
        }
    }
    Err(ExtensionError::BadValue(
        "OpenSearch extension config must define endpoints".into(),
    ))
}

fn config_from_endpoints(endpoints: Vec<String>) -> ExtensionResult<OpenSearchConfig> {
    if endpoints.is_empty() {
        return Err(ExtensionError::BadValue(
            "endpoints must contain at least one OpenSearch URL".into(),
        ));
    }
    for endpoint in &endpoints {
        let lower = endpoint.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(ExtensionError::BadValue(format!(
                "OpenSearch endpoint must start with http:// or https://: {endpoint}"
            )));
        }
    }
    Ok(OpenSearchConfig { endpoints })
}

pub fn id_lookup_expansion(stage_name: &str, args: &QueryArgs) -> extension_sdk_mongodb::Expansion {
    let mut inner = doc! {
        "path": &args.path,
        "query": &args.query,
        "limit": args.limit,
    };
    if let Some(filter) = &args.filter {
        inner.insert("filter", filter.clone());
    }
    extension_sdk_mongodb::Expansion::WithHostIdLookup {
        extension_stage: doc! { stage_name: inner },
        id_lookup: doc! { "$_internalSearchIdLookup": { "limit": args.limit } },
    }
}

pub fn opensearch_body(args: &QueryArgs) -> Value {
    let mut neural = json!({ "query_text": args.query, "k": args.limit });
    if let Some(filter) = &args.filter {
        neural["filter"] = bson_doc_to_json(filter);
    }
    json!({
        "size": args.limit,
        "_source": false,
        "query": { "neural": { format!("{}_embedding", args.path): neural } }
    })
}

fn bson_doc_to_json(doc: &Document) -> Value {
    serde_json::to_value(Bson::Document(doc.clone())).unwrap_or(Value::Null)
}

/// HTTP timeout for one OpenSearch attempt: `max`, shortened to the time left before the query
/// deadline (`i64::MAX` means no deadline).
pub fn request_timeout(
    deadline_ms: Option<i64>,
    now_ms: i64,
    max: Duration,
) -> ExtensionResult<Duration> {
    let Some(deadline) = deadline_ms.filter(|d| *d != i64::MAX) else {
        return Ok(max);
    };
    let remaining = deadline.saturating_sub(now_ms);
    if remaining <= 0 {
        return Err(ExtensionError::Runtime(
            "query deadline exceeded before OpenSearch request".into(),
        ));
    }
    Ok(max.min(Duration::from_millis(remaining as u64)))
}

/// Checks for interrupts and returns the timeout for the next OpenSearch attempt.
pub(crate) fn next_attempt_timeout(
    ctx: &mut StageContext,
    max: Duration,
) -> ExtensionResult<Duration> {
    ctx.check_interrupt()?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    request_timeout(ctx.deadline_timestamp_ms()?, now_ms, max)
}

pub(crate) fn default_timeout() -> Duration {
    Duration::from_millis(DEFAULT_TIMEOUT_MS)
}

pub fn open_state(args: QueryArgs, ctx: &mut StageContext) -> ExtensionResult<SearchState> {
    let catalog = ctx.catalog().ok_or_else(|| {
        ExtensionError::BadValue(
            "OpenSearch search stages require collection catalog context".into(),
        )
    })?;
    let config = parse_config(ctx.extension_options_raw())?;
    Ok(SearchState {
        args,
        index: format!("mongodb.{}", catalog.namespace()),
        config,
        timeout: default_timeout(),
        pending: VecDeque::new(),
        loaded: false,
    })
}

pub fn next_result(state: &mut SearchState, ctx: &mut StageContext) -> ExtensionResult<Next> {
    ctx.check_interrupt()?;
    if !state.loaded {
        state.pending = VecDeque::from(fetch_candidates(state, ctx)?);
        state.loaded = true;
    }
    match state.pending.pop_front() {
        Some(mut document) => {
            let metadata = document
                .remove("$vectorSearchScore")
                .map(|score| doc! { "$vectorSearchScore": score });
            Ok(Next::Advanced { document, metadata })
        }
        None => Ok(Next::Eof),
    }
}

fn fetch_candidates(state: &SearchState, ctx: &mut StageContext) -> ExtensionResult<Vec<Document>> {
    let body = opensearch_body(&state.args);
    let score_key = "$vectorSearchScore";
    let mut last_error = None;
    for endpoint in &state.config.endpoints {
        let timeout = next_attempt_timeout(ctx, state.timeout)?;
        match fetch_candidates_from_endpoint(
            endpoint,
            &state.index,
            &body,
            score_key,
            state.args.limit as usize,
            timeout,
        ) {
            Ok(documents) => return Ok(documents),
            Err(err) => last_error = Some(err),
        }
    }
    Err(ExtensionError::Runtime(format!(
        "all OpenSearch endpoints failed: {}",
        last_error.unwrap_or_else(|| "no endpoint attempted".to_string())
    )))
}

fn fetch_candidates_from_endpoint(
    endpoint: &str,
    index: &str,
    body: &Value,
    score_key: &str,
    limit: usize,
    timeout: Duration,
) -> Result<Vec<Document>, String> {
    let url = format!("{}/{}/_search", endpoint.trim_end_matches('/'), index);
    let value = transport::post_search(&url, body, timeout)?;
    decode_candidates(&value, score_key, limit)
}

fn decode_candidates(
    value: &Value,
    score_key: &str,
    limit: usize,
) -> Result<Vec<Document>, String> {
    let hits = value
        .pointer("/hits/hits")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenSearch response missing hits.hits".to_string())?;
    if hits.len() > limit {
        return Err(format!(
            "OpenSearch returned more hits than requested ({} > {limit})",
            hits.len()
        ));
    }
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        let id = hit
            .get("_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "OpenSearch hit missing _id".to_string())?;
        let score = hit.get("_score").and_then(Value::as_f64).unwrap_or(0.0);
        let mut candidate = doc! { "_id": id };
        candidate.insert(score_key, score);
        out.push(candidate);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_json_endpoints() {
        let cfg = parse_config(Some(
            br#"{"endpoints":["http://opensearch:9200"]}"#.to_vec(),
        ))
        .unwrap();
        assert_eq!(cfg.endpoints, vec!["http://opensearch:9200"]);
    }

    #[test]
    fn search_does_not_follow_redirects_to_another_endpoint() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let url = format!("http://{}", first.local_addr().unwrap());
        let redirect = format!("http://{}/other", target.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = first.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let length = std::str::from_utf8(&headers)
                .unwrap()
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            stream.read_exact(&mut vec![0; length]).unwrap();
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                if let Ok((mut redirected, _)) = target.accept() {
                    let body = r#"{"hits":{"hits":[]}}"#;
                    write!(
                        redirected,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        });
        let result = fetch_candidates_from_endpoint(
            &url,
            "mongodb.demo.items",
            &json!({}),
            "$vectorSearchScore",
            10,
            Duration::from_secs(2),
        );
        let followed = server.join().unwrap();
        assert!(result.is_err(), "redirect must not be accepted");
        assert!(
            !followed,
            "authorization must not be sent to the redirect target"
        );
    }

    fn hits(n: usize) -> Value {
        let hits: Vec<Value> = (0..n)
            .map(|i| json!({"_id": format!("p{i}"), "_score": 1.0}))
            .collect();
        json!({"hits": {"hits": hits}})
    }

    #[test]
    fn candidates_within_limit_are_decoded() {
        let docs = decode_candidates(&hits(2), "$vectorSearchScore", 2).unwrap();
        assert_eq!(
            docs,
            vec![
                doc! {"_id": "p0", "$vectorSearchScore": 1.0},
                doc! {"_id": "p1", "$vectorSearchScore": 1.0},
            ]
        );
    }

    #[test]
    fn more_hits_than_limit_is_rejected() {
        let err = decode_candidates(&hits(3), "$vectorSearchScore", 2).unwrap_err();
        assert!(err.contains("more hits than requested"), "{err}");
    }

    #[test]
    fn request_timeout_without_deadline_uses_default() {
        let max = Duration::from_millis(DEFAULT_TIMEOUT_MS);
        assert_eq!(request_timeout(None, 1_000, max).unwrap(), max);
        assert_eq!(request_timeout(Some(i64::MAX), 1_000, max).unwrap(), max);
        assert_eq!(request_timeout(Some(60_000), 1_000, max).unwrap(), max);
    }

    #[test]
    fn request_timeout_is_capped_by_query_deadline() {
        let max = Duration::from_millis(DEFAULT_TIMEOUT_MS);
        assert_eq!(
            request_timeout(Some(1_500), 1_000, max).unwrap(),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn request_timeout_rejects_expired_deadline() {
        let max = Duration::from_millis(DEFAULT_TIMEOUT_MS);
        for deadline in [1_000, 999, i64::MIN] {
            let err = request_timeout(Some(deadline), 1_000, max).unwrap_err();
            assert!(err.to_string().contains("deadline"), "{err}");
        }
    }

    #[test]
    fn parses_config_line_endpoints() {
        let cfg =
            parse_config(Some(b"endpoints: http://a:9200, http://b:9200\n".to_vec())).unwrap();
        assert_eq!(cfg.endpoints, vec!["http://a:9200", "http://b:9200"]);
    }

    #[test]
    fn rejects_missing_config_endpoints() {
        assert!(parse_config(None).is_err());
        assert!(parse_config(Some(b"sharedLibraryPath: /x.so\n".to_vec())).is_err());
    }

    #[test]
    fn rejects_non_http_endpoint() {
        assert!(parse_config(Some(b"endpoints: ftp://bad\n".to_vec())).is_err());
    }

    #[test]
    fn vector_args_require_path_and_query() {
        let args = parse_vector_search_args(doc! {
            "path": "description",
            "query": "waterproof",
            "limit": 3i32,
        })
        .unwrap();
        assert_eq!(args.path, "description");
        assert_eq!(args.query, "waterproof");
        assert_eq!(args.limit, 3);
    }

    #[test]
    fn args_reject_empty_path_query_and_bad_limit() {
        assert!(parse_vector_search_args(doc! { "path": "", "query": "boots" }).is_err());
        assert!(parse_vector_search_args(doc! { "path": "description", "query": "" }).is_err());
        assert!(parse_vector_search_args(
            doc! { "path": "description", "query": "boots", "limit": 0i32 }
        )
        .is_err());
        assert!(parse_vector_search_args(
            doc! { "path": "description", "query": "boots", "limit": 1001i32 }
        )
        .is_err());
    }

    fn with_filter(filter: impl Into<Bson>) -> ExtensionResult<QueryArgs> {
        parse_vector_search_args(doc! { "path": "description", "query": "boots", "filter": filter })
    }

    #[test]
    fn filter_accepts_allowlisted_clauses() {
        for filter in [
            doc! { "term": { "category": "bags" } },
            doc! { "term": { "category": { "value": "bags" } } },
            doc! { "terms": { "category": ["bags", "boots"] } },
            doc! { "ids": { "values": ["p007"] } },
            doc! { "exists": { "field": "price" } },
            doc! { "range": { "price": { "gte": 10, "lt": 99.5 } } },
            doc! { "bool": {
                "must": [{ "term": { "category": "bags" } }],
                "filter": { "range": { "price": { "lte": 50 } } },
                "should": [{ "terms": { "tags": ["rain"] } }],
                "must_not": [{ "ids": { "values": ["p001"] } }],
                "minimum_should_match": 1,
            } },
        ] {
            assert!(with_filter(filter.clone()).is_ok(), "{filter}");
        }
    }

    #[test]
    fn filter_rejects_dangerous_or_unknown_clauses() {
        for filter in [
            doc! { "script": { "script": { "source": "Thread.sleep(10000)" } } },
            doc! { "terms": { "category": { "index": "secrets", "id": "1", "path": "x" } } },
            doc! { "more_like_this": { "like": [{ "_index": "secrets", "_id": "1" }] } },
            doc! { "regexp": { "name": ".*(a+)+.*" } },
            doc! { "wildcard": { "name": "*a*" } },
            doc! { "query_string": { "query": "*" } },
            doc! { "bool": { "must": [{ "script": { "script": "1" } }] } },
            doc! { "bool": { "boost": 2 } },
            doc! { "term": { "category": "bags" }, "ids": { "values": ["x"] } },
            doc! { "term": { "_index": "secrets" } },
            doc! { "term": { "category": { "value": { "nested": 1 } } } },
            doc! { "range": { "price": { "gte": 1, "script": "x" } } },
            doc! {},
        ] {
            assert!(with_filter(filter.clone()).is_err(), "{filter}");
        }
    }

    #[test]
    fn filter_rejects_excessive_nesting() {
        let mut filter = doc! { "term": { "category": "bags" } };
        for _ in 0..MAX_FILTER_DEPTH {
            filter = doc! { "bool": { "filter": [filter] } };
        }
        assert!(with_filter(filter).is_err());
    }

    #[test]
    fn filter_must_be_a_document_not_silently_ignored() {
        for filter in [
            Bson::String("category:bags".into()),
            Bson::Array(vec![]),
            Bson::Int32(1),
        ] {
            assert!(with_filter(filter.clone()).is_err(), "{filter}");
        }
    }

    #[test]
    fn vector_body_disables_source() {
        let args =
            parse_vector_search_args(doc! { "path": "description", "query": "boots" }).unwrap();
        let body = opensearch_body(&args);
        assert_eq!(body["size"], 10);
        assert_eq!(body["_source"], false);
        assert_eq!(
            body["query"]["neural"]["description_embedding"]["query_text"],
            "boots"
        );
    }

    #[test]
    fn vector_filter_is_forwarded_without_legacy_tombstone_condition() {
        let vector = parse_vector_search_args(doc! {
            "path": "description", "query": "boots",
            "filter": { "term": { "category": "bags" } }
        })
        .unwrap();
        let body = opensearch_body(&vector);
        assert_eq!(
            body["query"]["neural"]["description_embedding"]["filter"]["term"]["category"],
            "bags"
        );
    }

    #[test]
    fn vector_body_includes_filter_when_present() {
        let args = parse_vector_search_args(doc! {
            "path": "description",
            "query": "boots",
            "filter": { "term": { "category": "bags" } },
        })
        .unwrap();
        let body = opensearch_body(&args);
        assert_eq!(
            body["query"]["neural"]["description_embedding"]["filter"]["term"]["category"],
            "bags"
        );
    }

    #[test]
    fn vector_body_uses_neural_query_text_without_model_id() {
        let args = parse_vector_search_args(doc! { "path": "description", "query": "rain shell" })
            .unwrap();
        let body = opensearch_body(&args);
        assert_eq!(
            body["query"]["neural"]["description_embedding"]["query_text"],
            "rain shell"
        );
        assert!(body["query"]["neural"]["description_embedding"]["model_id"].is_null());
        assert!(body["query"]["neural"]["description_embedding"]["filter"].is_null());
    }

    #[test]
    fn vector_body_targets_path_embedding_field_and_limit() {
        let args = parse_vector_search_args(doc! {
            "path": "description",
            "query": "rain shell",
            "limit": 7i32,
        })
        .unwrap();
        let body = opensearch_body(&args);
        assert_eq!(body["size"], 7);
        assert_eq!(body["query"]["neural"]["description_embedding"]["k"], 7);
    }

    #[test]
    fn expansion_preserves_vector_filter() {
        let filter = doc! { "ids": { "values": ["demo-shell"] } };
        let args = parse_vector_search_args(
            doc! { "path": "description", "query": "winter", "filter": &filter },
        )
        .unwrap();
        let stages = id_lookup_expansion("$vectorSearch", &args).stage_documents();
        let expanded = stages[0].get_document("$vectorSearch").unwrap();
        assert_eq!(expanded.get_document("filter").unwrap(), &filter);
    }

    #[test]
    fn expansion_appends_internal_id_lookup() {
        let args = parse_vector_search_args(doc! {
            "path": "description",
            "query": "boots",
            "limit": 5i32,
        })
        .unwrap();
        let expansion = id_lookup_expansion("$vectorSearch", &args);
        let stages = expansion.stage_documents();
        assert_eq!(
            stages[0],
            doc! { "$vectorSearch": { "path": "description", "query": "boots", "limit": 5i64 } }
        );
        assert_eq!(
            stages[1],
            doc! { "$_internalSearchIdLookup": { "limit": 5i64 } }
        );
    }

    #[test]
    fn next_result_moves_vector_score_to_metadata() {
        let args = parse_vector_search_args(doc! { "path": "description", "query": "warm sleep" })
            .unwrap();
        let mut state = SearchState {
            args,
            index: "search_demo.products".to_string(),
            config: OpenSearchConfig {
                endpoints: vec!["http://opensearch:9200".to_string()],
            },
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            pending: VecDeque::from([doc! { "_id": "p003", "$vectorSearchScore": 0.75 }]),
            loaded: true,
        };
        let mut ctx = StageContext::default();
        let next = next_result(&mut state, &mut ctx).unwrap();
        let Next::Advanced { document, metadata } = next else {
            panic!("expected advanced result");
        };
        assert_eq!(document, doc! { "_id": "p003" });
        assert_eq!(metadata, Some(doc! { "$vectorSearchScore": 0.75 }));
    }
}
