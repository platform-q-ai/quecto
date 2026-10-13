//! Reading one board commit's trees, which anyone with push access may
//! have written, so every part is checked before it is trusted:
//! - each tree's real type and size, before it is listed;
//! - each entry's real object type and size, in one `cat-file
//!   --batch-check`, before its bytes are read;
//! - the task files' bytes, in one `cat-file --batch`.
//!
//! A board's root holds its README (each write regenerates it), a `tasks`
//! tree, the other kinds' folders and an `archive` tree (`board_folders`,
//! #2481 S9), each a tree named once, and maybe stray files (each write
//! drops them). A root with anything else, any other directory above all,
//! or with neither a README nor a `tasks` tree, is someone's code, not a
//! board: reading, writing and bootstrapping all take the same view of it
//! ([`Unread::Failed`] with a clash).
//!
//! In `tasks/`, only names quecto keeps are anything but strays: ASCII
//! letters, digits and `-_.+=@,`, starting with a letter or digit, no
//! longer than an id and its extension ([`MAX_NAME_BYTES`]), so `mktree`
//! takes every name a write carries, every one shows as itself, and no
//! write makes a tree a read refuses.
//!
//! A regular file `<id>.md` named once is a task (#2481 S8): it is read
//! when it decodes as the task codec's canonical file of the task its name
//! spells. One that does not (edited by hand, of a newer schema, naming
//! another task), any other `.md` file of a kept name, and a `<id>.json`
//! task file of the JSON form older quecto wrote, are set aside (under
//! their id, when the name spells one, so the id stays blocked until a
//! clear) and carried forward by every write as they are, when the object
//! is what the mode names: a file or a link (100644, 100755, 120000) over a
//! blob, or a directory (040000) over a tree. Anything else is dropped by
//! the next write: a gitlink (160000: `mktree` refuses one over an object
//! of another type, and git reads any odd mode as one), or a mode over an
//! object of another type (git's transport refuses such a tree). A task
//! file named twice refuses every write (see [`Board::repeated`]). Any
//! other entry is a stray, set aside with no id, and dropped by the next
//! write.
//!
//! A board's root also holds its format marker (#2527): a `format` tree of
//! one regular file, `board`, reading [`FORMAT`]. A quecto before the task
//! files became markdown refuses any root tree but `tasks` as a clash, so
//! it never reads, and never writes over, a board whose tasks it would not
//! see. Every write adds the marker; a root without one is a board written
//! before it, read as it is; a marker of a newer format asks for an
//! upgrade.
//!
//! An object a partial clone does not hold yet is not fetched here (every
//! local call runs with `GIT_NO_LAZY_FETCH`): the read answers
//! [`Unread::Missing`] with every object it found missing, so the store
//! fetches them in one call and reads again.
use super::artifacts::task::TASK_LIMITS;
pub use super::cat_file::contents;
use super::cat_file::{Found, Object, objects};
use super::git::{Git, GitError, capped, quoted, sanitised, shown_name};
use crate::application::project_board::ports::{BoardArtifact, BoardStoreError, QuarantinedFile};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::shown_text::shown_board_path;
use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use std::collections::BTreeMap;

/// The punctuation a name kept in `tasks/` may hold besides letters and
/// numbers.
const NAME_PUNCTUATION: &str = "-_.+=@,";
/// The longest name kept in `tasks/`: an id and its five-byte `.json`,
/// the longer of the two extensions a task file has had.
pub const MAX_NAME_BYTES: usize = MAX_SLUG_BYTES + 5;
/// The most entries a write leaves in the board's `tasks/` tree: its
/// tasks, one file each, and the files set aside.
pub const MAX_TASK_ENTRIES: usize = 2_048;
/// The most entries a read takes in `tasks/`: a board written before the
/// task files became markdown holds a JSON file and a summary per task,
/// up to twice as many as a write leaves.
pub const MAX_READ_ENTRIES: usize = 2 * MAX_TASK_ENTRIES;
/// The largest task file read or written: the task codec's cap.
pub const MAX_TASK_FILE_BYTES: usize = TASK_LIMITS.file_bytes;
/// The largest tree read: [`MAX_READ_ENTRIES`] entries with long names.
pub const MAX_TREE_BYTES: usize = MAX_READ_ENTRIES * 160;
/// The most `ls-tree` prints for a tree of [`MAX_TREE_BYTES`]: each entry
/// of at least 28 bytes grows by at most 53 when its object name is
/// spelled in hex beside its type.
const MAX_LISTING_BYTES: usize = 4 * MAX_TREE_BYTES;
/// The most file bytes one read takes in all the board's folders.
pub const MAX_BOARD_BYTES: usize = 64 * 1024 * 1024;
/// The most task-file bytes one read takes in `tasks/` (#2481 M3).
pub const MAX_TASKS_BYTES: usize = 16 * 1024 * 1024;
pub const REGULAR_FILE: &str = "100644";
pub const TREE: &str = "040000";
const EXECUTABLE: &str = "100755";
const SYMLINK: &str = "120000";
const GITLINK: &str = "160000";

/// What a store error says, as text to quote in another.
pub fn told(error: &BoardStoreError) -> &str {
    match error {
        BoardStoreError::NoBoard => "there is no board",
        BoardStoreError::Clash(why)
        | BoardStoreError::InvalidWrite(why)
        | BoardStoreError::Unavailable(why)
        | BoardStoreError::TooLarge(why)
        | BoardStoreError::FolderSetAside(why) => why,
    }
}

/// What one read of a board takes at most; a write keeps within it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// File bytes read in all the board's folders.
    pub board_bytes: usize,
    /// Task-file bytes read in `tasks/`.
    pub tasks_bytes: usize,
    /// The bytes of one tree.
    pub tree_bytes: usize,
}

pub const LIMITS: Limits = Limits {
    board_bytes: MAX_BOARD_BYTES,
    tasks_bytes: MAX_TASKS_BYTES,
    tree_bytes: MAX_TREE_BYTES,
};

pub fn unavailable(error: GitError) -> BoardStoreError {
    BoardStoreError::Unavailable(error.to_string())
}

/// Why a board commit was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// Objects the commit names that this clone does not hold (a partial
    /// clone): fetch them, then read again.
    Missing(Vec<String>),
    Failed(BoardStoreError),
}

impl From<BoardStoreError> for Unread {
    fn from(error: BoardStoreError) -> Self {
        Self::Failed(error)
    }
}

/// A task as the board holds it: the task and its file's object, which a
/// write that leaves the task as it is carries.
#[derive(Debug, Clone)]
pub struct HeldTask {
    pub task: Task,
    pub object: String,
}

/// The board at one commit.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// By id.
    pub tasks: BTreeMap<Slug, HeldTask>,
    /// By path.
    pub quarantined: Vec<QuarantinedFile>,
    /// Task files set aside that every write carries forward as they are:
    /// by name, their tree entry (`<mode> <type> <oid>`).
    pub kept: BTreeMap<String, String>,
    /// Task file names listed more than once, each with every copy a write
    /// would carry. git rejects such a tree (`fsck`: duplicateEntries, run
    /// by GitHub and any remote with `receive.fsckObjects` on every push),
    /// so carrying all of them would make every later push fail, and
    /// keeping one would drop the rest unseen: every write is refused until
    /// a clear (`clear_set_aside`) resolves them all, naming the copy each
    /// keeps or that none is.
    pub repeated: Vec<(String, Vec<TreeCopy>)>,
    /// The size of every task file a read of this board reads, by name:
    /// what a write checks against the read bound.
    pub read_bytes: BTreeMap<String, usize>,
    /// The entries a read listed in `tasks/`, strays included.
    pub listed: usize,
    /// The other kinds' artifacts read, each under its own key.
    pub artifacts: Vec<BoardArtifact>,
    /// The folders besides `tasks/` as read, by name: a write carries each
    /// it leaves as it was as this tree, and rebuilds the rest from it.
    pub folders: BTreeMap<&'static str, CarriedTree>,
    /// The archive's tree, never read: every write carries it as it is.
    pub archive: Option<String>,
}

/// One copy of a name a write would carry, as its tree entry holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeCopy {
    pub mode: String,
    pub kind: String,
    pub oid: String,
    pub size: usize,
}

impl TreeCopy {
    /// The tree entry a write carries it by: `<mode> <type> <oid>`.
    pub fn entry(&self) -> String {
        format!("{} {} {}", self.mode, self.kind, self.oid)
    }

    /// Its size, when a read of the board reads it: a regular file of at
    /// most its kind's cap, `file_bytes`.
    pub fn read_size(&self, file_bytes: usize) -> Option<usize> {
        let read = self.mode == REGULAR_FILE && self.kind == "blob";
        (read && self.size <= file_bytes).then_some(self.size)
    }
}

/// Whether `name` is a `<id>.md` a read reads as a task.
pub fn spells_id(name: &str) -> bool {
    matches!(task_name(name), Some((Some(_), TaskFile::Markdown)))
}

/// The name a task's file has in `tasks/`.
pub fn task_file(id: &Slug) -> String {
    format!("{}.md", id.as_str())
}

/// Where a board keeps its task files.
pub const TASKS: &str = "tasks/";

/// How the README and a refusal say to undo a hand edit.
pub const RECOVERY: &str = "move `quecto/board` back to the commit before that edit";

impl Board {
    /// Why no write is taken, when a task file's name is listed twice.
    pub fn repeated_refusal(&self, head: &str) -> Option<String> {
        let (name, copies) = self.repeated.first()?;
        let path = format!("tasks/{}", quoted(name.as_bytes()));
        Some(named_twice(&path, head, copies, "no write"))
    }

    pub fn is_quarantined(&self, id: &Slug) -> Option<&QuarantinedFile> {
        self.quarantined.iter().find(|file| file.id() == Some(id))
    }
}

/// Why `writes` (every write, or every write in one folder) is refused
/// while `path` is listed more than once, as `copies`, in board commit
/// `head`.
pub(super) fn named_twice(path: &str, head: &str, copies: &[TreeCopy], writes: &str) -> String {
    let oids: Vec<&str> = copies.iter().map(|copy| copy.oid.as_str()).collect();
    format!(
        "{path} is named more than once in board commit {head} (objects {}). quecto never \
         writes such a tree, so a hand edit did, and git rejects it (fsck: duplicateEntries): \
         {writes} can carry it forward, and quecto will not choose a copy. To mend the board, \
         clear it (clear set-aside), naming the copy to keep for every name listed more than \
         once, or {RECOVERY}",
        oids.join(", ")
    )
}

/// One entry of a listed tree.
#[derive(Debug, Clone)]
struct Entry {
    mode: String,
    oid: String,
    /// As shown: invalid UTF-8 replaced.
    name: String,
    /// As the tree holds it.
    raw: Vec<u8>,
}

impl Entry {
    /// Whether quecto keeps this name: ASCII letters, digits and
    /// [`NAME_PUNCTUATION`], starting with a letter or digit, at most
    /// `max_name` bytes (its folder's longest name). Every id fits; no path
    /// separator, no bracket, no invisible or combining character, so
    /// `mktree` takes every name a write carries and every one shows as
    /// itself.
    fn kept_name(&self, max_name: usize) -> bool {
        let allowed = |b: &u8| b.is_ascii_alphanumeric() || NAME_PUNCTUATION.as_bytes().contains(b);
        self.raw.len() <= max_name
            && self.raw.first().is_some_and(u8::is_ascii_alphanumeric)
            && self.raw.iter().all(allowed)
    }

    /// Whether a write carries this entry as it is: its object is what its
    /// mode names, a file or link over a blob or a directory over a tree.
    fn carried(&self, kinds: &BTreeMap<String, Found>) -> bool {
        match (kind_of_mode(&self.mode), kinds.get(&self.oid)) {
            (Some(named @ ("blob" | "tree")), Some(Found::Present(object))) => object.kind == named,
            _ => false,
        }
    }
}

/// The object type a tree entry's mode names, for the modes git writes.
fn kind_of_mode(mode: &str) -> Option<&'static str> {
    match mode {
        REGULAR_FILE | EXECUTABLE | SYMLINK => Some("blob"),
        TREE => Some("tree"),
        GITLINK => Some("commit"),
        _ => None,
    }
}

/// The board at `commit`, read in two steps: every folder is scanned
/// (listed, each entry's type and size judged) and the format marker read
/// first, so a newer format, a damaged root or a board past what a read
/// takes is refused before any file's bytes are; only then are the files
/// read and decoded.
///
/// # Errors
/// Objects this clone does not hold; git fails, a tree is past its bounds,
/// or the commit is not a board (a clash).
pub fn read_board(git: &Git, commit: &str, limits: Limits) -> Result<Board, Unread> {
    let root = listed(git, &tree_of(git, commit)?, limits.tree_bytes)?;
    let kinds = objects(git, looked_up(&root))?;
    let named = |name: &'static str| root.iter().filter(move |entry| entry.name == name);
    // A tree named once: a folder to read, or the archive to carry.
    let once = |name: &'static str| match named(name).collect::<Vec<_>>().as_slice() {
        [entry] if is_tree(entry, &kinds) => Some(*entry),
        _ => None,
    };
    // The format marker, at the same depth as the folders. A file of its
    // name is a stray, as on a board before the marker; none is a board
    // before it.
    let marker = match named(FORMAT_TREE).collect::<Vec<_>>().as_slice() {
        [] => None,
        [entry] if is_tree(entry, &kinds) => Some(read_format(git, &entry.oid, limits)),
        [entry] if is_file(entry, &kinds) => None,
        [entry] => Some(Ok(Marker::Damaged(format!(
            "format at the root is mode {} over a {}",
            entry.mode,
            match kinds.get(&entry.oid) {
                Some(Found::Present(object)) => object.kind.as_str(),
                Some(Found::Missing) | None => "missing object",
            }
        )))),
        [_, _, ..] => Some(Ok(Marker::Damaged(
            "format is named more than once at the root".into(),
        ))),
    };
    // A newer format first: whatever else this quecto would make of the
    // board, it asks for an upgrade, before any folder is scanned.
    match &marker {
        Some(Ok(Marker::Newer(version))) => return Err(Unread::Failed(newer(version))),
        Some(Ok(Marker::Oversized(size))) => {
            return Err(Unread::Failed(BoardStoreError::Unavailable(format!(
                "its format marker (format/board) is {size} bytes, more than this quecto reads: \
                 if a newer quecto wrote the board, upgrade quecto; if a hand edit did, mend the \
                 board: {RECOVERY}"
            ))));
        }
        Some(Ok(Marker::Current | Marker::Damaged(_)) | Err(_)) | None => {}
    }
    // Every folder that really is a tree is scanned in the same round as
    // the root, so a partial clone fetches what they all miss at once.
    let tasks = named("tasks")
        .next()
        .filter(|entry| is_tree(entry, &kinds))
        .map(|entry| scan_tasks(git, &entry.oid, limits));
    let folders: Vec<(&'static str, Result<FolderScan, Unread>)> = FOLDERS
        .iter()
        .filter_map(|folder| {
            let name = folder.naming.folder;
            once(name).map(|entry| (name, scan_folder(git, &entry.oid, folder, limits)))
        })
        .collect();
    let mut missing = missing(&root, &kinds);
    let (tasks, marker) = (gather(tasks, &mut missing), gather(marker, &mut missing));
    let folders: Vec<(&'static str, Option<Result<FolderScan, Unread>>)> = folders
        .into_iter()
        .map(|(name, scan)| (name, gather(Some(scan), &mut missing)))
        .collect();
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(Unread::Missing(missing));
    }
    let marker = marker.transpose()?;
    // A second `tasks`, folder or archive is never read or carried, so a
    // write would lose what it holds: damage, whatever its copies are.
    let twice = root.iter().map(|entry| entry.name.as_str()).find(|name| {
        root_tree(name) && root.iter().filter(|entry| entry.name == *name).count() > 1
    });
    if let Some(name) = twice {
        return Err(damage(&format!("its root names {name} more than once")));
    }
    // A board that is no board, or a damaged one, says so before what its
    // folders' scans found.
    board_shape(&root, &kinds)?;
    match &marker {
        Some(Marker::Damaged(why)) => {
            return Err(damage(&format!("its format marker is damaged ({why})")));
        }
        Some(Marker::Current | Marker::Newer(_) | Marker::Oversized(_)) | None => {}
    }
    let tasks = tasks.transpose()?;
    let folders = folders
        .into_iter()
        .filter_map(|(name, scan)| Some(scan?.map(|scan| (name, scan))))
        .collect::<Result<Vec<_>, Unread>>()?;
    // What a read takes in all, before any file's bytes are read.
    let entries = tasks.as_ref().map_or(0, TasksScan::entries)
        + folders
            .iter()
            .map(|(_, scan)| scan.entries())
            .sum::<usize>();
    let bytes = tasks.as_ref().map_or(0, TasksScan::bytes)
        + folders.iter().map(|(_, scan)| scan.bytes()).sum::<usize>();
    within_read(entries, bytes, limits)?;
    // No `tasks` tree yet: a board that holds no task.
    let board = match tasks {
        Some(scan) => load_tasks(git, scan)?,
        None => Board::default(),
    };
    let folders = folders
        .into_iter()
        .map(|(name, scan)| Ok((name, load_folder(git, scan)?)))
        .collect::<Result<Vec<_>, Unread>>()?;
    let archive = once(ARCHIVE).map(|entry| entry.oid.clone());
    let board = board.with_folders(folders, archive);
    // Every path a listing shows is one an answer and the index take back.
    for file in &board.quarantined {
        assert!(
            shown_board_path(&file.path),
            "a path set aside shows as one name in a folder read: {:?}",
            file.path
        );
    }
    Ok(board)
}

/// Damage to a board's root or marker: no quecto writes it, so a hand
/// edit did.
fn damage(why: &str) -> Unread {
    Unread::Failed(BoardStoreError::Unavailable(format!(
        "{why}; quecto never writes a marker or root like this, so a hand edit did: to mend the \
         board, {RECOVERY}"
    )))
}

/// A read past what it takes.
pub(super) fn too_large(why: String) -> Unread {
    Unread::Failed(BoardStoreError::TooLarge(why))
}

/// The hex digits of an object's id an odd name shows.
const SHORT_OID: usize = 12;

/// A tree entry's name as a path set aside in `folder` shows it: its
/// [`shown_name`], when that is a path a listing, an answer and the index
/// take back ([`shown_board_path`], the one rule they share). Any other
/// (a name that shows as nothing, `.`, `..`, or holds a `/`, as only a
/// hand-made tree does) shows as `…` and the first digits of its object's
/// id, `oid`: two never collide, nor does one with a file named `…`.
pub(super) fn entry_path(folder: &str, raw: &[u8], oid: &str) -> String {
    let candidate = format!("{folder}/{}", shown_name(raw));
    if shown_board_path(&candidate) {
        return candidate;
    }
    let short: String = oid.chars().take(SHORT_OID).collect();
    let odd = format!("{folder}/…{short}");
    assert!(shown_board_path(&odd), "an odd name shows as one: {odd:?}");
    odd
}

/// `result`, its missing objects moved to `missing`.
fn gather<T>(
    result: Option<Result<T, Unread>>,
    missing: &mut Vec<String>,
) -> Option<Result<T, Unread>> {
    match result {
        Some(Err(Unread::Missing(more))) => {
            missing.extend(more);
            None
        }
        other => other,
    }
}

/// Whether `entry` is a file over a blob, as its mode names.
fn is_file(entry: &Entry, kinds: &BTreeMap<String, Found>) -> bool {
    kind_of_mode(&entry.mode) == Some("blob")
        && matches!(kinds.get(&entry.oid), Some(Found::Present(object)) if object.kind == "blob")
}

fn is_tree(entry: &Entry, kinds: &BTreeMap<String, Found>) -> bool {
    entry.mode == TREE
        && matches!(kinds.get(&entry.oid), Some(Found::Present(object)) if object.kind == "tree")
}

/// The entries whose objects are looked up: files and trees, whose
/// objects a board holds. A gitlink's commit is another repository's.
/// (git reads every mode as one of the five it writes, so no other mode
/// reaches here.)
fn looked_up(entries: &[Entry]) -> impl Iterator<Item = &str> {
    entries
        .iter()
        .filter(|entry| matches!(kind_of_mode(&entry.mode), Some("blob" | "tree")))
        .map(|entry| entry.oid.as_str())
}

fn missing(entries: &[Entry], kinds: &BTreeMap<String, Found>) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| matches!(kinds.get(&entry.oid), Some(Found::Missing)))
        .map(|entry| entry.oid.clone())
        .collect()
}

/// A board's root: a README file (a hand edit may have deleted it), a
/// `tasks` tree once it holds a task, the other kinds' folders and the
/// archive, each a tree, and stray files a write drops; at least one of the
/// README and `tasks`; and its format marker, a `format` tree, once a write
/// of this quecto added it.
fn board_shape(root: &[Entry], kinds: &BTreeMap<String, Found>) -> Result<(), BoardStoreError> {
    let kind = |entry: &Entry| match kinds.get(&entry.oid) {
        Some(Found::Present(object)) => Some(object.kind.as_str()),
        Some(Found::Missing) | None => None,
    };
    let file =
        |entry: &Entry| kind_of_mode(&entry.mode) == Some("blob") && kind(entry) == Some("blob");
    // The format marker is judged on its own (`read_board`).
    let fits = |entry: &Entry| match entry.name.as_str() {
        FORMAT_TREE => true,
        name if root_tree(name) => is_tree(entry, kinds),
        _ => file(entry),
    };
    let ours = |entry: &Entry| matches!(entry.name.as_str(), "README.md" | "tasks");
    let marked = root.iter().any(ours);
    match (marked, root.iter().find(|entry| !fits(entry))) {
        (true, None) => Ok(()),
        (false, None) => Err(BoardStoreError::Clash(
            "its root holds neither a README.md nor a tasks tree, so it is not a board".into(),
        )),
        (_, Some(entry)) => Err(BoardStoreError::Clash(format!(
            "`{}` at its root is a {} of mode {}, so it is not a board",
            quoted(&entry.raw),
            kind(entry).unwrap_or("gitlink"),
            entry.mode
        ))),
    }
}

/// The root tree of `commit`, read from the commit alone, so a clone
/// without the tree still names it.
fn tree_of(git: &Git, commit: &str) -> Result<String, Unread> {
    let args = ["log", "-1", "--no-show-signature", "--format=%T", commit];
    let tree = git.text(&args, None).map_err(unavailable)?;
    Ok(object_id(&tree)?)
}

/// The entries of `tree`, whose real type and size are checked before it
/// is listed.
fn listed(git: &Git, tree: &str, max_bytes: usize) -> Result<Vec<Entry>, Unread> {
    let not_read = |why: String| Unread::Failed(BoardStoreError::Unavailable(why));
    match objects(git, std::iter::once(tree))?.remove(tree) {
        Some(Found::Present(object)) if object.kind == "tree" && object.size <= max_bytes => {}
        Some(Found::Present(object)) if object.kind == "tree" => {
            return Err(too_large(format!(
                "the board tree {tree} is {} bytes; the most read is {max_bytes}",
                object.size
            )));
        }
        Some(Found::Present(object)) => {
            return Err(not_read(format!("{tree} is a {}, not a tree", object.kind)));
        }
        Some(Found::Missing) => return Err(Unread::Missing(vec![tree.to_string()])),
        None => return Err(not_read(format!("cat-file did not report {tree}"))),
    }
    let listing = git
        .ok_up_to(&["ls-tree", "-z", tree], None, MAX_LISTING_BYTES)
        .map_err(unavailable)?;
    Ok(listing
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
        .map(entry)
        .collect::<Result<Vec<Entry>, BoardStoreError>>()?)
}

fn entry(row: &[u8]) -> Result<Entry, BoardStoreError> {
    let malformed =
        || BoardStoreError::Unavailable(format!("unexpected ls-tree row {:?}", quoted(row)));
    let tab = row
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or_else(malformed)?;
    let (meta, raw) = (String::from_utf8_lossy(&row[..tab]), &row[tab + 1..]);
    match meta.split(' ').collect::<Vec<_>>().as_slice() {
        [mode, _kind, oid] => Ok(Entry {
            mode: (*mode).into(),
            oid: object_id(oid)?,
            name: String::from_utf8_lossy(raw).into_owned(),
            raw: raw.to_vec(),
        }),
        _ => Err(malformed()),
    }
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

#[path = "board_bounds.rs"]
mod bounds;
pub use bounds::within;
use bounds::within_read;
#[path = "board_entries.rs"]
mod entries;
pub use entries::TASK_NAMING;
use entries::{TaskFile, task_name};
#[path = "board_folders.rs"]
mod folders;
pub use folders::{ARCHIVE, CarriedTree, MAX_BOARD_ENTRIES, read_size_in};
use folders::{FOLDERS, FolderScan, load_folder, root_tree, scan_folder};
#[path = "board_tasks.rs"]
mod tasks;
#[path = "board_writes.rs"]
mod writes;
use tasks::{TasksScan, load_tasks, scan_tasks};
pub use writes::{FolderAfter, FolderFile, folder_file};
#[path = "board_format.rs"]
mod format;
pub use format::{FORMAT, FORMAT_FILE, FORMAT_TREE};
use format::{Marker, newer, read_format};

#[cfg(test)]
#[path = "board_tree_tests.rs"]
mod tests;
