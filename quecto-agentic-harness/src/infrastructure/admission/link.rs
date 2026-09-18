//! Reconnecting authority link (#2024 S3): the one connection every gate of a
//! process routes through, so a broker restart or `reset` no longer forces the
//! agent to restart.
//!
//! A root can re-register on its own (the owner token is on disk and readable
//! after a restart), so its link reconnects: it re-opens the client socket,
//! registers a fresh root scope with the owner token currently on disk and
//! binds it, then swaps the connection in. A loss is a closed socket (broker
//! died) *or* a revoked capability (`reset` keeps the socket open but drops
//! every credential). A child cannot re-register — its capability was minted
//! by the parent and is gone once the epoch or the broker changed — so a child
//! link has no reconnect and its next attempt fails closed with a clear error
//! (never a silent bypass); the parent respawns it.
//!
//! Reconnect is serialized and bounded: concurrent attempts share one
//! reconnection, and it gives up after a bounded number of jittered tries
//! rather than blocking a turn forever (health then reads `unavailable` until
//! a later attempt finds the broker again). A broker that came back with a
//! *different* policy is never rebound: the session composed against the
//! original proposal, so the link fails closed permanently with "restart
//! required" (the P2 restart-only contract), exactly as a root's own reload
//! would.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::client::{AuthorityConnection, ClientError, Inner};
use super::directory::AuthorityDirectory;
use crate::domain::inference_admission::WorkloadClass;
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Reconnect = Arc<dyn Fn() -> BoxFuture<Result<AuthorityConnection, String>> + Send + Sync>;
/// Called (from the I/O runtime or the reconnecting attempt) whenever the
/// link's health may have changed, so a projection can push it live.
pub type LinkChangeHook = Arc<dyn Fn() + Send + Sync>;

const RECONNECT_ATTEMPTS: u32 = 8;
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(2);

/// Health of a process's authority link as `get_state` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkHealth {
    /// Open socket and a live capability.
    Connected,
    /// Lost, and a root that will re-register on its next attempt.
    Reconnecting,
    /// Lost for good until something outside this process changes: a child
    /// (its parent respawns it), a root whose bounded reconnection was
    /// exhausted (retried on the next attempt), or a root facing a changed
    /// policy (restart required).
    Unavailable,
}

impl LinkHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Bounded exponential backoff with jitter (so many sessions do not all
/// hammer a restarting broker in lockstep).
#[derive(Debug, Clone, Copy)]
pub(super) struct Backoff {
    pub(super) attempts: u32,
    pub(super) base: Duration,
    pub(super) max: Duration,
}

impl Backoff {
    /// The delay after failed attempt `attempt` (0-based): `base * 2^attempt`
    /// capped at `max`, plus up to half of that again as jitter.
    pub(super) fn delay(&self, attempt: u32) -> Duration {
        use rand::Rng;
        let nominal = self
            .base
            .checked_mul(2u32.saturating_pow(attempt))
            .unwrap_or(self.max)
            .min(self.max);
        let jitter_cap = nominal.as_millis() as u64 / 2;
        let jitter = if jitter_cap == 0 {
            0
        } else {
            rand::thread_rng().gen_range(0..=jitter_cap)
        };
        nominal + Duration::from_millis(jitter)
    }
}

/// One process's live authority connection, reconnectable for a root.
pub struct AuthorityLink {
    current: Mutex<Arc<AuthorityConnection>>,
    reconnect: Option<Reconnect>,
    /// The policy this process composed against; a reconnection that finds a
    /// different one at the broker fails closed (restart required).
    expected: AdmissionRuntimeProposal,
    /// Serializes reconnection so concurrent attempts share one reconnect.
    gate: tokio::sync::Mutex<()>,
    backoff: Mutex<Backoff>,
    /// The last bounded reconnection gave up; cleared by a later success.
    exhausted: AtomicBool,
    /// Set once a reconnection met a changed policy: permanent for the process.
    restart_required: Mutex<Option<String>>,
    on_change: Arc<Mutex<Option<LinkChangeHook>>>,
}

impl std::fmt::Debug for AuthorityLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityLink")
            .field("can_reconnect", &self.reconnect.is_some())
            .field("health", &self.health())
            .finish_non_exhaustive()
    }
}

impl AuthorityLink {
    fn new(connection: AuthorityConnection, reconnect: Option<Reconnect>) -> Arc<Self> {
        let expected = connection.hello().proposal.clone();
        let on_change: Arc<Mutex<Option<LinkChangeHook>>> = Arc::new(Mutex::new(None));
        let connection = Arc::new(connection);
        watch(&connection, &on_change);
        Arc::new(Self {
            current: Mutex::new(connection),
            reconnect,
            expected,
            gate: tokio::sync::Mutex::new(()),
            backoff: Mutex::new(Backoff {
                attempts: RECONNECT_ATTEMPTS,
                base: RECONNECT_BASE_DELAY,
                max: RECONNECT_MAX_DELAY,
            }),
            exhausted: AtomicBool::new(false),
            restart_required: Mutex::new(None),
            on_change,
        })
    }

    /// A child link: no self-reconnect (the capability cannot be re-minted
    /// without the parent), so a lost or revoked connection fails closed.
    pub(super) fn child(connection: AuthorityConnection) -> Arc<Self> {
        Self::new(connection, None)
    }

    /// A root link that re-registers on the admission runtime after a loss.
    pub(super) fn root(
        connection: AuthorityConnection,
        directory: AuthorityDirectory,
        class: WorkloadClass,
        io_handle: tokio::runtime::Handle,
    ) -> Arc<Self> {
        let reconnect: Reconnect = Arc::new(move || {
            let directory = directory.clone();
            let io_handle = io_handle.clone();
            Box::pin(async move { reconnect_root(directory, class, io_handle).await })
        });
        Self::new(connection, Some(reconnect))
    }

    /// Install the hook told about every health change (one per link; a
    /// later call replaces it).
    pub fn on_change(&self, hook: LinkChangeHook) {
        *self.on_change.lock().expect("link hook") = Some(hook);
    }

    fn announce(&self) {
        let hook = self.on_change.lock().expect("link hook").clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// The current connection (whatever a reconnection last installed).
    pub(super) fn connection(&self) -> Arc<AuthorityConnection> {
        self.current.lock().expect("authority link").clone()
    }

    /// Open socket *and* a live capability: a revoked root is not connected.
    pub fn connected(&self) -> bool {
        let current = self.connection();
        current.is_open()
            && current.credential().is_some()
            && self.restart_required.lock().expect("link").is_none()
    }

    /// The authority epoch the current capability was minted in.
    pub fn epoch(&self) -> u64 {
        self.connection().hello().epoch
    }

    pub fn health(&self) -> LinkHealth {
        if self.connected() {
            LinkHealth::Connected
        } else if self.reconnect.is_some()
            && !self.exhausted.load(Ordering::Acquire)
            && self.restart_required.lock().expect("link").is_none()
        {
            LinkHealth::Reconnecting
        } else {
            LinkHealth::Unavailable
        }
    }

    #[cfg(test)]
    pub(super) fn set_backoff_for_test(&self, attempts: u32, base: Duration, max: Duration) {
        *self.backoff.lock().expect("backoff") = Backoff {
            attempts,
            base,
            max,
        };
    }

    /// The current connection's inner when it is open and still holds its
    /// capability; otherwise why it is unusable.
    fn usable(&self) -> Result<Arc<Inner>, ClientError> {
        let current = self.connection();
        if !current.is_open() {
            return Err(ClientError::Closed);
        }
        let inner = current.inner_arc();
        if !inner.credential_present() {
            return Err(ClientError::Unauthorized);
        }
        Ok(inner)
    }

    /// The inner of a live connection, reconnecting first if the current one
    /// closed or lost its capability. A child (no reconnect) fails closed.
    pub(super) async fn live_inner(&self) -> Result<Arc<Inner>, ClientError> {
        if let Some(reason) = self.restart_required.lock().expect("link").clone() {
            return Err(ClientError::Admission(reason));
        }
        let lost = match self.usable() {
            Ok(inner) => return Ok(inner),
            Err(error) => error,
        };
        let Some(reconnect) = self.reconnect.clone() else {
            return Err(match lost {
                ClientError::Unauthorized => ClientError::Admission(
                    "capability revoked by an authority reset; a child cannot re-register on its own — its parent must respawn it"
                        .into(),
                ),
                other => other,
            });
        };
        // Serialize: another attempt may have reconnected while we waited.
        let _guard = self.gate.lock().await;
        if let Ok(inner) = self.usable() {
            return Ok(inner);
        }
        let backoff = *self.backoff.lock().expect("backoff");
        let mut last = lost;
        for attempt in 0..backoff.attempts {
            match reconnect().await {
                Ok(connection) => {
                    if connection.hello().proposal != self.expected {
                        let reason = "admission policy at the broker differs from the one this session composed against; restart required".to_string();
                        // Drop the probe scope: never run under a policy this
                        // process did not compose against.
                        drop(connection);
                        *self.restart_required.lock().expect("link") = Some(reason.clone());
                        self.announce();
                        return Err(ClientError::Admission(reason));
                    }
                    let inner = connection.inner_arc();
                    let connection = Arc::new(connection);
                    watch(&connection, &self.on_change);
                    *self.current.lock().expect("authority link") = connection;
                    self.exhausted.store(false, Ordering::Release);
                    self.announce();
                    return Ok(inner);
                }
                Err(reason) => {
                    last = ClientError::Io(reason);
                    tokio::time::sleep(backoff.delay(attempt)).await;
                }
            }
        }
        self.exhausted.store(true, Ordering::Release);
        self.announce();
        Err(last)
    }
}

/// Watch one installed connection for closure or revocation on its own I/O
/// runtime and announce each to the link's hook, so health changes are seen
/// as they happen rather than at the next attempt.
fn watch(connection: &Arc<AuthorityConnection>, on_change: &Arc<Mutex<Option<LinkChangeHook>>>) {
    let inner = connection.inner_arc();
    let on_change = on_change.clone();
    let handle = inner.io_handle().clone();
    handle.spawn(async move {
        loop {
            let changed = inner.state_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let closed = inner.closed.load(Ordering::Acquire);
            if closed {
                break;
            }
            changed.await;
            let hook = on_change.lock().expect("link hook").clone();
            if let Some(hook) = hook {
                hook();
            }
        }
        let hook = on_change.lock().expect("link hook").clone();
        if let Some(hook) = hook {
            hook();
        }
    });
}

/// Re-open the client socket and register a fresh root scope on the admission
/// runtime (so the connection's I/O belongs to that runtime, as at first
/// negotiation). The owner token is read from disk here, so a restarted
/// broker's fresh token is honoured.
async fn reconnect_root(
    directory: AuthorityDirectory,
    class: WorkloadClass,
    io_handle: tokio::runtime::Handle,
) -> Result<AuthorityConnection, String> {
    io_handle
        .spawn(async move {
            let dir = AuthorityDirectory::existing(directory.path())
                .map_err(|e| format!("authority directory {} ({e})", directory.path().display()))?;
            let connection = AuthorityConnection::connect(&dir.client_socket())
                .await
                .map_err(|e| format!("reconnect to {} ({e})", dir.client_socket().display()))?;
            let credential = connection
                .register_root(class)
                .await
                .map_err(|e| format!("re-register root ({e})"))?;
            connection
                .bind(credential)
                .await
                .map_err(|e| format!("re-bind root ({e})"))?;
            Ok(connection)
        })
        .await
        .map_err(|e| format!("reconnect task ({e})"))?
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
