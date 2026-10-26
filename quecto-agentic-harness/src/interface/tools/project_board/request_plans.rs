//! A plan's ops as a request names them (#2481 M4b): `propose_plan` and
//! `edit_plan`, with a plan's task items and team nested as `show` gives
//! them, and its approach entering as its front end's policy resolves it
//! by whether the request gives a `handoff` (#2705 T3). Each nested value is read by an allowlist of its keys, every key
//! by checked access, and a fault names its path (`items/3/criteria/0`,
//! `team/roles/0/members`), which a record holds as its schema field
//! (`items`, `team`). A number's range is checked here against the
//! domain's own limits, so its refusal names the field as the request
//! spells it; the domain's checks stay as the backstop. A plan's text as
//! a whole is the domain's alone to judge, once normalised (`invalid_plan`,
//! field `text`).
use serde_json::{Map, Value};

use super::{Read, misfit, text};
use crate::application::project_board::dto::{
    PlanAcceptance, PlanDiscard, PlanEdit, PlanProposal, RunEnd, RunEndOutcome, TaskStart,
};
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::entities::plan::{MAX_PLAN_VERSION, PlanEntry, PlanItem};
use crate::domain::project_board::entities::run::MAX_RUN_NUMBER;
use crate::domain::project_board::entities::team::{
    MAX_DEADLINE_SECONDS, MAX_MEMBERS, MIN_DEADLINE_SECONDS, Role, RoleLimits, Team,
};
use crate::domain::project_board::services::run_outcomes::StopKind;
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::FaultKind;
use crate::domain::project_board::value_objects::vocabulary::Effort;

use super::RequestError;

/// A new draft: its approach, its task items, and its team, if any.
pub(super) fn propose_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    let handoff = read.text("handoff")?;
    Ok(BoardRequest::ProposePlan(PlanProposal {
        task: read.task()?,
        approach: read.text("approach")?.expect(present),
        entry: PlanEntry::of(read.policy, holds_handoff(handoff.as_deref())),
        handoff,
        items: items(read.value("items").expect(present))?,
        team: read.value("team").map(team).transpose()?.flatten(),
    }))
}

/// An edit of a draft: only the fields it gives change, so it names at
/// least one.
pub(super) fn edit_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let handoff = read.text("handoff")?;
    let edit = PlanEdit {
        task: read.task()?,
        version: version(read.value("version").expect("a required field is present"))?,
        approach: read.text("approach")?,
        entry: PlanEntry::of(read.policy, holds_handoff(handoff.as_deref())),
        handoff,
        items: read.value("items").map(items).transpose()?,
        team: read.value("team").map(team).transpose()?,
        expected: read.pin()?,
    };
    match edit.sets_something() {
        true => Ok(BoardRequest::EditPlan(edit)),
        // No field to change was given.
        false => Err(RequestError::of(
            "op",
            "edit_plan sets nothing: name a field to change",
            FaultKind::Missing,
        )),
    }
}

/// A draft accepted, pinned to the draft as it was read (#2481 M4c): a
/// pin, `expected_rev` (or the deprecated `expected_updated`), is required.
pub(super) fn accept_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    Ok(BoardRequest::AcceptPlan(PlanAcceptance {
        task: read.task()?,
        version: version(read.value("version").expect(present))?,
        expected: read
            .pin()?
            .ok_or_else(|| RequestError::of("expected_rev", "is required", FaultKind::Missing))?,
        expected_accepted: accepted(read.value("expected_accepted").expect(present))?,
    }))
}

/// A draft discarded, pinned to the draft as it was read when the
/// request gives a pin.
pub(super) fn discard_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    Ok(BoardRequest::DiscardPlan(PlanDiscard {
        task: read.task()?,
        version: version(read.value("version").expect("a required field is present"))?,
        expected: read.pin()?,
    }))
}

/// A run ended by hand (S4c): its outcome one of [`RunEndOutcome::ALL`],
/// pinned to the task as it was read when the request gives
/// a pin.
pub(super) fn end_run(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    let words: Vec<&str> = RunEndOutcome::ALL.iter().map(|end| end.as_str()).collect();
    let stops: Vec<&str> = StopKind::ALL.iter().map(|stop| stop.as_str()).collect();
    let problem = format!("must be a run number from 1 to {MAX_RUN_NUMBER}");
    let outcome = read
        .word("outcome", RunEndOutcome::parse, &words)?
        .expect(present);
    let stop = read.word("stop", StopKind::parse, &stops)?;
    // How a stop stopped is a stop's alone (an allowlist, #2705 T5).
    match (outcome, stop) {
        (_, None) | (RunEndOutcome::Stopped, Some(_)) => {}
        (RunEndOutcome::Succeeded | RunEndOutcome::Failed | RunEndOutcome::Lost, Some(_)) => {
            return Err(RequestError::of(
                "stop",
                "is taken only with outcome stopped",
                FaultKind::InvalidValue,
            ));
        }
    }
    Ok(BoardRequest::EndRun(RunEnd {
        task: read.task()?,
        run: number(
            "run",
            read.value("run").expect(present),
            1..=MAX_RUN_NUMBER,
            &problem,
        )?,
        outcome,
        expected: read.pin()?,
        stop,
    }))
}

/// Whether `handoff` gives a handoff once normalised as the board keeps
/// it: `""` (or blank lines alone) is none, so it caps no approach.
fn holds_handoff(handoff: Option<&str>) -> bool {
    handoff.is_some_and(|text| !board_text::text(text).is_empty())
}

/// A plan's version, from 1, as `version` names it.
pub(super) fn version(value: &Value) -> Result<u32, RequestError> {
    version_at("version", value)
}

/// A plan's version, from 1, as field `name` names it.
fn version_at(name: &str, value: &Value) -> Result<u32, RequestError> {
    let problem = format!("must be a plan version from 1 to {MAX_PLAN_VERSION}");
    number(name, value, 1..=MAX_PLAN_VERSION, &problem)
}

/// A task started on the accepted plan version its caller confirmed
/// (S4c).
pub(super) fn start(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    Ok(BoardRequest::Start(TaskStart {
        task: read.task()?,
        plan_version: version_at("plan_version", read.value("plan_version").expect(present))?,
    }))
}

/// The version a task was seen naming as accepted, or none for `null`.
fn accepted(value: &Value) -> Result<Option<u32>, RequestError> {
    if value.is_null() {
        return Ok(None);
    }
    let problem =
        format!("must be the accepted plan's version, 1 to {MAX_PLAN_VERSION}, or null for none");
    number("expected_accepted", value, 1..=MAX_PLAN_VERSION, &problem).map(Some)
}

/// The object at `at`, taking only `keys` (an allowlist); `what` it must
/// be. Its required keys are read by [`required`].
fn object<'v>(
    at: &str,
    value: &'v Value,
    keys: &[&str],
    what: &str,
) -> Result<&'v Map<String, Value>, RequestError> {
    let object = value
        .as_object()
        .ok_or_else(|| RequestError::of(at, format!("must be {what}"), FaultKind::WrongType))?;
    let strays: Vec<&String> = object
        .keys()
        .filter(|key| !keys.contains(&key.as_str()))
        .collect();
    match strays.first() {
        None => Ok(object),
        Some(stray) => {
            let problem = format!("{what} takes no such field");
            Err(RequestError::stray(
                &format!("{at}/{stray}"),
                problem,
                strays.len(),
            ))
        }
    }
}

/// The value at `key` of `object`, unless it is absent or `null`.
fn given<'v>(object: &'v Map<String, Value>, key: &str) -> Option<&'v Value> {
    object.get(key).filter(|value| !value.is_null())
}

fn id(at: &str, value: &Value, what: &str) -> Result<Slug, RequestError> {
    let text = value
        .as_str()
        .ok_or_else(|| RequestError::of(at, format!("must be {what}"), FaultKind::WrongType))?;
    Slug::parse(text).map_err(|error| RequestError::of(at, error.problem, FaultKind::InvalidValue))
}

/// A list at `at` (none, absent or `null`, is empty), each entry read by
/// `entry` at its own path.
fn list<T>(
    at: &str,
    value: Option<&Value>,
    what: &str,
    entry: impl Fn(&str, &Value) -> Result<T, RequestError>,
) -> Result<Vec<T>, RequestError> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| RequestError::of(at, format!("must be {what}"), FaultKind::WrongType))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, value)| entry(&format!("{at}/{index}"), value))
        .collect()
}

/// A whole number at `at` within `range`: of the wrong type, or out of
/// range, refused with `problem`.
fn number<T>(
    at: &str,
    value: &Value,
    range: std::ops::RangeInclusive<T>,
    problem: &str,
) -> Result<T, RequestError>
where
    T: TryFrom<u64> + PartialOrd,
{
    value
        .as_u64()
        .and_then(|number| T::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| RequestError::of(at, problem, misfit(value.is_number())))
}

/// The value of the required `key` of `object`, at `at`: refused, never
/// a panic, when it is not there.
fn required<'v>(
    at: &str,
    object: &'v Map<String, Value>,
    key: &str,
) -> Result<&'v Value, RequestError> {
    object
        .get(key)
        .ok_or_else(|| RequestError::of(&format!("{at}/{key}"), "is required", FaultKind::Missing))
}

/// The task items, each `{"id", "title"}` with its `criteria` and
/// `depends_on` (task item ids), both empty when not given.
fn items(value: &Value) -> Result<Vec<PlanItem>, RequestError> {
    let keys = ["id", "title", "criteria", "depends_on"];
    list("items", Some(value), "a list of task items", |at, item| {
        let item = object(at, item, &keys, "a task item")?;
        let field = |key: &str| format!("{at}/{key}");
        Ok(PlanItem {
            id: id(&field("id"), required(at, item, "id")?, "a task item id")?,
            title: text(&field("title"), required(at, item, "title")?)?,
            criteria: list(
                &field("criteria"),
                given(item, "criteria"),
                "a list of lines",
                text,
            )?,
            depends_on: list(
                &field("depends_on"),
                given(item, "depends_on"),
                "a list of task item ids",
                |at, dependency| id(at, dependency, "a task item id"),
            )?,
        })
    })
}

/// The team, or none for `null`.
fn team(value: &Value) -> Result<Option<Team>, RequestError> {
    if value.is_null() {
        return Ok(None);
    }
    let keys = ["roles", "deadline_seconds", "image"];
    let team = object("team", value, &keys, "a team")?;
    let role_keys = ["name", "model", "effort", "members", "turns"];
    let roles = required("team", team, "roles")?;
    let roles = list("team/roles", Some(roles), "a list of roles", |at, role| {
        let role = object(at, role, &role_keys, "a role")?;
        let field = |key: &str| format!("{at}/{key}");
        let name = id(&field("name"), required(at, role, "name")?, "a role name")?;
        let model = text(&field("model"), required(at, role, "model")?)?;
        let efforts: Vec<&str> = Effort::ALL.iter().map(|effort| effort.as_str()).collect();
        let effort = required(at, role, "effort")?;
        let effort = effort.as_str().and_then(Effort::parse).ok_or_else(|| {
            let problem = format!("must be one of {}", efforts.join(", "));
            RequestError::of(&field("effort"), problem, misfit(effort.is_string()))
        })?;
        let members = required(at, role, "members")?;
        let problem = format!("must be a number of members from 1 to {MAX_MEMBERS}");
        let members = number(&field("members"), members, 1..=MAX_MEMBERS, &problem)?;
        let problem = "must be a number of turns, at least 1";
        let turns = given(role, "turns")
            .map(|turns| number(&field("turns"), turns, 1..=u32::MAX, problem))
            .transpose()?;
        Ok(Role {
            name,
            model,
            effort,
            limits: RoleLimits { members, turns },
        })
    })?;
    let deadline_seconds = given(team, "deadline_seconds")
        .map(|seconds| {
            let problem = format!(
                "must be a number of seconds from {MIN_DEADLINE_SECONDS} to {MAX_DEADLINE_SECONDS}"
            );
            number(
                "team/deadline_seconds",
                seconds,
                MIN_DEADLINE_SECONDS..=MAX_DEADLINE_SECONDS,
                &problem,
            )
        })
        .transpose()?;
    Ok(Some(Team {
        roles,
        deadline_seconds,
        image: given(team, "image")
            .map(|image| text("team/image", image))
            .transpose()?,
    }))
}

#[cfg(test)]
#[path = "request_plans_tests.rs"]
mod tests;
