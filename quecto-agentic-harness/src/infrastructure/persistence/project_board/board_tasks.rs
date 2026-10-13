//! Reading a board's `tasks/` tree (`board_tree`) in two steps: a scan
//! lists it and judges every entry by its real type and size alone, and
//! only once every folder's scan keeps within what a read takes are the
//! task files' bytes read and decoded.
use super::entries::{Disposition, TASK_NAMING, TaskFile, disposition, task_name};
use super::{
    Board, Entry, Found, HeldTask, Limits, MAX_NAME_BYTES, MAX_READ_ENTRIES, REGULAR_FILE, TASKS,
    TreeCopy, Unread, contents, entry_path, kind_of_mode, listed, looked_up, missing, objects,
    task_file, too_large,
};
use crate::application::project_board::ports::{ArtifactKey, QuarantinedFile};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::persistence::project_board::artifacts::decode;
use crate::infrastructure::persistence::project_board::artifacts::task::TaskCodec;
use crate::infrastructure::persistence::project_board::git::{capped, sanitised};
use std::collections::{BTreeMap, BTreeSet};

/// `tasks/` scanned: the board as far as types and sizes tell, and the
/// task files still to read (id, entry), their bytes counted in
/// `board.read_bytes`.
pub(super) struct TasksScan {
    board: Board,
    to_read: Vec<(Slug, Entry)>,
}

impl TasksScan {
    /// The task-file bytes a load reads.
    pub(super) fn bytes(&self) -> usize {
        self.board.read_bytes.values().sum()
    }

    /// The entries the scan listed.
    pub(super) fn entries(&self) -> usize {
        self.board.listed
    }
}

fn set_aside(board: &mut Board, entry: &Entry, id: Option<Slug>, reason: &str, carried: bool) {
    board.quarantined.push(QuarantinedFile {
        path: entry_path("tasks", &entry.raw, &entry.oid),
        key: id.map(ArtifactKey::Task),
        // A file's own text (an id the codec refuses, say) can reach the
        // reason: only what shows as itself is kept, capped.
        reason: capped(&sanitised(reason.as_bytes())),
        object: entry.oid.clone(),
        carried,
    });
}

fn keep(board: &mut Board, entry: &Entry) {
    // A clear finds a kept name by the path the listing shows.
    assert_eq!(
        entry_path("tasks", &entry.raw, &entry.oid),
        format!("{TASKS}{}", entry.name),
        "a kept name shows as it is"
    );
    let kind = kind_of_mode(&entry.mode);
    debug_assert!(kind.is_some(), "only entries of git's own modes are kept");
    let kind = kind.unwrap_or("blob");
    board.kept.insert(
        entry.name.clone(),
        format!("{} {kind} {}", entry.mode, entry.oid),
    );
}

/// `tasks/` at `tree`, scanned: listed within [`MAX_READ_ENTRIES`], every
/// entry judged by its type and size, and its task files' bytes within
/// `limits.tasks_bytes`. No task file's bytes are read.
///
/// # Errors
/// Objects this clone does not hold; git fails, or the tree is past its
/// bounds.
pub(super) fn scan_tasks(
    git: &super::Git,
    tree: &str,
    limits: Limits,
) -> Result<TasksScan, Unread> {
    let entries = listed(git, tree, limits.tree_bytes)?;
    if entries.len() > MAX_READ_ENTRIES {
        return Err(too_large(format!(
            "the board's tasks tree holds {} entries; the most is {MAX_READ_ENTRIES}",
            entries.len()
        )));
    }
    let kinds = objects(git, looked_up(&entries))?;
    let missing = missing(&entries, &kinds);
    if !missing.is_empty() {
        return Err(Unread::Missing(missing));
    }
    // Copies of each name a write would carry: a copy it drops (a gitlink,
    // a mode over another type) makes no duplicate.
    let mut counts: BTreeMap<&[u8], usize> = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.carried(&kinds)) {
        *counts.entry(entry.raw.as_slice()).or_default() += 1;
    }
    let count = |entry: &Entry| counts.get(entry.raw.as_slice()).copied().unwrap_or(0);
    let found = |entry: &Entry| match kinds.get(&entry.oid) {
        Some(Found::Present(object)) => Some(object),
        Some(Found::Missing) | None => None,
    };
    let mut board = Board {
        listed: entries.len(),
        ..Board::default()
    };
    let mut to_read = vec![];
    for entry in &entries {
        let file = task_name(&entry.name);
        match disposition(entry, found(entry), count(entry), file, &TASK_NAMING) {
            Disposition::Read(id) => {
                let size = found(entry).map_or(0, |object| object.size);
                board.read_bytes.insert(entry.name.clone(), size);
                to_read.push((id, entry.clone()));
            }
            Disposition::Aside { id, reason, kept } => {
                if kept {
                    keep(&mut board, entry);
                }
                set_aside(&mut board, entry, id, &reason, kept);
            }
        }
    }
    // Only names a write would carry: a stray is dropped, however often
    // it is named.
    let mut repeated: BTreeMap<&str, Vec<TreeCopy>> = BTreeMap::new();
    for entry in &entries {
        let record = task_name(&entry.name).is_some();
        let named_twice = entry.kept_name(MAX_NAME_BYTES) && record && count(entry) > 1;
        match (named_twice, found(entry)) {
            (true, Some(object)) if entry.carried(&kinds) => {
                repeated.entry(&entry.name).or_default().push(TreeCopy {
                    mode: entry.mode.clone(),
                    kind: object.kind.clone(),
                    oid: entry.oid.clone(),
                    size: object.size,
                });
            }
            _ => {}
        }
    }
    board.repeated = repeated
        .into_iter()
        .map(|(name, oids)| (name.to_string(), oids))
        .collect();
    let scan = TasksScan { board, to_read };
    match scan.bytes() <= limits.tasks_bytes {
        true => Ok(scan),
        false => Err(too_large(format!(
            "the board's task files hold {} bytes; the most read is {}",
            scan.bytes(),
            limits.tasks_bytes
        ))),
    }
}

/// The board `scan` found, its task files read and decoded.
///
/// # Errors
/// git fails to give the files' bytes.
pub(super) fn load_tasks(git: &super::Git, scan: TasksScan) -> Result<Board, Unread> {
    let TasksScan { mut board, to_read } = scan;
    let wanted: Vec<(&str, usize)> = to_read
        .iter()
        .map(|(_, entry)| {
            let size = board.read_bytes.get(&entry.name).copied().unwrap_or(0);
            (entry.oid.as_str(), size)
        })
        .collect();
    let contents = contents(git, &wanted)?;
    // Ids with a task file in the JSON form older quecto wrote, carried
    // forward: an `.md` beside one that is no task file is the summary it
    // wrote with it.
    let legacy: BTreeSet<Slug> = board
        .kept
        .keys()
        .filter_map(|name| match task_name(name) {
            Some((Some(id), TaskFile::LegacyJson)) => Some(id),
            Some(_) | None => None,
        })
        .collect();
    for (id, entry) in to_read {
        // Two files may share one blob, so each is looked up, not taken.
        let bytes = contents.get(&entry.oid).cloned().unwrap_or_default();
        let read = decode::<TaskCodec>(&bytes)
            .map_err(|error| format!("task file: {error}"))
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
                board.tasks.insert(
                    id,
                    HeldTask {
                        task,
                        object: entry.oid.clone(),
                    },
                );
            }
            Err(reason) => {
                let reason = match legacy.contains(&id) {
                    true => "a summary older quecto wrote beside its JSON record; carried \
                             forward as it is"
                        .to_string(),
                    false => reason,
                };
                keep(&mut board, &entry);
                set_aside(&mut board, &entry, Some(id), &reason, true);
            }
        }
    }
    // A task with another file set aside under its id (one in the JSON
    // form older quecto wrote, a name listed twice) is never read
    // half-trusted: its file is set aside with it and carried as it is, so
    // clearing the other file frees it.
    let blocked: Vec<Slug> = board
        .tasks
        .keys()
        .filter(|id| board.is_quarantined(id).is_some())
        .cloned()
        .collect();
    for id in blocked {
        let Some(held) = board.tasks.remove(&id) else {
            continue;
        };
        let name = task_file(&id);
        board
            .kept
            .insert(name.clone(), format!("{REGULAR_FILE} blob {}", held.object));
        board.quarantined.push(QuarantinedFile {
            path: format!("{TASKS}{name}"),
            key: Some(ArtifactKey::Task(id)),
            reason: "another file of this task is set aside, so this one is too, carried \
                     forward as it is until that file is cleared or dropped"
                .into(),
            object: held.object,
            carried: true,
        });
    }
    assert!(
        board
            .tasks
            .keys()
            .all(|id| board.is_quarantined(id).is_none()),
        "a task read is never one set aside"
    );
    board
        .quarantined
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(board)
}
