//! The live board view on the parent socket (#2475, #2478, review #2488).
//!
//! `get_swarm_board {swarm, agent_id?, id?}` answers the board of a swarm
//! this session hosts: one of its own environments (as `get_containers`
//! lists them without `all`) that is running or retained, by its ref. It
//! reads through the environments capability's `ReadSwarmBoard`, which
//! reads the board's own adapter: no client ever opens the file. With
//! `agent_id` the board is asked of that descendant, routed as
//! `swarm_control` routes. Either way the answer is made by a task of its
//! own, so neither a running turn nor a slow board holds the connection's
//! reader; a `get_swarm_board` whose fields are not strings is refused at
//! once, never queued as a command.
//!
//! `swarm_board_changed {swarm, run, revision, removed}` tells every client
//! that a board appeared, moved, was made again (another `run`), or went
//! (`removed: true`, with `run` and `revision` null). The boards are polled
//! once a second, only while a client is connected, so no swarm is hinted
//! more than once a second; a board that cannot be read keeps what was last
//! seen of it. A client connecting later is told every board already seen,
//! on connect. A descendant's hints reach this harness's clients carrying
//! `agent_id`, the agent hosting that board (the sub-agent monitor forwards
//! them).
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::protocol::AgentEvent;
use super::uds_ext_protocol::ClientToolRegistry;
use crate::application::environments::dto::{BoardSighting, HostedBoardRevision};
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// How often the boards are polled: no swarm is hinted more than once in
/// this time.
pub(super) const HINT_INTERVAL: Duration = Duration::from_secs(1);
const COMMAND: &str = "get_swarm_board";
const HINT: &str = "swarm_board_changed";
const MALFORMED: &str = "get_swarm_board takes string fields: id, swarm and agent_id";

/// The board view's own handle on the session's boards (#2488 review M5):
/// the use case, once composition has installed it; `None` before, or in a
/// harness without environments.
#[derive(Clone)]
pub(crate) struct SwarmBoards(Arc<dyn Fn() -> Option<Arc<ReadSwarmBoard>> + Send + Sync>);

impl SwarmBoards {
    pub(crate) fn new(
        current: impl Fn() -> Option<Arc<ReadSwarmBoard>> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(current))
    }

    fn current(&self) -> Option<Arc<ReadSwarmBoard>> {
        (self.0)()
    }
}

/// A line's `type`, read first so only a `get_swarm_board` is parsed
/// further.
#[derive(Deserialize)]
struct Tagged<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
}

#[derive(Deserialize)]
struct Command {
    id: Option<String>,
    swarm: Option<String>,
    agent_id: Option<String>,
}

/// What a connection's reader hands the board view.
pub(super) struct BoardViewCtx<'a> {
    pub(super) boards: Option<&'a SwarmBoards>,
    pub(super) clients: &'a ClientToolRegistry,
    pub(super) subagents: &'a Option<SubagentRegistry>,
    pub(super) client_id: u64,
}

/// A connection's line, taken here when it is a `get_swarm_board`: `true`
/// when it was. Its answer is made and written by a task of its own (as
/// `swarm_control` routes are), so the reader goes on to its next line.
pub(super) fn intercept(line: &str, ctx: BoardViewCtx<'_>) -> bool {
    match serde_json::from_str::<Tagged<'_>>(line) {
        Ok(tagged) if tagged.kind == COMMAND => {}
        Ok(_) | Err(_) => return false,
    }
    let clients = ctx.clients.clone();
    let client_id = ctx.client_id;
    let Ok(Command {
        id,
        swarm,
        agent_id,
    }) = serde_json::from_str::<Command>(line)
    else {
        let id = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|command| command.get("id").and_then(Value::as_str).map(str::to_owned));
        tokio::spawn(async move {
            let event = AgentEvent::err(id.as_deref(), COMMAND, MALFORMED);
            write(&clients, client_id, id.as_deref(), &event).await;
        });
        return true;
    };
    match agent_id {
        Some(agent_id) => {
            let subagents = ctx.subagents.clone();
            let command: Value = serde_json::from_str(line).expect("a command that parsed");
            tokio::spawn(async move {
                let event = super::uds_busy_subagents::forward_child_command(
                    &subagents,
                    id.as_deref(),
                    &agent_id,
                    command,
                )
                .await;
                write(&clients, client_id, id.as_deref(), &event).await;
            });
        }
        None => {
            let boards = ctx.boards.and_then(SwarmBoards::current);
            tokio::spawn(async move {
                let event = answer(boards.as_deref(), id.as_deref(), swarm.as_deref()).await;
                write(&clients, client_id, id.as_deref(), &event).await;
            });
        }
    }
    true
}

async fn write(clients: &ClientToolRegistry, client_id: u64, id: Option<&str>, event: &AgentEvent) {
    super::uds_busy_subagents::write_event(clients, client_id, id, COMMAND, event).await;
}

/// The answer to `get_swarm_board` for `swarm`, from `boards` when this
/// harness has them: `{swarm, …board}`.
pub(super) async fn answer(
    boards: Option<&ReadSwarmBoard>,
    id: Option<&str>,
    swarm: Option<&str>,
) -> AgentEvent {
    let Some(swarm) = swarm else {
        return AgentEvent::err(
            id,
            COMMAND,
            "swarm is required (an environment ref such as C1)",
        );
    };
    let Some(boards) = boards else {
        return AgentEvent::err(id, COMMAND, "swarm boards are unavailable in this harness");
    };
    match boards.board(swarm).await {
        Ok(snapshot) => {
            let Value::Object(board) = snapshot.into_value() else {
                unreachable!("a board snapshot is a JSON object by construction");
            };
            assert!(
                !board.contains_key("swarm"),
                "the board's projection names no swarm of its own"
            );
            let mut data = Map::with_capacity(board.len() + 1);
            data.insert("swarm".into(), Value::from(swarm));
            data.extend(board);
            AgentEvent::ok(id, COMMAND, Some(Value::Object(data)))
        }
        Err(refusal) => AgentEvent::err(id, COMMAND, refusal.to_string()),
    }
}

/// The boards seen, by swarm: each one's last revision and run.
pub(super) type Seen = BTreeMap<String, HostedBoardRevision>;

/// What the poll has seen, kept by the poll and read by the accept loop to
/// tell each new connection (#2488 review M1).
#[derive(Clone, Default)]
pub(crate) struct BoardHints(Arc<Mutex<Seen>>);

impl BoardHints {
    /// One `swarm_board_changed` line per board already seen, ended by a
    /// newline, for a connection just accepted.
    pub(crate) fn connect_lines(&self) -> Vec<String> {
        self.seen()
            .iter()
            .map(|(swarm, revision)| changed(swarm, revision) + "\n")
            .collect()
    }

    fn seen(&self) -> Seen {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn keep(&self, seen: Seen) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = seen;
    }
}

fn changed(swarm: &str, revision: &HostedBoardRevision) -> String {
    serde_json::json!({
        "type": HINT,
        "swarm": swarm,
        "run": revision.run,
        "revision": revision.revision,
        "removed": false,
    })
    .to_string()
}

fn removed(swarm: &str) -> String {
    serde_json::json!({
        "type": HINT,
        "swarm": swarm,
        "run": null,
        "revision": null,
        "removed": true,
    })
    .to_string()
}

/// The hints for `sightings` against what was `seen`, and what is seen
/// after them. A board first seen, moved or made again (another run, even
/// at the same revision) is hinted; one seen before and not sighted now
/// (stopped, gone, no longer a board) is hinted removed; one that could not
/// be read keeps what was last seen and is not hinted.
pub(super) fn changes(
    seen: &Seen,
    sightings: BTreeMap<String, BoardSighting>,
) -> (Vec<String>, Seen) {
    let mut hints = Vec::new();
    let mut now = Seen::new();
    for (swarm, sighting) in sightings {
        match sighting {
            BoardSighting::Board(revision) => {
                if seen.get(&swarm) != Some(&revision) {
                    hints.push(changed(&swarm, &revision));
                }
                now.insert(swarm, revision);
            }
            BoardSighting::Unreadable(error) => {
                tracing::debug!(
                    swarm = %swarm,
                    %error,
                    "swarm board unreadable this poll; what was last seen of it stands"
                );
                if let Some(last) = seen.get(&swarm) {
                    now.insert(swarm, last.clone());
                }
            }
        }
    }
    for swarm in seen.keys().filter(|swarm| !now.contains_key(*swarm)) {
        hints.push(removed(swarm));
    }
    (hints, now)
}

/// Polls the session's boards every [`HINT_INTERVAL`] while a client is
/// connected and broadcasts each change. A tick that finds no receiver
/// (the last client left mid-tick) sends nothing and takes nothing as seen;
/// a send that fails is the same race and is let go. The poll ends only
/// with its owner ([`AbortOnDrop`], the accept loop).
pub(super) fn spawn_change_hints(
    boards: SwarmBoards,
    hints: BoardHints,
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
    live_clients: Arc<AtomicU32>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(HINT_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            let read = match (live_clients.load(Ordering::SeqCst), boards.current()) {
                (1.., Some(read)) => read,
                (0, _) | (_, None) => continue,
            };
            let (lines, now) = changes(&hints.seen(), read.sightings().await);
            match broadcast_tx.receiver_count() {
                0 => continue,
                _ => {
                    for line in lines {
                        let _ = broadcast_tx.send(line);
                    }
                    hints.keep(now);
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
