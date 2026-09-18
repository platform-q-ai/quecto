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
    ///
    /// Ordering: `WantedBy=default.target` already makes default.target run
    /// *after* this service, so the unit must not also be `After=default.target`
    /// (a cycle systemd resolves by dropping a job at login); it orders after
    /// `basic.target` instead. Exit status 3 is `Busy` — another broker holds
    /// the directory lock — and is excluded from the restart loop, since
    /// restarting against a held lock every `RestartSec` fixes nothing.
    pub fn unit_contents(request: &InstallServiceRequest) -> String {
        format!(
            "[Unit]\n\
             Description=Quecto shared inference-admission broker\n\
             After=basic.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart={binary} admission-broker run --config {config}\n\
             Restart=on-failure\n\
             RestartSec=2\n\
             RestartPreventExitStatus=3\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            binary = quote_unit_path(&request.binary),
            config = quote_unit_path(&request.config),
        )
    }

    pub fn execute(&self, request: InstallServiceRequest) -> Result<ServiceReport, String> {
        let unit_path = self.manager.unit_path();
        let unit = self.manager.unit_name();
        let contents = Self::unit_contents(&request);
        let mut actions = Vec::new();

        let status = self.manager.unit_status()?;
        let unit_matches = matches!(&status, UnitStatus::Present { contents: c } if *c == contents);

        let rewrite = matches!(&status, UnitStatus::Present { .. }) && !unit_matches;
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
            if rewrite {
                actions.push(ServiceAction::Planned {
                    description: format!("systemctl --user restart {unit} (unit changed)"),
                });
            }
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
        // `enable --now` leaves an already-running service on its old unit;
        // a rewritten unit is only live once the service restarts on it.
        if rewrite {
            self.manager.restart()?;
            actions.push(ServiceAction::Restarted { unit: unit.clone() });
        }

        Ok(ServiceReport {
            directory: request.directory,
            unit,
            unit_path,
            dry_run: false,
            actions,
        })
    }
}

/// Quote a path for a systemd `ExecStart=` line: double quotes with the
/// characters systemd unquotes (`\\`, `"`) escaped, so spaces survive.
fn quote_unit_path(path: &std::path::Path) -> String {
    let raw = path.display().to_string();
    let escaped: String = raw
        .chars()
        .flat_map(|c| match c {
            '\\' | '"' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    format!("\"{escaped}\"")
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
