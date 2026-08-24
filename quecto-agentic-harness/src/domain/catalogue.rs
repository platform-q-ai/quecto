//! Canonical provider/model catalogue concepts.
//!
//! These types are intentionally free of JSON schemas, filesystem paths, HTTP
//! clients, environment lookup, CLI/UDS DTOs, and TUI state. Infrastructure
//! adapters translate external catalogue formats into these descriptors; the
//! application layer resolves, publishes, queries, refreshes, and composes from
//! snapshots of these values.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogueDomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(CatalogueDomainError::EmptyProviderId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelId(String);

impl ModelId {
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogueDomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(CatalogueDomainError::EmptyModelId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable provider/model reference used internally for selection and upserts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelRef {
    provider: ProviderId,
    model: ModelId,
}

impl ModelRef {
    pub fn new(provider: ProviderId, model: ModelId) -> Self {
        Self { provider, model }
    }

    pub fn parse(
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, CatalogueDomainError> {
        Ok(Self::new(ProviderId::new(provider)?, ModelId::new(model)?))
    }

    pub fn parse_qualified(value: &str) -> Result<Self, CatalogueDomainError> {
        let Some((provider, model)) = value.split_once('/') else {
            return Err(CatalogueDomainError::UnqualifiedModelRef(value.to_string()));
        };
        Self::parse(provider, model)
    }

    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    OpenAiCompletions,
    AnthropicMessages,
    GoogleGenerativeAi,
}

impl TransportKind {
    pub fn stable_id(self) -> &'static str {
        match self {
            Self::OpenAiCompletions => "openai-completions",
            Self::AnthropicMessages => "anthropic-messages",
            Self::GoogleGenerativeAi => "google-generative-ai",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthIdentity {
    ApiKey,
    OAuth { provider: ProviderId },
}

impl AuthIdentity {
    pub fn stable_id(&self) -> &'static str {
        match self {
            Self::ApiKey => "apiKey",
            Self::OAuth { .. } => "oauth",
        }
    }

    pub fn oauth_provider(&self) -> Option<&ProviderId> {
        match self {
            Self::ApiKey => None,
            Self::OAuth { provider } => Some(provider),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCapabilities {
    pub input: Vec<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    pub context_window_explicit: bool,
    pub max_tokens_explicit: bool,
    pub reasoning: bool,
    pub cost: ModelCost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    MissingCredential,
    UnsupportedTransport { transport: TransportKind },
    InvalidConfiguration(String),
    PolicyDenied(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Runnable,
    KnownButUnavailable { reasons: Vec<UnavailableReason> },
}

impl Availability {
    pub fn runnable(&self) -> bool {
        matches!(self, Self::Runnable)
    }

    pub fn reasons(&self) -> &[UnavailableReason] {
        match self {
            Self::Runnable => &[],
            Self::KnownButUnavailable { reasons } => reasons,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelDescriptor {
    pub reference: ModelRef,
    pub display_name: Option<String>,
    pub transport: TransportKind,
    pub auth: AuthIdentity,
    pub base_url: Option<String>,
    pub auth_header: bool,
    pub allow_remote_http: bool,
    /// Whether local configuration supplies enough non-secret shape to consider
    /// the provider configured for projections. Resolved secret values are never
    /// stored in the descriptor.
    pub configured: bool,
    pub capabilities: ModelCapabilities,
    pub availability: Availability,
}

impl ModelDescriptor {
    pub fn qualified_id(&self) -> String {
        self.reference.qualified_id()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSnapshot {
    pub generation: u64,
    models: Vec<ModelDescriptor>,
}

impl CatalogueSnapshot {
    pub fn new(generation: u64, models: Vec<ModelDescriptor>) -> Self {
        Self { generation, models }
    }

    pub fn empty(generation: u64) -> Self {
        Self {
            generation,
            models: Vec::new(),
        }
    }

    pub fn models(&self) -> &[ModelDescriptor] {
        &self.models
    }

    pub fn find(&self, reference: &ModelRef) -> Option<&ModelDescriptor> {
        self.models.iter().find(|m| &m.reference == reference)
    }

    /// Merge source layers by stable provider/model identity. Later layers have
    /// higher precedence. When an override replaces an earlier entry it keeps
    /// the earlier ordering position, matching the legacy registry upsert
    /// behaviour while making the canonical rule explicit.
    pub fn merge_layers(
        generation: u64,
        layers: impl IntoIterator<Item = Vec<ModelDescriptor>>,
    ) -> Self {
        let mut models = Vec::<ModelDescriptor>::new();
        let mut positions = HashMap::<ModelRef, usize>::new();
        for layer in layers {
            for model in layer {
                if let Some(position) = positions.get(&model.reference).copied() {
                    models[position] = model;
                } else {
                    positions.insert(model.reference.clone(), models.len());
                    models.push(model);
                }
            }
        }
        Self { generation, models }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogueDomainError {
    EmptyProviderId,
    EmptyModelId,
    UnqualifiedModelRef(String),
}

impl std::fmt::Display for CatalogueDomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyProviderId => f.write_str("provider id must not be empty"),
            Self::EmptyModelId => f.write_str("model id must not be empty"),
            Self::UnqualifiedModelRef(value) => write!(
                f,
                "model reference '{value}' is missing provider/model syntax"
            ),
        }
    }
}

impl std::error::Error for CatalogueDomainError {}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
