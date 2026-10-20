//! Uncertainty, durability and epoch transitions for [`AdmissionPolicy`].
//!
//! Occupancy leaves through a verified terminal transition, an explicit
//! operator reset, or (#2664) an uncertain lease reaching its bound: the
//! attempt's deadline plus [`UNCERTAIN_LEASE_MARGIN_MS`]. Nothing else
//! reclaims capacity on elapsed time.
use super::*;

/// The instant an uncertain lease on an attempt with `deadline` clears.
fn lease_release(deadline: u64) -> u64 {
    deadline.saturating_add(UNCERTAIN_LEASE_MARGIN_MS)
}

impl AdmissionPolicy {
    /// Clear every uncertain lease and orphan whose bound has passed (#2664).
    /// An uncertain attempt becomes terminal (its late completion is still
    /// acknowledged); an orphan has no request, so it is dropped.
    pub(super) fn clear_elapsed_leases(&mut self, now: u64) {
        let cleared: Vec<RequestId> = self
            .groups
            .values()
            .flat_map(|group| group.uncertain.iter().copied())
            .filter(|id| match self.requests.get(id).map(|r| r.state) {
                Some(RequestState::Active { deadline, .. }) => lease_release(deadline) <= now,
                other => {
                    debug_assert!(false, "an uncertain lease is an active request: {other:?}");
                    false
                }
            })
            .collect();
        for id in cleared {
            self.terminal(id, TerminalOutcome::Cancelled, None);
            self.requests
                .get_mut(&id)
                .expect("a terminal request is retained")
                .lease_cleared = true;
        }
        for group in self.groups.values_mut() {
            group.orphans.retain(|orphan| orphan.release_at > now);
        }
    }

    /// The uncertain leases holding `group`, and when the latest clears;
    /// `None` when nothing uncertain holds it.
    pub(super) fn hold_of(&self, group: &GroupId) -> Option<(usize, u64)> {
        let held = &self.groups[group];
        let uncertain =
            held.uncertain
                .iter()
                .filter_map(|id| match self.requests.get(id).map(|r| r.state) {
                    Some(RequestState::Active { deadline, .. }) => Some(lease_release(deadline)),
                    _ => None,
                });
        let orphans = held.orphans.iter().map(|orphan| orphan.release_at);
        let clears_at = uncertain.chain(orphans).max()?;
        Some((held.uncertain.len() + held.orphans.len(), clears_at))
    }

    /// Why the queued wait `sequence` of `scope` expired, when it expired
    /// behind uncertain leases (#2664); `None` for any other request.
    pub fn held(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<Option<UncertainHold>, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let request = self.request(scope, sequence)?;
        Ok(request.held.map(|(leases, clears_at)| UncertainHold {
            group: request.group.clone(),
            leases,
            clears_in_ms: clears_at.saturating_sub(now),
        }))
    }

    /// Highest acquire sequence accepted for `scope`; a reconnecting client
    /// continues above it so replay fencing keeps holding.
    pub fn high_water(&self, scope: ScopeId) -> Result<u64, AdmissionError> {
        Ok(self.scope(scope)?.high_water)
    }

    /// The client owning `scope` vanished: queued work can never dispatch, and
    /// active work becomes uncertain occupancy that quarantines its group
    /// until it is completed, reset, or its lease clears (#2664).
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
                    report.active += 1;
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
            for orphan in &group.orphans {
                consider(orphan.release_at);
            }
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
                        if group.uncertain.contains(request_id) {
                            consider(lease_release(deadline));
                        } else if !cancellation_required {
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
            .filter_map(|(id, r)| match r.state {
                RequestState::Active { deadline, .. } => Some(OutstandingAttempt {
                    group: r.group.clone(),
                    scope: id.scope.serial,
                    sequence: id.sequence,
                    release_remaining_ms: Some(lease_release(deadline).saturating_sub(now)),
                }),
                _ => None,
            })
            .collect();
        let mut groups = BTreeMap::new();
        for (id, group) in &self.groups {
            outstanding.extend(group.orphans.iter().map(|orphan| OutstandingAttempt {
                release_remaining_ms: Some(orphan.release_at.saturating_sub(now)),
                ..orphan.attempt.clone()
            }));
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
    /// attempt becomes orphaned occupancy: its scope no longer exists, so it
    /// is released by an explicit reset or when its lease clears (#2664). The
    /// remaining time counts from the restart, so downtime only lengthens it,
    /// and it never exceeds the group's attempt timeout plus the margin.
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
            let bound = policy
                .config
                .groups
                .get(&attempt.group)
                .ok_or(AdmissionError::UnknownGroup)?
                .attempt_timeout_ms
                .saturating_add(UNCERTAIN_LEASE_MARGIN_MS);
            let remaining = attempt.release_remaining_ms.map_or(bound, |r| r.min(bound));
            policy
                .groups
                .get_mut(&attempt.group)
                .expect("configured group")
                .orphans
                .push(Orphan {
                    attempt: attempt.clone(),
                    release_at: now.saturating_add(remaining),
                });
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
