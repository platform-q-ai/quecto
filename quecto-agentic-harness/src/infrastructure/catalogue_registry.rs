//! Infrastructure adapters feeding the application catalogue (epic #1193,
//! slice 2).
//!
//! Implements the application's `CatalogueSource` and `CredentialStatusPort`
//! ports over the existing data: the built-in registry table and the current
//! `models.json` contents. Parsing stays here; the application layer only sees
//! domain catalogue entries. One process-wide snapshot store exists per base
//! directory so every read surface for that directory shares the published
//! generation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::application::ports::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SkippedRecord, SourceEntries,
};
use crate::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost as DomainModelCost,
    ModelDescriptor, ModelRef, ProviderDescriptor, ProviderId, SourceLayer, TransportKind,
};
use crate::infrastructure::model_registry::{
    AuthMode, DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_OUTPUT_TOKENS, ModelOverride, ModelRecord,
    ModelRegistry, ProviderApi, SkippedProviderBlock, UnsupportedProviderConfig,
    resolve_registry_value,
};

/// Map one legacy registry record into a domain catalogue entry. Availability
/// starts as runnable; the resolve use case derives the real status from the
/// credential port.
fn entry_from_record(record: &ModelRecord) -> Result<CatalogueEntry, String> {
    let provider_id = ProviderId::new(record.provider.clone()).map_err(|e| e.to_string())?;
    let reference =
        ModelRef::parse(record.provider.clone(), record.id.clone()).map_err(|e| e.to_string())?;
    let transport = match record.api {
        ProviderApi::OpenAiCompletions => TransportKind::OpenAiCompletions,
        ProviderApi::AnthropicMessages => TransportKind::AnthropicMessages,
        ProviderApi::GoogleGenerativeAi => TransportKind::GoogleGenerativeAi,
    };
    let auth = match record.auth {
        AuthMode::ApiKey => AuthIdentity::ApiKey,
        AuthMode::OAuth => AuthIdentity::OAuth {
            provider: record
                .oauth_provider
                .as_deref()
                .map(ProviderId::new)
                .transpose()
                .map_err(|e| e.to_string())?,
        },
    };
    Ok(CatalogueEntry {
        provider: ProviderDescriptor {
            id: provider_id,
            display_name: Some(record.provider.clone()),
            transport,
            auth,
        },
        model: ModelDescriptor {
            reference,
            display_name: record.display_name.clone(),
            capabilities: ModelCapabilities {
                input_modalities: record.input.clone(),
                context_window: record.context_window,
                max_output_tokens: record.max_tokens,
                context_window_explicit: record.context_window_explicit,
                max_output_tokens_explicit: record.max_tokens_explicit,
                reasoning: record.reasoning,
                effort_levels: ModelCapabilities::effort_vocabulary_for(&format!(
                    "{}/{}",
                    record.provider, record.id
                )),
                cost: DomainModelCost {
                    input: record.cost.input,
                    output: record.cost.output,
                    cache_read: record.cost.cache_read,
                    cache_write: record.cost.cache_write,
                },
            },
            availability: Availability::runnable(),
        },
    })
}

/// Map records into entries, keeping the layer alive when individual records
/// are unmappable: one invalid record must not erase its valid neighbours, so
/// failures become per-record `skipped` diagnostics instead of a layer error.
pub(crate) fn entries_from_records(records: &[ModelRecord]) -> SourceEntries {
    let mut loaded = SourceEntries::default();
    for record in records {
        match entry_from_record(record) {
            Ok(entry) => loaded.entries.push(entry),
            Err(error) => loaded.skipped.push(SkippedRecord {
                record: format!("{}/{}", record.provider, record.id),
                error,
            }),
        }
    }
    loaded
}

/// Map an unsupported-transport provider block into catalogue entries: its
/// models are known (listed) but can never become runnable, because the
/// declared transport has no adapter in this build (#1575, AC3). The resolve
/// use case derives the structured unsupported-transport reason from the
/// transport kind.
pub(crate) fn entries_for_unsupported_provider(
    config: &UnsupportedProviderConfig,
) -> SourceEntries {
    let mut loaded = SourceEntries::default();
    for (model_id, name) in &config.models {
        match unsupported_entry(config, model_id, name.as_deref()) {
            Ok(entry) => loaded.entries.push(entry),
            Err(error) => loaded.skipped.push(SkippedRecord {
                record: format!("{}/{}", config.provider, model_id),
                error,
            }),
        }
    }
    loaded
}

/// One unsupported-transport model as a catalogue entry (known but never
/// runnable). Shared by the user-file layer and the override layer, so an
/// override can patch a known-but-unrunnable entry (#1581 review).
fn unsupported_entry(
    config: &UnsupportedProviderConfig,
    model_id: &str,
    name: Option<&str>,
) -> Result<CatalogueEntry, String> {
    let provider_id = ProviderId::new(config.provider.clone()).map_err(|e| e.to_string())?;
    let reference = ModelRef::parse(config.provider.clone(), model_id.to_string())
        .map_err(|e| e.to_string())?;
    Ok(CatalogueEntry {
        provider: ProviderDescriptor {
            id: provider_id,
            display_name: Some(config.provider.clone()),
            transport: TransportKind::Unsupported {
                declared: config.declared_transport.clone(),
            },
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference,
            display_name: name.map(str::to_string),
            capabilities: default_capabilities(&format!("{}/{model_id}", config.provider)),
            availability: Availability::runnable(),
        },
    })
}

/// The synthesized capability defaults matching a `models.json` entry that
/// declares only an id (nothing explicit, so nothing clamps). The numbers
/// come from the registry's shared constants so every layer synthesizes the
/// same defaults (#1581 review).
pub fn default_capabilities(reference: &str) -> ModelCapabilities {
    ModelCapabilities {
        effort_levels: ModelCapabilities::effort_vocabulary_for(reference),
        input_modalities: vec!["text".to_string()],
        context_window: DEFAULT_CONTEXT_WINDOW,
        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
        context_window_explicit: false,
        max_output_tokens_explicit: false,
        reasoning: false,
        cost: DomainModelCost::default(),
    }
}

/// The built-in registry table as the `BuiltIn` source layer.
pub struct BuiltinCatalogueSource;

impl CatalogueSource for BuiltinCatalogueSource {
    fn id(&self) -> &str {
        "builtin"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(entries_from_records(ModelRegistry::builtin().models()))
    }
}

/// The user's `models.json` as the `UserDefined` source layer. Parsing stays
/// in [`ModelRegistry::load_file_records`]; a missing file is an empty layer.
pub struct ModelsFileCatalogueSource {
    input: ModelsFileInput,
}

enum ModelsFileInput {
    /// Read and parse the file at load time.
    Path(PathBuf),
    /// Entries already mapped by the caller from one `models.json` parse;
    /// `load` re-parses nothing, so one resolve reads `models.json` exactly
    /// once even when the caller also derives credential status from the same
    /// parse.
    Preloaded(Result<SourceEntries, String>),
}

impl ModelsFileCatalogueSource {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            input: ModelsFileInput::Path(base_dir.join("models.json")),
        }
    }

    /// Build the source over entries mapped from an already-parsed
    /// `models.json` read (listed records plus unsupported-transport blocks).
    pub fn preloaded(entries: Result<SourceEntries, String>) -> Self {
        Self {
            input: ModelsFileInput::Preloaded(entries),
        }
    }
}

impl CatalogueSource for ModelsFileCatalogueSource {
    fn id(&self) -> &str {
        "models.json"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::UserDefined
    }
    fn load(&self) -> Result<SourceEntries, String> {
        match &self.input {
            ModelsFileInput::Path(path) => {
                let config =
                    ModelRegistry::load_registry_config(path).map_err(|error| error.to_string())?;
                // This source is the UserDefined layer only: the file's
                // `overrides` section is a separate UserOverride layer,
                // applied by [`apply_overrides`] against built-in and
                // discovered patch targets this standalone source cannot
                // see. Compositions build both layers from one parse (see
                // `CatalogueInputs::load`).
                Ok(user_file_entries(
                    &config.records,
                    &config.unsupported,
                    &config.skipped,
                ))
            }
            ModelsFileInput::Preloaded(result) => result.clone(),
        }
    }
}

/// Map one `models.json` parse into the `UserDefined` layer's entries: the
/// listed records plus known-but-unrunnable entries for unsupported-transport
/// provider blocks.
pub(crate) fn user_file_entries(
    records: &[ModelRecord],
    unsupported: &[UnsupportedProviderConfig],
    skipped_blocks: &[SkippedProviderBlock],
) -> SourceEntries {
    let mut loaded = entries_from_records(records);
    for block in unsupported {
        let mut mapped = entries_for_unsupported_provider(block);
        loaded.entries.append(&mut mapped.entries);
        loaded.skipped.append(&mut mapped.skipped);
    }
    // Provider blocks the parse skipped (e.g. an unknown auth mode) surface
    // as per-record diagnostics so the degradation is never silent.
    for block in skipped_blocks {
        loaded.skipped.push(SkippedRecord {
            record: block.provider.clone(),
            error: block.error.clone(),
        });
    }
    loaded
}

/// The user's `overrides` section as the `UserOverride` source layer:
/// patched full entries built by the interface bridge from the same
/// `models.json` parse, plus per-override diagnostics (#1575, AC1/AC5).
pub struct UserOverrideCatalogueSource {
    entries: Result<SourceEntries, String>,
}

impl UserOverrideCatalogueSource {
    /// A failed parse is carried as the layer's error so the resolve
    /// retains the layer's last-good entries instead of publishing an empty
    /// override layer.
    pub fn preloaded(entries: Result<SourceEntries, String>) -> Self {
        Self { entries }
    }
}

impl CatalogueSource for UserOverrideCatalogueSource {
    fn id(&self) -> &str {
        "models.json overrides"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::UserOverride
    }
    fn load(&self) -> Result<SourceEntries, String> {
        self.entries.clone()
    }
}

/// Credential status derived from the parsed registry configuration: a model
/// counts as credentialed exactly when the legacy per-record `configured`
/// flag was true for it (a non-empty resolved API key or an explicit base
/// URL on that record). Keyed per qualified model, not per provider, so a
/// key declared for one model never marks its builtin siblings configured.
/// Only booleans leave this adapter — key material never reaches the
/// application layer or a snapshot.
pub struct RegistryCredentialStatus {
    configured: HashSet<String>,
}

impl RegistryCredentialStatus {
    pub fn from_records<'a>(records: impl IntoIterator<Item = &'a ModelRecord>) -> Self {
        let mut configured = HashSet::new();
        for record in records {
            let has_key = record.api_key.as_deref().is_some_and(|k| !k.is_empty());
            if has_key || record.base_url.is_some() {
                configured.insert(format!("{}/{}", record.provider, record.id));
            }
        }
        Self { configured }
    }
}

impl CredentialStatusPort for RegistryCredentialStatus {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool {
        self.configured.contains(&entry.reference().qualified_id())
    }
}

/// The outcome of applying the user's `overrides` section to its base
/// records and entries.
pub(crate) struct AppliedOverrides {
    /// Full patched records for override targets that have a routable base
    /// record (user file, discovered, built-in), so overridden connection or
    /// credential references are also routable and credential-checked.
    pub(crate) records: Vec<ModelRecord>,
    /// Patched entries for override targets that are known-but-unrunnable
    /// unsupported-transport declarations (no `ModelRecord` exists for
    /// them, but they are published catalogue entries and stay patchable).
    pub(crate) unsupported_entries: Vec<CatalogueEntry>,
    /// Per-override diagnostics for overrides that could not apply.
    pub(crate) skipped: Vec<SkippedRecord>,
}

/// Apply the user's stable-ID `overrides` to their base entries (#1575,
/// AC1): the base is the effective entry beneath the override layer (user
/// file, then discovered, then built-in, including unsupported-transport
/// declarations), and only declared fields change. Every override that
/// cannot apply becomes a per-record diagnostic instead of failing the
/// layer: an unknown target, a literal secret — catalogue files carry
/// credential *references* (`$ENV`), never key material (AC5) — or a
/// reference to an unset/empty environment variable, which must never
/// silently clobber a working credential with an empty key (#1581 review).
pub(crate) fn apply_overrides(
    overrides: &[(String, ModelOverride)],
    file_records: &[ModelRecord],
    discovered_records: &[ModelRecord],
    builtin: &ModelRegistry,
    unsupported: &[UnsupportedProviderConfig],
) -> AppliedOverrides {
    let mut applied = AppliedOverrides {
        records: Vec::new(),
        unsupported_entries: Vec::new(),
        skipped: Vec::new(),
    };
    for (qualified, patch) in overrides {
        let mut reject = |error: String| {
            applied.skipped.push(SkippedRecord {
                record: qualified.clone(),
                error,
            });
        };
        let Some((provider, id)) = qualified.split_once('/') else {
            reject(format!(
                "override key '{qualified}' must be a qualified provider/model id"
            ));
            continue;
        };
        let mut resolved_key = None;
        if let Some(reference) = patch.api_key.as_deref() {
            if !reference.starts_with('$') {
                reject(format!(
                    "override for '{qualified}' declares a literal apiKey; catalogue files accept only a credential reference like \"$MY_KEY\", never literal secrets"
                ));
                continue;
            }
            let value = resolve_registry_value(reference, |name| std::env::var(name).ok());
            if value.is_empty() {
                reject(format!(
                    "override for '{qualified}' references credential '{reference}' but that environment variable is unset or empty; the base credential was kept"
                ));
                continue;
            }
            resolved_key = Some(value);
        }
        let base = file_records
            .iter()
            .chain(discovered_records)
            .find(|r| r.provider == provider && r.id == id)
            .or_else(|| builtin.find(provider, id));
        if let Some(base) = base {
            let mut record = base.clone();
            if let Some(name) = &patch.name {
                record.display_name = Some(name.clone());
            }
            if let Some(window) = patch.context_window {
                record.context_window = window;
                record.context_window_explicit = true;
            }
            if let Some(cap) = patch.max_tokens {
                record.max_tokens = cap;
                record.max_tokens_explicit = true;
            }
            if let Some(key) = resolved_key {
                record.api_key = Some(key);
            }
            applied.records.push(record);
            continue;
        }
        // An unsupported-transport declaration has no ModelRecord, but it is
        // a known published entry and must stay patchable by stable ID
        // (#1581 review): metadata fields apply to the entry directly. A
        // credential reference is validated but carried nowhere — the entry
        // can never become runnable in this build.
        let unsupported_base = unsupported.iter().find_map(|config| {
            (config.provider == provider)
                .then(|| {
                    config
                        .models
                        .iter()
                        .find(|(model_id, _)| model_id == id)
                        .map(|(model_id, name)| (config, model_id.as_str(), name.clone()))
                })
                .flatten()
        });
        if let Some((config, model_id, name)) = unsupported_base {
            match unsupported_entry(config, model_id, name.as_deref()) {
                Ok(mut entry) => {
                    if let Some(name) = &patch.name {
                        entry.model.display_name = Some(name.clone());
                    }
                    if let Some(window) = patch.context_window {
                        entry.model.capabilities.context_window = window;
                        entry.model.capabilities.context_window_explicit = true;
                    }
                    if let Some(cap) = patch.max_tokens {
                        entry.model.capabilities.max_output_tokens = cap;
                        entry.model.capabilities.max_output_tokens_explicit = true;
                    }
                    applied.unsupported_entries.push(entry);
                }
                Err(error) => reject(error),
            }
            continue;
        }
        reject(format!(
            "override target '{qualified}' does not match any known model"
        ));
    }
    applied
}

/// The process-wide snapshot store for one base directory. Every read surface
/// (CLI listing, UDS queries, TUI projection via UDS) for that directory
/// shares this store, so they all render the same published generation.
pub fn snapshot_store_for(base_dir: &Path) -> CatalogueSnapshotStore {
    static STORES: OnceLock<Mutex<HashMap<PathBuf, CatalogueSnapshotStore>>> = OnceLock::new();
    let stores = STORES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut stores = stores
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    stores
        .entry(base_dir.to_path_buf())
        .or_insert_with(CatalogueSnapshotStore::empty)
        .clone()
}

/// The process-wide published runtime store for one base directory: every
/// entry point composing or selecting for that directory shares the same
/// published runtime generation, paired with [`snapshot_store_for`]'s
/// catalogue store.
pub fn runtime_store_for(base_dir: &Path) -> crate::application::ports::RuntimeSnapshotStore {
    use crate::application::ports::RuntimeSnapshotStore;
    static STORES: OnceLock<Mutex<HashMap<PathBuf, RuntimeSnapshotStore>>> = OnceLock::new();
    let stores = STORES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut stores = stores
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    stores.entry(base_dir.to_path_buf()).or_default().clone()
}

#[cfg(test)]
#[path = "catalogue_registry_tests.rs"]
mod tests;
