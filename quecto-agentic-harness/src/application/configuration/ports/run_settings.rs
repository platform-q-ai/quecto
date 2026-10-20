//! The run settings an effective document yields (#1843): what a run
//! would use once the document is realized with its defaults, without the
//! application touching the `Config` schema.

/// The settings `quecto status` reports. Keys are reported by presence
/// only: no key material crosses this port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSettings {
    /// The default workspace, `~` expanded.
    pub workspace: String,
    /// The default model.
    pub model: String,
    /// The default reasoning effort, when one is configured.
    pub effort: Option<String>,
    /// Whether `providers.openai.api_key` is set.
    pub openai_key: bool,
    /// Whether `providers.anthropic.api_key` is set.
    pub anthropic_key: bool,
    /// The prefixes of the `providers.openai_compatible` endpoints whose
    /// `api_key` is set, in configured order.
    pub endpoint_keys: Vec<String>,
}

pub trait RunSettingsSource: Send + Sync {
    /// The settings `document` (an effective configuration) yields, with
    /// the schema's defaults applied; why it yields none, in the schema's
    /// words, when it does not realize.
    fn settings(&self, document: serde_json::Value) -> Result<RunSettings, String>;
}
