//! A commit by its full id (#2481's artifact model), in a file of its own
//! so that an architecture test holds the whole file to a snapshot: a
//! `board_op` record's `head` is one, and only [`CommitId::parse`] builds
//! one. Re-exported from `code_ref`.
use super::schema_error::SchemaError;

/// A commit by its full id: 40 (SHA-1) or 64 (SHA-256) lowercase hex
/// digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct CommitId(String);

impl CommitId {
    /// # Errors
    /// Anything but a full lowercase commit id.
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        match full_hex(text) {
            true => Ok(Self(text.to_string())),
            false => Err(SchemaError::new(
                "commit",
                format!("{text:?} is not a full commit id"),
            )),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A full commit id's length, in lowercase hex.
pub(super) fn full_hex(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
