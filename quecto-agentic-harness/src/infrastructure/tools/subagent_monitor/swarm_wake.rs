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
    SubagentStatus, SwarmNoteState,
};
use crate::domain::swarm::parent_wake::{HeldNote, StretchEnd, WakeKind, settle_held_note};
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
    /// A turn failed since the last state: the next one reports it once.
    failed: bool,
    /// The one timer running for it: settling a held note, or the quiet
    /// report of a hold.
    timer: Option<Timer>,
    /// A swarm worker this process launched as its run's coordinator
    /// (#2471): the board its plain turn ends are settled by.
    pub launcher_board: Option<LauncherBoard>,
    /// Its launcher sent it a prompt, steer or follow-up: its next turn end
    /// is the reply, owed whatever the board says.
    reply_owed: bool,
}

/// The run control a coordinator reads its board through.
#[derive(Clone)]
pub struct LauncherBoard(pub std::sync::Arc<dyn crate::application::swarm::ports::SwarmRunControl>);

impl std::fmt::Debug for LauncherBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LauncherBoard")
    }
}

/// The board a newly launched child's turn ends are settled by (#2471):
/// only a swarm worker launched by the container's creator (the coordinator
/// spawned with `coordinator: true`) gets one.
pub fn launcher_board_for(
    launches_swarm_worker: bool,
    container_creator: bool,
    board: Option<LauncherBoard>,
) -> Option<LauncherBoard> {
    let _ = (launches_swarm_worker, container_creator);
    drop(board);
    None
}

/// Record that `agent_id`'s launcher sent it a prompt, steer or follow-up.
pub fn mark_reply_owed(registry: &SubagentRegistry, agent_id: &str) {
    let _ = (registry, agent_id);
}

/// Settle a swarm worker's plain turn end at its coordinator (#2471):
/// `true` when it is handled here (dropped, or the run's stall reported
/// later), `false` to send the ordinary note.
pub(super) fn worker_turn_end(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
) -> bool {
    let _ = notify_tx;
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let _ = entries.get(agent_id).map(|e| e.coordinator_wake.reply_owed);
    false
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
    /// A new turn is activity: a held coordinator is not quiet, its held
    /// note waits for this turn's own end, and an earlier failed turn is no
    /// longer the outcome.
    pub(super) fn turn_started(&mut self) {
        self.failed = false;
        self.cancel_timer();
    }

    /// A failed turn is the stretch's outcome: its error note stands, and
    /// no held turn end is settled over it.
    pub(super) fn turn_failed(&mut self) {
        self.failed = true;
        self.deferred = None;
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
    // A state that names no status is sent as the ordinary note.
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .filter(|status| !status.is_empty())
        .map(str::to_owned);
    let prompted = value.get("prompted").and_then(serde_json::Value::as_bool) == Some(true);
    let (held, errored) = {
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
        let errored = std::mem::take(&mut wake.failed);
        wake.cancel_timer();
        if kind == WakeKind::Hold {
            let timer = (TimerKind::Quiet, sequence, quiet_after);
            arm(wake, registry, notify_tx, agent_id, timer);
        }
        (wake.deferred.take(), errored)
    };
    // A turn that failed since the last state is reported once even with no
    // held turn end: its own error note may have been a repeat the parent's
    // queue dropped.
    if held.is_none() && !errored {
        return;
    }
    let named = |state| -> Box<dyn FnOnce(String) -> SubagentNotification> {
        Box::new(move |agent_id| SubagentNotification::SwarmState { agent_id, state })
    };
    let end = StretchEnd { prompted, errored };
    let note = match (settle_held_note(kind, end), status) {
        (HeldNote::Silent, _) => return,
        (HeldNote::Finished, Some(status)) => named(SwarmNoteState::Finished { status }),
        (HeldNote::Idle, Some(status)) => named(SwarmNoteState::Idle { status }),
        (HeldNote::Paused, _) => named(SwarmNoteState::Paused),
        (HeldNote::Stopped, _) => named(SwarmNoteState::Stopped),
        (HeldNote::Ordinary, _) | (HeldNote::Finished | HeldNote::Idle, None) => {
            Box::new(|agent_id| SubagentNotification::Completed { agent_id })
        }
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

/// A standing timer's note, sent under a fresh sequence so it is never
/// mistaken for one already delivered, and only while the child is alive
/// and idle.
fn fire(
    registry: &SubagentRegistry,
    tx: &NotificationTx,
    agent_id: &str,
    (kind, token, after): (TimerKind, u64, std::time::Duration),
) {
    let sequence = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(agent_id) else {
            return;
        };
        let idle = matches!(entry.status, SubagentStatus::Idle | SubagentStatus::Error);
        let wake = &mut entry.coordinator_wake;
        let standing = wake
            .timer
            .take_if(|timer| timer.kind == kind && timer.token == token)
            .is_some();
        let due = match kind {
            TimerKind::Settle => standing && wake.deferred.take().is_some(),
            TimerKind::Quiet => standing,
        };
        match due && idle {
            true => super::super::subagent_monitor_registry::next_sequence(&mut entries, agent_id),
            false => None,
        }
    };
    let Some(sequence) = sequence else {
        return;
    };
    let note: Box<dyn FnOnce(String) -> SubagentNotification> = match kind {
        TimerKind::Settle => Box::new(|agent_id| SubagentNotification::Completed { agent_id }),
        TimerKind::Quiet => Box::new(move |agent_id| SubagentNotification::SwarmState {
            agent_id,
            state: SwarmNoteState::Quiet {
                minutes: after.as_secs() / 60,
            },
        }),
    };
    send(registry, Some(tx), agent_id, sequence, note);
}

/// Send the note `note` builds for `agent_id`'s display label, as its
/// identity.
fn send(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    note: Box<dyn FnOnce(String) -> SubagentNotification>,
) {
    let Some(tx) = notify_tx else { return };
    let note = note(super::notification_display_label(registry, agent_id));
    let uuid = super::notification_agent_uuid(registry, agent_id);
    let _ = tx.try_send(SequencedSubagentNotification::new_for_agent(
        sequence, note, uuid,
    ));
}
