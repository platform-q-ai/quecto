//! A board request as a front end sends it: one JSON object naming its
//! `op` and that op's own fields, each of its own type. Any other field is
//! refused. Text is taken as sent: the use cases turn CRLF line ends to LF,
//! once, for every front end.
use serde_json::{Map, Value};

use crate::application::project_board::dto::{
    BoardOp, DraftUpdate, NewBoardTask, NewBoardTaskStatus,
};
use crate::application::project_board::operations::{BoardRequest, ListFilter, PlanPage, PlanView};
use crate::application::project_board::ports::KeepCopy;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{FaultKind, RequestFault, StaticKind};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use std::collections::{BTreeMap, BTreeSet};

/// Why a request was refused before the board was read: the field (or
/// `op`) and what it must be, and as the call's record holds it (epic
/// #2481 T3): which schema field, never the request's own text, and how it
/// was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestError {
    pub field: String,
    pub problem: String,
    pub fault: RequestFault,
}

impl RequestError {
    /// `field` was wrong as `fault` says: recorded as the schema field it
    /// names ([`schema_field`]). A key no schema names is [`Self::stray`].
    pub fn of(field: &str, problem: impl Into<String>, fault: FaultKind) -> Self {
        let recorded = match fault {
            // Whatever text names it, a stray key is never recorded.
            FaultKind::Unexpected => StaticKind::OTHER,
            FaultKind::Missing
            | FaultKind::WrongType
            | FaultKind::InvalidValue
            | FaultKind::TooLarge
            | FaultKind::NotJson
            | FaultKind::DuplicateKey => schema_field(field),
        };
        Self::recorded(field, problem, fault, recorded, None)
    }

    /// The request as a whole was wrong (too big, not JSON, not an
    /// object), whichever field the answer names: recorded as `request`.
    pub fn whole(field: &str, problem: impl Into<String>, fault: FaultKind) -> Self {
        Self::recorded(field, problem, fault, StaticKind::new("request"), None)
    }

    /// The request named `strays` keys no schema of its op names, `field`
    /// the first: recorded as `other`, counted, never named.
    pub fn stray(field: &str, problem: impl Into<String>, strays: usize) -> Self {
        let count = u64::try_from(strays).unwrap_or(u64::MAX);
        let fault = FaultKind::Unexpected;
        Self::recorded(field, problem, fault, StaticKind::OTHER, Some(count))
    }

    fn recorded(
        field: &str,
        problem: impl Into<String>,
        fault: FaultKind,
        recorded: StaticKind,
        unexpected_count: Option<u64>,
    ) -> Self {
        Self {
            field: field.into(),
            problem: problem.into(),
            fault: RequestFault {
                field: recorded,
                fault,
                unexpected_count,
            },
        }
    }
}

/// The fields a refusal names that are no op's: the op, and the request
/// as a whole.
const PSEUDO_FIELDS: &[&str] = &["op", "request"];

/// The schema field `field` names, as [`OPS`] spells it: its name up to
/// any `/` (`depends_on/0` is `depends_on`) when that is a field of an op
/// or one of [`PSEUDO_FIELDS`]; any other text is `other`. Only the
/// table's own `&'static str` reaches the kind, never the request's text.
pub fn schema_field(field: &str) -> StaticKind {
    let name = field.split('/').next().unwrap_or_default();
    OPS.iter()
        .flat_map(|(_, fields)| fields.iter().map(|field| field.name))
        .chain(PSEUDO_FIELDS.iter().copied())
        .find(|known| *known == name)
        .map_or(StaticKind::OTHER, StaticKind::new)
}

/// How a value of the wrong kind was wrong: not even the JSON type a field
/// takes (`is_type`), or of that type but no value it takes.
fn misfit(is_type: bool) -> FaultKind {
    match is_type {
        true => FaultKind::InvalidValue,
        false => FaultKind::WrongType,
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
    /// Where a page reads on from: a character or an item, from 0.
    Offset,
    /// A plan's version, from 1.
    Version,
    /// A plan's task items: `[{"id", "title", "criteria", "depends_on"}]`,
    /// as `show` gives them.
    PlanItems,
    /// A plan's team, as `show` gives it: `{"roles": [{"name", "model",
    /// "effort", "members", "turns"}], "budget_tokens", "deadline",
    /// "image"}`, or `null` for none.
    Team,
    /// A board file's path, as a listing's `quarantined` shows it.
    Path,
    /// A git object name, as a listing's `quarantined` shows it.
    ObjectId,
    /// For each name a listing shows more than once, by path, which copy
    /// stays: `{"keep": object, "seen": [objects]}` or `{"drop":
    /// [objects]}`, naming every copy's object the listing showed.
    Repeated,
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
    (
        "show",
        &[
            TASK,
            optional("approach_from", Kind::Offset),
            optional("items_from", Kind::Offset),
            optional("version", Kind::Version),
        ],
    ),
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
        "propose_plan",
        &[
            TASK,
            required("approach", Kind::Text),
            required("items", Kind::PlanItems),
            optional("team", Kind::Team),
        ],
    ),
    (
        "edit_plan",
        &[
            TASK,
            required("version", Kind::Version),
            optional("approach", Kind::Text),
            optional("items", Kind::PlanItems),
            optional("team", Kind::Team),
            optional("expected_updated", Kind::Timestamp),
        ],
    ),
    (
        "accept_plan",
        &[
            TASK,
            required("version", Kind::Version),
            required("expected_updated", Kind::Timestamp),
        ],
    ),
    (
        "discard_plan",
        &[
            TASK,
            required("version", Kind::Version),
            optional("expected_updated", Kind::Timestamp),
        ],
    ),
    (
        "clear_set_aside",
        &[
            required("path", Kind::Path),
            required("object", Kind::ObjectId),
            optional("repeated", Kind::Repeated),
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
    let object = value.as_object().ok_or_else(|| {
        RequestError::whole("op", "a request is a JSON object", FaultKind::WrongType)
    })?;
    let op = match object.get("op") {
        Some(Value::String(op)) => op.as_str(),
        named => {
            let fault = named.map_or(FaultKind::Missing, |_| FaultKind::WrongType);
            return Err(RequestError::of("op", "must name an op", fault));
        }
    };
    let fields = fields_of(op).ok_or_else(|| {
        let ops: Vec<&str> = OPS.iter().map(|(name, _)| *name).collect();
        let problem = format!("must be one of {}", ops.join(", "));
        RequestError::of("op", problem, FaultKind::InvalidValue)
    })?;
    let taken = |key: &String| key == "op" || fields.iter().any(|field| field.name == *key);
    let strays: Vec<&String> = object.keys().filter(|key| !taken(key)).collect();
    if let Some(stray) = strays.first() {
        let problem = format!("{op} takes no such field");
        return Err(RequestError::stray(stray, problem, strays.len()));
    }
    if let Some(missing) = fields
        .iter()
        .find(|field| field.required && !object.contains_key(field.name))
    {
        return Err(RequestError::of(
            missing.name,
            "is required",
            FaultKind::Missing,
        ));
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
            .ok_or_else(|| RequestError::of("task", "is required", FaultKind::Missing))
    }

    fn text(&self, name: &str) -> Result<Option<String>, RequestError> {
        self.value(name).map(|value| text(name, value)).transpose()
    }

    fn slugs(&self, name: &str) -> Result<Option<Vec<Slug>>, RequestError> {
        let Some(value) = self.value(name) else {
            return Ok(None);
        };
        let items = value.as_array().ok_or_else(|| {
            RequestError::of(name, "must be a list of task ids", FaultKind::WrongType)
        })?;
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
        let refused = || {
            let problem = format!("must be one of {}", words.join(", "));
            RequestError::of(name, problem, misfit(value.is_string()))
        };
        value.as_str().and_then(parse).map(Some).ok_or_else(refused)
    }

    /// A time as the board spells it, when given.
    fn timestamp(&self, name: &str) -> Result<Option<Timestamp>, RequestError> {
        self.value(name)
            .map(|value| timestamp(name, value))
            .transpose()
    }

    /// An offset, or none when it is absent or `null`.
    fn offset(&self, name: &str) -> Result<Option<usize>, RequestError> {
        let value = match self.value(name) {
            None | Some(Value::Null) => return Ok(None),
            Some(value) => value,
        };
        let refused = || {
            let problem = "must be a character or item to read on from, from 0";
            RequestError::of(name, problem, misfit(value.is_number()))
        };
        value
            .as_u64()
            .and_then(|offset| usize::try_from(offset).ok())
            .map(Some)
            .ok_or_else(refused)
    }

    fn pr(&self) -> Result<u64, RequestError> {
        let refused = || {
            let fault = misfit(self.value("pr").is_some_and(Value::is_number));
            RequestError::of("pr", "must be a pull request number from 1", fault)
        };
        self.value("pr")
            .and_then(Value::as_u64)
            .filter(|pr| *pr >= 1)
            .ok_or_else(refused)
    }
}

/// A time as the board spells it.
fn timestamp(name: &str, value: &Value) -> Result<Timestamp, RequestError> {
    value
        .as_str()
        .and_then(|text| Timestamp::parse(text).ok())
        .ok_or_else(|| {
            RequestError::of(
                name,
                "must be a time like 2026-10-10T08:00:00Z",
                misfit(value.is_string()),
            )
        })
}

fn slug(name: &str, value: &Value) -> Result<Slug, RequestError> {
    let text = value
        .as_str()
        .ok_or_else(|| RequestError::of(name, "must be a task id", FaultKind::WrongType))?;
    Slug::parse(text)
        .map_err(|error| RequestError::of(name, error.problem, FaultKind::InvalidValue))
}

/// Text as sent; the use case and the task's schema read the rest.
fn text(name: &str, value: &Value) -> Result<String, RequestError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| RequestError::of(name, "must be text", FaultKind::WrongType))
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
        "show" => Ok(BoardRequest::Show(
            read.task()?,
            PlanView {
                version: read.value("version").map(plans::version).transpose()?,
                page: PlanPage::of(read.offset("approach_from")?, read.offset("items_from")?),
            },
        )),
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
        "propose_plan" => plans::propose_plan(read),
        "edit_plan" => plans::edit_plan(read),
        "accept_plan" => plans::accept_plan(read),
        "discard_plan" => plans::discard_plan(read),
        "clear_set_aside" => clear(read),
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

/// The most bytes a path a clear names may take. A listing's path is at
/// most a few kilobytes (a kept name is an id and its extension; a stray's
/// is quoted to at most 2,048 characters, each at most four bytes), so this
/// bound refuses only what no listing could show.
const MAX_PATH_BYTES: usize = 9 * 1024;

/// A clear names files as the listing shows them: whether the board set
/// them aside is the use case's check, against the board itself.
fn clear(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let given = read.value("path").and_then(Value::as_str);
    let path = given
        .filter(|path| !path.is_empty() && path.len() <= MAX_PATH_BYTES)
        .ok_or_else(|| {
            let fault = match given {
                Some(path) if path.len() > MAX_PATH_BYTES => FaultKind::TooLarge,
                Some(_) => FaultKind::InvalidValue,
                None => FaultKind::WrongType,
            };
            RequestError::of(
                "path",
                "must be a path a listing's quarantined shows",
                fault,
            )
        })?;
    let object = object_value("object", read.value("object"))?;
    let repeated = match read.value("repeated") {
        None => BTreeMap::new(),
        Some(value) => repeated(value)?,
    };
    Ok(BoardRequest::Clear {
        path: path.to_string(),
        object,
        repeated,
    })
}

fn object_value(name: &str, value: Option<&Value>) -> Result<String, RequestError> {
    value
        .and_then(Value::as_str)
        .filter(|oid| object_id(oid))
        .map(str::to_owned)
        .ok_or_else(|| {
            let fault = match value {
                Some(value) => misfit(value.is_string()),
                None => FaultKind::Missing,
            };
            RequestError::of(name, "must be a full object id a listing shows", fault)
        })
}

/// The copies a clear keeps or drops, by path.
fn repeated(value: &Value) -> Result<BTreeMap<String, KeepCopy>, RequestError> {
    let shape = |at: &str, fault: FaultKind| {
        RequestError::of(
            at,
            "must map each repeated path to {\"keep\", \"seen\"} or {\"drop\"}",
            fault,
        )
    };
    let names = value
        .as_object()
        .ok_or_else(|| shape("repeated", FaultKind::WrongType))?;
    names
        .iter()
        .map(|(path, copy)| {
            let at = |key: &str| format!("repeated/{path}/{key}");
            let copy = copy
                .as_object()
                .ok_or_else(|| shape(&at(""), FaultKind::WrongType))?;
            let mut keys: Vec<&str> = copy.keys().map(String::as_str).collect();
            keys.sort_unstable();
            let keep = match keys.as_slice() {
                ["keep", "seen"] => KeepCopy::Object {
                    keep: object_value(&at("keep"), copy.get("keep"))?,
                    seen: objects(&at("seen"), &copy["seen"])?,
                },
                ["drop"] => KeepCopy::Drop(objects(&at("drop"), &copy["drop"])?),
                _ => return Err(shape(&at(""), FaultKind::InvalidValue)),
            };
            Ok((path.clone(), keep))
        })
        .collect()
}

/// A list of distinct object ids.
fn objects(at: &str, value: &Value) -> Result<BTreeSet<String>, RequestError> {
    let items = value
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            let fault = misfit(value.is_array());
            RequestError::of(at, "must list the objects the listing showed", fault)
        })?;
    let mut seen = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let object = object_value(&format!("{at}/{index}"), Some(item))?;
        if !seen.insert(object) {
            return Err(RequestError::of(
                &format!("{at}/{index}"),
                "is listed twice",
                FaultKind::InvalidValue,
            ));
        }
    }
    Ok(seen)
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
            Err(RequestError::of(
                second,
                "list takes at most one of status, parent and dependents_of",
                FaultKind::InvalidValue,
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
    let expected_updated = read.timestamp("expected_updated")?;
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
        // No field to change was given.
        false => Err(RequestError::of(
            "op",
            "update_draft sets nothing: name a field to change",
            FaultKind::Missing,
        )),
    }
}

#[path = "request_plans.rs"]
mod plans;

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
