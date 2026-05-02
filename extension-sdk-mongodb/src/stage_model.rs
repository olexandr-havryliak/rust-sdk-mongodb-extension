//! Core **stage model** for MongoDB extension aggregation stages: planner-facing
//! [`StageProperties`], how the stage executes (**[`ExecutionModel`]**), the logical **lifecycle**
//! outline, and parse-time **[`Expansion`]** (re-exported here for a single module entry point).
//!
//! Host-facing traits ([`crate::source_stage::SourceStage`], [`crate::transform_stage::TransformStage`],
//! [`crate::blocking_stage::BlockingStage`]) keep their historical shapes; their defaults are built
//! from the same [`StagePlan`] constructors so planner BSON and execution semantics stay aligned.

pub use crate::expansion::Expansion;

use crate::stage_properties::{StagePosition, StageProperties, StreamType};

/// How the stage body runs relative to upstream rows (SDK execution model).
///
/// This aligns with [`StreamType`] in [`StageProperties`] for the defaults produced by
/// [`StagePlan`] constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionModel {
    /// Incremental execution: open a cursor, then **`next`** / per-row **`transform`** until EOF.
    Streaming,
    /// Full-input execution: **`consume`** each row, then **`finish`** once after upstream EOF.
    Blocking,
}

/// Documented lifecycle outline for tooling and authors (not passed across the FFI boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageLifecycleShape {
    /// `parse` → `open` → pull rows until EOF (`next` / `transform`).
    ParseOpenPullRows,
    /// `parse` → `open` → `consume`* → `finish`.
    ParseOpenConsumeFinish,
}

/// Single planner + execution snapshot for one exported stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StagePlan {
    /// BSON-backed planner contract (`get_properties` on the AST node).
    pub properties: StageProperties,
    /// SDK execution model (streaming vs blocking buffer).
    pub execution: ExecutionModel,
}

impl StagePlan {
    /// Default plan for a **source** stage (first in pipeline, streaming pull / generator path).
    pub const fn source_default() -> Self {
        Self {
            properties: StageProperties::source_stage_default(),
            execution: ExecutionModel::Streaming,
        }
    }

    /// Default plan for a **streaming transform** (map / passthrough path).
    pub const fn transform_streaming_default() -> Self {
        Self {
            properties: StageProperties::transform_stage_default(),
            execution: ExecutionModel::Streaming,
        }
    }

    /// Default plan for a **blocking** stage (buffer then **`finish`**).
    pub const fn blocking_default() -> Self {
        Self {
            properties: StageProperties {
                stream_type: StreamType::Blocking,
                position: StagePosition::Anywhere,
                requires_input: true,
            },
            execution: ExecutionModel::Blocking,
        }
    }

    /// Build a plan from an explicit [`StageProperties`], inferring [`ExecutionModel`] from
    /// [`StageProperties::stream_type`].
    pub fn from_planner_properties(properties: StageProperties) -> Self {
        let execution = match properties.stream_type {
            StreamType::Streaming => ExecutionModel::Streaming,
            StreamType::Blocking => ExecutionModel::Blocking,
        };
        Self {
            properties,
            execution,
        }
    }

    /// Logical lifecycle implied by [`Self::execution`].
    pub const fn lifecycle(self) -> StageLifecycleShape {
        match self.execution {
            ExecutionModel::Streaming => StageLifecycleShape::ParseOpenPullRows,
            ExecutionModel::Blocking => StageLifecycleShape::ParseOpenConsumeFinish,
        }
    }

    /// BSON for `get_properties` (same as [`StageProperties::to_document`]).
    pub fn static_properties_document(self) -> bson::Document {
        self.properties.to_document()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_self_consistent() {
        for plan in [
            StagePlan::source_default(),
            StagePlan::transform_streaming_default(),
            StagePlan::blocking_default(),
        ] {
            let inferred = StagePlan::from_planner_properties(plan.properties);
            assert_eq!(inferred.execution, plan.execution);
            assert_eq!(inferred.properties, plan.properties);
        }
    }
}
