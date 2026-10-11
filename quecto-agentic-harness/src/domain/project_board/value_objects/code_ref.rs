//! How the board names code (#2481's artifact model): a commit by its full
//! id, a branch by a name git takes as it is, and a repository by its
//! host's `owner/name`.
use super::schema_error::SchemaError;

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

/// Names git keeps for itself, which a branch would shadow.
const RESERVED: &[&str] = &[
    "HEAD",
    "FETCH_HEAD",
    "ORIG_HEAD",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "BISECT_HEAD",
    "AUTO_MERGE",
];

/// A branch as git names it (`git check-ref-format --branch`), within
/// printable ASCII: at most [`MAX_BRANCH_BYTES`]; none of `~^:?*[\`;
/// `/`-separated components, none empty, none starting with `.` or ending
/// with `.lock`; no `..` or `@{`; not starting with `-`, not ending with
/// `.`, not `@`. Besides git's rules, a name git or a reader would take as
/// something else is refused: one of git's own names (`HEAD`,
/// `FETCH_HEAD`, …), a ref path (`refs/…`), or a bare commit id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// # Errors
    /// A name git would refuse or read as something else.
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        match git_branch(text) && ours(text) {
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

/// The printable characters git refuses in a ref name.
const GIT_SPECIAL: &[u8] = b"~^:?*[\\";

/// git's own rules for a branch name, within printable ASCII.
fn git_branch(text: &str) -> bool {
    let printable = text
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !GIT_SPECIAL.contains(&byte));
    let components = text.split('/').all(|component| {
        matches!(component.as_bytes().first(), Some(first) if *first != b'.')
            && !component.ends_with(".lock")
    });
    (1..=MAX_BRANCH_BYTES).contains(&text.len())
        && printable
        && components
        && !text.contains("..")
        && !text.contains("@{")
        && !text.starts_with('-')
        && !text.ends_with('.')
        && text != "@"
}

/// Names git takes but would read as something else.
fn ours(text: &str) -> bool {
    !RESERVED.contains(&text) && !text.starts_with("refs/") && !full_hex(text)
}

/// The most bytes in each part of a repository's name.
pub const MAX_REPO_PART_BYTES: usize = 100;

/// A repository as its host names it, `owner/name`: two parts of
/// letters, digits, `.`, `_` and `-`, each at most
/// [`MAX_REPO_PART_BYTES`], neither `.` nor `..`. A change names the one
/// its branch lives in, which a fork makes another than the board's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoName(String);

impl RepoName {
    /// # Errors
    /// Anything but `owner/name` in those characters.
    pub fn parse(text: &str) -> Result<Self, SchemaError> {
        let parts: Vec<&str> = text.split('/').collect();
        let named = parts.len() == 2
            && parts.iter().all(|part| {
                (1..=MAX_REPO_PART_BYTES).contains(&part.len())
                    && !matches!(*part, "." | "..")
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            });
        match named {
            true => Ok(Self(text.to_string())),
            false => Err(SchemaError::new(
                "repo",
                format!("{text:?} is not a repository's owner/name"),
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
