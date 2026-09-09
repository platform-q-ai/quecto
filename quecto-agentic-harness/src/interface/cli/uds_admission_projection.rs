//! Wire projection of admission activity (#1679 P4, ADR-0024): a bounded,
//! fresh `admission` object beside the execution phase, plus the `waiting`
//! progress verdict. Admission never becomes a lifecycle `state` value.
use serde::{Deserialize, Serialize};

use crate::domain::inference_admission::{AdmissionActivity, CooldownState};
use crate::domain::inference_admission_view::{WaitCause, waiting_verdict};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionSnapshot {
    pub waiting: usize,
    pub admitted: usize,
    /// Longest wait among the visible waiting attempts; absent when nothing waits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longest_wait_seconds: Option<u64>,
    pub groups: Vec<GroupSnapshot>,
    pub counters: AdmissionCounters,
    /// Waiting or admitted attempts beyond the bounded sample.
    pub hidden: usize,
    /// Advances on every transition (a delta cursor for `admission_state_changed`).
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionCounters {
    pub completed: u64,
    pub refused: u64,
    pub cancelled: u64,
    pub abandoned: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSnapshot {
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<CooldownSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CooldownSnapshot {
    /// `until`, `unknown` or `unavailable`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_seconds: Option<u64>,
}

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
        longest_wait_seconds: waiting_verdict(activity).map(|v| seconds(v.longest_wait_ms)),
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
    let cause = match verdict.cause {
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
            verdict.group.as_str(),
            seconds(verdict.longest_wait_ms)
        ),
    ))
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
