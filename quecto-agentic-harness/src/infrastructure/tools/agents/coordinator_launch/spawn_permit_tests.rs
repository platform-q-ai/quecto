//! S4c: the spawn permit follows the registry's policy over `spawn`,
//! as a parent's own call is judged.
use std::sync::Arc;

use super::SpawnPermit;
use crate::application::tools::ports::Tool;
use crate::domain::shared::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::tools::registry::ToolRegistryImpl;

/// A tool by `name` that does nothing.
struct Named(&'static str);

impl Tool for Named {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.0.into(),
            description: "a test tool".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        Box::pin(async { Err(DomainError::Tool("never run".into())) })
    }
}

#[test]
fn the_permit_follows_the_registrys_policy_over_spawn() {
    let permit = SpawnPermit::default();
    assert!(!permit.allowed(), "closed until a registry says otherwise");
    let mut registry = ToolRegistryImpl::new();
    registry.observe_policy(std::sync::Arc::new(permit.clone()));
    assert!(!permit.allowed(), "no spawn tool, no spawn");
    registry.register(Arc::new(Named("spawn")));
    assert!(permit.allowed(), "a registered, enabled spawn");
    registry.disable_tool("spawn");
    assert!(!permit.allowed(), "disabled at runtime");
    registry.enable_tool("spawn");
    assert!(permit.allowed(), "enabled again");
    registry.disable_tool_by_entrypoint_default("spawn");
    assert!(!permit.allowed(), "off by the entrypoint's default");
    let other = SpawnPermit::default();
    let mut unrelated = ToolRegistryImpl::new();
    unrelated.register(Arc::new(Named("read")));
    unrelated.observe_policy(std::sync::Arc::new(other.clone()));
    assert!(!other.allowed(), "another tool is no spawn");
}

/// A persisted policy (the user's saved preference) narrowing spawn to
/// children closes the permit, and lifting it opens it again.
#[test]
fn the_permit_follows_a_persisted_policy() {
    use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
    use crate::infrastructure::config::{ToolPolicyConfig, ToolPolicyEntryConfig};
    use crate::infrastructure::tools::registry::registration::ToolRegistration;
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Named("spawn")));
    registry.observe_policy(std::sync::Arc::new(permit.clone()));
    assert!(permit.allowed());
    let stable_id = ToolRegistration::official_native()
        .identity_for_name("spawn")
        .stable_id
        .to_string();
    let mut policy = ToolPolicyConfig::default();
    policy.entries.insert(
        stable_id,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Child,
        },
    );
    assert!(registry.apply_persisted_tool_policy(&policy).is_empty());
    assert!(!permit.allowed(), "a child-only spawn is no parent's");
    registry.apply_persisted_tool_policy(&ToolPolicyConfig::default());
    assert!(permit.allowed(), "the preference lifted");
}

/// A policy inherited from a parent session that keeps spawn from this
/// one closes the permit; one that grants it leaves it open.
#[test]
fn the_permit_follows_an_inherited_policy() {
    use std::collections::BTreeMap;

    use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
    use crate::infrastructure::tools::agents::inherited_tool_policy::InheritedToolPolicySnapshot;
    let inherited = |scope| {
        let permit = SpawnPermit::default();
        let mut registry = ToolRegistryImpl::new();
        registry.register(Arc::new(Named("spawn")));
        registry.observe_policy(std::sync::Arc::new(permit.clone()));
        let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([("spawn".into(), scope)]));
        registry.apply_inherited_tool_policy_snapshot(&snapshot);
        permit.allowed()
    };
    assert!(!inherited(ProfileAvailabilityScope::None));
    assert!(!inherited(ProfileAvailabilityScope::Child));
    assert!(inherited(ProfileAvailabilityScope::Both));
}

/// A guard that refuses every spawn.
struct NoSpawn;

impl crate::application::tools::ports::ToolGuard for NoSpawn {
    fn check(&self, tool_name: &str, _: &str) -> Result<(), String> {
        match tool_name {
            "spawn" => Err("spawn is guarded".into()),
            _ => Ok(()),
        }
    }
}

/// The permit carries the registry's guards, those registered before it
/// was watched and after, and judges a spawn call by them.
#[test]
fn the_permit_judges_a_spawn_by_the_registrys_guards() {
    let permit = SpawnPermit::default();
    assert_eq!(permit.check("{}"), Ok(()), "no guards, no refusal");
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Named("spawn")));
    registry.observe_policy(std::sync::Arc::new(permit.clone()));
    assert_eq!(permit.check("{}"), Ok(()));
    registry.register_guard(Arc::new(NoSpawn));
    assert_eq!(permit.check("{}"), Err("spawn is guarded".into()));
    let early = SpawnPermit::default();
    let mut guarded = ToolRegistryImpl::new();
    guarded.register_guard(Arc::new(NoSpawn));
    guarded.observe_policy(std::sync::Arc::new(early.clone()));
    assert_eq!(early.check("{}"), Err("spawn is guarded".into()));
}

/// Review round 3: a recorded persisted policy result changes the
/// configured scope, and the permit follows it.
#[test]
fn the_permit_follows_a_recorded_policy_result() {
    use crate::application::tools::ports::ToolPolicyMutator;
    use crate::domain::tool_policy::value_objects::tool::{
        ToolPolicyApplyMode, ToolPolicyMutationResult, ToolPolicyMutationStatus,
        ToolPolicyReconciliation,
    };
    use crate::domain::tool_policy::value_objects::tool_descriptor::{
        ProfileAvailabilityScope, ToolAvailability,
    };
    let permit = SpawnPermit::default();
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Named("spawn")));
    registry.observe_policy(Arc::new(permit.clone()));
    assert!(permit.allowed());
    let recorded = |scope| ToolPolicyReconciliation {
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![ToolPolicyMutationResult {
            name: "spawn".into(),
            requested_identifier: None,
            requested_availability: ToolAvailability::Enabled,
            requested_scope: scope,
            status: ToolPolicyMutationStatus::Applied,
            before: None,
            after: None,
            reason: "test".into(),
        }],
        correlation_id: None,
    };
    registry.record_persisted_tool_policy_results(&recorded(ProfileAvailabilityScope::Child));
    assert!(!permit.allowed(), "a child-only spawn is no parent's");
    registry.record_persisted_tool_policy_results(&recorded(ProfileAvailabilityScope::Both));
    assert!(permit.allowed());
}
