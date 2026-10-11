//! Contract for [`PullRequestStates`] (#2476): where each pull request a
//! task links stands, as the repository's host reports it. Checks take a
//! host arranged by the caller: they run over the in-memory fake and over
//! the `gh` adapter (#2477) with a fake `gh`.
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

/// A fake `gh` on a fresh `PATH` answering as [`check_reads_each_state`]
/// arranges, or failing every call.
fn fake_gh(
    answers: bool,
) -> (
    tempfile::TempDir,
    Vec<(std::ffi::OsString, std::ffi::OsString)>,
) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = match answers {
        true => {
            "#!/bin/sh\ncase \"$3\" in\n1) echo OPEN;;\n2) echo MERGED;;\n3) echo CLOSED;;\n\
                 *) echo 'GraphQL: Could not resolve to a PullRequest with the number of 4.' >&2; exit 1;;\nesac\n"
        }
        false => "#!/bin/sh\necho 'error connecting to api.github.com' >&2\nexit 1\n",
    };
    let gh = dir.path().join("gh");
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = dir.path().as_os_str().to_owned();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    (dir, vec![("PATH".into(), path)])
}

#[test]
fn the_gh_host_reads_each_state() {
    use quecto::infrastructure::persistence::project_board::gh_pull_requests::GhPullRequestStates;
    let (_dir, vars) = fake_gh(true);
    check_reads_each_state(&GhPullRequestStates::new("github.com/owner/repo", vars).unwrap());
}

#[test]
fn the_gh_host_reports_it_cannot_be_reached() {
    use quecto::infrastructure::persistence::project_board::gh_pull_requests::GhPullRequestStates;
    let (_dir, vars) = fake_gh(false);
    check_unreachable(&GhPullRequestStates::new("github.com/owner/repo", vars).unwrap());
}
