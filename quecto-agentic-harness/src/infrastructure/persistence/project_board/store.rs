//! A project's board on its repository's `quecto/board` branch, read and
//! written with git plumbing alone, as the board capability's
//! [`ProjectBoardStore`]. The store never touches the checkout's working
//! tree, index, `HEAD`, branches, tags or `FETCH_HEAD`: it builds blobs,
//! trees and commits in the object store, fetches the board into
//! `refs/remotes/<remote>/quecto/board` with an explicit refspec, and moves
//! the remote branch only by a compare-and-swap push.
//!
//! The branch is anyone's to push, so [`board_tree`](super::board_tree)
//! reads it as hostile. Each task is its canonical markdown file,
//! `tasks/<id>.md` (the task codec's). A write regenerates the README,
//! writes the tasks it is given, and carries forward every other task file
//! by its object: the tasks it read, and every task file set aside, as it
//! is and with its own mode (one of a newer schema, or the JSON form older
//! quecto wrote, say), so no write loses one. A task file named twice in
//! one tree refuses every write instead.
//!
//! The other kinds' folders are read (#2481 S9) and written (S10a) the same
//! way: each artifact written as its kind's file, every other file carried
//! by its object (`store_write`). The archive is carried as the tree it
//! read.
//!
//! The board is fetched from the remote's URL and pushed to its push URL;
//! a remote whose `pushurl` names another repository than its `url` is
//! not a board remote, and a push to it is judged by what `url` serves.
//!
//! In a partial clone, the board's commits are fetched alone and what a
//! read reads under them one tree level at a time, never one object at a
//! time and never what lies under the archive (`store_fetch`, #2481 S10b).
#[cfg(test)]
use super::artifacts::{encode, task::TaskCodec};
use super::board_tree::{
    Board, FORMAT, FORMAT_FILE, FORMAT_TREE, LIMITS, Limits, MAX_READ_ENTRIES, REGULAR_FILE, TREE,
    object_id, task_file, told, unavailable, within,
};
use super::git::{Git, GitError, SHORT_OUTPUT, failed, quoted};
use super::push_report::{PushReport, RefStatus, Rejection, ref_status, remote_rejection};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError,
    BoardWrite, ProjectBoardStore, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use super::board_tree::MAX_TASK_ENTRIES as MAX_ENTRIES;

pub const BOARD_REF: &str = "refs/heads/quecto/board";
/// The most a listing of a repository's branches takes.
const MAX_REF_LISTING: usize = 16 * 1024 * 1024;
/// The README every write leaves at the board's root (#2660): each write
/// writes it again, so a board written before this text has it once it is
/// next written.
pub(crate) const README: &[u8] = b"# quecto board\n\nThis branch holds the project's board: one markdown file per artifact, each in\nits kind's folder: `tasks/<id>.md`, `plans/<task>.<n>.md`, `runs/<task>.<n>.md`,\n`changes/<task>.<n>.md`, `reviews/<task>.<change>.md` and `discussions/<id>.md`,\nwith `format/board` naming the format and `archive/` kept for archived work. It is\nwritten only by quecto's board operations; do not edit, merge or rebase it by hand.\nIf a hand edit breaks it, move `quecto/board` back to the commit before that edit.\n";

pub struct GitBoardStore {
    git: Git,
    remote: String,
    /// The board last read, by its commit: a commit never changes, so a
    /// write from the head just read does not read the board again.
    last: Mutex<Option<(String, Arc<Board>)>>,
    /// What a read takes at most, and so a write makes ([`LIMITS`]).
    pub(super) limits: Limits,
    /// Set when the call this store serves was cancelled.
    cancelled: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Whether the remote is this clone's promisor (a partial clone), and
    /// the fetch filter the clone keeps for it, once asked.
    partial: std::sync::OnceLock<fetching::Partial>,
    /// How many times a fetch found the tracking ref locked by another
    /// writer in this clone and fetched again (#2654, #2663).
    pub(super) lock_races: std::sync::atomic::AtomicU64,
}

/// What became of a push.
#[derive(Debug, PartialEq, Eq)]
enum Pushed {
    Landed,
    Lost,
    Refused(String),
}

/// Why a store could not be opened on a remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOpenError {
    /// The name is no remote the repository configures: the caller's
    /// mistake, never a store out of reach (#2659).
    NoSuchRemote(String),
    /// git could not say (it did not run, or timed out).
    Unavailable(BoardStoreError),
}

/// Whether `remote` is spelt as a remote's name quecto accepts: letters,
/// digits, '.', '_' and '-', starting with a letter or a digit (an
/// allowlist), so it is never read as an option, a path or a URL.
///
/// # Errors
/// Any other spelling.
fn remote_name(remote: &str) -> Result<(), StoreOpenError> {
    let named = remote
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && remote
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric);
    match named {
        true => Ok(()),
        false => Err(StoreOpenError::NoSuchRemote(format!(
            "{remote:?} is not a remote name quecto accepts: letters, digits, '.', '_' and '-', starting with a letter or a digit"
        ))),
    }
}

/// Whether `remote` is a remote the repository `git` runs in configures:
/// a name quecto accepts ([`remote_name`]) that git resolves to a URL
/// that is not empty and is not the name itself (an allowlist: git
/// prints the name back for a remote whose `url` was emptied, and would
/// then read a folder of that name, #2705 B1).
///
/// # Errors
/// A name that is not a configured remote's; git that cannot say.
pub(super) fn configured_remote(git: &Git, remote: &str) -> Result<(), StoreOpenError> {
    remote_name(remote)?;
    // The name was checked first, so it is never read as an option.
    match git.text(&["remote", "get-url", remote], None) {
        Ok(url) if !url.is_empty() && url != remote => Ok(()),
        Ok(url) => Err(StoreOpenError::NoSuchRemote(format!(
            "{remote:?} names no repository of its own: git resolves it to {url:?}; give it a URL with git remote set-url {remote:?} <url>"
        ))),
        // git ran and failed: only a name its own list of remotes lacks
        // is no such remote (an allowlist); anything else (no
        // repository, a corrupt config, an unsafe directory) is a
        // store git cannot reach.
        Err(error @ GitError::Failed { .. }) => match git.text(&["remote"], None) {
            Ok(listed) if !listed.lines().any(|name| name == remote) => {
                Err(StoreOpenError::NoSuchRemote(format!(
                    "{remote:?} is not a remote of this repository: {error}"
                )))
            }
            Ok(_) | Err(_) => Err(StoreOpenError::Unavailable(BoardStoreError::Unavailable(
                format!("git could not say whether {remote:?} is a remote: {error}"),
            ))),
        },
        Err(
            error @ (GitError::NotFound
            | GitError::Spawn(_)
            | GitError::TimedOut { .. }
            | GitError::TooMuchOutput { .. }
            | GitError::Identity(_)),
        ) => Err(StoreOpenError::Unavailable(BoardStoreError::Unavailable(
            format!("git could not say whether {remote:?} is a remote: {error}"),
        ))),
    }
}

impl GitBoardStore {
    /// `remote` is the name of a remote the repository configures, so a
    /// path or URL spelled like a name is never taken for one.
    ///
    /// # Errors
    /// A name that is not a configured remote's; git that cannot say.
    pub fn new(git: Git, remote: &str) -> Result<Self, StoreOpenError> {
        configured_remote(&git, remote)?;
        Ok(Self::opened(git, remote))
    }

    fn opened(git: Git, remote: &str) -> Self {
        Self {
            git,
            remote: remote.to_string(),
            last: Mutex::new(None),
            limits: LIMITS,
            cancelled: None,
            partial: std::sync::OnceLock::new(),
            lock_races: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The git this store runs.
    pub(super) fn git(&self) -> &Git {
        &self.git
    }

    fn tracking(&self) -> String {
        format!("refs/remotes/{}/quecto/board", self.remote)
    }

    /// The remote board's commit, if the board exists: what a cached view
    /// compares with to know whether the board moved.
    ///
    /// # Errors
    /// The remote cannot be reached.
    pub fn head(&self) -> Result<Option<String>, BoardStoreError> {
        self.remote_head(BOARD_REF)
    }

    /// The repository's common git directory, shared by every worktree.
    ///
    /// # Errors
    /// git cannot name it.
    pub fn common_dir(&self) -> Result<std::path::PathBuf, BoardStoreError> {
        self.git.common_dir().map_err(unavailable)
    }

    /// The configured remote the board lives on.
    pub fn remote(&self) -> &str {
        &self.remote
    }

    fn last(&self) -> MutexGuard<'_, Option<(String, Arc<Board>)>> {
        self.last.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The board at `commit`, read once. The cache is locked only to look
    /// and to store, never across git.
    fn board_at(&self, commit: &str) -> Result<Arc<Board>, BoardStoreError> {
        let cached = self
            .last()
            .as_ref()
            .filter(|(at, _)| at == commit)
            .map(|(_, board)| Arc::clone(board));
        if let Some(board) = cached {
            return Ok(board);
        }
        let board = Arc::new(self.read_fetching(commit)?);
        *self.last() = Some((commit.to_string(), Arc::clone(&board)));
        Ok(board)
    }

    /// The remote's commit for `reference`, if it has one.
    fn remote_head(&self, reference: &str) -> Result<Option<String>, BoardStoreError> {
        let heads = self.remote_heads(&[reference])?;
        let exact = heads.into_iter().find(|(_, name)| name == reference);
        Ok(exact.map(|(oid, _)| oid))
    }

    /// The remote's branches matching `patterns` (all of them when none).
    fn remote_heads(&self, patterns: &[&str]) -> Result<Vec<(String, String)>, BoardStoreError> {
        let mut args = vec!["ls-remote", "--heads", "--refs", self.remote.as_str()];
        args.extend(patterns);
        let output = self
            .git
            .network_with(&args, None, MAX_REF_LISTING)
            .map_err(unavailable)?;
        if !output.status.success() {
            return Err(unavailable(failed(&args, &output)));
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|row| row.split_once('\t'))
            .map(|(oid, name)| Ok((object_id(oid)?, name.to_string())))
            .collect()
    }

    /// This store, pushing nothing once `cancelled` is set.
    #[must_use]
    pub fn with_cancel(mut self, cancelled: Option<Arc<std::sync::atomic::AtomicBool>>) -> Self {
        self.cancelled = cancelled;
        self
    }

    /// This store's board as another store that ignores any cancel: for an
    /// undo that must land even when its call was cancelled (S4c).
    #[must_use]
    pub fn without_cancel(&self) -> Self {
        Self {
            git: self.git.clone(),
            remote: self.remote.clone(),
            last: Mutex::new(None),
            limits: self.limits,
            cancelled: None,
            // What this store found of its remote holds for the same board.
            partial: self.partial.clone(),
            lock_races: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Moves the remote board from `base` (absent when `None`) to `commit`.
    /// A push that reports our ref updated landed. Any other outcome (a
    /// rejection, a failure, no report, a timeout) is judged by the board
    /// (see [`Self::judge`]).
    fn push(&self, base: Option<&str>, commit: &str) -> Result<Pushed, BoardStoreError> {
        // A cancelled call's write never lands after it (#2513).
        let cancelled = self
            .cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst));
        if cancelled {
            return Ok(Pushed::Refused(
                "the call was cancelled before its write".into(),
            ));
        }
        let lease = format!("--force-with-lease={BOARD_REF}:{}", base.unwrap_or(""));
        let spec = format!("{commit}:{BOARD_REF}");
        let args = [
            "push",
            "--porcelain",
            "--no-verify",
            "--no-follow-tags",
            "--recurse-submodules=no",
            &lease,
            &self.remote,
            &spec,
        ];
        let (status, said) = match self.git.network(&args) {
            Ok(output) => (ref_status(&output.stdout), quoted(&output.stderr)),
            Err(GitError::TimedOut { .. }) => (None, "the push timed out".to_string()),
            Err(error) => return Err(unavailable(error)),
        };
        let rejected = match status {
            Some(RefStatus::Updated) => return Ok(Pushed::Landed),
            Some(RefStatus::Rejected(reason)) => Some(reason),
            None => None,
        };
        self.judge(base, commit, &PushReport { rejected, said })
    }

    /// What became of a push that did not report our ref updated, judged by
    /// the board:
    /// - our commit on it: landed, however fast another writer stacked on
    ///   it;
    /// - any other board than `base`: lost;
    /// - the board still at `base`: refused when the remote declined it (a
    ///   hook, a protected branch or a rule: see [`remote_rejection`]), lost
    ///   when it lost a race on the ref there or git's own lease rejected
    ///   it (`[rejected]`: the board moved and moved back), and otherwise
    ///   unknown (no report, `[remote failure]`, any other reason): it may
    ///   still move, so it is an error.
    ///
    /// The push's own report is in every error, so a push that failed
    /// outright (no push rights, say) says why.
    fn judge(
        &self,
        base: Option<&str>,
        commit: &str,
        report: &PushReport,
    ) -> Result<Pushed, BoardStoreError> {
        let unread = |error: BoardStoreError| {
            BoardStoreError::Unavailable(format!(
                "the push did not land ({report}), and the board could not be read to check: {}",
                told(&error)
            ))
        };
        let head = match self.remote_head(BOARD_REF).map_err(unread)? {
            Some(_) => Some(self.fetch().map_err(unread)?),
            None => None,
        };
        let ours = match head.as_deref() {
            Some(head) => self.is_ancestor(commit, head)?,
            None => false,
        };
        let rejected = report.rejected.as_deref().unwrap_or("");
        let remote = remote_rejection(rejected);
        match (ours, head.as_deref() == base, remote) {
            (true, _, _) => Ok(Pushed::Landed),
            (false, false, _) => Ok(Pushed::Lost),
            (false, true, Some(Rejection::Declined)) => Ok(Pushed::Refused(report.to_string())),
            (false, true, Some(Rejection::Raced)) => Ok(Pushed::Lost),
            (false, true, None) if rejected.starts_with("[rejected]") => Ok(Pushed::Lost),
            (false, true, _) => Err(BoardStoreError::Unavailable(format!(
                "the push did not finish ({report}); the board may have moved since, so read it again"
            ))),
        }
    }

    /// Whether `older` is `newer` or one of its ancestors, both commits
    /// this clone holds.
    ///
    /// # Errors
    /// Either commit is unknown here, or git fails.
    pub fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool, BoardStoreError> {
        let (older, newer) = (object_id(older)?, object_id(newer)?);
        let args = [
            "merge-base",
            "--is-ancestor",
            older.as_str(),
            newer.as_str(),
        ];
        let output = self
            .git
            .local(&args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(unavailable(failed(&args, &output))),
        }
    }

    /// The commit `expected` names: the board's head or one of its
    /// ancestors, fetching the board when this clone does not hold it yet
    /// (another client's head). Refused as invalid when it is any other
    /// commit, or as no board when there is none.
    fn base(&self, expected: &BoardHead) -> Result<String, BoardStoreError> {
        let invalid =
            || BoardStoreError::InvalidWrite(format!("{:?} is not a board head", expected.0));
        let base = object_id(&expected.0).map_err(|_| invalid())?;
        if let Some(tip) = self.local_tip()? {
            if self.on_board(&base, &tip)? {
                return self.unmoved(base, &tip);
            }
        }
        if self.remote_head(BOARD_REF)?.is_none() {
            return Err(BoardStoreError::NoBoard);
        }
        let tip = self.fetch()?;
        match self.on_board(&base, &tip)? {
            true => self.unmoved(base, &tip),
            false => Err(invalid()),
        }
    }

    /// `base`, when a write may go on from it: the board's `tip` past it,
    /// if any, is judged by its format marker alone (#2705 B13). Only this
    /// quecto's marker, or none, goes on; a fence (a write from a head
    /// before it) and every other marker are refused as a read refuses
    /// them, before any commit is made.
    fn unmoved(&self, base: String, tip: &str) -> Result<String, BoardStoreError> {
        if base != tip {
            self.marker_fetching(tip)?;
        }
        Ok(base)
    }

    /// Whether `commit` is a commit here and `tip` or one of its ancestors.
    fn on_board(&self, commit: &str, tip: &str) -> Result<bool, BoardStoreError> {
        Ok(self.is_commit(commit)? && self.is_ancestor(commit, tip)?)
    }

    /// The board's commit as this clone last fetched it, if it has.
    fn local_tip(&self) -> Result<Option<String>, BoardStoreError> {
        let commit = format!("{}^{{commit}}", self.tracking());
        let args = ["rev-parse", "--verify", "--quiet", commit.as_str()];
        let output = self
            .git
            .local(&args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.code() {
            Some(0) => Ok(Some(object_id(
                String::from_utf8_lossy(&output.stdout).trim_end(),
            )?)),
            Some(1) => Ok(None),
            _ => Err(unavailable(failed(&args, &output))),
        }
    }

    fn is_commit(&self, oid: &str) -> Result<bool, BoardStoreError> {
        let args = ["cat-file", "-t", oid];
        let output = self
            .git
            .local(&args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        Ok(output.status.success() && output.stdout == b"commit\n")
    }

    fn blob(&self, bytes: &[u8]) -> Result<String, BoardStoreError> {
        object_id(
            &self
                .git
                .text(&["hash-object", "-w", "--stdin"], Some(bytes))
                .map_err(unavailable)?,
        )
    }

    /// A tree of `entries`, refused as invalid past the tree bound a read
    /// takes. Every entry carried is one a read typed, so git is not asked
    /// to find it (`--missing`): a partial clone carries the archive and a
    /// blob past every cap by their ids, never holding what they name.
    fn tree(&self, entries: &BTreeMap<String, String>) -> Result<String, BoardStoreError> {
        within(entries, self.limits)?;
        let listing: String = entries
            .iter()
            .map(|(name, meta)| format!("{meta}\t{name}\0"))
            .collect();
        object_id(
            &self
                .git
                .text(&["mktree", "-z", "--missing"], Some(listing.as_bytes()))
                .map_err(unavailable)?,
        )
    }

    /// The `tasks/` tree of `board` with `written` in place: each task
    /// written as its file, every other task carried by its object, and
    /// the task files set aside carried forward as they are.
    fn tasks_tree(
        &self,
        board: &Board,
        written: &[(&Task, Vec<u8>)],
    ) -> Result<String, BoardStoreError> {
        let mut files: BTreeMap<String, String> = board.kept.clone();
        let written_ids: BTreeSet<&Slug> =
            written.iter().map(|(task, _)| &task.fields().id).collect();
        for (id, held) in &board.tasks {
            if written_ids.contains(id) {
                continue;
            }
            let name = task_file(id);
            debug_assert!(!board.kept.contains_key(&name), "a task read is never kept");
            files.insert(name, format!("{REGULAR_FILE} blob {}", held.object));
        }
        for (task, bytes) in written {
            let object = self.blob(bytes)?;
            files.insert(
                task_file(&task.fields().id),
                format!("{REGULAR_FILE} blob {object}"),
            );
        }
        // A write leaves at most `MAX_TASK_ENTRIES`; a clear of a board read
        // past it (an older quecto's) leaves no more than it read.
        assert!(
            files.len() <= MAX_READ_ENTRIES,
            "the entry bound was checked first"
        );
        self.tree(&files)
    }

    /// The format marker's tree: [`FORMAT_FILE`] reading [`FORMAT`].
    fn format_tree(&self) -> Result<String, BoardStoreError> {
        let marker = self.blob(FORMAT)?;
        self.tree(&BTreeMap::from([(
            FORMAT_FILE.to_string(),
            format!("{REGULAR_FILE} blob {marker}"),
        )]))
    }

    /// A commit of the README, the format marker, `tasks`, the other
    /// folders' trees (`folders`, by name) and the archive `board` carries
    /// as it is, on `base`, as the store's writer, pushed only if the board
    /// is still at `base`.
    fn land(
        &self,
        base: &str,
        board: &Board,
        tasks: &str,
        folders: BTreeMap<String, String>,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        // No new tree holds a name twice: a folder that does is carried as
        // the very tree read (the remote holds it), never rebuilt.
        for (name, tree) in &board.folders {
            let carried = folders.get(*name) == Some(&format!("{TREE} tree {}", tree.oid));
            assert!(
                tree.repeated.is_empty() || carried,
                "a folder with a name listed twice is carried as it was read: {name}"
            );
        }
        let readme = self.blob(README)?;
        let format = self.format_tree()?;
        let mut root = BTreeMap::from([
            (
                "README.md".to_string(),
                format!("{REGULAR_FILE} blob {readme}"),
            ),
            (FORMAT_TREE.to_string(), format!("{TREE} tree {format}")),
            ("tasks".to_string(), format!("{TREE} tree {tasks}")),
        ]);
        let carried: BTreeMap<String, String> =
            folders.into_iter().chain(board.archive_entry()).collect();
        let names = root.len() + carried.len();
        root.extend(carried);
        assert_eq!(
            root.len(),
            names,
            "a carried tree is none of the root's own"
        );
        let tree = self.tree(&root)?;
        let args = ["commit-tree", tree.as_str(), "-p", base, "-m", message];
        let commit = self.git.text(&args, None).map_err(unavailable)?;
        match self.push(Some(base), &commit)? {
            Pushed::Landed => Ok(BoardWrite::Written(BoardHead(commit))),
            Pushed::Lost => Ok(BoardWrite::Lost),
            Pushed::Refused(reason) => Ok(BoardWrite::Refused(reason)),
        }
    }

    /// Branches that would hide or shadow `quecto/board`, as `prefix` +
    /// name: `quecto` itself, any under `quecto/board/` (a branch cannot be
    /// both a file and a directory of refs), and any whose name matches
    /// `quecto/board`'s or `quecto/`'s ignoring case (they collide on a
    /// case-insensitive filesystem).
    fn clashing<'a>(names: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<String> {
        names
            .filter_map(|name| name.strip_prefix(prefix))
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                lower == "quecto"
                    || (lower == "quecto/board" && *name != "quecto/board")
                    || lower.starts_with("quecto/board/")
                    || (lower.starts_with("quecto/") && !name.starts_with("quecto/"))
            })
            .map(|name| format!("{prefix}{name}"))
            .collect()
    }

    /// The board on the remote, fetched and read, so a branch that is not a
    /// board clashes.
    fn existing(&self) -> Result<BoardHead, BoardStoreError> {
        let head = self.fetch()?;
        self.board_at(&head)?;
        Ok(BoardHead(head))
    }

    fn clash(&self) -> Result<Option<String>, BoardStoreError> {
        let args = [
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/",
            "refs/remotes/",
        ];
        let local = self
            .git
            .ok_up_to(&args, None, MAX_REF_LISTING)
            .map_err(unavailable)?;
        let local = String::from_utf8_lossy(&local);
        let tracking = format!("refs/remotes/{}/", self.remote);
        let remote = self.remote_heads(&[])?;
        let mut found = Self::clashing(local.lines(), "refs/heads/");
        found.extend(Self::clashing(local.lines(), &tracking));
        found.extend(Self::clashing(
            remote.iter().map(|(_, name)| name.as_str()),
            "refs/heads/",
        ));
        Ok(found.first().map(|name| {
            format!(
                "{name} (here or on {}) hides `quecto/board`; rename it so the board can exist",
                self.remote
            )
        }))
    }
}

impl ProjectBoardStore for GitBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        if self.remote_head(BOARD_REF)?.is_none() {
            return Err(BoardStoreError::NoBoard);
        }
        let head = self.fetch()?;
        let board = self.board_at(&head)?;
        Ok(board.snapshot(BoardHead(head)).asserted())
    }

    /// As the board's commit `rev` held it, its head being the git
    /// store's revision: a rev that names no commit this clone holds on
    /// the board's history (as its last fetch left it) is none.
    fn artifact_at(
        &self,
        rev: &BoardHead,
        key: &ArtifactKey,
    ) -> Result<Option<BoardArtifact>, BoardStoreError> {
        let Ok(commit) = object_id(&rev.0) else {
            return Ok(None);
        };
        let on_board = match self.local_tip()? {
            Some(tip) => self.on_board(&commit, &tip)?,
            None => false,
        };
        match on_board {
            // Read past the cache, which keeps the head every write reads.
            true => Ok(self
                .read_fetching(&commit)?
                .snapshot(rev.clone())
                .artifact(key)),
            false => Ok(None),
        }
    }

    /// Writes `artifacts`, each as its kind's canonical markdown file, on
    /// top of `expected` in one commit and pushes it only if the board is
    /// still there.
    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.write_artifacts(expected, artifacts, message)
    }

    /// Creates the board with its README and format marker, unless it
    /// exists. An existing branch is the board only when read and write
    /// would take it as one.
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        if let Some(clash) = self.clash()? {
            return Err(BoardStoreError::Clash(clash));
        }
        if self.remote_head(BOARD_REF)?.is_some() {
            return self.existing().map(BoardBootstrap::Existing);
        }
        let readme = self.blob(README)?;
        let format = self.format_tree()?;
        let tree = self.tree(&BTreeMap::from([
            (
                "README.md".to_string(),
                format!("{REGULAR_FILE} blob {readme}"),
            ),
            (FORMAT_TREE.to_string(), format!("{TREE} tree {format}")),
        ]))?;
        let commit = self
            .git
            .text(&["commit-tree", &tree, "-m", "board: bootstrap"], None)
            .map_err(unavailable)?;
        match self.push(None, &commit)? {
            Pushed::Landed => Ok(BoardBootstrap::Created(BoardHead(commit))),
            Pushed::Lost => match self.remote_head(BOARD_REF)? {
                Some(_) => self.existing().map(BoardBootstrap::Existing),
                None => Err(BoardStoreError::Unavailable(
                    "the bootstrap lost a race, yet there is no board now; bootstrap again".into(),
                )),
            },
            Pushed::Refused(reason) => Err(BoardStoreError::Unavailable(format!(
                "the bootstrap push was refused: {reason}"
            ))),
        }
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.clear(expected, clear, message)
    }

    fn writer(&self) -> Identity {
        self.git.identity().clone()
    }

    fn lock_races(&self) -> u64 {
        self.lock_races.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[path = "store_clear.rs"]
mod clear;
pub(crate) use clear::cleared;
#[path = "store_committer.rs"]
mod committer;
#[path = "store_fence.rs"]
mod fence;
pub use fence::{Fence, GitBoardFence};
#[path = "store_fetch_state.rs"]
mod fetch_state;
#[path = "store_fetch.rs"]
mod fetching;
#[path = "store_write.rs"]
mod writing;

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "store_readme_tests.rs"]
mod readme_tests;

#[cfg(test)]
#[path = "store_open_tests.rs"]
mod open_tests;

#[cfg(test)]
#[path = "store_board_id_tests.rs"]
mod board_id_tests;

pub(crate) use writing::encoded;
