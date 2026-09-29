//! The coordination board's graph (#2270, epic #2265): the only place its
//! use cases are constructed. `build_swarm_board_handles` binds the SQLite
//! repository of one board file, the wall clock and random ids; `main`
//! injects it through `CliComposition.swarm_board`, and S13 carries it on
//! `CliContext` and threads it to `SwarmContext` and `HostedStore`.
//! `build_swarm_board_handles_with` composes the same graph over injected
//! ports, for the differential harness's deterministic clock and ids.
use std::sync::Arc;

use crate::application::audit::ports::AuditSink;
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::application::swarm::use_cases::{
    ActivateMember, AdmitMember, BootstrapRun, ClaimTask, CreateRun, CreateTask, JoinRun,
    ReadRunSnapshot, ReadRunStatus, ReadTask, RecordMemberLaunch, RegisterMemberSocket,
    ReleaseTask, ReleaseUnlaunchedMember, SetTaskDependencies,
};
use crate::infrastructure::persistence::swarm_board::encoding::PyJsonEncoding;
use crate::infrastructure::persistence::swarm_board::ids::Uuid4Ids;
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
pub use crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles;
use crate::infrastructure::tools::swarm_lifecycle::SystemClock;

/// The board handles over the SQLite file at `location`.
pub fn build_swarm_board_handles(location: BoardLocation) -> SwarmBoardHandles {
    build_swarm_board_handles_with(
        Arc::new(SqliteBoardRepository::new(&location)),
        Arc::new(SystemClock),
        Arc::new(Uuid4Ids),
    )
}

/// The board handles over injected ports.
pub fn build_swarm_board_handles_with(
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
) -> SwarmBoardHandles {
    let admit_member = Arc::new(AdmitMember::new(repository.clone(), clock.clone()));
    let activate_member = Arc::new(ActivateMember::new(repository.clone(), clock.clone()));
    SwarmBoardHandles {
        create_run: Arc::new(CreateRun::new(
            repository.clone(),
            clock.clone(),
            ids.clone(),
            Arc::new(PyJsonEncoding),
        )),
        bootstrap_run: Arc::new(BootstrapRun::new(
            repository.clone(),
            clock.clone(),
            ids.clone(),
        )),
        read_run_status: Arc::new(ReadRunStatus::new(repository.clone())),
        read_run_snapshot: Arc::new(ReadRunSnapshot::new(repository.clone(), clock.clone())),
        record_member_launch: Arc::new(RecordMemberLaunch::new(repository.clone(), clock.clone())),
        release_unlaunched_member: Arc::new(ReleaseUnlaunchedMember::new(
            repository.clone(),
            clock.clone(),
        )),
        register_member_socket: Arc::new(RegisterMemberSocket::new(
            repository.clone(),
            clock.clone(),
        )),
        create_task: Arc::new(CreateTask::new(
            repository.clone(),
            clock.clone(),
            Arc::new(PyJsonEncoding),
        )),
        set_task_dependencies: Arc::new(SetTaskDependencies::new(
            repository.clone(),
            clock.clone(),
        )),
        claim_task: Arc::new(ClaimTask::new(
            repository.clone(),
            clock.clone(),
            ids.clone(),
        )),
        release_task: Arc::new(ReleaseTask::new(repository.clone(), clock.clone())),
        read_task: Arc::new(ReadTask::new(repository.clone(), clock.clone())),
        join_run: Arc::new(JoinRun::new(
            repository,
            ids,
            admit_member.clone(),
            activate_member.clone(),
        )),
        admit_member,
        activate_member,
        event_log: None,
    }
}

/// `handles` recording a `swarm_op` per call in `event_log` (#2303). The
/// caller passes the log only when `telemetry.event_log.enabled` is on
/// (owner decision T1): without it nothing is measured or written.
pub fn with_event_log(
    handles: SwarmBoardHandles,
    event_log: Arc<dyn AuditSink>,
) -> SwarmBoardHandles {
    SwarmBoardHandles {
        event_log: Some(event_log),
        ..handles
    }
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
