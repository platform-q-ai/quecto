//! Clearing files the board set aside (#2477): one commit without the
//! file a clear names, every other file carried forward as a write carries
//! it, made as the store's committer so the board records who cleared it.
//!
//! A file is named by its path and, when two files set aside show the same
//! path, its object. While the board lists any name more than once (a hand
//! edit git rejects), one clear resolves them all: for each, the copy to
//! keep, which stays as it was (mode and type), or none. A clear that left
//! a name listed twice would push a tree no remote that checks its objects
//! takes.
//!
//! A clear is judged against the board's head first: from a head the board
//! moved past it is lost, whatever else it says. It keeps every bound a
//! write keeps, checked before anything is built.
//!
//! The clear works on the board's own view of its files (tasks, files
//! carried as they are, names listed twice), never on one folder's
//! layout, so it holds however many folders the board grows.
use super::*;
use crate::application::project_board::ports::{KeepCopy, QuarantinedFile, SetAsideClear};
use crate::infrastructure::persistence::project_board::board_tree::{TASKS, TreeCopy, spells_id};

/// The most names a refusal lists before it says how many more there are.
const LISTED: usize = 8;

/// `items`, at most [`LISTED`] of them, then how many more.
fn listed(items: impl IntoIterator<Item = String>) -> String {
    let items: Vec<String> = items.into_iter().collect();
    let shown = items
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    match items.len().saturating_sub(LISTED) {
        0 => shown,
        more => format!("{shown}; and {more} more"),
    }
}

impl GitBoardStore {
    pub(super) fn clear(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", message, 200).map_err(|error| invalid(error.to_string()))?;
        let objects: Vec<&String> = std::iter::once(&clear.object)
            .chain(clear.repeated.values().flat_map(|keep| match keep {
                KeepCopy::Object { keep, seen } => std::iter::once(keep).chain(seen).collect(),
                KeepCopy::Drop(seen) => seen.iter().collect::<Vec<_>>(),
            }))
            .collect();
        // A board holds at most so many names listed twice, and copies.
        match (
            clear.repeated.len() <= MAX_TASK_ENTRIES / 2,
            objects.len() <= MAX_TASK_ENTRIES + 1,
        ) {
            (true, true) => {}
            _ => {
                return Err(invalid(format!(
                    "a clear resolves at most {} names and {MAX_TASK_ENTRIES} copies",
                    MAX_TASK_ENTRIES / 2
                )));
            }
        }
        for oid in objects {
            object_id(oid)
                .map_err(|_| invalid(format!("{} is not an object id", quoted(oid.as_bytes()))))?;
        }
        let base = self.base(expected)?;
        // The head first: a clear from a head the board moved past is lost,
        // whatever else it says.
        match self.remote_head(BOARD_REF)? {
            Some(head) if head == base => {}
            Some(_) => return Ok(BoardWrite::Lost),
            None => return Err(BoardStoreError::NoBoard),
        }
        let board = self.board_at(&base)?;
        let after = match cleared(&board, clear) {
            Ok(after) => after,
            Err(refusal) => return Ok(BoardWrite::Refused(refusal)),
        };
        assert!(after.repeated.is_empty(), "a clear resolves every name");
        after.within_bounds(&[], self.limits)?;
        let tasks = self.tasks_tree(&after, &[])?;
        self.land(&base, &tasks, message)
    }
}

/// `board` once `clear` is made, or why it is refused.
fn cleared(board: &Board, clear: &SetAsideClear) -> Result<Board, String> {
    let shown = |text: &str| quoted(text.as_bytes());
    let objects = |copies: &[TreeCopy]| listed(copies.iter().map(|copy| copy.oid.clone()));
    let repeated: BTreeMap<String, &[TreeCopy]> = board
        .repeated
        .iter()
        .map(|(name, copies)| (format!("{TASKS}{name}"), copies.as_slice()))
        .collect();
    let unresolved = repeated
        .iter()
        .filter(|(path, _)| !clear.repeated.contains_key(*path))
        .map(|(path, copies)| format!("{path} (objects {})", objects(copies)));
    let unresolved: Vec<String> = unresolved.collect();
    match unresolved.is_empty() {
        true => {}
        false => {
            return Err(format!(
                "every name listed more than once is resolved in the same clear, naming every \
                 copy seen and the one to keep or none: {}",
                listed(unresolved)
            ));
        }
    }
    let mut after = board.clone();
    for (path, keep) in &clear.repeated {
        let (Some(copies), Some(name)) = (repeated.get(path), path.strip_prefix(TASKS)) else {
            return Err(format!("{} is not listed more than once", shown(path)));
        };
        resolve(&mut after, path, name, copies, keep)?;
        after.quarantined.retain(|file| file.path != *path);
    }
    after.repeated.clear();
    match repeated.get(&clear.path) {
        // Resolved with the rest above; the object named is one of its copies.
        Some(copies) => match copies.iter().any(|copy| copy.oid == clear.object) {
            true => Ok(after),
            false => Err(format!(
                "{} is not one of {}'s copies ({})",
                clear.object,
                shown(&clear.path),
                objects(copies)
            )),
        },
        None => {
            let file = named(board, clear)?;
            drop_file(&mut after, file);
            Ok(after)
        }
    }
}

/// Resolves the name listed more than once at `path` in `after`: its
/// copies must be exactly the ones the clear saw, never one more, and a
/// copy kept stays as it was.
fn resolve(
    after: &mut Board,
    path: &str,
    name: &str,
    copies: &[TreeCopy],
    keep: &KeepCopy,
) -> Result<(), String> {
    let (kept, seen) = match keep {
        KeepCopy::Object { keep, seen } => (Some(keep), seen),
        KeepCopy::Drop(seen) => (None, seen),
    };
    let held: BTreeSet<&str> = copies.iter().map(|copy| copy.oid.as_str()).collect();
    let saw: BTreeSet<&str> = seen.iter().map(String::as_str).collect();
    match held == saw {
        true => {}
        false => {
            return Err(format!(
                "{path}'s copies are {}, not the ones the clear saw ({}): read the board again",
                listed(held.iter().map(|oid| (*oid).to_string())),
                listed(saw.iter().map(|oid| quoted(oid.as_bytes())))
            ));
        }
    }
    let Some(keep) = kept else {
        return Ok(());
    };
    let candidates: Vec<&TreeCopy> = copies.iter().filter(|copy| copy.oid == *keep).collect();
    let modes: BTreeSet<&str> = candidates.iter().map(|copy| copy.mode.as_str()).collect();
    match (candidates.first(), modes.len()) {
        (Some(copy), 1) => {
            after.kept.insert(name.to_string(), copy.entry());
            if let Some(size) = copy.read_size() {
                after.read_bytes.insert(name.to_string(), size);
                after.pending_summaries += usize::from(spells_id(name));
            }
            Ok(())
        }
        (Some(_), _) => Err(format!(
            "{keep} names copies of {path} of different modes ({}): which one stays would be a \
             guess; drop them all",
            listed(modes.iter().map(|mode| (*mode).to_string()))
        )),
        (None, _) => Err(format!("{keep} is not one of {path}'s copies")),
    }
}

/// The file set aside that `clear` names by path and object. Entries that
/// share both are one file: the same contents, shown at the same path.
fn named<'a>(board: &'a Board, clear: &SetAsideClear) -> Result<&'a QuarantinedFile, String> {
    let path = quoted(clear.path.as_bytes());
    board
        .quarantined
        .iter()
        .find(|file| file.path == clear.path && file.object == clear.object)
        .ok_or_else(|| format!("no file set aside at {path} has object {}", clear.object))
}
/// `board` without `file`: the file carried as it is goes when it is that
/// object, so a file of another name that shows the same path stays.
fn drop_file(board: &mut Board, file: &QuarantinedFile) {
    let name = file.path.strip_prefix(TASKS);
    assert!(name.is_some(), "every file set aside is under {TASKS}");
    let name = name.unwrap_or_default();
    let carried = board
        .kept
        .get(name)
        .and_then(|entry| entry.rsplit(' ').next());
    if carried == Some(file.object.as_str()) {
        board.kept.remove(name);
        board.read_bytes.remove(name);
    }
    board.quarantined.retain(|other| {
        (other.path.as_str(), other.object.as_str()) != (file.path.as_str(), file.object.as_str())
    });
}
