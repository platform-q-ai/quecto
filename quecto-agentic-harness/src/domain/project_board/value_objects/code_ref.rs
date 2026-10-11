//! How the board names code (#2481's artifact model): a commit by its full
//! id, and a branch by a name git takes as it is.
use super::schema_error::SchemaError;
use regex::Regex;
use std::sync::LazyLock;

/// The longest branch name.
pub const MAX_BRANCH_BYTES: usize = 255;

/// A commit by its full id: 40 (SHA-1) or 64 (SHA-256) lowercase hex
/// digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// One component of a branch name: letters, digits, `.`, `_` and `-`,
/// starting with a letter or digit and not ending in `.`.
static COMPONENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9_-])?$").expect("component")
});

/// A branch as git names it, from the characters every remote and shell
/// takes as they are: `/`-separated components (see [`COMPONENT`]), none
/// ending in `.lock`, no `..`; not `HEAD`, not a ref path (`refs/…`), not a
/// bare commit id (which git would read as one); at most
/// [`MAX_BRANCH_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// # Errors
    /// A name git would refuse or read as something else.
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let components = text
            .split('/')
            .all(|component| COMPONENT.is_match(component) && !component.ends_with(".lock"));
        let named = text.len() <= MAX_BRANCH_BYTES
            && components
            && !text.contains("..")
            && text != "HEAD"
            && !text.starts_with("refs/")
            && !full_hex(text);
        match named {
            true => Ok(Self(text.to_string())),
            false => Err(SchemaError::new(
                "branch",
                format!("{text:?} is not a branch name of at most {MAX_BRANCH_BYTES} bytes"),
            )),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn full_hex(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
#[path = "code_ref_tests.rs"]
mod tests;
