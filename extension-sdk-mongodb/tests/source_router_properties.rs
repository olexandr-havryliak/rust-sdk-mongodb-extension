//! Router placement is exported through the public source-stage macro.

use bson::{doc, Document};
use extension_sdk_mongodb::stage_context::CatalogContext;
use extension_sdk_mongodb::{
    ExtensionResult, HostTypeRequirement, Next, SourceStage, StageContext, StageProperties,
};

struct RouterSource;

impl SourceStage for RouterSource {
    const NAME: &'static str = "$routerSource";
    type Args = ();
    type State = ();

    fn parse(_: Document) -> ExtensionResult<()> {
        Ok(())
    }
    fn open(_: (), _: &mut StageContext) -> ExtensionResult<()> {
        Ok(())
    }
    fn next(_: &mut (), _: &mut StageContext) -> ExtensionResult<Next> {
        Ok(Next::Eof)
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
        _: &(),
        catalog: Option<&CatalogContext>,
    ) -> ExtensionResult<Option<Vec<Document>>> {
        Ok(catalog.map(|c| {
            vec![doc! {
                "$lookup": {"from": c.collection_name.clone(), "localField": "_id",
                    "foreignField": "_id", "as": "document"}
            }]
        }))
    }
}

#[test]
fn source_macro_passes_bound_namespace_to_merging_pipeline() {
    let catalog = CatalogContext {
        database_name: "shop".into(),
        collection_name: "products".into(),
        uuid: None,
        in_router: true,
        verbosity: 0,
    };
    let pipeline = __sdk_source_merging_pipeline(doc! {}, Some(&catalog))
        .unwrap()
        .unwrap();
    assert_eq!(
        pipeline[0]
            .get_document("$lookup")
            .unwrap()
            .get_str("from")
            .unwrap(),
        "products"
    );
}

#[test]
fn source_macro_preserves_absent_catalog_for_merging_pipeline() {
    assert_eq!(__sdk_source_merging_pipeline(doc! {}, None).unwrap(), None);
}

extension_sdk_mongodb::export_source_stage!(RouterSource);

#[test]
fn source_macro_exports_router_placement_and_generator_properties() {
    assert_eq!(
        __sdk_source_static_properties(),
        doc! {
            "streamType": "streaming", "position": "first",
            "requiresInputDocSource": false, "hostType": "router",
        }
    );
}
