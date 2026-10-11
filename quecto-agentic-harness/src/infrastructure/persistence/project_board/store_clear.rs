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
use crate::infrastructure::persistence::project_board::board_tree::{TASKS, TreeCopy};

impl GitBoardStore {
    pub(super) fn clear(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", message, 200).map_err(|error| invalid(error.to_string()))?;
        let kept = clear.repeated.values().filter_map(|keep| match keep {
            KeepCopy::Object(oid) => Some(oid),
            KeepCopy::DropAll => None,
        });
        for oid in clear.object.iter().chain(kept) {
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
    let objects = |copies: &[TreeCopy]| {
        let oids: Vec<&str> = copies.iter().map(|copy| copy.oid.as_str()).collect();
        oids.join(", ")
    };
    let repeated: BTreeMap<String, &[TreeCopy]> = board
        .repeated
        .iter()
        .map(|(name, copies)| (format!("{TASKS}{name}"), copies.as_slice()))
        .collect();
    let unresolved: Vec<String> = repeated
        .iter()
        .filter(|(path, _)| !clear.repeated.contains_key(*path))
        .map(|(path, copies)| format!("{path} (objects {})", objects(copies)))
        .collect();
    match unresolved.as_slice() {
        [] => {}
        _ => {
            return Err(format!(
                "every name listed more than once is resolved in the same clear, naming the copy \
                 to keep or none: {}",
                unresolved.join("; ")
            ));
        }
    }
    let mut after = board.clone();
    for (path, keep) in &clear.repeated {
        let (Some(copies), Some(name)) = (repeated.get(path), path.strip_prefix(TASKS)) else {
            return Err(format!("{} is not listed more than once", shown(path)));
        };
        match keep {
            KeepCopy::DropAll => {}
            KeepCopy::Object(oid) => {
                let Some(copy) = copies.iter().find(|copy| copy.oid == *oid) else {
                    return Err(format!(
                        "{oid} is not one of {path}'s copies a write carries ({})",
                        objects(copies)
                    ));
                };
                after.kept.insert(name.to_string(), copy.entry());
                if let Some(size) = copy.read_size() {
                    after.read_bytes.insert(name.to_string(), size);
                }
            }
        }
        after.quarantined.retain(|file| file.path != *path);
    }
    after.repeated.clear();
    match repeated.contains_key(&clear.path) {
        // Resolved with the rest above.
        true => Ok(after),
        false => {
            let file = named(board, clear)?;
            drop_file(&mut after, file);
            Ok(after)
        }
    }
}

/// The one file set aside that `clear` names by path and object.
fn named<'a>(board: &'a Board, clear: &SetAsideClear) -> Result<&'a QuarantinedFile, String> {
    let path = quoted(clear.path.as_bytes());
    let matching: Vec<&QuarantinedFile> = board
        .quarantined
        .iter()
        .filter(|file| file.path == clear.path)
        .filter(|file| {
            clear
                .object
                .as_ref()
                .is_none_or(|object| *object == file.object)
        })
        .collect();
    match matching.as_slice() {
        [file] => Ok(file),
        [] => Err(match &clear.object {
            Some(object) => format!("no file set aside at {path} has object {object}"),
            None => format!("{path} is not set aside"),
        }),
        several => {
            let objects: Vec<&str> = several.iter().map(|file| file.object.as_str()).collect();
            Err(format!(
                "{} files set aside show {path}: name the object of the one to clear, one of {}",
                several.len(),
                objects.join(", ")
            ))
        }
    }
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
