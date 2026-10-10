//! Who holds a task: a git committer identity and the claim's window.
use super::super::value_objects::schema_error::{SchemaError, line};
use super::super::value_objects::timestamp::Timestamp;
use serde::{Deserialize, Serialize};

/// Who holds the task, from when, until when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub holder: Identity,
    pub since: Timestamp,
    pub expires: Timestamp,
}

/// A git committer identity: the store is given one and writes as it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Claim {
    pub fn validate(&self) -> Result<(), SchemaError> {
        self.holder.validate()?;
        if self.since < self.expires {
            Ok(())
        } else {
            Err(SchemaError::new("claim", "must expire after it was taken"))
        }
    }
}

impl Identity {
    pub fn validate(&self) -> Result<(), SchemaError> {
        line("claim/holder", &self.name, 100)?;
        let parts = self.email.split_once('@');
        let shaped = parts.is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
        let graphic = self.email.len() <= 254 && self.email.bytes().all(|b| b.is_ascii_graphic());
        if shaped && graphic {
            Ok(())
        } else {
            Err(SchemaError::new(
                "claim/holder",
                "email must be local@domain",
            ))
        }
    }
}
