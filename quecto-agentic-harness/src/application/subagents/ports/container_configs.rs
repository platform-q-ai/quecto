//! Container config selection at launch (#2024 S4a, S4e): the effective
//! configs and the standard bundle's verdict on a host-side script.

/// The container configs in effect for a launch, read from the
/// configuration capability's *effective* configuration: for the launching
/// agent, its base file with its checkout's trusted overlay merged in
/// (entry-wise; an overlay default un-defaults the global entries); for an
/// explicit file, that file alone. An untrusted or refused overlay is not
/// applied and is reported in the set's diagnostics, never prompted for.
/// Implemented by infrastructure over the configuration adapters;
/// composition binds the launching agent's checkout.
pub trait EffectiveContainerConfigs: Send + Sync {
    fn effective_container_configs(
        &self,
        source: &crate::application::subagents::dto::container_config::ContainerConfigSource,
    ) -> Result<
        crate::application::subagents::dto::container_config::EffectiveContainerConfigSet,
        crate::application::subagents::dto::container_config::ContainerConfigsError,
    >;
}

/// The standard bundle's verdict on a host-side script a container config
/// names (#2024 S4e): the launch policy asks it for every script of the
/// selected entry and refuses the launch when a standard asset no longer
/// carries the embedded bytes. Implemented by infrastructure over the
/// embedded bundle; a script outside the bundle's layout is
/// `NotStandard` and the configuration's trust alone vouches for it.
pub trait ContainerScriptIntegrity: Send + Sync {
    fn verify(
        &self,
        script: &std::path::Path,
    ) -> crate::application::subagents::dto::standard_script::StandardScriptVerdict;
}
