//! The coordination board's graph (#2270, epic #2265): the only place its
//! use cases are constructed. `build_swarm_board_handles` binds the SQLite
//! repository of one board file, the wall clock and random ids, and the
//! event log's `swarm_op` records when it is given one (#2303); `main`
//! injects it through `CliComposition.swarm_board`, and S13 carries it on
//! `CliContext` and threads it to `SwarmContext` and `HostedStore`, passing
//! [`board_op_log`] of the session's log. `build_swarm_board_handles_with`
//! composes the same graph over injected ports, for the differential
//! harness's deterministic clock and ids.
use std::sync::Arc;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardOpLog, BoardRepository, Clock, IdSource};
use crate::application::swarm::use_cases::{
    ActivateMember, AdmitMember, BlockTask, BootstrapRun, ClaimTask, CreateRun, CreateTask,
    JoinRun, ReadRunSnapshot, ReadRunStatus, ReadTask, RecordMemberLaunch, RegisterMemberSocket,
    ReleaseTask, ReleaseUnlaunchedMember, SetTaskDependencies, SubmitTask, UnblockTask, VerifyTask,
};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::board_op_log::EventLogBoardOps;
use crate::infrastructure::persistence::swarm_board::encoding::PyJsonEncoding;
use crate::infrastructure::persistence::swarm_board::ids::Uuid4Ids;
use crate::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
pub use crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles;
use crate::infrastructure::tools::swarm_board_dispatch::{ActorRefs, BoardTelemetry};
use crate::infrastructure::tools::swarm_lifecycle::SystemClock;

/// The board handles over the SQLite file at `location`, recording each
/// call in `event_log` when there is one ([`board_op_log`]: only while the
/// event log is on).
pub fn build_swarm_board_handles(
    location: BoardLocation,
    event_log: Option<Arc<dyn BoardOpLog>>,
) -> SwarmBoardHandles {
    let repository = SqliteBoardRepository::new(&location);
    let (clock, ids) = (Arc::new(SystemClock), Arc::new(Uuid4Ids));
    match event_log {
        Some(event_log) => with_event_log(repository, clock, ids, event_log),
        None => build_swarm_board_handles_with(Arc::new(repository), clock, ids),
    }
}

/// Where the board records its `swarm_op`s: the session's event log, only
/// when `event_log_enabled` (`telemetry.event_log.enabled`, owner decision
/// T1), and only when the log has a synchronous line to append through.
/// `None` otherwise: nothing is measured or written.
pub fn board_op_log(event_log_enabled: bool, log: &AuditLog) -> Option<Arc<dyn BoardOpLog>> {
    match event_log_enabled {
        true => log
            .crash_line()
            .map(|line| Arc::new(EventLogBoardOps::new(line)) as Arc<dyn BoardOpLog>),
        false => None,
    }
}

/// The board handles over injected ports.
pub fn build_swarm_board_handles_with(
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
) -> SwarmBoardHandles {
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
        block_task: Arc::new(BlockTask::new(repository.clone(), clock.clone())),
        unblock_task: Arc::new(UnblockTask::new(repository.clone(), clock.clone())),
        submit_task: Arc::new(SubmitTask::new(
            repository.clone(),
            clock.clone(),
            Arc::new(PyJsonEncoding),
        )),
        verify_task: Arc::new(VerifyTask::new(repository.clone(), clock.clone())),
        admit_member: Arc::new(AdmitMember::new(repository.clone(), clock.clone())),
        activate_member: Arc::new(ActivateMember::new(repository.clone(), clock.clone())),
        join_run: Arc::new(JoinRun::new(repository, clock, ids)),
        telemetry: None,
    }
}

/// The board handles over `repository`'s file, recording a `swarm_op` per
/// call in `event_log` (#2303). The caller passes the log only when
/// `telemetry.event_log.enabled` is on (owner decision T1): without it
/// nothing is measured or written.
///
/// The graph is composed once. Each call is served by the same use cases
/// over a metered call of its own ([`SqliteBoardCallMeter`]) that carries
/// that call's own measure: no call's measure is shared, and none passes
/// through ambient state.
pub fn with_event_log(
    repository: SqliteBoardRepository,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
    event_log: Arc<dyn BoardOpLog>,
) -> SwarmBoardHandles {
    let handles = build_swarm_board_handles_with(Arc::new(repository.clone()), clock, ids);
    SwarmBoardHandles {
        telemetry: Some(BoardTelemetry {
            log: event_log,
            meter: Arc::new(SqliteBoardCallMeter::new(repository)),
            actors: Arc::new(ActorRefs::default()),
        }),
        ..handles
    }
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
