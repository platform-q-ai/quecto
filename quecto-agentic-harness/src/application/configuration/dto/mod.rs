//! Boundary DTOs of the configuration capability (#1966, #2024).

pub mod config_patch;
pub mod config_read;
pub mod config_selection;
pub mod effective_config;
pub mod overlay_trust;

pub use config_patch::{
    ConfigLayer, ConfigPatch, ConfigPatchError, ConfigPatchReceipt, ConfigUnset,
};
pub use config_read::{ConfigReadError, ConfigReadRequest, ConfigReadScope, ConfigReadout};
pub use config_selection::{ConfigLayers, ConfigSelection, ConfigSelectionRequest};
pub use effective_config::{
    ConfigSources, EffectiveConfig, EffectiveConfigError, OverlayReport, OverlayState,
};
pub use overlay_trust::{OverlayTrustError, OverlayTrustRequest};
