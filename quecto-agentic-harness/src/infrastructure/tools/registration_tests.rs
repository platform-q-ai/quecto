use super::registration::ToolRegistration;
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolRestrictionReason, ToolSource};

#[test]
fn registration_builder_covers_session_spawn_unloadable_alias_and_stable_id() {
    let reg = ToolRegistration::official_native()
        .with_session_enabled(false, ToolRestrictionReason::Session)
        .with_spawn_restriction()
        .unloadable(true)
        .with_alias("alias")
        .with_stable_id("stable");
    assert!(reg.unloadable);
    assert_eq!(reg.session_enabled, Some(false));
    assert_eq!(reg.explicit_restriction, Some(ToolRestrictionReason::Spawn));
    let id = reg.identity_for_name("tool");
    assert_eq!(id.stable_id.as_ref(), "stable");
    assert_eq!(id.aliases.len(), 1);
}

#[test]
fn registration_runtime_and_uds_metadata_builders_are_distinct() {
    let uds = ToolRegistration::uds().with_provider_id("provider");
    assert_eq!(uds.source, ToolSource::Uds);
    assert_eq!(uds.provider_id.as_ref(), "provider");

    let runtime = ToolRegistration::runtime("owner");
    assert_eq!(runtime.source, ToolSource::Runtime);
    assert_eq!(runtime.owner.as_ref(), "owner");
}

#[test]
fn registration_entrypoint_and_profile_fields_are_mutable_metadata() {
    let mut reg = ToolRegistration::official_native().with_entrypoint_default_enabled(false);
    assert!(!reg.default_enabled);
    assert_eq!(
        reg.explicit_restriction,
        Some(ToolRestrictionReason::EntrypointDefault)
    );
    reg.profile_scope = Some(ProfileAvailabilityScope::Parent);
    reg.inherited_scope = Some(ProfileAvailabilityScope::Child);
    assert_eq!(reg.profile_scope, Some(ProfileAvailabilityScope::Parent));
    assert_eq!(reg.inherited_scope, Some(ProfileAvailabilityScope::Child));
}
