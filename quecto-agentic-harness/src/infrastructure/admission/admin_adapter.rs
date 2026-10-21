//! `AuthorityAdmin` over the authority's admin socket (#2024 S3). Wraps the
//! existing async `AdminConnection` behind the capability's synchronous port,
//! running each call on a short-lived current-thread runtime so the CLI's
//! per-command invocation needs no ambient runtime. Every call is bounded
//! (#2615): a broker that is stopped (SIGSTOP) or wedged answers nothing,
//! and is reported as failing, never waited on forever.

use std::path::Path;
use std::time::Duration;

use super::{AdminConnection, AuthorityDirectory};
use crate::application::admission::dto::{
    authority::AuthorityAdminError, authority::AuthorityGroupReport, authority::AuthorityReport,
    authority::ResetReport,
};
use crate::application::admission::ports::AuthorityAdmin;

/// How long one admin call (connect, hello, operation) may take.
pub const ADMIN_CALL_BOUND: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct SocketAuthorityAdmin {
    bound: Duration,
}

impl Default for SocketAuthorityAdmin {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketAuthorityAdmin {
    pub fn new() -> Self {
        Self {
            bound: ADMIN_CALL_BOUND,
        }
    }

    /// Bound each call by `bound` instead (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn bounded_by(bound: Duration) -> Self {
        Self { bound }
    }

    /// Run one admin call within the bound; past it the broker is there
    /// but not answering its owner.
    fn within<T>(
        &self,
        runtime: &tokio::runtime::Runtime,
        directory: &Path,
        call: impl std::future::Future<Output = Result<T, AuthorityAdminError>>,
    ) -> Result<T, AuthorityAdminError> {
        runtime
            .block_on(async { tokio::time::timeout(self.bound, call).await })
            .unwrap_or_else(|_| {
                Err(AuthorityAdminError::Unanswered {
                    directory: directory.to_path_buf(),
                    bound_secs: self.bound.as_secs(),
                })
            })
    }

    fn runtime() -> Result<tokio::runtime::Runtime, String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("admin runtime: {e}"))
    }
}

impl AuthorityAdmin for SocketAuthorityAdmin {
    fn inspect(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        let admin_socket = AuthorityDirectory::admin_socket_for(directory);
        let runtime = Self::runtime().map_err(|reason| AuthorityAdminError::Failed {
            directory: directory.to_path_buf(),
            reason,
        })?;
        self.within(&runtime, directory, async {
            let admin = AdminConnection::connect(&admin_socket).await.map_err(|e| {
                AuthorityAdminError::NotRunning {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;
            let read = admin
                .status()
                .await
                .map_err(|e| AuthorityAdminError::Failed {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                })?;
            let (status, on_demand, instance) = (read.status, read.on_demand, read.instance);
            Ok(AuthorityReport {
                directory: directory.to_path_buf(),
                on_demand,
                instance,
                epoch: status.epoch,
                journal_healthy: status.journal_healthy,
                live_scopes: status.live_scopes,
                groups: status
                    .groups
                    .into_iter()
                    .map(|(id, snapshot)| {
                        (
                            id.as_str().to_owned(),
                            AuthorityGroupReport {
                                active: snapshot.active,
                                queued: snapshot.queued,
                                uncertain: snapshot.uncertain,
                                cooldown_until_ms: snapshot.cooldown_until,
                                unavailable: snapshot.unavailable,
                            },
                        )
                    })
                    .collect(),
            })
        })
    }

    fn stop(&self, directory: &Path) -> Result<(), AuthorityAdminError> {
        let admin_socket = AuthorityDirectory::admin_socket_for(directory);
        let runtime = Self::runtime().map_err(|reason| AuthorityAdminError::Failed {
            directory: directory.to_path_buf(),
            reason,
        })?;
        self.within(&runtime, directory, async {
            let admin = AdminConnection::connect(&admin_socket).await.map_err(|e| {
                AuthorityAdminError::NotRunning {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;
            admin.stop().await.map_err(|e| AuthorityAdminError::Failed {
                directory: directory.to_path_buf(),
                reason: e.to_string(),
            })
        })
    }

    fn reset(&self, directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        let admin_socket = AuthorityDirectory::admin_socket_for(directory);
        let runtime = Self::runtime().map_err(|reason| AuthorityAdminError::Failed {
            directory: directory.to_path_buf(),
            reason,
        })?;
        self.within(&runtime, directory, async {
            let admin = AdminConnection::connect(&admin_socket).await.map_err(|e| {
                AuthorityAdminError::NotRunning {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;
            let epoch = admin
                .reset()
                .await
                .map_err(|e| AuthorityAdminError::Failed {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                })?;
            Ok(ResetReport {
                directory: directory.to_path_buf(),
                epoch,
            })
        })
    }
}

#[cfg(test)]
#[path = "admin_adapter_tests.rs"]
mod tests;
