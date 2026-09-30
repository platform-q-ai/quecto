//! A board call's records (#2303), written once it is finished, and the
//! records of a structured op refused before any call (#2279): the tool
//! refuses argument text no `Value` holds, and an op while the run is not
//! running, before it calls the dispatcher, and each refusal is still
//! recorded as the op's own. Also the Python signature each method binds by,
//! which the structured ops' table is checked against (#2279).
use std::time::Duration;

use serde_json::Value;

use super::super::swarm_board_telemetry::{
    Caller, Finished, Served, TELEMETRY_TARGET, actor_ref, observation, trace,
};
use super::BindingFaults;
use super::method::Method;
use super::{Level, SwarmBoardHandles};
use crate::application::swarm::dto::CallMeasure;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardRole, RefusalKind};

/// Whom a board call is made for (#2279 S15 final review): the calling
/// member, whose role in the run a member-facing op records, or the
/// harness itself, reading the board on the member's behalf (the post-call
/// lifecycle's summary, a settlement's reconcile), recorded as `host` so
/// it is never counted against the member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOrigin {
    Member,
    Harness,
    /// The harness's run watch (#2338): its `_event_cursor` polls and its
    /// `_snapshot`s, recorded as `host`; unchanged polls are aggregated
    /// ([`WatchPolls`]).
    Watch,
}

/// The run watch's polls not yet written (#2338), held with the handles of
/// one board file while the event log is on.
#[derive(Debug)]
pub struct WatchPolls {
    tally: std::sync::Mutex<crate::domain::swarm::watch_polls::PollTally>,
    origin: std::time::Instant,
}

impl Default for WatchPolls {
    fn default() -> Self {
        Self {
            tally: std::sync::Mutex::new(crate::domain::swarm::watch_polls::PollTally::default()),
            origin: std::time::Instant::now(),
        }
    }
}

/// Writes the run watch's polls `handles` holds, if any (#2338): the watch
/// ended, the run is summarised, or the handles are replaced.
pub fn flush_watch_polls(handles: &SwarmBoardHandles) {
    if let Some(telemetry) = &handles.telemetry {
        let _ = (&telemetry.polls.tally, telemetry.polls.origin);
    }
}

/// The role a call's records carry (#2303; #2282 final review): `host`
/// for the harness's own calls, whatever method they name; for a member's
/// call, its method's ([`Method::role`]: `None` when it is read from the
/// run the op found); and `None` for a member naming no board method,
/// whose caller is unproven and which reads no run. `host` is the
/// harness's alone (docs/swarm.md), so an unknown name never maps to it.
pub(super) fn recorded_role(origin: CallOrigin, known: Option<Method>) -> Option<BoardRole> {
    let role = match (origin, known) {
        (CallOrigin::Harness | CallOrigin::Watch, _) => Some(BoardRole::Host),
        (CallOrigin::Member, Some(known)) => known.role(),
        (CallOrigin::Member, None) => None,
    };
    debug_assert!(
        known.is_some()
            || matches!(origin, CallOrigin::Harness | CallOrigin::Watch)
            || role.is_none(),
        "a member's call naming no board method records no role"
    );
    role
}

/// The `tracing` record of `finished` and, while the event log is on, its
/// `swarm_op`, by the caller's kept or fresh ref (`caller` says whether the
/// board accepted it as a member).
pub(super) fn record(
    handles: &SwarmBoardHandles,
    finished: &Finished<'_>,
    caller: Caller,
    served: Option<&Served>,
    measure: Option<CallMeasure>,
) {
    let actor = handles
        .telemetry
        .as_ref()
        .map(|telemetry| telemetry.actors.of(finished.member, caller));
    if !finished.arguments.is_empty() {
        let actor = actor.clone().unwrap_or_else(|| actor_ref(finished.member));
        trace_arguments(finished, &actor);
    }
    match (&handles.telemetry, actor) {
        (Some(telemetry), Some(actor)) => {
            trace(finished, measure.as_ref(), Some(&actor));
            let observation = observation(finished, actor, caller, served, measure);
            telemetry.log.record(observation);
        }
        (Some(_), None) | (None, _) => trace(finished, None, None),
    }
}

/// The `tracing` record of a binding refusal's faults (#2341), on the
/// board's target: the op and the schema names only, each list
/// comma-joined (`missing_args=token,reason`), and the caller's redacted
/// `actor` ref, which matches it to the call's own record (#2346 review).
fn trace_arguments(finished: &Finished<'_>, actor: &Redacted) {
    let faults = finished.arguments.record();
    let unexpected = faults.unexpected_args.as_ref();
    let known = unexpected.map_or(&[][..], |unexpected| &unexpected.known[..]);
    let wrong: Vec<String> = faults
        .wrong_type_args
        .iter()
        .map(|wrong| format!("{}:{}", wrong.arg, wrong.expected))
        .collect();
    tracing::info!(
        target: TELEMETRY_TARGET,
        op = finished.op,
        member = actor.as_str(),
        missing_args = %faults.missing_args.join(","),
        unexpected_args = unexpected.map_or(0, |unexpected| unexpected.count),
        unexpected_known = %known.join(","),
        wrong_type_args = %wrong.join(","),
        unreadable_args = %faults.unreadable_args.join(","),
        "swarm board call arguments"
    );
}

/// Records `method`, called by `member`, as refused with `kind` before it
/// reached the board, `elapsed` after the op began: the same records a
/// refused call leaves, with nothing measured (no transaction began) and
/// the caller unproven. `arguments` is what the refusal found wrong in the
/// member's text (#2341: a value the board cannot hold), kept only for an
/// `invalid` or `calling` refusal.
pub fn refused(
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    kind: RefusalKind,
    arguments: BindingFaults,
    elapsed: Duration,
) {
    let known = Method::parse(method);
    let outcome = Err(kind);
    let finished = Finished {
        op: known.map_or("unknown", Method::name),
        level: known.map_or(Level::Mutation, Method::level),
        role: recorded_role(CallOrigin::Member, known),
        member,
        arguments: super::binding::recorded(&outcome, Some(arguments)),
        outcome,
        elapsed,
    };
    record(handles, &finished, Caller::Unproven, None, None);
}

/// The Python signature `method` binds by, each parameter's name and its
/// default (`None` for a required one); `None` for a name the dispatcher
/// does not serve.
pub fn signature(method: &str) -> Option<Vec<(&'static str, Option<Value>)>> {
    let parameters = Method::parse(method)?.parameters();
    Some(
        parameters
            .iter()
            .map(|parameter| (parameter.name, parameter.default.map(|default| default())))
            .collect(),
    )
}
