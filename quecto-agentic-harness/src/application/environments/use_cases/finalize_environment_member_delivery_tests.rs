//! #2666 round 2: the owner's kill of a succeeded board run's coordinator
//! tore its box down with a commit never pushed. The teardown now asks
//! where the run's completion went, and keeps the box (#1924's retained
//! state, `kill_container` to remove it) while the commit is not on the
//! board's remote or that cannot be told, so the work can be pushed from
//! its workspace. Any other end, or run, is torn down as before.

use std::sync::{Arc, Mutex};

use super::finalize_environment_member_retention_tests::{
    HostedRunPort, running_swarm, use_case, with_status,
};
use super::finalize_environment_member_tests::{block_on, committed_env};
use crate::application::environments::dto::{HostedBoardRevision, SwarmBoardSnapshot};
use crate::application::environments::ports::{
    PortFuture, hosted_swarm::CompletionDeliveryCheck, hosted_swarm::EndedSwarmBoards,
    hosted_swarm::HostedSwarmBoard,
};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environments::services::environment_retention::{
    CompletionDelivery, MemberFinalizeMode,
};
use crate::domain::swarm::value_objects::lifecycle::RunStatus;

/// The board read at the box, and the boards kept.
struct Boards;

impl HostedSwarmBoard for Boards {
    fn board<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let board = SwarmBoardSnapshot::object(serde_json::json!({"run": {"status": "succeeded"}}));
        Box::pin(async move { Ok(board) })
    }

    fn revision<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async { panic!("a teardown never polls a revision") })
    }
}

impl EndedSwarmBoards for Boards {
    fn keep(&self, _: &str, _: &str, _: SwarmBoardSnapshot) {}
}

/// A check answering `answer`, noting each label it was asked about and
/// whether it was given the board.
struct Check {
    answer: CompletionDelivery,
    asked: Mutex<Vec<(String, bool)>>,
}

impl CompletionDeliveryCheck for Check {
    fn delivery<'a>(
        &'a self,
        _session: &'a str,
        label: &'a str,
        board: Option<&'a SwarmBoardSnapshot>,
    ) -> PortFuture<'a, CompletionDelivery> {
        self.asked
            .lock()
            .unwrap()
            .push((label.to_owned(), board.is_some()));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

const LABEL: &str = "board-a-run-1";

/// The coordinator `LABEL` of `hosted` finalized in `mode`, the check
/// answering `answer`: the port, the check, the registry and the box.
fn finalized(
    hosted: crate::domain::environments::services::environment_retention::HostedSwarmRun,
    mode: MemberFinalizeMode,
    answer: CompletionDelivery,
) -> (Arc<HostedRunPort>, Arc<Check>, EnvironmentRegistry, String) {
    let port = Arc::new(HostedRunPort::hosting(Some(hosted)));
    let check = Arc::new(Check {
        answer,
        asked: Mutex::new(Vec::new()),
    });
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let finalizer = use_case(&registry, Arc::clone(&port))
        .keeping_ended_boards(Arc::new(Boards), Arc::new(Boards))
        .checking_delivery(Arc::clone(&check) as _);
    block_on(finalizer.finalize_labelled_member(
        &env_ref,
        "coordinator-uuid",
        Some(LABEL),
        None,
        mode,
    ));
    (port, check, registry, env_ref)
}

fn retained(registry: &EnvironmentRegistry, env_ref: &str) -> Option<String> {
    let record = registry.get(env_ref)?;
    match record.status {
        EnvironmentStatus::Retained => record.metadata["retained"].as_str().map(str::to_owned),
        _ => None,
    }
}

#[test]
fn a_succeeded_runs_box_is_kept_while_its_completion_is_not_on_the_remote() {
    let not_there = CompletionDelivery::NotOnRemote {
        revision: "2b23fe18d39ae8692eaab45d60cc50ff1d66c1d3".into(),
    };
    let (port, check, registry, env_ref) = finalized(
        with_status(RunStatus::Succeeded, None),
        MemberFinalizeMode::OwnerEnd,
        not_there,
    );
    assert_eq!(
        check.asked.lock().unwrap().as_slice(),
        &[(LABEL.to_owned(), true)]
    );
    assert!(port.kills.lock().unwrap().is_empty(), "no retained kill");
    assert_eq!(
        retained(&registry, &env_ref),
        Some(format!(
            "board run succeeded at 2b23fe18d39ae8692eaab45d60cc50ff1d66c1d3, which is on no \
             branch or tag of the board's remote: environment {env_ref} kept so its work can \
             be pushed from its workspace /ws/{env_ref}; kill_container {env_ref} to remove it \
             once pushed"
        ))
    );
}

#[test]
fn a_box_is_kept_when_where_the_completion_went_cannot_be_told() {
    let (port, _, registry, env_ref) = finalized(
        with_status(RunStatus::Succeeded, None),
        MemberFinalizeMode::OwnerEnd,
        CompletionDelivery::Unknown {
            why: "the remote is down".into(),
        },
    );
    assert!(port.kills.lock().unwrap().is_empty());
    assert_eq!(
        retained(&registry, &env_ref),
        Some(format!(
            "board run succeeded, but whether its completion commit is on the board's remote \
             could not be checked (the remote is down): environment {env_ref} kept so its work \
             can be pushed from its workspace /ws/{env_ref}; kill_container {env_ref} to remove \
             it once pushed"
        ))
    );
}

#[test]
fn a_box_whose_completion_is_on_the_remote_or_not_judged_goes_as_before() {
    for answer in [CompletionDelivery::OnRemote, CompletionDelivery::NotJudged] {
        let (port, _, registry, env_ref) = finalized(
            with_status(RunStatus::Succeeded, None),
            MemberFinalizeMode::OwnerEnd,
            answer.clone(),
        );
        assert_eq!(port.kills.lock().unwrap().len(), 1, "{answer:?}");
        assert_eq!(retained(&registry, &env_ref), None, "{answer:?}");
    }
}

/// Only the owner's kill of a run that succeeded is judged: a failed or
/// cancelled run, or a harness shutdown, never asks.
#[test]
fn only_the_owners_kill_of_a_succeeded_run_is_judged() {
    let not_there = || CompletionDelivery::NotOnRemote {
        revision: "abc".into(),
    };
    for (hosted, mode) in [
        (
            with_status(RunStatus::Failed, None),
            MemberFinalizeMode::OwnerEnd,
        ),
        (
            with_status(RunStatus::Cancelled, None),
            MemberFinalizeMode::OwnerEnd,
        ),
        (
            with_status(RunStatus::Succeeded, None),
            MemberFinalizeMode::ParentKill,
        ),
        (
            with_status(RunStatus::Succeeded, None),
            MemberFinalizeMode::OwnerTeardown,
        ),
        (running_swarm(), MemberFinalizeMode::OwnerEnd),
    ] {
        let (_, check, _, _) = finalized(hosted.clone(), mode, not_there());
        assert!(
            check.asked.lock().unwrap().is_empty(),
            "{hosted:?} {mode:?}"
        );
    }
}
