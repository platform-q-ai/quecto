//! One settling turn of the running authority (#1910): dispatch what the
//! policy allows, release lost scopes whose work has cleared, answer every
//! waiter that is no longer queued, and report the attempts that must stop
//! and when the policy next needs a turn.

use super::dispatch_admissions::DispatchAdmissions;
use crate::application::admission::dto::broker::{AuthorityError, Settlement};
use crate::application::admission::ports::inference_authority_ports::AdmissionSecretSource;
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::domain::admission::value_objects::inference_admission::{
    RequestId, RequestState, TerminalOutcome,
};

pub struct SettleAuthority {
    dispatch: DispatchAdmissions,
}

impl SettleAuthority {
    pub fn new(dispatch: DispatchAdmissions) -> Self {
        Self { dispatch }
    }

    /// The authority's last turn as it stops: its ledger made durable with
    /// the lease bounds as they stand now (#2664).
    pub fn checkpoint_on_stop<S: AdmissionSecretSource>(
        &self,
        authority: &mut AdmissionAuthority<S>,
        now: u64,
    ) -> Result<(), AuthorityError> {
        self.dispatch.checkpoint(authority, now)
    }

    /// Settle the authority at `now` for the requests still `waiting` for an
    /// answer. A waiter that observes `Cancelled` while the journal is
    /// unhealthy was withdrawn by dispatch, not cancelled by its client (a
    /// client's cancel answers its own waiter), so it is answered
    /// `JournalUnavailable`. A waiter whose wait expired while uncertain
    /// leases held its group is answered with that hold (#2664), so it says
    /// why it waited instead of timing out bare.
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
        // A lost client's scope whose lease cleared holds nothing and no
        // connection: release it so its slot is not leaked (#2664).
        authority.release_idle_detached();
        let journal_healthy = authority.journal_healthy();
        let answers = waiting
            .iter()
            .filter_map(|&id| {
                let answer = match authority.observe(id, now) {
                    Ok(RequestState::Queued { .. }) => return None,
                    Ok(RequestState::Terminal(TerminalOutcome::Cancelled)) if !journal_healthy => {
                        Err(AuthorityError::JournalUnavailable)
                    }
                    Ok(timed_out @ RequestState::Terminal(TerminalOutcome::TimedOut)) => {
                        match authority.held(id, now) {
                            Ok(Some(hold)) => Err(AuthorityError::HeldByUncertainLease(hold)),
                            Ok(None) => Ok(timed_out),
                            // The wait was just observed, so it is known; it
                            // still answers its timeout if that ever breaks.
                            Err(error) => {
                                debug_assert!(false, "an observed wait is known: {error:?}");
                                Ok(timed_out)
                            }
                        }
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
        Settlement {
            dispatch_failure,
            answers,
            cancellation_due,
            next_wake: authority.next_wake(now),
            journal_healthy,
            holding: authority.holds_anything(now),
        }
    }
}

#[cfg(test)]
#[path = "settle_authority_tests.rs"]
mod tests;
