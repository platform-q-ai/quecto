//! Infrastructure discovery adapter for the catalogue refresh port
//! (epic #1193, slice 4).
//!
//! Discovery results persist as a *source cache* in the generated-data layer
//! — never by rewriting user-owned catalogue data. Ordinary loads read only
//! the cache (network-free); network happens inside the refresh path.

use std::path::{Path, PathBuf};

use crate::application::catalogue::{CatalogueSource, SourceEntries};
use crate::domain::catalogue::SourceLayer;

/// Persisted per-provider discovery cache that doubles as a discovered-layer
/// catalogue source.
pub struct DiscoverySourceCache {
    dir: PathBuf,
    provider: String,
    id: String,
}

impl DiscoverySourceCache {
    pub fn new(dir: &Path, provider: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            provider: provider.to_string(),
            id: format!("discovered:{provider}"),
        }
    }

    /// Where this provider's cache lives on disk.
    pub fn cache_path(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.provider))
    }

    /// Map an OpenAI-compatible `/models` response body into catalogue
    /// entries and atomically persist them as this provider's source cache.
    /// Only mapped model entries are persisted — never the raw response, so
    /// credential material a server might echo can never reach disk.
    /// Returns the number of models persisted.
    pub fn store_models_response(&self, _body: &str) -> Result<usize, String> {
        // RED skeleton (#1574): parsing and atomic persistence not implemented.
        Ok(0)
    }
}

impl CatalogueSource for DiscoverySourceCache {
    fn id(&self) -> &str {
        &self.id
    }

    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }

    fn load(&self) -> Result<SourceEntries, String> {
        // RED skeleton (#1574): cache reading not implemented.
        Ok(SourceEntries::default())
    }
}

#[cfg(test)]
#[path = "catalogue_discovery_tests.rs"]
mod tests;
