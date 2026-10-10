//! The id of a task, item, role, run or review finding: safe as a file name
//! on every platform and as a git path.
use super::schema_error::SchemaError;

pub const MAX_SLUG_BYTES: usize = 64;

/// Device names Windows reserves whatever the extension (`nul.json`).
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// 1 to 64 of `a-z`, `0-9` and `-`, starting and ending with a letter or
/// digit, and not a Windows device name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slug(String);

impl Slug {
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let edge =
            |byte: Option<&u8>| byte.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let bytes = text.as_bytes();
        let body = bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
        let shaped =
            bytes.len() <= MAX_SLUG_BYTES && edge(bytes.first()) && edge(bytes.last()) && body;
        if shaped && !RESERVED.is_empty() {
            Ok(Self(text.to_string()))
        } else {
            Err(SchemaError::new(
                "id",
                format!(
                    "{text:?} must be 1 to {MAX_SLUG_BYTES} of a-z, 0-9 and inner '-', not a device name"
                ),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "slug_tests.rs"]
mod tests;
