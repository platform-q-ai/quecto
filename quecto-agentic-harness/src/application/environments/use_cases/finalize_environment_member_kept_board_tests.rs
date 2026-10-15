//! #2481 S4c follow-up: a closed run's box goes with its board once its
//! coordinator is killed, so the final-member teardown reads the board
//! of a created run and keeps it under the leaving member before anything
//! tears the box down; the project board's run is folded back from it.

use std::sync::{Arc, Mutex};

use super::finalize_environment_member_retention_tests::{
    HostedRunPort, running_swarm, use_case, with_status,
};
use super::finalize_environment_member_tests::{block_on, committed_env};
use crate::application::environments::dto::{HostedBoardRevision, SwarmBoardSnapshot};
use crate::application::environments::ports::{EndedSwarmBoards, HostedSwarmBoard, PortFuture};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry,
};
use crate::domain::environments::services::environment_retention::{
    HostedSwarmRun, MemberFinalizeMode, SwarmRunObservation,
};
use crate::domain::swarm::RunStatus;

/// The board a fake reads at each environment (`None`: none hosted;
/// `Err`: unreadable), noting how many retained kills `port` had run as
/// it was read; and the boards kept, by member.
struct BoardPort {
    board: Result<Option<serde_json::Value>, String>,
    port: Arc<HostedRunPort>,
    kills_at_read: Mutex<Vec<usize>>,
    kept: Mutex<Vec<(String, serde_json::Value)>>,
}

impl BoardPort {
    fn reading(
        board: Result<Option<serde_json::Value>, String>,
        port: &Arc<HostedRunPort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            board,
            port: Arc::clone(port),
            kills_at_read: Mutex::new(Vec::new()),
            kept: Mutex::new(Vec::new()),
        })
    }
}

impl HostedSwarmBoard for BoardPort {
    fn board<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>> {
        let kills = self.port.kills.lock().unwrap().len();
        self.kills_at_read.lock().unwrap().push(kills);
        let board = self
            .board
            .clone()
            .map(|board| board.map(|board| SwarmBoardSnapshot::object(board).unwrap()));
        Box::pin(async move { board })
    }

    fn revision<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>> {
        Box::pin(async { panic!("a teardown never polls a revision") })
    }
}

impl EndedSwarmBoards for BoardPort {
    fn keep(&self, member: &str, board: SwarmBoardSnapshot) {
        self.kept
            .lock()
            .unwrap()
            .push((member.to_owned(), board.into_value()));
    }
}

/// [`finalize`], keeping boards through `boards`.
fn finalize_keeping(
    port: Arc<HostedRunPort>,
    boards: &Arc<BoardPort>,
    mode: MemberFinalizeMode,
) -> (EnvironmentRegistry, String) {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let use_case = use_case(&registry, port).keeping_ended_boards(boards.clone(), boards.clone());
    block_on(use_case.finalize_member(&env_ref, "coordinator-uuid", None, mode));
    (registry, env_ref)
}

/// #2481 S4c follow-up: a closed run's box goes with its board when its
/// coordinator is killed, so the board is read and kept under the
/// coordinator first, before the retained kill: the project board's run
/// is folded back from it afterwards.
#[test]
fn a_created_runs_board_is_kept_under_its_member_before_any_teardown() {
    let board = serde_json::json!({"revision": 9, "run": {"status": "succeeded"}});
    for (hosted, mode, torn_down) in [
        (
            with_status(RunStatus::Succeeded, None),
            MemberFinalizeMode::OwnerEnd,
            true,
        ),
        (
            with_status(RunStatus::Cancelled, None),
            MemberFinalizeMode::OwnerEnd,
            false,
        ),
        (running_swarm(), MemberFinalizeMode::Exit, false),
    ] {
        let port = Arc::new(HostedRunPort::hosting(Some(hosted.clone())));
        let boards = BoardPort::reading(Ok(Some(board.clone())), &port);
        finalize_keeping(port.clone(), &boards, mode);
        assert_eq!(
            boards.kept.lock().unwrap().as_slice(),
            &[("coordinator-uuid".to_owned(), board.clone())],
            "{hosted:?}"
        );
        assert_eq!(
            boards.kills_at_read.lock().unwrap().as_slice(),
            &[0],
            "read before any kill ({hosted:?})"
        );
        assert_eq!(
            port.kills.lock().unwrap().len(),
            usize::from(torn_down),
            "the teardown goes on as ever ({hosted:?})"
        );
    }
}

/// No board is read where no swarm run was created (a plain container, a
/// placeholder), nor on an owner's teardown of everything (nothing is
/// observed); and a board that cannot be read, or holds none, keeps
/// nothing while the teardown goes on.
#[test]
fn no_board_is_kept_where_no_created_run_is_seen_or_none_can_be_read() {
    let placeholder = HostedSwarmRun {
        deadline: 0.0,
        ..with_status(RunStatus::Succeeded, None)
    };
    for (observed, mode) in [
        (SwarmRunObservation::NoStore, MemberFinalizeMode::OwnerEnd),
        (
            SwarmRunObservation::Run(placeholder),
            MemberFinalizeMode::OwnerEnd,
        ),
        (
            SwarmRunObservation::Unreadable("locked".into()),
            MemberFinalizeMode::OwnerEnd,
        ),
        (
            SwarmRunObservation::Run(with_status(RunStatus::Succeeded, None)),
            MemberFinalizeMode::OwnerTeardown,
        ),
    ] {
        let port = Arc::new(HostedRunPort::observing(observed.clone()));
        let boards = BoardPort::reading(Ok(Some(serde_json::json!({}))), &port);
        finalize_keeping(port, &boards, mode);
        assert!(
            boards.kills_at_read.lock().unwrap().is_empty(),
            "never read: {observed:?}, {mode:?}"
        );
        assert!(boards.kept.lock().unwrap().is_empty());
    }
    for board in [Err("the board is locked".to_owned()), Ok(None)] {
        let port = Arc::new(HostedRunPort::hosting(Some(with_status(
            RunStatus::Succeeded,
            None,
        ))));
        let boards = BoardPort::reading(board.clone(), &port);
        finalize_keeping(port.clone(), &boards, MemberFinalizeMode::OwnerEnd);
        assert!(boards.kept.lock().unwrap().is_empty(), "{board:?}");
        assert_eq!(port.kills.lock().unwrap().len(), 1, "torn down: {board:?}");
    }
}
