//! Request and report of a catalogue refresh (#1846, #1844).

use std::time::Duration;

/// Outcome-source id used when the catalogue file itself (rather than one
/// provider) cannot be enumerated.
pub const REGISTRY_FILE_SOURCE: &str = "models.json";

use crate::application::catalogue::ResolvedCatalogue;

/// Hard bounds every remote refresh runs under, so unattended refreshes can
/// never hang forever or buffer an unbounded response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshBounds {
    pub timeout: Duration,
    pub max_response_bytes: u64,
}

impl RefreshBounds {
    /// Default cap on one provider's listing response.
    pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 5 * 1024 * 1024;
}

impl Default for RefreshBounds {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            // The pre-slice-4 discovery path allowed 5 MiB bodies; keep that
            // bound so no previously discoverable listing silently fails.
            max_response_bytes: Self::DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

/// Terminal status of one source in a refresh run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefreshStatus {
    Updated {
        models: usize,
    },
    /// Nothing changed; `models` is the count the unchanged cache holds.
    Unchanged {
        models: usize,
    },
    Unsupported {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Cancelled,
}

/// One source's outcome, reported per source so a mixed run stays legible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRefreshOutcome {
    pub source: String,
    pub status: SourceRefreshStatus,
}

/// Which sources a refresh run targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshSelection {
    All,
    Only(Vec<String>),
}

/// The full result of one refresh run: per-source outcomes plus the resolve
/// result when a republish happened (`None` when nothing was republished).
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueRefreshReport {
    pub outcomes: Vec<SourceRefreshOutcome>,
    pub resolved: Option<ResolvedCatalogue>,
}
