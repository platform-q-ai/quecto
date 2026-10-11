//! Boards the front ends' tests build their requests against.
use std::sync::Arc;

use serde_json::json;

use super::{BuildError, ProjectBoardEnvironment, ProjectBoardHandles, run};
use crate::application::project_board::operations::BoardOperations;
use crate::application::project_board::operations::query_listings::QueryListings;
use crate::application::project_board::ports::{
    BoardClock, BoardClockError, ProjectBoardStore, ReferenceStatus,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A clock reading one time, with or without an outside reference.
pub(crate) struct Fixed {
    pub(crate) at: Timestamp,
    pub(crate) referenced: bool,
}

impl BoardClock for Fixed {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        Ok(self.at.clone())
    }

    fn reference(&self) -> Option<Timestamp> {
        self.referenced.then(|| self.at.clone())
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.referenced {
            true => ReferenceStatus::Referenced,
            false => ReferenceStatus::NoReference("no outside time in this test".into()),
        }
    }

    fn pause(&self, _: usize) {}
}

/// The remote [`memory_board`] has no committer for.
pub(crate) const NO_COMMITTER: &str = "no-committer";

/// A board with one ready task `a`, in memory, built afresh each request
/// at one fixed, referenced time (so two builds answer alike); a write on
/// the [`NO_COMMITTER`] remote has no committer.
pub(crate) fn memory_board(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    if environment.remote == NO_COMMITTER && environment.access == super::Access::Write {
        return Err(BuildError::NoCommitter(
            "user.name is not configured".into(),
        ));
    }
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = BoardOperations::new(
        Arc::new(store.client()),
        Arc::new(Fixed {
            at: Timestamp::parse("2026-10-10T10:00:00Z").unwrap(),
            referenced: true,
        }),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err("no pull request host in this test".into()),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    let created = run(
        &operations,
        &json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
    );
    assert_eq!(created["ok"], json!(true), "{created}");
    Ok(ProjectBoardHandles { operations })
}

/// A board that cannot be built; its reason carries a credential.
pub(crate) fn unbuildable(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    Err(BuildError::Unavailable(
        "https://user:hunter2@example.com/repo is unreachable".into(),
    ))
}

/// A board a request refused while being read must never build.
pub(crate) fn never_built(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    panic!("a request refused while being read builds nothing")
}
