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
use crate::infrastructure::model_registry::{AuthMode, ModelRecord, ModelRegistry, ProviderApi};

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
fn entries_from_records(records: &[ModelRecord]) -> SourceEntries {
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
    /// Records already parsed by the caller; `load` re-parses nothing, so one
    /// resolve reads `models.json` exactly once even when the caller also
    /// derives credential status from the same parse.
    Preloaded(Result<Vec<ModelRecord>, String>),
}

impl ModelsFileCatalogueSource {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            input: ModelsFileInput::Path(base_dir.join("models.json")),
        }
    }

    /// Build the source over an already-parsed `models.json` read.
    pub fn preloaded(records: Result<Vec<ModelRecord>, String>) -> Self {
        Self {
            input: ModelsFileInput::Preloaded(records),
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
        let records = match &self.input {
            ModelsFileInput::Path(path) => {
                ModelRegistry::load_file_records(path).map_err(|error| error.to_string())?
            }
            ModelsFileInput::Preloaded(result) => result.clone()?,
        };
        Ok(entries_from_records(&records))
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
