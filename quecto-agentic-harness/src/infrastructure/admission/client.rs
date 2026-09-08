//! Client side of the authority protocol: one connection per scope, request
//! correlation, notices, and explicit closure on drop.
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use quecto_line_io::{read_frame, write_frame};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;

use super::protocol::*;
use super::remote_gate::RemoteAdmission;
use crate::application::ports::AttemptAdmission;
use crate::application::ports::{AuthorityStatus, Credential};
use crate::domain::inference_admission::{Feedback, RequestState, WorkloadClass};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

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

pub(super) struct Inner {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Body>>>,
    pub(super) notices: Mutex<HashMap<u64, Arc<Notify>>>,
    pub(super) closed: AtomicBool,
    pub(super) closed_notify: Notify,
    next_id: AtomicU64,
    pub(super) next_sequence: AtomicU64,
    hello: OnceLock<Hello>,
    credential: Mutex<Option<Credential>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityClient")
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Inner {
    pub(super) fn send_detached(&self, op: Op) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let bytes = serde_json::to_vec(&Request { id, op }).expect("request is serializable");
        let _ = self.tx.send(bytes);
    }

    pub(super) async fn call(&self, op: Op) -> Result<Body, ClientError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ClientError::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, receiver) = oneshot::channel();
        self.pending.lock().expect("pending map").insert(id, reply);
        let bytes = serde_json::to_vec(&Request { id, op }).expect("request is serializable");
        if self.tx.send(bytes).is_err() {
            self.pending.lock().expect("pending map").remove(&id);
            return Err(ClientError::Closed);
        }
        match receiver.await {
            Ok(Body::Error { code, reason }) => Err(match code {
                ErrorCode::Unauthorized => ClientError::Unauthorized,
                ErrorCode::UnsupportedProtocol => ClientError::UnsupportedProtocol(reason),
                ErrorCode::JournalUnavailable => ClientError::JournalUnavailable,
                ErrorCode::Malformed => ClientError::Protocol(reason),
                ErrorCode::Admission => ClientError::Admission(reason),
                ErrorCode::Cancelled => ClientError::Cancelled,
                ErrorCode::Timeout => ClientError::Timeout,
                ErrorCode::EpochReset => ClientError::EpochReset,
            }),
            Ok(body) => Ok(body),
            Err(_) => Err(ClientError::Closed),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.pending.lock().expect("pending map").clear();
        for notify in self.notices.lock().expect("notice map").drain() {
            notify.1.notify_waiters();
        }
        self.closed_notify.notify_waiters();
    }

    pub(super) fn hello(&self) -> &Hello {
        self.hello.get().expect("hello completes before use")
    }
}

async fn io_loop(
    stream: UnixStream,
    inner: Arc<Inner>,
    mut outbound: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    loop {
        tokio::select! {
            frame = read_frame(&mut reader, FRAME_CAP) => {
                let Ok(Some(frame)) = frame else { break };
                let Ok(reply) = serde_json::from_slice::<Reply>(&frame) else { break };
                match (reply.id, reply.body) {
                    (Some(id), body) => {
                        if let Some(sender) = inner.pending.lock().expect("pending map").remove(&id) {
                            let _ = sender.send(body);
                        }
                    }
                    (None, Body::CancelRequired { sequence }) => {
                        if let Some(notify) = inner.notices.lock().expect("notice map").get(&sequence) {
                            notify.notify_waiters();
                            notify.notify_one();
                        }
                    }
                    (None, Body::Revoked) => {
                        // The capability is gone; the connection stays usable so
                        // the owner sees explicit Unauthorized rather than a drop.
                        inner.credential.lock().expect("credential").take();
                    }
                    (None, _) => break,
                }
            }
            bytes = outbound.recv() => match bytes {
                Some(bytes) => {
                    if write_frame(&mut write, &bytes, FRAME_CAP).await.is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    }
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
        notices: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
        closed_notify: Notify::new(),
        next_id: AtomicU64::new(1),
        next_sequence: AtomicU64::new(1),
        hello: OnceLock::new(),
        credential: Mutex::new(None),
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

/// A client-role connection. Dropping it closes the socket, which the
/// authority treats as abandonment of any unfinished work.
pub struct AuthorityConnection {
    inner: Arc<Inner>,
    io: JoinHandle<()>,
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
        Ok(Self { inner, io })
    }

    pub fn hello(&self) -> &Hello {
        self.inner.hello()
    }

    pub fn credential(&self) -> Option<Credential> {
        self.inner.credential.lock().expect("credential").clone()
    }

    pub async fn register_root(&self, class: WorkloadClass) -> Result<Credential, ClientError> {
        match self
            .inner
            .call(Op::RegisterRoot {
                class: class.into(),
            })
            .await?
        {
            Body::Credential { credential } => Ok(credential.into()),
            other => Err(ClientError::Protocol(format!(
                "expected credential, got {other:?}"
            ))),
        }
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
        match self.inner.call(Op::RegisterChild).await? {
            Body::Credential { credential } => Ok(credential.into()),
            other => Err(ClientError::Protocol(format!(
                "expected credential, got {other:?}"
            ))),
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
                groups,
            } => status_from_body(epoch, journal_healthy, groups)
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
