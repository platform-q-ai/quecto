//! Uninstall the broker's systemd user service (#2024 S3), idempotently,
//! reporting exactly what it did.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::admission::dto::{ServiceAction, ServiceReport};
use crate::application::admission::ports::AuthorityServiceManager;

pub struct UninstallAuthorityService {
    manager: Arc<dyn AuthorityServiceManager>,
}

impl UninstallAuthorityService {
    pub fn new(manager: Arc<dyn AuthorityServiceManager>) -> Self {
        Self { manager }
    }

    pub fn execute(&self, directory: PathBuf, dry_run: bool) -> Result<ServiceReport, String> {
        let unit_path = self.manager.unit_path();
        let unit = self.manager.unit_name();
        let present = self.manager.unit_status()?;
        let unit_present = matches!(
            present,
            crate::application::admission::ports::UnitStatus::Present { .. }
        );
        let mut actions = Vec::new();

        if dry_run {
            if unit_present {
                actions.push(ServiceAction::Planned {
                    description: format!("disable --now {unit} and remove {}", unit_path.display()),
                });
            } else {
                actions.push(ServiceAction::Planned {
                    description: format!("nothing to remove ({} absent)", unit_path.display()),
                });
            }
            return Ok(ServiceReport {
                directory,
                unit,
                unit_path,
                dry_run: true,
                actions,
            });
        }

        if !unit_present {
            actions.push(ServiceAction::NoUnitToRemove {
                path: unit_path.clone(),
            });
            return Ok(ServiceReport {
                directory,
                unit,
                unit_path,
                dry_run: false,
                actions,
            });
        }

        // Disable before removing so systemd forgets the enablement symlink.
        if self.manager.disable_now()? {
            actions.push(ServiceAction::DisabledAndStopped { unit: unit.clone() });
        } else {
            actions.push(ServiceAction::NothingToDisable { unit: unit.clone() });
        }
        if self.manager.remove_unit()? {
            actions.push(ServiceAction::RemovedUnit {
                path: unit_path.clone(),
            });
        } else {
            actions.push(ServiceAction::NoUnitToRemove {
                path: unit_path.clone(),
            });
        }
        self.manager.daemon_reload()?;
        actions.push(ServiceAction::DaemonReloaded);

        Ok(ServiceReport {
            directory,
            unit,
            unit_path,
            dry_run: false,
            actions,
        })
    }
}

impl std::fmt::Debug for UninstallAuthorityService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UninstallAuthorityService")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "uninstall_authority_service_tests.rs"]
mod tests;
