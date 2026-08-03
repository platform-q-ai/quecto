use super::registration::ToolRegistration;
use super::registry::ToolRegistryImpl;
use crate::domain::tool_id::{ToolIdResolveError, ToolIdResolver};

impl ToolRegistryImpl {
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

    pub(super) fn registration_identity_is_available(
        &self,
        name: &str,
        metadata: &ToolRegistration,
    ) -> Result<(), ToolIdResolveError> {
        let identity = metadata.identity_for_name(name);
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
