//! The authority's state machine: one actor owns the `AdmissionAuthority`,
//! the connected sessions, the scope bindings and the deferred (queued)
//! replies, and services `Command`s from the session tasks in arrival order.
//! Lifecycle, sockets and the accept loops are `server`.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

use super::journal::FileJournal;
use super::protocol::*;
use super::secret::RandomSecretSource;
use super::server::{Clock, wall_ms};
use crate::application::ports::{AdmissionAuthority, AuthorityError, Credential};
use crate::domain::inference_admission::{RequestId, RequestState, ScopeId, TerminalOutcome};

pub(super) enum Command {
    Open {
        session: u64,
        role: Role,
        notices: mpsc::UnboundedSender<Reply>,
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
}

type Authority = AdmissionAuthority<FileJournal, RandomSecretSource>;

pub(super) struct Actor {
    authority: Authority,
    clock: Clock,
    proposal: ProposalWire,
    owner_token: String,
    sessions: BTreeMap<u64, Session>,
    bound: BTreeMap<ScopeId, u64>,
    waiters: BTreeMap<RequestId, oneshot::Sender<Body>>,
    notified: BTreeSet<RequestId>,
    /// Last dispatch failure reported, so outages log on transitions only.
    last_dispatch_failure: Option<String>,
}

const JOURNAL_PROBE_INTERVAL: Duration = Duration::from_millis(250);
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

impl Actor {
    pub(super) fn new(
        authority: Authority,
        clock: Clock,
        proposal: ProposalWire,
        owner_token: String,
    ) -> Self {
        Self {
            authority,
            clock,
            proposal,
            owner_token,
            sessions: BTreeMap::new(),
            bound: BTreeMap::new(),
            waiters: BTreeMap::new(),
            notified: BTreeSet::new(),
            last_dispatch_failure: None,
        }
    }

    pub(super) async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<Command>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let sleep = tokio::time::sleep_until(far_future().into());
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = shutdown.changed() => return,
                command = rx.recv() => match command {
                    Some(command) => self.handle(command),
                    None => return,
                },
                _ = &mut sleep => {}
            }
            let wake = self.settle();
            sleep.as_mut().reset(wake.into());
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Open {
                session,
                role,
                notices,
            } => {
                self.sessions.insert(
                    session,
                    Session {
                        role,
                        greeted: false,
                        credential: None,
                        notices,
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
                match self.authority.disconnect(credential.scope, now) {
                    // Nothing unverified remains: the scope is released so an
                    // abrupt exit never consumes a scope slot until reset.
                    Ok(report) if report.uncertain == 0 => {
                        let _ = self.authority.release(credential.scope);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(?error, "admission disconnect handling failed");
                    }
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
            Op::Inspect | Op::Reset if role != Role::Admin => Err(AuthorityError::Unauthorized),
            Op::Inspect => Ok(status_body(&self.authority.inspect(now))),
            Op::Reset => self.reset(now),
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
        let high_water = self.authority.verify(&credential)?;
        if let Some(previous) = self.bound.insert(credential.scope, session) {
            if previous != session {
                // Supersede: the old session's queued work can never be
                // answered and its active work is unverified until this
                // session completes it.
                if let Some(old) = self.sessions.get_mut(&previous) {
                    old.credential = None;
                }
                let now = self.clock.now_ms();
                let _ = self.authority.disconnect(credential.scope, now);
                self.waiters.retain(|id, _| id.scope != credential.scope);
            }
        }
        self.sessions.get_mut(&session).expect("session").credential = Some(credential);
        Ok(Body::Bound {
            next_sequence: high_water.saturating_add(1),
        })
    }

    fn reset(&mut self, now: u64) -> Result<Body, AuthorityError> {
        let epoch = self.authority.reset(now)?;
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
                .authority
                .complete(credential, *sequence, (*feedback).into(), now)
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
            | Op::Reset => {
                unreachable!("unscoped operations are dispatched by request()")
            }
        };
        Some(result.unwrap_or_else(|e| error_body(&e)))
    }

    /// Dispatch, answer waiters, emit cancellation notices, compute the wake.
    fn settle(&mut self) -> Instant {
        let now = self.clock.now_ms();
        match self.authority.pump(now) {
            Err(AuthorityError::Admission(_)) | Ok(_) => {
                if self.last_dispatch_failure.take().is_some() {
                    tracing::info!("admission dispatch recovered");
                }
            }
            Err(other) => {
                let failure = format!("{other:?}");
                if self.last_dispatch_failure.as_ref() != Some(&failure) {
                    tracing::warn!(failure = %failure, "admission dispatch failed closed");
                    self.last_dispatch_failure = Some(failure);
                }
            }
        }
        let ids: Vec<RequestId> = self.waiters.keys().copied().collect();
        let journal_healthy = self.authority.journal_healthy();
        for id in ids {
            let body = match self.authority.observe(id, now) {
                Ok(RequestState::Queued { .. }) => continue,
                // A waiter still present that observes Cancelled was withdrawn
                // by the authority (client cancels answer their waiter directly).
                Ok(RequestState::Terminal(TerminalOutcome::Cancelled)) if !journal_healthy => {
                    error_body(&AuthorityError::JournalUnavailable)
                }
                Ok(state) => grant_or_terminal(state, now),
                Err(e) => error_body(&e),
            };
            if let Some(waiter) = self.waiters.remove(&id) {
                let _ = waiter.send(body);
            }
        }
        let due = self.authority.cancellation_due(now).unwrap_or_default();
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
        let wake = match self.authority.next_wake(now) {
            Ok(Some(ms)) => self.clock.instant_at(ms),
            Ok(None) => far_future(),
            Err(_) => Instant::now() + Duration::from_millis(50),
        };
        // During a ledger outage held work can only proceed after a successful
        // probe write, so probe at a bounded cadence rather than waiting for
        // queue deadlines.
        if self.authority.journal_healthy() {
            wake
        } else {
            wake.min(Instant::now() + JOURNAL_PROBE_INTERVAL)
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
            reason: "queue wait deadline elapsed".into(),
        },
        RequestState::Terminal(TerminalOutcome::Finished) => Body::State {
            state: state.into(),
        },
        RequestState::Queued { .. } => Body::State {
            state: state.into(),
        },
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}
