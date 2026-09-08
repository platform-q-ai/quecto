//! Uncertainty, durability and epoch transitions for [`AdmissionPolicy`].
//!
//! Nothing here reclaims capacity on elapsed time: occupancy leaves only through
//! a verified terminal transition or an explicit operator reset.
use super::*;

impl AdmissionPolicy {
    /// Highest acquire sequence accepted for `scope`; a reconnecting client
    /// continues above it so replay fencing keeps holding.
    pub fn high_water(&self, scope: ScopeId) -> Result<u64, AdmissionError> {
        Ok(self.scope(scope)?.high_water)
    }

    /// The client owning `scope` vanished: queued work can never dispatch, and
    /// active work becomes uncertain occupancy that quarantines its group.
    pub fn abandon(&mut self, scope: ScopeId, now: u64) -> Result<AbandonReport, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let mut report = AbandonReport::default();
        let owned: Vec<(RequestId, RequestState, GroupId)> = self
            .requests
            .iter()
            .filter(|(id, _)| id.scope == scope)
            .map(|(id, r)| (*id, r.state, r.group.clone()))
            .collect();
        for (id, state, group) in owned {
            match state {
                RequestState::Queued { .. } => {
                    self.terminal(id, TerminalOutcome::Cancelled, None);
                    report.cancelled += 1;
                }
                RequestState::Active { .. } => {
                    let group = self.groups.get_mut(&group).expect("configured group");
                    if group.uncertain.insert(id) {
                        report.uncertain += 1;
                    }
                }
                RequestState::Terminal(_) => {}
            }
        }
        Ok(report)
    }

    /// Terminate a grant the authority could not make durable. The client never
    /// learned of it, so no transport exists; the request-start charge remains.
    pub fn withdraw(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<(), AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let id = RequestId { scope, sequence };
        if !matches!(
            self.request(scope, sequence)?.state,
            RequestState::Active { .. }
        ) {
            return Err(AdmissionError::Conflict);
        }
        self.terminal(id, TerminalOutcome::Cancelled, None);
        Ok(())
    }

    /// Earliest future instant at which a dispatch decision may change without
    /// any client action: pacing/cooldown release, queue expiry or an attempt
    /// deadline. `None` means only client traffic can change the schedule.
    pub fn next_wake(&mut self, now: u64) -> Result<Option<u64>, AdmissionError> {
        self.tick(now)?;
        let mut wake: Option<u64> = None;
        let mut consider = |instant: u64| {
            if instant > now {
                wake = Some(wake.map_or(instant, |w| w.min(instant)));
            }
        };
        for (id, group) in &self.groups {
            let policy = &self.config.groups[id];
            let mut queued = 0usize;
            let mut active = group.orphans.len();
            for (request_id, request) in self.requests.iter().filter(|(_, r)| &r.group == id) {
                match request.state {
                    RequestState::Queued { deadline } => {
                        queued += 1;
                        consider(deadline);
                    }
                    RequestState::Active {
                        deadline,
                        cancellation_required,
                        ..
                    } => {
                        active += 1;
                        if !cancellation_required && !group.uncertain.contains(request_id) {
                            consider(deadline);
                        }
                    }
                    RequestState::Terminal(_) => {}
                }
            }
            let blocked = group.unavailable
                || !group.uncertain.is_empty()
                || !group.orphans.is_empty()
                || active >= policy.capacity;
            if queued > 0 && !blocked {
                consider(group.next_start.max(group.cooldown));
            }
        }
        Ok(wake)
    }

    /// Durable accounting relative to `now`. Contains no capability material.
    pub fn ledger(&mut self, now: u64) -> Result<AdmissionLedger, AdmissionError> {
        self.tick(now)?;
        let mut outstanding: Vec<OutstandingAttempt> = self
            .requests
            .iter()
            .filter(|(_, r)| matches!(r.state, RequestState::Active { .. }))
            .map(|(id, r)| OutstandingAttempt {
                group: r.group.clone(),
                scope: id.scope.serial,
                sequence: id.sequence,
            })
            .collect();
        let mut groups = BTreeMap::new();
        for (id, group) in &self.groups {
            outstanding.extend(group.orphans.iter().cloned());
            groups.insert(
                id.clone(),
                LedgerGroup {
                    cooldown_remaining_ms: group.cooldown.saturating_sub(now),
                    pacing_remaining_ms: group.next_start.saturating_sub(now),
                    unavailable: group.unavailable,
                },
            );
        }
        Ok(AdmissionLedger {
            epoch: self.epoch,
            outstanding,
            groups,
        })
    }

    /// Rehydrate a restarted authority at the ledger's epoch. Every outstanding
    /// attempt becomes orphaned occupancy: its scope no longer exists, so only
    /// an explicit reset can release it.
    pub fn restore(
        config: AdmissionConfig,
        ledger: &AdmissionLedger,
        now: u64,
    ) -> Result<Self, AdmissionError> {
        let mut policy = Self::new(ledger.epoch, config)?;
        policy.now = now;
        for (id, durable) in &ledger.groups {
            let group = policy
                .groups
                .get_mut(id)
                .ok_or(AdmissionError::UnknownGroup)?;
            group.cooldown = now
                .checked_add(durable.cooldown_remaining_ms)
                .ok_or(AdmissionError::Unavailable)?;
            group.next_start = now
                .checked_add(durable.pacing_remaining_ms)
                .ok_or(AdmissionError::Unavailable)?;
            group.unavailable = durable.unavailable;
        }
        for attempt in &ledger.outstanding {
            policy
                .groups
                .get_mut(&attempt.group)
                .ok_or(AdmissionError::UnknownGroup)?
                .orphans
                .push(attempt.clone());
        }
        Ok(policy)
    }

    /// Operator-acknowledged successor epoch: uncertainty and orphans are
    /// relinquished (old remote work is no longer claimed bounded), scopes are
    /// revoked, and provider-facing cooldown/pacing deadlines are preserved.
    pub fn reset(&mut self, now: u64) -> Result<u64, AdmissionError> {
        self.tick(now)?;
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or(AdmissionError::ScopeLimit)?;
        self.epoch = epoch;
        self.scopes.clear();
        self.requests.clear();
        self.terminals.clear();
        for group in self.groups.values_mut() {
            group.uncertain.clear();
            group.orphans.clear();
            group.unavailable = false;
            group.interactive_streak = 0;
            group.contested_pacing_streak = 0;
            group.last_root = [None; 2];
            group.last_agent.clear();
        }
        Ok(epoch)
    }
}
