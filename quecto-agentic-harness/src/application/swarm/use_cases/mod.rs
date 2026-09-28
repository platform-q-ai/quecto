//! The coordination board's use cases (epic #2265), one per file. Each is
//! constructed only by `composition::swarm`; the dispatcher
//! (`infrastructure::tools::swarm_board_dispatch`) holds injected handles.

mod bootstrap_run;
mod create_run;
mod read_run_snapshot;
mod read_run_status;

pub use bootstrap_run::BootstrapRun;
pub use create_run::CreateRun;
pub use read_run_snapshot::ReadRunSnapshot;
pub use read_run_status::ReadRunStatus;

#[cfg(test)]
pub(crate) mod fakes;
