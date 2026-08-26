//! Infrastructure adapter from the legacy `models.json` registry format into
//! the domain-owned catalogue descriptors.

use crate::application::ports::{CatalogueSource, derive_availability};
use crate::domain::catalogue::{
    AuthIdentity, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef, ProviderId,
    TransportKind,
};
use crate::infrastructure::model_registry::{AuthMode, ModelRecord, ModelRegistry, ProviderApi};

pub struct ModelRegistryCatalogueSource {
    registry: ModelRegistry,
}

impl ModelRegistryCatalogueSource {
    pub fn new(registry: ModelRegistry) -> Self {
        Self { registry }
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, String> {
        ModelRegistry::load_from_path(path)
            .map(Self::new)
            .map_err(|e| e.to_string())
    }
}

impl ModelRegistryCatalogueSource {
    pub fn load_valid_descriptors(&self) -> Result<Vec<ModelDescriptor>, String> {
        let mut descriptors = Vec::new();
        for record in self.registry.models() {
            if let Some(descriptor) = record_to_descriptor(record)? {
                descriptors.push(descriptor);
            }
        }
        Ok(descriptors)
    }
}

/// The built-in catalogue layer: the descriptors Quecto ships with.
pub struct BuiltinCatalogueSource;

impl CatalogueSource for BuiltinCatalogueSource {
    fn id(&self) -> &str {
        "builtin"
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        ModelRegistryCatalogueSource::new(ModelRegistry::builtin()).load_valid_descriptors()
    }
}

/// The user-owned catalogue layer: `models.json` entries only, without the
/// built-in layer merged underneath. Precedence between the two is resolved by
/// the application, not by this parser.
pub struct UserModelsJsonCatalogueSource {
    path: std::path::PathBuf,
}

impl UserModelsJsonCatalogueSource {
    pub fn from_base_dir(base_dir: &std::path::Path) -> Self {
        Self {
            path: base_dir.join("models.json"),
        }
    }
}

impl CatalogueSource for UserModelsJsonCatalogueSource {
    fn id(&self) -> &str {
        "models.json"
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        let registry =
            ModelRegistry::load_user_layer_from_path(&self.path).map_err(|e| e.to_string())?;
        ModelRegistryCatalogueSource::new(registry).load_valid_descriptors()
    }
}

pub fn record_to_descriptor(record: &ModelRecord) -> Result<Option<ModelDescriptor>, String> {
    record_to_descriptor_with_credential(record, None)
}

pub fn record_to_descriptor_with_credential(
    record: &ModelRecord,
    credential_available_override: Option<bool>,
) -> Result<Option<ModelDescriptor>, String> {
    if record.provider.trim().is_empty() || record.id.trim().is_empty() {
        return Ok(None);
    }

    let provider = ProviderId::new(record.provider.clone()).map_err(|e| e.to_string())?;
    let model =
        crate::domain::catalogue::ModelId::new(record.id.clone()).map_err(|e| e.to_string())?;
    let reference = ModelRef::new(provider.clone(), model);
    let transport = transport_from_provider_api(record.api);
    let auth = match record.auth {
        AuthMode::ApiKey => AuthIdentity::ApiKey,
        AuthMode::OAuth => AuthIdentity::OAuth {
            // A blank name is the same as none for the catalogue: the entry
            // declares OAuth without naming a provider. Construction reports it
            // with its provider key and file, where the user can act on it,
            // rather than aborting the whole composition here with a
            // context-free "provider id must not be empty".
            provider: record
                .oauth_provider
                .clone()
                .and_then(|provider| ProviderId::new(provider).ok()),
        },
    };
    let credential_available = credential_available_override.unwrap_or_else(|| match record.auth {
        AuthMode::ApiKey => record.api_key.as_deref().is_some_and(|k| !k.is_empty()),
        // The registry adapter must not read secrets. OAuth availability is
        // determined by credential-status application ports in the runtime path;
        // the descriptor records the auth identity and remains secret-free.
        AuthMode::OAuth => false,
    });
    let adapter_supported = !matches!(record.api, ProviderApi::GoogleGenerativeAi);
    let configured = credential_available || record.base_url.is_some();

    Ok(Some(ModelDescriptor {
        reference,
        display_name: record.display_name.clone(),
        transport,
        auth,
        base_url: record.base_url.clone(),
        auth_header: record.auth_header,
        allow_remote_http: record.allow_remote_http,
        configured,
        capabilities: ModelCapabilities {
            input: record.input.clone(),
            context_window: record.context_window,
            max_tokens: record.max_tokens,
            context_window_explicit: record.context_window_explicit,
            max_tokens_explicit: record.max_tokens_explicit,
            reasoning: record.reasoning,
            cost: ModelCost {
                input: record.cost.input,
                output: record.cost.output,
                cache_read: record.cost.cache_read,
                cache_write: record.cost.cache_write,
            },
        },
        availability: derive_availability(transport, adapter_supported, credential_available),
    }))
}

pub fn transport_from_provider_api(api: ProviderApi) -> TransportKind {
    match api {
        ProviderApi::OpenAiCompletions => TransportKind::OpenAiCompletions,
        ProviderApi::AnthropicMessages => TransportKind::AnthropicMessages,
        ProviderApi::GoogleGenerativeAi => TransportKind::GoogleGenerativeAi,
    }
}

#[cfg(test)]
#[path = "catalogue_registry_tests.rs"]
mod tests;
