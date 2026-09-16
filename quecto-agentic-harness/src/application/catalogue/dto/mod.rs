//! Boundary DTOs of the catalogue capability. They carry domain values
//! (entries, references, capabilities) — never a wire shape or a
//! persistence record; presenters render them.

pub mod model_listing;

pub use model_listing::{
    ListedModel, ListingDiagnostic, ModelCatalogueListing, ModelListingOutcome,
};
