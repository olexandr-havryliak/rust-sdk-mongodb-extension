//! Static stage properties exposed to the MongoDB host via `get_properties` on the AST node.
//! Prefer **[`crate::stage_model::StagePlan`]** when you want planner fields together with execution model and lifecycle.
//!
//! Field names and string values follow
//! [`extension_agg_stage_static_properties.idl`](https://github.com/mongodb/mongo/blob/v9.0/src/mongo/db/extension/public/extension_agg_stage_static_properties.idl)
//! (`MongoExtensionStaticProperties`). This SDK surface intentionally models the **core planner
//! contract** (`streamType`, `position`, `requiresInputDocSource`), with optional router placement
//! through [`StageProperties::to_document_with_host_type`]. Other IDL fields rely on the host's
//! defaults when absent from the returned document.

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

/// Host placement for an extension stage in a sharded pipeline.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostTypeRequirement {
    /// Preserve the host's default placement and the existing BSON contract.
    #[default]
    None,
    /// Execute on the router (`mongos`), not on a data shard.
    Router,
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

    /// Adds an explicit placement constraint without changing the core properties.
    pub fn to_document_with_host_type(self, host_type: HostTypeRequirement) -> Document {
        let mut document = self.to_document();
        if host_type == HostTypeRequirement::Router {
            document.insert("hostType", "router");
        }
        document
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
    fn router_placement_serializes_without_changing_core_properties() {
        let properties = StageProperties::source_stage_default();
        let mut expected = properties.to_document();
        expected.insert("hostType", "router");
        assert_eq!(
            properties.to_document_with_host_type(HostTypeRequirement::Router),
            expected
        );
    }

    #[test]
    fn unspecified_placement_preserves_existing_bson_contract() {
        let properties = StageProperties::source_stage_default();
        assert_eq!(
            properties.to_document_with_host_type(HostTypeRequirement::None),
            properties.to_document()
        );
    }

    #[test]
    fn default_properties_three_fields_and_idl_strings() {
        let d = StageProperties::default().to_document();
        assert_eq!(d, StageProperties::transform_stage_default().to_document());
        assert_eq!(d.len(), 3);
        assert_eq!(d.get_str("streamType").unwrap(), "streaming");
        assert_eq!(d.get_str("position").unwrap(), "none");
        assert!(d.get_bool("requiresInputDocSource").unwrap());
    }

    #[test]
    fn source_stage_default_is_streaming_first_requires_input() {
        let d = StageProperties::source_stage_default().to_document();
        assert_eq!(d.get_str("streamType").unwrap(), "streaming");
        assert_eq!(d.get_str("position").unwrap(), "first");
        assert!(d.get_bool("requiresInputDocSource").unwrap());
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
        assert!(!d.get_bool("requiresInputDocSource").unwrap());
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
