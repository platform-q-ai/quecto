use std::collections::BTreeMap;

use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::domain::tool_policy::inherited_child_policy_from_catalogue;
use crate::infrastructure::tools::inherited_tool_policy::{
    entrypoint_only_tools, is_entrypoint_only, recorded_scope,
};
use crate::infrastructure::tools::registration::ToolRegistration;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

impl ToolRegistryImpl {
    pub(crate) fn apply_inherited_tool_policy_snapshot(
        &mut self,
        snapshot: &crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        self.inherited_policy_scopes = snapshot
            .tools
            .iter()
            .map(|(name, scope)| (name.clone(), *scope))
            .collect();
        self.inherited_policy_default_scope = Some(ProfileAvailabilityScope::None);
        for (policy_id, scope) in &snapshot.tools {
            let Some(name) = self.resolve_tool_policy_id(policy_id).ok().or_else(|| {
                self.tools
                    .contains_key(policy_id)
                    .then(|| policy_id.clone())
            }) else {
                // #2446: an extension's tool registers later, not a typo.
                match crate::domain::tool_policy::registers_after_startup(policy_id) {
                    true => {}
                    false => warnings.push(policy_id.clone()),
                }
                continue;
            };
            let metadata = self
                .metadata
                .entry(name)
                .or_insert_with(ToolRegistration::official_native);
            metadata.inherited_scope = Some(*scope);
            metadata.profile_scope = Some(*scope);
            metadata.profile_enabled = Some(scope.is_enabled());
        }
        for (name, metadata) in self.metadata.iter_mut() {
            let identity = metadata.identity_for_name(name);
            // Recorded tools took their scope above. #2216: an unrecorded
            // entrypoint-only tool was never built by the parent, not denied
            // to it, so the child's own policy governs it. Every other
            // unrecorded tool is closed.
            if recorded_scope(&snapshot.tools, name, &identity).is_some()
                || is_entrypoint_only(name, metadata)
            {
                continue;
            }
            metadata.inherited_scope = Some(ProfileAvailabilityScope::None);
            metadata.profile_scope = Some(ProfileAvailabilityScope::None);
            metadata.profile_enabled = Some(false);
        }
        self.rebuild_definitions();
        self.refresh_spawn_inherited_child_policy_snapshot();
        warnings
    }

    /// The one sink for a spawn's inherited child policy (#2216): every
    /// snapshot, whoever folded it, is completed with the policy this
    /// registry holds over entrypoint-only tools before spawn sees it.
    pub(crate) fn hand_inherited_child_policy_to_spawn(
        &self,
        mut snapshot: BTreeMap<String, ProfileAvailabilityScope>,
    ) {
        self.record_unbuilt_entrypoint_only_policy(&mut snapshot);
        if let Some(spawn) = self.tools.get("spawn") {
            spawn.set_inherited_child_policy_snapshot_for_spawn(snapshot);
        }
    }

    /// #2216: marks an entrypoint-only tool this runtime could have built but
    /// withheld (`--no-workflow`, a swarm member), so its children stay
    /// closed to it as they would be to any tool the parent lacks.
    pub(crate) fn withhold_entrypoint_only_tool(&mut self, name: &str) {
        debug_assert!(
            entrypoint_only_tools().any(|(tool, _)| tool == name),
            "only an entrypoint-only tool can be withheld: {name}"
        );
        self.withheld_entrypoint_only.insert(name.to_string());
    }

    /// Whether `name` is registered as an entrypoint-only tool.
    pub(crate) fn registers_entrypoint_only(&self, name: &str) -> bool {
        self.tools.contains_key(name)
            && self
                .metadata
                .get(name)
                .is_some_and(|metadata| is_entrypoint_only(name, metadata))
    }

    /// #2216: a child reads an entrypoint-only tool's absence from the
    /// snapshot as "never built", so the policy this parent holds over such a
    /// tool must be recorded. A denial (a startup or spawn restriction, or a
    /// withheld tool) records `None` over any entry, so no tool claiming the
    /// same id can carry a wider scope. Otherwise a tool whose stable id is
    /// unrecorded records its persisted preference under both keys, never
    /// wider than a same-named tool's entry, as that would cap it had it been
    /// built.
    pub(super) fn record_unbuilt_entrypoint_only_policy(
        &self,
        snapshot: &mut BTreeMap<String, ProfileAvailabilityScope>,
    ) {
        for (name, registration) in entrypoint_only_tools() {
            let identity = registration.identity_for_name(name);
            let denied = self.withheld_entrypoint_only.contains(name)
                || identity
                    .resolver_inputs()
                    .iter()
                    .any(|input| self.denied_policy_ids.contains(input.as_ref()));
            // Keyed on the stable id alone: a same-named tool's name entry
            // says nothing about whether this tool was built.
            let built = snapshot.contains_key(identity.stable_id.as_ref());
            let scope = match (denied, built) {
                (true, _) => Some(ProfileAvailabilityScope::None),
                (false, true) => None,
                (false, false) => self
                    .persisted_scope_for(name, &registration)
                    .map(|persisted| match snapshot.get(name) {
                        Some(existing) => persisted.intersection(*existing),
                        None => persisted,
                    }),
            };
            if let Some(scope) = scope {
                snapshot.insert(identity.stable_id.to_string(), scope);
                snapshot.insert(name.to_string(), scope);
            }
            debug_assert!(
                !denied
                    || recorded_scope(snapshot, name, &identity)
                        .is_some_and(|scope| !scope.allows_child()),
                "a denied entrypoint-only tool must be closed to children: {name}"
            );
        }
    }

    pub(crate) fn refresh_spawn_inherited_child_policy_snapshot(&self) {
        self.hand_inherited_child_policy_to_spawn(inherited_child_policy_from_catalogue(
            self.catalogue_entries(),
        ));
    }
}
