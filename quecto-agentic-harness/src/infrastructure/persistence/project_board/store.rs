//! A project's board on its repository's `quecto/board` branch, read and
//! written with git plumbing alone, as the board capability's
//! [`ProjectBoardStore`]. The store never touches the checkout's working
//! tree, index, `HEAD`, branches, tags or `FETCH_HEAD`: it builds blobs,
//! trees and commits in the object store, fetches the board into
//! `refs/remotes/<remote>/quecto/board` with an explicit refspec, and moves
//! the remote branch only by a compare-and-swap push.
//!
//! The branch is anyone's to push, so [`board_tree`](super::board_tree)
//! reads it as hostile. A write regenerates every summary and the README
//! from what it writes, and carries forward every task file: the tasks it
//! read, and every task file set aside, as it is and with its own mode
//! (one of a newer schema, say), so no write loses one. A task file named
//! twice in one tree refuses every write instead.
//!
//! The board is fetched from the remote's URL and pushed to its push URL;
//! a remote whose `pushurl` names another repository than its `url` is
//! not a board remote, and a push to it is judged by what `url` serves.
//!
//! In a partial clone, the board is fetched with all its files
//! (`--no-filter`), and what a clone still misses (a board commit it
//! fetched with a filter before) is fetched in one call per tree level,
//! never one object at a time.
use super::board_tree::{
    Board, LIMITS, Limits, MAX_SUMMARY_BYTES, MAX_TASK_ENTRIES, REGULAR_FILE, TREE, Unread,
    contents, object_id, read_board, told, unavailable, within,
};
use super::git::{Git, GitError, SHORT_OUTPUT, failed, quoted};
use super::push_report::{PushReport, RefStatus, Rejection, ref_status, remote_rejection};
use super::summary::summary;
use super::task_record::{decode, encode};
use crate::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, ProjectBoardStore,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub use super::board_tree::{MAX_TASK_ENTRIES as MAX_ENTRIES, MAX_TASKS};

pub const BOARD_REF: &str = "refs/heads/quecto/board";
/// Fetches of what a partial clone misses, one per tree level: the root,
/// `tasks/`, then their files.
const FETCH_ROUNDS: usize = 3;
/// The most a listing of a repository's branches takes.
const MAX_REF_LISTING: usize = 16 * 1024 * 1024;
const README: &[u8] = b"# quecto board\n\nThis branch holds the project's board: one `tasks/<id>.json` per task, with a\nread-only `tasks/<id>.md` summary beside it. It is written only by quecto's\nboard operations; do not edit, merge or rebase it by hand. If a hand edit breaks it,\nmove `quecto/board` back to the commit before that edit.\n";

pub struct GitBoardStore {
    git: Git,
    remote: String,
    /// The board last read, by its commit: a commit never changes, so a
    /// write from the head just read does not read the board again.
    last: Mutex<Option<(String, Arc<Board>)>>,
    /// What a read takes at most, and so a write makes ([`LIMITS`]).
    pub(super) limits: Limits,
}

/// What became of a push.
#[derive(Debug, PartialEq, Eq)]
enum Pushed {
    Landed,
    Lost,
    Refused(String),
}

impl GitBoardStore {
    /// `remote` is the name of a remote the repository configures, so a
    /// path or URL spelled like a name is never taken for one.
    ///
    /// # Errors
    /// A name that is not a configured remote's.
    pub fn new(git: Git, remote: &str) -> Result<Self, BoardStoreError> {
        let named = remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            && remote
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric);
        if !named {
            return Err(BoardStoreError::Unavailable(format!(
                "{remote:?} is not a remote name"
            )));
        }
        // The name was checked first, so it is never read as an option.
        match git.ok(&["remote", "get-url", remote], None) {
            Ok(_) => Ok(Self {
                git,
                remote: remote.to_string(),
                last: Mutex::new(None),
                limits: LIMITS,
            }),
            Err(error) => Err(BoardStoreError::Unavailable(format!(
                "{remote:?} is not a remote of this repository: {error}"
            ))),
        }
    }

    fn tracking(&self) -> String {
        format!("refs/remotes/{}/quecto/board", self.remote)
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

    /// The board at `commit`, fetching in one call per round what this
    /// clone does not hold yet (a partial clone).
    fn read_fetching(&self, commit: &str) -> Result<Board, BoardStoreError> {
        let mut fetched: BTreeSet<String> = BTreeSet::new();
        for round in 0..=FETCH_ROUNDS {
            let missing = match read_board(&self.git, commit, self.limits) {
                Ok(board) => return Ok(board),
                Err(Unread::Failed(BoardStoreError::Clash(why))) => {
                    return Err(BoardStoreError::Clash(format!(
                        "`quecto/board` on {} is a branch of its own: {why}; rename it",
                        self.remote
                    )));
                }
                Err(Unread::Failed(error)) => return Err(error),
                Err(Unread::Missing(missing)) => missing,
            };
            assert!(!missing.is_empty(), "a read misses at least one object");
            let again = missing.iter().any(|oid| fetched.contains(oid));
            if again || round == FETCH_ROUNDS {
                return Err(BoardStoreError::Unavailable(format!(
                    "objects of board commit {commit} are missing here and were not fetched: {}",
                    missing.join(" ")
                )));
            }
            self.fetch_objects(&missing)?;
            fetched.extend(missing);
        }
        unreachable!("the last round returns")
    }

    /// Fetches `oids` and everything under them in one call, as git's own
    /// lazy fetch does, but with every file.
    fn fetch_objects(&self, oids: &[String]) -> Result<(), BoardStoreError> {
        let args = [
            "-c",
            "fetch.negotiationAlgorithm=noop",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--no-filter",
            "--stdin",
            &self.remote,
        ];
        let input: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
        let output = self
            .git
            .network_with(&args, Some(input.as_bytes()), SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.success() {
            true => Ok(()),
            false => Err(BoardStoreError::Unavailable(format!(
                "objects missing here could not be fetched from {}: {}: {}",
                self.remote,
                oids.join(" "),
                failed(&args, &output)
            ))),
        }
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

    /// Fetches the board alone into the tracking ref (no tags, no
    /// `FETCH_HEAD`), with all its files even in a partial clone; its
    /// commit.
    fn fetch(&self) -> Result<String, BoardStoreError> {
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--no-filter",
            &self.remote,
            &spec,
        ];
        let output = self.git.network(&args).map_err(unavailable)?;
        if !output.status.success() {
            return Err(unavailable(failed(&args, &output)));
        }
        let commit = format!("{}^{{commit}}", self.tracking());
        object_id(
            &self
                .git
                .text(&["rev-parse", "--verify", "--quiet", &commit], None)
                .map_err(unavailable)?,
        )
    }

    /// Moves the remote board from `base` (absent when `None`) to `commit`.
    /// A push that reports our ref updated landed. Any other outcome (a
    /// rejection, a failure, no report, a timeout) is judged by the board
    /// (see [`Self::judge`]).
    fn push(&self, base: Option<&str>, commit: &str) -> Result<Pushed, BoardStoreError> {
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

    /// The committer of `commit`, exactly as recorded, fetching the board
    /// first when this clone does not hold it yet.
    ///
    /// # Errors
    /// The commit is not on the board, or git fails.
    pub fn committed_by(&self, commit: &str) -> Result<Identity, BoardStoreError> {
        let commit = object_id(commit)?;
        let args = [
            "log",
            "-1",
            "--no-show-signature",
            "--format=%cn%x00%ce",
            commit.as_str(),
        ];
        let bytes = match self.git.ok(&args, None) {
            Ok(bytes) => bytes,
            Err(GitError::Failed { .. }) => {
                self.fetch()?;
                self.git.ok(&args, None).map_err(unavailable)?
            }
            Err(error) => return Err(unavailable(error)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let (name, email) = text
            .trim_end_matches('\n')
            .split_once('\0')
            .ok_or_else(|| BoardStoreError::Unavailable(format!("no committer in {commit}")))?;
        Ok(Identity {
            name: name.into(),
            email: email.into(),
        })
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
                return Ok(base);
            }
        }
        if self.remote_head(BOARD_REF)?.is_none() {
            return Err(BoardStoreError::NoBoard);
        }
        let tip = self.fetch()?;
        match self.on_board(&base, &tip)? {
            true => Ok(base),
            false => Err(invalid()),
        }
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
    /// takes.
    fn tree(&self, entries: &BTreeMap<String, String>) -> Result<String, BoardStoreError> {
        within(entries, self.limits)?;
        let listing: String = entries
            .iter()
            .map(|(name, meta)| format!("{meta}\t{name}\0"))
            .collect();
        object_id(
            &self
                .git
                .text(&["mktree", "-z"], Some(listing.as_bytes()))
                .map_err(unavailable)?,
        )
    }

    /// The `tasks/` tree of `board` with `written` in place: every task's
    /// file and a summary regenerated from it (an unchanged summary keeps
    /// its object), and the task files set aside carried forward as they
    /// are.
    fn tasks_tree(
        &self,
        board: &Board,
        written: &[(&Task, Vec<u8>)],
    ) -> Result<String, BoardStoreError> {
        let mut files: BTreeMap<String, String> = board.kept.clone();
        let written_ids: BTreeSet<&Slug> =
            written.iter().map(|(task, _)| &task.fields().id).collect();
        let carried: Vec<_> = board
            .tasks
            .iter()
            .filter(|(id, _)| !written_ids.contains(id))
            .collect();
        // Summaries of carried tasks that may already be right, read back in
        // one call and compared.
        let expected: BTreeMap<&Slug, String> = carried
            .iter()
            .map(|(id, held)| (*id, summary(&held.task)))
            .collect();
        let candidates: Vec<(&str, usize)> = carried
            .iter()
            .filter_map(|(id, held)| {
                let (oid, size) = held.summary.as_ref()?;
                (*size <= MAX_SUMMARY_BYTES && *size == expected[id].len())
                    .then_some((oid.as_str(), *size))
            })
            .collect();
        let existing = contents(&self.git, &candidates)?;
        for (id, held) in &carried {
            let text = &expected[id];
            let same = held
                .summary
                .as_ref()
                .and_then(|(oid, _)| existing.get(oid).map(|bytes| (oid, bytes)))
                .filter(|(_, bytes)| bytes.as_slice() == text.as_bytes());
            let summary_oid = match same {
                Some((oid, _)) => oid.clone(),
                None => self.blob(text.as_bytes())?,
            };
            let id = id.as_str();
            debug_assert!(!board.kept.contains_key(&format!("{id}.json")));
            files.insert(
                format!("{id}.json"),
                format!("{REGULAR_FILE} blob {}", held.json),
            );
            files.insert(
                format!("{id}.md"),
                format!("{REGULAR_FILE} blob {summary_oid}"),
            );
        }
        for (task, bytes) in written {
            let id = task.fields().id.as_str();
            let json = self.blob(bytes)?;
            let summary_oid = self.blob(summary(task).as_bytes())?;
            files.insert(format!("{id}.json"), format!("{REGULAR_FILE} blob {json}"));
            files.insert(
                format!("{id}.md"),
                format!("{REGULAR_FILE} blob {summary_oid}"),
            );
        }
        assert!(
            files.len() <= MAX_TASK_ENTRIES,
            "the entry bound was checked first"
        );
        self.tree(&files)
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
        Ok(BoardSnapshot {
            head: BoardHead(head),
            tasks: board.tasks.values().map(|held| held.task.clone()).collect(),
            quarantined: board.quarantined.clone(),
        })
    }

    /// Writes `tasks` and every summary on top of `expected` in one commit
    /// and pushes it only if the board is still there.
    fn write(
        &self,
        expected: &BoardHead,
        tasks: &[Task],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", message, 200).map_err(|error| invalid(error.to_string()))?;
        let ids: BTreeSet<&Slug> = tasks.iter().map(|task| &task.fields().id).collect();
        if ids.len() != tasks.len() || tasks.is_empty() {
            return Err(invalid("write one or more tasks, each once".into()));
        }
        let base = self.base(expected)?;
        let board = self.board_at(&base)?;
        if let Some(refusal) = board.repeated_refusal(&base) {
            return Ok(BoardWrite::Refused(refusal));
        }
        if let Some(file) = ids.iter().find_map(|id| board.is_quarantined(id)) {
            return Ok(BoardWrite::Refused(format!(
                "{} is set aside: {}",
                file.path, file.reason
            )));
        }
        let after: BTreeSet<&Slug> = board.tasks.keys().chain(ids.iter().copied()).collect();
        let entries = 2 * after.len() + board.kept.len();
        if entries > MAX_TASK_ENTRIES {
            return Err(invalid(format!(
                "the board would hold {} tasks and {} files set aside; it holds at most {MAX_TASKS} tasks",
                after.len(),
                board.kept.len()
            )));
        }
        let encoded = tasks
            .iter()
            .map(|task| {
                let id = task.fields().id.as_str();
                let bytes = encode(task).map_err(|error| invalid(format!("task {id}: {error}")))?;
                match decode(&bytes).ok().as_ref() == Some(task) {
                    true => Ok((task, bytes)),
                    false => Err(invalid(format!("task {id} does not read back as written"))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A read takes at most `limits.board_bytes` of task files, so no write
        // makes a board that cannot be read.
        let replaced: BTreeSet<String> = ids
            .iter()
            .map(|id| format!("{}.json", id.as_str()))
            .collect();
        let carried = board
            .read_bytes
            .iter()
            .filter(|(name, _)| !replaced.contains(*name));
        let bytes = carried.map(|(_, size)| size).sum::<usize>()
            + encoded.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        if bytes > self.limits.board_bytes {
            return Err(invalid(format!(
                "the board would hold {bytes} bytes of task files; a read takes at most {}",
                self.limits.board_bytes
            )));
        }
        let readme = self.blob(README)?;
        let root = BTreeMap::from([
            (
                "README.md".to_string(),
                format!("{REGULAR_FILE} blob {readme}"),
            ),
            (
                "tasks".to_string(),
                format!("{TREE} tree {}", self.tasks_tree(&board, &encoded)?),
            ),
        ]);
        let tree = self.tree(&root)?;
        let commit_args = [
            "commit-tree",
            tree.as_str(),
            "-p",
            base.as_str(),
            "-m",
            message,
        ];
        let commit = self.git.text(&commit_args, None).map_err(unavailable)?;
        match self.push(Some(&base), &commit)? {
            Pushed::Landed => Ok(BoardWrite::Written(BoardHead(commit))),
            Pushed::Lost => Ok(BoardWrite::Lost),
            Pushed::Refused(reason) => Ok(BoardWrite::Refused(reason)),
        }
    }

    /// Creates the board with only its README, unless it exists. An
    /// existing branch is the board only when read and write would take it
    /// as one.
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        if let Some(clash) = self.clash()? {
            return Err(BoardStoreError::Clash(clash));
        }
        if self.remote_head(BOARD_REF)?.is_some() {
            return self.existing().map(BoardBootstrap::Existing);
        }
        let readme = self.blob(README)?;
        let tree = self.tree(&BTreeMap::from([(
            "README.md".to_string(),
            format!("{REGULAR_FILE} blob {readme}"),
        )]))?;
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

    fn writer(&self) -> Identity {
        self.git.identity().clone()
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
