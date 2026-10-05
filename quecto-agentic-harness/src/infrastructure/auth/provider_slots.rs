//! Which provider slots the credential sources let the runtime build
//! (#2451): the one statement of the rules provider-runtime composition
//! builds its credential-backed providers by, shared with the catalogue's
//! credential status so the model selector offers exactly the models the
//! runtime can route.
//!
//! - An API-key slot (`openai-api`, `anthropic-api`) is built with the
//!   configured key, else an unexpired stored token credential of its
//!   vendor.
//! - An OAuth slot (`openai-oauth`, `anthropic-oauth`, and every OAuth
//!   provider such as `xai`) is built with its vendor's stored OAuth
//!   credential when its token is non-empty, expired or not: an expired
//!   token is refreshed lazily on the first 401 (#811).
//!
//! Key material stays here and in the runtime factory: [`ProviderSlots`]
//! holds only which slots and vendors are buildable.

use std::collections::{BTreeSet, HashMap};

use super::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::domain::catalogue::AuthIdentity;

/// The built-in slots a vendor credential builds directly, by slot name,
/// vendor (the credential store key) and auth mode.
const DEDICATED_SLOTS: [(&str, &str, AuthMethod); 4] = [
    ("openai-api", "openai", AuthMethod::Token),
    ("openai-oauth", "openai", AuthMethod::OAuth),
    ("anthropic-api", "anthropic", AuthMethod::Token),
    ("anthropic-oauth", "anthropic", AuthMethod::OAuth),
];

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

/// The configured API keys composition reads (`providers.<vendor>.api_key`,
/// environment overrides applied). Secret material: never rendered, so no
/// `Debug`.
#[derive(Clone, Copy, Default)]
pub struct ConfiguredApiKeys<'a> {
    pub openai: &'a str,
    pub anthropic: &'a str,
}

impl<'a> ConfiguredApiKeys<'a> {
    /// No configured key: the credential store alone decides.
    pub const NONE: ConfiguredApiKeys<'static> = ConfiguredApiKeys {
        openai: "",
        anthropic: "",
    };

    pub fn of(config: &'a crate::infrastructure::config::Config) -> Self {
        Self {
            openai: &config.providers.openai.api_key,
            anthropic: &config.providers.anthropic.api_key,
        }
    }

    fn for_vendor(&self, vendor: &str) -> &'a str {
        match vendor {
            "openai" => self.openai,
            "anthropic" => self.anthropic,
            _ => "",
        }
    }
}

/// The credential-backed provider slots the runtime builds, as booleans
/// only. Either probed from the configured keys and the credential store
/// (what a composition would build now) or read off a composed runtime's
/// routes (what it built).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSlots {
    Probed {
        /// Dedicated slot names the credentials build.
        dedicated: BTreeSet<&'static str>,
        /// Kernel OAuth vendors with a usable stored OAuth token.
        oauth_vendors: BTreeSet<String>,
    },
    /// The provider names a composed runtime routes, lowercased.
    Routed(BTreeSet<String>),
}

impl ProviderSlots {
    /// What a composition would build now from `keys` and `store`. A store
    /// that cannot be read or parsed builds no stored-credential slot, as in
    /// the runtime; the failure is logged once per process.
    pub fn probe(keys: ConfiguredApiKeys<'_>, store: &CredentialStore) -> Self {
        let stored = match store.load_snapshot() {
            Ok(stored) => stored,
            Err(error) => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| {
                    tracing::warn!(
                        %error,
                        path = %store.path().display(),
                        "credential store unreadable: models that need a stored credential are listed as missing one"
                    );
                });
                HashMap::new()
            }
        };
        Self::from_stored(keys, &stored)
    }

    fn from_stored(keys: ConfiguredApiKeys<'_>, stored: &HashMap<String, Credential>) -> Self {
        let credential = |vendor: &str| stored.get(vendor).cloned();
        let dedicated = DEDICATED_SLOTS
            .iter()
            .filter(|(_, vendor, method)| match method {
                AuthMethod::Token => {
                    api_slot_key(keys.for_vendor(vendor), credential(vendor)).is_some()
                }
                AuthMethod::OAuth => oauth_slot_token(credential(vendor)).is_some(),
            })
            .map(|(slot, _, _)| *slot)
            .collect();
        let oauth_vendors = stored
            .keys()
            .filter(|vendor| oauth_slot_token(credential(vendor)).is_some())
            .filter(|vendor| super::oauth::OAuthConfig::for_provider(vendor).is_some())
            .cloned()
            .collect();
        Self::Probed {
            dedicated,
            oauth_vendors,
        }
    }

    /// The slots a composed runtime routes, from its route order.
    pub fn routed(routes: impl IntoIterator<Item = String>) -> Self {
        Self::Routed(
            routes
                .into_iter()
                .map(|route| route.trim().to_ascii_lowercase())
                .collect(),
        )
    }

    /// Whether the runtime has a credential-backed provider for `provider`
    /// authenticating as `auth`. Only a dedicated slot or an OAuth provider
    /// is credential-backed: any other provider is credentialed by its own
    /// record alone, so a vendor sign-in never credits it.
    pub fn credits(&self, provider: &str, auth: &AuthIdentity) -> bool {
        let provider = provider.trim().to_ascii_lowercase();
        let dedicated = DEDICATED_SLOTS
            .iter()
            .find(|(slot, _, _)| *slot == provider)
            .map(|(slot, _, _)| *slot);
        match (self, dedicated, auth) {
            (
                Self::Probed {
                    dedicated: built, ..
                },
                Some(slot),
                _,
            ) => built.contains(slot),
            (
                Self::Probed { oauth_vendors, .. },
                None,
                AuthIdentity::OAuth { provider: vendor },
            ) => vendor
                .as_ref()
                .is_some_and(|vendor| oauth_vendors.contains(vendor.as_str())),
            (Self::Routed(routes), Some(_), _)
            | (Self::Routed(routes), None, AuthIdentity::OAuth { .. }) => {
                routes.contains(&provider)
            }
            (_, None, AuthIdentity::ApiKey) => false,
        }
    }
}

#[cfg(test)]
#[path = "provider_slots_tests.rs"]
mod tests;
