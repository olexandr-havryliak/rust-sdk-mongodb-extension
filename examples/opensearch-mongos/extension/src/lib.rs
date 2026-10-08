use std::{collections::VecDeque, ffi::c_void, time::Duration};

use bson::{doc, Bson, Document};
use extension_sdk_mongodb::source_stage::{get_multi_source_extension_impl, SourceOps};
use extension_sdk_mongodb::{
    stage_context::CatalogContext, ExtensionError, ExtensionResult, HostTypeRequirement, Next,
    SourceStage, StageContext, StageProperties,
};
use opensearch_extension_core::{
    opensearch_body, parse_config, parse_vector_search_args, OpenSearchConfig, QueryArgs,
};
use serde_json::Value;

fn invalid(message: impl Into<String>) -> ExtensionError {
    ExtensionError::Runtime(message.into())
}

fn decode_hit(hit: &Value) -> ExtensionResult<(Document, f64)> {
    let raw = hit
        .get("_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("OpenSearch hit missing string _id"))?;
    if raw.len() > 512 {
        return Err(invalid(
            "OpenSearch composite document key exceeds 512 UTF-8 bytes",
        ));
    }
    let json: Value = serde_json::from_str(raw)
        .map_err(|e| invalid(format!("Invalid document key JSON: {e}")))?;
    let bson =
        Bson::try_from(json).map_err(|e| invalid(format!("Invalid document key BSON: {e}")))?;
    let Bson::Document(key) = bson else {
        return Err(invalid("Document key must be an object"));
    };
    if !key.contains_key("_id")
        || key.keys().any(|path| {
            path.split('.')
                .any(|part| part.is_empty() || part.starts_with('$') || part.contains('\0'))
        })
    {
        return Err(invalid(
            "Document key needs _id and valid MongoDB field paths",
        ));
    }
    let score = hit
        .get("_score")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite())
        .ok_or_else(|| invalid("OpenSearch hit missing finite score"))?;
    Ok((key, score))
}

fn lookup_pipeline(catalog: Option<&CatalogContext>) -> ExtensionResult<Vec<Document>> {
    let c = catalog
        .filter(|c| c.in_router)
        .ok_or_else(|| invalid("OpenSearch mongos extension requires a bound router namespace"))?;
    // Field paths come with each candidate, not from client aggregation arguments.
    // Read dotted paths component by component, and normalize missing shard keys to null.
    let key_matches = doc! {"$allElementsTrue": [{"$map": {
        "input": "$$documentKey", "as": "part", "in": {"$eq": [
            {"$ifNull": [{"$reduce": {
                "input": {"$split": ["$$part.k", "."]}, "initialValue": "$$ROOT",
                "in": {"$cond": [ {"$eq": [{"$type": "$$value"}, "object"]},
                    {"$getField": {"field": "$$this", "input": "$$value"}}, null ]}
            }}, null]}, "$$part.v"]}
    }}]};
    Ok(vec![
        doc! {"$lookup": {"from": &c.collection_name, "localField": "_id", "foreignField": "_id",
        "let": {"documentKey": "$__mongoKey"},
        "pipeline": [{"$match": {"$expr": key_matches}}], "as": "__mongoDocument"}},
        doc! {"$unwind": "$__mongoDocument"},
        doc! {"$replaceWith": "$__mongoDocument"},
    ])
}

struct RouterVectorSearch;

struct SearchState {
    args: QueryArgs,
    index: String,
    config: OpenSearchConfig,
    rows: Option<VecDeque<(Document, f64)>>,
}

impl SourceStage for RouterVectorSearch {
    const NAME: &'static str = "$vectorSearch";
    type Args = QueryArgs;
    type State = SearchState;

    fn parse(args: Document) -> ExtensionResult<QueryArgs> {
        for name in args.keys() {
            if !["path", "query", "limit", "filter"].contains(&name.as_str()) {
                return Err(ExtensionError::BadValue(format!(
                    "Unsupported vectorSearch argument: {name}"
                )));
            }
        }
        if args
            .get("filter")
            .is_some_and(|v| !matches!(v, Bson::Document(_)))
        {
            return Err(ExtensionError::BadValue("filter must be a document".into()));
        }
        if let Some(limit) = args.get("limit") {
            if !matches!(limit, Bson::Int32(_) | Bson::Int64(_)) {
                return Err(ExtensionError::BadValue("limit must be an integer".into()));
            }
        }
        let parsed = parse_vector_search_args(args)?;
        if parsed.path.starts_with('_')
            || parsed.path.contains('.')
            || parsed.path.ends_with("_embedding")
        {
            return Err(ExtensionError::BadValue(
                "path must be a projected flat text field".into(),
            ));
        }
        Ok(parsed)
    }

    fn properties() -> StageProperties {
        StageProperties {
            requires_input: false,
            ..StageProperties::source_stage_default()
        }
    }

    fn host_type() -> HostTypeRequirement {
        HostTypeRequirement::Router
    }

    fn merging_pipeline(
        _: &QueryArgs,
        catalog: Option<&CatalogContext>,
    ) -> ExtensionResult<Option<Vec<Document>>> {
        lookup_pipeline(catalog).map(Some)
    }

    fn open(args: QueryArgs, ctx: &mut StageContext) -> ExtensionResult<SearchState> {
        let c = ctx
            .catalog()
            .filter(|c| c.in_router)
            .ok_or_else(|| invalid("OpenSearch mongos extension must execute on mongos"))?;
        Ok(SearchState {
            args,
            index: format!("mongodb.{}", c.namespace()),
            config: parse_config(ctx.extension_options_raw())?,
            rows: None,
        })
    }

    fn next(state: &mut SearchState, ctx: &mut StageContext) -> ExtensionResult<Next> {
        ctx.check_interrupt()?;
        if state.rows.is_none() {
            state.rows = Some(fetch(state, ctx)?.into());
        }
        Ok(match state.rows.as_mut().unwrap().pop_front() {
            Some((key, score)) => Next::Advanced {
                document: doc! {"_id": key.get("_id").unwrap().clone(),
                "__mongoKey": key.into_iter().map(|(k,v)| Bson::Document(doc! {"k": k, "v": v})).collect::<Vec<_>>()},
                metadata: Some(doc! {"$vectorSearchScore": score}),
            },
            None => Next::Eof,
        })
    }
}

fn decode_response(value: Value) -> ExtensionResult<Vec<(Document, f64)>> {
    if value.get("timed_out").and_then(Value::as_bool) == Some(true)
        || value
            .pointer("/_shards/failed")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    {
        return Err(invalid("OpenSearch returned incomplete results"));
    }
    value
        .pointer("/hits/hits")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("OpenSearch response missing hits.hits"))?
        .iter()
        .map(decode_hit)
        .collect()
}

fn fetch(state: &SearchState, ctx: &mut StageContext) -> ExtensionResult<Vec<(Document, f64)>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    let body = opensearch_body(&state.args);
    let mut errors = Vec::new();
    for endpoint in &state.config.endpoints {
        ctx.check_interrupt()?;
        let url = format!(
            "{}/{}/_search?allow_partial_search_results=false",
            endpoint.trim_end_matches('/'),
            state.index
        );
        match agent
            .post(&url)
            .send_json(body.clone())
            .and_then(|r| r.into_json::<Value>().map_err(ureq::Error::from))
        {
            Ok(response) => return decode_response(response),
            Err(e) => errors.push(format!("{endpoint}: {e}")),
        }
    }
    Err(invalid(format!(
        "All OpenSearch endpoints failed: {}",
        errors.join("; ")
    )))
}

fn open_raw(args: Document, ctx: &mut StageContext) -> ExtensionResult<*mut c_void> {
    Ok(Box::into_raw(Box::new(RouterVectorSearch::open(
        RouterVectorSearch::parse(args)?,
        ctx,
    )?)) as *mut c_void)
}

unsafe fn drop_raw(ptr: *mut c_void) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr as *mut SearchState));
    }
}

unsafe fn next_raw(ptr: *mut c_void, ctx: &mut StageContext) -> ExtensionResult<Next> {
    RouterVectorSearch::next(&mut *(ptr as *mut SearchState), ctx)
}

fn properties() -> Document {
    let mut d =
        RouterVectorSearch::properties().to_document_with_host_type(HostTypeRequirement::Router);
    d.insert("providedMetadataFields", vec!["vectorSearchScore"]);
    d
}

fn expand(args: Document) -> ExtensionResult<extension_sdk_mongodb::Expansion> {
    RouterVectorSearch::parse(args.clone())?;
    Ok(extension_sdk_mongodb::Expansion::SelfStage)
}

fn merging(
    args: Document,
    catalog: Option<&CatalogContext>,
) -> ExtensionResult<Option<Vec<Document>>> {
    RouterVectorSearch::merging_pipeline(&RouterVectorSearch::parse(args)?, catalog)
}

static OPS: SourceOps = SourceOps {
    name: "$vectorSearch",
    expect_empty: false,
    open_from_doc: open_raw,
    drop_state: drop_raw,
    next: next_raw,
    on_extension_initialized: None,
    static_properties_doc: properties,
    expand_inner: expand,
    merging_pipeline: Some(merging),
};

#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension_versions(
    out: *mut extension_sdk_mongodb::sys::MongoExtensionAPIVersionVector,
) {
    extension_sdk_mongodb::version::write_supported_versions(out)
}

#[no_mangle]
pub unsafe extern "C" fn get_mongodb_extension(
    version: extension_sdk_mongodb::sys::MongoExtensionAPIVersion,
    services: *const extension_sdk_mongodb::sys::MongoExtensionHostServices,
    out: *mut *const extension_sdk_mongodb::sys::MongoExtension,
) -> *mut extension_sdk_mongodb::sys::MongoExtensionStatus {
    get_multi_source_extension_impl(&[&OPS], version, services, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hit(key: Document) -> Value {
        json!({"_id": serde_json::to_string(&Bson::Document(key).into_canonical_extjson()).unwrap(), "_score": 0.75})
    }

    #[test]
    fn typed_keys_roundtrip_without_stringifying_bson_values() {
        for id in [
            Bson::Int32(7),
            Bson::Int64(7),
            Bson::Double(7.0),
            Bson::ObjectId(bson::oid::ObjectId::new()),
            Bson::String("7".into()),
            Bson::Document(doc! {"a": 1}),
            Bson::DateTime(bson::DateTime::from_millis(123)),
        ] {
            let key = doc! {"_id": id, "region": "eu", "tenant": 12i64};
            assert_eq!(decode_hit(&hit(key.clone())).unwrap(), (key, 0.75));
        }
    }

    #[test]
    fn duplicate_ids_on_different_shards_keep_distinct_full_keys() {
        let left = doc! {"_id": "same", "tenant": -1};
        let right = doc! {"_id": "same", "tenant": 1};
        assert_ne!(
            decode_hit(&hit(left)).unwrap().0,
            decode_hit(&hit(right)).unwrap().0
        );
    }

    #[test]
    fn key_limit_counts_utf8_bytes_and_accepts_exactly_512() {
        let overhead = hit(doc! {"_id": ""})["_id"].as_str().unwrap().len();
        let exact = hit(doc! {"_id": "x".repeat(512 - overhead)});
        assert_eq!(exact["_id"].as_str().unwrap().len(), 512);
        assert!(decode_hit(&exact).is_ok());
        assert!(decode_hit(&hit(doc! {"_id": "x".repeat(513 - overhead)})).is_err());
        assert!(decode_hit(&hit(doc! {"_id": "ї".repeat(256)})).is_err());
    }

    #[test]
    fn malformed_keys_and_scores_are_errors_not_missing_results() {
        for value in [
            json!({}),
            json!({"_id":"not json", "_score":1}),
            json!({"_id":"[]", "_score":1}),
            json!({"_id":"{}", "_score":1}),
            json!({"_id":"{\"_id\":1}"}),
            json!({"_id":"{\"_id\":1}", "_score":null}),
            json!({"_id":"{\"_id\":1,\"$bad\":2}", "_score":1}),
        ] {
            assert!(decode_hit(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn lookup_is_bound_to_router_collection_and_checks_full_key() {
        let catalog = CatalogContext {
            database_name: "shop".into(),
            collection_name: "items".into(),
            uuid: None,
            in_router: true,
            verbosity: 0,
        };
        let pipeline = lookup_pipeline(Some(&catalog)).unwrap();
        let lookup = pipeline[0].get_document("$lookup").unwrap();
        assert_eq!(lookup.get_str("from").unwrap(), "items");
        assert_eq!(lookup.get_str("localField").unwrap(), "_id");
        assert_eq!(lookup.get_str("foreignField").unwrap(), "_id");
        assert!(lookup.get_array("pipeline").unwrap()[0]
            .as_document()
            .unwrap()
            .contains_key("$match"));
        assert_eq!(
            pipeline.last().unwrap(),
            &doc! {"$replaceWith": "$__mongoDocument"}
        );
        assert!(lookup_pipeline(None).is_err());
        let mut shard = catalog;
        shard.in_router = false;
        assert!(lookup_pipeline(Some(&shard)).is_err());
    }

    #[test]
    fn only_vector_search_is_registered_as_router_generator_with_score() {
        assert_eq!(OPS.name, "$vectorSearch");
        let p = properties();
        assert_eq!(p.get_str("hostType").unwrap(), "router");
        assert!(!p.get_bool("requiresInputDocSource").unwrap());
        assert_eq!(
            p.get_array("providedMetadataFields").unwrap(),
            &[Bson::String("vectorSearchScore".into())]
        );
        assert!(OPS.merging_pipeline.is_some());
        assert_eq!(
            expand(doc! {"path": "description", "query": "rain"}).unwrap(),
            extension_sdk_mongodb::Expansion::SelfStage
        );
    }

    #[test]
    fn rejects_unknown_arguments_noninteger_limit_and_reserved_paths() {
        for args in [
            doc! {"path": "description", "query": "rain", "model_id": "x"},
            doc! {"path": "description", "query": "rain", "limit": 1.5},
            doc! {"path": "description", "query": "rain", "filter": 5},
            doc! {"path": "__mongodb", "query": "rain"},
            doc! {"path": "nested.text", "query": "rain"},
            doc! {"path": "description_embedding", "query": "rain"},
        ] {
            assert!(RouterVectorSearch::parse(args).is_err());
        }
    }

    #[test]
    fn generator_preserves_full_key_order_metadata_and_repeated_eof() {
        let args =
            RouterVectorSearch::parse(doc! {"path": "description", "query": "rain"}).unwrap();
        let mut state = SearchState {
            args,
            index: "mongodb.shop.items".into(),
            config: OpenSearchConfig {
                endpoints: vec!["http://unused:9200".into()],
            },
            rows: Some(VecDeque::from([
                (
                    doc! {"_id": 2i64, "location.region": "eu", "tenant": -1},
                    0.9,
                ),
                (
                    doc! {"_id": 2i64, "location.region": "eu", "tenant": 1},
                    0.8,
                ),
            ])),
        };
        let mut ctx = StageContext::new();
        for (tenant, score) in [(-1, 0.9), (1, 0.8)] {
            let Next::Advanced { document, metadata } =
                RouterVectorSearch::next(&mut state, &mut ctx).unwrap()
            else {
                panic!("expected row");
            };
            assert_eq!(document.get_i64("_id").unwrap(), 2);
            assert_eq!(metadata, Some(doc! {"$vectorSearchScore": score}));
            assert_eq!(
                document.get_array("__mongoKey").unwrap()[2],
                Bson::Document(doc! {"k": "tenant", "v": tenant})
            );
        }
        for _ in 0..2 {
            assert!(matches!(
                RouterVectorSearch::next(&mut state, &mut ctx).unwrap(),
                Next::Eof
            ));
        }
    }

    #[test]
    fn rejects_partial_responses_and_invalid_hit_without_silently_skipping_it() {
        for response in [
            json!({"timed_out": true, "hits": {"hits": []}}),
            json!({"_shards": {"failed": 1}, "hits": {"hits": []}}),
            json!({"hits": {"hits": [hit(doc! {"_id": 1}), {"_id": "bad", "_score": 1}]}}),
            json!({"hits": {}}),
        ] {
            assert!(decode_response(response).is_err());
        }
        assert_eq!(
            decode_response(json!({"hits": {"hits": []}})).unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn endpoint_failover_uses_native_neural_query_and_decodes_typed_key() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let response =
            json!({"hits": {"hits": [hit(doc! {"_id": 7i64, "tenant": 1})]}}).to_string();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(start) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..start]);
                    let length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= start + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with(
                "POST /mongodb.shop.items/_search?allow_partial_search_results=false"
            ));
            assert!(request.contains("description_embedding"));
            assert!(!request.contains("model_id"));
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        });
        let state = SearchState {
            args: RouterVectorSearch::parse(doc! {"path": "description", "query": "rain"}).unwrap(),
            index: "mongodb.shop.items".into(),
            config: OpenSearchConfig {
                endpoints: vec!["http://127.0.0.1:0".into(), endpoint],
            },
            rows: None,
        };
        let result = fetch(&state, &mut StageContext::new());
        worker.join().unwrap();
        assert_eq!(
            result.unwrap(),
            vec![(doc! {"_id": 7i64, "tenant": 1}, 0.75)]
        );
        let unavailable = SearchState {
            config: OpenSearchConfig {
                endpoints: vec!["http://127.0.0.1:0".into()],
            },
            ..state
        };
        assert!(fetch(&unavailable, &mut StageContext::new())
            .unwrap_err()
            .to_string()
            .contains("All OpenSearch endpoints failed"));
    }
}
