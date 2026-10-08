use std::collections::VecDeque;

use bson::{doc, Document};
use extension_sdk_mongodb::{
    error::ExtensionError, stage_context::CatalogContext, ExtensionResult, HostTypeRequirement,
    Next, SourceStage, StageContext, StageProperties,
};

struct RouterLookup;

impl SourceStage for RouterLookup {
    const NAME: &'static str = "$routerLookupPoc";
    type Args = Vec<Document>;
    type State = VecDeque<Document>;

    fn parse(args: Document) -> ExtensionResult<Self::Args> {
        if args.len() != 1 {
            return Err(ExtensionError::BadValue(
                "expected candidates only; namespace comes from bind".into(),
            ));
        }
        let rows = args
            .get_array("candidates")
            .map_err(|e| ExtensionError::BadValue(e.to_string()))?;
        rows.iter()
            .map(|row| {
                let row = row.as_document().ok_or_else(|| {
                    ExtensionError::BadValue("candidate must be a document".into())
                })?;
                if !row.contains_key("_id") {
                    return Err(ExtensionError::BadValue("candidate needs _id".into()));
                }
                match row.get("score") {
                    Some(bson::Bson::Double(score)) if score.is_finite() => {}
                    Some(bson::Bson::Int32(_) | bson::Bson::Int64(_)) => {}
                    _ => {
                        return Err(ExtensionError::BadValue(
                            "candidate needs a finite numeric score".into(),
                        ))
                    }
                }
                Ok(row.clone())
            })
            .collect()
    }

    fn open(args: Self::Args, ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        if !ctx.catalog().is_some_and(|c| c.in_router) {
            return Err(ExtensionError::Runtime("PoC must execute on mongos".into()));
        }
        Ok(args.into())
    }

    fn next(state: &mut Self::State, _: &mut StageContext) -> ExtensionResult<Next> {
        Ok(match state.pop_front() {
            Some(document) => Next::Advanced {
                document,
                metadata: None,
            },
            None => Next::Eof,
        })
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
        _: &Self::Args,
        catalog: Option<&CatalogContext>,
    ) -> ExtensionResult<Option<Vec<Document>>> {
        let c = catalog
            .filter(|c| c.in_router)
            .ok_or_else(|| ExtensionError::Runtime("PoC needs a bound mongos namespace".into()))?;
        Ok(Some(vec![
            doc! {"$lookup": {"from": c.collection_name.clone(), "localField": "_id",
            "foreignField": "_id", "as": "__pocDocument"}},
            doc! {"$unwind": "$__pocDocument"},
            doc! {"$replaceWith": {"$mergeObjects": ["$__pocDocument", {"score": "$score"}]}},
        ]))
    }
}

extension_sdk_mongodb::export_source_stage!(RouterLookup);

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> CatalogContext {
        CatalogContext {
            database_name: "shop".into(),
            collection_name: "products".into(),
            uuid: None,
            in_router: true,
            verbosity: 0,
        }
    }

    #[test]
    fn candidates_preserve_id_types_scores_and_order() {
        let rows = vec![
            doc! {"_id": 2, "score": 0.9},
            doc! {"_id": -1, "score": 0.7},
        ];
        assert_eq!(
            RouterLookup::parse(doc! {"candidates": rows.clone()}).unwrap(),
            rows
        );
    }

    #[test]
    fn lookup_uses_bound_namespace_without_client_from() {
        let pipeline = RouterLookup::merging_pipeline(&Vec::new(), Some(&catalog()))
            .unwrap()
            .unwrap();
        assert_eq!(
            pipeline[0],
            doc! {"$lookup": {
                "from": "products", "localField": "_id", "foreignField": "_id", "as": "__pocDocument"
            }}
        );
        assert_eq!(pipeline[1], doc! {"$unwind": "$__pocDocument"});
        assert_eq!(
            pipeline[2],
            doc! {"$replaceWith": {"$mergeObjects": ["$__pocDocument", {"score": "$score"}]}}
        );
    }

    #[test]
    fn missing_or_shard_catalog_is_rejected() {
        assert!(RouterLookup::merging_pipeline(&Vec::new(), None).is_err());
        let mut c = catalog();
        c.in_router = false;
        assert!(RouterLookup::merging_pipeline(&Vec::new(), Some(&c)).is_err());
    }

    #[test]
    fn invalid_candidates_are_rejected() {
        for args in [
            doc! {},
            doc! {"candidates": "bad"},
            doc! {"candidates": [1]},
            doc! {"candidates": [{"_id": 1}]},
            doc! {"candidates": [{"score": 0.5}]},
            doc! {"candidates": [{"_id": 1, "score": "bad"}]},
        ] {
            assert!(RouterLookup::parse(args).is_err());
        }
    }

    #[test]
    fn generator_preserves_sequence_then_repeats_eof() {
        let mut state = VecDeque::from([
            doc! {"_id": 2, "score": 0.9},
            doc! {"_id": -1, "score": 0.7},
        ]);
        let mut ctx = StageContext::new();
        for id in [2, -1] {
            let Next::Advanced { document, .. } = RouterLookup::next(&mut state, &mut ctx).unwrap()
            else {
                panic!("expected candidate")
            };
            assert_eq!(document.get_i32("_id").unwrap(), id);
        }
        for _ in 0..2 {
            assert!(matches!(
                RouterLookup::next(&mut state, &mut ctx).unwrap(),
                Next::Eof
            ));
        }
    }
}
