//! Use cases of the configuration capability. Constructed only by
//! `composition::configuration`; the interface holds injected handles.

pub mod patch_configuration;
pub mod read_configuration;
pub mod resolve_effective_config;
pub mod select_config;
pub mod trust_config_overlay;

pub use patch_configuration::PatchConfiguration;
pub use read_configuration::ReadConfiguration;
pub use resolve_effective_config::ResolveEffectiveConfig;
pub use select_config::SelectConfig;
pub use trust_config_overlay::TrustConfigOverlay;

#[cfg(test)]
pub(crate) mod fakes;
