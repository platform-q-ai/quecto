//! A swarm's run as the board keeps it once the swarm has ended (S4,
//! #2479): how its status, items, owners, evidence and spend map onto the
//! board's run of the plan it was briefed with.
//!
//! - **Outcome:** succeeded is succeeded; failed and budget-exhausted are
//!   failed; blocked and cancelled are stopped. Only those five statuses
//!   end a swarm, so only they make a [`SwarmEnd`], and a fold-back is only
//!   ever of an ended swarm.
//! - **Task items:** a swarm item is matched to the plan item whose brief
//!   title it carries ([`item_title`], `<slug>: <title>`), exactly; the
//!   first of several under one title is taken. Ready is todo, claimed
//!   doing, submitted review, completed done, blocked blocked. A plan item
//!   the swarm never seeded is todo (`unmatched`); a swarm item no plan
//!   item names is counted (`extra`), never kept.
//! - **Owner:** the member id, as one board line cut to
//!   [`MAX_OWNER_CHARS`]; none when the board cannot hold it.
//! - **Evidence:** one line per entry, `<artifact> at <revision>`, each
//!   bare `https://` address in it written as a `[link](…)`. A line the
//!   board cannot hold (its schema, its codec's text cell, a repeat, past
//!   [`MAX_PER_ITEM`] or past the run's [`MAX_RUN_TEXT_BYTES`]) is dropped
//!   and counted: one bad line never sinks the fold-back.
//! - **Cost:** the swarm's observed tokens, within the board's bound.
use super::ports::BoardTextCells;
use super::swarm_brief::item_title;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::run::{MAX_RUN_TEXT_BYTES, RunItem};
use crate::domain::project_board::entities::task_parts::{
    MAX_BUDGET_TOKENS, MAX_LINE_CHARS, MAX_OWNER_CHARS, MAX_PER_ITEM,
};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::telemetry::ItemCounts;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::domain::swarm::{RunState, TaskState};

/// The scheme of the one address a board line links.
const HTTPS: &str = "https://";
/// The text of a link written for a bare address.
const LINK_TEXT: &str = "link";

/// How a swarm ended: one of the statuses that end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmEnd {
    Succeeded,
    Failed,
    BudgetExhausted,
    Blocked,
    Cancelled,
}

impl SwarmEnd {
    pub const ALL: [Self; 5] = [
        Self::Succeeded,
        Self::Failed,
        Self::BudgetExhausted,
        Self::Blocked,
        Self::Cancelled,
    ];

    /// The end `status` is, when it ends the swarm (an allowlist: a
    /// swarm setting up, running or paused has not ended, nor has one
    /// whose status is any other text).
    pub fn of(status: &RunState) -> Option<Self> {
        match status.as_str() {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "budget-exhausted" => Some(Self::BudgetExhausted),
            "blocked" => Some(Self::Blocked),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// The board's outcome of a run that ended so.
    pub fn outcome(self) -> RunOutcome {
        match self {
            Self::Succeeded => RunOutcome::Succeeded,
            Self::Failed | Self::BudgetExhausted => RunOutcome::Failed,
            Self::Blocked | Self::Cancelled => RunOutcome::Stopped,
        }
    }
}

/// Where a swarm item stood: one of the five statuses the swarm writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmItemState {
    Ready,
    Claimed,
    Submitted,
    Completed,
    Blocked,
}

impl SwarmItemState {
    /// The state `state` is, when the swarm writes it (an allowlist).
    pub fn of(state: &TaskState) -> Option<Self> {
        match state.as_str() {
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "submitted" => Some(Self::Submitted),
            "completed" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }

    /// The board's state of a task item standing so.
    pub fn item_state(self) -> ItemState {
        match self {
            Self::Ready => ItemState::Todo,
            Self::Claimed => ItemState::Doing,
            Self::Submitted => ItemState::Review,
            Self::Completed => ItemState::Done,
            Self::Blocked => ItemState::Blocked,
        }
    }
}

/// One evidence entry a swarm item was submitted with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmEvidence {
    pub artifact: String,
    pub revision: String,
}

/// One item of an ended swarm's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmItemView {
    pub title: String,
    pub state: SwarmItemState,
    /// The member holding it, by id.
    pub owner: Option<String>,
    pub evidence: Vec<SwarmEvidence>,
}

/// An ended swarm's board, as its live view (`get_swarm_board`) reads
/// it: how it ended, its items in the view's order, and the tokens it
/// was observed to spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmRunView {
    pub end: SwarmEnd,
    pub items: Vec<SwarmItemView>,
    pub observed_tokens: u64,
}

/// What a fold-back keeps of a swarm's items: every plan item, in the
/// plan's order, and what was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldBackItems {
    pub items: Vec<RunItem>,
    /// Plan items no swarm item matched.
    pub unmatched: usize,
    /// Swarm items no plan item took.
    pub extra: usize,
    /// Evidence lines kept, and dropped.
    pub evidence: usize,
    pub dropped: usize,
}

impl FoldBackItems {
    /// The task items by the state each stood in.
    pub fn counts(&self) -> ItemCounts {
        let count = |state: ItemState| {
            let held = self.items.iter().filter(|item| item.state == state).count();
            u64::try_from(held).unwrap_or(u64::MAX)
        };
        ItemCounts {
            todo: count(ItemState::Todo),
            doing: count(ItemState::Doing),
            review: count(ItemState::Review),
            done: count(ItemState::Done),
            blocked: count(ItemState::Blocked),
        }
    }
}

/// Where each of `plan`'s task items stood when `view`'s swarm ended,
/// each line checked against `cells` as the board's codec holds it.
pub fn run_items(plan: &Plan, view: &SwarmRunView, cells: &dyn BoardTextCells) -> FoldBackItems {
    let planned = &plan.fields().items;
    let matched: Vec<Option<&SwarmItemView>> = planned
        .iter()
        .map(|item| {
            let key = item_title(item);
            view.items.iter().find(|swarm| swarm.title == key)
        })
        .collect();
    let taken = matched.iter().flatten().count();
    let owners: Vec<Option<String>> = matched
        .iter()
        .map(|swarm| swarm.and_then(|swarm| owner_of(swarm.owner.as_deref()?, cells)))
        .collect();
    // Owners first: what is left of the run's text is the evidence's.
    let mut room =
        MAX_RUN_TEXT_BYTES.saturating_sub(owners.iter().flatten().map(String::len).sum());
    let (mut evidence, mut dropped) = (0, 0);
    let items: Vec<RunItem> = planned
        .iter()
        .zip(matched.iter().zip(owners))
        .map(|(item, (swarm, owner))| {
            let lines = swarm.map_or(&[][..], |swarm| swarm.evidence.as_slice());
            let mut kept: Vec<String> = Vec::new();
            for entry in lines {
                let written = evidence_line(entry);
                let holds = kept.len() < MAX_PER_ITEM
                    && written.len() <= room
                    && !kept.contains(&written)
                    && held_line(&written, MAX_LINE_CHARS, cells);
                match holds {
                    true => {
                        room -= written.len();
                        kept.push(written);
                    }
                    false => dropped += 1,
                }
            }
            evidence += kept.len();
            RunItem {
                id: item.id.clone(),
                state: swarm.map_or(ItemState::Todo, |swarm| swarm.state.item_state()),
                owner,
                evidence: kept,
            }
        })
        .collect();
    let folded = FoldBackItems {
        unmatched: planned.len() - taken,
        extra: view.items.len() - taken,
        items,
        evidence,
        dropped,
    };
    debug_assert_eq!(folded.items.len(), planned.len(), "every plan item, once");
    debug_assert_eq!(
        folded.evidence + folded.dropped,
        matched
            .iter()
            .flatten()
            .map(|swarm| swarm.evidence.len())
            .sum::<usize>(),
        "every evidence entry of a matched item is kept or dropped"
    );
    folded
}

/// `entry` as one evidence line: `<artifact> at <revision>`, each part one
/// board line with its bare addresses linked. Empty when either part is.
pub fn evidence_line(entry: &SwarmEvidence) -> String {
    let artifact = linked(&board_text::line(&entry.artifact));
    let revision = linked(&board_text::line(&entry.revision));
    match (artifact.is_empty(), revision.is_empty()) {
        (false, false) => format!("{artifact} at {revision}"),
        _ => String::new(),
    }
}

/// The tokens a run spent: what the swarm observed, at most what the board
/// spells.
pub fn cost_of(view: &SwarmRunView) -> u64 {
    view.observed_tokens.min(MAX_BUDGET_TOKENS)
}

/// `text` with each word that is a bare `https://` address written as a
/// link to it; the codec judges the address itself.
fn linked(text: &str) -> String {
    text.split(' ')
        .map(
            |word| match word.starts_with(HTTPS) && word.len() > HTTPS.len() {
                true => format!("[{LINK_TEXT}]({word})"),
                false => word.to_string(),
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

/// Member id `raw` as an owner: one board line, cut to
/// [`MAX_OWNER_CHARS`]; none when the board cannot hold what is left.
fn owner_of(raw: &str, cells: &dyn BoardTextCells) -> Option<String> {
    let cut: String = board_text::line(raw)
        .chars()
        .take(MAX_OWNER_CHARS)
        .collect();
    let owner = board_text::line(&cut);
    held_line(&owner, MAX_OWNER_CHARS, cells).then_some(owner)
}

/// Whether the board holds `text` as one line of at most `max_chars`
/// characters: its schema's rule, and its codec's text cell.
fn held_line(text: &str, max_chars: usize, cells: &dyn BoardTextCells) -> bool {
    line("line", text, max_chars).is_ok() && cells.holds(text)
}

#[cfg(test)]
#[path = "fold_back_tests.rs"]
pub(super) mod tests;
