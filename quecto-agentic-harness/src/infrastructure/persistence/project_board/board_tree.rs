//! Reading one board commit's trees, which anyone with push access may
//! have written, so every part is checked before it is trusted:
//! - each tree's real type and size, before it is listed;
//! - each entry's real object type and size, in one `cat-file
//!   --batch-check`, before its bytes are read;
//! - the task files' bytes, in one `cat-file --batch`.
//!
//! A board's root holds its README (each write regenerates it), a `tasks`
//! tree, and maybe stray files (each write drops them). A root with
//! anything else, a directory above all, or with neither a README nor a
//! `tasks` tree, is someone's code, not a board: reading, writing and
//! bootstrapping all take the same view of it ([`Unread::Failed`] with a
//! clash).
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
//! An object a partial clone does not hold yet is not fetched here (every
//! local call runs with `GIT_NO_LAZY_FETCH`): the read answers
//! [`Unread::Missing`] with every object it found missing, so the store
//! fetches them in one call and reads again.
use super::artifacts::decode;
use super::artifacts::task::{TASK_LIMITS, TaskCodec};
pub use super::cat_file::contents;
use super::cat_file::{Found, Object, objects};
use super::git::{Git, GitError, capped, quoted, sanitised, shown_name};
use crate::application::project_board::ports::{ArtifactKey, BoardStoreError, QuarantinedFile};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use std::collections::{BTreeMap, BTreeSet};

/// The punctuation a name kept in `tasks/` may hold besides letters and
/// numbers.
const NAME_PUNCTUATION: &str = "-_.+=@,";
/// The longest name kept in `tasks/`: an id and its five-byte `.json`,
/// the longer of the two extensions a task file has had.
pub const MAX_NAME_BYTES: usize = MAX_SLUG_BYTES + 5;
/// The most entries the board's `tasks/` tree may hold: its tasks, one
/// file each, and the files set aside.
pub const MAX_TASK_ENTRIES: usize = 2_048;
/// The largest task file read or written: the task codec's cap.
pub const MAX_TASK_FILE_BYTES: usize = TASK_LIMITS.file_bytes;
/// The largest tree read: [`MAX_TASK_ENTRIES`] entries with long names.
pub const MAX_TREE_BYTES: usize = MAX_TASK_ENTRIES * 160;
/// The most `ls-tree` prints for a tree of [`MAX_TREE_BYTES`]: each entry
/// of at least 28 bytes grows by at most 53 when its object name is
/// spelled in hex beside its type.
const MAX_LISTING_BYTES: usize = 4 * MAX_TREE_BYTES;
/// The most task-file bytes one read takes in all.
pub const MAX_BOARD_BYTES: usize = 64 * 1024 * 1024;
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
        | BoardStoreError::Unavailable(why) => why,
    }
}

/// What one read of a board takes at most; a write keeps within it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Task-file bytes read in all.
    pub board_bytes: usize,
    /// The bytes of one tree.
    pub tree_bytes: usize,
}

pub const LIMITS: Limits = Limits {
    board_bytes: MAX_BOARD_BYTES,
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

    /// Its size, when a read of the board reads it (a regular file of at
    /// most a task file's size).
    pub fn read_size(&self) -> Option<usize> {
        let read = self.mode == REGULAR_FILE && self.kind == "blob";
        (read && self.size <= MAX_TASK_FILE_BYTES).then_some(self.size)
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
        let oids: Vec<&str> = copies.iter().map(|copy| copy.oid.as_str()).collect();
        Some(format!(
            "tasks/{} is named more than once in board commit {head} (objects {}). quecto \
             never writes such a tree, so a hand edit did, and git rejects it (fsck: \
             duplicateEntries): no write can carry it forward, and quecto will not choose a \
             copy. To mend the board, clear it (clear set-aside), naming the copy to keep for \
             every name listed more than once, or {RECOVERY}",
            quoted(name.as_bytes()),
            oids.join(", ")
        ))
    }

    /// The board, with `written` in place, keeps within what a read takes:
    /// the entries of its `tasks/` tree (each task one file, each file set
    /// aside one) and the task-file bytes a read reads. Every change checks
    /// it before it builds anything.
    ///
    /// # Errors
    /// An invalid write past either bound.
    pub fn within_bounds(
        &self,
        written: &[(&Task, Vec<u8>)],
        limits: Limits,
    ) -> Result<(), BoardStoreError> {
        let invalid = |why: String| Err(BoardStoreError::InvalidWrite(why));
        let ids: BTreeSet<&Slug> = written.iter().map(|(task, _)| &task.fields().id).collect();
        let after: BTreeSet<&Slug> = self.tasks.keys().chain(ids.iter().copied()).collect();
        let copies: usize = self.repeated.iter().map(|(_, copies)| copies.len()).sum();
        // A file a clear kept that a read will read as a task is in `kept`
        // until then; its id is not among the tasks, so it is counted once.
        let entries = after.len() + self.kept.len() + copies;
        if entries > MAX_TASK_ENTRIES {
            return invalid(format!(
                "the board would hold {} tasks and {} files set aside; it holds at most \
                 {MAX_TASK_ENTRIES} in all",
                after.len(),
                self.kept.len() + copies
            ));
        }
        let replaced: BTreeSet<String> = ids.iter().map(|id| task_file(id)).collect();
        let carried = self
            .read_bytes
            .iter()
            .filter(|(name, _)| !replaced.contains(*name));
        let bytes = carried.map(|(_, size)| size).sum::<usize>()
            + written.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        match bytes <= limits.board_bytes {
            true => Ok(()),
            false => invalid(format!(
                "the board would hold {bytes} bytes of task files; a read takes at most {}",
                limits.board_bytes
            )),
        }
    }

    pub fn is_quarantined(&self, id: &Slug) -> Option<&QuarantinedFile> {
        self.quarantined.iter().find(|file| file.id() == Some(id))
    }
}

/// One entry of a listed tree.
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
    /// [`MAX_NAME_BYTES`]. Every id fits; no path separator, no bracket, no
    /// invisible or combining character, so `mktree` takes every name a
    /// write carries and every one shows as itself.
    fn kept_name(&self) -> bool {
        let allowed = |b: &u8| b.is_ascii_alphanumeric() || NAME_PUNCTUATION.as_bytes().contains(b);
        self.raw.len() <= MAX_NAME_BYTES
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

/// The task files `tasks/` holds: the task codec's markdown, and the JSON
/// form older quecto wrote, which this one never reads (#2481 S8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskFile {
    Markdown,
    LegacyJson,
}

/// A task file's name, with the id it spells if it spells one: which file
/// it is.
fn task_name(name: &str) -> Option<(Option<Slug>, TaskFile)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let id = Slug::parse(stem).ok();
    match extension {
        "md" => Some((id, TaskFile::Markdown)),
        "json" => Some((id, TaskFile::LegacyJson)),
        _ => None,
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

/// The board at `commit`.
///
/// # Errors
/// Objects this clone does not hold; git fails, a tree is past its bounds,
/// or the commit is not a board (a clash).
pub fn read_board(git: &Git, commit: &str, limits: Limits) -> Result<Board, Unread> {
    let root = listed(git, &tree_of(git, commit)?, limits.tree_bytes)?;
    let kinds = objects(git, looked_up(&root))?;
    let tasks = root.iter().find(|entry| entry.name == "tasks");
    // The `tasks` tree, when it really is one, is read in the same round
    // as the root, so a partial clone fetches what both miss at once.
    let read = tasks
        .filter(|entry| is_tree(entry, &kinds))
        .map(|entry| read_tasks(git, &entry.oid, limits));
    let mut missing = missing(&root, &kinds);
    let board = match read {
        Some(Ok(board)) => Some(board),
        Some(Err(Unread::Missing(more))) => {
            missing.extend(more);
            None
        }
        Some(Err(failed)) => return Err(failed),
        None => None,
    };
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(Unread::Missing(missing));
    }
    board_shape(&root, &kinds)?;
    // No `tasks` tree yet: a board that holds no task.
    Ok(board.unwrap_or_default())
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
/// `tasks` tree once it holds a task, and stray files a write drops; at
/// least one of the README and `tasks`.
fn board_shape(root: &[Entry], kinds: &BTreeMap<String, Found>) -> Result<(), BoardStoreError> {
    let kind = |entry: &Entry| match kinds.get(&entry.oid) {
        Some(Found::Present(object)) => Some(object.kind.as_str()),
        Some(Found::Missing) | None => None,
    };
    let file =
        |entry: &Entry| kind_of_mode(&entry.mode) == Some("blob") && kind(entry) == Some("blob");
    let fits = |entry: &Entry| match entry.name.as_str() {
        "tasks" => is_tree(entry, kinds),
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

/// What becomes of one `tasks/` entry.
enum Disposition {
    Read(Slug),
    /// Set aside, and carried forward by every write when `kept`.
    Aside {
        id: Option<Slug>,
        reason: String,
        kept: bool,
    },
}

fn disposition(entry: &Entry, found: Option<&Object>, count: usize) -> Disposition {
    let aside =
        |id: Option<Slug>, reason: String, kept: bool| Disposition::Aside { id, reason, kept };
    match entry.kept_name() {
        true => {}
        false => {
            let reason = format!(
                "not a name quecto keeps (letters, numbers and {NAME_PUNCTUATION}, starting \
                 with a letter or number, at most {MAX_NAME_BYTES} bytes); the next write drops it"
            );
            return aside(None, reason, false);
        }
    }
    let Some((id, file)) = task_name(&entry.name) else {
        let reason = "not a task file: only <id>.md belongs in tasks/".into();
        return aside(None, reason, false);
    };
    let kind = found.map(|object| object.kind.as_str());
    let size = found.map_or(usize::MAX, |object| object.size);
    // Carried only when its object is what its mode names: a file or link
    // over a blob, a directory over a tree.
    let agrees = match kind_of_mode(&entry.mode) {
        Some(named @ ("blob" | "tree")) => kind == Some(named),
        Some(_) | None => false,
    };
    match (file, id, count, entry.mode.as_str(), kind) {
        (_, _, _, GITLINK, _) => aside(
            None,
            "a gitlink is not a task file; the next write drops it".into(),
            false,
        ),
        (_, id, 2.., _, _) => aside(id, "named more than once in the tree".into(), false),
        (TaskFile::Markdown, Some(id), _, REGULAR_FILE, Some("blob"))
            if size <= MAX_TASK_FILE_BYTES =>
        {
            Disposition::Read(id)
        }
        (TaskFile::LegacyJson, Some(id), _, REGULAR_FILE, Some("blob")) => aside(
            Some(id),
            "a task file in the JSON form older quecto wrote; this quecto keeps tasks as \
             <id>.md, so it is carried forward as it is and its task stays blocked until it is \
             cleared"
                .into(),
            true,
        ),
        (TaskFile::Markdown, id, _, REGULAR_FILE, Some("blob")) if id.is_some() => aside(
            id,
            format!("task file is {size} bytes; the most is {MAX_TASK_FILE_BYTES}"),
            true,
        ),
        (_, None, _, _, _) if agrees => aside(
            None,
            "its name is not a task id this quecto reads; carried forward as it is".into(),
            true,
        ),
        (_, id, _, mode, _) if agrees => aside(
            id,
            format!("not a regular file (mode {mode}); carried forward as it is"),
            true,
        ),
        // A copy a write drops names its id only when no copy a write
        // carries does: the carried copy is the task, read or set aside.
        (_, id, 0, mode, _) if kind_of_mode(mode).is_none() => aside(
            id,
            format!("mode {mode} is not one git writes, so the next write drops it"),
            false,
        ),
        (_, _, 1.., mode, _) => aside(
            None,
            format!("a copy of mode {mode} beside one a write carries; the next write drops it"),
            false,
        ),
        (_, id, 0, mode, _) => aside(
            id,
            format!(
                "mode {mode} over a {}: git cannot send such a tree, so the next write drops it",
                kind.unwrap_or("missing object")
            ),
            false,
        ),
    }
}

fn read_tasks(git: &Git, tree: &str, limits: Limits) -> Result<Board, Unread> {
    let max_bytes = limits.board_bytes;
    let entries = listed(git, tree, limits.tree_bytes)?;
    if entries.len() > MAX_TASK_ENTRIES {
        return Err(Unread::Failed(BoardStoreError::Unavailable(format!(
            "the board's tasks tree holds {} entries; the most is {MAX_TASK_ENTRIES}",
            entries.len()
        ))));
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
    let mut board = Board::default();
    let set_aside =
        |board: &mut Board, entry: &Entry, id: Option<Slug>, reason: String, carried: bool| {
            board.quarantined.push(QuarantinedFile {
                path: format!("tasks/{}", shown_name(&entry.raw)),
                key: id.map(ArtifactKey::Task),
                // A file's own text can reach the reason: keep what shows.
                reason: capped(&sanitised(reason.as_bytes())),
                object: entry.oid.clone(),
                carried,
            });
        };
    let keep = |board: &mut Board, entry: &Entry| {
        // A clear finds a kept name by the path the listing shows.
        let shown = shown_name(&entry.raw);
        assert_eq!(shown, entry.name, "a kept name shows as it is");
        let kind = kind_of_mode(&entry.mode);
        debug_assert!(kind.is_some(), "only entries of git's own modes are kept");
        let kind = kind.unwrap_or("blob");
        board.kept.insert(
            entry.name.clone(),
            format!("{} {kind} {}", entry.mode, entry.oid),
        );
    };
    let mut to_read = vec![];
    for entry in &entries {
        let found = match kinds.get(&entry.oid) {
            Some(Found::Present(object)) => Some(object),
            Some(Found::Missing) | None => None,
        };
        match disposition(entry, found, count(entry)) {
            Disposition::Read(id) => {
                let size = found.map_or(0, |object| object.size);
                board.read_bytes.insert(entry.name.clone(), size);
                to_read.push((id, entry, size));
            }
            Disposition::Aside { id, reason, kept } => {
                if kept {
                    keep(&mut board, entry);
                }
                set_aside(&mut board, entry, id, reason, kept);
            }
        }
    }
    // Only names a write would carry: a stray is dropped, however often
    // it is named.
    let mut repeated: BTreeMap<&str, Vec<TreeCopy>> = BTreeMap::new();
    for entry in &entries {
        let record = task_name(&entry.name).is_some();
        let found = match kinds.get(&entry.oid) {
            Some(Found::Present(object)) => Some(object),
            Some(Found::Missing) | None => None,
        };
        match (entry.kept_name() && record && count(entry) > 1, found) {
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
    let total: usize = to_read.iter().map(|(_, _, size)| size).sum();
    if total > max_bytes {
        return Err(Unread::Failed(BoardStoreError::Unavailable(format!(
            "the board's task files hold {total} bytes; the most read is {max_bytes}"
        ))));
    }
    let wanted: Vec<(&str, usize)> = to_read
        .iter()
        .map(|(_, entry, size)| (entry.oid.as_str(), *size))
        .collect();
    let contents = contents(git, &wanted)?;
    for (id, entry, _) in to_read {
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
                keep(&mut board, entry);
                set_aside(&mut board, entry, Some(id), reason, true);
            }
        }
    }
    // A task read is never one set aside; a tree that made one both is
    // refused, never read half-trusted.
    match board
        .tasks
        .keys()
        .find(|id| board.is_quarantined(id).is_some())
    {
        None => {}
        Some(id) => {
            return Err(Unread::Failed(BoardStoreError::Unavailable(format!(
                "task {:?} is both read and set aside in tree {tree}",
                id.as_str()
            ))));
        }
    }
    board
        .quarantined
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(board)
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
            return Err(not_read(format!(
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

/// A tree of `entries` fits what a read takes, or it is an invalid write,
/// so no write makes a board no one can read.
pub fn within(entries: &BTreeMap<String, String>, limits: Limits) -> Result<(), BoardStoreError> {
    match tree_bytes(entries) {
        bytes if bytes <= limits.tree_bytes => Ok(()),
        bytes => Err(BoardStoreError::InvalidWrite(format!(
            "the board would hold a tree of {bytes} bytes; a read takes at most {}",
            limits.tree_bytes
        ))),
    }
}

/// The bytes git takes to hold a tree of `entries` (name to `<mode> <type>
/// <oid>`): each is its mode without leading zeros, a space, its name, a
/// NUL and its raw object name.
fn tree_bytes(entries: &BTreeMap<String, String>) -> usize {
    entries
        .iter()
        .map(|(name, meta)| {
            let mut fields = meta.split(' ');
            let mode = fields.next().unwrap_or("").trim_start_matches('0');
            let oid = fields.nth(1).unwrap_or("");
            mode.len() + 1 + name.len() + 1 + oid.len() / 2
        })
        .sum()
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

#[cfg(test)]
#[path = "board_tree_tests.rs"]
mod tests;
