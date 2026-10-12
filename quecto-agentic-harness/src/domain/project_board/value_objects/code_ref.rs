//! How the board names code (#2481's artifact model): a commit by its full
//! id, a branch by a name git takes as it is, and a repository by its
//! host's `owner/name`.
//!
//! A branch name is refused where git or a reader would take it for one
//! of git's own refs. That list is a denylist by nature, since git defines
//! the names: its boundary is the names git itself writes in `$GIT_DIR`
//! (`HEAD`-like root refs, `refs/` and its siblings, worktree and remote
//! namespaces), plus the hosts' ref namespaces (`pull/`). A name git adds
//! later is caught by the rules' shape (`*_HEAD`) or added here.
pub use super::commit_id::CommitId;
use super::commit_id::full_hex;
use super::schema_error::SchemaError;

/// The longest branch name.
pub const MAX_BRANCH_BYTES: usize = 255;

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
/// printable ASCII: at most [`MAX_BRANCH_BYTES`]; none of `~^:?*[\`, nor
/// a backtick, which no board file holds;
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
                format!(
                    "{text:?} is not a branch name: at most {MAX_BRANCH_BYTES} bytes of printable \
                     ASCII but ~^:?*[\\ (git's) and a backtick (no board file holds one), in \
                     /-separated parts git takes"
                ),
            )),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The punctuation a branch may hold besides letters and digits: every
/// printable ASCII one git takes in a ref name (not `~^:?*[\`), less the
/// one no board file holds: a backtick, a code span's edge (a `|` is
/// written `\|` there, and shown as it is).
const BRANCH_PUNCTUATION: &[u8] = b"!\"#$%&'()+,-./;<=>@]_{|}";

/// git's own rules for a branch name, within the characters a board file
/// holds ([`BRANCH_PUNCTUATION`]).
fn git_branch(text: &str) -> bool {
    let printable = text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || BRANCH_PUNCTUATION.contains(&byte));
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
    // A root reads as a namespace only with a `/` after it: a bare
    // `notes` or `bisect` resolves as a branch.
    let root = lower.split_once('/').map(|(root, _)| root);
    let unlocked = lower
        .split('/')
        .all(|component| !component.ends_with(".lock"));
    lower != "@"
        && !RESERVED.contains(&lower.as_str())
        && !git_head(&lower)
        && root.is_none_or(|root| !REF_ROOTS.contains(&root))
        && unlocked
        && !full_hex(&lower)
}

/// The most bytes in a repository's name.
pub const MAX_REPO_PART_BYTES: usize = 100;
/// The most bytes in a repository owner's name, a `_shortcode` included
/// (GitHub's limit).
pub const MAX_OWNER_BYTES: usize = 39;
/// The most bytes in an enterprise-managed user's shortcode.
pub const MAX_SHORTCODE_BYTES: usize = 8;

/// A repository as GitHub names it, `owner/name`, by the board's one
/// grammar ([`is_owner`], [`is_repo_name`]): the owner letters, digits and
/// hyphens, not starting with a hyphen (an old account may end with one),
/// then at most one `_shortcode`, at most [`MAX_OWNER_BYTES`] in all; the
/// name letters, digits, `.`, `_` and `-`, at most [`MAX_REPO_PART_BYTES`],
/// not starting with `-`, neither `.` nor `..`. A change names the one its
/// branch lives in, which a fork makes another than the board's.
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
        let named = match text.split('/').collect::<Vec<_>>().as_slice() {
            [owner, name] => is_owner(owner) && is_repo_name(name),
            _ => false,
        };
        match named {
            true => Ok(Self {
                shown: text.to_string(),
                key: text.to_ascii_lowercase(),
            }),
            false => Err(SchemaError::new(
                "repo",
                format!(
                    "{text:?} is not GitHub's owner/name: an owner of letters, digits and \
                     hyphens, not starting with one, then at most one _shortcode of 1 to \
                     {MAX_SHORTCODE_BYTES} letters or digits, {MAX_OWNER_BYTES} in all; a name of letters, digits, \
                     ., _ and -, not starting with -, nor . or .."
                ),
            )),
        }
    }

    /// The name as it was given.
    pub fn as_str(&self) -> &str {
        &self.shown
    }
}

/// A GitHub owner, the one grammar the board and its host adapter keep:
/// letters, digits and hyphens, starting with a letter or digit (an old
/// account may end with a hyphen, or hold `--`); then, for an
/// enterprise-managed user, one `_` and a shortcode of 1 to
/// [`MAX_SHORTCODE_BYTES`] letters and digits; at most [`MAX_OWNER_BYTES`]
/// in all.
pub fn is_owner(owner: &str) -> bool {
    let (handle, short_code) = match owner.split_once('_') {
        Some((handle, short_code)) => (handle, Some(short_code)),
        None => (owner, None),
    };
    let handle_kept = !handle.is_empty()
        && handle
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let short_code_kept = short_code.is_none_or(|code| {
        (1..=MAX_SHORTCODE_BYTES).contains(&code.len())
            && code.bytes().all(|b| b.is_ascii_alphanumeric())
    });
    owner.len() <= MAX_OWNER_BYTES && handle_kept && short_code_kept
}

/// A GitHub repository's name, the one grammar the board and its host
/// adapter keep: letters, digits, `.`, `_` and `-`, at most
/// [`MAX_REPO_PART_BYTES`], not starting with `-` (an option to a tool),
/// neither `.` nor `..`.
pub fn is_repo_name(name: &str) -> bool {
    let first = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b"._".contains(&b));
    (1..=MAX_REPO_PART_BYTES).contains(&name.len())
        && first
        && !matches!(name, "." | "..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[cfg(test)]
#[path = "code_ref_tests.rs"]
mod tests;
