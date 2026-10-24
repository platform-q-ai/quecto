//! The board's operations as the front ends' tests build them: the
//! application's assemblies ([`query_listings`]) behind the local port,
//! so a test reaches the board as a front end does (#2705 B6).
use std::sync::Arc;

use super::LocalBoardOperations;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings;
use crate::application::project_board::ports::{
    BoardClock, BoardListings, CoordinatorLaunch, ProjectBoardStore, PullRequestStates, RunLiveness,
};

pub use query_listings::{QueryListings, Swarms};

/// As [`query_listings::assembled`], behind the port.
pub fn assembled(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
) -> Result<LocalBoardOperations, ProjectBoardError> {
    query_listings::assembled(store, clock, listings, prs).map(LocalBoardOperations::new)
}

/// As [`query_listings::assembled_launching`], behind the port.
pub fn assembled_launching(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    liveness: Option<Arc<dyn RunLiveness>>,
) -> Result<LocalBoardOperations, ProjectBoardError> {
    query_listings::assembled_launching(store, clock, listings, prs, launcher, liveness)
        .map(LocalBoardOperations::new)
}

/// As [`query_listings::assembled_reporting`], behind the port.
pub fn assembled_reporting(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    swarms: Swarms,
) -> Result<LocalBoardOperations, ProjectBoardError> {
    query_listings::assembled_reporting(store, clock, listings, prs, swarms)
        .map(LocalBoardOperations::new)
}
