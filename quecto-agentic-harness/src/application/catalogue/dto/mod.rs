//! Boundary DTOs of the catalogue capability. They carry domain values
//! (entries, references, capabilities) — never a wire shape or a
//! persistence record; presenters render them.

pub mod active_model;
pub mod effort;
pub mod model_listing;

pub use active_model::{ModelLimits, ModelSelectionVerdict, ModelSwitchPlan, ModelSwitched};
pub use effort::{EffortChangeError, EffortChangeOutcome, EffortChangeRequest};
pub use model_listing::{
    ListedModel, ListingDiagnostic, ModelCatalogueListing, ModelListingOutcome,
};
