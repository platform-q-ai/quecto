//! A plan's ops as a request names them (#2481 M4b): `propose_plan` and
//! `edit_plan`, with a plan's task items and team nested as `show` gives
//! them. Each nested value is read by an allowlist of its keys, every key
//! by checked access, and a fault names its path (`items/3/criteria/0`,
//! `team/roles/0/members`), which a record holds as its schema field
//! (`items`, `team`). Ranges are checked here against the domain's own
//! limits, so a refusal names the field as the request spells it; the
//! domain's checks stay as the backstop.
use serde_json::{Map, Value};

use super::{Read, misfit, text, timestamp};
use crate::application::project_board::dto::{PlanEdit, PlanProposal};
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::entities::plan::{
    MAX_PLAN_TEXT_BYTES, MAX_PLAN_VERSION, PlanItem, text_bytes,
};
use crate::domain::project_board::entities::task_parts::MAX_BUDGET_TOKENS;
use crate::domain::project_board::entities::team::{Budget, MAX_MEMBERS, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::FaultKind;
use crate::domain::project_board::value_objects::vocabulary::Effort;

use super::RequestError;

/// A new draft: its approach, its task items, and its team, if any.
pub(super) fn propose_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    let proposal = PlanProposal {
        task: read.task()?,
        approach: read.text("approach")?.expect(present),
        items: items(read.value("items").expect(present))?,
        team: read.value("team").map(team).transpose()?.flatten(),
    };
    fits(
        Some(&proposal.approach),
        Some(&proposal.items),
        proposal.team.as_ref(),
    )?;
    Ok(BoardRequest::ProposePlan(proposal))
}

/// An edit of a draft: only the fields it gives change, so it names at
/// least one.
pub(super) fn edit_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let edit = PlanEdit {
        task: read.task()?,
        version: version(read.value("version").expect("a required field is present"))?,
        approach: read.text("approach")?,
        items: read.value("items").map(items).transpose()?,
        team: read.value("team").map(team).transpose()?,
        expected_updated: read.timestamp("expected_updated")?,
    };
    match edit.sets_something() {
        true => {}
        // No field to change was given.
        false => {
            return Err(RequestError::of(
                "op",
                "edit_plan sets nothing: name a field to change",
                FaultKind::Missing,
            ));
        }
    }
    // The text it gives alone; with what the draft keeps, the domain's.
    let team = edit.team.as_ref().and_then(Option::as_ref);
    fits(edit.approach.as_deref(), edit.items.as_deref(), team)?;
    Ok(BoardRequest::EditPlan(edit))
}

/// The plan text given, as it enters the board (normalised), within the
/// most a plan holds.
fn fits(
    approach: Option<&str>,
    items: Option<&[PlanItem]>,
    team: Option<&Team>,
) -> Result<(), RequestError> {
    let approach = approach.map(board_text::text).unwrap_or_default();
    let items: Vec<PlanItem> = items
        .unwrap_or_default()
        .iter()
        .map(|item| PlanItem {
            title: board_text::line(&item.title),
            criteria: item
                .criteria
                .iter()
                .map(|line| board_text::line(line))
                .collect(),
            ..item.clone()
        })
        .collect();
    let bytes = text_bytes(&approach, &items, team);
    match bytes <= MAX_PLAN_TEXT_BYTES {
        true => Ok(()),
        false => Err(RequestError::of(
            "request",
            format!("holds {bytes} bytes of plan text; a plan holds at most {MAX_PLAN_TEXT_BYTES}"),
            FaultKind::TooLarge,
        )),
    }
}

/// A plan's version, from 1.
pub(super) fn version(value: &Value) -> Result<u32, RequestError> {
    number("version", value, 1..=MAX_PLAN_VERSION, "a plan version")
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

/// A whole number at `at` within `range`, `what` it counts: of the
/// wrong type, or out of range, refused naming both.
fn number<T>(
    at: &str,
    value: &Value,
    range: std::ops::RangeInclusive<T>,
    what: &str,
) -> Result<T, RequestError>
where
    T: TryFrom<u64> + PartialOrd + std::fmt::Display,
{
    value
        .as_u64()
        .and_then(|number| T::try_from(number).ok())
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            let (low, high) = (range.start(), range.end());
            let problem = format!("must be {what} from {low} to {high}");
            RequestError::of(at, problem, misfit(value.is_number()))
        })
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
    let keys = ["roles", "budget_tokens", "deadline", "image"];
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
        let members = number(
            &field("members"),
            members,
            1..=MAX_MEMBERS,
            "a number of members",
        )?;
        let turns = given(role, "turns")
            .map(|turns| number(&field("turns"), turns, 1..=u32::MAX, "a number of turns"))
            .transpose()?;
        Ok(Role {
            name,
            model,
            effort,
            limits: RoleLimits { members, turns },
        })
    })?;
    let budget = given(team, "budget_tokens")
        .map(|tokens| {
            let tokens_range = 1..=MAX_BUDGET_TOKENS;
            number(
                "team/budget_tokens",
                tokens,
                tokens_range,
                "a number of tokens",
            )
        })
        .transpose()?
        .map(|tokens| Budget { tokens });
    Ok(Some(Team {
        roles,
        budget,
        deadline: given(team, "deadline")
            .map(|deadline| timestamp("team/deadline", deadline))
            .transpose()?,
        image: given(team, "image")
            .map(|image| text("team/image", image))
            .transpose()?,
    }))
}

#[cfg(test)]
#[path = "request_plans_tests.rs"]
mod tests;
