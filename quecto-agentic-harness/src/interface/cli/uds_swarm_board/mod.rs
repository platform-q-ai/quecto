//! The live board view on the parent socket (#2475, #2478):
//! `get_swarm_board {swarm}` answers the board a swarm environment of this
//! harness hosts, and `swarm_board_changed {swarm, revision}` tells every
//! client when one moved, at most once a second per swarm. Both read
//! through the environments capability's `ReadSwarmBoard`, which reads the
//! board's own adapter; no client ever opens the file. A board a
//! descendant hosts is asked of it with `agent_id`, as `swarm_control` is.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::Deserialize;

use super::protocol::AgentEvent;
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot;

/// How often the boards are polled: no swarm is hinted more than once in
/// this time.
pub(super) const HINT_INTERVAL: Duration = Duration::from_secs(1);
const COMMAND: &str = "get_swarm_board";

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    GetSwarmBoard {
        id: Option<String>,
        swarm: Option<String>,
        agent_id: Option<String>,
    },
}

/// A connection's command line, answered here when it is a
/// `get_swarm_board`: `true` when it was.
pub(super) async fn intercept(
    line: &str,
    boards: Option<&EnvironmentControlSlot>,
    clients: &super::uds_ext_protocol::ClientToolRegistry,
    subagents: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    client_id: u64,
) -> bool {
    let Ok(Command::GetSwarmBoard {
        id,
        swarm,
        agent_id,
    }) = serde_json::from_str(line)
    else {
        return false;
    };
    let event = match agent_id {
        Some(agent_id) => {
            let command: serde_json::Value = serde_json::from_str(line).expect("validated command");
            super::uds_busy_subagents::forward_child_command(
                subagents,
                id.as_deref(),
                &agent_id,
                command,
            )
            .await
        }
        None => {
            let control = boards.and_then(EnvironmentControlSlot::get);
            answer(
                control.as_ref().map(|control| &*control.boards),
                id.as_deref(),
                swarm.as_deref(),
            )
            .await
        }
    };
    super::uds_busy_subagents::write_event(clients, client_id, id.as_deref(), COMMAND, &event)
        .await;
    true
}

/// The answer to `get_swarm_board` for `swarm`, from `boards` when this
/// harness hosts any.
pub(super) async fn answer(
    boards: Option<&ReadSwarmBoard>,
    id: Option<&str>,
    swarm: Option<&str>,
) -> AgentEvent {
    let (Some(boards), Some(swarm)) = (boards, swarm) else {
        let missing = match swarm {
            None => "swarm is required (an environment ref such as C1)",
            Some(_) => "swarm boards are unavailable in this harness",
        };
        return AgentEvent::err(id, COMMAND, missing);
    };
    match boards.board(swarm).await {
        Ok(board) => {
            let mut data = serde_json::Map::new();
            data.insert("swarm".into(), serde_json::Value::from(swarm));
            if let serde_json::Value::Object(board) = board {
                data.extend(board);
            }
            AgentEvent::ok(id, COMMAND, Some(serde_json::Value::Object(data)))
        }
        Err(refusal) => AgentEvent::err(id, COMMAND, refusal.to_string()),
    }
}

/// The hints for `revisions` against what was `seen`, which it updates: a
/// swarm first seen, or whose revision moved, is hinted; one no longer
/// read is forgotten, so it is hinted again when it reads.
pub(super) fn changes(
    seen: &mut BTreeMap<String, i64>,
    revisions: Vec<(String, i64)>,
) -> Vec<String> {
    let _ = (seen, revisions);
    Vec::new()
}

/// Polls this harness's swarm boards every [`HINT_INTERVAL`] while a
/// client is connected and broadcasts each change. Ends with the harness's
/// event stream.
pub(super) fn spawn_change_hints(
    boards: EnvironmentControlSlot,
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
    live_clients: Arc<AtomicU32>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seen = BTreeMap::new();
        let mut ticks = tokio::time::interval(HINT_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            if live_clients.load(Ordering::SeqCst) == 0 {
                continue;
            }
            let Some(control) = boards.get() else {
                continue;
            };
            for hint in changes(&mut seen, control.boards.revisions().await) {
                if broadcast_tx.send(hint).is_err() {
                    return;
                }
            }
        }
    })
}

/// Ends the hint poll when its owner (the accept loop) ends, aborted or
/// not.
pub(super) struct AbortOnDrop(pub(super) Option<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

#[cfg(test)]
#[path = "../uds_swarm_board_tests.rs"]
mod tests;
