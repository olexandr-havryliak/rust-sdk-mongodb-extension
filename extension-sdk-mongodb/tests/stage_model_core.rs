//! Core stage model: planner properties, execution model, and alignment with trait defaults.

use extension_sdk_mongodb::stage_model::{ExecutionModel, StageLifecycleShape, StagePlan};
use extension_sdk_mongodb::stage_properties::{StagePosition, StageProperties, StreamType};

#[test]
fn source_default_plan_matches_source_stage_trait_properties() {
    let plan = StagePlan::source_default();
    assert_eq!(plan.properties, StageProperties::source_stage_default());
    assert_eq!(plan.execution, ExecutionModel::Streaming);
    assert_eq!(plan.lifecycle(), StageLifecycleShape::ParseOpenPullRows);
}

#[test]
fn transform_streaming_default_matches_transform_trait_properties() {
    let plan = StagePlan::transform_streaming_default();
    assert_eq!(plan.properties, StageProperties::transform_stage_default());
    assert_eq!(plan.execution, ExecutionModel::Streaming);
    assert_eq!(plan.lifecycle(), StageLifecycleShape::ParseOpenPullRows);
}

#[test]
fn blocking_default_plan_uses_blocking_stream_type() {
    let plan = StagePlan::blocking_default();
    assert_eq!(plan.properties.stream_type, StreamType::Blocking);
    assert_eq!(plan.execution, ExecutionModel::Blocking);
    assert_eq!(plan.lifecycle(), StageLifecycleShape::ParseOpenConsumeFinish);
    assert_eq!(plan.properties.position, StagePosition::Anywhere);
    assert!(plan.properties.requires_input);
}

#[test]
fn from_planner_properties_infers_execution_from_stream_type() {
    let p = StageProperties {
        stream_type: StreamType::Blocking,
        position: StagePosition::Last,
        requires_input: false,
    };
    let plan = StagePlan::from_planner_properties(p);
    assert_eq!(plan.execution, ExecutionModel::Blocking);
    assert_eq!(plan.properties, p);
}
