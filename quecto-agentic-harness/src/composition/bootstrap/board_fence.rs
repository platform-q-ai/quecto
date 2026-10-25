//! Offline fence construction; never builds the token/listener graph.
use std::sync::Arc;

pub fn build_fence_from_ports(
    publication: Arc<dyn crate::application::project_board::ports::fence::BoardFence>,
) -> crate::application::project_board::use_cases::FenceBoard {
    crate::application::project_board::use_cases::FenceBoard::new(publication)
}

/// Capture with an unasked reader; resolve the caller's existing board identity
/// convention only if publication actually needs a new commit.
pub fn build_fence_cli(
    environment: crate::interface::cli::board_server::fence::FenceCliEnvironment,
) -> Result<
    crate::interface::cli::board_server::fence::FenceCliHandles,
    crate::application::project_board::ports::fence::FenceError,
> {
    use crate::{
        application::project_board::ports::{BoardHead, BoardStoreError, fence::FenceError},
        infrastructure::persistence::project_board::{
            git::{Git, GitEnvironment},
            store::{GitBoardFence, GitBoardStore},
        },
        interface::cli::board_server::fence::FenceCliHandles,
    };
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let reader = Git::reader_unasked(&environment.repo, vars.clone())
        .map_err(|error| FenceError::Unavailable(error.to_string()))?;
    let store = GitBoardStore::new(reader, &environment.remote)
        .map_err(|error| FenceError::Unavailable(format!("{error:?}")))?;
    let captured = BoardHead(
        store
            .head()
            .map_err(FenceError::Store)?
            .ok_or(FenceError::Store(BoardStoreError::NoBoard))?,
    );
    let publication = GitBoardFence::new(store, move || {
        if let Some(warning) = vars.command_scope_warning() {
            use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
            eprintln!(
                "{}",
                redact_secrets(&redact_url_userinfo(&format!(
                    "warning: project board: {warning}"
                )))
            );
        }
        let identity = match super::project_board::board_committer(&environment.vars) {
            Some(identity) => identity.map_err(BoardStoreError::Unavailable)?,
            None => Git::configured_committer(&environment.repo, &vars)
                .map_err(|error| BoardStoreError::Unavailable(error.to_string()))?,
        };
        Git::new(&environment.repo, vars.clone(), identity)
            .map_err(|error| BoardStoreError::Unavailable(error.to_string()))
    });
    Ok(FenceCliHandles {
        captured,
        fence: build_fence_from_ports(Arc::new(publication)),
    })
}
