//! The project board's operations for one run (#2477): the git store on
//! the repository's board remote, writing as its configured committer (a
//! read needs none); the SQLite index of the board in the repository's
//! git directory; the clock referenced to the remote's HTTPS `Date`
//! (#2491); and pull request states read with `gh` when the remote is on
//! GitHub or the configured enterprise host. Built per request for
//! `quecto board`, on the calling thread, and for the agent `board` tool,
//! on Tokio's blocking pool: it blocks.
use std::sync::Arc;

use crate::application::project_board::operations::BoardOperations;
use crate::application::project_board::ports::{
    BoardStoreError, PullRequestStates, PullRequestStatesError,
};
use crate::infrastructure::persistence::project_board::gh_pull_requests::{
    GhPullRequestStates, github_repository,
};
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::index::BoardIndex;
use crate::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
use crate::infrastructure::persistence::project_board::store::GitBoardStore;
use crate::infrastructure::time::remote_time::ReferencedBoardClock;
use crate::interface::tools::project_board::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// # Errors
/// A write with no committer configured; no git, no such remote, or an
/// index whose directory cannot be named.
pub fn build_project_board_handles(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    let unavailable = |why: String| BuildError::Unavailable(why);
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let git = match environment.access {
        Access::Write => {
            let committer = Git::configured_committer(&environment.repo, &vars)
                .map_err(|error| BuildError::NoCommitter(error.to_string()))?;
            Git::new(&environment.repo, vars, committer)
        }
        Access::Read => Git::reader(&environment.repo, vars),
    }
    .map_err(|error| unavailable(format!("git cannot run here: {error}")))?;
    let store = Arc::new(
        GitBoardStore::new(git, &environment.remote)
            .map_err(|error| unavailable(store_reason(&error)))?,
    );
    let url = store
        .remote_url()
        .map_err(|error| unavailable(store_reason(&error)))?;
    let clock = Arc::new(ReferencedBoardClock::for_remote(&url));
    let index = BoardIndex::of(&store)
        .map_err(|_| unavailable("the board index's directory cannot be named".into()))?;
    let listings = Arc::new(IndexedBoardListings::new(
        index,
        Arc::clone(&store) as _,
        Arc::clone(&store) as _,
    ));
    let configured_host = environment
        .vars
        .iter()
        .find(|(key, _)| key == "GH_HOST")
        .and_then(|(_, value)| value.to_str());
    let prs: Result<Arc<dyn PullRequestStates>, String> =
        match github_repository(&url, configured_host) {
            Some(repo) => GhPullRequestStates::new(&repo, environment.vars.iter().cloned())
                .map(|states| Arc::new(states) as _)
                .map_err(|error| match error {
                    PullRequestStatesError::Unavailable(why) => why,
                }),
            None => Err(
                "the board remote is not on GitHub or the configured host, so no pull request \
                 states are read"
                    .into(),
            ),
        };
    let operations = BoardOperations::new(store, clock, listings, prs)
        .map_err(|error| unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

/// A store's refusal in its own words.
fn store_reason(error: &BoardStoreError) -> String {
    match error {
        BoardStoreError::NoBoard => "the project has no board yet".into(),
        BoardStoreError::Clash(why)
        | BoardStoreError::InvalidWrite(why)
        | BoardStoreError::Unavailable(why) => why.clone(),
    }
}

#[cfg(test)]
#[path = "project_board_tests.rs"]
mod tests;
