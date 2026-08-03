use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use super::tool_descriptor::ToolSource;

#[cfg(test)]
#[path = "tool_id_tests.rs"]
mod tests;

pub const TOOL_ID_SCHEME_V1: &str = "tool.v1";
pub const LEGACY_NAME_SCHEME_V0: &str = "tool.name.v0";
const LEGACY_NAME_PREFIX_V0: &str = "tool.name.v0:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolIdentity {
    pub stable_id: Cow<'static, str>,
    pub legacy_name_id: Cow<'static, str>,
    pub aliases: Vec<Cow<'static, str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolIdResolveError {
    Unknown(String),
    Duplicate(String),
}

#[derive(Debug, Default, Clone)]
pub struct ToolIdResolver {
    canonical_by_input: BTreeMap<String, String>,
    canonical_ids: BTreeSet<String>,
}

impl ToolIdentity {
    pub fn new(
        source: ToolSource,
        provider_id: &str,
        name: &str,
        aliases: Vec<Cow<'static, str>>,
    ) -> Self {
        Self {
            stable_id: stable_tool_id(source, provider_id, name).into(),
            legacy_name_id: legacy_name_tool_id(name).into(),
            aliases,
        }
    }

    pub fn resolver_inputs(&self) -> Vec<Cow<'_, str>> {
        let mut inputs = vec![
            Cow::Borrowed(self.stable_id.as_ref()),
            Cow::Borrowed(self.legacy_name_id.as_ref()),
        ];
        if let Some(name) = self.legacy_name_id.strip_prefix(LEGACY_NAME_PREFIX_V0) {
            inputs.push(Cow::Borrowed(name));
        }
        for alias in &self.aliases {
            inputs.push(Cow::Borrowed(alias.as_ref()));
            inputs.push(Cow::Owned(legacy_name_tool_id(alias.as_ref())));
        }
        inputs
    }
}

pub fn stable_tool_id(source: ToolSource, provider_id: &str, name: &str) -> String {
    format!(
        "{}:{}:{}:{}",
        TOOL_ID_SCHEME_V1,
        source.as_str(),
        provider_id,
        name
    )
}

pub fn legacy_name_tool_id(name: &str) -> String {
    format!("{LEGACY_NAME_PREFIX_V0}{name}")
}

impl ToolIdResolver {
    pub fn register(&mut self, identity: &ToolIdentity) -> Result<(), ToolIdResolveError> {
        let canonical = identity.stable_id.to_string();
        if !self.canonical_ids.insert(canonical.clone()) {
            return Err(ToolIdResolveError::Duplicate(canonical));
        }
        for input in identity.resolver_inputs() {
            self.insert_alias(&input, &canonical)?;
        }
        Ok(())
    }

    pub fn resolve(&self, input: &str) -> Result<&str, ToolIdResolveError> {
        self.canonical_by_input
            .get(input)
            .map(String::as_str)
            .ok_or_else(|| ToolIdResolveError::Unknown(input.to_string()))
    }

    fn insert_alias(&mut self, alias: &str, canonical: &str) -> Result<(), ToolIdResolveError> {
        match self.canonical_by_input.get(alias) {
            Some(existing) if existing != canonical => {
                Err(ToolIdResolveError::Duplicate(alias.to_string()))
            }
            Some(_) => Ok(()),
            None => {
                self.canonical_by_input
                    .insert(alias.to_string(), canonical.to_string());
                Ok(())
            }
        }
    }
}
