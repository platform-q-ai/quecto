//! What becomes of one entry of a board's `tasks/` tree (`board_tree`):
//! read as a task, or set aside, carried forward as it is or dropped by
//! the next write.
use super::{
    Entry, GITLINK, MAX_NAME_BYTES, MAX_TASK_FILE_BYTES, NAME_PUNCTUATION, Object, REGULAR_FILE,
    kind_of_mode,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// The task files `tasks/` holds: the task codec's markdown, and the JSON
/// form older quecto wrote, which this one never reads (#2481 S8).
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

/// What becomes of one `tasks/` entry.
pub(super) enum Disposition {
    Read(Slug),
    /// Set aside, and carried forward by every write when `kept`.
    Aside {
        id: Option<Slug>,
        reason: String,
        kept: bool,
    },
}

pub(super) fn disposition(entry: &Entry, found: Option<&Object>, count: usize) -> Disposition {
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
