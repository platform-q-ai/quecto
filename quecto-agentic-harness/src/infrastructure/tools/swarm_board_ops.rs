//! Structured `swarm` board ops (#2279, epic #2265 owner decision D4):
//! each member-facing `board.<method>` is a `swarm` op of the same name,
//! with the same arguments (named JSON fields) and the same result, except
//! that no op answers a bare `null` (#2394: a changed task's row, or what
//! the op changed, where Python answered `None`).
//!
//! [`BOARD_OPS`] is the one table of them: the named-argument binding (the
//! dispatcher's, `swarm_board_dispatch::signature`, which a test holds the
//! table to), the schema's `op` enum and fields ([`tool_schema`], which
//! the checked-in `tool_schema.json` must equal), and E2-S5's per-op MCP
//! schemas. [`board_op`] serves one op through `SwarmContext`'s composed
//! board, constructing nothing:
//!
//! 1. The member's text is read as Python's `json.loads` reads it
//!    ([`member_arguments`]: `-0` is the integer 0); a value the board's
//!    value type cannot hold exactly (an integer beyond i64 and u64, a
//!    number that overflows, such as `1e400`, a lone surrogate) is refused
//!    with the store's error shape, never coerced (parent decision on
//!    #2278/#2279). `NaN` and `±Infinity`, which are not JSON, never get
//!    here: the agent loop answers such a call as invalid arguments.
//! 2. The running gate the removed `op=run` had (owner decision 2026-09-28,
//!    overruling the epic's P4; `op=run` itself was removed in #2282):
//!    unless the run is running and within its deadline, the op is refused
//!    with the running gate's guidance.
//! 3. A mutating op ([`MUTATING_OPS`]) reads the event cursor
//!    (`_event_cursor`) before and after its call; when it moved, the
//!    post-call lifecycle (the one `op=run` ran until #2282) runs (settle
//!    when the run no longer runs, else wake hints), and any delivery
//!    warnings ride the answer. A read-only op ([`READ_ONLY_OPS`]) reads
//!    neither.
//! 4. The answer is written as Python's `json.dumps` writes it
//!    ([`wire_text`]); a board refusal keeps its `swarm: "<text>"`, the
//!    lifecycle's notes (when there are any) on the line after it.
//!
//! Every op leaves the dispatcher's `swarm_op` records (a refusal before
//! the board is recorded as the op's own, with the schema fields its text
//! could not hold, #2341) and one `tracing` record on
//! [`TELEMETRY_TARGET`]: the op, the gate, whether the cursor moved, what
//! the lifecycle did, the warnings counted and the answer's size, never
//! argument text.
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::swarm_board_dispatch::{
    BindingFaults, BoardWire, TELEMETRY_TARGET, unreadable_arguments,
};
use super::swarm_bridge::SwarmContext;
use super::swarm_control::AfterExecution;
use super::swarm_output::tool_err;
use crate::domain::error::DomainError;
use crate::domain::swarm::{NameForm, RefusalKind};
use crate::domain::tool::ToolResult;

/// One argument of a board op: its name, its JSON schema (as JSON text),
/// and whether it is required or has a default (as JSON text).
#[derive(Debug)]
pub struct ArgSpec {
    pub name: &'static str,
    pub json_type: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
}

/// One board op: its name (the board method's), its arguments in the
/// Python signature's order, and whether it only reads the board.
#[derive(Debug)]
pub struct OpSpec {
    pub name: &'static str,
    pub args: &'static [ArgSpec],
    pub read_only: bool,
}

const INTEGER: &str = r#"{"type":"integer"}"#;
const STRING: &str = r#"{"type":"string"}"#;
const BOOLEAN: &str = r#"{"type":"boolean"}"#;
const STRINGS: &str = r#"{"type":"array","items":{"type":"string"}}"#;
/// The fields some op defaults to `null` (#2279 review N8): one schema per
/// field, so each admits `null` in every op that names it.
const NULLABLE_STRING: &str = r#"{"type":["string","null"]}"#;
const NULLABLE_INTEGER: &str = r#"{"type":["integer","null"]}"#;
const NULLABLE_INTEGERS: &str = r#"{"type":["array","null"],"items":{"type":"integer"}}"#;
const KIND: &str = r#"{"type":"string","enum":["command","review"]}"#;
const EVIDENCE: &str = r#"{"type":"array","items":{"type":"object","properties":{"artifact":{"type":"string"},"revision":{"type":"string"}},"required":["artifact","revision"]}}"#;
const CRITERIA: &str = r#"{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"kind":{"type":"string","enum":["command","review"]},"description":{"type":"string"}},"required":["id","kind","description"]}}"#;

const fn required(name: &'static str, json_type: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        json_type,
        required: true,
        default: None,
    }
}

const fn defaulted(name: &'static str, json_type: &'static str, default: &'static str) -> ArgSpec {
    ArgSpec {
        name,
        json_type,
        required: false,
        default: Some(default),
    }
}

const TASK_ID: ArgSpec = required("task_id", INTEGER);
const TOKEN: ArgSpec = required("token", STRING);
const REASON: ArgSpec = required("reason", STRING);
const REVISION: ArgSpec = required("revision", NULLABLE_STRING);
const MESSAGE_ID: ArgSpec = required("message_id", INTEGER);
const PAGE: [ArgSpec; 2] = [
    defaulted("offset", INTEGER, "0"),
    defaulted("limit", INTEGER, "50"),
];

const fn op(name: &'static str, args: &'static [ArgSpec], read_only: bool) -> OpSpec {
    OpSpec {
        name,
        args,
        read_only,
    }
}

/// The epic's structured-op mapping table, in its order: every
/// member-facing `board.` method the harness has no op of its own for.
pub const BOARD_OPS: &[OpSpec] = &[
    op("task", &[TASK_ID], true),
    op("tasks", &PAGE, true),
    op("file_owners", &PAGE, true),
    op(
        "task_create",
        &[
            required("request", STRING),
            required("title", STRING),
            required("acceptance", STRINGS),
            defaulted("dependencies", NULLABLE_INTEGERS, "null"),
        ],
        false,
    ),
    op(
        "dependencies",
        &[TASK_ID, required("dependencies", NULLABLE_INTEGERS)],
        false,
    ),
    op("claim", &[TASK_ID], false),
    op("release", &[TASK_ID, TOKEN], false),
    op("block", &[TASK_ID, TOKEN, REASON], false),
    op("unblock", &[TASK_ID, TOKEN, REASON], false),
    op(
        "submit",
        &[TASK_ID, TOKEN, required("evidence", EVIDENCE)],
        false,
    ),
    op(
        "reserve",
        &[TASK_ID, TOKEN, required("paths", STRINGS)],
        false,
    ),
    op(
        "release_files",
        &[TASK_ID, TOKEN, required("reservation", STRING)],
        false,
    ),
    op(
        "send",
        &[
            required("request", STRING),
            required("recipient", STRING),
            required("body", STRING),
            defaulted("revision", NULLABLE_STRING, "null"),
            defaulted("supersedes", NULLABLE_INTEGER, "null"),
        ],
        false,
    ),
    op("withdraw", &[MESSAGE_ID], false),
    op(
        "inbox",
        &[defaulted("include_consumed", BOOLEAN, "false")],
        true,
    ),
    op("ack", &[MESSAGE_ID], false),
    op(
        "evidence",
        &[
            required("criterion", STRING),
            required("artifact", STRING),
            REVISION,
            required("kind", KIND),
            required("passed", BOOLEAN),
        ],
        false,
    ),
    op(
        "amend",
        &[
            required("goal", STRING),
            required("constraints", STRINGS),
            required("criteria", CRITERIA),
            REASON,
        ],
        false,
    ),
    op("verify_task", &[TASK_ID, TOKEN, REVISION], false),
    op(
        "revalidate_task",
        &[TASK_ID, REVISION, required("evidence", EVIDENCE)],
        false,
    ),
    op(
        "recover",
        &[TASK_ID, defaulted("release_files", BOOLEAN, "false")],
        false,
    ),
    op("revoke", &[TASK_ID, REASON], false),
    op("complete", &[REVISION], false),
    op("stop", &[required("status", STRING), REASON], false),
    op("usage_report", &[], true),
];

/// The ops that only read the board: no cursor reads, no lifecycle.
pub const READ_ONLY_OPS: &[&str] = &["task", "tasks", "file_owners", "inbox", "usage_report"];

/// The ops that may change the board: the cursor is read around each, and
/// the lifecycle runs when it moved.
pub const MUTATING_OPS: &[&str] = &[
    "task_create",
    "dependencies",
    "claim",
    "release",
    "block",
    "unblock",
    "submit",
    "reserve",
    "release_files",
    "send",
    "withdraw",
    "ack",
    "evidence",
    "amend",
    "verify_task",
    "revalidate_task",
    "recover",
    "revoke",
    "complete",
    "stop",
];

/// The harness's own ops, before the board ops in the schema's enum.
pub const HARNESS_OPS: &[&str] = &[
    "create",
    "summary",
    "reconcile",
    "cancel_run",
    "pause",
    "resume",
    "events",
    "usage",
    "usage_budget",
];

/// The swarm's tasks are task items (#2478): `(task-item name, old name)`
/// for each op whose name says task. A task-item name is read as the old
/// one before anything else, so the board, its events, the golden fixtures
/// and every record keep the old names; each record says which names the
/// call used ([`name_form`]).
pub const OP_ALIASES: &[(&str, &str)] = &[
    ("task_item", "task"),
    ("task_items", "tasks"),
    ("task_item_create", "task_create"),
    ("verify_task_item", "verify_task"),
    ("revalidate_task_item", "revalidate_task"),
];

/// `task_id`'s task-item name, `(task-item name, old name)`: one argument
/// under two names, on every op that takes it.
pub const ID_ALIAS: (&str, &str) = ("task_item_id", "task_id");

/// The op `name` names: the old name a task-item name stands for, or
/// `name` itself.
pub fn canonical_op(name: &str) -> &str {
    match OP_ALIASES.iter().find(|(alias, _)| *alias == name) {
        Some((_, old)) => old,
        None => name,
    }
}

/// The name the tool advertises the op `name` (an old name) by: its
/// task-item name, or `name` itself (#2478 slice B).
pub fn advertised_op(name: &str) -> &str {
    match OP_ALIASES.iter().find(|(_, old)| *old == name) {
        Some((alias, _)) => alias,
        None => name,
    }
}

/// The name the tool advertises the argument `name` by: `task_item_id`
/// for `task_id`, or `name` itself (#2478 slice B).
pub fn advertised_arg(name: &str) -> &str {
    match name == ID_ALIAS.1 {
        true => ID_ALIAS.0,
        false => name,
    }
}

fn takes_task_id(spec: &OpSpec) -> bool {
    spec.args.iter().any(|arg| arg.name == ID_ALIAS.1)
}

/// The name `key`, given to `spec`, binds as: `task_id` for `task_item_id`
/// when `spec` takes `task_id`, else `key` (an op that takes no task id
/// refuses `task_item_id` under the name it was sent as).
fn canonical_key<'key>(spec: &OpSpec, key: &'key str) -> &'key str {
    match key == ID_ALIAS.0 && takes_task_id(spec) {
        true => ID_ALIAS.1,
        false => key,
    }
}

/// `fields`, given to `spec`, with `task_item_id` read as `task_id` in its
/// place (the binding names the first unexpected field in the member's
/// order) or dropped when it equals `task_id`; two values (`1` and `1.0`
/// differ, as `null` and `1` do) are left for the binding to refuse, after
/// the running gate, as an argument given twice.
fn canonical_fields(spec: &OpSpec, fields: Map<String, Value>) -> Map<String, Value> {
    let (alias, old) = ID_ALIAS;
    let mapped = match (takes_task_id(spec), fields.get(alias), fields.get(old)) {
        (true, Some(given), Some(same)) if given == same => {
            let mut fields = fields;
            fields.shift_remove(alias);
            fields
        }
        (true, Some(_), None) => fields
            .into_iter()
            .map(|(key, value)| (canonical_key(spec, &key).to_owned(), value))
            .collect(),
        (false, _, _) | (true, None, _) | (true, Some(_), Some(_)) => fields,
    };
    debug_assert!(
        !takes_task_id(spec) || !mapped.contains_key(alias) || mapped.contains_key(old),
        "task_item_id is left only beside a task_id of another value"
    );
    mapped
}

/// Which names a call used (#2478): `asked`, the op name it gave for
/// `spec`, when `spec` was renamed, and `task_id` or `task_item_id` among
/// `keys`, its arguments' names. [`NameForm::Legacy`] when it used any old
/// name (a call that would fail once they are retired),
/// [`NameForm::Current`] when it used task-item names only, `None` when it
/// used neither.
fn name_form(asked: &str, spec: &OpSpec, keys: &[String]) -> Option<NameForm> {
    let renamed = OP_ALIASES.iter().any(|(_, old)| *old == spec.name);
    let op = renamed.then_some(match asked == spec.name {
        true => NameForm::Legacy,
        false => NameForm::Current,
    });
    let given = |name: &str| keys.iter().any(|key| key == name);
    let old_id = given(ID_ALIAS.1).then_some(NameForm::Legacy);
    let new_id = given(ID_ALIAS.0).then_some(NameForm::Current);
    [op, old_id, new_id]
        .into_iter()
        .flatten()
        .reduce(|first, second| match (first, second) {
            (NameForm::Current, NameForm::Current) => NameForm::Current,
            (NameForm::Legacy, _) | (_, NameForm::Legacy) => NameForm::Legacy,
        })
}

/// The top-level keys of `text`, read without their values, so arguments
/// the board cannot hold (`1e400`) still name their fields; none for text
/// that is no JSON object.
fn keys_of(text: &str) -> Vec<String> {
    serde_json::from_str::<std::collections::BTreeMap<String, serde::de::IgnoredAny>>(text)
        .map(|fields| fields.into_keys().collect())
        .unwrap_or_default()
}

/// The harness ops' fields no board op has, with their JSON schemas.
const HARNESS_PROPERTIES: &[(&str, &str)] = &[
    (
        "member_limit",
        r#"{"type":"integer","minimum":1,"maximum":25}"#,
    ),
    ("deadline", r#"{"type":"number"}"#),
    ("deadline_in_seconds", r#"{"type":"number"}"#),
    ("after", r#"{"type":"integer","minimum":0}"#),
    ("since", r#"{"type":"integer","minimum":0}"#),
    ("token_limit", r#"{"type":["integer","null"],"minimum":1}"#),
    ("strict_unknown", BOOLEAN),
];

/// The op named `name` in [`BOARD_OPS`].
pub fn op_spec(name: &str) -> Option<&'static OpSpec> {
    BOARD_OPS.iter().find(|spec| spec.name == name)
}

fn schema_of(text: &str) -> Value {
    serde_json::from_str(text).expect("a checked-in JSON schema fragment")
}

/// The `swarm` tool's parameter schema, rendered from [`HARNESS_OPS`],
/// [`BOARD_OPS`] and the harness's own fields: a field several ops share
/// has one schema.
pub fn tool_schema() -> Value {
    // Every op by the name it is advertised by: a task item's in place of
    // a task's (#2478 slice B); the old names are still taken, not shown.
    let ops: Vec<&str> = HARNESS_OPS
        .iter()
        .copied()
        .chain(BOARD_OPS.iter().map(|spec| advertised_op(spec.name)))
        .collect();
    let mut properties = Map::new();
    properties.insert("op".to_owned(), json!({"type": "string", "enum": ops}));
    let fields = HARNESS_PROPERTIES
        .iter()
        .copied()
        .chain(BOARD_OPS.iter().flat_map(|spec| {
            spec.args
                .iter()
                .map(|arg| (advertised_arg(arg.name), arg.json_type))
        }));
    for (name, text) in fields {
        let schema = schema_of(text);
        match properties.get(name) {
            Some(existing) => assert_eq!(existing, &schema, "{name} has one schema"),
            None => {
                properties.insert(name.to_owned(), schema);
            }
        }
    }
    json!({"type": "object", "properties": properties, "required": ["op"]})
}

/// A member's argument text as `wire` reads it (composition binds Python's
/// `json.loads`), as the value the board takes, or the refusal
/// (`arguments: <why>`) for text that is no JSON or holds what that value
/// cannot hold exactly.
///
/// # Errors
/// The refusal text.
pub fn member_arguments(wire: &BoardWire, text: &str) -> Result<Value, String> {
    (wire.read)(text).map_err(|error| format!("arguments: {error}"))
}

/// `value` as `wire` writes an answer (composition binds Python's plain
/// `json.dumps`: insertion order, `", "` and `": "`, `ensure_ascii`, floats
/// as `repr`).
///
/// # Errors
/// A value nested beyond what the codec converts or writes.
pub fn wire_text(wire: &BoardWire, value: &Value) -> Result<String, String> {
    (wire.write)(value)
}

/// A structured op the tool was asked for: its table row, its named
/// arguments (the request's fields but `op`) or why the text was refused,
/// and the codec its answer is written with.
pub(super) struct BoardOpRequest {
    spec: &'static OpSpec,
    /// The arguments, or why the text was refused and which fields it
    /// could not hold (#2341).
    arguments: Result<Map<String, Value>, (String, BindingFaults)>,
    wire: BoardWire,
    /// Which names the call used where the swarm's tasks were renamed
    /// task items (#2478); `None` where none applies.
    name_form: Option<NameForm>,
}

/// The structured op `text` asks for, read by `wire`, when its `op` names
/// one; `None` for any other request, which the harness ops answer.
pub(super) fn requested(text: &str, wire: BoardWire) -> Option<BoardOpRequest> {
    // A task-item name is read as the old one before anything else
    // (#2478), and which names the call used is kept for its records.
    let asked = (wire.op)(text)?;
    let spec = op_spec(canonical_op(&asked))?;
    let read = member_arguments(&wire, text);
    let keys: Vec<String> = match &read {
        Ok(Value::Object(fields)) => fields.keys().cloned().collect(),
        Ok(_) => Vec::new(),
        Err(_) => keys_of(text),
    };
    let name_form = name_form(&asked, spec, &keys);
    let arguments = read.and_then(|value| match value {
        Value::Object(mut fields) => {
            // Keeps the member's order (#2279 final review): the binding
            // names the first unexpected field, as Python names the first
            // unexpected keyword; `remove` would move the last into `op`'s
            // slot.
            fields.shift_remove("op");
            Ok(canonical_fields(spec, fields))
        }
        _ => Err("arguments: not a JSON object".to_owned()),
    });
    let arguments = arguments.map_err(|refusal| {
        let keys = (wire.unreadable)(text).map(|keys| {
            keys.iter()
                .map(|key| canonical_key(spec, key).to_owned())
                .collect()
        });
        (refusal, unreadable_arguments(keys))
    });
    Some(BoardOpRequest {
        spec,
        arguments,
        wire,
        name_form,
    })
}

/// What one structured op did, for its `tracing` record.
struct OpRecord {
    op: &'static str,
    read_only: bool,
    gate: &'static str,
    cursor_moved: Option<bool>,
    lifecycle: &'static str,
    warnings: usize,
    /// Which names the call used (#2478); `None` where none applies.
    name_form: Option<NameForm>,
}

/// Serves `request` through `context`'s board: see the module docs.
pub(super) async fn board_op(
    context: SwarmContext,
    request: BoardOpRequest,
) -> Result<ToolResult, DomainError> {
    let started = Instant::now();
    let spec = request.spec;
    debug_assert!(
        spec.read_only == READ_ONLY_OPS.contains(&spec.name)
            && spec.read_only != MUTATING_OPS.contains(&spec.name),
        "every board op is read-only or mutating, never both"
    );
    let mut record = OpRecord {
        op: spec.name,
        read_only: spec.read_only,
        gate: "open",
        cursor_moved: None,
        lifecycle: "none",
        warnings: 0,
        name_form: request.name_form,
    };
    let result = serve(&context, request, &mut record, started).await;
    trace(&record, &result, started.elapsed());
    result
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, DomainError> {
    super::call_work::spawn_blocking_in_call(job)
        .await
        .map_err(|error| DomainError::Tool(error.to_string()))
}

/// Records `record`'s op as refused with `kind` before the board, with
/// what it found wrong in the arguments (#2341) and which names the call
/// used (#2478), then answers `text`.
async fn refuse(
    context: &SwarmContext,
    record: &OpRecord,
    (kind, faults): (RefusalKind, BindingFaults),
    started: Instant,
    text: String,
) -> Result<ToolResult, DomainError> {
    let ctx = context.clone();
    let (op, name_form) = (record.op, record.name_form);
    let elapsed = started.elapsed();
    blocking(move || ctx.refused(op, kind, faults, name_form, elapsed)).await?;
    tool_err(text)
}

async fn serve(
    context: &SwarmContext,
    request: BoardOpRequest,
    record: &mut OpRecord,
    started: Instant,
) -> Result<ToolResult, DomainError> {
    let op = request.spec.name;
    let arguments = match request.arguments {
        Ok(arguments) => arguments,
        Err((refusal, faults)) => {
            record.gate = "arguments";
            // Shaped as every board refusal reaches the member.
            let text = DomainError::Tool(format!("swarm: {}", Value::String(refusal))).to_string();
            let refusal = (RefusalKind::Invalid, faults);
            return refuse(context, record, refusal, started, text).await;
        }
    };
    let ctx = context.clone();
    let status = match blocking(move || ctx.call("_status", json!([]))).await? {
        Ok(status) => status,
        Err(error) => return unreadable(context, record, started, &error).await,
    };
    if let Some((kind, gate, text)) =
        super::swarm::swarm_guidance::running_gate(advertised_op(op), &status)
    {
        record.gate = gate;
        return refuse(context, record, (kind, BindingFaults::NONE), started, text).await;
    }
    let before = match record.read_only {
        true => None,
        false => match cursor(context).await? {
            Ok(cursor) => Some(cursor),
            Err(error) => return unreadable(context, record, started, &error).await,
        },
    };
    let ctx = context.clone();
    let name_form = record.name_form;
    let answer = blocking(move || ctx.call_named(op, Value::Object(arguments), name_form)).await?;
    let mut notes = Map::new();
    if let Some(before) = before {
        lifecycle(context, before, record, &mut notes).await?;
    }
    super::swarm_output::answered(&request.wire, op, answer, notes)
}

/// The gate's `_status` read or the first `_event_cursor` read failed
/// (#2279 review M2): the op is refused before the board with the store's
/// text, recorded as its own store refusal, its gate `unreadable`.
async fn unreadable(
    context: &SwarmContext,
    record: &mut OpRecord,
    started: Instant,
    error: &DomainError,
) -> Result<ToolResult, DomainError> {
    record.gate = "unreadable";
    let refusal = (RefusalKind::Store, BindingFaults::NONE);
    refuse(context, record, refusal, started, error.to_string()).await
}

/// The board's event cursor, `_event_cursor`.
async fn cursor(context: &SwarmContext) -> Result<Result<Value, DomainError>, DomainError> {
    let ctx = context.clone();
    blocking(move || ctx.call("_event_cursor", json!([]))).await
}

/// The post-call lifecycle (`op=run`'s until #2282 removed it) when the
/// cursor moved past `before`:
/// what it did goes on `record`, its warnings or failure into `notes`.
async fn lifecycle(
    context: &SwarmContext,
    before: Value,
    record: &mut OpRecord,
    notes: &mut Map<String, Value>,
) -> Result<(), DomainError> {
    let after = match cursor(context).await? {
        Ok(after) => after,
        Err(error) => {
            record.lifecycle = "failed";
            notes.insert("coordination_error".to_owned(), json!(error.to_string()));
            return Ok(());
        }
    };
    let moved = after != before;
    record.cursor_moved = Some(moved);
    if !moved {
        return Ok(());
    }
    let before = json!({"event_cursor": before});
    match super::swarm_control::lifecycle_after(context.clone(), &before).await {
        Ok(AfterExecution::Settled) => record.lifecycle = "settled",
        Ok(AfterExecution::Notified(warnings)) => {
            record.lifecycle = "notified";
            record.warnings = warnings.len();
            if !warnings.is_empty() {
                notes.insert("notification_warnings".to_owned(), json!(warnings));
            }
        }
        Ok(AfterExecution::Unchanged) => record.lifecycle = "unchanged",
        Err(error) => {
            record.lifecycle = "failed";
            notes.insert("coordination_error".to_owned(), json!(error.to_string()));
        }
    }
    Ok(())
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The op's `tracing` record: ids, kinds, counts and sizes only. A
/// read-only op records at DEBUG (members poll them); any other at INFO.
fn trace(record: &OpRecord, result: &Result<ToolResult, DomainError>, elapsed: Duration) {
    let (outcome, result_bytes) = match result {
        Ok(result) if !result.is_error => ("ok", result.content.len()),
        Ok(result) => ("refused", result.content.len()),
        Err(_) => ("failed", 0),
    };
    let OpRecord {
        op,
        gate,
        cursor_moved,
        lifecycle,
        warnings,
        name_form,
        ..
    } = *record;
    let duration_us = micros(elapsed);
    // Left out of the line where no renamed name applies.
    let name_form = name_form.map(NameForm::as_str);
    match record.read_only {
        true => tracing::debug!(
            target: TELEMETRY_TARGET, op, gate, outcome, ?cursor_moved, lifecycle, warnings,
            name_form, result_bytes, duration_us, "swarm structured op"
        ),
        false => tracing::info!(
            target: TELEMETRY_TARGET, op, gate, outcome, ?cursor_moved, lifecycle, warnings,
            name_form, result_bytes, duration_us, "swarm structured op"
        ),
    }
}

#[cfg(test)]
#[path = "swarm_board_ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_board_ops_table_tests.rs"]
mod table_tests;
