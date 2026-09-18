//! Wire projection of admission activity (#1679 P4, ADR-0024): a bounded,
//! fresh `admission` object beside the execution phase, plus the `waiting`
//! progress verdict. Admission never becomes a lifecycle `state` value.

use crate::domain::inference_admission::{AdmissionActivity, CooldownState};
use crate::domain::inference_admission_view::{WaitCause, waiting_verdict};

pub use crate::domain::state_snapshot::{
    AdmissionCounters, AdmissionSnapshot, CooldownSnapshot, GroupSnapshot,
};

fn seconds(ms: u64) -> u64 {
    ms / 1_000
}

fn cooldown_snapshot(cooldown: CooldownState, observed_at_ms: u64) -> CooldownSnapshot {
    match cooldown {
        CooldownState::Until { until_ms } => CooldownSnapshot {
            state: "until".into(),
            remaining_seconds: Some(seconds(until_ms.saturating_sub(observed_at_ms))),
        },
        CooldownState::Unknown { .. } => CooldownSnapshot {
            state: "unknown".into(),
            remaining_seconds: None,
        },
        CooldownState::Unavailable => CooldownSnapshot {
            state: "unavailable".into(),
            remaining_seconds: None,
        },
    }
}

pub(crate) fn project(activity: &AdmissionActivity) -> AdmissionSnapshot {
    AdmissionSnapshot {
        waiting: activity.waiting,
        admitted: activity.admitted,
        // Omitted when nothing waits or every waiting attempt is beyond the
        // sample: a client must not read "unknown" as "under a second".
        longest_wait_seconds: waiting_verdict(activity)
            .filter(|verdict| verdict.attribution.is_some())
            .map(|verdict| seconds(verdict.longest_wait_ms)),
        groups: activity
            .groups
            .iter()
            .map(|(group, view)| GroupSnapshot {
                group: group.as_str().to_owned(),
                cooldown: view
                    .cooldown
                    .map(|cooldown| cooldown_snapshot(cooldown, activity.observed_at_ms)),
                last_refusal: view.last_refusal.clone(),
            })
            .collect(),
        counters: AdmissionCounters {
            completed: activity.completed,
            refused: activity.refused,
            cancelled: activity.cancelled,
            abandoned: activity.abandoned,
        },
        hidden: activity.hidden,
        revision: activity.revision,
        // Authority-level facts are folded in by the execution-state snapshot
        // (which holds the process binding); the pushed event carries only the
        // bounded activity.
        directory: None,
        epoch: None,
        connected: None,
        authority_status: None,
    }
}

/// The authority-level view a `get_state` projection carries beside the
/// bounded activity (#2024 S3): where the authority is, which epoch this
/// process's capability came from, and whether it is reachable.
#[derive(Debug, Clone)]
pub(crate) struct AuthorityView {
    pub directory: String,
    pub epoch: u64,
    pub connected: bool,
    /// Whether this process can re-register on its own after a loss (a root
    /// can; a child depends on its parent).
    pub can_reconnect: bool,
}

impl AuthorityView {
    /// `connected` when the link is open, `reconnecting` when a root can
    /// re-register on its own, `unavailable` otherwise (a child whose parent
    /// authority is gone).
    pub(crate) fn status(&self) -> &'static str {
        if self.connected {
            "connected"
        } else if self.can_reconnect {
            "reconnecting"
        } else {
            "unavailable"
        }
    }

    /// Fold this authority view into an admission snapshot.
    pub(crate) fn apply(&self, snapshot: &mut AdmissionSnapshot) {
        snapshot.directory = Some(self.directory.clone());
        snapshot.epoch = Some(self.epoch);
        snapshot.connected = Some(self.connected);
        snapshot.authority_status = Some(self.status().to_string());
    }
}

/// The `waiting` progress verdict with its evidence, when attempts are queued.
pub(crate) fn waiting_progress(activity: &AdmissionActivity) -> Option<(&'static str, String)> {
    let verdict = waiting_verdict(activity)?;
    let noun = if verdict.waiting == 1 {
        "inference attempt"
    } else {
        "inference attempts"
    };
    let Some((group, cause)) = verdict.attribution else {
        return Some((
            "waiting",
            format!(
                "{} {noun} waiting for admission beyond the sampled attempts",
                verdict.waiting
            ),
        ));
    };
    let cause = match cause {
        WaitCause::Occupancy => String::new(),
        WaitCause::Cooldown { remaining_ms } => {
            format!("; cooldown {}s remaining", seconds(remaining_ms))
        }
        WaitCause::ThrottledIndefinitely => "; throttled without a deadline".to_owned(),
        WaitCause::Unavailable => "; group marked unavailable".to_owned(),
    };
    Some((
        "waiting",
        format!(
            "{} {noun} waiting for admission in group {} for {}s{cause}",
            verdict.waiting,
            group.as_str(),
            seconds(verdict.longest_wait_ms)
        ),
    ))
}

/// Wire a process admission binding into the socket loop: its view rides on
/// `get_state` and every transition is pushed. The hook keeps a sender alive
/// for the life of the process-wide binding; a later loop replaces it.
pub(crate) fn attach_process_admission(
    execution_state: &super::uds_execution_state::ExecutionStateHandle,
    process: &crate::infrastructure::admission::ProcessAdmission,
    broadcast_tx: &tokio::sync::broadcast::Sender<String>,
) {
    {
        let mut state = execution_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.set_admission_source(process.observation());
        // Directory/epoch are fixed for the binding; the connection is probed
        // live so health reflects a broker that has since gone away. A root can
        // re-register on its own; a child depends on its parent (#2024 S3).
        let connection = process.connection().clone();
        let can_reconnect = process.kind() == crate::infrastructure::admission::BindingKind::Root;
        state.set_admission_authority(super::uds_execution_state::AuthorityProbe::new(
            process.directory().display().to_string(),
            process.epoch(),
            can_reconnect,
            std::sync::Arc::new(move || connection.is_open()),
        ));
    }
    process.on_transition(admission_event_hook(broadcast_tx.clone()));
}

/// Hook that broadcasts one `admission_state_changed` event per transition.
pub(crate) fn admission_event_hook(
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
) -> crate::infrastructure::admission::ActivityHook {
    std::sync::Arc::new(move |activity: &AdmissionActivity| {
        let event = super::protocol::AgentEvent::AdmissionStateChanged {
            admission: project(activity),
        };
        if let Ok(line) = serde_json::to_string(&event) {
            // No receiver means no connected client; nothing to deliver.
            let _ = broadcast_tx.send(line);
        }
    })
}
