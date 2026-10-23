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

use crate::application::environments::dto::{BoardSighting, HostedBoardRevision};
use crate::application::environments::use_cases::ReadSwarmBoard;
use crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry;
use crate::interface::uds::controllers::uds_ext_protocol::ClientToolRegistry;
use crate::interface::uds::wire::protocol::AgentEvent;

/// How often the boards are polled: no swarm is hinted more than once in
/// this time.
pub(super) const HINT_INTERVAL: Duration = Duration::from_secs(1);
const COMMAND: &str = "get_swarm_board";
const HINT: &str = "swarm_board_changed";
pub(super) const MALFORMED: &str = "get_swarm_board takes string fields: id, swarm and agent_id";
/// The most `get_swarm_board` requests one connection has in flight.
const IN_FLIGHT: usize = 2;
const BUSY: &str = "get_swarm_board is busy: at most 2 requests in flight per connection";

/// One connection's board requests in flight (#2488 round 2 M3).
pub(in crate::interface) fn board_requests() -> Arc<tokio::sync::Semaphore> {
    Arc::new(tokio::sync::Semaphore::new(IN_FLIGHT))
}

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
    /// Read for a refusal's `id`, when it is a string.
    #[serde(default, deserialize_with = "string_or_none")]
    id: Option<String>,
}

/// A string, or `None` for anything else.
fn string_or_none<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => Some(text),
        _ => None,
    })
}

#[derive(Deserialize)]
struct Command {
    id: Option<String>,
    swarm: Option<String>,
    agent_id: Option<String>,
}

/// What a connection's reader hands the board view.
pub(in crate::interface) struct BoardViewCtx<'a> {
    pub(in crate::interface) boards: Option<&'a SwarmBoards>,
    pub(in crate::interface) clients: &'a ClientToolRegistry,
    pub(in crate::interface) subagents: &'a Option<SubagentRegistry>,
    pub(in crate::interface) client_id: u64,
    /// This connection's board requests in flight ([`board_requests`]).
    pub(in crate::interface) in_flight: &'a Arc<tokio::sync::Semaphore>,
}

/// A connection's line, taken here when it is a `get_swarm_board`: `true`
/// when it was. Its answer is made and written by a task of its own (as
/// `swarm_control` routes are), so the reader goes on to its next line.
pub(in crate::interface) fn intercept(line: &str, ctx: BoardViewCtx<'_>) -> bool {
    match serde_json::from_str::<Tagged<'_>>(line) {
        Ok(tagged) if tagged.kind == COMMAND => {}
        Ok(_) | Err(_) => return false,
    }
    let clients = ctx.clients.clone();
    let client_id = ctx.client_id;
    // One parse, whose depth the reader bounds: what fails it is refused
    // (round 2 M1), and the value parsed is what is forwarded.
    let parsed = serde_json::from_str::<Value>(line).ok();
    let command = parsed
        .as_ref()
        .and_then(|value| Command::deserialize(value).ok());
    let (
        Some(command_value),
        Some(Command {
            id,
            swarm,
            agent_id,
        }),
    ) = (parsed, command)
    else {
        let id = serde_json::from_str::<Tagged<'_>>(line)
            .ok()
            .and_then(|tagged| tagged.id);
        refuse(&clients, client_id, id.as_deref(), MALFORMED);
        return true;
    };
    let Ok(permit) = ctx.in_flight.clone().try_acquire_owned() else {
        refuse(&clients, client_id, id.as_deref(), BUSY);
        return true;
    };
    match agent_id {
        Some(agent_id) => {
            let subagents = ctx.subagents.clone();
            tokio::spawn(async move {
                let event = super::uds_busy_subagents::forward_child_command(
                    &subagents,
                    id.as_deref(),
                    &agent_id,
                    command_value,
                )
                .await;
                write(&clients, client_id, id.as_deref(), &event).await;
                drop(permit);
            });
        }
        None => {
            let boards = ctx.boards.and_then(SwarmBoards::current);
            tokio::spawn(async move {
                let event = answer(boards.as_deref(), id.as_deref(), swarm.as_deref()).await;
                write(&clients, client_id, id.as_deref(), &event).await;
                drop(permit);
            });
        }
    }
    true
}

/// A refusal written at once when the client's channel has room, and
/// dropped when it has none: no task waits on a client that does not read
/// (verification item 1).
fn refuse(clients: &ClientToolRegistry, client_id: u64, id: Option<&str>, error: &str) {
    let Some(writer) =
        crate::interface::uds::controllers::uds_ext_protocol::client_writer_tx(clients, client_id)
    else {
        return;
    };
    let mut line = serde_json::to_string(&AgentEvent::err(id, COMMAND, error)).unwrap_or_default();
    if line.len() > crate::infrastructure::processes::line_cap::EVENT_LINE_JSON_BUDGET {
        line = serde_json::to_string(&AgentEvent::err(None, COMMAND, error)).unwrap_or_default();
    }
    line.push('\n');
    match writer.try_send(line) {
        Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            tracing::debug!(
                client_id,
                "get_swarm_board refusal dropped: the client is not reading"
            );
        }
    }
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
        Ok(snapshot) => match snapshot.into_value() {
            // An object by construction, naming no swarm of its own; any
            // other answer is refused, never a panic on a client's path.
            Value::Object(board) if !board.contains_key("swarm") => {
                let mut data = Map::with_capacity(board.len() + 1);
                data.insert("swarm".into(), Value::from(swarm));
                data.extend(board);
                AgentEvent::ok(id, COMMAND, Some(Value::Object(data)))
            }
            Value::Object(_) => AgentEvent::err(
                id,
                COMMAND,
                "swarm board answer malformed: it names a swarm of its own",
            ),
            _ => AgentEvent::err(id, COMMAND, "swarm board answer malformed: not an object"),
        },
        Err(refusal) => AgentEvent::err(id, COMMAND, refusal.to_string()),
    }
}

/// The boards seen, by swarm: each one's last revision and run.
pub(super) type Seen = BTreeMap<String, HostedBoardRevision>;

/// What the poll has seen, kept by the poll and told to each new
/// connection (#2488 review M1). One lock orders both (round 2 M2): the
/// poll sends a tick's hints and keeps what it saw under it, and a
/// connection subscribes and reads what was seen under it, so it is told
/// each revision once and in order.
#[derive(Clone, Default)]
pub(crate) struct BoardHints(Arc<Mutex<Seen>>);

impl BoardHints {
    /// One `swarm_board_changed` line per board already seen, ended by a
    /// newline.
    #[cfg(test)]
    pub(crate) fn connect_lines(&self) -> Vec<String> {
        lines_of(&self.lock())
    }

    /// A new connection's broadcast receiver and the lines it is told on
    /// connect, taken together: every hint after these reaches it.
    pub(crate) fn subscribe(
        &self,
        broadcast_tx: &tokio::sync::broadcast::Sender<String>,
    ) -> (tokio::sync::broadcast::Receiver<String>, Vec<String>) {
        let seen = self.lock();
        (broadcast_tx.subscribe(), lines_of(&seen))
    }

    /// One tick: the hints `tick` makes from what was seen, sent, and what
    /// it saw kept, under the lock. With no receiver (the last client left
    /// mid-tick) nothing is sent or kept; a send that fails is that race,
    /// and is let go.
    pub(super) fn publish(
        &self,
        broadcast_tx: &tokio::sync::broadcast::Sender<String>,
        tick: impl FnOnce(&Seen) -> (Vec<String>, Seen),
    ) {
        let mut seen = self.lock();
        match broadcast_tx.receiver_count() {
            0 => {}
            _ => {
                let (lines, now) = tick(&seen);
                for line in lines {
                    let _ = broadcast_tx.send(line);
                }
                *seen = now;
            }
        }
    }

    /// Forgets what was seen (round 2 L5), under the lock and only while
    /// nobody is subscribed (the test `publish` makes): the next connection
    /// is told only what the poll sees from then on.
    pub(super) fn forget(&self, broadcast_tx: &tokio::sync::broadcast::Sender<String>) {
        let mut seen = self.lock();
        if broadcast_tx.receiver_count() == 0 {
            seen.clear();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn lines_of(seen: &Seen) -> Vec<String> {
    seen.iter()
        .map(|(swarm, revision)| changed(swarm, revision) + "\n")
        .collect()
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
/// connected and publishes each change ([`BoardHints::publish`]); with no
/// client connected, what was seen is forgotten. The poll ends only with
/// its owner ([`AbortOnDrop`], the accept loop).
pub(in crate::interface) fn spawn_change_hints(
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
                (0, _) => {
                    hints.forget(&broadcast_tx);
                    continue;
                }
                (_, None) => continue,
            };
            let sightings = read.sightings().await;
            hints.publish(&broadcast_tx, |seen| changes(seen, sightings));
        }
    })
}

/// Ends the hint poll when its owner (the accept loop) ends, aborted or
/// not.
pub(in crate::interface) struct AbortOnDrop(
    pub(in crate::interface) Option<tokio::task::JoinHandle<()>>,
);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

#[cfg(test)]
mod intercept_tests;
#[cfg(test)]
mod tests;
