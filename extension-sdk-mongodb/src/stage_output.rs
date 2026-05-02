//! Metadata-aware output for generator / source stages (`get_next`).

use bson::Document;

/// Result of a single [`crate::source_stage::SourceStage::next`](crate::source_stage::SourceStage::next) call.
///
/// Maps to [`MongoExtensionGetNextResult`](crate::sys::MongoExtensionGetNextResult): `Advanced` fills
/// `result_document` and optionally `result_metadata`; `Eof` ends the stream (empty containers, `kEOF`).
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// Emit one row; optional BSON metadata (e.g. scores) is written to `result_metadata`.
    Advanced {
        /// Primary row document.
        document: Document,
        /// Optional per-row metadata (separate from `document`).
        metadata: Option<Document>,
    },
    /// Stage is exhausted; no document is emitted.
    Eof,
}
