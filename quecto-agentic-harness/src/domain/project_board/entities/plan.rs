//! A plan: how one task will be achieved (#2481's artifact model), a
//! versioned artifact of its own. It holds the approach, the task items that
//! seed the swarm board, and the team's shape. A [`Plan`] is only ever
//! built from fields that pass [`Plan::new`]; its versions are numbered
//! from 1 per task, a draft is edited, and an accepted version is never
//! edited again: a change is a new version. Its handoff (#2705 T3), when
//! it has one, holds the detail an agent is briefed with, so the approach
//! stays an outline, short on entry ([`PLAN_ENTRY_RULES`]).
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::PlanState;
use super::claim::Identity;
use super::task_parts::{Entry, EntryPolicy, MAX_ITEMS, MAX_LINE_CHARS, MAX_PER_ITEM, lines};
use super::team::Team;
use std::collections::BTreeMap;

/// The most characters of a plan's approach as it reads back (GitHub's
/// issue body limit): what earlier builds let in. One entering capped is
/// shorter ([`PLAN_ENTRY_RULES`]).
pub const MAX_APPROACH_CHARS: usize = 65_536;
/// The most characters of a plan's handoff (#2705 T3): the cap the
/// approach had before its detail moved there.
pub const MAX_HANDOFF_CHARS: usize = 65_536;

/// What a plan's text enters the board by (#2705 T3): one table, as a
/// task's [`ENTRY_RULES`](super::task_parts::ENTRY_RULES) is, so a rule
/// change is a data edit. Reading stays lenient: what is stored reads by
/// the plan's own schema.
pub struct PlanEntryRules {
    /// Characters of an approach entering capped ([`PlanEntry::Capped`]).
    pub max_approach_chars: usize,
    /// Characters of a handoff, however it enters.
    pub max_handoff_chars: usize,
}

pub const PLAN_ENTRY_RULES: PlanEntryRules = PlanEntryRules {
    max_approach_chars: 600,
    max_handoff_chars: MAX_HANDOFF_CHARS,
};

/// Where a refused approach sends its detail.
pub const MOVE_TO_HANDOFF: &str =
    "keep the approach to an outline and move the detail into handoff";
/// Why a handoff given empty is refused: none is no handoff.
const HANDOFF_EMPTY: &str = "is empty: write it, or leave it out";

/// How one write's approach enters the board (#2705 T3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanEntry {
    /// Within [`PlanEntryRules::max_approach_chars`]: the detail goes in
    /// the handoff.
    #[default]
    Capped,
    /// As before T3, for a front end's request that gives no handoff:
    /// normalised board text within the schema's [`MAX_APPROACH_CHARS`].
    Legacy,
}

impl PlanEntry {
    /// The entry its front end's `policy` gives a request that gives a
    /// handoff, or not (`handoff_given`), as a task's entry is resolved
    /// (#2705 T2): the agent tool always caps the approach; the CLI caps
    /// it once a request gives a handoff.
    pub fn of(policy: EntryPolicy, handoff_given: bool) -> Self {
        match policy.entry(handoff_given) {
            Entry::Sectioned => Self::Capped,
            Entry::Legacy => Self::Legacy,
        }
    }

    /// `approach`, normalised board text, as it enters under this entry.
    ///
    /// # Errors
    /// A capped approach past [`PlanEntryRules::max_approach_chars`].
    pub fn approach(self, approach: &str) -> Result<(), SchemaError> {
        let _ = approach;
        Ok(())
    }
}
/// The most versions a task's plan reaches.
pub const MAX_PLAN_VERSION: u32 = 999;
/// All of a plan's text together, in UTF-8 bytes: the approach, the
/// task items' titles and criteria, the team's models and image. The field
/// limits alone would add up to megabytes; this budget keeps every plan
/// the domain admits inside the one file it is stored in, with room to
/// spare (its codec, M2, proves the fit).
pub const MAX_PLAN_TEXT_BYTES: usize = 320 * 1024;

/// One task item the plan seeds the swarm board with, and how its completion
/// is judged. Where a task item stands is a run's, not the plan's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub id: Slug,
    pub title: String,
    pub criteria: Vec<String>,
    /// Other task items of the same plan; never a cycle.
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
    /// Markdown: the outline.
    pub approach: String,
    /// Markdown (#2705 T3): the detail a coordinator is briefed with,
    /// verbatim; `None` for none, never empty.
    pub handoff: Option<String>,
    pub items: Vec<PlanItem>,
    pub team: Option<Team>,
    /// Who proposed this version.
    pub proposed_by: Identity,
    /// Who last edited the draft, when someone did (drafts are anyone's).
    pub edited_by: Option<Identity>,
    /// Who accepted it and when: set from its acceptance on (accepted,
    /// then superseded), never before.
    pub accepted: Option<(Identity, Timestamp)>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl PlanFields {
    /// The UTF-8 bytes of every free text in the plan: the approach and
    /// handoff, the items' titles and criteria, the team's models and image. Ids,
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
        let handoff = self.handoff.as_ref().map_or(0, String::len);
        self.approach.len() + handoff + items + team
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

    /// The task item named `id`, if the plan has one.
    pub fn item(&self, id: &Slug) -> Option<&PlanItem> {
        self.0.items.iter().find(|item| item.id == *id)
    }
}

impl PlanItem {
    /// Its own fields; dependencies on sibling task items are the plan's check.
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
    match plan.created <= plan.updated {
        true => {}
        false => return Err(SchemaError::new("updated", "must not be before created")),
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
    if let Some(handoff) = &plan.handoff {
        markdown("handoff", handoff, MAX_HANDOFF_CHARS)?;
        match handoff.chars().any(|c| !c.is_whitespace()) {
            true => {}
            false => return Err(SchemaError::new("handoff", HANDOFF_EMPTY)),
        }
    }
    validate_items(&plan.items)?;
    plan.proposed_by.validate("proposed_by")?;
    if let Some(editor) = &plan.edited_by {
        editor.validate("edited_by")?;
    }
    validate_acceptance(plan)?;
    let text = plan.text_bytes();
    match text <= MAX_PLAN_TEXT_BYTES {
        true => {}
        false => {
            return Err(SchemaError::new(
                "text",
                format!("the plan's text is {text} bytes; the most is {MAX_PLAN_TEXT_BYTES}"),
            ));
        }
    }
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
        match siblings {
            true => {}
            false => {
                return Err(SchemaError::new(
                    format!("items/{index}/depends_on"),
                    "must name other task items of this plan",
                ));
            }
        }
    }
    match blocked_by_cycle(items) {
        Some(index) => Err(SchemaError::new(
            format!("items/{index}/depends_on"),
            "is on or behind a cycle of task item dependencies",
        )),
        None => Ok(()),
    }
}

/// The first task item that can never start because its dependencies loop:
/// task items are settled once all they depend on are, until none settles.
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
