//! Pure queue, pacing and capacity transitions for one accounting epoch.
use super::inference_admission::*;
use super::inference_cooldown::FallbackCooldown;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[path = "inference_admission_recovery.rs"]
mod recovery;

#[derive(Debug)]
struct Scope {
    root: ScopeId,
    class: WorkloadClass,
    high_water: u64,
    retired: bool,
}

#[derive(Debug)]
struct Request {
    group: GroupId,
    state: RequestState,
    feedback: Option<Feedback>,
    // One fingerprint per retained attempt, never an unbounded receipt history.
    // It survives completion and is evicted with the terminal request.
    last_report: Option<(u64, ThrottleFeedback)>,
}

#[derive(Debug)]
struct Group {
    fallback: FallbackCooldown,
    next_start: u64,
    cooldown: u64,
    unavailable: bool,
    interactive_streak: u8,
    contested_pacing_streak: u8,
    last_root: [Option<ScopeId>; 2],
    last_agent: BTreeMap<(u8, ScopeId), ScopeId>,
    /// Active attempts whose client vanished before acknowledgement.
    uncertain: BTreeSet<RequestId>,
    /// Outstanding attempts inherited from a previous authority process.
    orphans: Vec<OutstandingAttempt>,
}

/// In-memory policy, deliberately without persistence or transport assumptions.
#[derive(Debug)]
pub struct AdmissionPolicy {
    config: AdmissionConfig,
    epoch: u64,
    now: u64,
    /// Monotonic: a serial is never reused, even after retirement.
    next_serial: u64,
    scopes: BTreeMap<ScopeId, Scope>,
    requests: BTreeMap<RequestId, Request>,
    terminals: VecDeque<RequestId>,
    groups: BTreeMap<GroupId, Group>,
}

impl AdmissionPolicy {
    pub fn new(epoch: u64, config: AdmissionConfig) -> Result<Self, AdmissionError> {
        config.validate()?;
        let groups = config
            .groups
            .iter()
            .map(|(id, policy)| {
                let maximum = policy.max_cooldown_ms;
                Ok((
                    id.clone(),
                    Group {
                        fallback: FallbackCooldown::new(policy.fallback_base_ms, maximum)?,
                        next_start: 0,
                        cooldown: 0,
                        unavailable: false,
                        interactive_streak: 0,
                        contested_pacing_streak: 0,
                        last_root: [None; 2],
                        last_agent: BTreeMap::new(),
                        uncertain: BTreeSet::new(),
                        orphans: Vec::new(),
                    },
                ))
            })
            .collect::<Result<_, AdmissionError>>()?;
        Ok(Self {
            config,
            epoch,
            now: 0,
            next_serial: 0,
            scopes: BTreeMap::new(),
            requests: BTreeMap::new(),
            terminals: VecDeque::new(),
            groups,
        })
    }

    pub fn register_root(&mut self, class: WorkloadClass) -> Result<ScopeId, AdmissionError> {
        self.register(None, class)
    }

    pub fn register_child(&mut self, parent: ScopeId) -> Result<ScopeId, AdmissionError> {
        let root = self.scope(parent)?.root;
        self.register(Some(root), WorkloadClass::Background)
    }

    fn register(
        &mut self,
        root: Option<ScopeId>,
        class: WorkloadClass,
    ) -> Result<ScopeId, AdmissionError> {
        // The limit bounds live scopes; retired scopes free their slot while
        // their serial stays burned so identity is never recycled.
        let live = self.scopes.values().filter(|scope| !scope.retired).count();
        if live >= self.config.max_scopes {
            return Err(AdmissionError::ScopeLimit);
        }
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(AdmissionError::ScopeLimit)?;
        self.next_serial = serial;
        let id = ScopeId {
            epoch: self.epoch,
            serial,
        };
        self.scopes.insert(
            id,
            Scope {
                root: root.unwrap_or(id),
                class,
                high_water: 0,
                retired: false,
            },
        );
        Ok(id)
    }

    fn scope(&self, id: ScopeId) -> Result<&Scope, AdmissionError> {
        if id.epoch != self.epoch {
            return Err(AdmissionError::StaleEpoch);
        }
        self.scopes
            .get(&id)
            .filter(|scope| !scope.retired)
            .ok_or(AdmissionError::UnknownScope)
    }

    pub fn retire(&mut self, scope: ScopeId) -> Result<(), AdmissionError> {
        self.scope(scope)?;
        if self
            .requests
            .iter()
            .any(|(id, r)| id.scope == scope && matches!(r.state, RequestState::Active { .. }))
        {
            return Err(AdmissionError::Busy);
        }
        let queued: Vec<_> = self
            .requests
            .iter()
            .filter(|(id, r)| id.scope == scope && matches!(r.state, RequestState::Queued { .. }))
            .map(|(id, _)| *id)
            .collect();
        for id in queued {
            self.terminal(id, TerminalOutcome::Cancelled, None);
        }
        self.scopes
            .get_mut(&scope)
            .expect("validated scope")
            .retired = true;
        Ok(())
    }

    fn tick(&mut self, now: u64) -> Result<(), AdmissionError> {
        if now < self.now {
            return Err(AdmissionError::TimeRegression);
        }
        self.now = now;
        let expired: Vec<_> = self
            .requests
            .iter()
            .filter_map(|(id, r)| match r.state {
                RequestState::Queued { deadline } if deadline <= now => Some(*id),
                _ => None,
            })
            .collect();
        for id in expired {
            self.terminal(id, TerminalOutcome::TimedOut, None);
        }
        for request in self.requests.values_mut() {
            if let RequestState::Active {
                deadline,
                cancellation_required,
                ..
            } = &mut request.state
            {
                if *deadline <= now {
                    *cancellation_required = true;
                }
            }
        }
        Ok(())
    }

    fn terminal(&mut self, id: RequestId, outcome: TerminalOutcome, feedback: Option<Feedback>) {
        let request = self.requests.get_mut(&id).expect("live request");
        request.state = RequestState::Terminal(outcome);
        request.feedback = feedback;
        // A terminal transition is verified (client-acknowledged) evidence.
        self.groups
            .get_mut(&request.group)
            .expect("configured group")
            .uncertain
            .remove(&id);
        self.terminals.push_back(id);
        while self.terminals.len() > self.config.terminal_capacity {
            if let Some(old) = self.terminals.pop_front() {
                self.requests.remove(&old);
            }
        }
    }

    pub fn enqueue(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        alias: &str,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let group = self
            .config
            .aliases
            .get(alias)
            .ok_or(AdmissionError::UnknownGroup)?
            .clone();
        let id = RequestId { scope, sequence };
        if let Some(request) = self.requests.get(&id) {
            return if request.group == group {
                Ok(request.state)
            } else {
                Err(AdmissionError::Conflict)
            };
        }
        if sequence <= self.scope(scope)?.high_water {
            return Err(AdmissionError::Replay);
        }
        if self.groups[&group].unavailable {
            return Err(AdmissionError::Unavailable);
        }
        let queued = self
            .requests
            .values()
            .filter(|r| r.group == group && matches!(r.state, RequestState::Queued { .. }))
            .count();
        let policy = &self.config.groups[&group];
        if queued >= policy.queue_capacity {
            return Err(AdmissionError::QueueFull);
        }
        let Some(deadline) = now.checked_add(policy.queue_timeout_ms) else {
            self.groups
                .get_mut(&group)
                .expect("configured group")
                .unavailable = true;
            return Err(AdmissionError::Unavailable);
        };
        let state = RequestState::Queued { deadline };
        self.requests.insert(
            id,
            Request {
                group,
                state,
                feedback: None,
                last_report: None,
            },
        );
        self.scopes
            .get_mut(&scope)
            .expect("validated scope")
            .high_water = sequence;
        Ok(state)
    }

    fn request(&self, scope: ScopeId, sequence: u64) -> Result<&Request, AdmissionError> {
        let identity = self.scope(scope)?;
        self.requests.get(&RequestId { scope, sequence }).ok_or(
            if sequence <= identity.high_water {
                AdmissionError::Replay
            } else {
                AdmissionError::UnknownRequest
            },
        )
    }

    pub fn status(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        Ok(self.request(scope, sequence)?.state)
    }

    pub fn cancel(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let id = RequestId { scope, sequence };
        match self.request(scope, sequence)?.state {
            RequestState::Queued { .. } => self.terminal(id, TerminalOutcome::Cancelled, None),
            RequestState::Active { .. } => {
                if let RequestState::Active {
                    cancellation_required,
                    ..
                } = &mut self.requests.get_mut(&id).expect("live request").state
                {
                    *cancellation_required = true;
                }
            }
            RequestState::Terminal(_) => {}
        }
        Ok(self.requests[&id].state)
    }

    /// Merge receipt-anchored advice without releasing transport occupancy.
    pub fn report_feedback(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        report: u64,
        feedback: ThrottleFeedback,
        now: u64,
    ) -> Result<(), AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let request = self.request(scope, sequence)?;
        if let Some((last, fingerprint)) = request.last_report {
            if report < last {
                return Err(AdmissionError::Replay);
            }
            if report == last {
                return if feedback == fingerprint {
                    Ok(())
                } else {
                    Err(AdmissionError::Conflict)
                };
            }
        }
        // Only retained duplicates are valid after completion. A queued or
        // unknown attempt has no authority to change group cooldowns.
        if !matches!(request.state, RequestState::Active { .. }) {
            return Err(AdmissionError::Conflict);
        }
        let group_id = request.group.clone();
        let group = self.groups.get_mut(&group_id).expect("configured group");
        // Every accepted throttle advances the group-wide streak, including
        // hinted advice. Identity/state checks above keep duplicates inert.
        let jitter = match feedback {
            ThrottleFeedback::NoHint { jitter } => jitter,
            _ => 0,
        };
        let fallback_delay = group.fallback.throttle(jitter);
        match feedback {
            ThrottleFeedback::NoHint { .. } => match now.checked_add(fallback_delay) {
                Some(deadline) => group.cooldown = group.cooldown.max(deadline),
                None => group.unavailable = true,
            },
            ThrottleFeedback::Until(deadline)
                if deadline.saturating_sub(now)
                    <= self.config.groups[&group_id].max_cooldown_ms =>
            {
                group.cooldown = group.cooldown.max(deadline);
            }
            // Never clamp excessive advice into an earlier retry.
            ThrottleFeedback::Until(_) | ThrottleFeedback::Unavailable => {
                group.unavailable = true;
            }
        }
        self.requests
            .get_mut(&RequestId { scope, sequence })
            .expect("validated active request")
            .last_report = Some((report, feedback));
        Ok(())
    }

    pub fn complete(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.scope(scope)?;
        self.tick(now)?;
        let id = RequestId { scope, sequence };
        // A sequence above the high-water mark was never enqueued, so no grant
        // can exist for it: refuse without touching any group's availability.
        // Replays below the mark are fenced separately and cannot free work.
        let request = self.request(scope, sequence)?;
        if let RequestState::Terminal(_) = request.state {
            return if request.feedback == Some(feedback) {
                Ok(request.state)
            } else {
                Err(AdmissionError::Conflict)
            };
        }
        if !matches!(request.state, RequestState::Active { .. }) {
            return Err(AdmissionError::Conflict);
        }
        let group_id = request.group.clone();
        let receipt_reported = request.last_report.is_some();
        let group = self.groups.get_mut(&group_id).expect("configured group");
        if feedback == Feedback::Success {
            group.fallback.success();
        }
        if let Feedback::Throttle { delay_ms } = feedback {
            // Legacy completion-only feedback also counts as a throttle, but
            // must not count an already reported receipt a second time.
            if !receipt_reported {
                group.fallback.throttle(0);
            }
            match now.checked_add(delay_ms) {
                Some(deadline) if delay_ms <= self.config.groups[&group_id].max_cooldown_ms => {
                    group.cooldown = group.cooldown.max(deadline)
                }
                _ => group.unavailable = true,
            }
        }
        self.terminal(id, TerminalOutcome::Finished, Some(feedback));
        Ok(RequestState::Terminal(TerminalOutcome::Finished))
    }

    pub fn snapshot(&mut self, id: &GroupId, now: u64) -> Result<GroupSnapshot, AdmissionError> {
        self.tick(now)?;
        let group = self.groups.get(id).ok_or(AdmissionError::UnknownGroup)?;
        let mut snapshot = GroupSnapshot {
            active: group.orphans.len(),
            queued: 0,
            uncertain: group.uncertain.len() + group.orphans.len(),
            cooldown_until: group.cooldown,
            unavailable: group.unavailable,
            observed_at: now,
        };
        for r in self.requests.values().filter(|r| &r.group == id) {
            match r.state {
                RequestState::Queued { .. } => snapshot.queued += 1,
                RequestState::Active { .. } => snapshot.active += 1,
                RequestState::Terminal(_) => {}
            }
        }
        Ok(snapshot)
    }

    pub fn next(&mut self, id: &GroupId, now: u64) -> Result<Option<RequestId>, AdmissionError> {
        let snapshot = self.snapshot(id, now)?;
        let policy = &self.config.groups[id];
        let group = &self.groups[id];
        if group.unavailable {
            return Err(AdmissionError::Unavailable);
        }
        if snapshot.uncertain > 0 {
            return Err(AdmissionError::Quarantined);
        }
        if snapshot.active >= policy.capacity || now < group.next_start || now < group.cooldown {
            return Ok(None);
        }
        let background_active = self
            .requests
            .iter()
            .filter(|(request_id, r)| {
                &r.group == id
                    && matches!(r.state, RequestState::Active { .. })
                    && self.scopes[&request_id.scope].class == WorkloadClass::Background
            })
            .count();
        // Interactive transports occupy the reserve first. A long interactive
        // stream must not turn every remaining free slot into a reserved slot.
        let interactive_active = snapshot.active - background_active;
        let shared_active = background_active + interactive_active.saturating_sub(policy.reserve);
        let shared = shared_active < policy.capacity - policy.reserve;
        let interactive = self.candidate(id, WorkloadClass::Interactive);
        let background = if shared && background_active < policy.capacity - policy.reserve {
            self.candidate(id, WorkloadClass::Background)
        } else {
            None
        };
        let selected = match (interactive, background) {
            (Some(i), Some(b)) => Some(
                if group.interactive_streak >= 3 || group.contested_pacing_streak >= 3 {
                    b
                } else {
                    i
                },
            ),
            (i, b) => i.or(b),
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        let (Some(next_start), Some(deadline)) = (
            now.checked_add(policy.min_interval_ms),
            now.checked_add(policy.attempt_timeout_ms),
        ) else {
            self.groups
                .get_mut(id)
                .expect("configured group")
                .unavailable = true;
            return Err(AdmissionError::Unavailable);
        };
        let scope = &self.scopes[&selected.scope];
        let class = usize::from(scope.class == WorkloadClass::Background);
        let group = self.groups.get_mut(id).expect("configured group");
        group.next_start = next_start;
        group.last_root[class] = Some(scope.root);
        group
            .last_agent
            .insert((class as u8, scope.root), selected.scope);
        // Reserve grants do not spend shared-slot turns, but cannot monopolize
        // every paced start while background is eligible for shared capacity.
        if background.is_some() {
            group.contested_pacing_streak = if class == 0 {
                group.contested_pacing_streak.saturating_add(1).min(3)
            } else {
                0
            };
        }
        let uses_shared = class == 1 || interactive_active >= policy.reserve;
        if uses_shared {
            group.contested_pacing_streak = 0;
            group.interactive_streak = if class == 0 {
                group.interactive_streak.saturating_add(1).min(3)
            } else {
                0
            };
        }
        self.requests
            .get_mut(&selected)
            .expect("selected queue head")
            .state = RequestState::Active {
            started: now,
            deadline,
            cancellation_required: false,
        };
        Ok(Some(selected))
    }

    fn candidate(&self, id: &GroupId, class: WorkloadClass) -> Option<RequestId> {
        let mut heads = BTreeMap::<ScopeId, BTreeMap<ScopeId, RequestId>>::new();
        for (request_id, request) in &self.requests {
            let scope = &self.scopes[&request_id.scope];
            if &request.group == id
                && scope.class == class
                && matches!(request.state, RequestState::Queued { .. })
            {
                heads
                    .entry(scope.root)
                    .or_default()
                    .entry(request_id.scope)
                    .or_insert(*request_id);
            }
        }
        let class = usize::from(class == WorkloadClass::Background);
        let group = &self.groups[id];
        let root = round_robin(&heads, group.last_root[class])?;
        let agents = &heads[&root];
        let agent = round_robin(agents, group.last_agent.get(&(class as u8, root)).copied())?;
        Some(agents[&agent])
    }
}

fn round_robin<T>(entries: &BTreeMap<ScopeId, T>, after: Option<ScopeId>) -> Option<ScopeId> {
    after
        .and_then(|previous| entries.keys().find(|id| **id > previous).copied())
        .or_else(|| entries.keys().next().copied())
}
