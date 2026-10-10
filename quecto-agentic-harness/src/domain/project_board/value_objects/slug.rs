//! The id of a task, item, role or run: safe as a file name and a ref path.
use super::schema_error::SchemaError;
use serde::{Deserialize, Serialize};

pub const MAX_SLUG_BYTES: usize = 64;

/// 1 to 64 of `a-z`, `0-9` and `-`, starting and ending with a letter or digit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Slug(String);

impl Slug {
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let edge =
            |byte: Option<&u8>| byte.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let bytes = text.as_bytes();
        let body = bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
        if bytes.len() <= MAX_SLUG_BYTES && edge(bytes.first()) && edge(bytes.last()) && body {
            Ok(Self(text.to_string()))
        } else {
            Err(SchemaError::new(
                "id",
                format!("{text:?} must be 1 to {MAX_SLUG_BYTES} of a-z, 0-9 and inner '-'"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Slug {
    type Error = SchemaError;
    fn try_from(text: String) -> Result<Self, SchemaError> {
        Self::parse(&text)
    }
}

impl From<Slug> for String {
    fn from(slug: Slug) -> Self {
        slug.0
    }
}

#[cfg(test)]
#[path = "slug_tests.rs"]
mod tests;
