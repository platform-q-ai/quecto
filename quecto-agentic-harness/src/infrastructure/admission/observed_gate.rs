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
    AdmissionActivity, AdmissionPhase, AttemptObservation, Feedback, GroupId, ThrottleFeedback,
};

#[derive(Debug, Default)]
struct RecorderState {
    next: u64,
    live: BTreeMap<u64, AttemptObservation>,
    completed: u64,
    refused: u64,
    cancelled: u64,
    cooldown_until_ms: Option<u64>,
    last_refusal: Option<String>,
}

/// Process-wide admission activity; clock is monotonic from construction.
#[derive(Debug)]
pub struct AdmissionRecorder {
    start: Instant,
    state: Mutex<RecorderState>,
}

impl Default for AdmissionRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl AdmissionRecorder {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            state: Mutex::new(RecorderState::default()),
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

    fn begin(&self, alias: &str, group: &GroupId) -> u64 {
        let now = self.now_ms();
        let mut state = self.lock();
        state.next = state.next.wrapping_add(1);
        let id = state.next;
        // Bounded live view: a leaked attempt can never grow it unboundedly.
        while state.live.len() >= AdmissionActivity::MAX_LIVE_ATTEMPTS {
            let Some(oldest) = state.live.keys().next().copied() else {
                break;
            };
            state.live.remove(&oldest);
        }
        state.live.insert(
            id,
            AttemptObservation {
                alias: alias.to_owned(),
                group: group.clone(),
                phase: AdmissionPhase::Waiting { since_ms: now },
                elapsed_ms: 0,
            },
        );
        id
    }

    fn admitted(&self, id: u64) {
        let now = self.now_ms();
        if let Some(attempt) = self.lock().live.get_mut(&id) {
            attempt.phase = AdmissionPhase::Admitted { since_ms: now };
        }
    }

    fn refused(&self, id: u64, reason: &str) {
        let mut state = self.lock();
        state.live.remove(&id);
        state.refused = state.refused.saturating_add(1);
        state.last_refusal = Some(reason.chars().take(200).collect());
    }

    fn cancelled(&self, id: u64) {
        let mut state = self.lock();
        if state.live.remove(&id).is_some() {
            state.cancelled = state.cancelled.saturating_add(1);
        }
    }

    fn completed(&self, id: u64, feedback: Feedback) {
        let now = self.now_ms();
        let mut state = self.lock();
        state.live.remove(&id);
        state.completed = state.completed.saturating_add(1);
        if let Feedback::Throttle { delay_ms } = feedback {
            let until = now.saturating_add(delay_ms);
            state.cooldown_until_ms = Some(state.cooldown_until_ms.map_or(until, |c| c.max(until)));
        }
    }

    fn cooldown(&self, receipt_ms: u64, feedback: ThrottleFeedback) {
        // `Until` deadlines are on the authority's clock; translate through the
        // permit's receipt so the local view stays comparable with `observed_at`.
        let now = self.now_ms();
        let until = match feedback {
            ThrottleFeedback::Until(deadline) => {
                now.saturating_add(deadline.saturating_sub(receipt_ms))
            }
            ThrottleFeedback::NoHint { .. } | ThrottleFeedback::Unavailable => return,
        };
        let mut state = self.lock();
        state.cooldown_until_ms = Some(state.cooldown_until_ms.map_or(until, |c| c.max(until)));
    }
}

impl AdmissionObservation for AdmissionRecorder {
    fn snapshot(&self) -> AdmissionActivity {
        let now = self.now_ms();
        let state = self.lock();
        let mut attempts = BTreeMap::new();
        let (mut waiting, mut admitted) = (0usize, 0usize);
        for (id, attempt) in &state.live {
            let since = match attempt.phase {
                AdmissionPhase::Waiting { since_ms } => {
                    waiting += 1;
                    since_ms
                }
                AdmissionPhase::Admitted { since_ms } => {
                    admitted += 1;
                    since_ms
                }
            };
            let mut view = attempt.clone();
            view.elapsed_ms = now.saturating_sub(since);
            attempts.insert(*id, view);
        }
        AdmissionActivity {
            waiting,
            admitted,
            completed: state.completed,
            refused: state.refused,
            cancelled: state.cancelled,
            cooldown_until_ms: state.cooldown_until_ms.filter(|until| *until > now),
            last_refusal: state.last_refusal.clone(),
            attempts,
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
        if let Some(inner) = self.inner.as_mut() {
            inner.throttle_without_hint();
        }
    }
    fn feedback(&mut self, feedback: ThrottleFeedback) {
        let (receipt_ms, _) = self.receipt_clock();
        self.recorder.cooldown(receipt_ms, feedback);
        if let Some(inner) = self.inner.as_mut() {
            inner.feedback(feedback);
        }
    }
    fn finish(mut self: Box<Self>, feedback: Feedback) {
        self.recorder.completed(self.id, feedback);
        if let Some(inner) = self.inner.take() {
            inner.finish(feedback);
        }
    }
}

impl Drop for ObservedPermit {
    fn drop(&mut self) {
        // Drop is not release (ADR-0026); the attempt is no longer this
        // process's to report, so it leaves the live view as cancelled.
        if self.inner.is_some() {
            self.recorder.cancelled(self.id);
        }
    }
}

#[cfg(test)]
#[path = "observed_gate_tests.rs"]
mod tests;
