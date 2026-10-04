//! `extensions` in config (#2446): the UDS extensions every agent launches.
//! Global-only (a repository overlay may not define or change it), checked
//! whole at load: a name each instance's state directory is named by, an
//! absolute command, and only known placeholders in `args` and `env`.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::agents::configured_extensions::{ExtensionSpec, check_placeholders};

use super::ConfigError;

/// One entry of `extensions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Whether each locally spawned child launches its own instance.
    #[serde(default = "launched_by_children")]
    pub children: bool,
}

fn launched_by_children() -> bool {
    true
}

impl ExtensionConfig {
    pub fn spec(&self) -> ExtensionSpec {
        ExtensionSpec {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env: self.env.clone(),
            children: self.children,
        }
    }

    fn check(&self) -> Result<(), String> {
        let name_allowed = !self.name.is_empty()
            && self.name.len() <= 64
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if !name_allowed {
            return Err(format!(
                "name {:?} must be 1 to 64 ASCII letters, digits, `-` or `_`",
                self.name
            ));
        }
        if !std::path::Path::new(&self.command).is_absolute() {
            return Err(format!(
                "`{}`: command {:?} must be an absolute path",
                self.name, self.command
            ));
        }
        let key_allowed =
            |key: &String| !key.is_empty() && !key.contains('=') && !key.contains('\0');
        if let Some(key) = self.env.keys().find(|key| !key_allowed(key)) {
            return Err(format!("`{}`: env name {key:?} is not valid", self.name));
        }
        self.args
            .iter()
            .chain(self.env.values())
            .try_for_each(|template| check_placeholders(template))
            .map_err(|error| format!("`{}`: {error}", self.name))
    }
}

/// Every entry valid, and no two sharing a name.
pub fn validate(extensions: &[ExtensionConfig]) -> Result<(), ConfigError> {
    let mut names = std::collections::BTreeSet::new();
    for extension in extensions {
        extension.check().map_err(ConfigError::Extensions)?;
        if !names.insert(extension.name.as_str()) {
            return Err(ConfigError::Extensions(format!(
                "`{}` is configured more than once",
                extension.name
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "extensions_tests.rs"]
mod tests;
