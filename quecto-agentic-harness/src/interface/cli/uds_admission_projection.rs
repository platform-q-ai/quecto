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
    execution_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .set_admission_source(process.observation());
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
