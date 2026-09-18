//! `AuthorityAdmin` over the authority's admin socket (#2024 S3). Wraps the
//! existing async `AdminConnection` behind the capability's synchronous port,
//! running each call on a short-lived current-thread runtime so the CLI's
//! per-command invocation needs no ambient runtime.

use std::path::Path;

use super::{AdminConnection, AuthorityDirectory};
use crate::application::admission::dto::{
    AuthorityAdminError, AuthorityGroupReport, AuthorityReport, ResetReport,
};
use crate::application::admission::ports::AuthorityAdmin;

#[derive(Debug, Default)]
pub struct SocketAuthorityAdmin;

impl SocketAuthorityAdmin {
    pub fn new() -> Self {
        Self
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
        runtime.block_on(async {
            let admin = AdminConnection::connect(&admin_socket).await.map_err(|e| {
                AuthorityAdminError::NotRunning {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                }
            })?;
            let status = admin
                .inspect()
                .await
                .map_err(|e| AuthorityAdminError::Failed {
                    directory: directory.to_path_buf(),
                    reason: e.to_string(),
                })?;
            Ok(AuthorityReport {
                directory: directory.to_path_buf(),
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

    fn reset(&self, directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        let admin_socket = AuthorityDirectory::admin_socket_for(directory);
        let runtime = Self::runtime().map_err(|reason| AuthorityAdminError::Failed {
            directory: directory.to_path_buf(),
            reason,
        })?;
        runtime.block_on(async {
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
