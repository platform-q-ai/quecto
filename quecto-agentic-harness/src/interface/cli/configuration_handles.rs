//! The configuration handles the CLI holds (#1966, #2024). Declared here
//! as a plain struct of use-case handles; composition
//! (`composition::configuration`) fills it through the builder `main`
//! hands to the CLI. The interface never constructs a use case behind it.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::configuration::use_cases::{
    PatchConfiguration, ReadConfiguration, ResolveEffectiveConfig, SelectConfig, TrustConfigOverlay,
};

#[derive(Clone)]
pub struct ConfigurationHandles {
    /// Which files a run loads (#1966).
    pub select: Arc<SelectConfig>,
    /// The effective document: global plus trusted overlay (#2024).
    pub resolve: Arc<ResolveEffectiveConfig>,
    /// `quecto config get`.
    pub read: Arc<ReadConfiguration>,
    /// `quecto config set` and the tool-policy persistence hook: the one
    /// safe write path.
    pub patch: Arc<PatchConfiguration>,
    /// `quecto config trust`.
    pub trust: Arc<TrustConfigOverlay>,
}

impl std::fmt::Debug for ConfigurationHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigurationHandles")
            .finish_non_exhaustive()
    }
}

/// What composition needs to build the handles for one invocation: where
/// the trust store lives, and whether an unrecorded overlay may be offered
/// to an interactive user (only an agent run started from a terminal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationEnvironment {
    pub base_dir: PathBuf,
    pub prompt_for_trust: bool,
}
