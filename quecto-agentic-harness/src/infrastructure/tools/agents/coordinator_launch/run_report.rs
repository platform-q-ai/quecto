//! A kept swarm board read as the project board folds a run back (#2481
//! S4c follow-up): the board's projection (the `_board` read the parent
//! socket serves as `get_swarm_board`) as a [`SwarmRunView`]: how the run
//! ended, its items with their states, owners and evidence, and the tokens
//! it was observed to spend.
//!
//! Only a whole ended run is a view: one whose status is none of the five
//! that end a swarm (it was still running, or paused, as its coordinator
//! stopped) is none, nor is a board with no run or no readable spend, nor
//! one whose projection left items out (fewer listed than its `totals`
//! count, or no count), whose items cannot all be told: that one is
//! logged. Rows of other kinds left out (members, evidence) do not matter. An
//! item is read only when its title is text and its state one the swarm
//! writes (an allowlist): any other is left out, so the plan item it
//! might stand for stays as todo. An evidence entry is read only when its
//! artifact and revision are both text.
use serde_json::Value;

use crate::application::project_board::dto::{
    SwarmEnd, SwarmEvidence, SwarmItemState, SwarmItemView, SwarmRunView,
};
use crate::domain::swarm::{RunState, TaskState};

/// The view of `board`, a kept board's projection, when its run ended.
pub fn swarm_run_view(board: &Value) -> Option<SwarmRunView> {
    // Only a projection that holds every item (an allowlist): one past a
    // row limit or its byte budget on its items cannot tell an item the
    // swarm never seeded from one left out. Members or evidence left out
    // do not matter here.
    let listed = board.get("items")?.as_array()?;
    let total = board.get("totals")?.get("items")?.as_u64()?;
    match usize::try_from(total) {
        Ok(total) if total == listed.len() => {}
        Ok(_) | Err(_) => {
            tracing::warn!(
                total,
                listed = listed.len(),
                "a swarm board left items out, so it is no whole view of its run"
            );
            return None;
        }
    }
    let run = board.get("run")?.as_object()?;
    let end = SwarmEnd::of(&RunState::new(run.get("status")?.as_str()?))?;
    let observed_tokens = run.get("budget")?.get("observed_tokens")?.as_u64()?;
    let items = listed.iter().filter_map(item).collect();
    Some(SwarmRunView {
        end,
        items,
        observed_tokens,
    })
}

/// One of the board's items, when its title and state can be read.
fn item(item: &Value) -> Option<SwarmItemView> {
    let title = item.get("title")?.as_str()?.to_owned();
    let state = SwarmItemState::of(&TaskState::new(item.get("state")?.as_str()?))?;
    let owner = item.get("owner").and_then(Value::as_str).map(str::to_owned);
    let evidence = item
        .get("evidence")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |entries| {
            entries.iter().filter_map(evidence).collect()
        });
    Some(SwarmItemView {
        title,
        state,
        owner,
        evidence,
    })
}

/// One evidence entry, when its artifact and revision are both text.
fn evidence(entry: &Value) -> Option<SwarmEvidence> {
    Some(SwarmEvidence {
        artifact: entry.get("artifact")?.as_str()?.to_owned(),
        revision: entry.get("revision")?.as_str()?.to_owned(),
    })
}

#[cfg(test)]
#[path = "run_report_tests.rs"]
mod tests;
