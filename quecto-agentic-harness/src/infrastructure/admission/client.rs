//! Client side of the authority protocol: one connection per scope, request
//! correlation, notices, tracked cancellations/completions and explicit
//! closure on drop. Reading and writing run on separate tasks so a partially
//! read frame is never dropped while the client sends.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use quecto_line_io::{read_frame, write_frame};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;

use super::protocol::*;
use super::remote_gate::RemoteAdmission;
use crate::application::ports::{AttemptAdmission, AuthorityStatus, Credential};
use crate::domain::inference_admission::{Feedback, RequestState, ScopeId, WorkloadClass};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

const COMPLETE_RETRIES: u32 = 40;
const COMPLETE_RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    Unauthorized,
    UnsupportedProtocol(String),
    Admission(String),
    JournalUnavailable,
    Cancelled,
    Timeout,
    EpochReset,
    Closed,
    Protocol(String),
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("admission capability rejected"),
            Self::UnsupportedProtocol(m) => write!(f, "unsupported admission protocol: {m}"),
            Self::Admission(m) => write!(f, "admission refused: {m}"),
            Self::JournalUnavailable => f.write_str("admission ledger not durable"),
            Self::Cancelled => f.write_str("admission request cancelled"),
            Self::Timeout => f.write_str("admission queue wait deadline elapsed"),
            Self::EpochReset => f.write_str("admission authority was reset"),
            Self::Closed => f.write_str("admission authority connection closed"),
            Self::Protocol(m) => write!(f, "admission protocol violation: {m}"),
            Self::Io(m) => write!(f, "admission transport failure: {m}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hello {
    pub epoch: u64,
    pub role: Role,
    pub proposal: AdmissionRuntimeProposal,
}

/// Detached operations whose reply still needs handling.
enum Tracked {
    /// A cancel for `sequence`; an `Active` reply means the grant raced the
    /// cancel and no transport exists, so the attempt is completed as failed.
    Cancel { sequence: u64 },
}

pub(super) struct Inner {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Serializes id/sequence allocation with the send so sequences reach the
    /// authority in order (its replay fence is per scope).
    pending: Mutex<HashMap<u64, oneshot::Sender<Body>>>,
    tracked: Mutex<HashMap<u64, Tracked>>,
    pub(super) notices: Mutex<HashMap<u64, Arc<Notify>>>,
    pub(super) closed: AtomicBool,
    pub(super) closed_notify: Notify,
    next_id: AtomicU64,
    next_sequence: AtomicU64,
    hello: OnceLock<Hello>,
    credential: Mutex<Option<Credential>>,
    /// Runtime owning this connection's I/O; completions are spawned here so
    /// they outlive whichever runtime a permit was finished on.
    io_handle: tokio::runtime::Handle,
    inflight_completions: AtomicUsize,
    drained: Notify,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityClient")
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

fn map_error(code: ErrorCode, reason: String) -> ClientError {
    match code {
        ErrorCode::Unauthorized => ClientError::Unauthorized,
        ErrorCode::UnsupportedProtocol => ClientError::UnsupportedProtocol(reason),
        ErrorCode::JournalUnavailable => ClientError::JournalUnavailable,
        ErrorCode::Malformed => ClientError::Protocol(reason),
        ErrorCode::Admission => ClientError::Admission(reason),
        ErrorCode::Cancelled => ClientError::Cancelled,
        ErrorCode::Timeout => ClientError::Timeout,
        ErrorCode::EpochReset => ClientError::EpochReset,
    }
}

fn encode(id: u64, op: Op) -> Vec<u8> {
    serde_json::to_vec(&Request { id, op }).expect("request is serializable")
}

impl Inner {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Fire-and-forget; the reply is dropped.
    pub(super) fn send_detached(&self, op: Op) {
        let _ = self.tx.send(encode(self.next_id(), op));
    }

    /// Cancel an attempt from a dropped acquire. If the grant raced the cancel,
    /// the reply is `Active` and the attempt is completed as failed because no
    /// transport ever existed (ADR-0026 "client acknowledges no dispatch").
    pub(super) fn cancel_detached(&self, sequence: u64) {
        self.notices.lock().expect("notice map").remove(&sequence);
        let id = self.next_id();
        self.tracked
            .lock()
            .expect("tracked map")
            .insert(id, Tracked::Cancel { sequence });
        let _ = self.tx.send(encode(id, Op::Cancel { sequence }));
    }

    fn register(&self, op: Op) -> Result<(u64, oneshot::Receiver<Body>), ClientError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ClientError::Closed);
        }
        let id = self.next_id();
        let (reply, receiver) = oneshot::channel();
        let mut pending = self.pending.lock().expect("pending map");
        pending.insert(id, reply);
        if self.tx.send(encode(id, op)).is_err() {
            pending.remove(&id);
            return Err(ClientError::Closed);
        }
        Ok((id, receiver))
    }

    async fn await_reply(receiver: oneshot::Receiver<Body>) -> Result<Body, ClientError> {
        match receiver.await {
            Ok(Body::Error { code, reason }) => Err(map_error(code, reason)),
            Ok(body) => Ok(body),
            Err(_) => Err(ClientError::Closed),
        }
    }

    pub(super) async fn call(&self, op: Op) -> Result<Body, ClientError> {
        let (_, receiver) = self.register(op)?;
        Self::await_reply(receiver).await
    }

    /// Allocate the next acquire sequence and send it under one lock so two
    /// concurrent attempts cannot reach the authority out of order.
    pub(super) fn send_acquire(
        &self,
        alias: &str,
        notify: Arc<Notify>,
    ) -> Result<(u64, oneshot::Receiver<Body>), ClientError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ClientError::Closed);
        }
        let mut pending = self.pending.lock().expect("pending map");
        let sequence = self.next_sequence.fetch_add(1, Ordering::AcqRel);
        self.notices
            .lock()
            .expect("notice map")
            .insert(sequence, notify);
        let id = self.next_id();
        let (reply, receiver) = oneshot::channel();
        pending.insert(id, reply);
        let op = Op::Acquire {
            sequence,
            alias: alias.to_owned(),
        };
        if self.tx.send(encode(id, op)).is_err() {
            pending.remove(&id);
            self.notices.lock().expect("notice map").remove(&sequence);
            return Err(ClientError::Closed);
        }
        Ok((sequence, receiver))
    }

    pub(super) async fn await_acquire(
        receiver: oneshot::Receiver<Body>,
    ) -> Result<Body, ClientError> {
        Self::await_reply(receiver).await
    }

    /// Complete an attempt on the connection's own runtime with bounded retries
    /// while the ledger is not durable; counted so a shutdown can drain it.
    pub(super) fn spawn_completion(self: &Arc<Self>, sequence: u64, feedback: Feedback) {
        self.notices.lock().expect("notice map").remove(&sequence);
        self.inflight_completions.fetch_add(1, Ordering::AcqRel);
        let inner = self.clone();
        self.io_handle.spawn(async move {
            let op = Op::Complete {
                sequence,
                feedback: feedback.into(),
            };
            for _ in 0..COMPLETE_RETRIES {
                match inner.call(op.clone()).await {
                    Err(ClientError::JournalUnavailable) => {
                        tokio::time::sleep(COMPLETE_RETRY_DELAY).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, sequence, "admission completion not acknowledged");
                        break;
                    }
                    Ok(_) => break,
                }
            }
            if inner.inflight_completions.fetch_sub(1, Ordering::AcqRel) == 1 {
                inner.drained.notify_waiters();
            }
        });
    }

    /// Wait until every spawned completion has been answered or `limit` elapses.
    pub(super) async fn drain(&self, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.inflight_completions.load(Ordering::Acquire) == 0 {
                return true;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.inflight_completions.load(Ordering::Acquire) == 0;
            }
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.pending.lock().expect("pending map").clear();
        self.tracked.lock().expect("tracked map").clear();
        for notify in self.notices.lock().expect("notice map").drain() {
            notify.1.notify_waiters();
        }
        self.closed_notify.notify_waiters();
    }

    pub(super) fn hello(&self) -> &Hello {
        self.hello.get().expect("hello completes before use")
    }

    fn on_reply(&self, reply: Reply) -> bool {
        match (reply.id, reply.body) {
            (Some(id), body) => {
                let tracked = self.tracked.lock().expect("tracked map").remove(&id);
                match tracked {
                    Some(Tracked::Cancel { sequence }) => {
                        if let Body::State {
                            state: StateWire::Active { .. },
                        } = body
                        {
                            self.send_detached(Op::Complete {
                                sequence,
                                feedback: FeedbackWire::Failure,
                            });
                        }
                    }
                    None => {
                        if let Some(sender) = self.pending.lock().expect("pending map").remove(&id)
                        {
                            let _ = sender.send(body);
                        }
                    }
                }
                true
            }
            (None, Body::CancelRequired { sequence }) => {
                if let Some(notify) = self.notices.lock().expect("notice map").get(&sequence) {
                    notify.notify_waiters();
                    notify.notify_one();
                }
                true
            }
            (None, Body::Revoked) => {
                // The capability is gone; the connection stays usable so the
                // owner sees explicit Unauthorized rather than a drop.
                self.credential.lock().expect("credential").take();
                true
            }
            (None, _) => false,
        }
    }
}

/// Aborts the paired writer when the reader task ends or is aborted.
struct AbortOnDrop(JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn io_loop(
    stream: UnixStream,
    inner: Arc<Inner>,
    mut outbound: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let (read, mut write) = stream.into_split();
    let writer = AbortOnDrop(tokio::spawn(async move {
        while let Some(bytes) = outbound.recv().await {
            if write_frame(&mut write, &bytes, FRAME_CAP).await.is_err() {
                break;
            }
        }
    }));
    let mut reader = BufReader::new(read);
    // A dedicated reader never drops a half-read frame: the only await here
    // is the read itself.
    while let Ok(Some(frame)) = read_frame(&mut reader, FRAME_CAP).await {
        let Ok(reply) = serde_json::from_slice::<Reply>(&frame) else {
            break;
        };
        if !inner.on_reply(reply) {
            break;
        }
    }
    drop(writer);
    inner.close();
}

async fn open(path: &Path, version: u8) -> Result<(Arc<Inner>, JoinHandle<()>), ClientError> {
    let stream = UnixStream::connect(path)
        .await
        .map_err(|e| ClientError::Io(format!("connect {}: {e}", path.display())))?;
    let (tx, rx) = mpsc::unbounded_channel();
    let inner = Arc::new(Inner {
        tx,
        pending: Mutex::new(HashMap::new()),
        tracked: Mutex::new(HashMap::new()),
        notices: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
        closed_notify: Notify::new(),
        next_id: AtomicU64::new(1),
        next_sequence: AtomicU64::new(1),
        hello: OnceLock::new(),
        credential: Mutex::new(None),
        io_handle: tokio::runtime::Handle::current(),
        inflight_completions: AtomicUsize::new(0),
        drained: Notify::new(),
    });
    let io = tokio::spawn(io_loop(stream, inner.clone(), rx));
    let hello = inner
        .call(Op::Hello {
            version,
            capability: CAPABILITY_DIRECT.into(),
        })
        .await;
    let hello = match hello {
        Ok(Body::Hello {
            epoch,
            role,
            proposal,
            ..
        }) => {
            let proposal = AdmissionRuntimeProposal::try_from(proposal)
                .map_err(|e| ClientError::Protocol(format!("published policy invalid: {e:?}")))?;
            Hello {
                epoch,
                role,
                proposal,
            }
        }
        Ok(other) => {
            io.abort();
            return Err(ClientError::Protocol(format!(
                "unexpected hello reply {other:?}"
            )));
        }
        Err(error) => {
            io.abort();
            return Err(error);
        }
    };
    inner.hello.set(hello).ok();
    Ok((inner, io))
}

fn expect_state(body: Body) -> Result<RequestState, ClientError> {
    match body {
        Body::State { state } => Ok(state.into()),
        other => Err(ClientError::Protocol(format!(
            "expected state, got {other:?}"
        ))),
    }
}

fn expect_credential(body: Body) -> Result<Credential, ClientError> {
    match body {
        Body::Credential { credential } => Ok(credential.into()),
        other => Err(ClientError::Protocol(format!(
            "expected credential, got {other:?}"
        ))),
    }
}

/// The owner token lives two levels above the client socket
/// (`<authority>/root.token` beside `<authority>/client/admission.sock`).
pub fn root_token_path_for(client_socket: &Path) -> Option<PathBuf> {
    client_socket
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("root.token"))
}

/// A client-role connection. Dropping it closes the socket, which the
/// authority treats as abandonment of any unfinished work.
pub struct AuthorityConnection {
    inner: Arc<Inner>,
    io: JoinHandle<()>,
    socket: PathBuf,
}

impl std::fmt::Debug for AuthorityConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityConnection")
            .field("socket", &self.socket)
            .field("bound", &self.credential().is_some())
            .finish()
    }
}

impl Drop for AuthorityConnection {
    fn drop(&mut self) {
        self.io.abort();
        self.inner.close();
    }
}

impl AuthorityConnection {
    pub async fn connect(path: &Path) -> Result<Self, ClientError> {
        Self::connect_with_version(path, PROTOCOL_VERSION).await
    }

    pub async fn connect_with_version(path: &Path, version: u8) -> Result<Self, ClientError> {
        let (inner, io) = open(path, version).await?;
        Ok(Self {
            inner,
            io,
            socket: path.to_path_buf(),
        })
    }

    pub fn hello(&self) -> &Hello {
        self.inner.hello()
    }

    pub fn credential(&self) -> Option<Credential> {
        self.inner.credential.lock().expect("credential").clone()
    }

    /// Mint a root with the owner token read from the authority directory.
    /// A process that can only see the mounted client directory fails here.
    pub async fn register_root(&self, class: WorkloadClass) -> Result<Credential, ClientError> {
        let path = root_token_path_for(&self.socket)
            .ok_or_else(|| ClientError::Protocol("client socket has no authority root".into()))?;
        let token = std::fs::read_to_string(&path).map_err(|e| {
            ClientError::Io(format!(
                "owner token {} unreadable ({e}); only the authority's owner can register a root",
                path.display()
            ))
        })?;
        self.register_root_with_token(class, token.trim()).await
    }

    pub async fn register_root_with_token(
        &self,
        class: WorkloadClass,
        owner_token: &str,
    ) -> Result<Credential, ClientError> {
        expect_credential(
            self.inner
                .call(Op::RegisterRoot {
                    class: class.into(),
                    owner_token: owner_token.to_owned(),
                })
                .await?,
        )
    }

    /// Authenticate this connection as `credential`; the sequence counter
    /// resumes above the authority's high-water mark for the scope.
    pub async fn bind(&self, credential: Credential) -> Result<(), ClientError> {
        match self
            .inner
            .call(Op::Bind {
                credential: CredentialWire::from(&credential),
            })
            .await?
        {
            Body::Bound { next_sequence } => {
                self.inner
                    .next_sequence
                    .store(next_sequence.max(1), Ordering::Release);
                *self.inner.credential.lock().expect("credential") = Some(credential);
                Ok(())
            }
            other => Err(ClientError::Protocol(format!(
                "expected bound, got {other:?}"
            ))),
        }
    }

    /// Register a descendant under this connection's capability. The child
    /// receives the credential out of band and binds its own connection.
    pub async fn register_child(&self) -> Result<Credential, ClientError> {
        expect_credential(self.inner.call(Op::RegisterChild).await?)
    }

    /// Retire a descendant this connection registered (a launch that never ran).
    pub async fn retire_child(&self, scope: ScopeId) -> Result<(), ClientError> {
        match self
            .inner
            .call(Op::RetireChild {
                scope: scope.into(),
            })
            .await?
        {
            Body::Ok => Ok(()),
            other => Err(ClientError::Protocol(format!("expected ok, got {other:?}"))),
        }
    }

    /// Transport-neutral gate for one published alias.
    pub fn gate(&self, alias: &str) -> Result<Arc<dyn AttemptAdmission>, ClientError> {
        let hello = self.inner.hello();
        let group =
            hello.proposal.policy.aliases.get(alias).ok_or_else(|| {
                ClientError::Admission(format!("alias '{alias}' is not published"))
            })?;
        let max_cooldown_ms = hello.proposal.policy.groups[group].max_cooldown_ms;
        Ok(Arc::new(RemoteAdmission::new(
            self.inner.clone(),
            alias.to_owned(),
            max_cooldown_ms,
        )))
    }

    pub async fn status(&self, sequence: u64) -> Result<RequestState, ClientError> {
        expect_state(self.inner.call(Op::Status { sequence }).await?)
    }

    pub async fn complete(
        &self,
        sequence: u64,
        feedback: Feedback,
    ) -> Result<RequestState, ClientError> {
        expect_state(
            self.inner
                .call(Op::Complete {
                    sequence,
                    feedback: feedback.into(),
                })
                .await?,
        )
    }

    /// Cancel `sequence` without waiting; a raced grant is completed as failed.
    pub fn cancel_detached(&self, sequence: u64) {
        self.inner.cancel_detached(sequence);
    }

    /// Wait (bounded) for spawned completions to be acknowledged.
    pub async fn drain(&self, limit: Duration) -> bool {
        self.inner.drain(limit).await
    }

    pub async fn retire(&self) -> Result<(), ClientError> {
        match self.inner.call(Op::Retire).await? {
            Body::Ok => {
                self.inner.credential.lock().expect("credential").take();
                Ok(())
            }
            other => Err(ClientError::Protocol(format!("expected ok, got {other:?}"))),
        }
    }
}

/// Owner-only administration over the admin socket.
pub struct AdminConnection {
    inner: Arc<Inner>,
    io: JoinHandle<()>,
}

impl std::fmt::Debug for AdminConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminConnection").finish_non_exhaustive()
    }
}

impl Drop for AdminConnection {
    fn drop(&mut self) {
        self.io.abort();
        self.inner.close();
    }
}

impl AdminConnection {
    pub async fn connect(path: &Path) -> Result<Self, ClientError> {
        let (inner, io) = open(path, PROTOCOL_VERSION).await?;
        if inner.hello().role != Role::Admin {
            io.abort();
            return Err(ClientError::Unauthorized);
        }
        Ok(Self { inner, io })
    }

    pub fn hello(&self) -> &Hello {
        self.inner.hello()
    }

    pub async fn inspect(&self) -> Result<AuthorityStatus, ClientError> {
        match self.inner.call(Op::Inspect).await? {
            Body::Status {
                epoch,
                journal_healthy,
                live_scopes,
                groups,
            } => status_from_body(epoch, journal_healthy, live_scopes, groups)
                .map_err(|e| ClientError::Protocol(format!("status: {e:?}"))),
            other => Err(ClientError::Protocol(format!(
                "expected status, got {other:?}"
            ))),
        }
    }

    pub async fn reset(&self) -> Result<u64, ClientError> {
        match self.inner.call(Op::Reset).await? {
            Body::Reset { epoch } => Ok(epoch),
            other => Err(ClientError::Protocol(format!(
                "expected reset, got {other:?}"
            ))),
        }
    }
}
