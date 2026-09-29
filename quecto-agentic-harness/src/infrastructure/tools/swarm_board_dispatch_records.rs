//! A board call's records (#2303), written once it is finished, and the
//! records of a structured op refused before any call (#2279): the tool
//! refuses argument text no `Value` holds, and an op while the run is not
//! running, before it calls the dispatcher, and each refusal is still
//! recorded as the op's own. Also the Python signature each method binds by,
//! which the structured ops' table is checked against (#2279).
use std::time::Duration;

use serde_json::Value;

use super::super::swarm_board_telemetry::{Caller, Finished, Served, observation, trace};
use super::method::Method;
use super::{Level, SwarmBoardHandles};
use crate::application::swarm::dto::CallMeasure;
use crate::domain::swarm::{BoardRole, RefusalKind};

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
    match &handles.telemetry {
        Some(telemetry) => {
            let actor = telemetry.actors.of(finished.member, caller);
            trace(finished, measure.as_ref(), Some(&actor));
            let observation = observation(finished, actor, served, measure);
            telemetry.log.record(observation);
        }
        None => trace(finished, None, None),
    }
}

/// Records `method`, called by `member`, as refused with `kind` before it
/// reached the board, `elapsed` after the op began: the same records a
/// refused call leaves, with nothing measured (no transaction began) and
/// the caller unproven.
pub fn refused(
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    kind: RefusalKind,
    elapsed: Duration,
) {
    let known = Method::parse(method);
    let finished = Finished {
        op: known.map_or("unknown", Method::name),
        level: known.map_or(Level::Mutation, Method::level),
        role: known.map_or(Some(BoardRole::Host), Method::role),
        member,
        outcome: Err(kind),
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
