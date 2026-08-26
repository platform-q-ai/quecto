//! Canonical provider/model catalogue domain model (epic #1193, slice 1).
//!
//! These types are intentionally free of JSON/TOML schemas, filesystem paths,
//! environment variables, HTTP clients, CLI/UDS DTOs, and TUI state.
//! Infrastructure adapters translate external catalogue formats into these
//! descriptors in later slices; this slice defines identities, descriptors,
//! capabilities, availability, immutable snapshots, and the pure merge /
//! validation / override rules over them.
//!
//! # Source-layer precedence
//!
//! Catalogue entries arrive from several source layers. Resolution applies a
//! stable-identity upsert with the documented precedence order (lowest to
//! highest):
//!
//! `BuiltIn < Generated < Discovered < Extension < UserDefined < UserOverride`
//!
//! A higher-precedence entry for the same [`ModelRef`] replaces the
//! lower-precedence entry while keeping the earlier ordering position, matching
//! the legacy registry upsert behaviour. Precedence is a property of the layer,
//! not of the order layers are handed to [`resolve_catalogue`].

use std::collections::HashMap;

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

/// Stable model identity within a provider (e.g. `gpt-5`).
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

/// Display metadata plus the transport and authentication identity of a
/// provider. Two descriptors with the same vendor metadata but different
/// [`AuthIdentity`] values are distinct provider identities.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub display_name: Option<String>,
    pub transport: TransportKind,
    pub auth: AuthIdentity,
}

impl ProviderDescriptor {
    /// Whether two descriptors name the same provider identity (id + auth).
    pub fn same_identity(&self, other: &Self) -> bool {
        self.id == other.id && self.auth == other.auth
    }
}

/// Per-token cost figures, in USD per million tokens.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Capabilities and limits of a model, replacing the heuristics currently
/// scattered across consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCapabilities {
    /// Input modalities (e.g. `text`, `image`).
    pub input_modalities: Vec<String>,
    /// Context window limit in tokens.
    pub context_window: u32,
    /// Output token limit.
    pub max_output_tokens: u32,
    /// Whether the limits were declared by the source (vs defaulted).
    pub context_window_explicit: bool,
    pub max_output_tokens_explicit: bool,
    /// Whether the model supports reasoning/effort controls.
    pub reasoning: bool,
    /// The reasoning-effort vocabulary this model accepts, in ascending
    /// order, as API string values (epic #1193, slice 6). Canonical
    /// capability metadata: every listing/selection surface projects this
    /// field instead of re-deriving a vocabulary of its own.
    pub effort_levels: Vec<String>,
    pub cost: ModelCost,
}

impl ModelCapabilities {
    /// The canonical reasoning-effort vocabulary for a model reference
    /// (`provider/model-id`, or a bare model id).
    ///
    /// This is the single domain rule that seeds `effort_levels` in
    /// catalogue metadata. Consumers holding a snapshot read the field;
    /// surfaces keyed only by an active model string (session state, spawn
    /// argument validation, open-router ids the catalogue cannot enumerate)
    /// call this same rule, so every surface speaks one vocabulary.
    pub fn effort_vocabulary_for(reference: &str) -> Vec<String> {
        crate::domain::provider::EffortLevel::levels_for_model(reference)
            .iter()
            .map(|level| level.as_str().to_string())
            .collect()
    }
}

/// Why a known model is not runnable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    MissingCredential,
    UnsupportedTransport { transport: TransportKind },
    InvalidConfiguration(String),
    PolicyDenied(String),
}

/// Availability ladder: `Known < Configured < Available < Runnable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvailabilityStatus {
    /// The catalogue knows the entry exists.
    Known,
    /// Local configuration supplies enough non-secret shape to configure it.
    Configured,
    /// A transport adapter exists and configuration is valid.
    Available,
    /// Fully runnable right now.
    Runnable,
}

/// Availability status with structured reasons for why an entry is not
/// runnable. `Runnable` carries no reasons by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    status: AvailabilityStatus,
    reasons: Vec<UnavailableReason>,
}

impl Availability {
    pub fn runnable() -> Self {
        Self {
            status: AvailabilityStatus::Runnable,
            reasons: Vec::new(),
        }
    }

    /// A non-runnable availability. Returns an error when `status` is
    /// `Runnable` (runnable entries carry no reasons) or when `reasons` is
    /// empty (non-runnable entries must say why).
    pub fn unavailable(
        status: AvailabilityStatus,
        reasons: Vec<UnavailableReason>,
    ) -> Result<Self, CatalogueDomainError> {
        if status == AvailabilityStatus::Runnable {
            return Err(CatalogueDomainError::RunnableWithReasons);
        }
        if reasons.is_empty() {
            return Err(CatalogueDomainError::UnavailableWithoutReason);
        }
        Ok(Self { status, reasons })
    }

    pub fn status(&self) -> AvailabilityStatus {
        self.status
    }

    pub fn is_runnable(&self) -> bool {
        self.status == AvailabilityStatus::Runnable
    }

    pub fn reasons(&self) -> &[UnavailableReason] {
        &self.reasons
    }
}

/// A model's descriptor within the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDescriptor {
    pub reference: ModelRef,
    pub display_name: Option<String>,
    pub capabilities: ModelCapabilities,
    pub availability: Availability,
}

/// One catalogue entry: a model together with the provider identity it runs
/// under. `model.reference.provider()` must equal `provider.id` for the entry
/// to be valid.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueEntry {
    pub provider: ProviderDescriptor,
    pub model: ModelDescriptor,
}

impl CatalogueEntry {
    pub fn reference(&self) -> &ModelRef {
        &self.model.reference
    }
}

/// Source layers in ascending precedence order. `Ord` encodes the documented
/// precedence: `BuiltIn < Generated < Discovered < Extension < UserDefined <
/// UserOverride`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceLayer {
    BuiltIn,
    Generated,
    Discovered,
    Extension,
    UserDefined,
    UserOverride,
}

/// Why an entry was rejected during resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedEntry {
    pub entry: CatalogueEntry,
    pub layer: SourceLayer,
    pub error: CatalogueDomainError,
}

/// Immutable, versioned view over the resolved catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSnapshot {
    generation: u64,
    entries: Vec<CatalogueEntry>,
}

impl CatalogueSnapshot {
    pub fn empty(generation: u64) -> Self {
        Self {
            generation,
            entries: Vec::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn entries(&self) -> &[CatalogueEntry] {
        &self.entries
    }

    pub fn find(&self, reference: &ModelRef) -> Option<&CatalogueEntry> {
        self.entries
            .iter()
            .find(|entry| entry.reference() == reference)
    }

    /// A narrowed view over this snapshot: same generation, only the entries
    /// `keep` accepts. A projection narrows the entry list, never the
    /// generation, so consumers can prove which publication they render.
    pub fn filtered(&self, keep: impl Fn(&CatalogueEntry) -> bool) -> Self {
        Self {
            generation: self.generation,
            entries: self
                .entries
                .iter()
                .filter(|entry| keep(entry))
                .cloned()
                .collect(),
        }
    }
}

/// Outcome of resolving source layers: the snapshot plus every rejected entry.
/// Invalid entries never corrupt the rest of a resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueResolution {
    pub snapshot: CatalogueSnapshot,
    pub rejected: Vec<RejectedEntry>,
}

/// Validate a single entry: the model reference must name the entry's own
/// provider, and declared limits must be non-zero.
pub fn validate_entry(entry: &CatalogueEntry) -> Result<(), CatalogueDomainError> {
    if entry.model.reference.provider() != &entry.provider.id {
        return Err(CatalogueDomainError::ProviderMismatch {
            entry_provider: entry.provider.id.as_str().to_string(),
            model_provider: entry.model.reference.provider().as_str().to_string(),
        });
    }
    if entry.model.capabilities.context_window == 0 {
        return Err(CatalogueDomainError::ZeroLimit(
            "context_window".to_string(),
        ));
    }
    if entry.model.capabilities.max_output_tokens == 0 {
        return Err(CatalogueDomainError::ZeroLimit(
            "max_output_tokens".to_string(),
        ));
    }
    Ok(())
}

/// Deterministically resolve source layers into a snapshot.
///
/// Rules (see module docs): layers are ordered by [`SourceLayer`] precedence
/// regardless of the order supplied; entries upsert by stable [`ModelRef`]
/// identity, an override keeping the overridden entry's ordering position;
/// invalid entries are recorded in `rejected` and skipped without affecting
/// any other entry. Within a single layer, a later duplicate of the same
/// reference wins (last-writer within the layer).
pub fn resolve_catalogue(
    generation: u64,
    mut layers: Vec<(SourceLayer, Vec<CatalogueEntry>)>,
) -> CatalogueResolution {
    // Precedence is a property of the layer, not the input order. The sort is
    // stable, so multiple inputs for the same layer keep their relative order
    // and last-writer-wins applies within a layer.
    layers.sort_by_key(|(layer, _)| *layer);

    let mut entries: Vec<CatalogueEntry> = Vec::new();
    let mut positions: HashMap<ModelRef, usize> = HashMap::new();
    let mut rejected: Vec<RejectedEntry> = Vec::new();

    for (layer, layer_entries) in layers {
        for entry in layer_entries {
            if let Err(error) = validate_entry(&entry) {
                rejected.push(RejectedEntry {
                    entry,
                    layer,
                    error,
                });
                continue;
            }
            match positions.get(entry.reference()) {
                Some(&position) => entries[position] = entry,
                None => {
                    positions.insert(entry.reference().clone(), entries.len());
                    entries.push(entry);
                }
            }
        }
    }

    CatalogueResolution {
        snapshot: CatalogueSnapshot {
            generation,
            entries,
        },
        rejected,
    }
}

/// Domain-level catalogue errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogueDomainError {
    EmptyProviderId,
    EmptyModelId,
    UnqualifiedModelRef(String),
    ProviderMismatch {
        entry_provider: String,
        model_provider: String,
    },
    ZeroLimit(String),
    RunnableWithReasons,
    UnavailableWithoutReason,
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
            Self::ProviderMismatch {
                entry_provider,
                model_provider,
            } => write!(
                f,
                "entry provider '{entry_provider}' does not match model provider '{model_provider}'"
            ),
            Self::ZeroLimit(field) => write!(f, "model limit '{field}' must be non-zero"),
            Self::RunnableWithReasons => {
                f.write_str("runnable availability must not carry unavailability reasons")
            }
            Self::UnavailableWithoutReason => {
                f.write_str("non-runnable availability must carry at least one reason")
            }
        }
    }
}

impl std::error::Error for CatalogueDomainError {}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
