//! Contract for the `AuthorityAdmin` port (#2024 S3): a directory with no
//! broker answers `NotRunning`; a running authority answers inspect with the
//! epoch and its directory, and reset advances the epoch. The adapter under
//! test drives a real in-process `AuthorityServer` over its admin socket.
use std::collections::BTreeMap;
use std::sync::Arc;

use quecto::application::admission::dto::AuthorityAdminError;
use quecto::application::admission::ports::AuthorityAdmin;
use quecto::domain::inference_admission::{AdmissionConfig, GroupId, GroupPolicy};
use quecto::infrastructure::admission::{
    AuthorityDirectory, AuthorityServer, SocketAuthorityAdmin,
};
use quecto::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

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
fn an_unreachable_directory_is_reported_as_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    let admin = SocketAuthorityAdmin::new();
    let directory = tmp.path().join("nothing-here");
    let error = admin.inspect(&directory).unwrap_err();
    assert!(
        matches!(error, AuthorityAdminError::NotRunning { directory: d, .. } if d == directory)
    );
}

#[test]
fn inspect_and_reset_address_the_running_authority_by_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("authority");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    let admin = Arc::new(SocketAuthorityAdmin::new());

    let report = admin.inspect(&directory).unwrap();
    assert_eq!(report.directory, directory);
    assert_eq!(report.epoch, 1);
    assert_eq!(report.live_scopes, 0);
    assert!(report.groups.contains_key("g"));

    let reset = admin.reset(&directory).unwrap();
    assert_eq!(reset.directory, directory);
    assert_eq!(reset.epoch, 2);

    rt.block_on(server.shutdown());
}
