//! Contract for the `AuthoritySession` port (#2615): over an empty binding
//! slot nothing is joined and nothing rejoins; a root binding answers the
//! launch it remembered, is connected and rejoins its broker, and is told
//! the policy changed when its broker came back with another one; a child
//! binding answers `Child`. Over real in-process authorities.
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use quecto::application::admission::ports::{AuthorityLaunch, AuthoritySession, JoinedAuthority};
use quecto::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, GroupId, GroupPolicy,
};
use quecto::infrastructure::admission::process::install_in;
use quecto::infrastructure::admission::{
    AuthorityDirectory, CurrentProcessSession, Negotiation, ProcessAdmission,
    write_admission_context,
};
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
                    queue_timeout_ms: 1_000,
                    attempt_timeout_ms: 2_000,
                    fallback_base_ms: 50,
                    max_cooldown_ms: 500,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), group)]),
            max_scopes: 16,
            terminal_capacity: 32,
        },
        bindings: BTreeMap::from([("fake".into(), "acct".into())]),
    }
}

fn slot() -> &'static OnceLock<Arc<ProcessAdmission>> {
    Box::leak(Box::new(OnceLock::new()))
}

#[test]
fn an_empty_slot_has_joined_nothing() {
    let session = CurrentProcessSession::over(slot());
    assert_eq!(session.joined(), None);
    assert_eq!(session.health(), "unavailable");
    assert!(session.rejoin().is_err());
}

#[test]
fn a_root_answers_its_launch_and_rejoins_and_a_child_is_a_child() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = runtime
        .block_on(quecto::composition::admission::start_authority(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    let root_slot = slot();
    let root = install_in(
        root_slot,
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    let launch = AuthorityLaunch {
        directory: directory.clone(),
        config: temp.path().join("config.json"),
        base_dir: temp.path().to_path_buf(),
    };
    root.remember_launch(launch.clone());
    let session = CurrentProcessSession::over(root_slot);
    assert_eq!(session.joined(), Some(JoinedAuthority::Root(launch)));
    assert_eq!(session.health(), "connected");
    assert_eq!(session.rejoin(), Ok(()));
    let credential = runtime.block_on(root.register_child()).unwrap();
    let context = temp.path().join("child.ctx");
    write_admission_context(&context, &root.endpoint(), &credential).unwrap();
    let child_slot = slot();
    install_in(child_slot, Negotiation::Child { context }).unwrap();
    assert_eq!(
        CurrentProcessSession::over(child_slot).joined(),
        Some(JoinedAuthority::Child)
    );
    runtime.block_on(server.shutdown());
}

/// #2615: a root whose broker came back publishing a different policy
/// cannot rejoin it; the refusal says the policy changed (only a session
/// restart adopts it), not merely that the rejoin failed.
#[test]
fn a_broker_back_with_another_policy_refuses_the_rejoin_as_policy_changed() {
    use quecto::application::admission::ports::RejoinRefusal;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let first = runtime
        .block_on(quecto::composition::admission::start_authority(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    let root_slot = slot();
    install_in(
        root_slot,
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    let session = CurrentProcessSession::over(root_slot);
    runtime.block_on(first.shutdown());
    let mut changed = proposal();
    for group in changed.policy.groups.values_mut() {
        group.capacity = 2;
    }
    let second = runtime
        .block_on(quecto::composition::admission::start_authority(
            AuthorityDirectory::open(&directory).unwrap(),
            changed,
        ))
        .unwrap();
    assert_eq!(session.rejoin(), Err(RejoinRefusal::PolicyChanged));
    assert_eq!(session.health(), "unavailable");
    runtime.block_on(second.shutdown());
}
