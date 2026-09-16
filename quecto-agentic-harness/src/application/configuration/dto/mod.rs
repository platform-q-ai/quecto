//! Boundary DTOs of the configuration capability (#1966).

pub mod config_selection;

pub use config_selection::{
    ConfigSelection, ConfigSelectionError, ConfigSelectionRequest, LocalConfigRejection,
};
