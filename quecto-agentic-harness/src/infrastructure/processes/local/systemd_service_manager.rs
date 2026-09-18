//! `AuthorityServiceManager` over a systemd *user* unit (#2024 S3). The unit
//! lives under `$XDG_CONFIG_HOME/systemd/user` (or `~/.config/systemd/user`);
//! lifecycle is `systemctl --user`. The contract test puts a fake `systemctl`
//! on the PATH, so the binary is looked up from the PATH, never hard-coded.

use std::path::PathBuf;
use std::process::Command;

use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};

pub const UNIT_NAME: &str = "quecto-admission-broker.service";

/// Manager rooted at an explicit user-unit directory. Composition builds it
/// from the process's `$XDG_CONFIG_HOME`/`$HOME`; tests point it at a tempdir.
#[derive(Debug, Clone)]
pub struct SystemdUserServiceManager {
    unit_dir: PathBuf,
    systemctl: PathBuf,
}

impl SystemdUserServiceManager {
    pub fn new(unit_dir: PathBuf) -> Self {
        Self {
            unit_dir,
            systemctl: PathBuf::from("systemctl"),
        }
    }

    /// Point the manager at a specific `systemctl` binary (a fake one in
    /// tests, so the contract test needs no PATH mutation of the whole
    /// process).
    pub fn with_systemctl_binary(mut self, systemctl: PathBuf) -> Self {
        self.systemctl = systemctl;
        self
    }

    /// Resolve the systemd user-unit directory from the environment, honoring
    /// `$XDG_CONFIG_HOME` then `$HOME`.
    pub fn for_user_environment() -> Result<Self, String> {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                crate::infrastructure::tools::path_utils::home_dir()
                    .map(|home| home.join(".config"))
            })
            .ok_or_else(|| {
                "cannot resolve the systemd user unit directory: no $XDG_CONFIG_HOME or $HOME"
                    .to_string()
            })?;
        Ok(Self::new(config_home.join("systemd").join("user")))
    }

    fn unit_file(&self) -> PathBuf {
        self.unit_dir.join(UNIT_NAME)
    }

    fn systemctl(&self, args: &[&str]) -> Result<std::process::Output, String> {
        Command::new(&self.systemctl)
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| format!("running `systemctl --user {}`: {e}", args.join(" ")))
    }

    fn run_checked(&self, args: &[&str]) -> Result<(), String> {
        let output = self.systemctl(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "`systemctl --user {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

impl AuthorityServiceManager for SystemdUserServiceManager {
    fn unit_path(&self) -> PathBuf {
        self.unit_file()
    }

    fn unit_name(&self) -> String {
        UNIT_NAME.to_string()
    }

    fn unit_status(&self) -> Result<UnitStatus, String> {
        match std::fs::read_to_string(self.unit_file()) {
            Ok(contents) => Ok(UnitStatus::Present { contents }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UnitStatus::Absent),
            Err(error) => Err(format!(
                "reading unit {}: {error}",
                self.unit_file().display()
            )),
        }
    }

    fn write_unit(&self, spec: &ServiceUnitSpec) -> Result<(), String> {
        std::fs::create_dir_all(&self.unit_dir)
            .map_err(|e| format!("creating unit directory {}: {e}", self.unit_dir.display()))?;
        crate::infrastructure::atomic_write::atomic_write(
            &self.unit_file(),
            spec.contents.as_bytes(),
            Some(0o644),
        )
        .map_err(|e| format!("writing unit {}: {e}", self.unit_file().display()))
    }

    fn remove_unit(&self) -> Result<bool, String> {
        match std::fs::remove_file(self.unit_file()) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "removing unit {}: {error}",
                self.unit_file().display()
            )),
        }
    }

    fn daemon_reload(&self) -> Result<(), String> {
        self.run_checked(&["daemon-reload"])
    }

    fn enable_now(&self) -> Result<(), String> {
        self.run_checked(&["enable", "--now", UNIT_NAME])
    }

    fn disable_now(&self) -> Result<bool, String> {
        // `disable --now` on a not-enabled unit is not a hard failure: report
        // it as "nothing to disable" so uninstall stays idempotent.
        let output = self.systemctl(&["disable", "--now", UNIT_NAME])?;
        if output.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not loaded")
                || stderr.contains("does not exist")
                || stderr.contains("No such file")
                || stderr.contains("not-found")
            {
                Ok(false)
            } else {
                Err(format!(
                    "`systemctl --user disable --now {UNIT_NAME}` failed: {}",
                    stderr.trim()
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "systemd_service_manager_tests.rs"]
mod tests;
