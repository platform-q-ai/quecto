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
//! layout, so it holds however many folders the board grows. A file set
//! aside in another folder is cleared alone (#2481 S10a): its folder is
//! rebuilt without it. A name listed twice there (S10b) is resolved as one
//! in `tasks/` is: the clear names the copy it keeps, which joins what its
//! folder carries, or that none is, and the folder is rebuilt; quecto
//! never picks a copy itself.
use super::*;
use crate::application::project_board::ports::{KeepCopy, QuarantinedFile, SetAsideClear};
use crate::infrastructure::persistence::project_board::board_tree::{
    ClearKeeps, FolderAfter, MAX_BOARD_ENTRIES, MAX_TASK_FILE_BYTES, TASK_NAMING, TASKS, TreeCopy,
    is_folder, read_size_in, spells_id,
};

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
        let seen = |keep: &KeepCopy| match keep {
            KeepCopy::Object { seen, .. } | KeepCopy::Drop(seen) => seen.len(),
        };
        // A board a read takes holds at most so many entries in all its
        // folders, each name listed twice two of them, and one clear
        // resolves every such name: what a clear saw is bounded by them;
        // the copy it keeps is one it saw, not counted again.
        let saw: usize = clear.repeated.values().map(seen).sum();
        match (
            clear.repeated.len() <= MAX_BOARD_ENTRIES / 2,
            saw <= MAX_BOARD_ENTRIES,
        ) {
            (true, true) => {}
            _ => {
                return Err(invalid(format!(
                    "a clear resolves at most {} names and {MAX_BOARD_ENTRIES} copies",
                    MAX_BOARD_ENTRIES / 2
                )));
            }
        }
        let objects = std::iter::once(&clear.object).chain(clear.repeated.values().flat_map(
            |keep| match keep {
                KeepCopy::Object { keep, seen } => std::iter::once(keep).chain(seen).collect(),
                KeepCopy::Drop(seen) => seen.iter().collect::<Vec<_>>(),
            },
        ));
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
        let (after, keeps) = match cleared(&board, clear) {
            Ok(cleared) => cleared,
            Err(refusal) => return Ok(BoardWrite::Refused(refusal)),
        };
        // So every folder is rebuilt or carried as a write leaves it.
        let resolved = after.folders.values().all(|tree| tree.repeated.is_empty());
        assert!(
            after.repeated.is_empty() && resolved,
            "a clear resolves every name"
        );
        // A clear is a write: the other folders drop what a write drops.
        let folders = after.folders_after(&[]);
        // A folder that listed a name twice is rebuilt, holding it once.
        for (name, tree) in &board.folders {
            let rebuilt = matches!(folders.get(name), Some(FolderAfter::Rebuilt { .. }));
            assert!(
                tree.repeated.is_empty() || rebuilt,
                "{name}/ listed a name twice and is rebuilt"
            );
        }
        after.within_clear_bounds(&board, &folders, self.limits, &keeps)?;
        let tasks = self.tasks_tree(&after, &[])?;
        let folders = self.folder_trees(&folders)?;
        self.land(&base, &after, &tasks, folders, message)
    }
}

/// Where a name listed more than once lies: its folder (`tasks` among
/// them), its name there and its copies.
type Repeated<'a> = (&'static str, &'a str, &'a [TreeCopy]);

/// Every name the board lists more than once, by the path its listing
/// shows: in `tasks/`, then in each other folder.
fn repeated_names(board: &Board) -> BTreeMap<String, Repeated<'_>> {
    let tasks = board.repeated.iter().map(|(name, copies)| {
        let place = (TASK_NAMING.folder, name.as_str(), copies.as_slice());
        (format!("{TASKS}{name}"), place)
    });
    let folders = board.folders.iter().flat_map(|(folder, tree)| {
        tree.repeated.iter().map(move |(name, copies)| {
            let place = (*folder, name.as_str(), copies.as_slice());
            (format!("{folder}/{name}"), place)
        })
    });
    tasks.chain(folders).collect()
}

/// `board` once `clear` is made, and what it keeps, or why it is refused.
pub(crate) fn cleared(board: &Board, clear: &SetAsideClear) -> Result<(Board, ClearKeeps), String> {
    let shown = |text: &str| quoted(text.as_bytes());
    let objects = |copies: &[TreeCopy]| listed(copies.iter().map(|copy| copy.oid.clone()));
    let repeated = repeated_names(board);
    let unresolved = repeated
        .iter()
        .filter(|(path, _)| !clear.repeated.contains_key(*path))
        .map(|(path, (_, _, copies))| format!("{path} (objects {})", objects(copies)));
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
    // Every file set aside, by path, grouped once: a clear may name
    // thousands of paths.
    let mut by_path: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for file in &board.quarantined {
        by_path
            .entry(file.path.as_str())
            .or_default()
            .insert(file.object.as_str());
    }
    let none = BTreeSet::new();
    let listed_at = |path: &str| by_path.get(path).unwrap_or(&none);
    let mut after = board.clone();
    let mut kept: Vec<String> = vec![];
    for (path, keep) in &clear.repeated {
        let Some((folder, name, copies)) = repeated.get(path) else {
            return Err(format!("{} is not listed more than once", shown(path)));
        };
        if let Some(copy) = resolve(path, copies, listed_at(path), keep)? {
            keep_copy(&mut after, folder, name, copy);
            kept.push(path.clone());
        }
    }
    // The copies of the names this clear resolved leave the listing; every
    // other file set aside stays listed as it was.
    let (resolved, left): (Vec<QuarantinedFile>, Vec<QuarantinedFile>) =
        std::mem::take(&mut after.quarantined)
            .into_iter()
            .partition(|file| clear.repeated.contains_key(&file.path));
    debug_assert!(
        resolved
            .iter()
            .all(|file| repeated.contains_key(&file.path)),
        "a clear resolves only names listed twice"
    );
    after.quarantined = left;
    let keeps = match kept.is_empty() {
        true => ClearKeeps::Nothing,
        false => ClearKeeps::Copies(kept),
    };
    after.repeated.clear();
    for tree in after.folders.values_mut() {
        tree.repeated.clear();
    }
    match repeated.get(&clear.path) {
        // Resolved with the rest above; the object named is one of its copies.
        Some(_) => match listed_at(&clear.path).contains(clear.object.as_str()) {
            true => Ok((after, keeps)),
            false => Err(format!(
                "{} is not one of the files set aside at {}",
                clear.object,
                shown(&clear.path)
            )),
        },
        None => {
            let file = named(board, clear)?;
            drop_file(&mut after, file);
            Ok((after, keeps))
        }
    }
}

/// The copy of the name listed more than once at `path` that a clear
/// keeps, if any: its copies must be exactly the ones the clear saw, never
/// one more, and a copy kept stays as it was.
fn resolve<'a>(
    path: &str,
    copies: &'a [TreeCopy],
    shown: &BTreeSet<&str>,
    keep: &KeepCopy,
) -> Result<Option<&'a TreeCopy>, String> {
    let (kept, seen) = match keep {
        KeepCopy::Object { keep, seen } => (Some(keep), seen),
        KeepCopy::Drop(seen) => (None, seen),
    };
    debug_assert!(
        copies.iter().all(|copy| shown.contains(copy.oid.as_str())),
        "every carried copy of {path} is a file set aside there"
    );
    let saw: BTreeSet<&str> = seen.iter().map(String::as_str).collect();
    match *shown == saw {
        true => {}
        false => {
            return Err(format!(
                "the files set aside at {path} are {}, not the ones the clear saw ({}): read the \
                 board again",
                listed(shown.iter().map(|oid| (*oid).to_string())),
                listed(saw.iter().map(|oid| quoted(oid.as_bytes())))
            ));
        }
    }
    let Some(keep) = kept else {
        return Ok(None);
    };
    let candidates: Vec<&TreeCopy> = copies.iter().filter(|copy| copy.oid == *keep).collect();
    let modes: BTreeSet<&str> = candidates.iter().map(|copy| copy.mode.as_str()).collect();
    match (candidates.first(), modes.len()) {
        (Some(copy), 1) => Ok(Some(copy)),
        (Some(_), _) => Err(format!(
            "{keep} names copies of {path} of different modes ({}): which one stays would be a \
             guess; drop them all",
            listed(modes.iter().map(|mode| (*mode).to_string()))
        )),
        (None, _) => Err(format!(
            "{keep} is not one of {path}'s copies a write carries, the only ones a clear keeps"
        )),
    }
}

/// `board` with `copy` kept as `name` in `folder`, as it was: carried by
/// the write, and counted as a read reads it when a read will.
fn keep_copy(board: &mut Board, folder: &str, name: &str, copy: &TreeCopy) {
    match folder == TASK_NAMING.folder {
        true => {
            board.kept.insert(name.to_string(), copy.entry());
            // A read reads it, as a task, only when its name spells an id.
            if let (Some(size), true) = (copy.read_size(MAX_TASK_FILE_BYTES), spells_id(name)) {
                board.read_bytes.insert(name.to_string(), size);
            }
        }
        false => {
            let size = read_size_in(folder, name, copy);
            // A name listed twice lies in one of the folders a read read.
            match (is_folder(folder), board.folders.get_mut(folder)) {
                (true, Some(tree)) => {
                    tree.files.insert(name.to_string(), copy.entry());
                    if let Some(size) = size {
                        tree.sizes.insert(name.to_string(), size);
                    }
                }
                (true, None) | (false, _) => {
                    unreachable!("a name listed twice lies in a folder read: {folder}/")
                }
            }
        }
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
/// object, in `tasks/` or in the folder it lies in (which the write then
/// rebuilds without it). A carried name always shows exactly as it is (the
/// read asserts it), so the shown path finds it; a stray that shows the
/// same path is never carried, and the write drops it.
fn drop_file(board: &mut Board, file: &QuarantinedFile) {
    let split = file.path.split_once('/');
    let (folder, name) = split.unwrap_or_default();
    // A read sets aside files only in `tasks/` and the folders it read.
    assert!(
        folder == TASK_NAMING.folder || board.folders.contains_key(folder),
        "a file set aside lies in a folder the read read: {}",
        file.path
    );
    let object = |entry: &String| entry.rsplit(' ').next() == Some(file.object.as_str());
    match (folder == TASK_NAMING.folder, board.folders.get_mut(folder)) {
        (true, _) => {
            if board.kept.get(name).is_some_and(object) {
                board.kept.remove(name);
                board.read_bytes.remove(name);
            }
        }
        (false, Some(tree)) => {
            if tree.files.get(name).is_some_and(object) {
                tree.files.remove(name);
                tree.sizes.remove(name);
            }
        }
        (false, None) => {}
    }
    board.quarantined.retain(|other| {
        (other.path.as_str(), other.object.as_str()) != (file.path.as_str(), file.object.as_str())
    });
}
