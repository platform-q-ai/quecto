//! Outcome of the list-models use case (#1845).

use crate::application::catalogue::CatalogueSourceError;
use crate::domain::catalogue::CatalogueEntry;

/// One catalogue entry as listed: the domain entry plus whether it can run
/// right now (its availability reached the top of the ladder).
#[derive(Debug, Clone, PartialEq)]
pub struct ListedModel {
    pub entry: CatalogueEntry,
    pub runnable: bool,
}

/// A record the catalogue dropped while resolving: the domain rejected it
/// after validation, or a source could not map it at all. Surfaced so a
/// model that vanishes from the listing is never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingDiagnostic {
    pub model: String,
    pub reason: String,
}

/// The listing of one published snapshot generation.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelCatalogueListing {
    pub generation: u64,
    pub models: Vec<ListedModel>,
    pub rejected: Vec<ListingDiagnostic>,
}

/// What a list request yields. When any source failed to load altogether
/// (a malformed `models.json`, a corrupt discovery cache) the listing is
/// withheld and every failed source is reported, rather than listing a
/// catalogue the user's inputs no longer match; the resolve still published
/// the generation the healthy layers produced, and that stays current.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelListingOutcome {
    Listed(ModelCatalogueListing),
    /// Non-empty: one entry per source that failed to load, in precedence
    /// order.
    SourcesUnavailable(Vec<CatalogueSourceError>),
}
