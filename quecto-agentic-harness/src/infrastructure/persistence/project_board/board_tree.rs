//! Reading one board commit's trees, which anyone with push access may
//! have written, so every part is checked before it is trusted:
//! - each tree's size, before it is listed;
//! - each entry's real object type and size, in one `cat-file
//!   --batch-check`, before its bytes are read;
//! - the task files' bytes, in one `cat-file --batch`.
//!
//! Only a regular-file blob named `<id>.json` or `<id>.md` belongs in
//! `tasks/`, each name once. Anything else is set aside (quarantined), and
//! a task with any part set aside is not read.
use super::git::{Git, GitError, failed};
use super::task_record::{MAX_TASK_FILE_BYTES, decode};
use crate::application::project_board::ports::{BoardStoreError, QuarantinedFile};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};

/// The most entries the board's `tasks/` tree may hold.
pub const MAX_TASK_ENTRIES: usize = 4_096;
/// The most tasks a board holds: each is a file and its summary.
pub const MAX_TASKS: usize = MAX_TASK_ENTRIES / 2;
/// The largest tree read: [`MAX_TASK_ENTRIES`] entries with long names.
pub const MAX_TREE_BYTES: usize = MAX_TASK_ENTRIES * 160;
/// The most task-file bytes one read takes in all.
pub const MAX_BOARD_BYTES: usize = 64 * 1024 * 1024;
/// The largest summary read back to compare with the one it should be.
pub const MAX_SUMMARY_BYTES: usize = 2 * MAX_TASK_FILE_BYTES;
pub const REGULAR_FILE: &str = "100644";
const TREE: &str = "040000";

pub fn unavailable(error: GitError) -> BoardStoreError {
    BoardStoreError::Unavailable(format!("{error:?}"))
}

/// A task as the board holds it: the task and its files' objects.
#[derive(Debug, Clone)]
pub struct HeldTask {
    pub task: Task,
    pub json: String,
    /// The summary's object and size, when it is a regular file.
    pub summary: Option<(String, usize)>,
}

/// The board at one commit.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// By id.
    pub tasks: BTreeMap<Slug, HeldTask>,
    /// By path.
    pub quarantined: Vec<QuarantinedFile>,
    /// Task files set aside that are still regular-file blobs named once
    /// by their own id (one too large, or of a newer schema, say): carried
    /// forward as they are, so no write loses them and they stay set
    /// aside.
    pub kept: BTreeMap<String, String>,
}

impl Board {
    pub fn is_quarantined(&self, id: &Slug) -> Option<&QuarantinedFile> {
        self.quarantined
            .iter()
            .find(|file| file.id.as_ref() == Some(id))
    }
}

/// One entry of a listed tree.
struct Entry {
    mode: String,
    oid: String,
    name: String,
}

/// An object's real type and size, by `cat-file --batch-check`.
#[derive(Clone, PartialEq, Eq)]
struct Object {
    kind: String,
    size: usize,
}

/// A `<id>.json` or `<id>.md` name: its id and whether it is the task.
fn task_name(name: &str) -> Option<(Slug, bool)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let json = match extension {
        "json" => true,
        "md" => false,
        _ => return None,
    };
    Slug::parse(stem).ok().map(|id| (id, json))
}

/// The board at `commit`.
///
/// # Errors
/// git fails, or a tree is past its bounds.
pub fn read_board(git: &Git, commit: &str) -> Result<Board, BoardStoreError> {
    let root = listed(git, &format!("{commit}^{{tree}}"))?;
    let tasks = root
        .iter()
        .find(|entry| entry.name == "tasks" && entry.mode == TREE);
    match tasks {
        Some(entry) => read_tasks(git, &entry.oid),
        None => Ok(Board::default()),
    }
}

/// The root of `commit` holds only what a board holds: its README and its
/// `tasks/` tree.
///
/// # Errors
/// git fails, or the tree is past its bounds.
pub fn is_board(git: &Git, commit: &str) -> Result<bool, BoardStoreError> {
    let root = listed(git, &format!("{commit}^{{tree}}"))?;
    let objects = objects(git, root.iter().map(|entry| entry.oid.as_str()))?;
    let fits = |entry: &Entry| match (entry.name.as_str(), objects.get(&entry.oid)) {
        ("README.md", Some(object)) => entry.mode == REGULAR_FILE && object.kind == "blob",
        ("tasks", Some(object)) => entry.mode == TREE && object.kind == "tree",
        _ => false,
    };
    Ok(root.iter().any(|entry| entry.name == "README.md") && root.iter().all(fits))
}

fn read_tasks(git: &Git, tree: &str) -> Result<Board, BoardStoreError> {
    let entries = listed(git, tree)?;
    if entries.len() > MAX_TASK_ENTRIES {
        return Err(BoardStoreError::Unavailable(format!(
            "the board's tasks tree holds {} entries; the most is {MAX_TASK_ENTRIES}",
            entries.len()
        )));
    }
    let objects = objects(git, entries.iter().map(|entry| entry.oid.as_str()))?;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &entries {
        *counts.entry(entry.name.as_str()).or_default() += 1;
    }
    let mut board = Board::default();
    let set_aside = |board: &mut Board, entry: &Entry, reason: String| {
        board.quarantined.push(QuarantinedFile {
            path: format!("tasks/{}", entry.name),
            id: task_name(&entry.name).map(|(id, _)| id),
            reason,
        });
    };
    let (mut to_read, mut summaries) = (vec![], BTreeMap::new());
    for entry in &entries {
        let object = objects.get(&entry.oid);
        let regular = entry.mode == REGULAR_FILE && object.is_some_and(|o| o.kind == "blob");
        match (task_name(&entry.name), regular, counts[entry.name.as_str()]) {
            (None, _, _) => set_aside(
                &mut board,
                entry,
                "not a task file: only <id>.json and <id>.md belong in tasks/".into(),
            ),
            (Some(_), false, _) => set_aside(&mut board, entry, "not a regular file".into()),
            (Some(_), true, 2..) => {
                set_aside(&mut board, entry, "named more than once in the tree".into())
            }
            (Some((id, false)), true, _) => {
                let size = object.map_or(usize::MAX, |object| object.size);
                summaries.insert(id, (entry.oid.clone(), size));
            }
            (Some((id, true)), true, _) => {
                let size = object.map_or(usize::MAX, |object| object.size);
                match size <= MAX_TASK_FILE_BYTES {
                    true => to_read.push((id, entry, size)),
                    false => {
                        board.kept.insert(entry.name.clone(), entry.oid.clone());
                        set_aside(
                            &mut board,
                            entry,
                            format!("task file is {size} bytes; the most is {MAX_TASK_FILE_BYTES}"),
                        );
                    }
                }
            }
        }
    }
    let total: usize = to_read.iter().map(|(_, _, size)| size).sum();
    if total > MAX_BOARD_BYTES {
        return Err(BoardStoreError::Unavailable(format!(
            "the board's task files hold {total} bytes; the most read is {MAX_BOARD_BYTES}"
        )));
    }
    let oids: Vec<&str> = to_read
        .iter()
        .map(|(_, entry, _)| entry.oid.as_str())
        .collect();
    let contents = contents(git, &oids)?;
    for (id, entry, _) in to_read {
        // Two files may share one blob, so each is looked up, not taken.
        let bytes = contents.get(&entry.oid).cloned().unwrap_or_default();
        let read = decode(&bytes)
            .map_err(|error| error.to_string())
            .and_then(|task| match task.fields().id == id {
                true => Ok(task),
                false => Err(format!(
                    "holds task {:?}, not {:?}",
                    task.fields().id.as_str(),
                    id.as_str()
                )),
            });
        match read {
            Ok(task) => {
                let summary = summaries.get(&id).cloned();
                board.tasks.insert(
                    id,
                    HeldTask {
                        task,
                        json: entry.oid.clone(),
                        summary,
                    },
                );
            }
            Err(reason) => {
                board.kept.insert(entry.name.clone(), entry.oid.clone());
                set_aside(&mut board, entry, reason);
            }
        }
    }
    // A task with any part set aside is not read.
    let aside: BTreeSet<Slug> = board
        .quarantined
        .iter()
        .filter_map(|file| file.id.clone())
        .collect();
    board.tasks.retain(|id, _| !aside.contains(id));
    board
        .quarantined
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(board)
}

/// The entries of `tree`, whose size is checked before it is listed.
fn listed(git: &Git, tree: &str) -> Result<Vec<Entry>, BoardStoreError> {
    let size = git
        .text(&["cat-file", "-s", tree], None)
        .map_err(unavailable)?;
    match size.parse::<usize>() {
        Ok(size) if size <= MAX_TREE_BYTES => {}
        _ => {
            return Err(BoardStoreError::Unavailable(format!(
                "the board tree {tree} is {size} bytes; the most read is {MAX_TREE_BYTES}"
            )));
        }
    }
    let listing = git
        .ok(&["ls-tree", "-z", tree], None)
        .map_err(unavailable)?;
    listing
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
        .map(entry)
        .collect()
}

fn entry(row: &[u8]) -> Result<Entry, BoardStoreError> {
    let row = String::from_utf8_lossy(row);
    let malformed = || BoardStoreError::Unavailable(format!("unexpected ls-tree row {row:?}"));
    let (meta, name) = row.split_once('\t').ok_or_else(malformed)?;
    match meta.split(' ').collect::<Vec<_>>().as_slice() {
        [mode, _kind, oid] => Ok(Entry {
            mode: (*mode).into(),
            oid: object_id(oid)?,
            name: name.into(),
        }),
        _ => Err(malformed()),
    }
}

/// The real type and size of each object, from one `cat-file --batch-check`.
fn objects<'a>(
    git: &Git,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, Object>, BoardStoreError> {
    let input: String = oids.map(|oid| format!("{oid}\n")).collect();
    if input.is_empty() {
        return Ok(BTreeMap::new());
    }
    let format = "--batch-check=%(objectname) %(objecttype) %(objectsize)";
    let listing = git
        .text(&["cat-file", format], Some(input.as_bytes()))
        .map_err(unavailable)?;
    let mut found = BTreeMap::new();
    for row in listing.lines() {
        if let [oid, kind, size] = row.split(' ').collect::<Vec<_>>().as_slice() {
            let size = size.parse().map_err(|_| {
                BoardStoreError::Unavailable(format!("unexpected cat-file row {row:?}"))
            })?;
            found.insert(
                object_id(oid)?,
                Object {
                    kind: (*kind).into(),
                    size,
                },
            );
        }
    }
    Ok(found)
}

/// The bytes of each blob, from one `cat-file --batch`; the caller checked
/// every size first.
pub fn contents(git: &Git, oids: &[&str]) -> Result<BTreeMap<String, Vec<u8>>, BoardStoreError> {
    if oids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
    let args = ["cat-file", "--batch"];
    let output = git
        .run(&args, Some(input.as_bytes()), git.local_timeout)
        .map_err(unavailable)?;
    if !output.status.success() {
        return Err(unavailable(failed(&args, &output)));
    }
    let malformed = || BoardStoreError::Unavailable("unexpected cat-file --batch output".into());
    let (mut rest, mut found) = (output.stdout.as_slice(), BTreeMap::new());
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(malformed)?;
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        let fields: Vec<&str> = header.split(' ').collect();
        let [oid, "blob", size] = fields.as_slice() else {
            return Err(BoardStoreError::Unavailable(format!(
                "cat-file read {header:?}"
            )));
        };
        let size: usize = size.parse().map_err(|_| malformed())?;
        let body = rest.get(..size).ok_or_else(malformed)?;
        found.insert(object_id(oid)?, body.to_vec());
        rest = rest.get(size + 1..).ok_or_else(malformed)?;
    }
    Ok(found)
}

/// A full SHA-1 or SHA-256 object name.
pub fn object_id(text: &str) -> Result<String, BoardStoreError> {
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
