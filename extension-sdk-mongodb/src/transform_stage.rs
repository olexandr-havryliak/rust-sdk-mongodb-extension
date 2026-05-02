//! Trait-based **transform** stages (per-document map with typed arguments and [`StageContext`](crate::stage_context::StageContext)).
//!
//! Export with [`export_transform_stage_type!`](crate::export_transform_stage_type) (wraps the map-transform FFI path).

use bson::Document;

use crate::error::Result;
use crate::expansion::Expansion;
use crate::stage_context::StageContext;
use crate::stage_model::StagePlan;
use crate::stage_properties::StageProperties;

/// Implement this trait for a transform stage, then export it with [`export_transform_stage_type!`](crate::export_transform_stage_type).
pub trait TransformStage: Sized + Send + 'static {
    /// Stage key including leading `$`, must match BSON (`{ Self::NAME: <args> }`).
    const NAME: &'static str;
    /// Parsed inner object for `{ Self::NAME: <args> }`.
    type Args;

    /// Validates and decodes stage arguments.
    fn parse(args: Document) -> Result<Self::Args>;

    /// Maps one upstream document to one output document.
    fn transform(input: Document, args: &Self::Args, ctx: &mut StageContext) -> Result<Document>;

    /// Lower this stage to itself or to a linear pipeline (parse-time expansion).
    ///
    /// Default: no expansion ([`Expansion::SelfStage`]).
    fn expand(_args: &Self::Args) -> Expansion {
        Expansion::SelfStage
    }

    /// Static planner properties (`MongoExtensionStaticProperties` on the host).
    ///
    /// Default: [`StagePlan::transform_streaming_default`](crate::stage_model::StagePlan::transform_streaming_default).
    fn properties() -> StageProperties {
        StagePlan::transform_streaming_default().properties
    }
}
