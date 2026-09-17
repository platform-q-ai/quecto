//! The configuration schema as a port (#2024): the use cases patch and
//! merge plain JSON documents; whether the result is a configuration the
//! harness accepts is the schema owner's call.

use std::path::Path;

/// Resolves a document's file-relative references and validates a document
/// against the configuration schema. Implemented by infrastructure over the
/// `Config` struct and its load-time validations; faked in use-case tests.
pub trait ConfigValidator: Send + Sync {
    /// Resolve references that are relative to the file the document was
    /// read from (workflow step `ref`s) and reject keys the schema retired,
    /// so the document can be merged with others regardless of where each
    /// one lived. `path` is the document's own location.
    fn resolve(
        &self,
        document: serde_json::Value,
        path: &Path,
    ) -> Result<serde_json::Value, String>;

    /// Whether `document` round-trips through the configuration schema and
    /// every load-time validation; the error names what does not.
    fn validate(&self, document: &serde_json::Value) -> Result<(), String>;
}
