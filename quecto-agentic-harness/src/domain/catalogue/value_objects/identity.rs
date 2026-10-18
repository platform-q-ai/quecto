//! Stable provider and model identities, and the transport and
//! authentication identity a provider runs under (epic #1193, slice 1).

use super::error::CatalogueDomainError;

/// Stable provider identity. Serialized form is the exact string used by
/// CLI/config/UDS today (e.g. `openai-api`).
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

/// Stable model identity within a provider (e.g. `gpt-6.1-sol`).
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

/// Typed replacement for the `provider/model` strings used across the CLI,
/// config, and UDS surfaces. `qualified_id` round-trips byte-for-byte with the
/// existing string form.
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
        Ok(Self {
            provider: ProviderId::new(provider)?,
            model: ModelId::new(model)?,
        })
    }

    /// Parse the qualified `provider/model` string form. Only the first `/`
    /// separates provider from model, so model ids may themselves contain `/`.
    pub fn parse_qualified(value: &str) -> Result<Self, CatalogueDomainError> {
        let (provider, model) = value
            .split_once('/')
            .ok_or_else(|| CatalogueDomainError::UnqualifiedModelRef(value.to_string()))?;
        Self::parse(provider, model)
    }

    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// The exact `provider/model` string used by CLI/config/UDS today.
    pub fn qualified_id(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// Transport capability identifier. Names the wire protocol an adapter must
/// implement; carries no concrete HTTP types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TransportKind {
    OpenAiCompletions,
    AnthropicMessages,
    GoogleGenerativeAi,
    /// A transport a catalogue file declared but no adapter in this build
    /// implements. Carries the declared name so a known-but-unrunnable entry
    /// can say exactly which transport it is waiting on (#1575, AC3).
    Unsupported {
        declared: String,
    },
}

impl TransportKind {
    /// The stable wire identifier for this transport, as written in
    /// catalogue files and rendered by read surfaces.
    pub fn stable_id(&self) -> &str {
        match self {
            Self::OpenAiCompletions => "openai-completions",
            Self::AnthropicMessages => "anthropic-messages",
            Self::GoogleGenerativeAi => "google-generative-ai",
            Self::Unsupported { declared } => declared,
        }
    }
}

/// Authentication identity as a property of provider identity. API-key and
/// OAuth identities stay distinct even when they share vendor model metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuthIdentity {
    ApiKey,
    /// An OAuth identity. `provider` is the credential provider the entry
    /// names; `None` when the entry declares OAuth without naming one, which is
    /// a misconfiguration consumers must be able to see rather than infer.
    OAuth {
        provider: Option<ProviderId>,
    },
}

impl AuthIdentity {
    pub fn oauth_provider(&self) -> Option<&ProviderId> {
        match self {
            Self::ApiKey => None,
            Self::OAuth { provider } => provider.as_ref(),
        }
    }
}
