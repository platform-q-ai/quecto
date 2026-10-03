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

#[path = "catalogue/effort_vocabulary.rs"]
mod effort_vocabulary;
pub use effort_vocabulary::EffortVocabulary;
#[path = "catalogue/retired.rs"]
mod retired;
pub use retired::retired_builtin;

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
    /// How the provider bounds the prompt inside the window (#2405).
    pub prompt_limit: PromptLimit,
}

/// How a provider bounds the prompt inside a model's window (#2405).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptLimit {
    /// The provider checks the prompt plus the requested `max_tokens`
    /// against the window (Anthropic, generic OpenAI-compatible endpoints,
    /// local servers).
    #[default]
    SharedWithRequest,
    /// The provider fixes the input limit at the window less the declared
    /// output cap (OpenAI Responses and Codex: 400k = 272k + 128k).
    WindowLessOutputCap,
}

/// The percent of a fixed input limit kept free (#2405 review L5): the
/// estimate can run under the provider's count between calibrations, and a
/// prompt over a fixed input limit is refused outright. Codex keeps the
/// same margin (`effective_context_window_percent: 95` in openai/codex).
pub const FIXED_INPUT_HEADROOM_PERCENT: usize = 5;

/// Where the provider shares the window with the request, the prompt keeps
/// at least `1 / PROMPT_FLOOR_DIVISOR` of it whatever the reply reserves
/// (#2405 review M1).
const PROMPT_FLOOR_DIVISOR: usize = 2;

/// A model's declared context window and what its reply needs beside the
/// prompt (#2405): the one rule the context ceiling is computed from.
///
/// The prompt's room is the window less the reply's reserve, which depends
/// on how the provider bounds the prompt ([`PromptLimit`]). Under a fixed
/// input limit it is that limit less the headroom, never lifted past it.
/// Where the window is shared with the request it is never less than half
/// the window: a reserve past that is clamped, so the room never collapses
/// when a declared cap nearly fills the window. Either way the room never
/// falls as the window grows, and a reserve leaving the prompt under half
/// the window is reported ([`Self::reserve_clamped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelWindow {
    /// The declared window in tokens, `None` when the model declares none.
    pub window: Option<usize>,
    /// How the provider bounds the prompt inside the window.
    pub prompt_limit: PromptLimit,
    /// The model's declared output cap, `None` when it declares none.
    pub output_cap: Option<usize>,
    /// The output limit a request asks for (`max_tokens` clamped to the cap).
    pub requested_output: usize,
}

impl ModelWindow {
    pub fn new(
        window: Option<usize>,
        prompt_limit: PromptLimit,
        output_cap: Option<usize>,
        requested_output: usize,
    ) -> Self {
        Self {
            window,
            prompt_limit,
            output_cap,
            requested_output,
        }
    }

    /// The room the window leaves the prompt; `None` when unknown.
    pub fn prompt_room(self) -> Option<usize> {
        let window = self.window?;
        let unfloored = self.unfloored_room(window);
        if self.fixed_input() {
            // The provider's own limit: the floor never lifts it (#2405
            // final review L1).
            debug_assert!(
                unfloored <= window.saturating_sub(self.reply_reserve()),
                "a fixed input ceiling stays within the provider's limit"
            );
            return Some(unfloored);
        }
        let floor = window / PROMPT_FLOOR_DIVISOR;
        let room = unfloored.max(floor);
        debug_assert!(
            floor <= room && room <= window,
            "a shared prompt keeps half the window, and no more than the window"
        );
        Some(room)
    }

    /// Whether the reply's reserve leaves the prompt under half the window:
    /// clamped where the window is shared, kept as the provider sets it
    /// under a fixed input limit; either way the declaration deserves a
    /// warning.
    pub fn reserve_clamped(self) -> bool {
        self.window
            .is_some_and(|window| self.unfloored_room(window) < window / PROMPT_FLOOR_DIVISOR)
    }

    /// Whether the provider fixes the input limit at the window less a
    /// declared output cap; without a declared cap that limit is unknown,
    /// and the reply reserves what a request asks for, as elsewhere.
    fn fixed_input(self) -> bool {
        self.prompt_limit == PromptLimit::WindowLessOutputCap && self.output_cap.is_some()
    }

    /// What the reply needs beside the prompt: the declared cap under a
    /// fixed input limit; elsewhere what a request can ask for, the
    /// effective limit or, after an output-limit cut-off, up to twice it
    /// within the cap (#2124).
    fn reply_reserve(self) -> usize {
        let requested = self.requested_output;
        match self.output_cap {
            Some(cap) if self.fixed_input() => cap,
            Some(cap) => cap.min(requested.saturating_mul(2)).max(requested),
            None => requested,
        }
    }

    /// The window less the reply's reserve (and, under a fixed input limit,
    /// its headroom), before the floor.
    fn unfloored_room(self, window: usize) -> usize {
        let room = window.saturating_sub(self.reply_reserve());
        if self.fixed_input() {
            // Exact in u128, and never more than `room`.
            let headroom = room as u128 * FIXED_INPUT_HEADROOM_PERCENT as u128 / 100;
            room - usize::try_from(headroom).unwrap_or(room)
        } else {
            room
        }
    }

    /// The context ceiling: the lower of the `configured` budget
    /// (`max_context_tokens`) and the prompt's room. An unknown window
    /// leaves the configured budget.
    pub fn ceiling(self, configured: usize) -> usize {
        self.prompt_room()
            .map_or(configured, |room| configured.min(room))
    }

    /// The configured budget clamped to the whole window, prompt and reply
    /// together (the room a raised output limit may use, #2124).
    pub fn budget(self, configured: usize) -> usize {
        self.window
            .map_or(configured, |window| configured.min(window))
    }
}

/// Why a known model is not runnable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableReason {
    MissingCredential,
    UnsupportedTransport {
        transport: TransportKind,
    },
    InvalidConfiguration(String),
    PolicyDenied(String),
    /// The provider refused the model for the account or auth mode in use
    /// (#2435), with the provider's own words: learned from a definitive
    /// refusal, it holds until it expires or the provider serves the model.
    RefusedForAccount(String),
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
#[cfg(test)]
#[path = "catalogue_window_tests.rs"]
mod window_tests;
