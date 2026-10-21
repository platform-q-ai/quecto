//! Contract for the `AuthorityAdmin` port (#2024 S3): a directory with no
//! broker answers `NotRunning`; a running authority answers inspect with the
//! epoch and its directory, and reset advances the epoch. The adapter under
//! test drives a real in-process `AuthorityServer` over its admin socket.
use std::collections::BTreeMap;
use std::sync::Arc;

use quecto::application::admission::dto::authority::AuthorityAdminError;
use quecto::application::admission::ports::AuthorityAdmin;
use quecto::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, GroupId, GroupPolicy,
};
use quecto::infrastructure::admission::{AuthorityDirectory, SocketAuthorityAdmin};
use quecto::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;

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
        .block_on(quecto::composition::admission::start_authority(
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
    assert!(!report.on_demand, "an always-on authority");

    let reset = admin.reset(&directory).unwrap();
    assert_eq!(reset.directory, directory);
    assert_eq!(reset.epoch, 2);

    rt.block_on(server.shutdown());
}

/// #2615: stop asks the running authority to stop serving; it answers, then
/// stops itself on request. Nothing running is `NotRunning`.
#[test]
fn stop_asks_the_running_authority_to_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("authority");
    let admin = SocketAuthorityAdmin::new();
    assert!(matches!(
        admin.stop(&directory),
        Err(AuthorityAdminError::NotRunning { .. })
    ));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = rt
        .block_on(quecto::composition::admission::start_authority(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    admin.stop(&directory).unwrap();
    let reason = rt
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), server.stopped()).await
        })
        .expect("the authority stops on request");
    assert_eq!(
        reason,
        quecto::domain::admission::value_objects::authority_idle::AuthorityStop::Requested
    );
    rt.block_on(server.shutdown());
}

/// #2615 review: an authority started on demand says so to `status`.
#[test]
fn an_on_demand_authority_reports_itself_as_one() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("authority");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = rt
        .block_on(
            quecto::composition::admission::start_authority_stopping_when_idle(
                AuthorityDirectory::open(&directory).unwrap(),
                proposal(),
                std::time::Duration::from_secs(3600),
            ),
        )
        .unwrap();
    let report = SocketAuthorityAdmin::new().inspect(&directory).unwrap();
    assert!(report.on_demand);
    rt.block_on(server.shutdown());
}

/// #2615 review: a broker that accepts but never answers (stopped with
/// SIGSTOP, say) fails each admin call within its bound, as `Failed`, never
/// as `NotRunning` and never by waiting forever.
#[test]
fn a_broker_that_never_answers_fails_within_the_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let directory = tmp.path().join("authority");
    std::fs::create_dir_all(&directory).unwrap();
    let _silent = std::os::unix::net::UnixListener::bind(directory.join("admin.sock")).unwrap();
    let admin = SocketAuthorityAdmin::bounded_by(std::time::Duration::from_millis(200));
    let started = std::time::Instant::now();
    for result in [
        admin.inspect(&directory).map(drop),
        admin.stop(&directory),
        admin.reset(&directory).map(drop),
    ] {
        assert!(
            matches!(result, Err(AuthorityAdminError::Failed { ref reason, .. }) if reason.contains("no answer")),
            "{result:?}"
        );
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}
