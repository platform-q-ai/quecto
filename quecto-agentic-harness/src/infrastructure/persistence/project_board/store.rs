//! A project's board on its repository's `quecto/board` branch, read and
//! written with git plumbing alone. The store never touches the checkout's
//! working tree, index, `HEAD`, branches or `FETCH_HEAD`: it builds blobs,
//! trees and commits in the object store, fetches the board into
//! `refs/remotes/<remote>/quecto/board` with an explicit refspec, and moves
//! the remote branch only by a compare-and-swap push.
use super::git::{Git, GitError};
use super::summary::summary;
use super::task_record::{MAX_TASK_FILE_BYTES, RecordError, decode, encode};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};

pub const BOARD_REF: &str = "refs/heads/quecto/board";
const README: &[u8] = b"# quecto board\n\nThis branch holds the project's board: one `tasks/<id>.json` per task, with a\nread-only `tasks/<id>.md` summary beside it. It is written only by quecto's\nboard operations; do not edit, merge or rebase it by hand.\n";

/// Why the store could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Git(GitError),
    /// The remote has no board yet: bootstrap it.
    NoBoard,
    /// A `quecto` branch would hide `quecto/board`.
    Clash(String),
    /// Someone else moved the board since `base`; it was refetched.
    Lost {
        head: Option<String>,
    },
    /// The push was refused and the board did not move.
    Refused(String),
    Record {
        id: String,
        error: RecordError,
    },
    /// An argument the store does not accept.
    Invalid(String),
}

impl From<GitError> for StoreError {
    fn from(error: GitError) -> Self {
        Self::Git(error)
    }
}

/// A file on the board that is not a readable task, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quarantined {
    pub path: String,
    pub reason: String,
}

/// The board at one commit: its valid tasks and what was set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub head: String,
    pub tasks: Vec<Task>,
    pub quarantined: Vec<Quarantined>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bootstrapped {
    Created { head: String },
    Existing { head: String },
}

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

impl GitBoardStore {
    /// `remote` is a configured remote's name.
    pub fn new(git: Git, remote: &str) -> Result<Self, StoreError> {
        let named = remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if named
            && remote
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            Ok(Self {
                git,
                remote: remote.to_string(),
            })
        } else {
            Err(StoreError::Invalid(format!(
                "{remote:?} is not a remote name"
            )))
        }
    }

    fn tracking(&self) -> String {
        format!("refs/remotes/{}/quecto/board", self.remote)
    }

    /// The remote's commit for `reference`, if it has one.
    fn remote_head(&self, reference: &str) -> Result<Option<String>, StoreError> {
        let args = ["ls-remote", "--refs", &self.remote, reference];
        let output = self.git.network(&args)?;
        if !output.status.success() {
            return Err(super::git::failed(&args, &output).into());
        }
        let listing = String::from_utf8_lossy(&output.stdout);
        let exact = listing
            .lines()
            .filter_map(|row| row.split_once('\t'))
            .find(|(_, name)| *name == reference);
        exact.map(|(oid, _)| object_id(oid)).transpose()
    }

    /// Fetches the board into the tracking ref; its commit.
    fn fetch(&self) -> Result<String, StoreError> {
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            &self.remote,
            &spec,
        ];
        let output = self.git.network(&args)?;
        if !output.status.success() {
            return Err(super::git::failed(&args, &output).into());
        }
        object_id(&self.git.text(
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{}^{{commit}}", self.tracking()),
            ],
            None,
        )?)
    }

    /// Creates the board with only its README, unless it exists.
    pub fn bootstrap(&self) -> Result<Bootstrapped, StoreError> {
        let local = self.git.run(
            &["rev-parse", "--verify", "--quiet", "refs/heads/quecto"],
            None,
            self.git.local_timeout,
        )?;
        let tracking = format!("refs/remotes/{}/quecto", self.remote);
        let tracked = self.git.run(
            &["rev-parse", "--verify", "--quiet", &tracking],
            None,
            self.git.local_timeout,
        )?;
        if local.status.success()
            || tracked.status.success()
            || self.remote_head("refs/heads/quecto")?.is_some()
        {
            return Err(StoreError::Clash(format!(
                "a branch named `quecto` exists here or on {}; rename it so `quecto/board` can exist",
                self.remote
            )));
        }
        if self.remote_head(BOARD_REF)?.is_some() {
            return Ok(Bootstrapped::Existing {
                head: self.fetch()?,
            });
        }
        let readme = self.blob(README)?;
        let tree = self.git.text(
            &["mktree", "-z"],
            Some(format!("100644 blob {readme}\tREADME.md\0").as_bytes()),
        )?;
        let commit = self
            .git
            .text(&["commit-tree", &tree, "-m", "board: bootstrap"], None)?;
        match self.push(None, &commit) {
            Ok(head) => Ok(Bootstrapped::Created { head }),
            Err(StoreError::Lost { .. }) => Ok(Bootstrapped::Existing {
                head: self.fetch()?,
            }),
            Err(error) => Err(error),
        }
    }

    /// The board as the remote holds it now.
    pub fn read(&self) -> Result<BoardSnapshot, StoreError> {
        if self.remote_head(BOARD_REF)?.is_none() {
            return Err(StoreError::NoBoard);
        }
        let head = self.fetch()?;
        let mut snapshot = BoardSnapshot {
            head: head.clone(),
            tasks: vec![],
            quarantined: vec![],
        };
        let Some(tasks) = self.subtree(&head)? else {
            return Ok(snapshot);
        };
        for entry in self.entries(&tasks, true)? {
            let path = format!("tasks/{}", entry.name);
            match self.task_at(&entry) {
                Ok(Some(task)) => snapshot.tasks.push(task),
                Ok(None) => {}
                Err(reason) => snapshot.quarantined.push(Quarantined { path, reason }),
            }
        }
        Ok(snapshot)
    }

    /// The task in one `tasks/` entry, `None` for a summary, or why the
    /// entry is set aside.
    fn task_at(&self, entry: &Entry) -> Result<Option<Task>, String> {
        let is_blob = entry.mode == "100644" && entry.kind == "blob";
        let (stem, json) = match entry.name.rsplit_once('.') {
            Some((stem, "json")) if is_blob => (stem, true),
            Some((stem, "md")) if is_blob && Slug::parse(stem).is_ok() => (stem, false),
            _ => return Err("not a task file: only <id>.json and <id>.md belong in tasks/".into()),
        };
        if !json {
            return Ok(None);
        }
        let size = entry.size.unwrap_or(usize::MAX);
        if size > MAX_TASK_FILE_BYTES {
            return Err(RecordError::TooLarge { bytes: size }.to_string());
        }
        let bytes = self
            .git
            .ok(&["cat-file", "blob", &entry.oid], None)
            .map_err(|error| format!("{error:?}"))?;
        let task = decode(&bytes).map_err(|error| error.to_string())?;
        if task.fields().id.as_str() == stem {
            Ok(Some(task))
        } else {
            Err(format!(
                "holds task {:?}, not {stem:?}",
                task.fields().id.as_str()
            ))
        }
    }

    /// Writes `tasks` and their summaries on top of `base` in one commit and
    /// pushes it only if the board is still at `base`; the new head.
    pub fn write(&self, base: &str, tasks: &[Task], message: &str) -> Result<String, StoreError> {
        let base = object_id(base)?;
        line("message", message, 200).map_err(|error| StoreError::Invalid(error.to_string()))?;
        let ids: BTreeSet<&str> = tasks.iter().map(|task| task.fields().id.as_str()).collect();
        if ids.len() != tasks.len() || tasks.is_empty() {
            return Err(StoreError::Invalid(
                "write one or more tasks, each once".into(),
            ));
        }
        let mut files: BTreeMap<String, String> = match self.subtree(&base)? {
            Some(tree) => self
                .entries(&tree, false)?
                .into_iter()
                .map(|e| (e.name.clone(), tree_line(&e)))
                .collect(),
            None => BTreeMap::new(),
        };
        for task in tasks {
            let id = task.fields().id.as_str();
            let record = |error| StoreError::Record {
                id: id.into(),
                error,
            };
            let bytes = encode(task).map_err(record)?;
            if decode(&bytes).ok().as_ref() != Some(task) {
                return Err(StoreError::Invalid(format!(
                    "task {id} does not read back as written"
                )));
            }
            files.insert(
                format!("{id}.json"),
                format!("100644 blob {}", self.blob(&bytes)?),
            );
            files.insert(
                format!("{id}.md"),
                format!("100644 blob {}", self.blob(summary(task).as_bytes())?),
            );
        }
        let tasks_tree = self.tree(&files)?;
        let mut root: BTreeMap<String, String> = self
            .entries(&base, false)?
            .into_iter()
            .map(|e| (e.name.clone(), tree_line(&e)))
            .collect();
        root.insert("tasks".into(), format!("040000 tree {tasks_tree}"));
        let tree = self.tree(&root)?;
        let commit = self
            .git
            .text(&["commit-tree", &tree, "-p", &base, "-m", message], None)?;
        self.push(Some(&base), &commit)
    }

    /// Moves the remote board from `base` (absent when `None`) to `commit`.
    /// A refused push is judged by what the remote holds afterwards: our
    /// commit (it landed), another commit (lost: refetched), or `base`
    /// (a real refusal).
    fn push(&self, base: Option<&str>, commit: &str) -> Result<String, StoreError> {
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
        let pushed = self.git.network(&args)?;
        let now = self.remote_head(BOARD_REF)?;
        match now.as_deref() {
            Some(head) if head == commit => self.fetch(),
            head if head != base => {
                if now.is_some() {
                    self.fetch()?;
                }
                Err(StoreError::Lost { head: now })
            }
            _ => Err(StoreError::Refused(
                String::from_utf8_lossy(&pushed.stderr).trim().to_string(),
            )),
        }
    }

    fn blob(&self, bytes: &[u8]) -> Result<String, StoreError> {
        object_id(
            &self
                .git
                .text(&["hash-object", "-w", "--stdin"], Some(bytes))?,
        )
    }

    fn tree(&self, entries: &BTreeMap<String, String>) -> Result<String, StoreError> {
        let listing: String = entries
            .iter()
            .map(|(name, meta)| format!("{meta}\t{name}\0"))
            .collect();
        object_id(&self.git.text(&["mktree", "-z"], Some(listing.as_bytes()))?)
    }

    /// The `tasks` tree of a commit, if it has one.
    fn subtree(&self, commit: &str) -> Result<Option<String>, StoreError> {
        let found = self
            .entries(commit, false)?
            .into_iter()
            .find(|entry| entry.name == "tasks" && entry.kind == "tree");
        Ok(found.map(|entry| entry.oid))
    }

    fn entries(&self, tree: &str, sizes: bool) -> Result<Vec<Entry>, StoreError> {
        let args: &[&str] = if sizes {
            &["ls-tree", "-z", "-l", tree]
        } else {
            &["ls-tree", "-z", tree]
        };
        let listing = self.git.ok(args, None)?;
        listing
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
            .map(parse_entry)
            .collect()
    }
}

fn tree_line(entry: &Entry) -> String {
    format!("{} {} {}", entry.mode, entry.kind, entry.oid)
}

fn parse_entry(row: &[u8]) -> Result<Entry, StoreError> {
    let row = String::from_utf8_lossy(row);
    let malformed = || StoreError::Invalid(format!("unexpected ls-tree row {row:?}"));
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
fn object_id(text: &str) -> Result<String, StoreError> {
    let hex = text
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if hex && matches!(text.len(), 40 | 64) {
        Ok(text.to_string())
    } else {
        Err(StoreError::Invalid(format!("{text:?} is not an object id")))
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
