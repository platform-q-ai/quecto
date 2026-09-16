//! The catalogue capability's [`RefreshInputsLoader`] over one base
//! directory (#1846): one `models.json` parse feeds both the refreshable
//! source enumeration and the post-refresh resolve, so a concurrent edit can
//! never make the refreshed source set and the republished catalogue
//! describe two different files in one report.

use std::path::{Path, PathBuf};

use crate::application::catalogue::ports::{
    LoadedRefreshInputs, RefreshInputsLoader, RefreshRedactionPort, RefreshableCatalogueSource,
};
use crate::application::ports::{CatalogueSource, CredentialStatusPort};
use crate::infrastructure::catalogue_discovery::{
    ConfiguredDiscovery, SecretsRedaction, configured_discovery,
};
use crate::infrastructure::catalogue_inputs::CatalogueInputs;

#[derive(Debug)]
pub struct FileRefreshInputs {
    base_dir: PathBuf,
}

impl FileRefreshInputs {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }
}

impl RefreshInputsLoader for FileRefreshInputs {
    fn load(&self) -> Result<Box<dyn LoadedRefreshInputs>, String> {
        let inputs = CatalogueInputs::load(&self.base_dir);
        let discovery = match inputs.provider_defaults() {
            Ok(providers) => configured_discovery(&self.base_dir, providers),
            Err(error) => return Err(error.clone()),
        };
        let redaction = SecretsRedaction::new(discovery.secrets.clone());
        Ok(Box::new(LoadedFileRefreshInputs {
            inputs,
            discovery,
            redaction,
        }))
    }
}

struct LoadedFileRefreshInputs {
    inputs: CatalogueInputs,
    discovery: ConfiguredDiscovery,
    redaction: SecretsRedaction,
}

impl LoadedRefreshInputs for LoadedFileRefreshInputs {
    fn refreshables(&self) -> Vec<&dyn RefreshableCatalogueSource> {
        self.discovery.sources.iter().map(AsRef::as_ref).collect()
    }

    /// `inputs.sources()` already feeds the discovered layer from every
    /// persisted discovery cache (caches are read lazily, so they see this
    /// run's rewrites); live sources are appended only for providers whose
    /// cache did not exist when the inputs were enumerated (first refresh),
    /// so no provider's models are fed in twice. Precedence is layer-based,
    /// so the user layers win regardless of input order.
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        let cached = self.inputs.discovered_providers();
        let mut sources = self.inputs.sources();
        sources.extend(
            self.discovery
                .sources
                .iter()
                .filter(|s| !cached.contains(&s.id()))
                .map(|s| s.as_ref() as &dyn CatalogueSource),
        );
        sources
    }

    fn credentials(&self) -> &dyn CredentialStatusPort {
        &self.inputs.credentials
    }

    fn redaction(&self) -> &dyn RefreshRedactionPort {
        &self.redaction
    }
}
