//! A board request as a front end sends it: one JSON object naming its
//! `op` and that op's own fields, each of its own type. Any other field is
//! refused. Text is taken as sent: the use cases turn CRLF line ends to LF,
//! once, for every front end.
use serde_json::{Map, Value};

use crate::application::project_board::dto::{
    BoardOp, DraftUpdate, NewBoardTask, NewBoardTaskStatus, plain_board_file,
};
use crate::application::project_board::operations::{BoardRequest, ListFilter};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

/// Why a request was refused before the board was read: the field (or
/// `op`) and what it must be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestError {
    pub field: String,
    pub problem: String,
}

impl RequestError {
    fn new(field: &str, problem: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            problem: problem.into(),
        }
    }
}

/// The type of a field's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A task id.
    Slug,
    /// A task id, or `null` for none.
    OptionalSlug,
    /// A list of task ids.
    Slugs,
    /// Text.
    Text,
    /// A time as the board spells it (RFC 3339, UTC, to the second).
    Timestamp,
    /// A task status.
    Status,
    /// `draft` or `ready`.
    NewStatus,
    /// `epic`, `task` or `chore`.
    TaskKind,
    /// A pull request number, from 1.
    Pr,
    /// A board file's path.
    Path,
    /// A git object name.
    ObjectId,
}

/// One field an op takes.
pub struct Field {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
}

const fn required(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        required: true,
    }
}

const fn optional(name: &'static str, kind: Kind) -> Field {
    Field {
        name,
        kind,
        required: false,
    }
}

const TASK: Field = required("task", Kind::Slug);

/// Every op and the fields it takes: the one table both front ends read.
pub const OPS: &[(&str, &[Field])] = &[
    ("bootstrap", &[]),
    (
        "list",
        &[
            optional("status", Kind::Status),
            optional("parent", Kind::Slug),
            optional("dependents_of", Kind::Slug),
        ],
    ),
    ("list_ready", &[]),
    ("list_takeable", &[]),
    ("show", &[TASK]),
    (
        "create",
        &[
            TASK,
            required("title", Kind::Text),
            optional("kind", Kind::TaskKind),
            optional("description", Kind::Text),
            optional("status", Kind::NewStatus),
            optional("parent", Kind::Slug),
            optional("depends_on", Kind::Slugs),
        ],
    ),
    (
        "update_draft",
        &[
            TASK,
            optional("title", Kind::Text),
            optional("description", Kind::Text),
            optional("parent", Kind::OptionalSlug),
            optional("depends_on", Kind::Slugs),
            optional("expected_updated", Kind::Timestamp),
        ],
    ),
    ("move", &[TASK, required("to", Kind::Status)]),
    ("renew", &[TASK]),
    ("take_over", &[TASK]),
    ("link_pr", &[TASK, required("pr", Kind::Pr)]),
    ("unlink_pr", &[TASK, required("pr", Kind::Pr)]),
    ("complete", &[TASK]),
    (
        "clear_set_aside",
        &[
            required("path", Kind::Path),
            optional("keep", Kind::ObjectId),
        ],
    ),
];

/// The op `value` names, when it is one of [`OPS`].
pub fn op_of(value: &Value) -> Option<String> {
    value
        .get("op")
        .and_then(Value::as_str)
        .filter(|op| fields_of(op).is_some())
        .map(str::to_owned)
}

/// The fields `op` takes, if it is an op.
pub fn fields_of(op: &str) -> Option<&'static [Field]> {
    OPS.iter()
        .find(|(name, _)| *name == op)
        .map(|(_, fields)| *fields)
}

/// The request `value` names.
///
/// # Errors
/// Not an object, no or an unknown `op`, a field the op does not take, a
/// required field missing, or a value of the wrong type.
pub fn parse(value: &Value) -> Result<BoardRequest, RequestError> {
    let object = value
        .as_object()
        .ok_or_else(|| RequestError::new("op", "a request is a JSON object"))?;
    let op = object
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| RequestError::new("op", "must name an op"))?;
    let fields = fields_of(op).ok_or_else(|| {
        let ops: Vec<&str> = OPS.iter().map(|(name, _)| *name).collect();
        RequestError::new("op", format!("must be one of {}", ops.join(", ")))
    })?;
    if let Some(stray) = object
        .keys()
        .find(|key| *key != "op" && !fields.iter().any(|field| field.name == *key))
    {
        return Err(RequestError::new(
            stray,
            format!("{op} takes no such field"),
        ));
    }
    if let Some(missing) = fields
        .iter()
        .find(|field| field.required && !object.contains_key(field.name))
    {
        return Err(RequestError::new(missing.name, "is required"));
    }
    let read = Read { object };
    build(op, &read)
}

/// An op's fields, read by name and type.
struct Read<'a> {
    object: &'a Map<String, Value>,
}

impl Read<'_> {
    fn value(&self, name: &str) -> Option<&Value> {
        self.object.get(name)
    }

    fn slug(&self, name: &str) -> Result<Option<Slug>, RequestError> {
        self.value(name).map(|value| slug(name, value)).transpose()
    }

    fn task(&self) -> Result<Slug, RequestError> {
        self.slug("task")?
            .ok_or_else(|| RequestError::new("task", "is required"))
    }

    fn text(&self, name: &str) -> Result<Option<String>, RequestError> {
        self.value(name).map(|value| text(name, value)).transpose()
    }

    fn slugs(&self, name: &str) -> Result<Option<Vec<Slug>>, RequestError> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let items = value
            .as_array()
            .ok_or_else(|| RequestError::new(name, "must be a list of task ids"))?;
        let at = |index: usize| format!("{name}/{index}");
        items
            .iter()
            .enumerate()
            .map(|(index, item)| slug(&at(index), item))
            .collect::<Result<_, _>>()
            .map(Some)
    }

    fn status(&self, name: &str) -> Result<Option<TaskStatus>, RequestError> {
        let words: Vec<&str> = TaskStatus::ALL
            .iter()
            .map(|status| status.as_str())
            .collect();
        self.word(name, TaskStatus::parse, &words)
    }

    fn word<T>(
        &self,
        name: &str,
        parse: impl Fn(&str) -> Option<T>,
        words: &[&str],
    ) -> Result<Option<T>, RequestError> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let refused = || RequestError::new(name, format!("must be one of {}", words.join(", ")));
        value.as_str().and_then(parse).map(Some).ok_or_else(refused)
    }

    fn pr(&self) -> Result<u64, RequestError> {
        let refused = || RequestError::new("pr", "must be a pull request number from 1");
        self.value("pr")
            .and_then(Value::as_u64)
            .filter(|pr| *pr >= 1)
            .ok_or_else(refused)
    }
}

fn slug(name: &str, value: &Value) -> Result<Slug, RequestError> {
    let text = value
        .as_str()
        .ok_or_else(|| RequestError::new(name, "must be a task id"))?;
    Slug::parse(text).map_err(|error| RequestError::new(name, error.problem))
}

/// Text as sent; the use case and the task's schema read the rest.
fn text(name: &str, value: &Value) -> Result<String, RequestError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| RequestError::new(name, "must be text"))
}

fn build(op: &str, read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let change = |op: BoardOp| -> Result<BoardRequest, RequestError> {
        Ok(BoardRequest::Change(read.task()?, op))
    };
    match op {
        "bootstrap" => Ok(BoardRequest::Bootstrap),
        "list" => list(read),
        "list_ready" => Ok(BoardRequest::ListReady),
        "list_takeable" => Ok(BoardRequest::ListTakeable),
        "show" => Ok(BoardRequest::Show(read.task()?)),
        "create" => create(read),
        "update_draft" => update_draft(read),
        "move" => {
            let to = read.status("to")?.expect("a required field is present");
            change(BoardOp::Move(to))
        }
        "renew" => change(BoardOp::Renew),
        "take_over" => change(BoardOp::TakeOver),
        "link_pr" => change(BoardOp::AddPr(read.pr()?)),
        "unlink_pr" => change(BoardOp::RemovePr(read.pr()?)),
        "complete" => Ok(BoardRequest::Complete(read.task()?)),
        "clear_set_aside" => {
            let path = read
                .value("path")
                .and_then(Value::as_str)
                .filter(|path| plain_board_file(path))
                .ok_or_else(|| {
                    RequestError::new("path", "must be a board file's path under tasks/")
                })?;
            let keep = match read.value("keep") {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .filter(|oid| object_id(oid))
                        .ok_or_else(|| RequestError::new("keep", "must be a full git object id"))?
                        .to_string(),
                ),
            };
            Ok(BoardRequest::Clear {
                path: path.to_string(),
                keep,
            })
        }
        other => unreachable!("op {other} is in OPS but not built"),
    }
}

/// A full SHA-1 or SHA-256 object name in lowercase hex.
fn object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn list(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let status = read.status("status")?;
    let parent = read.slug("parent")?;
    let dependents = read.slug("dependents_of")?;
    match (status, parent, dependents) {
        (None, None, None) => Ok(BoardRequest::List(ListFilter::All)),
        (Some(status), None, None) => Ok(BoardRequest::List(ListFilter::Status(status))),
        (None, Some(parent), None) => Ok(BoardRequest::List(ListFilter::Children(parent))),
        (None, None, Some(task)) => Ok(BoardRequest::List(ListFilter::Dependents(task))),
        (status, parent, _) => {
            // The second filter given is the one refused.
            let second = match (status.is_some(), parent.is_some()) {
                (true, true) => "parent",
                (true, false) | (false, true) | (false, false) => "dependents_of",
            };
            Err(RequestError::new(
                second,
                "list takes at most one of status, parent and dependents_of",
            ))
        }
    }
}

fn create(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let kinds: Vec<&str> = TaskKind::ALL.iter().map(|kind| kind.as_str()).collect();
    let status = read.word(
        "status",
        |word| match word {
            "draft" => Some(NewBoardTaskStatus::Draft),
            "ready" => Some(NewBoardTaskStatus::Ready),
            _ => None,
        },
        &["draft", "ready"],
    )?;
    Ok(BoardRequest::Create(NewBoardTask {
        id: read.task()?,
        title: read.text("title")?.expect("a required field is present"),
        kind: read
            .word("kind", TaskKind::parse, &kinds)?
            .unwrap_or(TaskKind::Task),
        description: read.text("description")?.unwrap_or_default(),
        status: status.unwrap_or(NewBoardTaskStatus::Draft),
        parent: read.slug("parent")?,
        depends_on: read.slugs("depends_on")?.unwrap_or_default(),
    }))
}

fn update_draft(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let parent = match read.value("parent") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(slug("parent", value)?)),
    };
    let expected_updated = match read.value("expected_updated") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(|text| Timestamp::parse(text).ok())
                .ok_or_else(|| {
                    RequestError::new(
                        "expected_updated",
                        "must be a time like 2026-10-10T08:00:00Z",
                    )
                })?,
        ),
    };
    let update = DraftUpdate {
        id: read.task()?,
        title: read.text("title")?,
        description: read.text("description")?,
        parent,
        depends_on: read.slugs("depends_on")?,
        expected_updated,
    };
    let sets_something = update.title.is_some()
        || update.description.is_some()
        || update.parent.is_some()
        || update.depends_on.is_some();
    match sets_something {
        true => Ok(BoardRequest::UpdateDraft(update)),
        false => Err(RequestError::new(
            "op",
            "update_draft sets nothing: name a field to change",
        )),
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
