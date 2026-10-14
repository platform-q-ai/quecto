//! A started run's keeper (S4d, #2479): what it sees of the run's swarm,
//! and what one look at the run and its swarm comes to.
//!
//! A keeper looks every [`KEEPER_PERIOD`] while its session lasts:
//! - **The board first.** A run ended (by `end_run`, or as lost by the
//!   merge that completed its task) or a task gone (archived) ends the
//!   keeping: nothing is left to keep.
//! - **The swarm next**, by its environment ref, as the session's live
//!   board view reads it. A swarm ended (one of the five terminal
//!   statuses) is folded back, once; still going (paused included, as it
//!   waits for its parent to close it) it is watched on.
//! - **Then the claim**, renewed while less than [`RENEW_BELOW_SECONDS`]
//!   are left. A claim another holds, or none, is no longer the keeper's
//!   to renew: it stops renewing and keeps watching, so it can still fold
//!   the run back.
//! - **A coordinator lost.** A swarm whose board cannot be read, or whose
//!   coordinator is not alive, while no terminal status was ever seen, is
//!   stalled. Stalled for [`STALL_LIMIT`] in a row, the keeper gives up:
//!   it writes nothing (a board unreadable for a while is no proof the
//!   run failed, and a fold-back cannot be taken back), stops renewing so
//!   the claim lapses, and leaves the run open for `end_run`.
use std::time::Duration;

use serde_json::Value;

use super::dto::ProjectBoardError;
use super::fold_back::{SwarmEnd, SwarmEvidence, SwarmItemState, SwarmItemView, SwarmRunView};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::swarm::policy::status_is_alive;
use crate::domain::swarm::{RunState, TaskState};

/// How often a keeper looks.
pub const KEEPER_PERIOD: Duration = Duration::from_secs(60);
/// A claim with less than this left (45 minutes) is renewed.
pub const RENEW_BELOW_SECONDS: i64 = 45 * 60;
/// How long a swarm may stall before its keeper gives up (30 minutes).
pub const STALL_LIMIT: Duration = Duration::from_secs(30 * 60);

/// A started run to keep: the task, its run and the environment its swarm
/// runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptRun {
    pub task: Slug,
    pub run: u32,
    pub environment_ref: String,
}

/// What a keeper sees of a started run's swarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmSight {
    /// Still going, or paused: whether its coordinator is a live member.
    Going { coordinator_alive: bool },
    /// Ended, as it stood then.
    Ended(SwarmRunView),
}

/// What a swarm board's projection (the live view's `get_swarm_board`
/// shape) shows of its swarm; `None` for a projection not of that shape,
/// which a keeper counts as unread.
pub fn sight_of(projection: &Value) -> Option<SwarmSight> {
    let run = projection.get("run")?;
    let ended = match run {
        Value::Null => None,
        run => SwarmEnd::of(&RunState::new(run.get("status")?.as_str()?)),
    };
    let Some(end) = ended else {
        let members = projection.get("members")?.as_array()?;
        let coordinator_alive = members.iter().any(|member| {
            member.get("role").and_then(Value::as_str) == Some("coordinator")
                && status_is_alive(member.get("status").and_then(Value::as_str))
        });
        return Some(SwarmSight::Going { coordinator_alive });
    };
    let items = projection.get("items")?.as_array()?;
    let tokens = run
        .get("budget")
        .and_then(|budget| budget.get("observed_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some(SwarmSight::Ended(SwarmRunView {
        end,
        items: items.iter().filter_map(item_of).collect(),
        observed_tokens: u64::try_from(tokens).unwrap_or(0),
    }))
}

/// An item of the projection, when its title is text and its state one
/// the swarm writes; evidence entries that are not objects naming an
/// artifact and a revision as text are skipped.
fn item_of(item: &Value) -> Option<SwarmItemView> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let state = SwarmItemState::of(&TaskState::new(item.get("state")?.as_str()?))?;
    let evidence = item
        .get("evidence")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(SwarmEvidence {
                        artifact: text(entry, "artifact")?,
                        revision: text(entry, "revision")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(SwarmItemView {
        title: text(item, "title")?,
        state,
        owner: text(item, "owner"),
        evidence,
    })
}

/// What a keeper carries from one look to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    /// Whether the claim is still the keeper's to renew.
    pub renewing: bool,
    /// Since when (the board clock's monotonic time) the swarm has been
    /// stalled, while it is.
    pub stalled_since: Option<Duration>,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            renewing: true,
            stalled_since: None,
        }
    }
}

/// A renewal a look made, with the seconds the claim had left before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewal {
    Renewed {
        left_s: i64,
    },
    Refused {
        left_s: i64,
        error: ProjectBoardError,
    },
}

/// Why a keeper stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperEnd {
    /// It folded the run back.
    Folded,
    /// The run had ended already: folded back, ended by hand, or lost by
    /// the merge that completed its task.
    Ended,
    /// The task, or its run, is no longer on the board (archived).
    Gone,
    /// The fold-back was refused for good (its record says why); the run
    /// is left open for `end_run`.
    FoldBackRefused,
    /// The swarm stalled past [`STALL_LIMIT`]; the run is left open for
    /// `end_run`.
    SwarmLost,
    /// Its session ended.
    SessionEnded,
}

/// What one look came to: a renewal it made, and whether the keeping ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    pub renewal: Option<Renewal>,
    pub end: Option<KeeperEnd>,
}

impl Look {
    pub fn ending(end: KeeperEnd) -> Self {
        Self {
            renewal: None,
            end: Some(end),
        }
    }
}

#[cfg(test)]
#[path = "keeper_tests.rs"]
pub(super) mod tests;
