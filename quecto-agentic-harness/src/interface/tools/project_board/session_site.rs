//! The agent session's board (#2477): its repository's board, the
//! coordinator launcher a start launches through (S4c), and the keeping
//! of the runs it starts (S4d), which reads the swarm boards the session's
//! environment control carries, on the session's runtime.
use std::path::PathBuf;
use std::sync::Arc;

use super::keeper::{SessionEnd, SessionKeeper, SessionSwarmBoards};
use super::site::{BoardSite, Launcher, ProjectBoardHandlesBuilder};
use crate::application::project_board::keeper::KEEPER_PERIOD;
use crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot;

/// The board of `repo`'s `origin`, built by `build`, launching with
/// `launcher`; it keeps the runs it starts while the session has a runtime
/// to read their swarms on.
pub fn session_site(
    repo: PathBuf,
    build: ProjectBoardHandlesBuilder,
    launcher: Launcher,
    environments: EnvironmentControlSlot,
) -> BoardSite {
    let keeper = tokio::runtime::Handle::try_current()
        .ok()
        .map(|runtime| SessionKeeper {
            end: Arc::new(SessionEnd::default()),
            swarm: Arc::new(SessionSwarmBoards::new(
                Arc::new(move || environments.get().map(|control| control.boards)),
                runtime,
            )),
            period: KEEPER_PERIOD,
        });
    BoardSite {
        repo,
        remote: "origin".into(),
        build: Some(build),
        launcher: Some(launcher),
        keeper,
    }
}
