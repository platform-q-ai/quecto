//! The read models of the board dispatch (#2277, #1969): `summary`,
//! `events` and `tasks`, and the summaries `create`, `_join` and
//! `_bootstrap` answer with, their Python signatures and their serving,
//! and the summary rendered in Python's key order. Every argument reaches
//! the use case as the JSON value passed (Python type-checks a cursor, an
//! offset or a limit at run time). A record names no task or message; its
//! decision says which branch the op took, and its detail (#2277 review
//! M2) the owners it read, the page it answered, whether `events` has
//! more, whether a cursor moved or an owner turned idle defeated the
//! summary's fast path, and whether `_bootstrap` wrote the placeholder:
//! counts and flags only. A `create` whose summary refuses after the run
//! committed answers that refusal, recorded as committed, and so do
//! `_bootstrap` and `_join` refused after the placeholder or the join
//! committed (#2277 final review L2).
use serde_json::{Map, Value};

use super::{Parameter, Served, done, member_row, required, take};
use crate::application::swarm::dto::{
    BootstrapMemberRequest, CreateBranch, CreateRunRequest, CreatedRun, EventPage, FullSummary,
    JoinRunRequest, Joined, LaunchIdentity, ListTasksRequest, ReadRunEventsRequest,
    ReadRunSummaryRequest, RunSummary, SummaryScan, TaskPage,
};
use crate::application::swarm::use_cases::{
    BootstrapMember, CreateRun, JoinMember, ListTasks, ReadRunEvents, ReadRunSummary,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `summary(since=None)`.
pub(super) const SUMMARY: [Parameter; 1] = [Parameter {
    name: "since",
    default: Some(|| Value::Null),
}];
/// `events(after=0, limit=25)`.
pub(super) const EVENTS: [Parameter; 2] = [
    Parameter {
        name: "after",
        default: Some(|| Value::from(0)),
    },
    Parameter {
        name: "limit",
        default: Some(|| Value::from(25)),
    },
];
/// `tasks(offset=0, limit=50)`.
pub(super) const TASKS: [Parameter; 2] = [
    Parameter {
        name: "offset",
        default: Some(|| Value::from(0)),
    },
    Parameter {
        name: "limit",
        default: Some(|| Value::from(50)),
    },
];
/// `create(goal, constraints, criteria, member_limit, deadline)`, and
/// the test-only `create_run`.
pub(super) const CREATE: [Parameter; 5] = [
    required("goal"),
    required("constraints"),
    required("criteria"),
    required("member_limit"),
    required("deadline"),
];
/// `_bootstrap(pid, started, socket, reservation=None)`, and the
/// test-only `bootstrap_join`.
pub(super) const BOOTSTRAP: [Parameter; 4] = [
    required("pid"),
    required("started"),
    required("socket"),
    Parameter {
        name: "reservation",
        default: Some(|| Value::Null),
    },
];
/// `_join(reservation, pid, started, socket)`.
pub(super) const JOIN: [Parameter; 4] = [
    required("reservation"),
    required("pid"),
    required("started"),
    required("socket"),
];

/// `value` answered with `decision`.
fn answered(value: Value, decision: &'static str) -> Served {
    let mut served = done(decision);
    served.value = value;
    served
}

/// The summary, or `{unchanged, event_cursor, status,
/// next_liveness_check_at}` for a cursor that is the board's.
pub(super) fn summary(
    read_run_summary: &ReadRunSummary,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [since] = take(arguments)?;
    let summary = read_run_summary.execute(ReadRunSummaryRequest {
        actor: actor.to_owned(),
        since,
    })?;
    let (decision, scan, page_size) = match &summary {
        RunSummary::Unchanged { scan, .. } => ("unchanged", *scan, None),
        RunSummary::Full(full) => ("full", full.scan, Some(count(full.tasks.len()))),
    };
    let mut served = answered(rendered(summary), decision);
    served.cursor_moved = scan.cursor_moved;
    served.detail = scanned(scan, page_size);
    Ok(served)
}

/// A length as a telemetry count.
fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// The detail of a summary that read as `scan`, holding `page_size` tasks
/// (`None` for the fast path's answer, which holds none).
fn scanned(scan: SummaryScan, page_size: Option<u64>) -> BoardOpDetail {
    BoardOpDetail {
        owners_scanned: Some(scan.owners_scanned),
        page_size,
        fast_path_defeated: scan.fast_path_defeated,
        ..BoardOpDetail::NONE
    }
}

/// `{events, cursor, has_more}`, each event as `dict(row)`.
pub(super) fn events(
    read_run_events: &ReadRunEvents,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [after, limit] = take(arguments)?;
    let EventPage {
        events,
        cursor,
        has_more,
    } = read_run_events.execute(ReadRunEventsRequest {
        actor: actor.to_owned(),
        after,
        limit,
    })?;
    let mut page = Map::new();
    let page_size = count(events.len());
    let events = events.into_iter().map(|event| event.into_value()).collect();
    page.insert("events".to_owned(), Value::Array(events));
    page.insert("cursor".to_owned(), Value::from(cursor));
    page.insert("has_more".to_owned(), Value::Bool(has_more));
    let mut served = answered(Value::Object(page), "read");
    served.detail = BoardOpDetail {
        page_size: Some(page_size),
        has_more: Some(has_more),
        ..BoardOpDetail::NONE
    };
    Ok(served)
}

/// The page of tasks, each with its owner's liveness.
pub(super) fn tasks(
    list_tasks: &ListTasks,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [offset, limit] = take(arguments)?;
    let TaskPage {
        tasks,
        owners_scanned,
    } = list_tasks.execute(ListTasksRequest {
        actor: actor.to_owned(),
        offset,
        limit,
    })?;
    let page_size = count(tasks.len());
    let page = tasks.into_iter().map(|task| task.into_value()).collect();
    let mut served = answered(Value::Array(page), "read");
    served.detail = BoardOpDetail {
        owners_scanned: Some(owners_scanned),
        page_size: Some(page_size),
        ..BoardOpDetail::NONE
    };
    Ok(served)
}

/// The run `create` made (or took over), before its summary.
pub(super) fn created(
    create_run: &CreateRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<CreatedRun, BoardError> {
    let [goal, constraints, criteria, member_limit, deadline] = take(arguments)?;
    create_run.execute(CreateRunRequest {
        member: member.to_owned(),
        goal,
        constraints,
        criteria,
        member_limit,
        deadline,
    })
}

/// `create`'s decision.
pub(super) fn branch(created: &CreatedRun) -> &'static str {
    match created.branch {
        CreateBranch::Fresh => "fresh",
        CreateBranch::OverSetup => "over_setup",
    }
}

/// The creator's summary, or its refusal: the run is created either way.
pub(super) fn create(
    create_run: &CreateRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let created = created(create_run, member, arguments)?;
    let decision = branch(&created);
    // Fresh or over the placeholder, the run now runs (#2390).
    Ok(Served {
        controls_run: true,
        ..summarised(created.summary, decision, BoardOpDetail::NONE)
    })
}

/// The summary answered with `decision` and `detail`, or the refusal met
/// after the op's writes committed: the call answers that refusal, and
/// its record says what the op decided before it.
fn summarised(
    summary: Result<RunSummary, BoardError>,
    decision: &'static str,
    detail: BoardOpDetail,
) -> Served {
    match summary {
        Ok(summary) => Served {
            detail,
            ..answered(rendered(summary), decision)
        },
        Err(refusal) => Served {
            refused: Some(refusal),
            controls_run: false,
            detail,
            ..done(decision)
        },
    }
}

fn joined(joined: &Joined) -> &'static str {
    match joined {
        Joined::Admitted => "admitted",
        Joined::AlreadyLive { .. } => "already_live",
        Joined::Reactivated => "reactivated",
    }
}

/// The coordinator's summary, or the refusal met after the placeholder
/// or the join committed, recorded with the join's branch (`placeholder`
/// when it took none) and whether the placeholder was written.
pub(super) fn bootstrap(
    bootstrap_member: &BootstrapMember,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [pid, started, socket, reservation] = take(arguments)?;
    let answer = bootstrap_member.execute(BootstrapMemberRequest {
        member: member.to_owned(),
        pid,
        started,
        socket,
        reservation,
    })?;
    let decision = answer.joined.as_ref().map_or("placeholder", joined);
    let detail = BoardOpDetail {
        placeholder_created: Some(answer.created),
        ..BoardOpDetail::NONE
    };
    Ok(summarised(answer.summary, decision, detail))
}

/// The coordinator's summary, or the refusal met after the join's
/// writes committed, recorded with its branch.
pub(super) fn join(
    join_member: &JoinMember,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [reservation, pid, started, socket] = take(arguments)?;
    let answer = join_member.execute(JoinRunRequest {
        member: member.to_owned(),
        reservation,
        launch: LaunchIdentity { pid, started },
        socket,
    })?;
    let decision = joined(&answer.joined);
    Ok(summarised(answer.summary, decision, BoardOpDetail::NONE))
}

fn float(value: Option<f64>) -> Value {
    value.map_or(Value::Null, Value::from)
}

/// The summary as Python's dict: the run's columns, then the keys
/// `summary` adds, in its order.
fn rendered(summary: RunSummary) -> Value {
    let mut answer = Map::new();
    match summary {
        RunSummary::Unchanged {
            event_cursor,
            status,
            next_liveness_check_at,
            scan: _,
        } => {
            answer.insert("unchanged".to_owned(), Value::Bool(true));
            answer.insert("event_cursor".to_owned(), Value::from(event_cursor));
            answer.insert("status".to_owned(), status);
            answer.insert(
                "next_liveness_check_at".to_owned(),
                float(next_liveness_check_at),
            );
        }
        RunSummary::Full(full) => full_summary(*full, &mut answer),
    }
    Value::Object(answer)
}

fn full_summary(full: FullSummary, answer: &mut Map<String, Value>) {
    answer.extend(full.run.columns);
    let mut put = |key: &str, value: Value| {
        answer.insert(key.to_owned(), value);
    };
    put("next_liveness_check_at", float(full.next_liveness_check_at));
    put(
        "members",
        Value::Array(full.members.into_iter().map(member_row).collect()),
    );
    put("usage", Value::from(full.usage));
    put("task_count", Value::from(full.task_count));
    let tasks = full.tasks.into_iter().map(|task| task.into_value());
    put("tasks", Value::Array(tasks.collect()));
    put("file_count", Value::from(full.file_count));
    let files = full.files.into_iter().map(|file| file.into_value());
    put("files", Value::Array(files.collect()));
    let evidence = full.evidence.into_iter().map(|row| row.into_value());
    put("evidence", Value::Array(evidence.collect()));
    put("control_generation", Value::from(full.control_generation));
    put("event_cursor", Value::from(full.event_cursor));
    let counts = full.counts;
    let mut by_status = Map::new();
    for (key, count) in [
        ("ready", counts.ready),
        ("claimed", counts.claimed),
        ("blocked", counts.blocked),
        ("submitted", counts.submitted),
        ("completed", counts.completed),
        (
            "members_without_claim",
            counts.members.members_without_claim,
        ),
        ("members_dead", counts.members.members_dead),
    ] {
        by_status.insert(key.to_owned(), Value::from(count));
    }
    put("counts", Value::Object(by_status));
}
