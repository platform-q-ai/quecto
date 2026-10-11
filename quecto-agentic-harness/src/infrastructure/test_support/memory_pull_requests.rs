//! An in-memory [`PullRequestStates`] for tests (#2476): pull requests are
//! set to a state one by one, any other is unknown, and the host can be
//! made unreachable.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::application::project_board::ports::{
    PullRequestState, PullRequestStates, PullRequestStatesError,
};

#[derive(Default)]
struct Host {
    states: BTreeMap<u64, PullRequestState>,
    failure: Option<String>,
    reads: usize,
}

/// Clones share one host.
#[derive(Clone, Default)]
pub struct MemoryPullRequests {
    host: Arc<Mutex<Host>>,
}

impl MemoryPullRequests {
    /// Pull request `pr` stands at `state` from now on.
    pub fn set(&self, pr: u64, state: PullRequestState) {
        self.lock().states.insert(pr, state);
    }

    /// Every read fails with `reason` until [`Self::recover`].
    pub fn fail(&self, reason: &str) {
        self.lock().failure = Some(reason.into());
    }

    pub fn recover(&self) {
        self.lock().failure = None;
    }

    /// The states read so far, failed reads included.
    pub fn reads(&self) -> usize {
        self.lock().reads
    }

    fn lock(&self) -> MutexGuard<'_, Host> {
        self.host.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PullRequestStates for MemoryPullRequests {
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError> {
        let mut host = self.lock();
        host.reads += 1;
        match &host.failure {
            Some(reason) => Err(PullRequestStatesError::Unavailable(reason.clone())),
            None => Ok(host
                .states
                .get(&pr)
                .copied()
                .unwrap_or(PullRequestState::Unknown)),
        }
    }
}
