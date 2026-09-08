//! P3 authority process adapters over real UDS/files: singleton lock, durable
//! journal, framed protocol, client capability lifetime and recovery.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use quecto::application::ports::AdmissionJournal;
use quecto::domain::inference_admission::*;
use quecto::infrastructure::admission::{
    AdminConnection, AuthorityConnection, AuthorityDirectory, AuthorityServer, ClientError,
    FileJournal, ServerError, SingletonLock,
};
use quecto::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use tokio::time::timeout;

const LIMIT: Duration = Duration::from_secs(5);

fn proposal(capacity: usize, queue_timeout_ms: u64) -> AdmissionRuntimeProposal {
    let g = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                g.clone(),
                GroupPolicy {
                    capacity,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 4,
                    queue_timeout_ms,
                    attempt_timeout_ms: 60_000,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 10_000,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), g)]),
            max_scopes: 16,
            terminal_capacity: 32,
        },
        bindings: BTreeMap::from([("openai".into(), "acct".into())]),
    }
}

fn group() -> GroupId {
    GroupId::new("g").unwrap()
}

/// Completion acknowledgements are asynchronous; wait (bounded) for the
/// authority to observe the expected occupancy.
async fn wait_for(admin: &AdminConnection, expected: (usize, usize)) {
    timeout(LIMIT, async {
        loop {
            let status = admin.inspect().await.unwrap();
            let snapshot = status.groups[&group()];
            if (snapshot.active, snapshot.queued) == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("authority never observed occupancy {expected:?}"));
}

async fn root(server: &AuthorityServer) -> AuthorityConnection {
    let connection = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    let credential = connection
        .register_root(WorkloadClass::Interactive)
        .await
        .unwrap();
    connection.bind(credential).await.unwrap();
    connection
}

#[test]
fn directory_is_private_and_the_lock_is_a_singleton() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("authority");
    let dir = AuthorityDirectory::open(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "owner-only directory");
    assert_eq!(
        std::fs::metadata(dir.client_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let first = SingletonLock::acquire(&dir).unwrap();
    assert!(
        SingletonLock::acquire(&dir).is_err(),
        "second authority refused while the first lives"
    );
    drop(first);
    assert!(
        SingletonLock::acquire(&dir).is_ok(),
        "lock released with owner"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(
        AuthorityDirectory::open(&path).is_err(),
        "group/other-accessible directory is refused"
    );
}

#[test]
fn file_journal_is_durable_round_trip_and_fails_closed_on_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    assert_eq!(FileJournal::load(&dir).unwrap(), None, "fresh authority");
    let mut journal = FileJournal::new(&dir);
    let ledger = AdmissionLedger {
        epoch: 9,
        outstanding: vec![OutstandingAttempt {
            group: group(),
            scope: 3,
            sequence: 11,
        }],
        groups: BTreeMap::from([(
            group(),
            LedgerGroup {
                cooldown_remaining_ms: 400,
                pacing_remaining_ms: 2,
                unavailable: false,
            },
        )]),
    };
    journal.persist(&ledger).unwrap();
    assert_eq!(FileJournal::load(&dir).unwrap(), Some(ledger));
    let mode = std::fs::metadata(dir.journal_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    std::fs::write(dir.journal_path(), b"{not json").unwrap();
    assert!(
        FileJournal::load(&dir).is_err(),
        "corrupt ledger never restarts empty"
    );
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let denied = journal.persist(&AdmissionLedger {
        epoch: 10,
        outstanding: vec![],
        groups: BTreeMap::new(),
    });
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(denied.is_err(), "write failure is reported, not warned");
}

#[tokio::test]
async fn hello_publishes_the_effective_policy_and_grants_release_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 5_000),
    )
    .await
    .unwrap();
    let a = root(&server).await;
    let hello = a.hello();
    assert_eq!(
        hello.proposal.policy.aliases,
        proposal(1, 5_000).policy.aliases
    );
    assert_eq!(hello.proposal.bindings, proposal(1, 5_000).bindings);
    let b = root(&server).await;
    let gate_a = a.gate("acct").unwrap();
    let gate_b = b.gate("acct").unwrap();
    let first = timeout(LIMIT, gate_a.acquire()).await.unwrap().unwrap();
    let mut second = Box::pin(gate_b.acquire());
    assert!(
        timeout(Duration::from_millis(100), &mut second)
            .await
            .is_err(),
        "capacity one keeps the second request queued"
    );
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    let status = admin.inspect().await.unwrap();
    assert_eq!(
        (
            status.groups[&group()].active,
            status.groups[&group()].queued
        ),
        (1, 1)
    );
    first.finish(Feedback::Success);
    let second = timeout(LIMIT, second).await.unwrap().unwrap();
    second.finish(Feedback::Success);
    wait_for(&admin, (0, 0)).await;
    server.shutdown().await;
}

#[tokio::test]
async fn dropping_a_queued_acquire_cancels_it_before_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 60_000),
    )
    .await
    .unwrap();
    let a = root(&server).await;
    let b = root(&server).await;
    let gate_a = a.gate("acct").unwrap();
    let gate_b = b.gate("acct").unwrap();
    let first = timeout(LIMIT, gate_a.acquire()).await.unwrap().unwrap();
    let second = Box::pin(gate_b.acquire());
    assert!(timeout(Duration::from_millis(50), second).await.is_err());
    // The dropped future has cancelled its request at the authority.
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    // The queue deadline is a minute away: only an explicit cancel can empty
    // the queue this quickly.
    timeout(Duration::from_secs(1), async {
        loop {
            if admin.inspect().await.unwrap().groups[&group()].queued == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancellation reaches the authority");
    first.finish(Feedback::Success);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let status = admin.inspect().await.unwrap();
    assert_eq!(
        status.groups[&group()].active,
        0,
        "cancelled work never dispatches"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn connection_loss_quarantines_until_the_same_capability_reconciles() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(2, 300),
    )
    .await
    .unwrap();
    let a = root(&server).await;
    let credential = a.credential().unwrap();
    let permit = timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .unwrap();
    // A freshly bound scope starts its acquire sequence at one.
    let sequence = 1;
    drop(permit);
    drop(a);
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    timeout(LIMIT, async {
        loop {
            let status = admin.inspect().await.unwrap();
            if status.groups[&group()].uncertain == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("abandoned attempt becomes uncertain");
    let b = root(&server).await;
    let outcome = timeout(LIMIT, b.gate("acct").unwrap().acquire())
        .await
        .unwrap();
    assert!(
        outcome.is_err(),
        "quarantined group yields a bounded explicit failure, not a grant"
    );
    let again = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    again.bind(credential).await.unwrap();
    again.complete(sequence, Feedback::Success).await.unwrap();
    let status = admin.inspect().await.unwrap();
    assert_eq!(status.groups[&group()].uncertain, 0);
    timeout(LIMIT, b.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .expect("grants resume after verified termination");
    server.shutdown().await;
}

#[tokio::test]
async fn reset_revokes_capabilities_and_restart_orphans_outstanding_work() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("authority");
    let server = AuthorityServer::start(AuthorityDirectory::open(&path).unwrap(), proposal(2, 300))
        .await
        .unwrap();
    let a = root(&server).await;
    let permit = timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .unwrap();
    let epoch = a.hello().epoch;
    server.shutdown().await;
    std::mem::forget(permit);
    let restarted =
        AuthorityServer::start(AuthorityDirectory::open(&path).unwrap(), proposal(2, 300))
            .await
            .unwrap();
    let admin = AdminConnection::connect(&restarted.directory().admin_socket())
        .await
        .unwrap();
    let status = admin.inspect().await.unwrap();
    assert_eq!(status.epoch, epoch, "restart keeps the accounting epoch");
    assert_eq!(
        status.groups[&group()].uncertain,
        1,
        "outstanding work is orphaned"
    );
    let b = root(&restarted).await;
    assert!(
        timeout(LIMIT, b.gate("acct").unwrap().acquire())
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(admin.reset().await.unwrap(), epoch + 1);
    assert!(
        matches!(b.status(1).await, Err(ClientError::Unauthorized)),
        "old capability revoked by reset"
    );
    let c = root(&restarted).await;
    assert_eq!(c.credential().unwrap().scope.epoch, epoch + 1);
    timeout(LIMIT, c.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .expect("new epoch grants");
    restarted.shutdown().await;
}

#[tokio::test]
async fn journal_failure_denies_grants_instead_of_bypassing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("authority");
    let server = AuthorityServer::start(AuthorityDirectory::open(&path).unwrap(), proposal(2, 200))
        .await
        .unwrap();
    let a = root(&server).await;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let denied = timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap();
    assert!(denied.is_err(), "no grant without a durable ledger");
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    assert!(!admin.inspect().await.unwrap().journal_healthy);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .expect("grants resume once the journal is writable");
    assert!(admin.inspect().await.unwrap().journal_healthy);
    server.shutdown().await;
}

#[tokio::test]
async fn protocol_rejects_unsupported_hello_unbound_operations_and_child_admin() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 300),
    )
    .await
    .unwrap();
    assert!(matches!(
        AuthorityConnection::connect_with_version(&server.directory().client_socket(), 99).await,
        Err(ClientError::UnsupportedProtocol(_))
    ));
    let unbound = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    assert!(matches!(
        unbound.status(1).await,
        Err(ClientError::Unauthorized)
    ));
    assert!(
        AdminConnection::connect(&server.directory().client_socket())
            .await
            .is_err(),
        "administration is not offered on the child-visible socket"
    );
    let parent = root(&server).await;
    let child = parent.register_child().await.unwrap();
    assert!(child.scope.serial > parent.credential().unwrap().scope.serial);
    server.shutdown().await;
}

// ---- Review fixes (adversarial review of P3) --------------------------------

fn write_probe_config(
    policy: &AdmissionRuntimeProposal,
    capacity: usize,
) -> AdmissionRuntimeProposal {
    let mut p = policy.clone();
    for g in p.policy.groups.values_mut() {
        g.capacity = capacity;
        g.queue_capacity = 512;
    }
    p
}

/// HIGH-1: a full-duplex client must never lose a partially read frame while
/// it is writing. Many concurrent acquire/finish cycles over one connection
/// keep grants and completions interleaved on the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_grants_and_completions_never_desynchronize_the_connection() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        write_probe_config(&proposal(4, 20_000), 4),
    )
    .await
    .unwrap();
    let a = Arc::new(root(&server).await);
    let mut tasks = Vec::new();
    for _ in 0..64 {
        let a = a.clone();
        tasks.push(tokio::spawn(async move {
            for _ in 0..4 {
                let permit = timeout(LIMIT, a.gate("acct").unwrap().acquire())
                    .await
                    .expect("bounded")
                    .expect("grant on a healthy connection");
                permit.finish(Feedback::Success);
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    wait_for(&admin, (0, 0)).await;
    timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .expect("connection is still open and bound after 256 cycles")
        .finish(Feedback::Success);
    wait_for(&admin, (0, 0)).await;
    server.shutdown().await;
}

/// HIGH-2: cancelling an attempt that was already granted (grant/cancel race)
/// must complete it as a failure — no transport ever existed — so the slot is
/// released instead of leaking until process exit.
#[tokio::test]
async fn cancel_after_grant_releases_the_slot_as_a_failed_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 20_000),
    )
    .await
    .unwrap();
    let a = root(&server).await;
    let permit = timeout(LIMIT, a.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .unwrap();
    std::mem::forget(permit);
    a.cancel_detached(1);
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    wait_for(&admin, (0, 0)).await;
    let status = admin.inspect().await.unwrap();
    assert_eq!(
        status.groups[&group()].uncertain,
        0,
        "verified release, not uncertainty"
    );
    assert_eq!(
        a.status(1).await.unwrap(),
        RequestState::Terminal(TerminalOutcome::Finished)
    );
    server.shutdown().await;
}

/// MEDIUM-1: concurrent acquires on one scope are serialized so the
/// authority's replay fence never rejects a legitimate attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_acquires_on_one_scope_are_never_rejected_as_replay() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        write_probe_config(&proposal(32, 20_000), 32),
    )
    .await
    .unwrap();
    let a = Arc::new(root(&server).await);
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let a = a.clone();
        tasks.push(tokio::spawn(async move {
            timeout(LIMIT, a.gate("acct").unwrap().acquire())
                .await
                .unwrap()
                .map(|permit| permit.finish(Feedback::Success))
        }));
    }
    for task in tasks {
        task.await
            .unwrap()
            .expect("no Replay for concurrent attempts");
    }
    server.shutdown().await;
}

/// HIGH-4b: minting a root requires the owner token kept outside `client/`, so
/// a process that only sees the mounted client directory cannot self-promote.
#[tokio::test]
async fn root_registration_requires_the_owner_token_outside_the_client_directory() {
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let server = AuthorityServer::start(dir.clone(), proposal(1, 300))
        .await
        .unwrap();
    let token_path = dir.root_token_path();
    assert!(token_path.starts_with(dir.path()) && !token_path.starts_with(dir.client_dir()));
    let mode = std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let connection = AuthorityConnection::connect(&dir.client_socket())
        .await
        .unwrap();
    assert!(matches!(
        connection
            .register_root_with_token(WorkloadClass::Interactive, "forged")
            .await,
        Err(ClientError::Unauthorized)
    ));
    let token = std::fs::read_to_string(&token_path).unwrap();
    connection
        .register_root_with_token(WorkloadClass::Interactive, token.trim())
        .await
        .expect("owner token mints a root");
    server.shutdown().await;
}

/// MEDIUM-2: a ledger that vanished after prior operation never restarts as an
/// empty epoch-one budget; a fresh directory persists its initial ledger.
#[tokio::test]
async fn missing_ledger_after_prior_operation_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let server = AuthorityServer::start(dir.clone(), proposal(1, 300))
        .await
        .unwrap();
    assert!(
        dir.journal_path().exists(),
        "fresh start persists an initial ledger"
    );
    server.shutdown().await;
    std::fs::remove_file(dir.journal_path()).unwrap();
    let refused = match AuthorityServer::start(dir.clone(), proposal(1, 300)).await {
        Err(ServerError::Journal(message)) => message,
        Err(other) => panic!("unexpected refusal {other}"),
        Ok(_) => panic!("a vanished ledger must not start an empty budget"),
    };
    assert!(refused.contains("missing"), "{refused}");
    let accepted = AuthorityServer::start_accepting_missing_ledger(dir.clone(), proposal(1, 300))
        .await
        .expect("explicit operator acknowledgement starts a fresh ledger");
    accepted.shutdown().await;
}

/// MEDIUM-3: a capability re-bound on a new connection supersedes the old one:
/// the old session's queued work is cancelled and its active work is uncertain
/// until the new session completes it.
#[tokio::test]
async fn rebinding_a_capability_supersedes_the_previous_session() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 60_000),
    )
    .await
    .unwrap();
    let old = root(&server).await;
    let credential = old.credential().unwrap();
    let permit = timeout(LIMIT, old.gate("acct").unwrap().acquire())
        .await
        .unwrap()
        .unwrap();
    let old_gate = old.gate("acct").unwrap();
    let queued = Box::pin(old_gate.acquire());
    assert!(timeout(Duration::from_millis(50), queued).await.is_err());
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    let fresh = AuthorityConnection::connect(&server.directory().client_socket())
        .await
        .unwrap();
    fresh.bind(credential).await.unwrap();
    wait_for(&admin, (1, 0)).await;
    assert_eq!(admin.inspect().await.unwrap().groups[&group()].uncertain, 1);
    fresh.complete(1, Feedback::Success).await.unwrap();
    std::mem::forget(permit);
    wait_for(&admin, (0, 0)).await;
    assert_eq!(admin.inspect().await.unwrap().groups[&group()].uncertain, 0);
    server.shutdown().await;
}

/// MEDIUM-7: while the ledger cannot be written the queue is neither drained
/// nor answered as "cancelled"; waiters hold until durability returns.
#[tokio::test]
async fn journal_outage_holds_queued_work_and_reports_the_ledger_not_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("authority");
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&path).unwrap(),
        proposal(2, 20_000),
    )
    .await
    .unwrap();
    let a = root(&server).await;
    let b = root(&server).await;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let gate_a = a.gate("acct").unwrap();
    let gate_b = b.gate("acct").unwrap();
    // The first selection discovers the outage: its grant is withdrawn and the
    // waiter learns it was the ledger, not a cancellation.
    let first = timeout(LIMIT, gate_a.acquire()).await.unwrap();
    let error = first
        .expect_err("withdrawn without a durable ledger")
        .to_string();
    assert!(error.contains("ledger"), "{error}");
    // Later requests are held while the journal is probed, never drained.
    let mut second = Box::pin(gate_b.acquire());
    assert!(
        timeout(Duration::from_millis(300), &mut second)
            .await
            .is_err()
    );
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    let status = admin.inspect().await.unwrap();
    assert_eq!(
        status.groups[&group()].queued,
        1,
        "queue is held, not drained"
    );
    assert!(!status.journal_healthy);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let p2 = timeout(LIMIT, second)
        .await
        .unwrap()
        .expect("granted once durable");
    p2.finish(Feedback::Success);
    wait_for(&admin, (0, 0)).await;
    server.shutdown().await;
}

/// LOW-6: a peer that sends a frame prefix and stalls is disconnected within
/// the framing deadline instead of holding a session forever.
#[tokio::test]
async fn a_stalled_frame_is_disconnected_within_the_framing_deadline() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let temp = tempfile::tempdir().unwrap();
    let server = AuthorityServer::start(
        AuthorityDirectory::open(&temp.path().join("authority")).unwrap(),
        proposal(1, 300),
    )
    .await
    .unwrap();
    let mut raw = tokio::net::UnixStream::connect(server.directory().client_socket())
        .await
        .unwrap();
    raw.write_all(&[0, 0, 0, 40]).await.unwrap();
    let mut buf = [0u8; 16];
    let closed = timeout(
        quecto::infrastructure::admission::protocol::FRAME_DEADLINE + Duration::from_secs(5),
        raw.read(&mut buf),
    )
    .await
    .expect("server closes the stalled session");
    assert!(matches!(closed, Ok(0) | Err(_)), "{closed:?}");
    server.shutdown().await;
}
