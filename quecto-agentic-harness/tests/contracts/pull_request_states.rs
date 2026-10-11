//! Contract for [`PullRequestStates`] (#2476): where each pull request a
//! task links stands, as the repository's host reports it. Checks take a
//! host arranged by the caller, so S2's adapter over the real host runs
//! them too; here they run over the in-memory fake.
use quecto::application::project_board::ports::{
    PullRequestState, PullRequestStates, PullRequestStatesError,
};
use quecto::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;

/// A host where pull request 1 is open, 2 merged and 3 closed without
/// merging, and 4 does not exist: each reads as it stands, every time.
pub fn check_reads_each_state(host: &dyn PullRequestStates) {
    for _ in 0..2 {
        assert_eq!(host.state(1), Ok(PullRequestState::Open));
        assert_eq!(host.state(2), Ok(PullRequestState::Merged));
        assert_eq!(host.state(3), Ok(PullRequestState::Closed));
        assert_eq!(host.state(4), Ok(PullRequestState::Unknown));
    }
}

/// A host that cannot be reached answers an error, never a state.
pub fn check_unreachable(host: &dyn PullRequestStates) {
    assert!(matches!(
        host.state(1),
        Err(PullRequestStatesError::Unavailable(_))
    ));
}

fn memory() -> MemoryPullRequests {
    let host = MemoryPullRequests::default();
    host.set(1, PullRequestState::Open);
    host.set(2, PullRequestState::Merged);
    host.set(3, PullRequestState::Closed);
    host
}

#[test]
fn the_memory_host_reads_each_state() {
    check_reads_each_state(&memory());
}

#[test]
fn the_memory_host_reports_it_cannot_be_reached() {
    let host = memory();
    host.fail("the host hung up");
    check_unreachable(&host);
}
