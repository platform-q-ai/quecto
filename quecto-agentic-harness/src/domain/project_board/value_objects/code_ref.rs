//! How the board names code (#2481's artifact model): a commit by its full
//! id, a branch by a name git takes as it is, and a repository by its
//! host's `owner/name`.
//!
//! A branch name is refused where git or a reader would take it for one
//! of git's own refs. That list is a denylist by nature, since git defines
//! the names: its boundary is the names git itself writes in `$GIT_DIR`
//! (`HEAD`-like root refs, `refs/` and its siblings, worktree and remote
//! namespaces). A name git adds later is caught by the rules' shape
//! (`*_HEAD`) or added here.
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

/// Names git keeps for itself, which a branch would shadow, in lowercase:
/// they are refused in any case, as a case-insensitive file system would
/// take `Head` for `HEAD`. Besides these, every one-component name of
/// letters and `_` ending in `_head` is git's ([`git_head`]):
/// `FETCH_HEAD`, `REBASE_HEAD` and any it adds.
const RESERVED: &[&str] = &[
    "head",
    "auto_merge",
    "stash",
    "bisect_expected_rev",
    "notes_merge_ref",
    "notes_merge_partial",
    "merge_autostash",
];

/// A branch as git names it (`git check-ref-format --branch`), within
/// printable ASCII: at most [`MAX_BRANCH_BYTES`]; none of `~^:?*[\`;
/// `/`-separated components, none empty, none starting with `.` or ending
/// with `.lock`; no `..` or `@{`; not starting with `-`, not ending with
/// `.`. Besides git's rules, the board refuses names git takes but git or
/// a reader would take as something else: `@` (git's own short name for
/// `HEAD`), one of git's own names (`HEAD`, `stash`, `AUTO_MERGE`, any
/// `*_HEAD`), a ref path or another of git's refs (one starting `refs/`,
/// `heads/`, `tags/`, `remotes/`, `notes/`, `bisect/`, `stash/` or
/// `replace/`), a component ending `.lock` in any case, or a bare commit
/// id; all of them in any case.
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
}

/// The first components that make a name read as a ref path or another
/// of git's own refs (`notes/commits`, `bisect/bad`, `stash`,
/// `replace/…`), in any case.
const REF_ROOTS: &[&str] = &[
    "refs",
    "heads",
    "tags",
    "remotes",
    "notes",
    "bisect",
    "stash",
    "replace",
    "main-worktree",
    "worktrees",
    "worktree",
    "prefetch",
    "rewritten",
    "original",
    "pull",
];

/// A one-component name of letters, `_` and `-` ending in `_head`
/// (lowercase), as git's root refs are spelt: git's own, such as
/// `fetch_head` or `rebase_head`, and any like them (`my_head`, `x-y_head`).
fn git_head(lower: &str) -> bool {
    lower.ends_with("_head")
        && lower
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte == b'-')
}

/// Names git takes but would read as something else, all judged in
/// lowercase: `.lock` too, as a case-insensitive file system would.
fn ours(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let root = lower.split('/').next().unwrap_or(&lower);
    let unlocked = lower
        .split('/')
        .all(|component| !component.ends_with(".lock"));
    lower != "@"
        && !RESERVED.contains(&lower.as_str())
        && !git_head(&lower)
        && !REF_ROOTS.contains(&root)
        && unlocked
        && !full_hex(&lower)
}

/// The most bytes in each part of a repository's name.
pub const MAX_REPO_PART_BYTES: usize = 100;

/// A repository as its host names it, `owner/name`: two parts of
/// letters, digits, `.`, `_` and `-`, each at most
/// [`MAX_REPO_PART_BYTES`], neither `.` nor `..`. A change names the one
/// its branch lives in, which a fork makes another than the board's.
///
/// The host names repositories without regard to case, so two names that
/// differ only in case are the same repository: equality, order and hash
/// go by the lowercased name, and the name is shown as it was given.
#[derive(Debug, Clone)]
pub struct RepoName {
    shown: String,
    key: String,
}

impl PartialEq for RepoName {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for RepoName {}

impl PartialOrd for RepoName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RepoName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl std::hash::Hash for RepoName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

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
            true => Ok(Self {
                shown: text.to_string(),
                key: text.to_ascii_lowercase(),
            }),
            false => Err(SchemaError::new(
                "repo",
                format!("{text:?} is not a repository's owner/name"),
            )),
        }
    }

    /// The name as it was given.
    pub fn as_str(&self) -> &str {
        &self.shown
    }
}

/// A full commit id's length, in lowercase hex.
fn full_hex(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
#[path = "code_ref_tests.rs"]
mod tests;
