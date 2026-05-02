//! [`extension_sdk_mongodb::transform_stage::TransformStage`] (pure Rust; no `get_mongodb_extension`).

use bson::doc;
use extension_sdk_mongodb::transform_stage::TransformStage;
use extension_sdk_mongodb::ExtensionError;
use extension_sdk_mongodb::StageContext;

struct UpperName;

impl TransformStage for UpperName {
    const NAME: &'static str = "$upperName";
    type Args = String;

    fn parse(args: bson::Document) -> extension_sdk_mongodb::ExtensionResult<Self::Args> {
        args.get_str("field")
            .map(|s| s.to_uppercase())
            .map_err(|_| ExtensionError::BadValue("missing field".into()))
    }

    fn transform(
        input: bson::Document,
        args: &Self::Args,
        ctx: &mut StageContext,
    ) -> extension_sdk_mongodb::ExtensionResult<bson::Document> {
        ctx.log_debug(1, "transform");
        let mut out = input;
        out.insert("FIELD", args.as_str());
        Ok(out)
    }
}

#[test]
fn transform_stage_parse_and_transform() {
    let args = UpperName::parse(doc! { "field": "name" }).expect("parse");
    assert_eq!(args, "NAME");
    let mut ctx = StageContext::new();
    let out = UpperName::transform(doc! { "x": 1i32 }, &args, &mut ctx).expect("transform");
    assert_eq!(out.get_str("FIELD").ok(), Some("NAME"));
}
