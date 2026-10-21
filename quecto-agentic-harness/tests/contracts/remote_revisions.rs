//! The remote revisions port's contract (#2666), run against the git
//! store: two clones of one bare remote. A revision is on the remote when
//! one of its branches or tags reaches the commit it names (by its full id
//! or a prefix of at least 12 hex digits), whichever clone asks and whether
//! or not that clone holds the commit; a commit made in a clone and never
//! pushed is not, nor is text that names no commit. Asking moves no
//! branch, tag or `FETCH_HEAD` of the asking clone.
use super::project_board_git_store::{Remote, sh};
use quecto::application::project_board::ports::{BoardStoreError, RemoteRevisions};

/// A new empty commit on clone `clone`'s branch, not pushed: its id.
fn commit_in(remote: &Remote, clone: usize, message: &str) -> String {
    let dir = remote.clone_dir(clone);
    sh(dir, &["commit", "-q", "--allow-empty", "-m", message], b"");
    sh(dir, &["rev-parse", "HEAD"], b"")
}

/// Clone `clone` pushes `commit` to `reference` on the remote.
fn push(remote: &Remote, clone: usize, commit: &str, reference: &str) {
    let spec = format!("{commit}:{reference}");
    sh(
        remote.clone_dir(clone),
        &["push", "-q", "origin", &spec],
        b"",
    );
}

/// Every ref and `FETCH_HEAD` of clone `clone`, as git lists them.
fn refs_of(remote: &Remote, clone: usize) -> (String, bool) {
    let dir = remote.clone_dir(clone);
    let refs = sh(
        dir,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
        b"",
    );
    (refs, dir.join(".git/FETCH_HEAD").exists())
}

fn asked(store: &dyn RemoteRevisions, revisions: &[&str]) -> Vec<String> {
    let revisions: Vec<String> = revisions.iter().map(|r| (*r).to_string()).collect();
    store.not_on_remote(&revisions).unwrap()
}

#[test]
fn a_revision_is_on_the_remote_only_when_a_branch_or_tag_there_reaches_it() {
    let remote = Remote::new();
    let (alice, bob) = (remote.client(0, "Alice"), remote.client(1, "Bob"));
    let pushed = commit_in(&remote, 0, "pushed");
    push(&remote, 0, &pushed, "refs/heads/feature");
    let unpushed = commit_in(&remote, 0, "made in the container, never pushed");
    let before = refs_of(&remote, 1);
    let asking = [
        pushed.as_str(),
        &pushed[..12],
        &pushed[..7],
        &pushed.to_uppercase(),
        unpushed.as_str(),
        &unpushed[..12],
        "working tree",
        "HEAD",
        "",
        "abc12",
    ];
    let not_there = [
        pushed[..7].to_string(),
        unpushed.clone(),
        unpushed[..12].to_string(),
        "working tree".into(),
        "HEAD".into(),
        String::new(),
        "abc12".into(),
    ];
    assert_eq!(
        asked(&*alice, &asking),
        not_there,
        "the clone that made them"
    );
    assert_eq!(asked(&*bob, &asking), not_there, "a clone that holds none");
    assert_eq!(
        refs_of(&remote, 1),
        before,
        "the asking clone's refs as they were"
    );
    // An ancestor of a branch's tip is reached; so is one a tag reaches.
    let older = commit_in(&remote, 1, "older");
    let tip = commit_in(&remote, 1, "tip");
    push(&remote, 1, &tip, "refs/heads/other");
    assert_eq!(asked(&*alice, &[&older, &tip]), Vec::<String>::new());
    let tagged = commit_in(&remote, 0, "tagged, on top of the unpushed one");
    sh(
        remote.clone_dir(0),
        &["tag", "-a", "-m", "v1", "v1", &tagged],
        b"",
    );
    push(&remote, 0, "refs/tags/v1", "refs/tags/v1");
    assert_eq!(
        asked(&*bob, &[&unpushed]),
        Vec::<String>::new(),
        "a tag reaches it now"
    );
}

#[test]
fn nothing_is_judged_while_the_remote_cannot_be_read() {
    let remote = Remote::new();
    let alice = remote.client(0, "Alice");
    let commit = commit_in(&remote, 0, "made here");
    let gone = remote.clone_dir(0).join("no-such-remote.git");
    sh(
        remote.clone_dir(0),
        &["remote", "set-url", "origin", gone.to_str().unwrap()],
        b"",
    );
    assert!(matches!(
        alice.not_on_remote(&[commit]),
        Err(BoardStoreError::Unavailable(_))
    ));
}

#[test]
fn text_that_names_no_commit_never_reads_the_remote() {
    let remote = Remote::new();
    let alice = remote.client(0, "Alice");
    let gone = remote.clone_dir(0).join("no-such-remote.git");
    sh(
        remote.clone_dir(0),
        &["remote", "set-url", "origin", gone.to_str().unwrap()],
        b"",
    );
    assert_eq!(
        asked(&*alice, &["working tree", "-v"]),
        ["working tree", "-v"],
        "no commit is named, so none is on the remote"
    );
}
