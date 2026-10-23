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
use crate::application::admission::dto::authority::{AuthorityAdminError, AuthorityReport};
use crate::application::admission::dto::negotiation::{
    Restart, RestartError, RestartOutcome, RestartReport,
};
use crate::application::admission::ports::{
    AuthorityAdmin, AuthorityLaunch, AuthorityLauncher, AuthorityServiceManager, AuthoritySession,
    JoinedAuthority, RejoinRefusal, UnitStatus,
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
    /// root restarts: a child's capability belongs to its parent. A broker
    /// that other sessions or sub-agents share is restarted only once
    /// confirmed for at least as many of them as are registered now
    /// (`confirmed`: the count the user acknowledged): their sub-agents
    /// fail and must be respawned, and their in-flight requests fail once.
    pub fn execute(&self, confirmed: Option<usize>) -> Result<Restart, RestartError> {
        let launch = match self.session.joined() {
            Some(JoinedAuthority::Root(launch)) => launch,
            Some(JoinedAuthority::Child) => return Err(RestartError::Child),
            None => return Err(RestartError::NotConfigured),
        };
        if self.in_flight.swap(true, Ordering::AcqRel) {
            return Err(RestartError::InProgress);
        }
        let _in_flight = InFlight(&self.in_flight);
        // Only a broker that is not there at all counts as "nothing
        // running": one that does not answer its owner is not replaced on
        // a guess.
        let running = match self.admin.inspect(&launch.directory) {
            Ok(report) => Some(report),
            Err(AuthorityAdminError::NotRunning { .. }) => None,
            // Only a broker that answers nothing at all (stopped, wedged) is
            // restarted around, and only by its unit.
            Err(unanswered @ AuthorityAdminError::Unanswered { .. }) => {
                return self
                    .restart_unanswering(&launch, &unanswered.to_string())
                    .map_err(RestartError::Restart)
                    .and_then(|outcome| self.rejoin(outcome));
            }
            Err(AuthorityAdminError::Failed { reason, .. }) => {
                return Err(RestartError::Restart(format!(
                    "the admission broker at {} could not report who shares it ({reason}); nothing was restarted",
                    launch.directory.display()
                )));
            }
        };
        // This root is one live scope; every other is a session or a
        // sub-agent the restart breaks.
        let others = running
            .as_ref()
            .map_or(0, |report| report.live_scopes.saturating_sub(1));
        let acknowledged = matches!(confirmed, Some(count) if count >= others);
        if others > 0 && !acknowledged {
            return Ok(Restart::Unconfirmed { others });
        }
        let outcome = self
            .restart(&launch, running.as_ref())
            .map_err(RestartError::Restart)?;
        self.rejoin(outcome)
    }

    /// Rejoin the restarted broker and report.
    fn rejoin(&self, outcome: RestartOutcome) -> Result<Restart, RestartError> {
        // A broker back with a different policy is the restart this command
        // offers the session (#2615): reported, not refused.
        let session_restart_required = match self.session.rejoin() {
            Ok(()) => false,
            Err(RejoinRefusal::PolicyChanged) => true,
            Err(RejoinRefusal::Failed(reason)) => {
                return Err(RestartError::Rejoin { outcome, reason });
            }
        };
        Ok(Restart::Done(RestartReport {
            outcome,
            health: self.session.health(),
            session_restart_required,
        }))
    }

    /// Restart the broker serving `launch` (`running` when one answered),
    /// and answer once a **new** broker serves: one whose instance differs
    /// from the old one's. The installed unit is restarted in place only
    /// when it is active and is what serves; otherwise whatever serves is
    /// stopped first, and the unit (when installed for this config) or an
    /// on-demand broker is started in its place.
    /// The broker does not answer its owner: only an active unit serving
    /// this config can be restarted around it (systemd ends its process);
    /// anything else is refused, naming how to stop it.
    fn restart_unanswering(
        &self,
        launch: &AuthorityLaunch,
        reason: &str,
    ) -> Result<RestartOutcome, String> {
        let directory = &launch.directory;
        let unit_installed = match self.service.unit_status()? {
            UnitStatus::Present { contents } => unit_serves_config(&contents, &launch.config),
            UnitStatus::Absent => false,
        };
        if unit_installed && self.service.is_active()? {
            self.service.restart()?;
            self.wait_until(|| self.admin.inspect(directory).is_ok())
                .map_err(|()| {
                    format!(
                        "{} was restarted but no broker answered at {}",
                        self.service.unit_name(),
                        directory.display()
                    )
                })?;
            return Ok(RestartOutcome::ServiceRestarted);
        }
        Err(format!(
            "the admission broker at {} does not answer its owner ({reason}); stop it (`fuser -k -TERM {}/authority.lock`) and restart again",
            directory.display(),
            directory.display()
        ))
    }

    fn restart(
        &self,
        launch: &AuthorityLaunch,
        running: Option<&AuthorityReport>,
    ) -> Result<RestartOutcome, String> {
        let directory = &launch.directory;
        let unit_installed = match self.service.unit_status()? {
            UnitStatus::Present { contents } => unit_serves_config(&contents, &launch.config),
            UnitStatus::Absent => false,
        };
        let before = running.map(|report| report.instance);
        let unit_serving = match running {
            Some(report) if unit_installed && !report.on_demand => self.service.is_active()?,
            _ => false,
        };
        if running.is_some() && !unit_serving {
            self.admin
                .stop(directory)
                .map_err(|error| error.to_string())?;
            self.wait_until(|| self.launcher.probe(directory).is_err())
                .map_err(|()| {
                    format!(
                        "the admission broker at {} did not stop",
                        directory.display()
                    )
                })?;
        }
        if unit_installed {
            // A unit that lost the lock to a broker still exiting exits Busy
            // and is not restarted by systemd: start it once more.
            for _ in 0..UNIT_STARTS {
                self.service.restart()?;
                if self
                    .wait_until(|| self.answers_anew(directory, before))
                    .is_ok()
                {
                    return Ok(RestartOutcome::ServiceRestarted);
                }
            }
            // The unit did not serve: a broker started on demand does.
        }
        self.negotiate.ensure_running(launch)?;
        self.wait_until(|| self.answers_anew(directory, before))
            .map_err(|()| match unit_installed {
                true => format!(
                    "{} was restarted but no new broker answered at {}, nor did one started on demand",
                    self.service.unit_name(),
                    directory.display()
                ),
                false => format!(
                    "no new admission broker answered at {}",
                    directory.display()
                ),
            })?;
        Ok(match running {
            Some(_) => RestartOutcome::Restarted,
            None => RestartOutcome::Started,
        })
    }

    /// Whether a broker answers at `directory` that is not the one before.
    fn answers_anew(&self, directory: &std::path::Path, before: Option<u64>) -> bool {
        self.admin
            .inspect(directory)
            .is_ok_and(|report| Some(report.instance) != before)
    }

    /// Check through the bounded backoff until `done`.
    fn wait_until(&self, done: impl Fn() -> bool) -> Result<(), ()> {
        for attempt in 0..ENSURE_ATTEMPTS {
            if done() {
                return Ok(());
            }
            self.launcher.pause(attempt);
        }
        match done() {
            true => Ok(()),
            false => Err(()),
        }
    }
}

/// How many times the unit is started before the restart gives up.
const UNIT_STARTS: usize = 2;

#[cfg(test)]
#[path = "restart_authority_tests.rs"]
mod tests;
