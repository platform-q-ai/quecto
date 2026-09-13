//! Port fakes for the selected-termination and compensation use cases
//! (#1936). Test-only: an in-memory registry of rows with the same claim
//! machinery the production adapter provides, a recording fallback, and a
//! recording compensation.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::application::subagents::ports::{
    Compensated, CompensationObservation, ConclusionBudget, DelegatedAgentRegistry,
    OwnedChildTermination, PortFuture, ProtocolAttempt, ResolutionError, StoppingClaimError,
    TeardownCompensation, TerminalClaim, TerminationCause, TerminationConclusion,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::DelegatedAgentIdentity;

use super::teardown_fakes::identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Live,
    Stopping(TerminationCause),
    Compensating,
    Compensated,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub identity: DelegatedAgentIdentity,
    pub display: String,
    pub phase: Phase,
    pub holds_process: bool,
    /// `None` for a row that is not a delegated agent (a fixture, a stub).
    pub delegated: bool,
}

/// In-memory rows keyed by uuid, with a notification so `await_compensated`
/// can wait instead of polling.
#[derive(Default)]
pub struct FakeRegistry {
    pub rows: Mutex<BTreeMap<String, Row>>,
    pub changed: tokio::sync::Notify,
    /// Claims in the order they were taken, for ordering assertions.
    pub trace: Mutex<Vec<String>>,
    /// When set, `await_compensated` reports a timeout instead of waiting.
    pub time_out_waits: Mutex<bool>,
}

impl FakeRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn with_row(self: Arc<Self>, uuid: &str, generation: u64, display: &str) -> Arc<Self> {
        self.rows.lock().unwrap().insert(
            uuid.to_owned(),
            Row {
                identity: identity(uuid, generation),
                display: display.to_owned(),
                phase: Phase::Live,
                holds_process: false,
                delegated: true,
            },
        );
        self
    }

    pub fn holding_process(self: Arc<Self>, uuid: &str) -> Arc<Self> {
        self.rows
            .lock()
            .unwrap()
            .get_mut(uuid)
            .unwrap()
            .holds_process = true;
        self
    }

    pub fn not_delegated(self: Arc<Self>, uuid: &str) -> Arc<Self> {
        self.rows.lock().unwrap().get_mut(uuid).unwrap().delegated = false;
        self
    }

    pub fn phase(&self, uuid: &str) -> Phase {
        self.rows.lock().unwrap()[uuid].phase
    }

    /// A compensated row's process has been reaped: nothing is retained.
    pub fn set_phase(&self, uuid: &str, phase: Phase) {
        let mut rows = self.rows.lock().unwrap();
        let row = rows.get_mut(uuid).unwrap();
        row.phase = phase;
        if phase == Phase::Compensated {
            row.holds_process = false;
        }
        drop(rows);
        self.changed.notify_waiters();
    }

    pub fn trace(&self) -> Vec<String> {
        self.trace.lock().unwrap().clone()
    }

    fn record(&self, event: impl Into<String>) {
        self.trace.lock().unwrap().push(event.into());
    }
}

impl DelegatedAgentRegistry for FakeRegistry {
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError> {
        let rows = self.rows.lock().unwrap();
        let row = match rows.get(reference) {
            Some(row) => row,
            None => {
                let mut matches = rows.values().filter(|row| {
                    row.display == reference && !matches!(row.phase, Phase::Compensated)
                });
                let first = matches.next().ok_or(ResolutionError::Unknown)?;
                if matches.next().is_some() {
                    return Err(ResolutionError::Ambiguous);
                }
                first
            }
        };
        if matches!(row.phase, Phase::Compensated | Phase::Compensating) {
            return Err(ResolutionError::Exited);
        }
        if !row.delegated {
            return Err(ResolutionError::NotDelegated);
        }
        Ok(row.identity.clone())
    }

    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .get_mut(target.uuid.as_str())
            .filter(|row| row.identity.generation == target.generation)
            .ok_or(StoppingClaimError::Unknown)?;
        match row.phase {
            Phase::Live => {
                row.phase = Phase::Stopping(cause);
                self.record(format!("claim-stopping {}", target.uuid));
                Ok(())
            }
            Phase::Stopping(_) => Err(StoppingClaimError::AlreadyStopping),
            Phase::Compensating | Phase::Compensated => Err(StoppingClaimError::Exited),
        }
    }

    fn release_stopping(&self, target: &DelegatedAgentIdentity) {
        let mut rows = self.rows.lock().unwrap();
        if let Some(row) = rows.get_mut(target.uuid.as_str()) {
            if matches!(row.phase, Phase::Stopping(_)) {
                row.phase = Phase::Live;
                self.record(format!("release-stopping {}", target.uuid));
            }
        }
    }

    fn claim_terminal(&self, target: &DelegatedAgentIdentity) -> TerminalClaim {
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows.get_mut(target.uuid.as_str()) else {
            return TerminalClaim::AlreadyClaimed;
        };
        match row.phase {
            Phase::Live | Phase::Stopping(_) => {
                row.phase = Phase::Compensating;
                self.record(format!("claim-terminal {}", target.uuid));
                TerminalClaim::Claimed
            }
            Phase::Compensating | Phase::Compensated => TerminalClaim::AlreadyClaimed,
        }
    }

    fn holds_process(&self, target: &DelegatedAgentIdentity) -> bool {
        self.rows
            .lock()
            .unwrap()
            .get(target.uuid.as_str())
            .is_some_and(|row| row.holds_process)
    }

    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool {
        self.rows
            .lock()
            .unwrap()
            .get(target.uuid.as_str())
            .is_some_and(|row| matches!(row.phase, Phase::Compensating | Phase::Compensated))
    }

    fn await_compensated<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation> {
        Box::pin(async move {
            self.record(format!("await-compensated {}", target.uuid));
            loop {
                let notified = self.changed.notified();
                {
                    let rows = self.rows.lock().unwrap();
                    let Some(row) = rows.get(target.uuid.as_str()) else {
                        return CompensationObservation::Unknown;
                    };
                    if row.phase == Phase::Compensated {
                        return CompensationObservation::Compensated;
                    }
                }
                if *self.time_out_waits.lock().unwrap() {
                    return CompensationObservation::TimedOut;
                }
                notified.await;
            }
        })
    }
}

/// A fallback that answers with a scripted conclusion and records what it
/// was handed, so a test can prove the protocol attempt reached it and that
/// a graceful outcome never consulted it for a signal.
pub struct FakeTermination {
    pub conclusion: Mutex<TerminationConclusion>,
    pub calls: Mutex<Vec<(DelegatedAgentIdentity, ProtocolAttempt, ConclusionBudget)>>,
    pub registry: Arc<FakeRegistry>,
    /// Mark the row compensated by "the reaper" while concluding, to model
    /// the exit path racing the kill.
    pub reaper_compensates: Mutex<bool>,
}

impl FakeTermination {
    pub fn new(registry: Arc<FakeRegistry>, conclusion: TerminationConclusion) -> Arc<Self> {
        Arc::new(Self {
            conclusion: Mutex::new(conclusion),
            calls: Mutex::new(Vec::new()),
            registry,
            reaper_compensates: Mutex::new(false),
        })
    }

    pub fn calls(&self) -> Vec<(DelegatedAgentIdentity, ProtocolAttempt, ConclusionBudget)> {
        self.calls.lock().unwrap().clone()
    }
}

impl OwnedChildTermination for FakeTermination {
    fn conclude<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
        budget: ConclusionBudget,
    ) -> PortFuture<'a, TerminationConclusion> {
        self.calls
            .lock()
            .unwrap()
            .push((child.clone(), attempt, budget));
        let conclusion = self.conclusion.lock().unwrap().clone();
        let reaper = *self.reaper_compensates.lock().unwrap();
        Box::pin(async move {
            if reaper {
                // The reaper observed the exit first and ran the compensation.
                assert_eq!(
                    self.registry.claim_terminal(child),
                    TerminalClaim::Claimed,
                    "the reaper claims while the kill is still concluding"
                );
                self.registry
                    .record(format!("reaper-compensated {}", child.uuid));
                self.registry
                    .set_phase(child.uuid.as_str(), Phase::Compensated);
            }
            conclusion
        })
    }
}

#[derive(Default)]
pub struct FakeCompensation {
    pub calls: Mutex<Vec<(DelegatedAgentIdentity, TerminationCause)>>,
    pub registry: Mutex<Option<Arc<FakeRegistry>>>,
    /// Descendants reported as removed together with each target.
    pub descendants: Mutex<Vec<AgentUuid>>,
}

impl FakeCompensation {
    pub fn new(registry: Arc<FakeRegistry>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            registry: Mutex::new(Some(registry)),
            descendants: Mutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> Vec<(DelegatedAgentIdentity, TerminationCause)> {
        self.calls.lock().unwrap().clone()
    }
}

impl TeardownCompensation for FakeCompensation {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated> {
        self.calls.lock().unwrap().push((target.clone(), cause));
        Box::pin(async move {
            let mut removed = vec![target.uuid.clone()];
            removed.extend(self.descendants.lock().unwrap().iter().cloned());
            if let Some(registry) = self.registry.lock().unwrap().as_ref() {
                registry.record(format!("compensated {}", target.uuid));
                registry.set_phase(target.uuid.as_str(), Phase::Compensated);
            }
            Compensated { removed }
        })
    }
}
