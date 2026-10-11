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
use super::task_parts::{MAX_ITEMS, MAX_LINE_CHARS, MAX_PER_ITEM, Team, lines};
use std::collections::BTreeMap;

/// The most characters of a plan's approach (GitHub's issue body limit).
pub const MAX_APPROACH_CHARS: usize = 65_536;
/// The most versions a task's plan reaches.
pub const MAX_PLAN_VERSION: u32 = 999;

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
    pub created: Timestamp,
    pub updated: Timestamp,
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
    if !(1..=MAX_PLAN_VERSION).contains(&plan.version) {
        return Err(SchemaError::new(
            "version",
            format!("must be 1 to {MAX_PLAN_VERSION}"),
        ));
    }
    markdown("approach", &plan.approach, MAX_APPROACH_CHARS)?;
    validate_items(&plan.items)?;
    match &plan.team {
        Some(team) => {
            team.validate("team")?;
            let deadline = team.deadline.as_ref();
            match deadline.is_none_or(|deadline| *deadline > plan.created) {
                true => Ok(()),
                false => Err(SchemaError::new(
                    "team/deadline",
                    "must be after the plan was created",
                )),
            }
        }
        None => Ok(()),
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
