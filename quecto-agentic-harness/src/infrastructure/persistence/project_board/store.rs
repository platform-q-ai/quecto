//! A project's board on its repository's `quecto/board` branch, read and
//! written with git plumbing alone, as the board capability's
//! [`ProjectBoardStore`]. The store never touches the checkout's working
//! tree, index, `HEAD`, branches, tags or `FETCH_HEAD`: it builds blobs,
//! trees and commits in the object store, fetches the board into
//! `refs/remotes/<remote>/quecto/board` with an explicit refspec, and moves
//! the remote branch only by a compare-and-swap push.
//!
//! The branch is anyone's to push, so it is read as hostile: only a
//! `tasks/<id>.json` or `tasks/<id>.md` regular file is read, its size is
//! checked before its bytes, a tree holds at most [`MAX_TASK_ENTRIES`]
//! entries and nothing below `tasks/` is descended into; anything else is
//! set aside (quarantined). A write carries forward only what the store
//! itself writes: the README and those regular files.
use super::git::{Git, GitError, failed};
use super::summary::summary;
use super::task_record::{MAX_TASK_FILE_BYTES, decode, encode};
use crate::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, ProjectBoardStore,
    QuarantinedFile,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};

pub const BOARD_REF: &str = "refs/heads/quecto/board";
/// The most entries the board's `tasks/` tree may hold: a file and its
/// summary for every task a board holds, with room to spare.
pub const MAX_TASK_ENTRIES: usize = 4_096;
const README: &[u8] = b"# quecto board\n\nThis branch holds the project's board: one `tasks/<id>.json` per task, with a\nread-only `tasks/<id>.md` summary beside it. It is written only by quecto's\nboard operations; do not edit, merge or rebase it by hand.\n";
const REGULAR_FILE: &str = "100644";

pub struct GitBoardStore {
    git: Git,
    remote: String,
}

/// One `ls-tree -z` entry: mode, type, object, size (`-` for trees), name.
struct Entry {
    mode: String,
    kind: String,
    oid: String,
    size: Option<usize>,
    name: String,
}

impl Entry {
    fn regular(&self) -> bool {
        self.mode == REGULAR_FILE && self.kind == "blob"
    }

    /// A `<id>.json` or `<id>.md` name: its id and whether it is the task.
    fn task_name(&self) -> Option<(Slug, bool)> {
        let (stem, extension) = self.name.rsplit_once('.')?;
        let json = match extension {
            "json" => true,
            "md" => false,
            _ => return None,
        };
        Slug::parse(stem).ok().map(|id| (id, json))
    }
}

/// The board at one commit: its tasks, what is set aside, and the
/// `tasks/` entries as listed.
struct Board {
    tasks: Vec<Task>,
    quarantined: Vec<QuarantinedFile>,
    entries: Vec<Entry>,
}

/// What became of a push.
#[derive(Debug, PartialEq, Eq)]
enum Pushed {
    Landed,
    Lost,
    Refused(String),
}

fn unavailable(error: GitError) -> BoardStoreError {
    BoardStoreError::Unavailable(format!("{error:?}"))
}

impl GitBoardStore {
    /// `remote` is a configured remote's name.
    ///
    /// # Errors
    /// A name that is not a remote's.
    pub fn new(git: Git, remote: &str) -> Result<Self, BoardStoreError> {
        let named = remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        match named
            && remote
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            true => Ok(Self {
                git,
                remote: remote.to_string(),
            }),
            false => Err(BoardStoreError::Unavailable(format!(
                "{remote:?} is not a remote name"
            ))),
        }
    }

    fn tracking(&self) -> String {
        format!("refs/remotes/{}/quecto/board", self.remote)
    }

    /// The remote's commit for `reference`, if it has one.
    fn remote_head(&self, reference: &str) -> Result<Option<String>, BoardStoreError> {
        let args = ["ls-remote", "--refs", &self.remote, reference];
        let output = self.git.network(&args).map_err(unavailable)?;
        if !output.status.success() {
            return Err(unavailable(failed(&args, &output)));
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        let exact = listing
            .lines()
            .filter_map(|row| row.split_once('\t'))
            .find(|(_, name)| *name == reference);
        exact.map(|(oid, _)| object_id(oid)).transpose()
    }

    /// Fetches the board alone into the tracking ref (no tags, no pruning,
    /// no `FETCH_HEAD`); its commit.
    fn fetch(&self) -> Result<String, BoardStoreError> {
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-prune",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
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
    /// Judged by the push's own status for our ref, so a write that landed
    /// is never taken for lost however fast another writer stacks on it.
    /// A push that reports no status (it failed or timed out) is judged by
    /// whether the remote board now holds our commit.
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
        let status = match self.git.network(&args) {
            Ok(output) => ref_status(&output.stdout),
            Err(GitError::TimedOut { .. } | GitError::Failed { .. }) => None,
            Err(error) => return Err(unavailable(error)),
        };
        match status {
            Some(RefStatus::Updated) => Ok(Pushed::Landed),
            Some(RefStatus::Rejected(reason)) => {
                match self.remote_head(BOARD_REF)?.as_deref() == base {
                    true => Ok(Pushed::Refused(reason)),
                    false => Ok(Pushed::Lost),
                }
            }
            None => self.landed(base, commit),
        }
    }

    /// Whether a push that reported nothing landed: the remote board holds
    /// our commit (at its head or below a later one).
    fn landed(&self, base: Option<&str>, commit: &str) -> Result<Pushed, BoardStoreError> {
        let head = self.fetch()?;
        let ancestry = ["merge-base", "--is-ancestor", commit, head.as_str()];
        let output = self
            .git
            .run(&ancestry, None, self.git.local_timeout)
            .map_err(unavailable)?;
        match (output.status.code(), Some(head.as_str()) == base) {
            (Some(0), _) => Ok(Pushed::Landed),
            (Some(1), false) => Ok(Pushed::Lost),
            (Some(1), true) => Err(BoardStoreError::Unavailable(
                "the push failed and the board did not move".into(),
            )),
            _ => Err(unavailable(failed(&ancestry, &output))),
        }
    }

    /// The task in one `tasks/` entry, `None` for a summary, or why the
    /// entry is set aside.
    fn task_at(&self, entry: &Entry) -> Result<Option<Task>, String> {
        let (id, json) =
            match (entry.regular(), entry.task_name()) {
                (true, Some(named)) => named,
                _ => return Err(
                    "not a task file: only regular <id>.json and <id>.md files belong in tasks/"
                        .into(),
                ),
            };
        if !json {
            return Ok(None);
        }
        let size = entry.size.unwrap_or(usize::MAX);
        if size > MAX_TASK_FILE_BYTES {
            return Err(format!(
                "task file is {size} bytes; the most is {MAX_TASK_FILE_BYTES}"
            ));
        }
        let bytes = self
            .git
            .ok(&["cat-file", "blob", &entry.oid], None)
            .map_err(|error| format!("{error:?}"))?;
        let task = decode(&bytes).map_err(|error| error.to_string())?;
        match task.fields().id == id {
            true => Ok(Some(task)),
            false => Err(format!(
                "holds task {:?}, not {:?}",
                task.fields().id.as_str(),
                id.as_str()
            )),
        }
    }

    fn blob(&self, bytes: &[u8]) -> Result<String, BoardStoreError> {
        object_id(
            &self
                .git
                .text(&["hash-object", "-w", "--stdin"], Some(bytes))
                .map_err(unavailable)?,
        )
    }

    fn tree(&self, entries: &BTreeMap<String, String>) -> Result<String, BoardStoreError> {
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

    /// The `tasks` tree of a commit, if it has one.
    fn subtree(&self, commit: &str) -> Result<Option<String>, BoardStoreError> {
        let found = self
            .entries(commit)?
            .into_iter()
            .find(|entry| entry.name == "tasks" && entry.kind == "tree");
        Ok(found.map(|entry| entry.oid))
    }

    /// A tree's entries with their sizes, refused past [`MAX_TASK_ENTRIES`].
    fn entries(&self, tree: &str) -> Result<Vec<Entry>, BoardStoreError> {
        let listing = self
            .git
            .ok(&["ls-tree", "-z", "-l", tree], None)
            .map_err(unavailable)?;
        let rows: Vec<&[u8]> = listing
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
            .collect();
        if rows.len() > MAX_TASK_ENTRIES {
            return Err(BoardStoreError::Unavailable(format!(
                "the board's tree holds {} entries; the most is {MAX_TASK_ENTRIES}",
                rows.len()
            )));
        }
        rows.into_iter().map(parse_entry).collect()
    }

    /// The board's `tasks/` entries at `commit`, and what is set aside.
    fn read_at(&self, commit: &str) -> Result<Board, BoardStoreError> {
        let Some(tree) = self.subtree(commit)? else {
            return Ok(Board {
                tasks: vec![],
                quarantined: vec![],
                entries: vec![],
            });
        };
        let (mut tasks, mut quarantined) = (vec![], vec![]);
        let entries = self.entries(&tree)?;
        for entry in &entries {
            match self.task_at(entry) {
                Ok(Some(task)) => tasks.push(task),
                Ok(None) => {}
                Err(reason) => quarantined.push(QuarantinedFile {
                    path: format!("tasks/{}", entry.name),
                    id: entry.task_name().map(|(id, _)| id),
                    reason,
                }),
            }
        }
        Ok(Board {
            tasks,
            quarantined,
            entries,
        })
    }
}

impl ProjectBoardStore for GitBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        if self.remote_head(BOARD_REF)?.is_none() {
            return Err(BoardStoreError::NoBoard);
        }
        let head = self.fetch()?;
        let Board {
            tasks, quarantined, ..
        } = self.read_at(&head)?;
        Ok(BoardSnapshot {
            head: BoardHead(head),
            tasks,
            quarantined,
        })
    }

    /// Writes `tasks` and their summaries on top of `expected` in one commit
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
        let base = object_id(&expected.0)
            .map_err(|_| invalid(format!("{:?} is not a board head", expected.0)))?;
        let kind = self.git.text(&["cat-file", "-t", &base], None).ok();
        if kind.as_deref() != Some("commit") {
            return Err(invalid(format!(
                "{base} is not a board head this clone has read"
            )));
        }
        let Board {
            quarantined,
            entries,
            ..
        } = self.read_at(&base)?;
        if let Some(file) = quarantined
            .iter()
            .find(|file| file.id.as_ref().is_some_and(|id| ids.contains(id)))
        {
            return Ok(BoardWrite::Refused(format!(
                "{} is set aside: {}",
                file.path, file.reason
            )));
        }
        // Only what the store writes is carried forward.
        let mut files: BTreeMap<String, String> = entries
            .iter()
            .filter(|entry| entry.regular() && entry.task_name().is_some())
            .map(|entry| (entry.name.clone(), tree_line(entry)))
            .collect();
        for task in tasks {
            let id = task.fields().id.as_str();
            let bytes = encode(task).map_err(|error| invalid(format!("task {id}: {error}")))?;
            if decode(&bytes).ok().as_ref() != Some(task) {
                return Err(invalid(format!("task {id} does not read back as written")));
            }
            files.insert(
                format!("{id}.json"),
                format!("{REGULAR_FILE} blob {}", self.blob(&bytes)?),
            );
            files.insert(
                format!("{id}.md"),
                format!(
                    "{REGULAR_FILE} blob {}",
                    self.blob(summary(task).as_bytes())?
                ),
            );
        }
        let mut root: BTreeMap<String, String> = self
            .entries(&base)?
            .into_iter()
            .filter(|entry| entry.name == "README.md" && entry.regular())
            .map(|entry| (entry.name.clone(), tree_line(&entry)))
            .collect();
        root.insert(
            "tasks".into(),
            format!("040000 tree {}", self.tree(&files)?),
        );
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
            Pushed::Lost => {
                self.fetch()?;
                Ok(BoardWrite::Lost)
            }
            Pushed::Refused(reason) => Ok(BoardWrite::Refused(reason)),
        }
    }

    /// Creates the board with only its README, unless it exists.
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        let exists = |reference: &str| -> Result<bool, BoardStoreError> {
            let args = ["rev-parse", "--verify", "--quiet", reference];
            Ok(self
                .git
                .run(&args, None, self.git.local_timeout)
                .map_err(unavailable)?
                .status
                .success())
        };
        let tracking = format!("refs/remotes/{}/quecto", self.remote);
        if exists("refs/heads/quecto")?
            || exists(&tracking)?
            || self.remote_head("refs/heads/quecto")?.is_some()
        {
            return Err(BoardStoreError::Clash(format!(
                "a branch named `quecto` exists here or on {}; rename it so `quecto/board` can exist",
                self.remote
            )));
        }
        if self.remote_head(BOARD_REF)?.is_some() {
            return Ok(BoardBootstrap::Existing(BoardHead(self.fetch()?)));
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
            Pushed::Landed => Ok(BoardBootstrap::Created(BoardHead(self.fetch()?))),
            Pushed::Lost => Ok(BoardBootstrap::Existing(BoardHead(self.fetch()?))),
            Pushed::Refused(reason) => Err(BoardStoreError::Unavailable(format!(
                "the bootstrap push was refused: {reason}"
            ))),
        }
    }

    fn writer(&self) -> Identity {
        self.git.identity().clone()
    }
}

/// Our ref's line in `git push --porcelain`: `<flag>\t<from>:<to>\t<summary>`.
#[derive(Debug, PartialEq, Eq)]
enum RefStatus {
    /// ` ` fast-forward, `+` forced, `*` new, `=` already there.
    Updated,
    /// `!` rejected, with the summary's reason.
    Rejected(String),
}

fn ref_status(stdout: &[u8]) -> Option<RefStatus> {
    let text = String::from_utf8_lossy(stdout);
    let ours = format!(":{BOARD_REF}");
    text.lines().find_map(|row| {
        let mut fields = row.splitn(3, '\t');
        let (flag, refs, summary) = (fields.next()?, fields.next()?, fields.next().unwrap_or(""));
        if !refs.ends_with(&ours) {
            return None;
        }
        match flag {
            " " | "+" | "*" | "=" => Some(RefStatus::Updated),
            "!" => Some(RefStatus::Rejected(summary.to_string())),
            _ => None,
        }
    })
}

fn tree_line(entry: &Entry) -> String {
    format!("{} {} {}", entry.mode, entry.kind, entry.oid)
}

fn parse_entry(row: &[u8]) -> Result<Entry, BoardStoreError> {
    let row = String::from_utf8_lossy(row);
    let malformed = || BoardStoreError::Unavailable(format!("unexpected ls-tree row {row:?}"));
    let (meta, name) = row.split_once('\t').ok_or_else(malformed)?;
    let fields: Vec<&str> = meta.split_whitespace().collect();
    let (mode, kind, oid) = match fields.as_slice() {
        [mode, kind, oid] | [mode, kind, oid, _] => (*mode, *kind, *oid),
        _ => return Err(malformed()),
    };
    let size = fields.get(3).and_then(|size| size.parse().ok());
    Ok(Entry {
        mode: mode.into(),
        kind: kind.into(),
        oid: object_id(oid)?,
        size,
        name: name.into(),
    })
}

/// A full SHA-1 or SHA-256 object name.
fn object_id(text: &str) -> Result<String, BoardStoreError> {
    let hex = text
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    match hex && matches!(text.len(), 40 | 64) {
        true => Ok(text.to_string()),
        false => Err(BoardStoreError::Unavailable(format!(
            "{text:?} is not an object id"
        ))),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
