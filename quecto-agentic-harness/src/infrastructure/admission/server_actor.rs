//! The authority's state machine: one actor owns the `AdmissionAuthority`,
//! the connected sessions, the scope bindings and the deferred (queued)
//! replies, and services `Command`s from the session tasks in arrival order.
//! Lifecycle, sockets and the accept loops are `server`.
//!
//! The admission decisions are the admission use cases' (#1910): settling a
//! turn (dispatch, waiter answers, cancellations due and the next wake),
//! binding and disconnecting a scope, completion and an epoch reset. What
//! stays here is transport: the session table and its bindings, the hello
//! handshake, the owner-token gate on `RegisterRoot` and the admin-role gate
//! on `Inspect`/`Reset` (both properties of the connection, checked before any
//! authority operation), answering a client's own cancel to the waiter it
//! deferred, and turning the next wake into a timer.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

use super::protocol::*;
use super::secret::RandomSecretSource;
use super::server::{AuthorityUseCases, Clock, wall_ms};
use crate::application::admission::dto::broker::{AuthorityError, Credential};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::authority_idle::{
    AuthorityLifetime, AuthorityStop, IdleVerdict,
};
use crate::domain::admission::value_objects::inference_admission::{
    RequestId, RequestState, ScopeId, TerminalOutcome,
};
use crate::domain::shared::secret_compare::constant_time_eq;

pub(super) enum Command {
    Open {
        session: u64,
        role: Role,
        notices: mpsc::UnboundedSender<Reply>,
        /// Ends the connection when sent or dropped.
        close: oneshot::Sender<()>,
    },
    Request {
        session: u64,
        request: Request,
        reply: oneshot::Sender<Body>,
    },
    Closed {
        session: u64,
    },
}

struct Session {
    role: Role,
    greeted: bool,
    credential: Option<Credential>,
    notices: mpsc::UnboundedSender<Reply>,
    close: Option<oneshot::Sender<()>>,
}

type Authority = AdmissionAuthority<RandomSecretSource>;

pub(super) struct Actor {
    authority: Authority,
    use_cases: AuthorityUseCases,
    clock: Clock,
    proposal: ProposalWire,
    owner_token: String,
    sessions: BTreeMap<u64, Session>,
    bound: BTreeMap<ScopeId, u64>,
    waiters: BTreeMap<RequestId, oneshot::Sender<Body>>,
    notified: BTreeSet<RequestId>,
    /// Last dispatch failure reported, so outages log on transitions only.
    last_dispatch_failure: Option<String>,
    /// Whether this authority stops itself when idle (#2615, ADR-0032).
    lifetime: AuthorityLifetime,
    /// Told why when the authority stops itself.
    stopped: Option<watch::Sender<Option<AuthorityStop>>>,
    /// The idle period ran out or its owner asked: the loop ends after
    /// this turn.
    stopping: Option<AuthorityStop>,
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

impl Actor {
    pub(super) fn new(
        authority: Authority,
        use_cases: AuthorityUseCases,
        clock: Clock,
        proposal: ProposalWire,
        owner_token: String,
    ) -> Self {
        Self {
            authority,
            use_cases,
            clock,
            proposal,
            owner_token,
            sessions: BTreeMap::new(),
            bound: BTreeMap::new(),
            waiters: BTreeMap::new(),
            notified: BTreeSet::new(),
            last_dispatch_failure: None,
            lifetime: AuthorityLifetime::AlwaysOn,
            stopped: None,
            stopping: None,
        }
    }

    /// Serve with `lifetime`, telling `stopped` why when it stops itself.
    pub(super) fn with_lifetime(
        mut self,
        lifetime: AuthorityLifetime,
        stopped: watch::Sender<Option<AuthorityStop>>,
    ) -> Self {
        self.lifetime = lifetime;
        self.stopped = Some(stopped);
        self
    }

    pub(super) async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<Command>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        // The first turn is due at once, so a broker that no client ever
        // reaches still observes its own idleness (#2615) and clears what
        // its restored ledger lets it.
        let sleep = tokio::time::sleep_until(Instant::now().into());
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                command = rx.recv() => match command {
                    Some(command) => self.handle(command),
                    None => break,
                },
                _ = &mut sleep => {}
            }
            let wake = self.settle();
            if let Some(reason) = self.stopping {
                // Stopped in the same turn that decided it: nothing that
                // arrives after this decision is served or granted.
                if let Some(stopped) = &self.stopped {
                    let _ = stopped.send(Some(reason));
                }
                break;
            }
            sleep.as_mut().reset(wake.into());
        }
        // The last turn: lease bounds are durable as they stand, so the
        // uptime already served is not lost to a restart (#2664).
        let now = self.clock.now_ms();
        if let Err(error) = self
            .use_cases
            .settle
            .checkpoint_on_stop(&mut self.authority, now)
        {
            tracing::warn!(?error, "admission ledger not made durable at shutdown");
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Open {
                session,
                role,
                notices,
                close,
            } => {
                self.sessions.insert(
                    session,
                    Session {
                        role,
                        greeted: false,
                        credential: None,
                        notices,
                        close: Some(close),
                    },
                );
            }
            Command::Request {
                session,
                request,
                reply,
            } => {
                let body = self.request(session, &request);
                if let Some(body) = body {
                    let _ = reply.send(body);
                } else {
                    // Deferred: the waiter answers when granted, cancelled or expired.
                    if let Some(credential) = self
                        .sessions
                        .get(&session)
                        .and_then(|s| s.credential.as_ref())
                    {
                        if let Op::Acquire { sequence, .. } = request.op {
                            let id = RequestId {
                                scope: credential.scope,
                                sequence,
                            };
                            match self.waiters.entry(id) {
                                // A replayed acquire observes the queue; it
                                // never displaces the original waiter.
                                std::collections::btree_map::Entry::Occupied(_) => {
                                    let now = self.clock.now_ms();
                                    let body = match self.authority.observe(id, now) {
                                        Ok(state) => Body::State {
                                            state: state.into(),
                                        },
                                        Err(e) => error_body(&e),
                                    };
                                    let _ = reply.send(body);
                                }
                                std::collections::btree_map::Entry::Vacant(slot) => {
                                    slot.insert(reply);
                                }
                            }
                        }
                    }
                }
            }
            Command::Closed { session } => self.closed(session),
        }
    }

    fn closed(&mut self, session: u64) {
        let Some(state) = self.sessions.remove(&session) else {
            return;
        };
        if let Some(credential) = state.credential {
            if self.bound.get(&credential.scope) == Some(&session) {
                self.bound.remove(&credential.scope);
                let now = self.clock.now_ms();
                if let Err(error) =
                    self.use_cases
                        .disconnect
                        .execute(&mut self.authority, credential.scope, now)
                {
                    tracing::warn!(?error, "admission disconnect handling failed");
                }
                self.waiters.retain(|id, _| id.scope != credential.scope);
            }
        }
    }

    /// `None` defers the reply (queued acquire).
    fn request(&mut self, session: u64, request: &Request) -> Option<Body> {
        let now = self.clock.now_ms();
        let Some(state) = self.sessions.get(&session) else {
            return Some(Body::Error {
                code: ErrorCode::Malformed,
                reason: "unknown session".into(),
            });
        };
        let role = state.role;
        if let Op::Hello {
            version,
            capability,
        } = &request.op
        {
            if *version != PROTOCOL_VERSION || capability != CAPABILITY_DIRECT {
                return Some(Body::Error {
                    code: ErrorCode::UnsupportedProtocol,
                    reason: format!(
                        "authority speaks version {PROTOCOL_VERSION} capability {CAPABILITY_DIRECT}"
                    ),
                });
            }
            self.sessions.get_mut(&session).expect("session").greeted = true;
            return Some(Body::Hello {
                version: PROTOCOL_VERSION,
                role,
                epoch: self.authority.inspect(now).epoch,
                proposal: self.proposal.clone(),
            });
        }
        if !state.greeted {
            return Some(Body::Error {
                code: ErrorCode::UnsupportedProtocol,
                reason: "hello required first".into(),
            });
        }
        let credential = state.credential.clone();
        let result = match &request.op {
            Op::Hello { .. } => unreachable!("handled above"),
            Op::RegisterRoot { class, owner_token } => {
                if constant_time_eq(owner_token, &self.owner_token) {
                    self.authority
                        .register_root((*class).into(), now)
                        .map(|c| Body::Credential {
                            credential: CredentialWire::from(&c),
                        })
                } else {
                    Err(AuthorityError::Unauthorized)
                }
            }
            Op::Bind { credential } => self.bind(session, credential.clone().into()),
            Op::Inspect | Op::Reset | Op::Stop if role != Role::Admin => {
                Err(AuthorityError::Unauthorized)
            }
            Op::Inspect => Ok(status_body(&self.authority.inspect(now))),
            Op::Reset => self.reset(now),
            Op::Stop => {
                // Answered first; the loop ends after this turn (#2615).
                let _ = AuthorityStop::Requested;
                Ok(Body::Ok)
            }
            op => match credential {
                None => Err(AuthorityError::Unauthorized),
                Some(credential) => return self.scoped(op, &credential, now),
            },
        };
        Some(result.unwrap_or_else(|e| error_body(&e)))
    }

    fn bind(&mut self, session: u64, credential: Credential) -> Result<Body, AuthorityError> {
        if self
            .sessions
            .get(&session)
            .is_some_and(|state| state.credential.is_some())
        {
            return Ok(Body::Error {
                code: ErrorCode::Malformed,
                reason: "session already bound; one connection binds one capability".into(),
            });
        }
        let supersedes = self
            .bound
            .get(&credential.scope)
            .is_some_and(|previous| *previous != session);
        let now = self.clock.now_ms();
        let next_sequence =
            self.use_cases
                .bind
                .execute(&mut self.authority, &credential, supersedes, now)?;
        let previous = self.bound.insert(credential.scope, session);
        debug_assert_eq!(
            previous.is_some_and(|previous| previous != session),
            supersedes,
            "the binding supersedes exactly the connection it was told of"
        );
        if let Some(previous) = previous.filter(|previous| *previous != session) {
            // The superseded connection no longer holds the capability, and
            // its waiters can never be answered. It is closed, so its client
            // sees the loss and tears down any transport it still runs
            // before the lease it left can clear (#2664, ADR-0031).
            if let Some(old) = self.sessions.get_mut(&previous) {
                old.credential = None;
                if let Some(close) = old.close.take() {
                    let _ = close.send(());
                }
            }
            self.waiters.retain(|id, _| id.scope != credential.scope);
        }
        self.sessions.get_mut(&session).expect("session").credential = Some(credential);
        Ok(Body::Bound { next_sequence })
    }

    fn reset(&mut self, now: u64) -> Result<Body, AuthorityError> {
        let epoch = self
            .use_cases
            .advance_epoch
            .execute(&mut self.authority, now)?;
        self.bound.clear();
        for state in self.sessions.values_mut() {
            if state.credential.take().is_some() {
                let _ = state.notices.send(Reply {
                    id: None,
                    body: Body::Revoked,
                });
            }
        }
        for (_, waiter) in std::mem::take(&mut self.waiters) {
            let _ = waiter.send(Body::Error {
                code: ErrorCode::EpochReset,
                reason: "authority reset".into(),
            });
        }
        self.notified.clear();
        Ok(Body::Reset { epoch })
    }

    fn scoped(&mut self, op: &Op, credential: &Credential, now: u64) -> Option<Body> {
        let result = match op {
            Op::RegisterChild => {
                self.authority
                    .register_child(credential, now)
                    .map(|c| Body::Credential {
                        credential: CredentialWire::from(&c),
                    })
            }
            Op::Acquire { sequence, alias } => {
                match self.authority.acquire(credential, *sequence, alias, now) {
                    Ok(RequestState::Queued { .. }) => return None,
                    Ok(state) => Ok(grant_or_terminal(state, now)),
                    Err(e) => Err(e),
                }
            }
            Op::Cancel { sequence } => {
                let id = RequestId {
                    scope: credential.scope,
                    sequence: *sequence,
                };
                let outcome = self.authority.cancel(credential, *sequence, now);
                if let Some(waiter) = self.waiters.remove(&id) {
                    let _ = waiter.send(Body::Error {
                        code: ErrorCode::Cancelled,
                        reason: "cancelled".into(),
                    });
                }
                outcome.map(|state| Body::State {
                    state: state.into(),
                })
            }
            Op::Feedback {
                sequence,
                report,
                feedback,
            } => self
                .authority
                .feedback(credential, *sequence, *report, (*feedback).into(), now)
                .map(|()| Body::Ok),
            Op::Complete { sequence, feedback } => self
                .use_cases
                .complete
                .execute(
                    &mut self.authority,
                    credential,
                    *sequence,
                    (*feedback).into(),
                    now,
                )
                .map(|state| Body::State {
                    state: state.into(),
                }),
            Op::Status { sequence } => {
                self.authority
                    .status(credential, *sequence, now)
                    .map(|state| Body::State {
                        state: state.into(),
                    })
            }
            Op::RetireChild { scope } => {
                let child: ScopeId = (*scope).into();
                let outcome = self
                    .authority
                    .retire_child(credential, child, now)
                    .map(|()| Body::Ok);
                if outcome.is_ok() {
                    if let Some(session) = self.bound.remove(&child) {
                        if let Some(state) = self.sessions.get_mut(&session) {
                            state.credential = None;
                        }
                    }
                }
                outcome
            }
            Op::Retire => {
                let outcome = self.authority.retire(credential, now).map(|()| Body::Ok);
                if outcome.is_ok() {
                    if let Some(session) = self.bound.remove(&credential.scope) {
                        if let Some(state) = self.sessions.get_mut(&session) {
                            state.credential = None;
                        }
                    }
                }
                outcome
            }
            Op::Hello { .. }
            | Op::RegisterRoot { .. }
            | Op::Bind { .. }
            | Op::Inspect
            | Op::Reset
            | Op::Stop => {
                unreachable!("unscoped operations are dispatched by request()")
            }
        };
        Some(result.unwrap_or_else(|e| error_body(&e)))
    }

    /// Settle the authority through its one use case, then answer the
    /// waiters, emit cancellation notices and compute the wake from what it
    /// decided.
    fn settle(&mut self) -> Instant {
        let now = self.clock.now_ms();
        let waiting: Vec<RequestId> = self.waiters.keys().copied().collect();
        let mut settlement = self
            .use_cases
            .settle
            .execute(&mut self.authority, &waiting, now);
        match settlement.dispatch_failure.take() {
            None => {
                if self.last_dispatch_failure.take().is_some() {
                    tracing::info!("admission dispatch recovered");
                }
            }
            Some(other) => {
                let failure = format!("{other:?}");
                if self.last_dispatch_failure.as_ref() != Some(&failure) {
                    tracing::warn!(failure = %failure, "admission dispatch failed closed");
                    self.last_dispatch_failure = Some(failure);
                }
            }
        }
        for (id, answer) in std::mem::take(&mut settlement.answers) {
            let body = match answer {
                Ok(state) => grant_or_terminal(state, now),
                Err(e) => error_body(&e),
            };
            if let Some(waiter) = self.waiters.remove(&id) {
                let _ = waiter.send(body);
            }
        }
        let due = std::mem::take(&mut settlement.cancellation_due);
        for id in &due {
            if self.notified.insert(*id) {
                if let Some(state) = self.bound.get(&id.scope).and_then(|s| self.sessions.get(s)) {
                    let _ = state.notices.send(Reply {
                        id: None,
                        body: Body::CancelRequired {
                            sequence: id.sequence,
                        },
                    });
                }
            }
        }
        self.notified.retain(|id| due.contains(id));
        // The idle stop (#2615, ADR-0032): only a turn that finds nothing
        // held and no connection open counts as idle, so a broker holding a
        // root, a permit, a wait or a lease (uncertain ones included), or
        // serving any connection, never stops.
        let holding = settlement.holding || !self.sessions.is_empty();
        let idle_deadline = match self.lifetime.observe(holding, self.clock.now_ms()) {
            IdleVerdict::Holding => None,
            IdleVerdict::IdleUntil(deadline) => Some(deadline),
            IdleVerdict::Stop => {
                debug_assert!(!holding, "only an idle authority stops itself");
                self.stopping = Some(AuthorityStop::Idle);
                None
            }
        };
        // Armed now, after the turn: a slow write cannot leave the next
        // probe already due.
        let wake = settlement.wake_at(self.clock.now_ms());
        let wake = match (wake, idle_deadline) {
            (Some(wake), Some(idle)) => Some(wake.min(idle)),
            (wake, idle) => wake.or(idle),
        };
        match wake {
            Some(ms) => self.clock.instant_at(ms),
            None => far_future(),
        }
    }
}

fn grant_or_terminal(state: RequestState, now: u64) -> Body {
    match state {
        RequestState::Active { deadline, .. } => Body::Granted {
            deadline_ms: deadline,
            receipt_ms: now,
            receipt_wall_ms: wall_ms(),
        },
        RequestState::Terminal(TerminalOutcome::Cancelled) => Body::Error {
            code: ErrorCode::Cancelled,
            reason: "cancelled".into(),
        },
        RequestState::Terminal(TerminalOutcome::TimedOut) => Body::Error {
            code: ErrorCode::Timeout,
            reason: QUEUE_TIMEOUT_REASON.into(),
        },
        RequestState::Terminal(TerminalOutcome::Finished) => Body::State {
            state: state.into(),
        },
        RequestState::Queued { .. } => Body::State {
            state: state.into(),
        },
    }
}

#[cfg(test)]
#[path = "server_actor_tests.rs"]
mod tests;
