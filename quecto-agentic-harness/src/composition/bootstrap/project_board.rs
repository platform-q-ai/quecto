//! The project board's operations for one run (#2477): the git store on
//! the repository's board remote, writing as its configured committer (a
//! read needs none); the SQLite index of the board in the repository's
//! git directory; the clock referenced to the remote's HTTPS `Date`
//! (#2491); and pull request states read with `gh` when the remote is on
//! GitHub or the configured enterprise host. Built for `quecto board`,
//! which runs on the calling thread (it blocks).
use std::sync::Arc;

use crate::application::project_board::operations::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, PullRequestStates, PullRequestStatesError,
};
use crate::application::project_board::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    RunBoardOperation, UpdateDraftTask,
};
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::persistence::project_board::gh_pull_requests::{
    GhPullRequestStates, github_repository,
};
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment, GitError};
use crate::infrastructure::persistence::project_board::index::BoardIndex;
use crate::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
use crate::infrastructure::persistence::project_board::store::GitBoardStore;
use crate::infrastructure::time::remote_time::ReferencedBoardClock;
use crate::interface::cli::board::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// # Errors
/// A write with no committer configured, or one git would not record as
/// given; no git, a repository git cannot run in, no such remote, an
/// index whose directory cannot be named (each with its cause). A write
/// warns on standard error of a malformed git command-scope set, which it
/// drops, or of one naming half an identity.
pub fn build_project_board_handles(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    build_with_warnings(environment, &mut |line| eprintln!("{line}"))
}

/// [`build_project_board_handles`], telling `warn` each line to warn of: a
/// write's malformed git command-scope set, dropped, and why, or one
/// naming half an identity.
fn build_with_warnings(
    environment: &ProjectBoardEnvironment,
    warn: &mut dyn FnMut(&str),
) -> Result<ProjectBoardHandles, BuildError> {
    let unavailable = |why: String| BuildError::Unavailable(why);
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    // Only a write names a committer, so only a write is told what the
    // set naming one warns of. A warning names variables and the
    // identity's keys alone; it is redacted all the same, in case one ever
    // quotes more.
    match (environment.access, vars.command_scope_warning()) {
        (Access::Write, Some(warning)) => warn(&redact_secrets(&redact_url_userinfo(&format!(
            "warning: project board: {warning}"
        )))),
        (Access::Write | Access::Read, None) | (Access::Read, Some(_)) => {}
    }
    let git = match environment.access {
        Access::Write => Git::configured_committer(&environment.repo, &vars)
            .and_then(|committer| Git::new(&environment.repo, vars, committer))
            .map_err(write_refusal)?,
        Access::Read => Git::reader(&environment.repo, vars)
            .map_err(|error| unavailable(format!("git cannot run here: {error}")))?,
    };
    let store = Arc::new(
        GitBoardStore::new(git, &environment.remote)
            .map_err(|error| unavailable(store_reason(&error)))?
            .with_cancel(environment.cancelled.clone()),
    );
    let url = store
        .remote_url()
        .map_err(|error| unavailable(store_reason(&error)))?;
    let clock: Arc<dyn BoardClock> = Arc::new(ReferencedBoardClock::for_remote(&url));
    let index = BoardIndex::of(&store).map_err(|error| {
        unavailable(format!(
            "the board index's directory cannot be named: {error}"
        ))
    })?;
    let listings = Arc::new(IndexedBoardListings::new(
        index,
        Arc::clone(&store) as _,
        Arc::clone(&store) as _,
    ));
    let prs = pull_request_states(&url, &environment.vars);
    let store: Arc<dyn ProjectBoardStore> = store;
    let built = |error: crate::application::project_board::dto::ProjectBoardError| {
        unavailable(error.to_string())
    };
    let complete = match prs {
        Ok(prs) => Ok(
            CompleteMergedTask::new(Arc::clone(&store), Arc::clone(&clock), prs).map_err(built)?,
        ),
        Err(why) => Err(why),
    };
    let parts = BoardOperationParts {
        bootstrap: BootstrapProjectBoard::new(Arc::clone(&store)),
        create: CreateBoardTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        update: UpdateDraftTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        change: ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        complete,
        clock,
        listings,
    };
    Ok(ProjectBoardHandles {
        operations: RunBoardOperation::new(parts),
    })
}

/// Pull request states of the remote at `url`, read with `gh` when it is
/// on GitHub or the configured enterprise host (`GH_HOST` in `vars`); or
/// why none are.
fn pull_request_states(
    url: &str,
    vars: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Arc<dyn PullRequestStates>, NoPullRequestHost> {
    let configured_host = vars
        .iter()
        .find(|(key, _)| key == "GH_HOST")
        .and_then(|(_, value)| value.to_str());
    match github_repository(url, configured_host) {
        Some(repo) => GhPullRequestStates::new(&repo, vars.iter().cloned())
            .map(|states| Arc::new(states) as _)
            .map_err(|error| match error {
                PullRequestStatesError::Unavailable(why) => NoPullRequestHost::Unreachable(why),
            }),
        None => Err(NoPullRequestHost::NotAPullRequestHost),
    }
}

/// Why a write's store cannot be built: an identity problem (none
/// configured, or one git would not record as given) is the committer's;
/// anything else (no git, a timeout, a repository git cannot run in) is
/// the store's, with its cause.
fn write_refusal(error: GitError) -> BuildError {
    match error {
        GitError::Identity(why) => BuildError::NoCommitter(why),
        unavailable @ (GitError::NotFound
        | GitError::Spawn(_)
        | GitError::TimedOut { .. }
        | GitError::TooMuchOutput { .. }
        | GitError::Failed { .. }) => {
            BuildError::Unavailable(format!("git cannot run here: {unavailable}"))
        }
    }
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
