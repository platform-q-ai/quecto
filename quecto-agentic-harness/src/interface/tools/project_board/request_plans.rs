//! A plan's ops as a request names them (#2481 M4b): `propose_plan` and
//! `edit_plan`, with a plan's task items and team nested as `show` gives
//! them. Each nested value is read by an allowlist of its keys, and a
//! fault names its path (`items/3/criteria/0`), which a record holds as
//! its schema field (`items`).
use serde_json::{Map, Value};

use super::{Read, misfit, text, timestamp};
use crate::application::project_board::dto::{PlanEdit, PlanProposal};
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::entities::plan::{MAX_PLAN_VERSION, PlanItem};
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::FaultKind;
use crate::domain::project_board::value_objects::vocabulary::Effort;

use super::RequestError;

/// A new draft: its approach, its task items, and its team, if any.
pub(super) fn propose_plan(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let present = "a required field is present";
    Ok(BoardRequest::ProposePlan(PlanProposal {
        task: read.task()?,
        approach: read.text("approach")?.expect(present),
        items: items(read.value("items").expect(present))?,
        team: read.value("team").map(team).transpose()?.flatten(),
    }))
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
        true => Ok(BoardRequest::EditPlan(edit)),
        // No field to change was given.
        false => Err(RequestError::of(
            "op",
            "edit_plan sets nothing: name a field to change",
            FaultKind::Missing,
        )),
    }
}

fn version(value: &Value) -> Result<u32, RequestError> {
    value
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
        .filter(|version| (1..=MAX_PLAN_VERSION).contains(version))
        .ok_or_else(|| {
            let problem = format!("must be a plan version from 1 to {MAX_PLAN_VERSION}");
            RequestError::of("version", problem, misfit(value.is_number()))
        })
}

/// The object at `at`, taking only `keys` (an allowlist) and holding each
/// of the first `required` of them; `what` it must be.
fn object<'v>(
    at: &str,
    value: &'v Value,
    keys: &[&str],
    required: usize,
    what: &str,
) -> Result<&'v Map<String, Value>, RequestError> {
    let object = value
        .as_object()
        .ok_or_else(|| RequestError::of(at, format!("must be {what}"), FaultKind::WrongType))?;
    let strays: Vec<&String> = object
        .keys()
        .filter(|key| !keys.contains(&key.as_str()))
        .collect();
    if let Some(stray) = strays.first() {
        let problem = format!("{what} takes no such field");
        return Err(RequestError::stray(
            &format!("{at}/{stray}"),
            problem,
            strays.len(),
        ));
    }
    match keys[..required]
        .iter()
        .find(|key| !object.contains_key(**key))
    {
        Some(missing) => Err(RequestError::of(
            &format!("{at}/{missing}"),
            "is required",
            FaultKind::Missing,
        )),
        None => Ok(object),
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

/// A whole number at `at` that `T` holds.
fn number<T: TryFrom<u64>>(at: &str, value: &Value, problem: &str) -> Result<T, RequestError> {
    value
        .as_u64()
        .and_then(|number| T::try_from(number).ok())
        .ok_or_else(|| RequestError::of(at, problem, misfit(value.is_number())))
}

/// The task items, each `{"id", "title"}` with its `criteria` and
/// `depends_on` (task item ids), both empty when not given.
fn items(value: &Value) -> Result<Vec<PlanItem>, RequestError> {
    let keys = ["id", "title", "criteria", "depends_on"];
    list("items", Some(value), "a list of task items", |at, item| {
        let item = object(at, item, &keys, 2, "a task item")?;
        let field = |key: &str| format!("{at}/{key}");
        Ok(PlanItem {
            id: id(&field("id"), &item["id"], "a task item id")?,
            title: text(&field("title"), &item["title"])?,
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
    let team = object("team", value, &keys, 1, "a team")?;
    let role_keys = ["name", "model", "effort", "members", "turns"];
    let roles = list(
        "team/roles",
        Some(&team["roles"]),
        "a list of roles",
        |at, role| {
            let role = object(at, role, &role_keys, 4, "a role")?;
            let field = |key: &str| format!("{at}/{key}");
            let efforts: Vec<&str> = Effort::ALL.iter().map(|effort| effort.as_str()).collect();
            let effort = role["effort"]
                .as_str()
                .and_then(Effort::parse)
                .ok_or_else(|| {
                    let problem = format!("must be one of {}", efforts.join(", "));
                    RequestError::of(
                        &field("effort"),
                        problem,
                        misfit(role["effort"].is_string()),
                    )
                })?;
            let turns = given(role, "turns")
                .map(|turns| number(&field("turns"), turns, "must be a number of turns"))
                .transpose()?;
            Ok(Role {
                name: id(&field("name"), &role["name"], "a role name")?,
                model: text(&field("model"), &role["model"])?,
                effort,
                limits: RoleLimits {
                    members: number(
                        &field("members"),
                        &role["members"],
                        "must be a number of members",
                    )?,
                    turns,
                },
            })
        },
    )?;
    let budget = given(team, "budget_tokens")
        .map(|tokens| number("team/budget_tokens", tokens, "must be a number of tokens"))
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
