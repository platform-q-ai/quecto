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
        "{}:{}:{}:{}:{}",
        TOOL_ID_SCHEME_V1,
        source.as_str(),
        provider_id.len(),
        provider_id,
        name
    )
}

/// A stable tool id taken apart (#2247 round 2): the namespace it was
/// minted in, the provider that registers it and the tool's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StableToolId<'a> {
    pub source: ToolSource,
    pub provider_id: &'a str,
    pub name: &'a str,
}

/// Parse `id` by the grammar [`stable_tool_id`] mints:
/// `tool.v1:<source>:<len>:<provider id of len bytes>:<name>`. An allowlist:
/// the source is one [`ToolSource`] label, the length plain decimal digits
/// matching a non-empty provider id, the name one or more of
/// `[A-Za-z0-9_-]` (the characters a model-facing tool name may use).
/// Anything else is not a stable id.
pub fn parse_stable_tool_id(id: &str) -> Option<StableToolId<'_>> {
    let rest = id.strip_prefix(TOOL_ID_SCHEME_V1)?.strip_prefix(':')?;
    let (label, rest) = rest.split_once(':')?;
    let source = ToolSource::parse(label)?;
    let (length, rest) = rest.split_once(':')?;
    let length = length
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| length.parse::<usize>().ok())
        .flatten()?;
    let provider_id = rest.get(..length).filter(|provider| !provider.is_empty())?;
    let name = rest.get(length..)?.strip_prefix(':')?;
    let name_allowed = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    let parsed = name_allowed.then_some(StableToolId {
        source,
        provider_id,
        name,
    })?;
    debug_assert_eq!(
        stable_tool_id(parsed.source, parsed.provider_id, parsed.name),
        id,
        "a parsed stable id is exactly the id its parts mint"
    );
    Some(parsed)
}

pub fn legacy_name_tool_id(name: &str) -> String {
    format!("{LEGACY_NAME_PREFIX_V0}{name}")
}

pub fn equivalent_policy_inputs(policy_id: &str) -> BTreeSet<String> {
    let mut inputs = BTreeSet::from([policy_id.to_string()]);
    if let Some(name) = policy_id.strip_prefix(LEGACY_NAME_PREFIX_V0) {
        inputs.insert(name.to_string());
    } else if !policy_id.starts_with(&format!("{TOOL_ID_SCHEME_V1}:")) {
        inputs.insert(legacy_name_tool_id(policy_id));
    }
    inputs
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
