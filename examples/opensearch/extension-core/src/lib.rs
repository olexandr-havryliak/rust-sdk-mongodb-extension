use std::collections::VecDeque;
use std::time::Duration;

use bson::{doc, Bson, Document};
use extension_sdk_mongodb::{ExtensionError, ExtensionResult, Next, StageContext};
use serde_json::{json, Value};

const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 1000;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSearchConfig {
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryKind {
    Search,
    VectorSearch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryArgs {
    pub kind: QueryKind,
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

fn optional_filter(args: &Document) -> ExtensionResult<Option<Document>> {
    match args.get("filter") {
        None => Ok(None),
        Some(Bson::Document(filter)) => Ok(Some(filter.clone())),
        Some(_) => Err(ExtensionError::BadValue(
            "filter must be a document".into(),
        )),
    }
}

pub fn parse_search_args(args: Document) -> ExtensionResult<QueryArgs> {
    Ok(QueryArgs {
        kind: QueryKind::Search,
        path: bson_string(&args, "path")?,
        query: bson_string(&args, "query")?,
        limit: bson_i64(&args, "limit", DEFAULT_LIMIT, 1, MAX_LIMIT)?,
        filter: optional_filter(&args)?,
    })
}

pub fn parse_vector_search_args(args: Document) -> ExtensionResult<QueryArgs> {
    Ok(QueryArgs {
        kind: QueryKind::VectorSearch,
        path: bson_string(&args, "path")?,
        query: bson_string(&args, "query")?,
        limit: bson_i64(&args, "limit", DEFAULT_LIMIT, 1, MAX_LIMIT)?,
        filter: optional_filter(&args)?,
    })
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
                    .ok_or_else(|| ExtensionError::BadValue("endpoints must contain strings".into()))
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
    match args.kind {
        QueryKind::Search => {
            let mut filters = Vec::new();
            if let Some(filter) = &args.filter {
                filters.push(bson_doc_to_json(filter));
            }
            json!({
                "size": args.limit,
                "_source": ["_mongo_id"],
                "query": {
                    "bool": {
                        "must": [{ "match": { args.path.clone(): args.query.clone() } }],
                        "filter": filters,
                        "must_not": [{ "term": { "_sync_deleted": true } }]
                    }
                }
            })
        }
        QueryKind::VectorSearch => {
            let mut neural = json!({
                "query_text": args.query,
                "k": args.limit
            });
            let filters: Vec<Value> = args.filter.iter().map(bson_doc_to_json).collect();
            neural["filter"] = json!({ "bool": {
                "filter": filters,
                "must_not": [{ "term": { "_sync_deleted": true } }]
            }});
            json!({
                "size": args.limit,
                "_source": ["_mongo_id"],
                "query": {
                    "neural": {
                        format!("{}_embedding", args.path): neural
                    }
                }
            })
        }
    }
}

fn bson_doc_to_json(doc: &Document) -> Value {
    serde_json::to_value(Bson::Document(doc.clone())).unwrap_or(Value::Null)
}

pub fn open_state(args: QueryArgs, ctx: &mut StageContext) -> ExtensionResult<SearchState> {
    let catalog = ctx.catalog().ok_or_else(|| {
        ExtensionError::BadValue("OpenSearch search stages require collection catalog context".into())
    })?;
    let config = parse_config(ctx.extension_options_raw())?;
    Ok(SearchState {
        args,
        index: catalog.namespace(),
        config,
        timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        pending: VecDeque::new(),
        loaded: false,
    })
}

pub fn next_result(state: &mut SearchState, ctx: &mut StageContext) -> ExtensionResult<Next> {
    ctx.check_interrupt()?;
    if !state.loaded {
        state.pending = VecDeque::from(fetch_candidates(state)?);
        state.loaded = true;
    }
    match state.pending.pop_front() {
        Some(mut document) => {
            let metadata = document
                .remove("$searchScore")
                .map(|score| doc! { "$searchScore": score })
                .or_else(|| {
                    document
                        .remove("$vectorSearchScore")
                        .map(|score| doc! { "$vectorSearchScore": score })
                });
            Ok(Next::Advanced { document, metadata })
        }
        None => Ok(Next::Eof),
    }
}

fn fetch_candidates(state: &SearchState) -> ExtensionResult<Vec<Document>> {
    let body = opensearch_body(&state.args);
    let score_key = score_metadata_key(&state.args.kind);
    let mut last_error = None;
    for endpoint in &state.config.endpoints {
        match fetch_candidates_from_endpoint(endpoint, &state.index, &body, score_key, state.timeout)
        {
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
    timeout: Duration,
) -> Result<Vec<Document>, String> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let url = format!("{}/{}/_search", endpoint.trim_end_matches('/'), index);
    let response = agent
        .post(&url)
        .send_json(body.clone())
        .map_err(|e| e.to_string())?;
    let value: Value = response.into_json().map_err(|e| e.to_string())?;
    let hits = value
        .pointer("/hits/hits")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenSearch response missing hits.hits".to_string())?;
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        out.push(candidate_from_hit(hit, score_key)?);
    }
    Ok(out)
}

fn candidate_from_hit(hit: &Value, score_key: &str) -> Result<Document, String> {
    let id = bson_id_from_hit(hit)?;
    let score = hit.get("_score").and_then(Value::as_f64).unwrap_or(0.0);
    let mut candidate = Document::new();
    candidate.insert("_id", id);
    candidate.insert(score_key, score);
    Ok(candidate)
}

fn bson_id_from_hit(hit: &Value) -> Result<Bson, String> {
    if let Some(preserved) = hit.pointer("/_source/_mongo_id") {
        let text = preserved
            .as_str()
            .ok_or_else(|| "OpenSearch _mongo_id must be a string".to_string())?;
        let value: Value = serde_json::from_str(text)
            .map_err(|err| format!("OpenSearch _mongo_id json: {err}"))?;
        return bson_id_from_json(&value);
    }
    let id = hit
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "OpenSearch hit missing _id".to_string())?;
    Ok(Bson::String(id.to_string()))
}

fn bson_id_from_json(value: &Value) -> Result<Bson, String> {
    match value {
        Value::String(text) => Ok(Bson::String(text.clone())),
        Value::Number(number) => bson_id_from_json_number(number),
        Value::Object(fields) if fields.len() == 1 => decode_extended_id(fields),
        _ => Err("preserved MongoDB _id is not a supported BSON id".into()),
    }
}

/// Simplified JSON emits ordinary integers as JSON numbers and drops the BSON
/// int32/int64 distinction. Values that fit in int32 are restored as int32, which
/// is what mongosh and the connector emit for `_id: 1`. Explicit `$numberInt` and
/// `$numberLong` forms keep the width they declare.
fn bson_id_from_json_number(number: &serde_json::Number) -> Result<Bson, String> {
    if let Some(value) = number.as_i64() {
        if (i32::MIN as i64..=i32::MAX as i64).contains(&value) {
            return Ok(Bson::Int32(value as i32));
        }
        return Ok(Bson::Int64(value));
    }
    if let Some(value) = number.as_u64() {
        if value <= i32::MAX as u64 {
            return Ok(Bson::Int32(value as i32));
        }
        if value <= i64::MAX as u64 {
            return Ok(Bson::Int64(value as i64));
        }
        return Err(format!("preserved MongoDB _id integer is too large: {value}"));
    }
    let value = number
        .as_f64()
        .ok_or_else(|| "preserved MongoDB _id number is not finite".to_string())?;
    if !value.is_finite() {
        return Err("preserved MongoDB _id number is not finite".into());
    }
    Ok(Bson::Double(value))
}

fn decode_extended_id(fields: &serde_json::Map<String, Value>) -> Result<Bson, String> {
    let (key, value) = fields
        .iter()
        .next()
        .ok_or_else(|| "preserved MongoDB _id object is empty".to_string())?;
    let text = value
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))?;
    match key.as_str() {
        "$oid" => {
            let oid = bson::oid::ObjectId::parse_str(text)
                .map_err(|err| format!("invalid $oid: {err}"))?;
            Ok(Bson::ObjectId(oid))
        }
        "$uuid" => {
            let uuid = bson::Uuid::parse_str(text).map_err(|err| format!("invalid $uuid: {err}"))?;
            Ok(Bson::Binary(bson::Binary::from_uuid(uuid)))
        }
        "$numberInt" => {
            let parsed = text
                .parse::<i32>()
                .map_err(|_| format!("invalid $numberInt: {text}"))?;
            Ok(Bson::Int32(parsed))
        }
        "$numberLong" => {
            let parsed = text
                .parse::<i64>()
                .map_err(|_| format!("invalid $numberLong: {text}"))?;
            Ok(Bson::Int64(parsed))
        }
        "$numberDouble" => {
            let parsed = text
                .parse::<f64>()
                .map_err(|_| format!("invalid $numberDouble: {text}"))?;
            if !parsed.is_finite() {
                return Err("preserved MongoDB _id double is not finite".into());
            }
            Ok(Bson::Double(parsed))
        }
        other => Err(format!("unsupported preserved MongoDB _id type {other}")),
    }
}

fn score_metadata_key(kind: &QueryKind) -> &'static str {
    match kind {
        QueryKind::Search => "$searchScore",
        QueryKind::VectorSearch => "$vectorSearchScore",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_json_endpoints() {
        let cfg = parse_config(Some(br#"{"endpoints":["http://opensearch:9200"]}"#.to_vec()))
            .unwrap();
        assert_eq!(cfg.endpoints, vec!["http://opensearch:9200"]);
    }

    #[test]
    fn parses_config_line_endpoints() {
        let cfg = parse_config(Some(b"endpoints: http://a:9200, http://b:9200\n".to_vec()))
            .unwrap();
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
    fn search_args_require_path_and_query() {
        let args = parse_search_args(doc! {
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
        assert!(parse_search_args(doc! { "path": "", "query": "boots" }).is_err());
        assert!(parse_search_args(doc! { "path": "description", "query": "" }).is_err());
        assert!(parse_search_args(doc! { "path": "description", "query": "boots", "limit": 0i32 }).is_err());
        assert!(
            parse_search_args(doc! { "path": "description", "query": "boots", "limit": 1001i32 })
                .is_err()
        );
    }

    #[test]
    fn search_rejects_non_document_filter() {
        let err = parse_search_args(doc! {
            "path": "description",
            "query": "boots",
            "filter": "category",
        })
        .expect_err("string filter");
        assert!(
            matches!(err, ExtensionError::BadValue(ref msg) if msg.contains("filter")),
            "{err}"
        );
    }

    #[test]
    fn vector_search_rejects_non_document_filter() {
        let err = parse_vector_search_args(doc! {
            "path": "description",
            "query": "boots",
            "filter": [1i32],
        })
        .expect_err("array filter");
        assert!(
            matches!(err, ExtensionError::BadValue(ref msg) if msg.contains("filter")),
            "{err}"
        );
    }

    #[test]
    fn search_body_requests_preserved_mongo_id() {
        let args = parse_search_args(doc! { "path": "description", "query": "boots" }).unwrap();
        let body = opensearch_body(&args);
        assert_eq!(body["size"], 10);
        assert_eq!(body["_source"], json!(["_mongo_id"]));
        assert_eq!(body["query"]["bool"]["must"][0]["match"]["description"], "boots");
    }

    #[test]
    fn both_queries_exclude_persistent_tombstones() {
        let search = parse_search_args(doc! { "path": "description", "query": "boots" }).unwrap();
        let vector = parse_vector_search_args(doc! {
            "path": "description", "query": "boots",
            "filter": { "term": { "category": "bags" } }
        }).unwrap();
        assert_eq!(opensearch_body(&search)["query"]["bool"]["must_not"][0]["term"]["_sync_deleted"], true);
        let body = opensearch_body(&vector);
        let filter = &body["query"]["neural"]["description_embedding"]["filter"]["bool"];
        assert_eq!(filter["must_not"][0]["term"]["_sync_deleted"], true);
        assert_eq!(filter["filter"][0]["term"]["category"], "bags");
    }

    #[test]
    fn search_body_includes_filter_when_present() {
        let args = parse_search_args(doc! {
            "path": "description",
            "query": "boots",
            "filter": { "term": { "category": "bags" } },
        })
        .unwrap();
        let body = opensearch_body(&args);
        assert_eq!(body["query"]["bool"]["filter"][0]["term"]["category"], "bags");
    }

    #[test]
    fn vector_body_uses_neural_query_text_without_model_id() {
        let args =
            parse_vector_search_args(doc! { "path": "description", "query": "rain shell" }).unwrap();
        let body = opensearch_body(&args);
        assert_eq!(
            body["query"]["neural"]["description_embedding"]["query_text"],
            "rain shell"
        );
        assert!(body["query"]["neural"]["description_embedding"]["model_id"].is_null());
        assert_eq!(body["_source"], json!(["_mongo_id"]));
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
    fn expansion_preserves_filters_for_both_search_kinds() {
        let filter = doc! { "ids": { "values": ["demo-shell"] } };
        for (name, args) in [
            (
                "$search",
                parse_search_args(doc! { "path": "description", "query": "winter", "filter": &filter }).unwrap(),
            ),
            (
                "$vectorSearch",
                parse_vector_search_args(doc! { "path": "description", "query": "winter", "filter": &filter }).unwrap(),
            ),
        ] {
            let stages = id_lookup_expansion(name, &args).stage_documents();
            let expanded = stages[0].get_document(name).unwrap();
            assert_eq!(expanded.get_document("filter").unwrap(), &filter);
        }
    }

    #[test]
    fn expansion_appends_internal_id_lookup() {
        let args = parse_search_args(doc! {
            "path": "description",
            "query": "boots",
            "limit": 5i32,
        })
        .unwrap();
        let expansion = id_lookup_expansion("$search", &args);
        let stages = expansion.stage_documents();
        assert_eq!(stages[0], doc! { "$search": { "path": "description", "query": "boots", "limit": 5i64 } });
        assert_eq!(stages[1], doc! { "$_internalSearchIdLookup": { "limit": 5i64 } });
    }

    #[test]
    fn next_result_moves_search_score_to_metadata() {
        let args = parse_search_args(doc! { "path": "description", "query": "boots" }).unwrap();
        let mut state = SearchState {
            args,
            index: "search_demo.products".to_string(),
            config: OpenSearchConfig {
                endpoints: vec!["http://opensearch:9200".to_string()],
            },
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            pending: VecDeque::from([doc! { "_id": "p001", "$searchScore": 12.5 }]),
            loaded: true,
        };
        let mut ctx = StageContext::default();
        let next = next_result(&mut state, &mut ctx).unwrap();
        let Next::Advanced { document, metadata } = next else {
            panic!("expected advanced result");
        };
        assert_eq!(document, doc! { "_id": "p001" });
        assert_eq!(metadata, Some(doc! { "$searchScore": 12.5 }));
    }

    #[test]
    fn next_result_moves_vector_score_to_metadata() {
        let args =
            parse_vector_search_args(doc! { "path": "description", "query": "warm sleep" }).unwrap();
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

    #[test]
    fn candidate_keeps_string_id_when_preserved_id_is_absent() {
        let hit = json!({ "_id": "p001", "_score": 1.5 });
        let doc = candidate_from_hit(&hit, "$searchScore").unwrap();
        assert_eq!(doc, doc! { "_id": "p001", "$searchScore": 1.5 });
    }

    #[test]
    fn candidate_restores_preserved_string_id() {
        let hit = json!({
            "_id": "ignored",
            "_score": 2.0,
            "_source": { "_mongo_id": "\"p001\"" }
        });
        let doc = candidate_from_hit(&hit, "$searchScore").unwrap();
        assert_eq!(doc.get("_id"), Some(&Bson::String("p001".into())));
    }

    #[test]
    fn candidate_restores_small_json_integer_as_int32() {
        let hit = json!({
            "_id": "1",
            "_score": 1.0,
            "_source": { "_mongo_id": "1" }
        });
        let doc = candidate_from_hit(&hit, "$searchScore").unwrap();
        assert_eq!(doc.get("_id"), Some(&Bson::Int32(1)));
    }

    #[test]
    fn candidate_restores_object_id() {
        let oid = bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
        let hit = json!({
            "_id": "507f1f77bcf86cd799439011",
            "_score": 1.0,
            "_source": { "_mongo_id": "{\"$oid\":\"507f1f77bcf86cd799439011\"}" }
        });
        let doc = candidate_from_hit(&hit, "$searchScore").unwrap();
        assert_eq!(doc.get("_id"), Some(&Bson::ObjectId(oid)));
    }

    #[test]
    fn candidate_restores_number_long_that_fits_in_int32() {
        let hit = json!({
            "_id": "1",
            "_score": 1.0,
            "_source": { "_mongo_id": "{\"$numberLong\":\"1\"}" }
        });
        let doc = candidate_from_hit(&hit, "$searchScore").unwrap();
        assert_eq!(doc.get("_id"), Some(&Bson::Int64(1)));
    }
}
