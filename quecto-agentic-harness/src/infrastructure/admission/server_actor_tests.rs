//! The actor's own timing (#1910 review): the next turn is armed after the
//! turn that decided it, so a slow journal write cannot leave it already due.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::Actor;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionJournal, JournalError, SharedJournal,
};
use crate::application::admission::services::inference_authority::AdmissionAuthority;
use crate::application::admission::use_cases::{
    AdvanceAuthorityEpoch, BindScope, CompleteAdmission, DisconnectScope, DispatchAdmissions,
    RestoreAuthority, SettleAuthority,
};
use crate::domain::admission::value_objects::authority_idle::AuthorityStop;
use crate::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, AdmissionLedger, GroupId, GroupPolicy, WorkloadClass,
};
use crate::infrastructure::admission::protocol::ProposalWire;
use crate::infrastructure::admission::secret::RandomSecretSource;
use crate::infrastructure::admission::server::{AuthorityUseCases, Clock};
use crate::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;

/// How long each write of the hung journal takes before it fails: longer
/// than the journal probe interval.
const SLOW_WRITE: Duration = Duration::from_millis(300);

/// A journal on a hung disk: every write takes `SLOW_WRITE`, then fails.
struct HungJournal;

impl AdmissionJournal for HungJournal {
    fn persist(&mut self, _ledger: &AdmissionLedger) -> Result<(), JournalError> {
        std::thread::sleep(SLOW_WRITE);
        Err(JournalError::Unavailable("disk hung".into()))
    }

    fn load(&mut self) -> Result<Option<AdmissionLedger>, JournalError> {
        Ok(None)
    }
}

fn proposal() -> AdmissionRuntimeProposal {
    let group = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 8,
                    queue_timeout_ms: 60_000,
                    attempt_timeout_ms: 60_000,
                    fallback_base_ms: 50,
                    max_cooldown_ms: 500,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), group)]),
            max_scopes: 8,
            terminal_capacity: 16,
        },
        bindings: BTreeMap::from([("fake".into(), "acct".into())]),
    }
}

/// Review MED (#2625 round 2): a turn whose dispatch write hangs past the
/// probe interval still arms the next probe in the future, a probe interval
/// after the turn ended, rather than already due.
#[test]
fn a_slow_failing_write_arms_the_next_probe_after_the_turn() {
    let journal: SharedJournal = Arc::new(Mutex::new(HungJournal));
    let use_cases = AuthorityUseCases {
        restore: RestoreAuthority::new(journal.clone()),
        settle: SettleAuthority::new(DispatchAdmissions::new(journal.clone())),
        complete: CompleteAdmission::new(journal.clone()),
        advance_epoch: AdvanceAuthorityEpoch::new(journal),
        bind: BindScope::new(),
        disconnect: DisconnectScope::new(),
    };
    let proposal = proposal();
    let clock = Clock::starting_now();
    let mut authority =
        AdmissionAuthority::new(1, proposal.policy.clone(), RandomSecretSource).unwrap();
    let root = authority
        .register_root(WorkloadClass::Interactive, clock.now_ms())
        .unwrap();
    authority.acquire(&root, 1, "acct", clock.now_ms()).unwrap();
    let mut actor = Actor::new(
        authority,
        use_cases,
        clock,
        ProposalWire::from(&proposal),
        "owner".into(),
    );
    let started = Instant::now();
    let wake = actor.settle();
    let ended = Instant::now();
    assert!(
        ended.duration_since(started) >= SLOW_WRITE,
        "the turn waited out the hung write"
    );
    assert!(
        wake > ended + Duration::from_millis(150),
        "the next probe is armed after the turn, not already due: {:?} after it ended",
        wake.saturating_duration_since(ended)
    );
    assert!(
        wake <= ended + Duration::from_millis(250),
        "and no later than one probe interval after it"
    );
}

fn memory_use_cases() -> AuthorityUseCases {
    struct Memory;
    impl AdmissionJournal for Memory {
        fn persist(&mut self, _ledger: &AdmissionLedger) -> Result<(), JournalError> {
            Ok(())
        }
        fn load(&mut self) -> Result<Option<AdmissionLedger>, JournalError> {
            Ok(None)
        }
    }
    let journal: SharedJournal = Arc::new(Mutex::new(Memory));
    AuthorityUseCases {
        restore: RestoreAuthority::new(journal.clone()),
        settle: SettleAuthority::new(DispatchAdmissions::new(journal.clone())),
        complete: CompleteAdmission::new(journal.clone()),
        advance_epoch: AdvanceAuthorityEpoch::new(journal),
        bind: BindScope::new(),
        disconnect: DisconnectScope::new(),
    }
}

/// #2615: an owner's Stop is answered first, and the authority stops once
/// that asker has left — not in the turn that answered it (its reply would
/// be lost with its session), and not before the asker leaves unless the
/// grace runs out.
#[test]
fn a_requested_stop_waits_for_its_asker_to_leave() {
    use crate::infrastructure::admission::protocol::{
        Body, CAPABILITY_DIRECT, Op, PROTOCOL_VERSION, Request, Role,
    };
    let proposal = proposal();
    let clock = Clock::starting_now();
    let authority =
        AdmissionAuthority::new(1, proposal.policy.clone(), RandomSecretSource).unwrap();
    let mut actor = Actor::new(
        authority,
        memory_use_cases(),
        clock,
        ProposalWire::from(&proposal),
        "owner".into(),
    );
    let (notices, _notices_rx) = tokio::sync::mpsc::unbounded_channel();
    let (close, _closed) = tokio::sync::oneshot::channel();
    actor.handle(super::Command::Open {
        session: 7,
        role: Role::Admin,
        notices,
        close,
    });
    let hello = Request {
        id: 1,
        op: Op::Hello {
            version: PROTOCOL_VERSION,
            capability: CAPABILITY_DIRECT.into(),
        },
    };
    assert!(matches!(actor.request(7, &hello), Some(Body::Hello { .. })));
    let stop = Request {
        id: 2,
        op: Op::Stop,
    };
    assert_eq!(actor.request(7, &stop), Some(Body::Ok), "answered first");
    actor.settle();
    assert_eq!(
        actor.stopping, None,
        "still serving while the asker is here"
    );
    actor.handle(super::Command::Closed { session: 7 });
    assert_eq!(actor.stopping, Some(AuthorityStop::Requested));
}

/// #2615 review: a client that connects as the idle stop is decided is
/// served first, and its open connection holds the broker.
#[test]
fn a_connection_pending_at_the_idle_stop_keeps_the_broker_serving() {
    use crate::domain::admission::value_objects::authority_idle::{AuthorityLifetime, IdleWatch};
    use crate::infrastructure::admission::protocol::Role;
    let proposal = proposal();
    let (stopped, _watch) = tokio::sync::watch::channel(None);
    let mut actor = Actor::new(
        AdmissionAuthority::new(1, proposal.policy.clone(), RandomSecretSource).unwrap(),
        memory_use_cases(),
        Clock::starting_now(),
        ProposalWire::from(&proposal),
        "owner".into(),
    )
    .with_lifetime(
        AuthorityLifetime::StopWhenIdle(IdleWatch::new(1).unwrap()),
        stopped,
    );
    actor.settle();
    std::thread::sleep(Duration::from_millis(5));
    actor.settle();
    assert_eq!(
        actor.stopping,
        Some(AuthorityStop::Idle),
        "idle past its period: the stop is decided"
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (notices, _notices) = tokio::sync::mpsc::unbounded_channel();
    let (close, _closed) = tokio::sync::oneshot::channel();
    tx.send(super::Command::Open {
        session: 1,
        role: Role::Client,
        notices,
        close,
    })
    .unwrap();
    assert!(actor.drain_pending(&mut rx));
    actor.stopping = None;
    actor.settle();
    assert_eq!(actor.stopping, None, "the open connection holds it");
    assert!(!actor.drain_pending(&mut rx), "nothing more waiting");
}

/// #2615 review: only the owner's admin socket may stop the authority; a
/// client's Stop is refused and the authority keeps serving.
#[test]
fn a_client_cannot_stop_the_authority() {
    use crate::infrastructure::admission::protocol::{
        Body, CAPABILITY_DIRECT, ErrorCode, Op, PROTOCOL_VERSION, Request, Role,
    };
    let proposal = proposal();
    let mut actor = Actor::new(
        AdmissionAuthority::new(1, proposal.policy.clone(), RandomSecretSource).unwrap(),
        memory_use_cases(),
        Clock::starting_now(),
        ProposalWire::from(&proposal),
        "owner".into(),
    );
    let (notices, _notices_rx) = tokio::sync::mpsc::unbounded_channel();
    let (close, _closed) = tokio::sync::oneshot::channel();
    actor.handle(super::Command::Open {
        session: 3,
        role: Role::Client,
        notices,
        close,
    });
    let hello = Request {
        id: 1,
        op: Op::Hello {
            version: PROTOCOL_VERSION,
            capability: CAPABILITY_DIRECT.into(),
        },
    };
    assert!(matches!(actor.request(3, &hello), Some(Body::Hello { .. })));
    let stop = Request {
        id: 2,
        op: Op::Stop,
    };
    assert!(
        matches!(
            actor.request(3, &stop),
            Some(Body::Error {
                code: ErrorCode::Unauthorized,
                ..
            })
        ),
        "a client's Stop is refused"
    );
    actor.handle(super::Command::Closed { session: 3 });
    actor.settle();
    assert_eq!(actor.stopping, None, "still serving");
}
