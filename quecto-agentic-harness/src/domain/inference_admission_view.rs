//! Progress verdict derived from admission activity (#1679 P4): a process
//! whose attempts are queued at the authority is *waiting*, never quiet or
//! stalled. The verdict is a pure projection of [`AdmissionActivity`]; it
//! carries no lifecycle state and no presentation text.
use super::inference_admission::{AdmissionActivity, AdmissionPhase, CooldownState, GroupId};

/// Why the longest-waiting attempt is still queued, as far as this process
/// last learned from its own feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitCause {
    /// The group's capacity or queue is occupied; no throttle is known.
    Occupancy,
    /// The group cools down for `remaining_ms` more (0 once it has elapsed
    /// locally but the authority has not yet released the queue).
    Cooldown { remaining_ms: u64 },
    /// Throttled without a usable deadline.
    ThrottledIndefinitely,
    /// The authority marked the group unavailable.
    Unavailable,
}

/// The waiting verdict: how many attempts are queued, the longest wait and,
/// when a waiting attempt is visible, the group and cause of that wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingVerdict {
    pub waiting: usize,
    pub longest_wait_ms: u64,
    /// `None` when every waiting attempt is beyond the sample bound: the
    /// wait is then not attributed to any group rather than to a guess.
    pub attribution: Option<(GroupId, WaitCause)>,
}

fn cause_for(activity: &AdmissionActivity, group: &GroupId) -> WaitCause {
    match activity.groups.get(group).and_then(|g| g.cooldown) {
        Some(CooldownState::Until { until_ms }) => WaitCause::Cooldown {
            remaining_ms: until_ms.saturating_sub(activity.observed_at_ms),
        },
        Some(CooldownState::Unknown { .. }) => WaitCause::ThrottledIndefinitely,
        Some(CooldownState::Unavailable) => WaitCause::Unavailable,
        None => WaitCause::Occupancy,
    }
}

/// `Some` exactly when at least one attempt is waiting. The sampled attempts
/// are the oldest, so the longest wait among them is the longest overall;
/// when every waiting attempt is hidden by the sample bound the wait is
/// reported as unknown (0) and unattributed rather than invented.
pub fn waiting_verdict(activity: &AdmissionActivity) -> Option<WaitingVerdict> {
    if activity.waiting == 0 {
        return None;
    }
    let longest = activity
        .attempts
        .values()
        .filter(|attempt| matches!(attempt.phase, AdmissionPhase::Waiting { .. }))
        .max_by_key(|attempt| attempt.elapsed_ms);
    Some(WaitingVerdict {
        waiting: activity.waiting,
        longest_wait_ms: longest.map_or(0, |attempt| attempt.elapsed_ms),
        attribution: longest.map(|attempt| {
            let cause = cause_for(activity, &attempt.group);
            (attempt.group.clone(), cause)
        }),
    })
}

#[cfg(test)]
#[path = "inference_admission_view_tests.rs"]
mod tests;
