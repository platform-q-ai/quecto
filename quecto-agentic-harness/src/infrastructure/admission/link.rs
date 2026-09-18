//! Reconnecting authority link (#2024 S3): the one connection every gate of a
//! process routes through, so a broker restart or `reset` no longer forces the
//! agent to restart.
//!
//! A root can re-register on its own (the owner token is on disk and readable
//! after a restart), so its link reconnects: it re-opens the client socket,
//! registers a fresh root scope and binds it, then swaps the connection in.
//! A child cannot — its capability was minted by the parent and is gone once
//! the epoch or the broker changed — so a child link has no reconnect and its
//! next attempt fails closed with a clear error (never a silent bypass); the
//! parent respawns it. Reconnect is serialized and bounded: concurrent
//! attempts share one reconnection, and it gives up after a bounded number of
//! tries rather than blocking a turn forever.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::client::{AuthorityConnection, ClientError, Inner};
use super::directory::AuthorityDirectory;
use crate::domain::inference_admission::WorkloadClass;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Reconnect = Arc<dyn Fn() -> BoxFuture<Result<AuthorityConnection, String>> + Send + Sync>;

const RECONNECT_ATTEMPTS: u32 = 8;
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(2);

/// One process's live authority connection, reconnectable for a root.
pub struct AuthorityLink {
    current: Mutex<Arc<AuthorityConnection>>,
    reconnect: Option<Reconnect>,
    /// Serializes reconnection so concurrent attempts share one reconnect.
    gate: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for AuthorityLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityLink")
            .field("can_reconnect", &self.reconnect.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthorityLink {
    /// A child link: no self-reconnect (the capability cannot be re-minted
    /// without the parent), so a lost connection fails closed.
    pub(super) fn child(connection: AuthorityConnection) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(Arc::new(connection)),
            reconnect: None,
            gate: tokio::sync::Mutex::new(()),
        })
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
        Arc::new(Self {
            current: Mutex::new(Arc::new(connection)),
            reconnect: Some(reconnect),
            gate: tokio::sync::Mutex::new(()),
        })
    }

    /// The current connection (whatever a reconnection last installed).
    pub(super) fn connection(&self) -> Arc<AuthorityConnection> {
        self.current.lock().expect("authority link").clone()
    }

    /// Test-only: force the current connection closed, as a killed broker
    /// would, so the reconnect path is exercised deterministically.
    #[cfg(test)]
    pub(super) fn close_current_for_test(&self) {
        self.current
            .lock()
            .expect("authority link")
            .close_for_test();
    }

    /// The inner of a live connection, reconnecting first if the current one
    /// closed. A child (no reconnect) whose connection closed fails closed.
    pub(super) async fn live_inner(&self) -> Result<Arc<Inner>, ClientError> {
        {
            let current = self.current.lock().expect("authority link");
            if current.is_open() {
                return Ok(current.inner_arc());
            }
        }
        let Some(reconnect) = self.reconnect.clone() else {
            return Err(ClientError::Closed);
        };
        // Serialize: another attempt may have reconnected while we waited.
        let _guard = self.gate.lock().await;
        {
            let current = self.current.lock().expect("authority link");
            if current.is_open() {
                return Ok(current.inner_arc());
            }
        }
        let mut delay = RECONNECT_BASE_DELAY;
        let mut last = ClientError::Closed;
        for _ in 0..RECONNECT_ATTEMPTS {
            match reconnect().await {
                Ok(connection) => {
                    let inner = connection.inner_arc();
                    *self.current.lock().expect("authority link") = Arc::new(connection);
                    return Ok(inner);
                }
                Err(reason) => {
                    last = ClientError::Io(reason);
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RECONNECT_MAX_DELAY);
                }
            }
        }
        Err(last)
    }
}

/// Re-open the client socket and register a fresh root scope on the admission
/// runtime (so the connection's I/O belongs to that runtime, as at first
/// negotiation).
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
