//! Authority process: singleton lock, durable journal, two framed UDS
//! listeners (client and owner-only admin) and one serialized actor.
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::directory::{AuthorityDirectory, SingletonLock};
use super::journal::FileJournal;
use super::protocol::*;
use super::secret::RandomSecretSource;
use super::session;
use crate::application::ports::{AdmissionAuthority, AuthorityError, Credential};
use crate::domain::inference_admission::{RequestId, RequestState, ScopeId, TerminalOutcome};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

#[derive(Debug)]
pub enum ServerError {
    Busy(String),
    Io(String),
    Journal(String),
    Policy(String),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(m) | Self::Io(m) | Self::Journal(m) | Self::Policy(m) => f.write_str(m),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Clock {
    start: Instant,
}

impl Clock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn instant_at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }
}

pub(super) fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

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

struct Actor {
    authority: Authority,
    clock: Clock,
    proposal: ProposalWire,
    sessions: BTreeMap<u64, Session>,
    bound: BTreeMap<ScopeId, u64>,
    waiters: BTreeMap<RequestId, oneshot::Sender<Body>>,
    notified: BTreeSet<RequestId>,
}

fn bind_private(path: &Path) -> Result<UnixListener, ServerError> {
    // The singleton lock is held: any file at this path is a stale socket.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)
        .map_err(|e| ServerError::Io(format!("bind {}: {e}", path.display())))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        let _ = std::fs::remove_file(path);
        ServerError::Io(format!("chmod {}: {e}", path.display()))
    })?;
    Ok(listener)
}

/// A running authority. Dropping without `shutdown` aborts its tasks.
pub struct AuthorityServer {
    dir: AuthorityDirectory,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    _lock: SingletonLock,
}

impl AuthorityServer {
    pub async fn start(
        dir: AuthorityDirectory,
        proposal: AdmissionRuntimeProposal,
    ) -> Result<Self, ServerError> {
        proposal
            .policy
            .validate()
            .map_err(|e| ServerError::Policy(format!("invalid admission policy: {e:?}")))?;
        let lock = SingletonLock::acquire(&dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock => ServerError::Busy(e.to_string()),
            _ => ServerError::Io(e.to_string()),
        })?;
        let clock = Clock {
            start: Instant::now(),
        };
        let journal = FileJournal::new(&dir);
        let ledger = FileJournal::load(&dir).map_err(|e| ServerError::Journal(format!("{e:?}")))?;
        let authority = match ledger {
            Some(ledger) => AdmissionAuthority::restore(
                proposal.policy.clone(),
                &ledger,
                journal,
                RandomSecretSource,
                clock.now_ms(),
            ),
            None => {
                AdmissionAuthority::new(1, proposal.policy.clone(), journal, RandomSecretSource)
            }
        }
        .map_err(|e| ServerError::Journal(format!("ledger incompatible with policy: {e:?}")))?;
        let client_listener = bind_private(&dir.client_socket())?;
        let admin_listener = bind_private(&dir.admin_socket())?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (shutdown, shutdown_rx) = watch::channel(false);
        let actor = Actor {
            authority,
            clock,
            proposal: ProposalWire::from(&proposal),
            sessions: BTreeMap::new(),
            bound: BTreeMap::new(),
            waiters: BTreeMap::new(),
            notified: BTreeSet::new(),
        };
        let tasks = vec![
            tokio::spawn(actor.run(rx, shutdown_rx.clone())),
            tokio::spawn(accept_loop(
                client_listener,
                Role::Client,
                tx.clone(),
                shutdown_rx.clone(),
                1,
            )),
            tokio::spawn(accept_loop(
                admin_listener,
                Role::Admin,
                tx,
                shutdown_rx,
                1 << 62,
            )),
        ];
        Ok(Self {
            dir,
            shutdown,
            tasks,
            _lock: lock,
        })
    }

    pub fn directory(&self) -> &AuthorityDirectory {
        &self.dir
    }

    /// Stop accepting, close sessions and exit the actor. The journal keeps
    /// outstanding work for the next start; sockets are removed.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        for task in std::mem::take(&mut self.tasks) {
            task.abort();
            let _ = task.await;
        }
        let _ = std::fs::remove_file(self.dir.client_socket());
        let _ = std::fs::remove_file(self.dir.admin_socket());
    }
}

impl Drop for AuthorityServer {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn accept_loop(
    listener: UnixListener,
    role: Role,
    commands: mpsc::UnboundedSender<Command>,
    mut shutdown: watch::Receiver<bool>,
    first_session: u64,
) {
    let mut next = first_session;
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let session = next;
                    next += 1;
                    tokio::spawn(session::serve(stream, session, role, commands.clone()));
                }
                Err(error) => {
                    tracing::warn!(%error, "admission authority accept failed");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

impl Actor {
    async fn run(
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
                            self.waiters.insert(id, reply);
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
                if let Err(error) = self.authority.disconnect(credential.scope, now) {
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
            Op::RegisterRoot { class } => {
                self.authority
                    .register_root((*class).into(), now)
                    .map(|c| Body::Credential {
                        credential: CredentialWire::from(&c),
                    })
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
        let high_water = self.authority.verify(&credential)?;
        if let Some(previous) = self.bound.insert(credential.scope, session) {
            if previous != session {
                if let Some(old) = self.sessions.get_mut(&previous) {
                    old.credential = None;
                }
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
        if let Err(error) = self.authority.pump(now) {
            match error {
                AuthorityError::Admission(_) => {}
                other => tracing::warn!(?other, "admission dispatch failed closed"),
            }
        }
        let ids: Vec<RequestId> = self.waiters.keys().copied().collect();
        for id in ids {
            let body = match self.authority.observe(id, now) {
                Ok(RequestState::Queued { .. }) => continue,
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
        match self.authority.next_wake(now) {
            Ok(Some(ms)) => self.clock.instant_at(ms),
            Ok(None) => far_future(),
            Err(_) => Instant::now() + Duration::from_millis(50),
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
