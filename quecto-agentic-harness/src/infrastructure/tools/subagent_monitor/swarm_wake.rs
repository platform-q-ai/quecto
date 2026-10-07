//! A swarm coordinator's parent wakes on the run's state, not on every idle
//! turn (#2467). The coordinator's harness reads its board at each idle
//! boundary and sends `swarm_state`; once a child has sent one, its turn-end
//! note waits for the next, which holds it (workers busy), names the state
//! (finished, idle with nothing in flight) or, when the board could not be
//! read, sends the ordinary turn-end note. A child that never sends one keeps
//! today's note on every turn end.

use super::super::subagent_registry::{
    NotificationTx, SequencedSubagentNotification, SubagentNotification, SubagentRegistry,
    SwarmNoteState,
};

/// How long a held coordinator may go without a new turn before its parent
/// hears it has been quiet.
pub(crate) const QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Whether `agent_id`'s turn end waits for its next `swarm_state`: it has
/// sent one before. The held turn end is marked for that state to settle.
pub(super) fn defer_completion(registry: &SubagentRegistry, agent_id: &str) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(entry) = entries.get_mut(agent_id) else {
        return false;
    };
    if entry.swarm_reporting {
        entry.completion_deferred = true;
    }
    entry.swarm_reporting
}

/// Settle a `swarm_state`: the child now reports, its held turn end (if
/// any) is decided by the wake, and a hold arms the quiet timer.
pub(super) fn classify_swarm_state(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
    quiet_after: std::time::Duration,
) {
    let wake = value.get("wake").and_then(serde_json::Value::as_str);
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (held, hold) = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(agent_id) else {
            return;
        };
        entry.swarm_reporting = true;
        let held = std::mem::take(&mut entry.completion_deferred);
        let hold = (wake == Some("hold")).then(|| {
            let token = sequence;
            entry.swarm_hold = Some(token);
            token
        });
        (held, hold)
    };
    if let Some(token) = hold {
        arm_quiet_timer(registry, notify_tx, agent_id, token, quiet_after);
    }
    if !held {
        return;
    }
    let note = match wake {
        Some("hold") => None,
        Some("finished") => Some(SubagentNotification::SwarmState {
            agent_id: String::new(),
            state: SwarmNoteState::Finished { status },
        }),
        Some("idle") => Some(SubagentNotification::SwarmState {
            agent_id: String::new(),
            state: SwarmNoteState::Idle,
        }),
        // Unknown, or a wake this build does not know: the ordinary note.
        _ => Some(SubagentNotification::Completed {
            agent_id: String::new(),
        }),
    };
    if let Some(note) = note {
        send(registry, notify_tx, agent_id, sequence, note);
    }
}

/// One quiet note if the hold `token` still stands `quiet_after` from now:
/// a new turn (`agent_start`) or a later hold supersedes it.
fn arm_quiet_timer(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    token: u64,
    quiet_after: std::time::Duration,
) {
    let (Some(tx), Ok(runtime)) = (notify_tx.cloned(), tokio::runtime::Handle::try_current())
    else {
        return;
    };
    let (registry, agent_id) = (registry.clone(), agent_id.to_owned());
    runtime.spawn(async move {
        tokio::time::sleep(quiet_after).await;
        let standing = {
            let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .get_mut(&agent_id)
                .is_some_and(|entry| entry.swarm_hold.take_if(|held| *held == token).is_some())
        };
        if standing {
            let minutes = quiet_after.as_secs() / 60;
            let note = SubagentNotification::SwarmState {
                agent_id: String::new(),
                state: SwarmNoteState::Quiet { minutes },
            };
            send(&registry, Some(&tx), &agent_id, token, note);
        }
    });
}

/// Send `note` for `agent_id` as its display label and identity.
fn send(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    note: SubagentNotification,
) {
    let Some(tx) = notify_tx else { return };
    let label = super::notification_display_label(registry, agent_id);
    let note = match note {
        SubagentNotification::SwarmState { state, .. } => SubagentNotification::SwarmState {
            agent_id: label,
            state,
        },
        SubagentNotification::Completed { .. } => {
            SubagentNotification::Completed { agent_id: label }
        }
        other => other,
    };
    let uuid = super::notification_agent_uuid(registry, agent_id);
    let _ = tx.try_send(SequencedSubagentNotification::new_for_agent(
        sequence, note, uuid,
    ));
}
