//! The credential-status port the resolve use case derives availability from.

use crate::domain::catalogue::entities::entry::CatalogueEntry;

/// Auth availability without exposing secret values. Judged per entry, not
/// per provider, matching the legacy per-record `configured` predicate: a key
/// declared for one model must not mark its siblings configured.
pub trait CredentialStatusPort: Send + Sync {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool;

    /// This status as of a composed runtime whose router holds `routes`,
    /// its route order (#2451): a credential the runtime turns into a
    /// provider — a configured key, a stored token, a sign-in — counts
    /// exactly when the runtime built that provider. `None` keeps this
    /// status unchanged. No default: a status that wraps or delegates must
    /// forward it, or every runtime-built credential silently reads as
    /// missing (the bug of #2451).
    fn for_composed_routes(&self, routes: &[String]) -> Option<Box<dyn CredentialStatusPort + '_>>;
}
