//! Where a succeeded board run's work went, read as its owner's kill
//! tears its box down (#2666): the commit the run completed at (its
//! `outcome_reason`, [`COMPLETED_AT`] then the revision) asked of the
//! board's remote, through the project board's own [`RemoteRevisions`]
//! port, on the repository the session's board tool works on (one
//! authority: [`register_board_repository`], as its board site is built).
//! Only a board run's coordinator ([`super::is_board_label`]) is judged.
//! A completion that names no commit the push rule checks
//! ([`commit_token`], [`checks_commit`]), a board, repository or remote
//! that cannot be read, is `Unknown`, which keeps the box (fail safe).
//! The repository's remote is opened lazily, on the first such judgement
//! (again on the next, when it could not be),
//! and read within a short bound. Blocking: the remote is read inline, as
//! the teardown's scripts run.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::PortFuture;
use crate::application::environments::ports::hosted_swarm::CompletionDeliveryCheck;
use crate::application::project_board::ports::{BoardStoreError, RemoteRevisions};
use crate::domain::environments::services::environment_retention::CompletionDelivery;
use crate::domain::project_board::value_objects::commit_token::{checks_commit, commit_token};
use crate::domain::swarm::value_objects::lifecycle::COMPLETED_AT;

/// The repository each session's board tool works on, by session.
fn repositories() -> &'static Mutex<BTreeMap<String, PathBuf>> {
    static REPOSITORIES: OnceLock<Mutex<BTreeMap<String, PathBuf>>> = OnceLock::new();
    REPOSITORIES.get_or_init(Mutex::default)
}

/// `session`'s board tool works on `repo`: what its teardown judges a
/// succeeded board run's completion against.
pub fn register_board_repository(session: &str, repo: &Path) {
    repositories()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session.to_owned(), repo.to_path_buf());
}

/// A repository's board remote, opened, or why it could not be.
type Opened = Result<Arc<dyn RemoteRevisions>, String>;

/// How a repository's board remote is opened (composition's).
pub type OpenRemote = fn(&Path) -> Opened;

/// Judges completions against the board's remote, or says why it cannot.
pub struct RemoteCompletionDelivery {
    open: OpenRemote,
    /// Each repository's remote, once opened: one that could not be is
    /// tried again on its next judgement.
    opened: Mutex<BTreeMap<PathBuf, Arc<dyn RemoteRevisions>>>,
}

impl RemoteCompletionDelivery {
    /// Opening each repository's remote with `open`, when first needed.
    pub fn new(open: OpenRemote) -> Self {
        Self {
            open,
            opened: Mutex::default(),
        }
    }

    /// The remote of `session`'s board repository, opened once.
    fn remote(&self, session: &str) -> Opened {
        let repo = repositories()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session)
            .cloned()
            .ok_or_else(|| "no board repository is known for this session".to_owned())?;
        let held = self
            .opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&repo)
            .cloned();
        if let Some(remote) = held {
            return Ok(remote);
        }
        // Opened with no lock held: git runs here.
        let remote = (self.open)(&repo)?;
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(opened.entry(repo).or_insert(remote)))
    }

    fn judged(
        &self,
        session: &str,
        label: &str,
        board: Option<&SwarmBoardSnapshot>,
    ) -> CompletionDelivery {
        match super::is_board_label(label) {
            true => {}
            false => return CompletionDelivery::NotJudged,
        }
        let unknown = |why: String| CompletionDelivery::Unknown { why };
        let Some(board) = board else {
            return unknown("its board could not be read".into());
        };
        let board = board.clone().into_value();
        let reason = board
            .get("run")
            .and_then(|run| run.get("outcome_reason"))
            .and_then(serde_json::Value::as_str);
        let Some(revision) = reason.and_then(|reason| reason.strip_prefix(COMPLETED_AT)) else {
            return unknown("it recorded no completion revision".into());
        };
        let Some(token) = commit_token(revision) else {
            return unknown(format!("its completion {revision:?} names no one commit"));
        };
        let remote = match self.remote(session) {
            Ok(remote) => remote,
            Err(why) => return unknown(why),
        };
        let id_len = match remote.commit_id_hex_len() {
            Ok(id_len) => id_len,
            Err(error) => return unknown(told(&error)),
        };
        match checks_commit(&token, id_len) {
            true => {}
            false => {
                return unknown(format!(
                    "its completion commit {token} is too short or too long to check"
                ));
            }
        }
        match remote.not_on_remote(std::slice::from_ref(&token)) {
            Ok(missing) => match missing.as_slice() {
                [] => CompletionDelivery::OnRemote,
                [_, ..] => CompletionDelivery::NotOnRemote { revision: token },
            },
            Err(error) => unknown(told(&error)),
        }
    }
}

/// What a store error says.
fn told(error: &BoardStoreError) -> String {
    match error {
        BoardStoreError::NoBoard => "the repository has no board".into(),
        BoardStoreError::Clash(why)
        | BoardStoreError::InvalidWrite(why)
        | BoardStoreError::WriteTooLarge(why)
        | BoardStoreError::Unavailable(why)
        | BoardStoreError::TooLarge(why)
        | BoardStoreError::FolderSetAside(why) => why.clone(),
    }
}

impl CompletionDeliveryCheck for RemoteCompletionDelivery {
    fn delivery<'a>(
        &'a self,
        session: &'a str,
        label: &'a str,
        board: Option<&'a SwarmBoardSnapshot>,
    ) -> PortFuture<'a, CompletionDelivery> {
        let judged = self.judged(session, label, board);
        Box::pin(async move { judged })
    }
}
