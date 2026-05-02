//! Default and overridden [`TransformStage::properties`] / [`SourceStage::properties`].

use bson::doc;

use extension_sdk_mongodb::source_stage::SourceStage;
use extension_sdk_mongodb::stage_output::Next;
use extension_sdk_mongodb::transform_stage::TransformStage;
use extension_sdk_mongodb::{
    Expansion, ExtensionError, ExtensionResult, StageContext, StagePosition, StageProperties,
    StreamType,
};

struct MapDefaultProps;

impl TransformStage for MapDefaultProps {
    const NAME: &'static str = "$mapDefaultProps";
    type Args = ();

    fn parse(args: bson::Document) -> ExtensionResult<Self::Args> {
        if args.is_empty() {
            Ok(())
        } else {
            Err(ExtensionError::BadValue("expected empty".into()))
        }
    }

    fn transform(
        input: bson::Document,
        _args: &Self::Args,
        _ctx: &mut StageContext,
    ) -> ExtensionResult<bson::Document> {
        Ok(input)
    }
}

#[test]
fn transform_stage_default_properties_match_transform_defaults() {
    let p = MapDefaultProps::properties();
    assert_eq!(p, StageProperties::transform_stage_default());
    assert_eq!(p, StageProperties::default());
    assert!(p.requires_input);
    assert_eq!(p.position, StagePosition::Anywhere);
}

#[test]
fn transform_stage_default_expand_is_self_stage() {
    let a = MapDefaultProps::parse(bson::doc! {}).unwrap();
    assert_eq!(MapDefaultProps::expand(&a), Expansion::SelfStage);
}

struct MapCustomProps;

impl TransformStage for MapCustomProps {
    const NAME: &'static str = "$mapCustomProps";
    type Args = ();

    fn parse(args: bson::Document) -> ExtensionResult<Self::Args> {
        if args.is_empty() {
            Ok(())
        } else {
            Err(ExtensionError::BadValue("expected empty".into()))
        }
    }

    fn transform(
        input: bson::Document,
        _args: &Self::Args,
        _ctx: &mut StageContext,
    ) -> ExtensionResult<bson::Document> {
        Ok(input)
    }

    fn properties() -> StageProperties {
        StageProperties {
            stream_type: StreamType::Blocking,
            position: StagePosition::First,
            requires_input: false,
        }
    }
}

#[test]
fn transform_stage_custom_properties_to_document() {
    let d = MapCustomProps::properties().to_document();
    assert_eq!(d.len(), 3);
    assert_eq!(d.get_str("streamType").unwrap(), "blocking");
    assert_eq!(d.get_str("position").unwrap(), "first");
    assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), false);
}

struct SrcDefault;

impl SourceStage for SrcDefault {
    const NAME: &'static str = "$srcDefaultProps";
    type Args = ();
    type State = ();

    fn parse(_args: bson::Document) -> ExtensionResult<Self::Args> {
        Ok(())
    }

    fn open(_args: Self::Args, _ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        Ok(())
    }

    fn next(_state: &mut Self::State, _ctx: &mut StageContext) -> ExtensionResult<Next> {
        Ok(Next::Eof)
    }
}

#[test]
fn source_stage_default_properties_are_streaming_first_requires_input() {
    let p = SrcDefault::properties();
    assert_eq!(p, StageProperties::source_stage_default());
    assert_eq!(p.position, StagePosition::First);
    assert!(p.requires_input);
    let d = p.to_document();
    assert_eq!(d.get_str("position").unwrap(), "first");
    assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), true);
}

#[test]
fn source_stage_default_expand_is_self_stage() {
    let a = SrcDefault::parse(bson::doc! {}).unwrap();
    assert_eq!(SrcDefault::expand(&a), Expansion::SelfStage);
}

struct SrcCustom;

impl SourceStage for SrcCustom {
    const NAME: &'static str = "$srcCustomProps";
    type Args = ();
    type State = ();

    fn parse(_args: bson::Document) -> ExtensionResult<Self::Args> {
        Ok(())
    }

    fn open(_args: Self::Args, _ctx: &mut StageContext) -> ExtensionResult<Self::State> {
        Ok(())
    }

    fn next(_state: &mut Self::State, _ctx: &mut StageContext) -> ExtensionResult<Next> {
        Ok(Next::Eof)
    }

    fn properties() -> StageProperties {
        StageProperties {
            stream_type: StreamType::Streaming,
            position: StagePosition::Last,
            requires_input: true,
        }
    }
}

#[test]
fn source_stage_custom_properties_round_trip_document() {
    let d = SrcCustom::properties().to_document();
    assert_eq!(d.len(), 3);
    assert_eq!(d.get_str("position").unwrap(), "last");
    assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), true);
}

/// Exercises [`TransformStage::expand`] returning [`Expansion::Pipeline`].
struct ExpandPipe;

impl TransformStage for ExpandPipe {
    const NAME: &'static str = "$expandPipe";
    type Args = i32;

    fn parse(args: bson::Document) -> ExtensionResult<Self::Args> {
        args.get_i32("k")
            .map_err(|e| ExtensionError::FailedToParse(e.to_string()))
    }

    fn transform(
        input: bson::Document,
        _args: &Self::Args,
        _ctx: &mut StageContext,
    ) -> ExtensionResult<bson::Document> {
        Ok(input)
    }

    fn expand(args: &Self::Args) -> Expansion {
        if *args == 0 {
            Expansion::SelfStage
        } else {
            Expansion::Pipeline(vec![
                doc! { "$expandPipe": { "k": 0i32 } },
                doc! { "$expandPipe": { "k": 0i32 } },
            ])
        }
    }
}

#[test]
fn transform_expand_pipeline_matches_arg_blobs_from_expand() {
    let a = ExpandPipe::parse(doc! { "k": 2i32 }).unwrap();
    let ex = ExpandPipe::expand(&a);
    let Expansion::Pipeline(docs) = ex else {
        panic!("expected Pipeline");
    };
    let blobs = Expansion::pipeline_stage_arg_blobs(ExpandPipe::NAME, &docs).unwrap();
    assert_eq!(blobs.len(), 2);
}
