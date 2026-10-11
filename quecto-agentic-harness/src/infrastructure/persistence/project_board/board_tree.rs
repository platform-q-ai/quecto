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
//! In `tasks/`, a regular file `<id>.json` named once is a task and is
//! read. Any other `.json` file is set aside (under its id, when its name
//! spells one) and carried forward by every write as it is, whatever its
//! mode or name, so no write loses it, a later quecto's included, unless
//! its mode and its object disagree (a file mode over a tree): git's own
//! transport refuses such a tree, so carrying it would leave a board no
//! one can fetch. A `.json` named twice refuses every write (see
//! [`Board::repeated`]). Summaries are derived, so a `<id>.md` that is
//! not a regular file named once is a stray, never sets its task aside,
//! and the next write regenerates it. Any other name, or an entry of a
//! mode git never writes, is a stray, set aside with no id, and dropped by
//! the next write.
//!
//! An object a partial clone does not hold yet is not fetched here (every
//! local call runs with `GIT_NO_LAZY_FETCH`): the read answers
//! [`Unread::Missing`] with every object it found missing, so the store
//! fetches them in one call and reads again.
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
/// The most `ls-tree` prints for a tree of [`MAX_TREE_BYTES`]: each entry
/// of at least 28 bytes grows by at most 53 when its object name is
/// spelled in hex beside its type.
const MAX_LISTING_BYTES: usize = 4 * MAX_TREE_BYTES;
/// The most a `cat-file` header or `--batch-check` row takes: a SHA-256
/// name, a type and a size.
const OBJECT_ROW_BYTES: usize = 128;
/// The most task-file bytes one read takes in all.
pub const MAX_BOARD_BYTES: usize = 64 * 1024 * 1024;
/// The largest summary read back to compare with the one it should be.
pub const MAX_SUMMARY_BYTES: usize = 2 * MAX_TASK_FILE_BYTES;
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

/// A task as the board holds it: the task and its files' objects.
#[derive(Debug, Clone)]
pub struct HeldTask {
    pub task: Task,
    pub json: String,
    /// The summary's object and size, when it is a regular file named
    /// once.
    pub summary: Option<(String, usize)>,
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
    /// `.json` names listed more than once, each with every object it
    /// names. git rejects such a tree (`fsck`: duplicateEntries, run by
    /// GitHub and any remote with `receive.fsckObjects` on every push), so
    /// carrying all of them would make every later push fail, and keeping
    /// one would drop the rest unseen: every write is refused until the
    /// board is moved back past the edit that made it.
    pub repeated: Vec<(String, Vec<String>)>,
    /// The size of every task file a read of this board reads, by name:
    /// what a write checks against the read bound.
    pub read_bytes: BTreeMap<String, usize>,
}

/// How the README and a refusal say to undo a hand edit.
pub const RECOVERY: &str = "move `quecto/board` back to the commit before that edit";

impl Board {
    /// Why no write is taken, when a `.json` name is listed twice.
    pub fn repeated_refusal(&self, head: &str) -> Option<String> {
        let (name, oids) = self.repeated.first()?;
        Some(format!(
            "tasks/{name} is named more than once in board commit {head} (objects {}). quecto \
             never writes such a tree, so a hand edit did, and git rejects it (fsck: \
             duplicateEntries): no write can carry it forward, and quecto will not choose a \
             copy. To mend the board, {RECOVERY}",
            oids.join(", ")
        ))
    }

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
#[derive(Debug, Clone, PartialEq, Eq)]
struct Object {
    kind: String,
    size: usize,
}

/// What `cat-file --batch-check` found for one object.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    Present(Object),
    Missing,
}

/// The two files a task has in `tasks/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskFile {
    Record,
    Summary,
}

/// A `.json` name, with the id it spells if it spells one, or a `<id>.md`
/// name: which file it is.
fn task_name(name: &str) -> Option<(Option<Slug>, TaskFile)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let id = Slug::parse(stem).ok();
    match (extension, id) {
        ("json", id) => Some((id, TaskFile::Record)),
        ("md", Some(id)) => Some((Some(id), TaskFile::Summary)),
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
pub fn read_board(git: &Git, commit: &str, max_bytes: usize) -> Result<Board, Unread> {
    let root = listed(git, &tree_of(git, commit)?)?;
    let kinds = objects(git, looked_up(&root))?;
    let tasks = root.iter().find(|entry| entry.name == "tasks");
    // The `tasks` tree, when it really is one, is read in the same round
    // as the root, so a partial clone fetches what both miss at once.
    let read = tasks
        .filter(|entry| is_tree(entry, &kinds))
        .map(|entry| read_tasks(git, &entry.oid, max_bytes));
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
            entry.name,
            kind(entry).unwrap_or("gitlink"),
            entry.mode
        ))),
    }
}

/// What becomes of one `tasks/` entry.
enum Disposition {
    Read(Slug),
    Summary(Slug),
    /// Set aside, and carried forward by every write when `kept`.
    Aside {
        id: Option<Slug>,
        reason: String,
        kept: bool,
    },
    /// A stray summary: dropped, and regenerated by the next write.
    Stray,
}

fn disposition(entry: &Entry, found: Option<&Object>, count: usize) -> Disposition {
    let aside =
        |id: Option<Slug>, reason: String, kept: bool| Disposition::Aside { id, reason, kept };
    let Some((id, file)) = task_name(&entry.name) else {
        let reason = "not a task file: only <id>.json and <id>.md belong in tasks/".into();
        return aside(None, reason, false);
    };
    let kind = found.map(|object| object.kind.as_str());
    let size = found.map_or(usize::MAX, |object| object.size);
    // A gitlink's commit is not looked up; any other kept entry's object
    // is the kind its mode names.
    let agrees = match kind_of_mode(&entry.mode) {
        Some("commit") => true,
        Some(named) => kind == Some(named),
        None => false,
    };
    match (file, id, count, entry.mode.as_str(), kind) {
        (TaskFile::Summary, Some(id), 1, REGULAR_FILE, Some("blob")) => Disposition::Summary(id),
        (TaskFile::Summary, _, _, _, _) => Disposition::Stray,
        (TaskFile::Record, id, 2.., _, _) => {
            aside(id, "named more than once in the tree".into(), false)
        }
        (TaskFile::Record, Some(id), _, REGULAR_FILE, Some("blob"))
            if size <= MAX_TASK_FILE_BYTES =>
        {
            Disposition::Read(id)
        }
        (TaskFile::Record, id, _, REGULAR_FILE, Some("blob")) if id.is_some() => aside(
            id,
            format!("task file is {size} bytes; the most is {MAX_TASK_FILE_BYTES}"),
            true,
        ),
        (TaskFile::Record, None, _, _, _) if agrees => aside(
            None,
            "its name is not a task id this quecto reads; carried forward as it is".into(),
            true,
        ),
        (TaskFile::Record, id, _, mode, _) if agrees => aside(
            id,
            format!("not a regular file (mode {mode}); carried forward as it is"),
            true,
        ),
        (TaskFile::Record, id, _, mode, _) if kind_of_mode(mode).is_none() => aside(
            id,
            format!("mode {mode} is not one git writes, so the next write drops it"),
            false,
        ),
        (TaskFile::Record, id, _, mode, _) => aside(
            id,
            format!(
                "mode {mode} over a {}: git cannot send such a tree, so the next write drops it",
                kind.unwrap_or("missing object")
            ),
            false,
        ),
    }
}

fn read_tasks(git: &Git, tree: &str, max_bytes: usize) -> Result<Board, Unread> {
    let entries = listed(git, tree)?;
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
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &entries {
        *counts.entry(entry.name.as_str()).or_default() += 1;
    }
    let mut board = Board::default();
    let set_aside = |board: &mut Board, entry: &Entry, id: Option<Slug>, reason: String| {
        board.quarantined.push(QuarantinedFile {
            path: format!("tasks/{}", entry.name),
            id,
            reason,
        });
    };
    let keep = |board: &mut Board, entry: &Entry| {
        let kind = kind_of_mode(&entry.mode);
        debug_assert!(kind.is_some(), "only entries of git's own modes are kept");
        let kind = kind.unwrap_or("blob");
        board.kept.insert(
            entry.name.clone(),
            format!("{} {kind} {}", entry.mode, entry.oid),
        );
    };
    let (mut to_read, mut summaries) = (vec![], BTreeMap::new());
    for entry in &entries {
        let found = match kinds.get(&entry.oid) {
            Some(Found::Present(object)) => Some(object),
            Some(Found::Missing) | None => None,
        };
        match disposition(entry, found, counts[entry.name.as_str()]) {
            Disposition::Read(id) => {
                let size = found.map_or(0, |object| object.size);
                board.read_bytes.insert(entry.name.clone(), size);
                to_read.push((id, entry, size));
            }
            Disposition::Summary(id) => {
                let size = found.map_or(usize::MAX, |object| object.size);
                summaries.insert(id, (entry.oid.clone(), size));
            }
            Disposition::Aside { id, reason, kept } => {
                if kept {
                    keep(&mut board, entry);
                }
                set_aside(&mut board, entry, id, reason);
            }
            Disposition::Stray => {}
        }
    }
    board.repeated = counts
        .iter()
        .filter(|(name, count)| {
            **count > 1 && task_name(name).is_some_and(|(_, file)| file == TaskFile::Record)
        })
        .map(|(name, _)| {
            let oids = entries.iter().filter(|entry| entry.name == *name);
            (
                (*name).to_string(),
                oids.map(|entry| entry.oid.clone()).collect(),
            )
        })
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
                keep(&mut board, entry);
                set_aside(&mut board, entry, Some(id), reason);
            }
        }
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

/// The root tree of `commit`, read from the commit alone, so a clone
/// without the tree still names it.
fn tree_of(git: &Git, commit: &str) -> Result<String, Unread> {
    let args = ["log", "-1", "--no-show-signature", "--format=%T", commit];
    let tree = git.text(&args, None).map_err(unavailable)?;
    Ok(object_id(&tree)?)
}

/// The entries of `tree`, whose real type and size are checked before it
/// is listed.
fn listed(git: &Git, tree: &str) -> Result<Vec<Entry>, Unread> {
    let not_read = |why: String| Unread::Failed(BoardStoreError::Unavailable(why));
    match objects(git, std::iter::once(tree))?.remove(tree) {
        Some(Found::Present(object)) if object.kind == "tree" && object.size <= MAX_TREE_BYTES => {}
        Some(Found::Present(object)) if object.kind == "tree" => {
            return Err(not_read(format!(
                "the board tree {tree} is {} bytes; the most read is {MAX_TREE_BYTES}",
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

/// The real type and size of each object, or that it is missing, from one
/// `cat-file --batch-check`; every object asked for is reported, or it is
/// an error.
fn objects<'a>(
    git: &Git,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<String, Found>, BoardStoreError> {
    let wanted: BTreeSet<&str> = oids.collect();
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = wanted.iter().map(|oid| format!("{oid}\n")).collect();
    let format = "--batch-check=%(objectname) %(objecttype) %(objectsize)";
    let listing = git
        .ok_up_to(
            &["cat-file", format],
            Some(input.as_bytes()),
            wanted.len() * OBJECT_ROW_BYTES,
        )
        .map_err(unavailable)?;
    let malformed =
        |row: &str| BoardStoreError::Unavailable(format!("unexpected cat-file row {row:?}"));
    let mut found = BTreeMap::new();
    for row in String::from_utf8_lossy(&listing).lines() {
        let (oid, object) = match row.split(' ').collect::<Vec<_>>().as_slice() {
            [oid, "missing"] => (object_id(oid)?, Found::Missing),
            [oid, kind, size] => (
                object_id(oid)?,
                Found::Present(Object {
                    kind: (*kind).into(),
                    size: size.parse().map_err(|_| malformed(row))?,
                }),
            ),
            _ => return Err(malformed(row)),
        };
        found.insert(oid, object);
    }
    match found.keys().map(String::as_str).eq(wanted.iter().copied()) {
        true => Ok(found),
        false => Err(BoardStoreError::Unavailable(format!(
            "cat-file reported {} objects for the {} asked for",
            found.len(),
            wanted.len()
        ))),
    }
}

/// The bytes of each blob, from one `cat-file --batch`; `wanted` names each
/// blob with the size its caller checked first.
pub fn contents(
    git: &Git,
    wanted: &[(&str, usize)],
) -> Result<BTreeMap<String, Vec<u8>>, BoardStoreError> {
    if wanted.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = wanted.iter().map(|(oid, _)| format!("{oid}\n")).collect();
    let limit = wanted
        .iter()
        .map(|(_, size)| size.saturating_add(OBJECT_ROW_BYTES))
        .fold(0usize, usize::saturating_add);
    let args = ["cat-file", "--batch"];
    let output = git
        .local(&args, Some(input.as_bytes()), limit)
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
    for (oid, size) in wanted {
        match found.get(*oid) {
            Some(body) if body.len() == *size => {}
            _ => {
                return Err(BoardStoreError::Unavailable(format!(
                    "cat-file did not return {oid} at its {size} bytes"
                )));
            }
        }
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
