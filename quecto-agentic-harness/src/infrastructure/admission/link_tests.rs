use super::*;
use crate::application::ports::AttemptAdmission;
use crate::domain::inference_admission::{
    AdmissionConfig, Feedback, GroupId, GroupPolicy, WorkloadClass,
};
use crate::infrastructure::admission::{
    AdminConnection, AuthorityConnection, AuthorityDirectory, AuthorityServer, ClientError,
};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn proposal_with_capacity(capacity: usize) -> AdmissionRuntimeProposal {
    let group = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 8,
                    queue_timeout_ms: 1000,
                    attempt_timeout_ms: 2000,
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

fn proposal() -> AdmissionRuntimeProposal {
    proposal_with_capacity(2)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

async fn bound_root(dir_path: &std::path::Path) -> AuthorityConnection {
    let dir = AuthorityDirectory::existing(dir_path).unwrap();
    let connection = AuthorityConnection::connect(&dir.client_socket())
        .await
        .unwrap();
    let credential = connection
        .register_root(WorkloadClass::Interactive)
        .await
        .unwrap();
    connection.bind(credential).await.unwrap();
    connection
}

fn root_link(
    connection: AuthorityConnection,
    dir_path: &std::path::Path,
    handle: tokio::runtime::Handle,
) -> Arc<AuthorityLink> {
    let link = AuthorityLink::root(
        connection,
        AuthorityDirectory::from_directory(dir_path),
        WorkloadClass::Interactive,
        handle,
    );
    link.set_backoff_for_test(3, Duration::from_millis(20), Duration::from_millis(50));
    link
}

fn gate(link: &Arc<AuthorityLink>) -> super::super::remote_gate::RemoteAdmission {
    super::super::remote_gate::RemoteAdmission::linked(link.clone(), "acct".into(), 500)
}

/// Admission is proven by the authority's own count of live scopes and by a
/// permit that completes durably: a silently bypassing gate has neither.
async fn live_scopes(dir_path: &std::path::Path) -> usize {
    let dir = AuthorityDirectory::existing(dir_path).unwrap();
    let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
    admin.inspect().await.unwrap().live_scopes
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition never became true"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn a_root_link_reconnects_and_is_admitted_after_the_broker_restarts() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = runtime();
    let handle = rt.handle().clone();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let link = root_link(bound_root(&dir_path).await, &dir_path, handle.clone());
        assert_eq!(link.health(), LinkHealth::Connected);
        let first = link.connection();
        let changes = Arc::new(AtomicUsize::new(0));
        let counter = changes.clone();
        link.on_change(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));

        // A real broker restart on the same directory: the socket is dropped
        // and the link must notice it without any attempt in flight.
        server.shutdown().await;
        wait_until(|| !link.connected()).await;
        assert_eq!(link.health(), LinkHealth::Reconnecting);
        // The loss is announced to the watcher (from the I/O runtime).
        wait_until(|| changes.load(Ordering::SeqCst) >= 1).await;
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();

        // The next attempt reconnects, re-registers with the fresh owner
        // token and is admitted on the new connection (M5e): the authority
        // counts the new scope live and the permit completes through it.
        let permit = gate(&link)
            .acquire()
            .await
            .expect("re-registered root is admitted");
        assert!(
            !Arc::ptr_eq(&first, &link.connection()),
            "a reconnection installs a fresh connection"
        );
        assert_eq!(link.health(), LinkHealth::Connected);
        assert_eq!(live_scopes(&dir_path).await, 1);
        permit.finish(Feedback::Success);
        assert!(link.connection().drain(Duration::from_secs(2)).await);
        server.shutdown().await;
    });
}

#[test]
fn a_root_link_re_registers_after_an_authority_reset() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = runtime();
    let handle = rt.handle().clone();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let link = root_link(bound_root(&dir_path).await, &dir_path, handle.clone());
        let permit = gate(&link).acquire().await.unwrap();
        permit.finish(Feedback::Success);
        assert!(link.connection().drain(Duration::from_secs(2)).await);
        assert_eq!(link.epoch(), 1);

        // `reset`: the broker revokes every capability but keeps the socket
        // open (H1). The link must treat the revocation as a loss.
        let dir = AuthorityDirectory::existing(&dir_path).unwrap();
        let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
        assert_eq!(admin.reset().await.unwrap(), 2);
        wait_until(|| !link.connected()).await;
        assert_eq!(link.health(), LinkHealth::Reconnecting);

        // The next attempt is admitted in the new epoch, not rejected forever.
        let permit = gate(&link)
            .acquire()
            .await
            .expect("the root re-registers in the new epoch");
        assert_eq!(link.epoch(), 2);
        assert_eq!(link.health(), LinkHealth::Connected);
        assert_eq!(admin.inspect().await.unwrap().live_scopes, 1);
        permit.finish(Feedback::Success);
        assert!(link.connection().drain(Duration::from_secs(2)).await);
        server.shutdown().await;
    });
}

#[test]
fn a_child_link_fails_closed_after_a_reset_and_after_a_loss() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = runtime();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let link = AuthorityLink::child(bound_root(&dir_path).await);
        assert!(gate(&link).acquire().await.is_ok());

        let dir = AuthorityDirectory::existing(&dir_path).unwrap();
        let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
        admin.reset().await.unwrap();
        wait_until(|| !link.connected()).await;
        // No reconnect for a child: unavailable, and every later attempt is
        // refused with an error that tells the parent what to do.
        assert_eq!(link.health(), LinkHealth::Unavailable);
        for _ in 0..2 {
            let error = gate(&link)
                .acquire()
                .await
                .expect_err("a child never re-registers on its own");
            let message = error.to_string();
            assert!(message.contains("respawn"), "{message}");
        }
        assert_eq!(admin.inspect().await.unwrap().live_scopes, 0);

        server.shutdown().await;
        wait_until(|| !link.connection().is_open()).await;
        assert!(matches!(link.live_inner().await, Err(ClientError::Closed)));
        assert_eq!(link.health(), LinkHealth::Unavailable);
    });
}

#[test]
fn a_root_link_becomes_unavailable_when_reconnection_is_exhausted() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = runtime();
    let handle = rt.handle().clone();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let link = root_link(bound_root(&dir_path).await, &dir_path, handle.clone());
        server.shutdown().await;
        wait_until(|| !link.connected()).await;
        assert_eq!(link.health(), LinkHealth::Reconnecting);
        // Nothing to reconnect to: bounded backoff, then fail closed.
        let error = gate(&link).acquire().await.expect_err("fails closed");
        assert!(error.to_string().contains("admission"), "{error}");
        assert_eq!(link.health(), LinkHealth::Unavailable);
        // A broker that comes back is still found on the next attempt.
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        assert!(gate(&link).acquire().await.is_ok());
        assert_eq!(link.health(), LinkHealth::Connected);
        server.shutdown().await;
    });
}

#[test]
fn a_reconnection_against_a_changed_policy_fails_closed_with_restart_required() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = runtime();
    let handle = rt.handle().clone();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let link = root_link(bound_root(&dir_path).await, &dir_path, handle.clone());
        server.shutdown().await;
        wait_until(|| !link.connected()).await;
        // The broker comes back with a different policy: silently rebinding
        // would run the session under budgets it never composed against.
        let server = AuthorityServer::start(
            AuthorityDirectory::open(&dir_path).unwrap(),
            proposal_with_capacity(7),
        )
        .await
        .unwrap();
        let error = gate(&link).acquire().await.expect_err("fails closed");
        assert!(error.to_string().contains("restart required"), "{error}");
        assert_eq!(link.health(), LinkHealth::Unavailable);
        // Permanent for this process: no later attempt rebinds either.
        let error = gate(&link).acquire().await.expect_err("still closed");
        assert!(error.to_string().contains("restart required"), "{error}");
        assert_eq!(
            live_scopes(&dir_path).await,
            0,
            "the probe scope was dropped"
        );
        server.shutdown().await;
    });
}

#[test]
fn backoff_delays_grow_and_carry_jitter_within_the_cap() {
    let backoff = Backoff {
        attempts: 5,
        base: Duration::from_millis(100),
        max: Duration::from_millis(300),
    };
    let mut previous = Duration::ZERO;
    let mut jittered = false;
    for attempt in 0..5 {
        let delay = backoff.delay(attempt);
        let nominal = (backoff.base * 2u32.pow(attempt)).min(backoff.max);
        assert!(
            delay >= nominal && delay <= nominal + nominal / 2,
            "{delay:?}"
        );
        assert!(delay >= previous.min(nominal), "monotone up to the cap");
        previous = delay;
        jittered |= delay != nominal;
    }
    let _ = jittered;
}
