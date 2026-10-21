//! AdmissionJournal / AdmissionAuthority contract (#1679 P3 AC1/AC6, #1910):
//! the durability use cases persist the ledger before any grant is visible
//! and before a completion is acknowledged, and fail closed when durability
//! fails.
use quecto::application::admission::dto::broker::{AuthorityError, Credential};
use quecto::application::admission::dto::broker::{AuthorityStart, RestoreError, Settlement};
use quecto::application::admission::ports::inference_authority_ports::{
    AdmissionJournal, AdmissionSecretSource, JournalError, SharedJournal,
};
use quecto::application::admission::services::inference_authority::AdmissionAuthority;
use quecto::application::admission::use_cases::{
    AdvanceAuthorityEpoch, BindScope, CompleteAdmission, DisconnectScope, DispatchAdmissions,
    RestoreAuthority, SettleAuthority,
};
use quecto::domain::admission::value_objects::inference_admission::{
    AbandonReport, AdmissionConfig, AdmissionError, AdmissionLedger, Feedback, GroupId,
    GroupPolicy, RequestId, RequestState, ScopeId, TerminalOutcome, UNCERTAIN_LEASE_MARGIN_MS,
    UncertainHold, WorkloadClass,
};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct JournalLog {
    writes: Vec<AdmissionLedger>,
    fail: bool,
    unreadable: bool,
}

#[derive(Clone, Default)]
struct FakeJournal(Arc<Mutex<JournalLog>>);

impl FakeJournal {
    fn log(&self) -> MutexGuard<'_, JournalLog> {
        self.0.lock().unwrap()
    }
}

impl AdmissionJournal for FakeJournal {
    fn persist(&mut self, ledger: &AdmissionLedger) -> Result<(), JournalError> {
        let mut log = self.log();
        if log.fail {
            return Err(JournalError::Unavailable("disk full".into()));
        }
        log.writes.push(ledger.clone());
        Ok(())
    }

    fn load(&mut self) -> Result<Option<AdmissionLedger>, JournalError> {
        let log = self.log();
        if log.unreadable {
            return Err(JournalError::Unavailable("corrupt ledger".into()));
        }
        Ok(log.writes.last().cloned())
    }
}

struct Counter(u64);
impl AdmissionSecretSource for Counter {
    fn mint(&mut self) -> String {
        self.0 += 1;
        format!("secret-{}", self.0)
    }
}

fn config() -> AdmissionConfig {
    let g = GroupId::new("g").unwrap();
    AdmissionConfig {
        groups: BTreeMap::from([(
            g.clone(),
            GroupPolicy {
                capacity: 1,
                reserve: 0,
                min_interval_ms: 1,
                queue_capacity: 4,
                queue_timeout_ms: 1_000,
                attempt_timeout_ms: 5_000,
                fallback_base_ms: 100,
                max_cooldown_ms: 10_000,
            },
        )]),
        aliases: BTreeMap::from([("g".into(), g)]),
        max_scopes: 8,
        terminal_capacity: 16,
    }
}

fn group() -> GroupId {
    GroupId::new("g").unwrap()
}

/// The authority driven through its use cases, as the broker's actor drives
/// it; plain authority operations pass straight through.
struct Authority {
    authority: AdmissionAuthority<Counter>,
    dispatch: DispatchAdmissions,
    completion: CompleteAdmission,
    advance_epoch: AdvanceAuthorityEpoch,
    settlement: SettleAuthority,
    binding: BindScope,
    disconnection: DisconnectScope,
}

impl Authority {
    fn settle(&mut self, waiting: &[RequestId], now: u64) -> Settlement {
        self.settlement.execute(&mut self.authority, waiting, now)
    }

    /// The cancellations due, read the way the broker reads them: through a
    /// settling turn, which also dispatches whatever the policy allows.
    fn cancellation_due(&mut self, now: u64) -> Vec<RequestId> {
        self.settle(&[], now).cancellation_due
    }

    fn bind(
        &mut self,
        credential: &Credential,
        supersedes: bool,
        now: u64,
    ) -> Result<u64, AuthorityError> {
        self.binding
            .execute(&mut self.authority, credential, supersedes, now)
    }

    fn disconnect(&mut self, scope: ScopeId, now: u64) -> Result<AbandonReport, AuthorityError> {
        self.disconnection.execute(&mut self.authority, scope, now)
    }

    fn pump(&mut self, now: u64) -> Result<Vec<RequestId>, AuthorityError> {
        self.dispatch.execute(&mut self.authority, now)
    }

    fn complete(
        &mut self,
        credential: &Credential,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.completion
            .execute(&mut self.authority, credential, sequence, feedback, now)
    }

    fn reset(&mut self, now: u64) -> Result<u64, AuthorityError> {
        self.advance_epoch.execute(&mut self.authority, now)
    }
}

impl Deref for Authority {
    type Target = AdmissionAuthority<Counter>;
    fn deref(&self) -> &Self::Target {
        &self.authority
    }
}

impl DerefMut for Authority {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.authority
    }
}

fn authority() -> (Authority, FakeJournal) {
    let journal = FakeJournal::default();
    let shared: SharedJournal = Arc::new(Mutex::new(journal.clone()));
    let authority = Authority {
        authority: AdmissionAuthority::new(7, config(), Counter(0)).unwrap(),
        dispatch: DispatchAdmissions::new(shared.clone()),
        completion: CompleteAdmission::new(shared.clone()),
        advance_epoch: AdvanceAuthorityEpoch::new(shared.clone()),
        settlement: SettleAuthority::new(DispatchAdmissions::new(shared)),
        binding: BindScope::new(),
        disconnection: DisconnectScope::new(),
    };
    (authority, journal)
}

fn wrong(credential: &Credential) -> Credential {
    Credential {
        scope: credential.scope,
        token: "forged".into(),
    }
}

#[test]
fn roots_receive_distinct_secrets_and_forged_tokens_are_unauthorized() {
    let (mut a, _) = authority();
    let one = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let two = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    assert_ne!(one.token, two.token, "distinct capabilities");
    assert_ne!(one.scope, two.scope);
    assert_eq!(
        a.acquire(&wrong(&one), 1, "g", 0).err(),
        Some(AuthorityError::Unauthorized)
    );
    assert_eq!(
        a.acquire(&one, 1, "g", 0).unwrap(),
        RequestState::Queued { deadline: 1_000 }
    );
}

#[test]
fn child_registration_requires_the_parent_capability_and_inherits_its_root() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    assert_eq!(
        a.register_child(&wrong(&root), 0).err(),
        Some(AuthorityError::Unauthorized)
    );
    let child = a.register_child(&root, 0).unwrap();
    let grandchild = a.register_child(&child, 0).unwrap();
    assert_eq!(grandchild.scope.epoch, 7);
    assert!(grandchild.scope.serial > child.scope.serial);
    assert_eq!(
        a.lineage(grandchild.scope).unwrap(),
        root.scope,
        "descendants report their root"
    );
}

#[test]
fn a_grant_is_journaled_before_it_becomes_visible() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    let grants = a.pump(0).unwrap();
    assert_eq!(
        grants,
        vec![RequestId {
            scope: root.scope,
            sequence: 1
        }]
    );
    let log = journal.log();
    let last = log.writes.last().expect("ledger persisted");
    assert_eq!(
        last.outstanding.len(),
        1,
        "outstanding attempt durable before grant"
    );
    assert_eq!(last.outstanding[0].sequence, 1);
    assert_eq!(last.epoch, 7);
}

#[test]
fn journal_failure_withdraws_the_grant_and_stops_new_grants_until_recovery() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    journal.log().fail = true;
    assert_eq!(a.pump(0).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(
        a.status(&root, 1, 0).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled),
        "undelivered grant withdrawn"
    );
    assert!(!a.inspect(0).journal_healthy);
    a.acquire(&root, 2, "g", 1).unwrap();
    assert_eq!(a.pump(1).err(), Some(AuthorityError::JournalUnavailable));
    assert!(
        matches!(a.status(&root, 2, 1).unwrap(), RequestState::Queued { .. }),
        "while unhealthy the probe fails first and queued work is held"
    );
    journal.log().fail = false;
    a.acquire(&root, 3, "g", 2).unwrap();
    assert_eq!(
        a.pump(2).unwrap().len(),
        1,
        "grants resume after a durable write"
    );
    assert!(a.inspect(2).journal_healthy);
}

#[test]
fn completion_persists_the_release_before_acknowledging() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.complete(&root, 1, Feedback::Success, 1).unwrap();
    let log = journal.log();
    assert!(
        log.writes.last().unwrap().outstanding.is_empty(),
        "release durable at acknowledgement"
    );
    drop(log);
    journal.log().fail = true;
    a.acquire(&root, 2, "g", 2).unwrap();
    journal.log().fail = false;
    a.pump(2).unwrap();
    journal.log().fail = true;
    assert_eq!(
        a.complete(&root, 2, Feedback::Success, 3).err(),
        Some(AuthorityError::JournalUnavailable),
        "completion is not acknowledged without a durable release"
    );
}

#[test]
fn disconnect_cancels_queued_work_and_quarantines_active_work() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let other = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    a.acquire(&other, 1, "g", 0).unwrap();
    let report = a.disconnect(root.scope, 1).unwrap();
    assert_eq!((report.cancelled, report.uncertain), (1, 1));
    assert_eq!(
        a.pump(1).err(),
        Some(AuthorityError::Admission(AdmissionError::Quarantined))
    );
    let status = a.inspect(1);
    assert_eq!(status.groups[&group()].uncertain, 1);
    // The same capability reconnecting and completing clears the quarantine.
    a.complete(&root, 1, Feedback::Success, 2).unwrap();
    assert_eq!(a.pump(2).unwrap().len(), 1);
}

#[test]
fn repeated_acquire_reconciles_the_existing_request() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    assert!(matches!(
        a.acquire(&root, 1, "g", 1).unwrap(),
        RequestState::Active { .. }
    ));
    assert_eq!(a.pump(1).unwrap(), vec![], "no second grant");
}

#[test]
fn reset_revokes_every_capability_and_starts_a_new_epoch() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(root.scope, 1).unwrap();
    assert_eq!(a.reset(2).unwrap(), 8);
    assert_eq!(
        a.status(&root, 1, 2).err(),
        Some(AuthorityError::Unauthorized),
        "old capability revoked"
    );
    assert_eq!(
        journal.log().writes.last().unwrap().epoch,
        8,
        "reset is durable"
    );
    let fresh = a.register_root(WorkloadClass::Interactive, 3).unwrap();
    assert_eq!(fresh.scope.epoch, 8);
    a.acquire(&fresh, 1, "g", 3).unwrap();
    assert_eq!(a.pump(3).unwrap().len(), 1);
}

#[test]
fn reset_is_refused_when_the_journal_cannot_record_it() {
    let (mut a, journal) = authority();
    journal.log().fail = true;
    assert_eq!(a.reset(1).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(a.inspect(1).epoch, 7, "epoch unchanged");
    assert!(
        !a.inspect(1).journal_healthy,
        "the failed write is recorded"
    );
    journal.log().fail = false;
    assert_eq!(a.reset(2).unwrap(), 8, "a durable reset advances the epoch");
    assert!(a.inspect(2).journal_healthy, "and restores health");
}

#[test]
fn cancellation_due_lists_only_active_attempts_that_must_stop() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    assert_eq!(a.cancellation_due(1), vec![]);
    a.cancel(&root, 1, 2).unwrap();
    assert_eq!(
        a.cancellation_due(2),
        vec![RequestId {
            scope: root.scope,
            sequence: 1
        }]
    );
    a.complete(&root, 1, Feedback::Failure, 3).unwrap();
    assert_eq!(a.cancellation_due(3), vec![]);
}

#[test]
fn retire_releases_a_scope_and_its_capability() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.retire(&root, 0).unwrap();
    assert_eq!(
        a.acquire(&root, 1, "g", 1).err(),
        Some(AuthorityError::Unauthorized)
    );
}

/// Review MEDIUM-5: a parent can retire a descendant it registered (for
/// example when the launch failed), but not an unrelated scope.
#[test]
fn a_parent_can_retire_only_its_own_descendants() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let other = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let child = a.register_child(&root, 0).unwrap();
    assert_eq!(
        a.retire_child(&other, child.scope, 1).err(),
        Some(AuthorityError::Unauthorized)
    );
    a.retire_child(&root, child.scope, 1).unwrap();
    assert_eq!(
        a.acquire(&child, 1, "g", 2).err(),
        Some(AuthorityError::Unauthorized),
        "retired child capability is revoked"
    );
    assert_eq!(a.inspect(2).live_scopes, 2);
}

/// Review MEDIUM-7: an unhealthy journal is probed before any candidate is
/// selected, so queued work is neither withdrawn nor charged for pacing.
#[test]
fn unhealthy_journal_is_probed_before_dispatch_so_queued_work_is_held() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    journal.log().fail = true;
    a.acquire(&root, 1, "g", 0).unwrap();
    assert_eq!(a.pump(0).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(
        a.status(&root, 1, 1).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled),
        "the first selected grant is withdrawn"
    );
    a.acquire(&root, 2, "g", 1).unwrap();
    assert_eq!(a.pump(1).err(), Some(AuthorityError::JournalUnavailable));
    assert!(
        matches!(a.status(&root, 2, 1).unwrap(), RequestState::Queued { .. }),
        "while unhealthy, the probe fails before any further selection"
    );
    journal.log().fail = false;
    assert_eq!(
        a.pump(2).unwrap().len(),
        1,
        "held work dispatches once durable"
    );
}

fn start(previously_operated: bool, accept_missing_ledger: bool) -> AuthorityStart {
    AuthorityStart {
        policy: config(),
        previously_operated,
        accept_missing_ledger,
    }
}

fn restore_over(
    journal: &FakeJournal,
    start: AuthorityStart,
) -> Result<AdmissionAuthority<Counter>, RestoreError> {
    let shared: SharedJournal = Arc::new(Mutex::new(journal.clone()));
    RestoreAuthority::new(shared).execute(start, Counter(0), 0)
}

#[test]
fn a_fresh_journal_starts_a_durable_first_epoch() {
    let journal = FakeJournal::default();
    let mut authority = restore_over(&journal, start(false, false)).unwrap();
    assert_eq!(authority.inspect(0).epoch, 1);
    assert!(authority.inspect(0).journal_healthy);
    assert_eq!(
        journal.log().writes.len(),
        1,
        "the initial ledger is durable"
    );
    assert_eq!(journal.log().writes[0].epoch, 1);
}

#[test]
fn a_journaled_ledger_is_restored_and_checkpointed() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    let written = journal.log().writes.len();
    let mut restored = restore_over(&journal, start(true, false)).unwrap();
    assert_eq!(restored.inspect(0).epoch, 7, "the journaled epoch");
    assert_eq!(
        restored.inspect(0).groups[&group()].uncertain,
        1,
        "the outstanding attempt is unverified after a restart"
    );
    assert_eq!(journal.log().writes.len(), written + 1, "checkpointed");
}

#[test]
fn a_ledger_missing_after_prior_operation_is_refused_unless_accepted() {
    let journal = FakeJournal::default();
    assert_eq!(
        restore_over(&journal, start(true, false)).err(),
        Some(RestoreError::MissingLedger)
    );
    assert!(journal.log().writes.is_empty(), "nothing is written");
    let mut accepted = restore_over(&journal, start(true, true)).unwrap();
    assert_eq!(
        accepted.inspect(0).epoch,
        1,
        "the operator accepted an empty ledger"
    );
    assert_eq!(journal.log().writes.len(), 1);
}

#[test]
fn an_unreadable_incompatible_or_undurable_journal_refuses_to_start() {
    let journal = FakeJournal::default();
    journal.log().unreadable = true;
    assert!(matches!(
        restore_over(&journal, start(false, false)),
        Err(RestoreError::Unreadable(reason)) if reason.contains("corrupt")
    ));
    journal.log().unreadable = false;
    journal.log().fail = true;
    assert_eq!(
        restore_over(&journal, start(false, false)).err(),
        Some(RestoreError::NotDurable(AuthorityError::JournalUnavailable))
    );
    journal.log().fail = false;
    let (mut a, held) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    let mut renamed = start(true, false);
    let policy = renamed.policy.groups.remove(&group()).unwrap();
    let other = GroupId::new("other").unwrap();
    renamed.policy.groups.insert(other.clone(), policy);
    renamed.policy.aliases = BTreeMap::from([("g".into(), other)]);
    assert!(
        matches!(
            restore_over(&held, renamed),
            Err(RestoreError::Incompatible(_))
        ),
        "a ledger naming a group the policy no longer has"
    );
}

/// A panic while the journal's lock was held poisons it; the durability use
/// cases recover the journal rather than turning every later write into a
/// panic.
#[test]
fn a_poisoned_journal_lock_still_takes_durable_writes() {
    let journal = FakeJournal::default();
    let shared: SharedJournal = Arc::new(Mutex::new(journal.clone()));
    let poisoner = shared.clone();
    let poisoned = std::thread::spawn(move || {
        let _held = poisoner.lock().unwrap();
        panic!("poison the journal lock");
    })
    .join();
    assert!(poisoned.is_err(), "the holder panicked");
    assert!(shared.is_poisoned());
    let mut authority = RestoreAuthority::new(shared)
        .execute(start(false, false), Counter(0), 0)
        .unwrap();
    assert_eq!(journal.log().writes.len(), 1, "the write went through");
    assert!(authority.inspect(0).journal_healthy);
}

#[test]
fn journal_errors_convert_to_the_authority_error() {
    let converted: AuthorityError = JournalError::Unavailable("disk".into()).into();
    assert_eq!(converted, AuthorityError::JournalUnavailable);
}

/// The running authority's session policy, over this file's harness.
#[path = "admission_session.rs"]
mod session;

/// #2664: a wait that expires behind an uncertain lease is not answered as a
/// bare queue timeout: the answer names the lease that held it.
#[test]
fn a_wait_that_expires_behind_an_uncertain_lease_says_so() {
    let (mut a, _) = authority();
    let lost = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let waiting = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&lost, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(lost.scope, 1).unwrap();
    a.acquire(&waiting, 1, "g", 10).unwrap();
    let id = RequestId {
        scope: waiting.scope,
        sequence: 1,
    };
    assert_eq!(
        a.settle(&[id], 1_010).answers,
        vec![(
            id,
            Err(AuthorityError::HeldByUncertainLease(UncertainHold {
                group: group(),
                leases: 1,
                clears_in_ms: 5_000 + UNCERTAIN_LEASE_MARGIN_MS - 1_010,
            }))
        )],
        "not a bare timeout: the answer names the lease and when it clears"
    );
}

/// #2664: a plain queue timeout, with no uncertain lease, is unchanged.
#[test]
fn a_wait_that_expires_without_an_uncertain_lease_is_a_plain_timeout() {
    let (mut a, _) = authority();
    let busy = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let waiting = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&busy, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&waiting, 1, "g", 10).unwrap();
    let id = RequestId {
        scope: waiting.scope,
        sequence: 1,
    };
    assert_eq!(
        a.settle(&[id], 1_010).answers,
        vec![(id, Ok(RequestState::Terminal(TerminalOutcome::TimedOut)))]
    );
}

/// #2664: once a lost client's lease clears, its scope holds nothing and no
/// connection, so it is released like a scope that disconnected idle: the
/// group dispatches again and the scope slot is not leaked.
#[test]
fn a_lost_scope_is_released_once_its_lease_clears() {
    let (mut a, _) = authority();
    let lost = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let other = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&lost, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(lost.scope, 1).unwrap();
    assert_eq!(
        a.inspect(1).live_scopes,
        2,
        "the lost scope is kept for now"
    );
    let release = 5_000 + UNCERTAIN_LEASE_MARGIN_MS;
    a.settle(&[], release - 1);
    assert_eq!(
        a.inspect(release - 1).live_scopes,
        2,
        "a lost scope still holding its lease stays tracked"
    );
    a.acquire(&other, 1, "g", release).unwrap();
    let settled = a.settle(&[], release);
    assert_eq!(settled.dispatch_failure, None);
    assert_eq!(a.inspect(release).groups[&group()].uncertain, 0);
    assert_eq!(a.inspect(release).groups[&group()].active, 1, "granted");
    assert_eq!(
        a.inspect(release).live_scopes,
        1,
        "the lost scope is released"
    );
    assert_eq!(
        a.acquire(&lost, 2, "g", release).err(),
        Some(AuthorityError::Unauthorized),
        "its capability went with it"
    );
}

/// #2664: a scope whose owner rebound its capability is live, so its lease
/// clearing does not release it.
#[test]
fn a_rebound_scope_is_kept_when_its_lease_clears() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(root.scope, 1).unwrap();
    assert_eq!(a.bind(&root, false, 2).unwrap(), 2, "rebound");
    let release = 5_000 + UNCERTAIN_LEASE_MARGIN_MS;
    a.settle(&[], release);
    assert_eq!(a.inspect(release).live_scopes, 1, "the rebound scope stays");
    a.acquire(&root, 2, "g", release).unwrap();
    assert_eq!(a.pump(release).unwrap().len(), 1, "and keeps working");
}

/// #2664 review: a lease that clears on time leaves no write of its own, so
/// the turn that clears it makes the ledger durable; a restart cannot bring
/// it back. A later quiet turn writes nothing more.
#[test]
fn a_lease_that_clears_on_time_is_checkpointed() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(root.scope, 1).unwrap();
    let release = 5_000 + UNCERTAIN_LEASE_MARGIN_MS;
    let written = journal.log().writes.len();
    a.settle(&[], release - 1);
    assert_eq!(journal.log().writes.len(), written, "nothing cleared yet");
    a.settle(&[], release);
    assert_eq!(
        journal.log().writes.len(),
        written + 1,
        "the clearing is durable"
    );
    assert!(journal.log().writes.last().unwrap().outstanding.is_empty());
    a.settle(&[], release + 1);
    assert_eq!(journal.log().writes.len(), written + 1, "and written once");
}

/// #2664 review: a lease lost, rebound and lost again keeps the instant it
/// was first lost: the margin runs from there, not from the second loss.
#[test]
fn a_lease_lost_twice_keeps_its_first_loss() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let other = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    let first = 5_000 + UNCERTAIN_LEASE_MARGIN_MS;
    a.disconnect(root.scope, first).unwrap();
    a.bind(&root, false, first + 1).unwrap();
    a.disconnect(root.scope, first + 30_000).unwrap();
    let release = first + UNCERTAIN_LEASE_MARGIN_MS;
    a.acquire(&other, 1, "g", release - 1).unwrap();
    a.settle(&[], release - 1);
    assert_eq!(a.inspect(release - 1).groups[&group()].uncertain, 1);
    a.settle(&[], release);
    assert_eq!(
        a.inspect(release).groups[&group()].uncertain,
        0,
        "cleared a margin after the first loss"
    );
    assert_eq!(a.inspect(release).groups[&group()].active, 1, "and granted");
}
