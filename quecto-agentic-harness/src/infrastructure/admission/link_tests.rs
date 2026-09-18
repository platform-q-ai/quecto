use super::*;
use crate::domain::inference_admission::{AdmissionConfig, GroupId, GroupPolicy, WorkloadClass};
use crate::infrastructure::admission::{AuthorityDirectory, AuthorityServer};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use std::collections::BTreeMap;

fn proposal() -> AdmissionRuntimeProposal {
    let group = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
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

#[test]
fn a_root_link_reconnects_after_the_broker_restarts() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.handle().clone();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let dir = AuthorityDirectory::existing(&dir_path).unwrap();
        let connection =
            crate::infrastructure::admission::AuthorityConnection::connect(&dir.client_socket())
                .await
                .unwrap();
        let credential = connection
            .register_root(WorkloadClass::Interactive)
            .await
            .unwrap();
        connection.bind(credential).await.unwrap();
        let link = AuthorityLink::root(
            connection,
            AuthorityDirectory::from_directory(&dir_path),
            WorkloadClass::Interactive,
            handle.clone(),
        );
        // Live before the loss.
        assert!(link.live_inner().await.is_ok());
        let first = link.connection();

        // Simulate the broker dropping the socket (a real broker death does
        // this by closing the fd; here we force it deterministically). The
        // broker itself stays up, standing in for a fresh restart on the same
        // directory that the reconnection re-registers against.
        link.close_current_for_test();
        assert!(!link.connection().is_open());

        // The link reconnects on the next use, without any caller restart, and
        // installs a fresh connection.
        let inner = link
            .live_inner()
            .await
            .expect("link reconnects to the broker");
        assert!(!inner.closed.load(std::sync::atomic::Ordering::Acquire));
        assert!(link.connection().is_open());
        assert!(
            !std::sync::Arc::ptr_eq(&first, &link.connection()),
            "a reconnection installs a fresh connection"
        );
        server.shutdown().await;
    });
}

#[test]
fn a_child_link_fails_closed_without_reconnecting() {
    let temp = tempfile::tempdir().unwrap();
    let dir_path = temp.path().join("authority");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async move {
        let server =
            AuthorityServer::start(AuthorityDirectory::open(&dir_path).unwrap(), proposal())
                .await
                .unwrap();
        let dir = AuthorityDirectory::existing(&dir_path).unwrap();
        let connection =
            crate::infrastructure::admission::AuthorityConnection::connect(&dir.client_socket())
                .await
                .unwrap();
        let credential = connection
            .register_root(WorkloadClass::Interactive)
            .await
            .unwrap();
        connection.bind(credential).await.unwrap();
        let link = AuthorityLink::child(connection);
        assert!(link.live_inner().await.is_ok());
        link.close_current_for_test();
        // No reconnect for a child: it fails closed rather than silently
        // bypassing admission.
        assert!(matches!(
            link.live_inner().await,
            Err(crate::infrastructure::admission::ClientError::Closed)
        ));
        server.shutdown().await;
    });
}
