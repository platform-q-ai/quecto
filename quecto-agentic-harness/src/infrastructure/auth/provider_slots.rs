//! The provider slots credentials turn into runtime providers (#2451).
//!
//! The runtime factory builds its credential-backed providers by the two
//! slot rules here:
//!
//! - An API-key slot (`openai-api`, `anthropic-api`) is built with the
//!   configured key, else an unexpired stored token credential of its
//!   vendor.
//! - An OAuth slot (`openai-oauth`, `anthropic-oauth`, and every OAuth
//!   provider such as `xai`) is built with its vendor's stored OAuth
//!   credential when its token is non-empty, expired or not: an expired
//!   token is refreshed lazily on the first 401 (#811).
//!
//! The catalogue's credential status does not re-derive them: it reads
//! which slots the composed runtime actually built ([`ProviderSlots`], the
//! router's route order), so the model selector offers exactly what the
//! runtime can route and the two can never disagree. Only provider names
//! cross into the catalogue; key material stays with the factory.

use std::collections::BTreeSet;

use super::credential_store::{AuthMethod, Credential};
use crate::application::catalogue::ports::RuntimeSnapshotSource;
use crate::domain::catalogue::AuthIdentity;

/// The built-in slots the runtime builds straight from a vendor credential
/// or a configured key, by the names the factory gives them: the factory
/// names its providers with these constants, so a slot it builds is always
/// one the catalogue credits.
pub(crate) const OPENAI_API: &str = "openai-api";
pub(crate) const OPENAI_OAUTH: &str = "openai-oauth";
pub(crate) const ANTHROPIC_API: &str = "anthropic-api";
pub(crate) const ANTHROPIC_OAUTH: &str = "anthropic-oauth";
const DEDICATED_SLOTS: [&str; 4] = [OPENAI_API, OPENAI_OAUTH, ANTHROPIC_API, ANTHROPIC_OAUTH];

/// The API key a vendor's API-key slot is built with: the configured key,
/// else the stored token credential while it is unexpired. `None` builds
/// no slot.
pub(crate) fn api_slot_key(configured: &str, stored: Option<Credential>) -> Option<String> {
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    stored
        .filter(|credential| credential.method == AuthMethod::Token && !credential.is_expired())
        .map(|credential| credential.token)
        .filter(|token| !token.is_empty())
}

/// The token an OAuth slot is built with: the stored OAuth credential's
/// token when non-empty, expired or not (refreshed lazily, #811). `None`
/// builds no slot.
pub(crate) fn oauth_slot_token(stored: Option<Credential>) -> Option<String> {
    stored
        .filter(|credential| credential.method == AuthMethod::OAuth)
        .map(|credential| credential.token)
        .filter(|token| !token.is_empty())
}

/// The provider names a composed runtime routes, lowercased: which
/// credential-backed slots it built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderSlots {
    routes: BTreeSet<String>,
}

impl ProviderSlots {
    /// No runtime composed: nothing is routed, so no credential-backed slot
    /// is either.
    pub fn none() -> Self {
        Self::default()
    }

    /// The slots a router holding `routes` (its route order) built.
    pub fn routed(routes: impl IntoIterator<Item = String>) -> Self {
        let routes: BTreeSet<String> = routes
            .into_iter()
            .map(|route| route.trim().to_ascii_lowercase())
            .collect();
        debug_assert!(
            routes.iter().all(|route| !route.is_empty()),
            "a router names every provider it routes"
        );
        Self { routes }
    }

    /// The slots `runtime`'s current generation built, or [`Self::none`]
    /// before one is composed.
    pub fn of_runtime(runtime: &dyn RuntimeSnapshotSource) -> Self {
        match runtime.current_runtime() {
            Some(runtime) => Self::routed(runtime.provider.route_order()),
            None => Self::none(),
        }
    }

    /// Whether the runtime built a credential-backed provider for
    /// `provider` authenticating as `auth`. Only a dedicated slot or an
    /// OAuth provider is credential-backed: any other provider is
    /// credentialed by its own records alone, so a vendor sign-in or a
    /// sibling's key never credits it.
    pub fn credits(&self, provider: &str, auth: &AuthIdentity) -> bool {
        let provider = provider.trim().to_ascii_lowercase();
        let credential_backed = match auth {
            AuthIdentity::OAuth { .. } => true,
            AuthIdentity::ApiKey => DEDICATED_SLOTS.contains(&provider.as_str()),
        };
        credential_backed && self.routes.contains(&provider)
    }
}

#[cfg(test)]
#[path = "provider_slots_tests.rs"]
mod tests;
