//! What becomes of one entry of a board's folder (`board_tree`): read as
//! its kind's file, or set aside, carried forward as it is or dropped by
//! the next write. `tasks/` and the other kinds' folders (#2481 S9) are
//! judged by the same rules, each by its own [`Naming`].
use super::{
    Entry, GITLINK, MAX_NAME_BYTES, MAX_TASK_FILE_BYTES, NAME_PUNCTUATION, Object, REGULAR_FILE,
    kind_of_mode,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// The task files `tasks/` holds: the task codec's markdown, and the JSON
/// form older quecto wrote, which this one never reads (#2481 S8). The
/// other folders hold markdown alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskFile {
    Markdown,
    LegacyJson,
}

/// A task file's name, with the id it spells if it spells one: which file
/// it is.
pub(super) fn task_name(name: &str) -> Option<(Option<Slug>, TaskFile)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let id = Slug::parse(stem).ok();
    match extension {
        "md" => Some((id, TaskFile::Markdown)),
        "json" => Some((id, TaskFile::LegacyJson)),
        _ => None,
    }
}

/// How a folder names its files, and how a reason names them.
#[derive(Debug, Clone, Copy)]
pub(super) struct Naming {
    /// The folder, `tasks`.
    pub folder: &'static str,
    /// What one of its files holds: `task`.
    pub noun: &'static str,
    /// What its files' names spell: `task id`.
    pub spelled: &'static str,
    /// The name of a file of it: `<id>.md`.
    pub pattern: &'static str,
    /// The longest name it keeps.
    pub max_name: usize,
    /// The largest file of it read.
    pub file_bytes: usize,
}

/// How `tasks/` names its files.
pub(super) const TASK_NAMING: Naming = Naming {
    folder: "tasks",
    noun: "task",
    spelled: "task id",
    pattern: "<id>.md",
    max_name: MAX_NAME_BYTES,
    file_bytes: MAX_TASK_FILE_BYTES,
};

/// What becomes of one entry of a folder, its file named by `K` (a task's
/// id, or another kind's key).
pub(super) enum Disposition<K> {
    Read(K),
    /// Set aside, and carried forward by every write when `kept`.
    Aside {
        id: Option<K>,
        reason: String,
        kept: bool,
    },
}

/// What becomes of `entry` of a folder named by `naming`, whose object is
/// `found`, named `count` times among the copies a write carries; `file` is
/// which file of the folder its name spells, and the key it spells.
pub(super) fn disposition<K>(
    entry: &Entry,
    found: Option<&Object>,
    count: usize,
    file: Option<(Option<K>, TaskFile)>,
    naming: &Naming,
) -> Disposition<K> {
    let aside = |id: Option<K>, reason: String, kept: bool| Disposition::Aside { id, reason, kept };
    let Naming {
        folder,
        noun,
        spelled,
        pattern,
        max_name,
        file_bytes,
    } = *naming;
    match entry.kept_name(max_name) {
        true => {}
        false => {
            let reason = format!(
                "not a name quecto keeps (letters, numbers and {NAME_PUNCTUATION}, starting \
                 with a letter or number, at most {max_name} bytes); the next write drops it"
            );
            return aside(None, reason, false);
        }
    }
    let Some((id, file)) = file else {
        let reason = format!("not a {noun} file: only {pattern} belongs in {folder}/");
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
            format!("a gitlink is not a {noun} file; the next write drops it"),
            false,
        ),
        (_, id, 2.., _, _) => aside(id, "named more than once in the tree".into(), false),
        (TaskFile::Markdown, Some(id), _, REGULAR_FILE, Some("blob")) if size <= file_bytes => {
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
            format!("{noun} file is {size} bytes; the most is {file_bytes}"),
            true,
        ),
        (_, None, _, _, _) if agrees => aside(
            None,
            format!("its name is not a {spelled} this quecto reads; carried forward as it is"),
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
