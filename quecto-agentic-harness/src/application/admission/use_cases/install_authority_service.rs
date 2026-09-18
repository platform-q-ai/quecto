//! Install the broker as a systemd *user* service (#2024 S3), idempotently,
//! reporting exactly what it did. The unit template lives here (policy); the
//! manager port persists the file and runs `systemctl --user`.

use std::sync::Arc;

use crate::application::admission::dto::{InstallServiceRequest, ServiceAction, ServiceReport};
use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};

pub struct InstallAuthorityService {
    manager: Arc<dyn AuthorityServiceManager>,
}

impl InstallAuthorityService {
    pub fn new(manager: Arc<dyn AuthorityServiceManager>) -> Self {
        Self { manager }
    }

    /// The systemd user unit that runs the broker with an explicit config so
    /// it never depends on a working directory, and restarts on failure.
    pub fn unit_contents(request: &InstallServiceRequest) -> String {
        format!(
            "[Unit]\n\
             Description=Quecto shared inference-admission broker\n\
             After=default.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart={binary} admission-broker run --config {config}\n\
             Restart=on-failure\n\
             RestartSec=2\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            binary = request.binary.display(),
            config = request.config.display(),
        )
    }

    pub fn execute(&self, request: InstallServiceRequest) -> Result<ServiceReport, String> {
        let unit_path = self.manager.unit_path();
        let unit = self.manager.unit_name();
        let contents = Self::unit_contents(&request);
        let mut actions = Vec::new();

        let status = self.manager.unit_status()?;
        let unit_matches = matches!(&status, UnitStatus::Present { contents: c } if *c == contents);

        if request.dry_run {
            actions.push(match &status {
                UnitStatus::Present { .. } if unit_matches => ServiceAction::Planned {
                    description: format!("unit {} already up to date", unit_path.display()),
                },
                UnitStatus::Present { .. } => ServiceAction::Planned {
                    description: format!("rewrite unit {}", unit_path.display()),
                },
                UnitStatus::Absent => ServiceAction::Planned {
                    description: format!("write unit {}", unit_path.display()),
                },
            });
            actions.push(ServiceAction::Planned {
                description: format!("systemctl --user daemon-reload && enable --now {unit}"),
            });
            return Ok(ServiceReport {
                directory: request.directory,
                unit,
                unit_path,
                dry_run: true,
                actions,
            });
        }

        if unit_matches {
            actions.push(ServiceAction::UnitUnchanged {
                path: unit_path.clone(),
            });
        } else {
            self.manager.write_unit(&ServiceUnitSpec { contents })?;
            actions.push(ServiceAction::WroteUnit {
                path: unit_path.clone(),
            });
        }
        // daemon-reload and enable --now are always run: they are themselves
        // idempotent, and re-running install must converge a partially applied
        // install (unit present but never enabled).
        self.manager.daemon_reload()?;
        actions.push(ServiceAction::DaemonReloaded);
        self.manager.enable_now()?;
        actions.push(ServiceAction::EnabledAndStarted { unit: unit.clone() });

        Ok(ServiceReport {
            directory: request.directory,
            unit,
            unit_path,
            dry_run: false,
            actions,
        })
    }
}

impl std::fmt::Debug for InstallAuthorityService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallAuthorityService")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "install_authority_service_tests.rs"]
mod tests;
