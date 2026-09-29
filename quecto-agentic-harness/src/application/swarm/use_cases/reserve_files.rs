//! `Tasks.reserve(task_id, token, paths)` (#2275): the owner of a claim
//! reserves files in the shared checkout, all or none.
use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::owned;
use crate::application::swarm::dto::{NewReservation, Reservation, ReserveFilesRequest};
use crate::application::swarm::ports::{BoardRepository, CheckoutPaths, Clock, IdSource};
use crate::domain::swarm::{Access, BoardError, bounded};

/// The most paths one call reserves.
const PATHS_PER_CALL: usize = 100;
/// The most bytes one path may take.
const PATH_MAX_BYTES: usize = 4096;
/// The most reservations the board holds.
const BOARD_CAPACITY: i64 = 1000;

/// Before the operation gate, the paths must be a list of 1 to 100, and
/// each, in order, a bounded text the checkout normalises (Python's
/// `resolve()` relative to the checkout). Through the gate (a running
/// run), only the claim's owner with its current token reserves; the
/// board must have room for the distinct paths, and none may be reserved
/// already (the first reserved one in sort order is named). An ownership
/// token is then drawn, a row is inserted per path in sorted order, and
/// the event `files_reserved{task,paths}` names the task id as given and
/// the paths sorted.
pub struct ReserveFiles {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
    checkout: Arc<dyn CheckoutPaths>,
}

impl ReserveFiles {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
        checkout: Arc<dyn CheckoutPaths>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
            checkout,
        }
    }

    /// # Errors
    /// The paths' count, a path's bound or its escape, an authorisation or
    /// budget refusal, `unknown task`, `stale or unowned claim`, a full
    /// board, a path already reserved, or the store's.
    pub fn execute(&self, request: ReserveFilesRequest) -> Result<Reservation, BoardError> {
        let paths = self.normalized(&request.paths)?;
        let actor = request.actor.as_str();
        let running = Access {
            active: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            running,
            |transaction, _| {
                owned(transaction, &request.task_id, &request.token, actor)?;
                let wanted = i64::try_from(paths.len()).unwrap_or(i64::MAX);
                if transaction.file_count()?.saturating_add(wanted) > BOARD_CAPACITY {
                    return Err(BoardError::new(
                        "file reservation board full (1000); release settled work",
                    ));
                }
                for path in &paths {
                    if transaction.file_reserved(path)? {
                        return Err(BoardError::new(format!(
                            "file already reserved: {path}; acquire the entire set or release and retry"
                        )));
                    }
                }
                let reservation = NewReservation {
                    task: request.task_id.clone(),
                    owner: actor.to_owned(),
                    claim: request.token.clone(),
                    token: self.ids.hex32(),
                    paths: paths.iter().cloned().collect(),
                };
                transaction.insert_files(&reservation)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "files_reserved",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("paths", Value::from(reservation.paths.clone())),
                    ]),
                )?;
                Ok(Reservation {
                    token: reservation.token,
                    paths: reservation.paths,
                })
            },
        )
    }

    /// The distinct normalised paths, sorted: each given path bounded and
    /// then normalised, in order, so the first bad one is refused.
    fn normalized(&self, paths: &Value) -> Result<BTreeSet<String>, BoardError> {
        let given = match paths {
            Value::Array(given) if (1..=PATHS_PER_CALL).contains(&given.len()) => given,
            _ => return Err(BoardError::new("reserve 1 through 100 paths together")),
        };
        let mut normalized = BTreeSet::new();
        for path in given {
            let path = bounded(path, "path", PATH_MAX_BYTES)?;
            let inside = self.checkout.normalize(path)?;
            debug_assert!(
                !inside.is_empty() && !inside.starts_with('/'),
                "the checkout answers a relative path: {inside}"
            );
            normalized.insert(inside);
        }
        debug_assert!(!normalized.is_empty(), "at least one path was given");
        Ok(normalized)
    }
}

#[cfg(test)]
#[path = "reserve_files_tests.rs"]
mod tests;
