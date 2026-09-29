//! The coordination board's graph (#2270, epic #2265): the only place its
//! use cases are constructed. `build_swarm_board_handles` binds the SQLite
//! repository of one board file, the wall clock and random ids; `main`
//! injects it through `CliComposition.swarm_board`, and S13 carries it on
//! `CliContext` and threads it to `SwarmContext` and `HostedStore`.
//! `build_swarm_board_handles_with` composes the same graph over injected
//! ports, for the differential harness's deterministic clock and ids.
use std::sync::Arc;

use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::application::swarm::use_cases::{
    BootstrapRun, CreateRun, ReadRunSnapshot, ReadRunStatus,
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
    SwarmBoardHandles {
        create_run: Arc::new(CreateRun::new(
            repository.clone(),
            clock.clone(),
            ids.clone(),
            Arc::new(PyJsonEncoding),
        )),
        bootstrap_run: Arc::new(BootstrapRun::new(repository.clone(), clock.clone(), ids)),
        read_run_status: Arc::new(ReadRunStatus::new(repository.clone())),
        read_run_snapshot: Arc::new(ReadRunSnapshot::new(repository, clock)),
    }
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
