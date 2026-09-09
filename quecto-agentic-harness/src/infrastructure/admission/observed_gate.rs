//! Observation decorator around an admission gate (#1679 P4): records every
//! transition of the attempts this process owns without touching the attempt
//! transport, the permit semantics, or any lifecycle enum.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use crate::application::ports::{
    AdmissionObservation, AttemptAcquisition, AttemptAdmission, AttemptPermit,
};
use crate::domain::inference_admission::{
    AdmissionActivity, AdmissionPhase, AttemptObservation, CooldownState, Feedback, GroupActivity,
    GroupId, ThrottleFeedback,
};

#[derive(Debug, Clone)]
struct Live {
    alias: String,
    group: GroupId,
    phase: AdmissionPhase,
}

/// Receives a fresh view after every transition (never under the recorder lock).
pub type ActivityHook = Arc<dyn Fn(&AdmissionActivity) + Send + Sync>;

#[derive(Default)]
struct RecorderState {
    next: u64,
    revision: u64,
    hook: Option<ActivityHook>,
    /// Every live attempt (exact counts). Bounded in practice by the
    /// authority's per-group capacity plus queue capacity.
    live: BTreeMap<u64, Live>,
    completed: u64,
    refused: u64,
    cancelled: u64,
    abandoned: u64,
    groups: BTreeMap<GroupId, GroupActivity>,
}

/// Process-wide admission activity; clock is monotonic from construction.
pub struct AdmissionRecorder {
    start: Instant,
    state: Mutex<RecorderState>,
    /// Held across snapshot + hook so views reach the hook in revision order
    /// (never held together with `state`).
    notify: Mutex<()>,
}

impl std::fmt::Debug for AdmissionRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let revision = self.state.try_lock().map(|state| state.revision).ok();
        f.debug_struct("AdmissionRecorder")
            .field("revision", &revision)
            .finish_non_exhaustive()
    }
}

impl Default for AdmissionRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Keep at most `limit` bytes, never splitting a character.
fn truncate_bytes(reason: &str, limit: usize) -> String {
    if reason.len() <= limit {
        return reason.to_owned();
    }
    let mut end = limit;
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].to_owned()
}

fn merge_until(current: Option<CooldownState>, until_ms: u64) -> Option<CooldownState> {
    match current {
        Some(CooldownState::Unavailable) => Some(CooldownState::Unavailable),
        Some(CooldownState::Until { until_ms: existing }) => Some(CooldownState::Until {
            until_ms: existing.max(until_ms),
        }),
        _ => Some(CooldownState::Until { until_ms }),
    }
}

impl AdmissionRecorder {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            state: Mutex::new(RecorderState::default()),
            notify: Mutex::new(()),
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RecorderState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Install the transition hook (one per recorder; a later call replaces it).
    pub fn set_hook(&self, hook: ActivityHook) {
        self.lock().hook = Some(hook);
    }

    /// Apply one transition: `mutate` runs under the state lock and returns
    /// `Some` when the view changed; the revision is bumped in that same
    /// critical section, so a snapshot's revision exactly describes its
    /// contents. The hook then receives a fresh view outside the state lock.
    /// Deliveries are serialized behind `notify` (always taken first) so two
    /// transitions on different workers cannot reach the hook with their
    /// revisions swapped (clients do a simple replace).
    fn transition<R>(
        &self,
        mutate: impl FnOnce(&mut RecorderState, u64) -> Option<R>,
    ) -> Option<R> {
        let now = self.now_ms();
        let _ordered = self
            .notify
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (result, hook) = {
            let mut state = self.lock();
            let result = mutate(&mut state, now)?;
            state.revision = state.revision.wrapping_add(1);
            (result, state.hook.clone())
        };
        if let Some(hook) = hook {
            hook(&self.snapshot());
        }
        Some(result)
    }

    fn begin(&self, alias: &str, group: &GroupId) -> u64 {
        self.transition(|state, now| {
            state.next = state.next.wrapping_add(1);
            let id = state.next;
            state.live.insert(
                id,
                Live {
                    alias: alias.to_owned(),
                    group: group.clone(),
                    phase: AdmissionPhase::Waiting { since_ms: now },
                },
            );
            state.groups.entry(group.clone()).or_default();
            Some(id)
        })
        .expect("begin always transitions")
    }

    fn admitted(&self, id: u64) {
        self.transition(|state, now| {
            let live = state.live.get_mut(&id)?;
            live.phase = AdmissionPhase::Admitted { since_ms: now };
            Some(())
        });
    }

    fn refused(&self, id: u64, reason: &str) {
        self.transition(|state, _| {
            let live = state.live.remove(&id)?;
            state.refused = state.refused.saturating_add(1);
            state.groups.entry(live.group).or_default().last_refusal =
                Some(truncate_bytes(reason, AdmissionActivity::MAX_REFUSAL_BYTES));
            Some(())
        });
    }

    fn cancelled(&self, id: u64) {
        self.transition(|state, _| {
            state.live.remove(&id)?;
            state.cancelled = state.cancelled.saturating_add(1);
            Some(())
        });
    }

    fn abandoned(&self, id: u64) {
        self.transition(|state, _| {
            state.live.remove(&id)?;
            state.abandoned = state.abandoned.saturating_add(1);
            Some(())
        });
    }

    fn completed(&self, id: u64, maximum_ms: u64, feedback: Feedback) {
        self.transition(|state, now| {
            let live = state.live.remove(&id)?;
            state.completed = state.completed.saturating_add(1);
            let group = state.groups.entry(live.group).or_default();
            match feedback {
                // The authority marks the group unavailable for advice beyond
                // its maximum; mirror that instead of showing an expiring
                // cooldown.
                Feedback::Throttle { delay_ms } if delay_ms > maximum_ms => {
                    group.cooldown = Some(CooldownState::Unavailable);
                }
                Feedback::Throttle { delay_ms } => {
                    group.cooldown = merge_until(group.cooldown, now.saturating_add(delay_ms));
                }
                Feedback::Success => {
                    // A confirmed success ends an open-ended throttle; a dated
                    // one expires on its own.
                    if matches!(group.cooldown, Some(CooldownState::Unknown { .. })) {
                        group.cooldown = None;
                    }
                }
                Feedback::Failure => {}
            }
            Some(())
        });
    }

    /// Receipt advice for the attempt `id`, anchored at its grant: the
    /// authority's deadline is `receipt_ms + offset`, so locally it becomes
    /// `admitted_since + offset`. The authority judges advice against its
    /// maximum by the delay *remaining when the report arrives*; the same
    /// measure is used here (offset minus the time since the grant) so a long
    /// transport never turns acceptable advice into a false, sticky
    /// `Unavailable`. A local `Unavailable` clears only when the process
    /// re-negotiates after an operator reset, exactly like the authority's.
    fn cooldown(&self, id: u64, receipt_ms: u64, maximum_ms: u64, feedback: ThrottleFeedback) {
        self.transition(|state, now| {
            let live = state.live.get(&id).cloned()?;
            let anchor = match live.phase {
                AdmissionPhase::Admitted { since_ms } => since_ms,
                AdmissionPhase::Waiting { .. } => now,
            };
            let group = state.groups.entry(live.group).or_default();
            group.cooldown = match feedback {
                ThrottleFeedback::Until(deadline) => {
                    let offset = deadline.saturating_sub(receipt_ms);
                    let remaining = offset.saturating_sub(now.saturating_sub(anchor));
                    if remaining > maximum_ms {
                        Some(CooldownState::Unavailable)
                    } else {
                        merge_until(group.cooldown, anchor.saturating_add(offset))
                    }
                }
                // Known limitation: a no-hint throttle on top of an unexpired
                // dated cooldown keeps the dated one locally, while the
                // authority may extend it by its own fallback; the view never
                // claims a cooldown the authority lacks, it may only
                // under-report its length.
                ThrottleFeedback::NoHint { .. } => match group.cooldown {
                    Some(CooldownState::Until { until_ms }) if until_ms > now => group.cooldown,
                    Some(CooldownState::Unavailable) => group.cooldown,
                    _ => Some(CooldownState::Unknown { since_ms: now }),
                },
                ThrottleFeedback::Unavailable => Some(CooldownState::Unavailable),
            };
            Some(())
        });
    }
}

impl AdmissionObservation for AdmissionRecorder {
    fn snapshot(&self) -> AdmissionActivity {
        let now = self.now_ms();
        let state = self.lock();
        let (mut waiting, mut admitted) = (0usize, 0usize);
        let mut attempts = BTreeMap::new();
        for (id, live) in &state.live {
            let since = match live.phase {
                AdmissionPhase::Waiting { since_ms } => {
                    waiting += 1;
                    since_ms
                }
                AdmissionPhase::Admitted { since_ms } => {
                    admitted += 1;
                    since_ms
                }
            };
            // The sample keeps the oldest (longest-waiting) attempts.
            if attempts.len() < AdmissionActivity::MAX_LIVE_ATTEMPTS {
                attempts.insert(
                    *id,
                    AttemptObservation {
                        alias: live.alias.clone(),
                        group: live.group.clone(),
                        phase: live.phase,
                        elapsed_ms: now.saturating_sub(since),
                    },
                );
            }
        }
        let groups = state
            .groups
            .iter()
            .map(|(group, activity)| {
                let cooldown = match activity.cooldown {
                    Some(CooldownState::Until { until_ms }) if until_ms <= now => None,
                    other => other,
                };
                (
                    group.clone(),
                    GroupActivity {
                        cooldown,
                        last_refusal: activity.last_refusal.clone(),
                    },
                )
            })
            .collect();
        AdmissionActivity {
            waiting,
            admitted,
            completed: state.completed,
            refused: state.refused,
            cancelled: state.cancelled,
            abandoned: state.abandoned,
            groups,
            hidden: state.live.len().saturating_sub(attempts.len()),
            attempts,
            revision: state.revision,
            observed_at_ms: now,
        }
    }
}

/// `AttemptAdmission` that records around an inner gate.
#[derive(Debug)]
pub struct ObservedAdmission {
    inner: Arc<dyn AttemptAdmission>,
    alias: String,
    group: GroupId,
    recorder: Arc<AdmissionRecorder>,
}

impl ObservedAdmission {
    pub fn new(
        inner: Arc<dyn AttemptAdmission>,
        alias: &str,
        group: GroupId,
        recorder: Arc<AdmissionRecorder>,
    ) -> Self {
        Self {
            inner,
            alias: alias.to_owned(),
            group,
            recorder,
        }
    }
}

/// Marks a dropped wait as cancelled unless the acquire settled.
struct WaitGuard {
    recorder: Arc<AdmissionRecorder>,
    id: u64,
    settled: bool,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if !self.settled {
            self.recorder.cancelled(self.id);
        }
    }
}

impl AttemptAdmission for ObservedAdmission {
    fn acquire(&self) -> AttemptAcquisition<'_> {
        Box::pin(async move {
            let id = self.recorder.begin(&self.alias, &self.group);
            let mut guard = WaitGuard {
                recorder: self.recorder.clone(),
                id,
                settled: false,
            };
            match self.inner.acquire().await {
                Ok(permit) => {
                    guard.settled = true;
                    self.recorder.admitted(id);
                    Ok(Box::new(ObservedPermit {
                        inner: Some(permit),
                        recorder: self.recorder.clone(),
                        id,
                    }) as Box<dyn AttemptPermit>)
                }
                Err(error) => {
                    guard.settled = true;
                    self.recorder.refused(id, &error.to_string());
                    Err(error)
                }
            }
        })
    }
}

#[derive(Debug)]
pub struct ObservedPermit {
    inner: Option<Box<dyn AttemptPermit>>,
    recorder: Arc<AdmissionRecorder>,
    id: u64,
}

impl ObservedPermit {
    fn inner(&self) -> &dyn AttemptPermit {
        self.inner.as_deref().expect("permit present until finish")
    }
}

impl AttemptPermit for ObservedPermit {
    fn deadline_expired(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        self.inner().deadline_expired()
    }
    fn receipt_clock(&self) -> (u64, SystemTime) {
        self.inner().receipt_clock()
    }
    fn maximum_cooldown_ms(&self) -> u64 {
        self.inner().maximum_cooldown_ms()
    }
    fn throttle_without_hint(&mut self) {
        let (receipt_ms, _) = self.receipt_clock();
        let maximum = self.maximum_cooldown_ms();
        self.recorder.cooldown(
            self.id,
            receipt_ms,
            maximum,
            ThrottleFeedback::NoHint { jitter: 0 },
        );
        if let Some(inner) = self.inner.as_mut() {
            inner.throttle_without_hint();
        }
    }
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        let (receipt_ms, _) = self.receipt_clock();
        let maximum = self.maximum_cooldown_ms();
        self.recorder
            .cooldown(self.id, receipt_ms, maximum, feedback);
        if let Some(inner) = self.inner.as_mut() {
            inner.feedback(feedback);
        }
    }
    fn finish(mut self: Box<Self>, feedback: Feedback) {
        let maximum = self.maximum_cooldown_ms();
        self.recorder.completed(self.id, maximum, feedback);
        if let Some(inner) = self.inner.take() {
            inner.finish(feedback);
        }
    }
}

impl Drop for ObservedPermit {
    fn drop(&mut self) {
        // Drop is not release (ADR-0026): the authority keeps this occupancy
        // as uncertain, so locally it is an abandonment, not a cancellation.
        if self.inner.is_some() {
            self.recorder.abandoned(self.id);
        }
    }
}

#[cfg(test)]
#[path = "observed_gate_tests.rs"]
mod tests;
