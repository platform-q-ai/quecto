use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

// AgentId is the user-facing subagent display label. This legacy type is not
// durable identity; do not use it as a registry, socket, or persistence key.
string_id!(AgentId);
string_id!(MessageId);
string_id!(ToolCallId);
string_id!(CommandId);

/// Hidden durable subagent identity minted for every spawn.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentUuid(String);

impl AgentUuid {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn mint() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for AgentUuid {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for AgentUuid {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl AsRef<str> for AgentUuid {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for AgentUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
