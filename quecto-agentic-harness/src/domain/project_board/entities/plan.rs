//! A plan: how one task will be achieved (#2481's artifact model), a
//! versioned artifact of its own. It holds the approach, the items that
//! seed the swarm board, and the team's shape. A [`Plan`] is only ever
//! built from fields that pass [`Plan::new`]; its versions are numbered
//! from 1 per task, a draft is edited, and an accepted version is never
//! edited again: a change is a new version.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::PlanState;
use super::claim::Identity;
use super::task_parts::{MAX_ITEMS, MAX_LINE_CHARS, MAX_PER_ITEM, lines};
use super::team::Team;
use std::collections::BTreeMap;

/// The most characters of a plan's approach (GitHub's issue body limit).
pub const MAX_APPROACH_CHARS: usize = 65_536;
/// The most versions a task's plan reaches.
pub const MAX_PLAN_VERSION: u32 = 999;
/// All of a plan's text together, in UTF-8 bytes: the approach, the
/// items' titles and criteria, the team's models and image. The field
/// limits alone would add up to megabytes; this budget keeps every plan
/// the domain admits inside the one file it is stored in, with room to
/// spare (its codec, M2, proves the fit).
pub const MAX_PLAN_TEXT_BYTES: usize = 320 * 1024;

/// One item the plan seeds the swarm board with, and how its completion
/// is judged. Where an item stands is a run's, not the plan's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub id: Slug,
    pub title: String,
    pub criteria: Vec<String>,
    /// Other items of the same plan; never a cycle.
    pub depends_on: Vec<Slug>,
}

/// A plan's content, unchecked: what [`Plan::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFields {
    /// The task it plans.
    pub task: Slug,
    /// From 1, per task.
    pub version: u32,
    pub state: PlanState,
    /// Markdown.
    pub approach: String,
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
    /// Who proposed this version.
    pub proposed_by: Identity,
    /// Who accepted it and when: set from its acceptance on (accepted,
    /// then superseded), never before.
    pub accepted: Option<(Identity, Timestamp)>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl PlanFields {
    /// The UTF-8 bytes of every free text in the plan: the approach, the
    /// items' titles and criteria, the team's models and image. Ids,
    /// words, numbers, times and identities are bounded by the field
    /// limits.
    pub fn text_bytes(&self) -> usize {
        let items: usize = self
            .items
            .iter()
            .map(|item| item.title.len() + item.criteria.iter().map(String::len).sum::<usize>())
            .sum();
        let team = self.team.as_ref().map_or(0, |team| {
            let models: usize = team.roles.iter().map(|role| role.model.len()).sum();
            models + team.image.as_ref().map_or(0, String::len)
        });
        self.approach.len() + items + team
    }
}

/// A plan whose fields keep every rule of its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan(PlanFields);

impl Plan {
    pub fn new(fields: PlanFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &PlanFields {
        &self.0
    }

    pub fn into_fields(self) -> PlanFields {
        self.0
    }

    /// The item named `id`, if the plan has one.
    pub fn item(&self, id: &Slug) -> Option<&PlanItem> {
        self.0.items.iter().find(|item| item.id == *id)
    }
}

impl PlanItem {
    /// Its own fields; dependencies on sibling items are the plan's check.
    pub fn validate(&self, field: &str) -> Result<(), SchemaError> {
        line(&format!("{field}/title"), &self.title, MAX_LINE_CHARS)?;
        lines(&format!("{field}/criteria"), &self.criteria, MAX_PER_ITEM)?;
        distinct(
            &format!("{field}/depends_on"),
            &self.depends_on,
            MAX_PER_ITEM,
        )
    }
}

fn validate(plan: &PlanFields) -> Result<(), SchemaError> {
    if plan.created > plan.updated {
        return Err(SchemaError::new("updated", "must not be before created"));
    }
    match (1..=MAX_PLAN_VERSION).contains(&plan.version) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "version",
                format!("must be 1 to {MAX_PLAN_VERSION}"),
            ));
        }
    }
    markdown("approach", &plan.approach, MAX_APPROACH_CHARS)?;
    validate_items(&plan.items)?;
    plan.proposed_by.validate("proposed_by")?;
    validate_acceptance(plan)?;
    let text = plan.text_bytes();
    if text > MAX_PLAN_TEXT_BYTES {
        return Err(SchemaError::new(
            "text",
            format!("the plan's text is {text} bytes; the most is {MAX_PLAN_TEXT_BYTES}"),
        ));
    }
    // A deadline is informational: one already past is allowed.
    match &plan.team {
        Some(team) => team.validate("team"),
        None => Ok(()),
    }
}

/// An acceptance is recorded from the plan's acceptance on (accepted, and
/// superseded after), never on a draft or a discarded draft, and falls
/// while the plan exists.
fn validate_acceptance(plan: &PlanFields) -> Result<(), SchemaError> {
    let was_accepted = match plan.state {
        PlanState::Accepted | PlanState::Superseded => true,
        PlanState::Draft | PlanState::Discarded => false,
    };
    match (&plan.accepted, was_accepted) {
        (None, false) => Ok(()),
        (Some((by, at)), true) => {
            by.validate("accepted/by")?;
            match plan.created <= *at && *at <= plan.updated {
                true => Ok(()),
                false => Err(SchemaError::new(
                    "accepted/at",
                    "must be from when the plan was created to when it was updated",
                )),
            }
        }
        (_, _) => Err(SchemaError::new(
            "accepted",
            format!("does not match state {}", plan.state.as_str()),
        )),
    }
}

fn validate_items(items: &[PlanItem]) -> Result<(), SchemaError> {
    let ids: Vec<&Slug> = items.iter().map(|item| &item.id).collect();
    distinct("items", &ids, MAX_ITEMS)?;
    for (index, item) in items.iter().enumerate() {
        item.validate(&format!("items/{index}"))?;
        let siblings = item
            .depends_on
            .iter()
            .all(|dependency| *dependency != item.id && ids.contains(&dependency));
        if !siblings {
            return Err(SchemaError::new(
                format!("items/{index}/depends_on"),
                "must name other items of this plan",
            ));
        }
    }
    match blocked_by_cycle(items) {
        Some(index) => Err(SchemaError::new(
            format!("items/{index}/depends_on"),
            "is on or behind a dependency cycle",
        )),
        None => Ok(()),
    }
}

/// The first item that can never start because its dependencies loop:
/// items are settled once all they depend on are, until none settles.
/// Every dependency names a sibling (checked before).
fn blocked_by_cycle(items: &[PlanItem]) -> Option<usize> {
    let position: BTreeMap<&Slug, usize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (&item.id, index))
        .collect();
    let mut settled = vec![false; items.len()];
    loop {
        let ready: Vec<usize> = (0..items.len())
            .filter(|&index| !settled[index])
            .filter(|&index| {
                items[index]
                    .depends_on
                    .iter()
                    .all(|dependency| settled[position[dependency]])
            })
            .collect();
        if ready.is_empty() {
            return settled.iter().position(|done| !done);
        }
        ready.into_iter().for_each(|index| settled[index] = true);
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
