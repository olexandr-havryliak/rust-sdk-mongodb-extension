//! Static stage properties exposed to the MongoDB host via `get_properties` on the AST node.
//! Prefer **[`crate::stage_model::StagePlan`]** when you want planner fields together with execution model and lifecycle.
//!
//! Field names and string values follow
//! [`extension_agg_stage_static_properties.idl`](https://github.com/mongodb/mongo/blob/v8.3/src/mongo/db/extension/public/extension_agg_stage_static_properties.idl)
//! (`MongoExtensionStaticProperties`). This SDK surface intentionally models the **core planner
//! contract** (`streamType`, `position`, `requiresInputDocSource`); other IDL fields rely on the
//! host’s defaults when absent from the returned document.

use bson::doc;
use bson::Document;

/// Whether the stage is treated as streaming or blocking for planning (`streamType` in host BSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamType {
    /// `"streaming"` — yields rows incrementally.
    Streaming,
    /// `"blocking"` — must consume input before producing output.
    Blocking,
}

impl StreamType {
    fn as_idl_str(self) -> &'static str {
        match self {
            StreamType::Streaming => "streaming",
            StreamType::Blocking => "blocking",
        }
    }
}

/// Pipeline position constraint (`position` in host BSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StagePosition {
    /// `"none"` — no special placement requirement (IDL `kNone`).
    Anywhere,
    /// `"first"` — must be the first stage.
    First,
    /// `"last"` — must be the last stage.
    Last,
}

impl StagePosition {
    fn as_idl_str(self) -> &'static str {
        match self {
            StagePosition::Anywhere => "none",
            StagePosition::First => "first",
            StagePosition::Last => "last",
        }
    }
}

/// Planner-facing static properties for an aggregation stage extension (core contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StageProperties {
    /// Host `streamType`: streaming vs blocking for planner behavior.
    pub stream_type: StreamType,
    /// Host `position`: required placement in the pipeline.
    pub position: StagePosition,
    /// Host `requiresInputDocSource` — whether the stage consumes an upstream document source.
    pub requires_input: bool,
}

impl StageProperties {
    /// Default planner shape for a **transform** stage: streaming, no fixed position, consumes a
    /// document source.
    pub const fn transform_stage_default() -> Self {
        Self {
            stream_type: StreamType::Streaming,
            position: StagePosition::Anywhere,
            requires_input: true,
        }
    }

    /// Default planner shape for a **source** stage: streaming, **first** (must be first in the
    /// pipeline), consumes a document source when present (runtime may still use the generator path
    /// on empty scan).
    pub const fn source_stage_default() -> Self {
        Self {
            stream_type: StreamType::Streaming,
            position: StagePosition::First,
            requires_input: true,
        }
    }

    /// BSON document for `MongoExtensionAggStageAstNodeVTable::get_properties`.
    ///
    /// Uses camelCase keys for the fields modeled here; other static properties use host defaults.
    pub fn to_document(self) -> Document {
        doc! {
            "streamType": self.stream_type.as_idl_str(),
            "position": self.position.as_idl_str(),
            "requiresInputDocSource": self.requires_input,
        }
    }
}

impl Default for StageProperties {
    /// Same as [`StageProperties::transform_stage_default`].
    fn default() -> Self {
        Self::transform_stage_default()
    }
}

/// Default static properties for map / passthrough transforms (`requiresInputDocSource: true`).
pub fn default_map_stage_static_properties() -> Document {
    StageProperties::default().to_document()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_properties_three_fields_and_idl_strings() {
        let d = StageProperties::default().to_document();
        assert_eq!(d, StageProperties::transform_stage_default().to_document());
        assert_eq!(d.len(), 3);
        assert_eq!(d.get_str("streamType").unwrap(), "streaming");
        assert_eq!(d.get_str("position").unwrap(), "none");
        assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), true);
    }

    #[test]
    fn source_stage_default_is_streaming_first_requires_input() {
        let d = StageProperties::source_stage_default().to_document();
        assert_eq!(d.get_str("streamType").unwrap(), "streaming");
        assert_eq!(d.get_str("position").unwrap(), "first");
        assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), true);
    }

    #[test]
    fn custom_properties_serialize_three_fields() {
        let p = StageProperties {
            stream_type: StreamType::Blocking,
            position: StagePosition::First,
            requires_input: false,
        };
        let d = p.to_document();
        assert_eq!(d.len(), 3);
        assert_eq!(d.get_str("streamType").unwrap(), "blocking");
        assert_eq!(d.get_str("position").unwrap(), "first");
        assert_eq!(d.get_bool("requiresInputDocSource").unwrap(), false);
    }

    #[test]
    fn last_position_serializes() {
        let p = StageProperties {
            position: StagePosition::Last,
            ..StageProperties::default()
        };
        assert_eq!(p.to_document().get_str("position").unwrap(), "last");
    }

    #[test]
    fn anywhere_maps_to_none_string() {
        assert_eq!(StagePosition::Anywhere.as_idl_str(), "none");
    }
}
