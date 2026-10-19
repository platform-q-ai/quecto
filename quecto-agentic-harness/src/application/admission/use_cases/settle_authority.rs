//! One settling turn of the running authority (#1910): dispatch what the
//! policy allows, answer every waiter that is no longer queued, and report
//! the attempts that must stop and when the policy next needs a turn.

use super::dispatch_admissions::DispatchAdmissions;
use crate::application::admission::dto::broker::{AuthorityError, Settlement};
use crate::application::admission::ports::inference_authority_ports::AdmissionSecretSource;
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::{
    RequestId, RequestState, TerminalOutcome,
};

/// How soon a turn retries when the policy cannot say when it next needs one.
const WAKE_RETRY_MS: u64 = 50;
/// How often held work probes the journal during an outage: it can proceed
/// only after a successful write, so it does not wait for queue deadlines.
const JOURNAL_PROBE_INTERVAL_MS: u64 = 250;

pub struct SettleAuthority {
    dispatch: DispatchAdmissions,
}

impl SettleAuthority {
    pub fn new(dispatch: DispatchAdmissions) -> Self {
        Self { dispatch }
    }

    /// Settle the authority at `now` for the requests still `waiting` for an
    /// answer. A waiter that observes `Cancelled` while the journal is
    /// unhealthy was withdrawn by dispatch, not cancelled by its client (a
    /// client's cancel answers its own waiter), so it is answered
    /// `JournalUnavailable`.
    pub fn execute<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        waiting: &[RequestId],
        now: u64,
    ) -> Settlement {
        let dispatch_failure = match self.dispatch.execute(authority, now) {
            // An admission refusal (a quarantined group, a full scope) is
            // routine; only a failure that closes dispatch is reported.
            Ok(_) | Err(AuthorityError::Admission(_)) => None,
            Err(failure) => Some(failure),
        };
        let journal_healthy = authority.journal_healthy();
        let answers = waiting
            .iter()
            .filter_map(|&id| {
                let answer = match authority.observe(id, now) {
                    Ok(RequestState::Queued { .. }) => return None,
                    Ok(RequestState::Terminal(TerminalOutcome::Cancelled)) if !journal_healthy => {
                        Err(AuthorityError::JournalUnavailable)
                    }
                    observed => observed,
                };
                Some((id, answer))
            })
            .collect::<Vec<_>>();
        debug_assert!(
            answers.iter().all(|(id, _)| waiting.contains(id)),
            "only a waiting request is answered"
        );
        let cancellation_due = authority.cancellation_due(now).unwrap_or_default();
        let wake_at = wake_at(authority.next_wake(now), journal_healthy, now);
        debug_assert!(
            journal_healthy || wake_at.is_some(),
            "an unhealthy journal is always probed again"
        );
        Settlement {
            dispatch_failure,
            answers,
            cancellation_due,
            wake_at,
            journal_healthy,
        }
    }
}

/// When the authority next needs a turn: when the policy says, soon when it
/// cannot say, and no later than the next journal probe during an outage.
fn wake_at(
    next: Result<Option<u64>, AuthorityError>,
    journal_healthy: bool,
    now: u64,
) -> Option<u64> {
    let wake = match next {
        Ok(wake) => wake,
        Err(_) => Some(now.saturating_add(WAKE_RETRY_MS)),
    };
    if journal_healthy {
        return wake;
    }
    let probe = now.saturating_add(JOURNAL_PROBE_INTERVAL_MS);
    Some(wake.map_or(probe, |wake| wake.min(probe)))
}
