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
        let _ = value.into();
        unimplemented!("issue #1571 slice 1: ProviderId::new")
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
        let _ = value.into();
        unimplemented!("issue #1571 slice 1: ModelId::new")
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
        let _ = (provider.into(), model.into());
        unimplemented!("issue #1571 slice 1: ModelRef::parse")
    }

    /// Parse the qualified `provider/model` string form.
    pub fn parse_qualified(_value: &str) -> Result<Self, CatalogueDomainError> {
        unimplemented!("issue #1571 slice 1: ModelRef::parse_qualified")
    }

    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// The exact `provider/model` string used by CLI/config/UDS today.
    pub fn qualified_id(&self) -> String {
        unimplemented!("issue #1571 slice 1: ModelRef::qualified_id")
    }
}

/// Transport capability identifier. Names the wire protocol an adapter must
/// implement; carries no concrete HTTP types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    OpenAiCompletions,
    AnthropicMessages,
    GoogleGenerativeAi,
}

impl TransportKind {
    pub fn stable_id(self) -> &'static str {
        unimplemented!("issue #1571 slice 1: TransportKind::stable_id")
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
    pub fn stable_id(&self) -> &'static str {
        unimplemented!("issue #1571 slice 1: AuthIdentity::stable_id")
    }

    pub fn oauth_provider(&self) -> Option<&ProviderId> {
        unimplemented!("issue #1571 slice 1: AuthIdentity::oauth_provider")
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
        let _ = other;
        unimplemented!("issue #1571 slice 1: ProviderDescriptor::same_identity")
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
    pub cost: ModelCost,
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
        unimplemented!("issue #1571 slice 1: Availability::runnable")
    }

    /// A non-runnable availability. Returns an error when `status` is
    /// `Runnable` (runnable entries carry no reasons) or when `reasons` is
    /// empty (non-runnable entries must say why).
    pub fn unavailable(
        status: AvailabilityStatus,
        reasons: Vec<UnavailableReason>,
    ) -> Result<Self, CatalogueDomainError> {
        let _ = (status, reasons);
        unimplemented!("issue #1571 slice 1: Availability::unavailable")
    }

    pub fn status(&self) -> AvailabilityStatus {
        self.status
    }

    pub fn is_runnable(&self) -> bool {
        unimplemented!("issue #1571 slice 1: Availability::is_runnable")
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
        let _ = generation;
        unimplemented!("issue #1571 slice 1: CatalogueSnapshot::empty")
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn entries(&self) -> &[CatalogueEntry] {
        &self.entries
    }

    pub fn find(&self, reference: &ModelRef) -> Option<&CatalogueEntry> {
        let _ = reference;
        unimplemented!("issue #1571 slice 1: CatalogueSnapshot::find")
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
    let _ = entry;
    unimplemented!("issue #1571 slice 1: validate_entry")
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
    layers: Vec<(SourceLayer, Vec<CatalogueEntry>)>,
) -> CatalogueResolution {
    let _ = (generation, layers, HashMap::<ModelRef, usize>::new());
    unimplemented!("issue #1571 slice 1: resolve_catalogue")
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
