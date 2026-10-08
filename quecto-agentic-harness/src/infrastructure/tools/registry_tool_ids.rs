use std::borrow::Cow;

use super::registration::ToolRegistration;
use super::registry::ToolRegistryImpl;
use crate::domain::tool_policy::value_objects::tool_id::{ToolIdResolveError, ToolIdResolver};

impl ToolRegistryImpl {
    pub(super) fn reserve_removed_tool_identity(&mut self, name: &str) {
        self.denied_names.insert(name.to_string());
        match self.metadata.get(name) {
            Some(metadata) => self.denied_policy_ids.extend(
                metadata
                    .identity_for_name(name)
                    .resolver_inputs()
                    .into_iter()
                    .map(Cow::into_owned),
            ),
            None => self.denied_policy_ids.extend(
                crate::domain::tool_policy::value_objects::tool_id::equivalent_policy_inputs(name),
            ),
        }
    }

    pub(super) fn tool_id_resolver(&self) -> Result<ToolIdResolver, ToolIdResolveError> {
        self.tool_id_resolver_excluding(None)
    }

    pub(super) fn tool_id_resolver_excluding(
        &self,
        excluded_name: Option<&str>,
    ) -> Result<ToolIdResolver, ToolIdResolveError> {
        let mut resolver = ToolIdResolver::default();
        for (name, metadata) in &self.metadata {
            if excluded_name == Some(name.as_str()) {
                continue;
            }
            resolver.register(&metadata.identity_for_name(name))?;
        }
        Ok(resolver)
    }

    pub(super) fn name_for_stable_id(&self, stable_id: &str) -> Option<String> {
        self.metadata.iter().find_map(|(name, metadata)| {
            (metadata.identity_for_name(name).stable_id.as_ref() == stable_id).then(|| name.clone())
        })
    }

    pub fn resolve_tool_policy_id(&self, policy_id: &str) -> Result<String, ToolIdResolveError> {
        let stable_id = self.tool_id_resolver()?.resolve(policy_id)?.to_string();
        self.name_for_stable_id(&stable_id)
            .ok_or_else(|| ToolIdResolveError::Unknown(policy_id.to_string()))
    }

    pub(super) fn inherited_scope_for(
        &self,
        name: &str,
        metadata: &ToolRegistration,
    ) -> Option<crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope>
    {
        let identity = metadata.identity_for_name(name);
        // #2446: a configured extension's tool the snapshot does not name
        // is covered by its extension's entry, which records that the parent
        // holds that extension's tools.
        let extension = crate::domain::tool_policy::services::tool_policy::configured_extension_key(
            &identity.stable_id,
        );
        self.inherited_policy_scopes
            .get(identity.stable_id.as_ref())
            .or_else(|| self.inherited_policy_scopes.get(name))
            .or_else(|| extension.and_then(|key| self.inherited_policy_scopes.get(key)))
            .copied()
    }

    pub(super) fn registration_identity_is_available(
        &self,
        name: &str,
        metadata: &ToolRegistration,
    ) -> Result<(), ToolIdResolveError> {
        let identity = metadata.identity_for_name(name);
        // #2216: only a bundled-native registration may use an id in the
        // bundled-native namespace, so no UDS or runtime tool can pose as a
        // bundled tool that inherited policy treats specially.
        if !claims_its_own_namespace(&identity, metadata) {
            return Err(ToolIdResolveError::Duplicate(
                identity.stable_id.into_owned(),
            ));
        }
        let resolver_inputs = identity.resolver_inputs();
        if self
            .denied_policy_ids
            .iter()
            .any(|denied| resolver_inputs.iter().any(|input| denied == input.as_ref()))
        {
            return Err(ToolIdResolveError::Duplicate(
                identity.stable_id.into_owned(),
            ));
        }
        self.tool_id_resolver_excluding(Some(name))?
            .register(&identity)
    }
}

/// Whether `metadata` may use its stable id: ids in the bundled-native
/// namespace belong to bundled-native registrations alone.
fn claims_its_own_namespace(
    identity: &crate::domain::tool_policy::value_objects::tool_id::ToolIdentity,
    metadata: &ToolRegistration,
) -> bool {
    let bundled_namespace = format!(
        "{}:{}:",
        crate::domain::tool_policy::value_objects::tool_id::TOOL_ID_SCHEME_V1,
        crate::domain::tool_policy::value_objects::tool_descriptor::ToolSource::BundledNative
            .as_str()
    );
    match identity.stable_id.starts_with(&bundled_namespace) {
        true => metadata.source == crate::domain::tool_policy::value_objects::tool_descriptor::ToolSource::BundledNative,
        false => true,
    }
}
