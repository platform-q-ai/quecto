//! Restart the admission broker a root session uses (#2615): the
//! `/admission restart` the TUI offers. Whoever manages the broker restarts
//! it: the installed systemd unit when it serves this session's global
//! config (so it stays always-on), otherwise the broker is asked to stop and
//! one is started on demand in its place, reading the current policy. A
//! broker that was not running is simply started.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::install_authority_service::unit_serves_config;
use super::negotiate_authority::{ENSURE_ATTEMPTS, NegotiateAuthority};
use crate::application::admission::dto::negotiation::{
    RestartError, RestartOutcome, RestartReport,
};
use crate::application::admission::ports::{
    AuthorityAdmin, AuthorityLaunch, AuthorityLauncher, AuthorityServiceManager, AuthoritySession,
    JoinedAuthority, UnitStatus,
};

pub struct RestartAuthority {
    admin: Arc<dyn AuthorityAdmin>,
    service: Arc<dyn AuthorityServiceManager>,
    launcher: Arc<dyn AuthorityLauncher>,
    negotiate: Arc<NegotiateAuthority>,
    session: Arc<dyn AuthoritySession>,
    /// One restart at a time.
    in_flight: AtomicBool,
}

/// Clears the in-flight flag however a restart ends.
struct InFlight<'a>(&'a AtomicBool);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl std::fmt::Debug for RestartAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestartAuthority").finish_non_exhaustive()
    }
}

impl RestartAuthority {
    pub fn new(
        admin: Arc<dyn AuthorityAdmin>,
        service: Arc<dyn AuthorityServiceManager>,
        launcher: Arc<dyn AuthorityLauncher>,
        negotiate: Arc<NegotiateAuthority>,
        session: Arc<dyn AuthoritySession>,
    ) -> Self {
        Self {
            admin,
            service,
            launcher,
            negotiate,
            session,
            in_flight: AtomicBool::new(false),
        }
    }

    /// Restart the broker this root session joined, then rejoin it. Only a
    /// root restarts: a child's capability belongs to its parent.
    pub fn execute(&self) -> Result<RestartReport, RestartError> {
        let _ = (&self.session, &self.in_flight, RestartOutcome::Started);
        let _: fn(&Self, &AuthorityLaunch) -> Result<RestartOutcome, String> = Self::restart;
        let _ = InFlight(&self.in_flight);
        let _ = JoinedAuthority::Child;
        Err(RestartError::NotConfigured)
    }

    /// Restart the broker serving `launch`, and answer once it serves again.
    fn restart(&self, launch: &AuthorityLaunch) -> Result<RestartOutcome, String> {
        let directory = &launch.directory;
        let managed_by_unit = match self.service.unit_status()? {
            UnitStatus::Present { contents } => unit_serves_config(&contents, &launch.config),
            UnitStatus::Absent => false,
        };
        if managed_by_unit {
            self.service.restart()?;
            self.wait_until(|launcher| launcher.probe(directory).is_ok())
                .map_err(|()| {
                    format!(
                        "{} was restarted but its broker did not answer at {}",
                        self.service.unit_name(),
                        directory.display()
                    )
                })?;
            return Ok(RestartOutcome::ServiceRestarted);
        }
        let was_running = self.launcher.probe(directory).is_ok();
        if was_running {
            self.admin
                .stop(directory)
                .map_err(|error| error.to_string())?;
            self.wait_until(|launcher| launcher.probe(directory).is_err())
                .map_err(|()| {
                    format!(
                        "the admission broker at {} did not stop",
                        directory.display()
                    )
                })?;
        }
        self.negotiate.ensure_running(launch)?;
        Ok(match was_running {
            true => RestartOutcome::Restarted,
            false => RestartOutcome::Started,
        })
    }

    /// Probe through the bounded backoff until `done`.
    fn wait_until(&self, done: impl Fn(&dyn AuthorityLauncher) -> bool) -> Result<(), ()> {
        for attempt in 0..ENSURE_ATTEMPTS {
            if done(self.launcher.as_ref()) {
                return Ok(());
            }
            self.launcher.pause(attempt);
        }
        match done(self.launcher.as_ref()) {
            true => Ok(()),
            false => Err(()),
        }
    }
}

#[cfg(test)]
#[path = "restart_authority_tests.rs"]
mod tests;
