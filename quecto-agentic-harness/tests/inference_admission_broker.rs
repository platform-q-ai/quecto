//! P3 authority process adapters over real UDS/files: singleton lock, durable
//! journal, framed protocol, client capability lifetime and recovery.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use quecto::application::ports::AdmissionJournal;
use quecto::domain::inference_admission::*;
use quecto::infrastructure::admission::{
    AdminConnection, AuthorityConnection, AuthorityDirectory, AuthorityServer, ClientError,
    FileJournal, SingletonLock,
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
        proposal(1, 5_000),
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
    timeout(LIMIT, async {
        loop {
            if admin.inspect().await.unwrap().groups[&group()].queued == 0 {
                break;
            }
            tokio::task::yield_now().await;
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
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(denied.is_err(), "no grant without a durable ledger");
    let admin = AdminConnection::connect(&server.directory().admin_socket())
        .await
        .unwrap();
    assert!(!admin.inspect().await.unwrap().journal_healthy);
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
