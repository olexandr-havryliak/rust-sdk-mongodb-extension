//! Pipeline **expansion**: one parsed stage may lower to multiple planner stages (`expand` on the
//! parse node). The host receives an expanded array of AST or parse nodes. See
//! **[`crate::stage_model`]** for how expansion fits the overall stage plan.

use bson::Document;

use crate::error::{ExtensionError, Result};

/// Result of [`SourceStage::expand`](crate::source_stage::SourceStage::expand) /
/// [`TransformStage::expand`](crate::transform_stage::TransformStage::expand) /
/// [`BlockingStage::expand`](crate::blocking_stage::BlockingStage::expand).
#[derive(Debug, Clone, PartialEq)]
pub enum Expansion {
    /// Keep this stage as a single AST node (no expansion).
    SelfStage,
    /// Replace with a linear pipeline of stages. Each document must be
    /// **`{ stageName: innerArgs }`** where **`stageName`** matches the exporting stage’s
    /// [`NAME`](crate::source_stage::SourceStage::NAME) (same encoding as a normal parse node’s
    /// inner object).
    Pipeline(Vec<Document>),
    /// Replace this stage with an extension-owned candidate source stage followed by a host-owned
    /// `$_internalSearchIdLookup` AST node.
    WithHostIdLookup {
        /// Full stage document for the extension-owned candidate source stage.
        extension_stage: Document,
        /// Full stage document for `$_internalSearchIdLookup`.
        id_lookup: Document,
    },
}

impl Expansion {
    /// Full stage documents in execution order, useful for tests and diagnostics.
    pub fn stage_documents(&self) -> Vec<Document> {
        match self {
            Expansion::SelfStage => Vec::new(),
            Expansion::Pipeline(stages) => stages.clone(),
            Expansion::WithHostIdLookup {
                extension_stage,
                id_lookup,
            } => vec![extension_stage.clone(), id_lookup.clone()],
        }
    }

    /// Validates that each stage document has exactly one key equal to **`stage_name`** and returns
    /// serialized inner argument blobs (same wire shape as stored on the parse node).
    pub fn pipeline_stage_arg_blobs(
        stage_name: &str,
        pipeline: &[Document],
    ) -> Result<Vec<Vec<u8>>> {
        if pipeline.is_empty() {
            return Err(ExtensionError::BadValue(
                "expanded pipeline must not be empty".into(),
            ));
        }
        let mut out = Vec::with_capacity(pipeline.len());
        for d in pipeline {
            let mut keys = d.keys();
            let k = keys
                .next()
                .ok_or_else(|| ExtensionError::BadValue("empty stage document".into()))?;
            if keys.next().is_some() {
                return Err(ExtensionError::BadValue(
                    "stage document must have exactly one field".into(),
                ));
            }
            if k != stage_name {
                return Err(ExtensionError::BadValue(format!(
                    "expanded pipeline stages must use operator {stage_name}, got {k}"
                )));
            }
            let inner = d
                .get_document(k)
                .map_err(|e| ExtensionError::BadValue(e.to_string()))?;
            let mut w = Vec::new();
            inner
                .to_writer(&mut w)
                .map_err(|e| ExtensionError::FailedToParse(e.to_string()))?;
            out.push(w);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn pipeline_stage_arg_blobs_two_stages() {
        let blobs = Expansion::pipeline_stage_arg_blobs(
            "$x",
            &[doc! { "$x": { "a": 1i32 } }, doc! { "$x": { "b": 2i32 } }],
        )
        .unwrap();
        assert_eq!(blobs.len(), 2);
        assert_ne!(blobs[0], blobs[1]);
    }

    #[test]
    fn pipeline_stage_arg_blobs_rejects_wrong_operator() {
        let e = Expansion::pipeline_stage_arg_blobs("$x", &[doc! { "$y": {} }]).unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn pipeline_stage_arg_blobs_rejects_empty_pipeline() {
        let e = Expansion::pipeline_stage_arg_blobs("$x", &[]).unwrap_err();
        assert!(matches!(e, ExtensionError::BadValue(_)));
    }

    #[test]
    fn mongot_candidates_with_id_lookup_expansion_keeps_candidate_stage_first() {
        let expansion = Expansion::WithHostIdLookup {
            extension_stage: doc! { "$x": { "query": "boots" } },
            id_lookup: doc! { "$_internalSearchIdLookup": {} },
        };
        let stages = expansion.stage_documents();
        assert_eq!(stages[0], doc! { "$x": { "query": "boots" } });
        assert_eq!(stages[1], doc! { "$_internalSearchIdLookup": {} });
    }
}
