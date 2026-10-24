//! The remote revisions port's contract, run against git (#2705 B1):
//! [`GitRemoteRevisions`], its own adapter apart from the board store,
//! opened on a clone's `origin` (as a teardown opens it) and on the remote
//! a board store was opened on (as the board's composition opens it). It
//! opens only on a remote the clone configures.
use super::project_board_git_store::Remote;
use super::remote_revisions::{
    asked, check_nothing_is_judged_while_the_remote_cannot_be_read,
    check_on_the_remote_only_when_a_branch_or_tag_there_reaches_it,
    check_text_that_names_no_commit_never_reads_the_remote, commit_in, push,
};
use quecto::application::project_board::ports::RemoteRevisions;
use quecto::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use quecto::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};

/// Clone `clone`'s `origin`, opened on its own.
fn opened(remote: &Remote, clone: usize, name: &str) -> Box<dyn RemoteRevisions> {
    Box::new(GitRemoteRevisions::new(remote.git(clone, name), "origin").unwrap())
}

/// The remote of a board store opened on clone `clone`'s `origin`.
fn of_store(remote: &Remote, clone: usize, name: &str) -> Box<dyn RemoteRevisions> {
    let store = GitBoardStore::new(remote.git(clone, name), "origin").unwrap();
    Box::new(GitRemoteRevisions::of_store(&store))
}

#[test]
fn a_revision_is_on_the_remote_only_when_a_branch_or_tag_there_reaches_it() {
    check_on_the_remote_only_when_a_branch_or_tag_there_reaches_it(opened);
    check_on_the_remote_only_when_a_branch_or_tag_there_reaches_it(of_store);
}

#[test]
fn nothing_is_judged_while_the_remote_cannot_be_read() {
    check_nothing_is_judged_while_the_remote_cannot_be_read(opened);
    check_nothing_is_judged_while_the_remote_cannot_be_read(of_store);
}

#[test]
fn text_that_names_no_commit_never_reads_the_remote() {
    check_text_that_names_no_commit_never_reads_the_remote(opened);
    check_text_that_names_no_commit_never_reads_the_remote(of_store);
}

#[test]
fn it_opens_only_on_a_remote_the_clone_configures() {
    let remote = Remote::new();
    for name in ["upstream", "../remote.git", "-v", ""] {
        assert!(
            matches!(
                GitRemoteRevisions::new(remote.git(0, "Alice"), name),
                Err(StoreOpenError::NoSuchRemote(_))
            ),
            "{name:?}"
        );
    }
    let pushed = commit_in(&remote, 0, "pushed");
    push(&remote, 0, &pushed, "refs/heads/feature");
    assert_eq!(
        asked(&*opened(&remote, 0, "Alice"), &[&pushed]),
        Vec::<String>::new()
    );
}
