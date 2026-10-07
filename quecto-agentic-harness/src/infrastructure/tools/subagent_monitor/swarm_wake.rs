//! A swarm coordinator's parent wakes on the run's state, not on every idle
//! turn (#2467). Only a child launched with `coordinator: true` takes part:
//! its harness reads its board at each idle boundary and sends
//! `swarm_state`. Once it has sent one, its turn-end note waits for the
//! next, which settles it (`settle_held_note`): no note while work is in
//! flight, the run's state when it finished or went idle, the ordinary note
//! when the board could not be read or a client's prompt is owed its reply.
//! A child that never sends one keeps today's note on every turn end, and
//! a held note no state settles within [`SETTLE_AFTER`] is sent as it was.

use super::super::subagent_registry::{
    NotificationTx, SequencedSubagentNotification, SubagentNotification, SubagentRegistry,
    SwarmNoteState,
};
use crate::domain::swarm::parent_wake::{HeldNote, WakeKind, settle_held_note};
use serde::Deserialize as _;

/// How long a held coordinator may go without a new turn before its parent
/// hears it has been quiet.
pub(crate) const QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// How long a held turn end waits for the `swarm_state` its idle boundary
/// sends before it is sent as the ordinary note (a lost or late state).
pub(crate) const SETTLE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// A child's part in coordinator wakes, kept on its registry entry.
#[derive(Debug, Clone, Default)]
pub struct CoordinatorWake {
    /// Launched with `coordinator: true` (#2461): the one kind of child
    /// whose turn ends wait on its run's state.
    pub launched: bool,
    /// It has sent a `swarm_state`: its turn ends wait for the next one.
    reporting: bool,
    /// The held turn end's sequence, until a state settles it.
    deferred: Option<u64>,
    /// The one timer running for it: settling a held note, or the quiet
    /// report of a hold.
    timer: Option<Timer>,
}

#[derive(Debug, Clone)]
struct Timer {
    kind: TimerKind,
    token: u64,
    handle: tokio::task::AbortHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Settle,
    Quiet,
}

impl CoordinatorWake {
    /// A new turn is activity: a held coordinator is not quiet, and its held
    /// note waits for this turn's own end.
    pub(super) fn turn_started(&mut self) {
        self.cancel_timer();
    }

    fn cancel_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.handle.abort();
        }
    }
}

/// Whether `agent_id`'s turn end (`sequence`) waits for its next
/// `swarm_state`: a launched coordinator that has reported before. The
/// held turn end is sent as it was if no state settles it in time.
pub(super) fn defer_completion(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(entry) = entries.get_mut(agent_id) else {
        return false;
    };
    let wake = &mut entry.coordinator_wake;
    match wake.launched && wake.reporting {
        true => {
            wake.deferred = Some(sequence);
            let timer = (TimerKind::Settle, sequence, SETTLE_AFTER);
            arm(wake, registry, notify_tx, agent_id, timer);
            true
        }
        false => false,
    }
}

/// Settle a `swarm_state` from a launched coordinator: it now reports, its
/// held turn end (if any) becomes the note the wake decides, and a hold
/// arms the quiet report. Any other child's `swarm_state` is ignored.
pub(super) fn classify_swarm_state(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
    quiet_after: std::time::Duration,
) {
    let kind = value
        .get("wake")
        .and_then(|wake| WakeKind::deserialize(wake).ok())
        .unwrap_or(WakeKind::Unknown);
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let prompted = value.get("prompted").and_then(serde_json::Value::as_bool) == Some(true);
    let held = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(agent_id) else {
            return;
        };
        let wake = &mut entry.coordinator_wake;
        match wake.launched {
            true => {}
            false => return,
        }
        wake.reporting = true;
        wake.cancel_timer();
        if kind == WakeKind::Hold {
            let timer = (TimerKind::Quiet, sequence, quiet_after);
            arm(wake, registry, notify_tx, agent_id, timer);
        }
        wake.deferred.take()
    };
    if held.is_none() {
        return;
    }
    let state = |state| SubagentNotification::SwarmState {
        agent_id: String::new(),
        state,
    };
    let note = match settle_held_note(kind, prompted) {
        HeldNote::Silent => return,
        HeldNote::Ordinary => SubagentNotification::Completed {
            agent_id: String::new(),
        },
        HeldNote::Finished => state(SwarmNoteState::Finished { status }),
        HeldNote::Idle => state(SwarmNoteState::Idle { status }),
    };
    send(registry, notify_tx, agent_id, sequence, note);
}

/// Run `wake`'s one timer, replacing any it had: when it fires and is still
/// the one standing, a settle sends the held note as it was and a quiet
/// timer reports the hold.
fn arm(
    wake: &mut CoordinatorWake,
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    (kind, token, after): (TimerKind, u64, std::time::Duration),
) {
    wake.cancel_timer();
    let (Some(tx), Ok(runtime)) = (notify_tx.cloned(), tokio::runtime::Handle::try_current())
    else {
        return;
    };
    let (registry_for_timer, agent) = (registry.clone(), agent_id.to_owned());
    let task = runtime.spawn(async move {
        tokio::time::sleep(after).await;
        fire(&registry_for_timer, &tx, &agent, (kind, token, after));
    });
    wake.timer = Some(Timer {
        kind,
        token,
        handle: task.abort_handle(),
    });
}

fn fire(
    registry: &SubagentRegistry,
    tx: &NotificationTx,
    agent_id: &str,
    (kind, token, after): (TimerKind, u64, std::time::Duration),
) {
    let standing = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(wake) = entries.get_mut(agent_id).map(|e| &mut e.coordinator_wake) else {
            return;
        };
        let standing = wake
            .timer
            .take_if(|timer| timer.kind == kind && timer.token == token)
            .is_some();
        match (standing, kind) {
            (true, TimerKind::Settle) => wake.deferred.take().is_some(),
            (standing, _) => standing,
        }
    };
    let note = match (standing, kind) {
        (false, _) => return,
        (true, TimerKind::Settle) => SubagentNotification::Completed {
            agent_id: String::new(),
        },
        (true, TimerKind::Quiet) => SubagentNotification::SwarmState {
            agent_id: String::new(),
            state: SwarmNoteState::Quiet {
                minutes: after.as_secs() / 60,
            },
        },
    };
    send(registry, Some(tx), agent_id, token, note);
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
