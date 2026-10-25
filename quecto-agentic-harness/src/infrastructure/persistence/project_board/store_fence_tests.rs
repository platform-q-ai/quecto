use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;
#[test]
fn fence_invalid_capture_refuses_before_remote_or_lazy_writer() {
    let temp = tempfile::tempdir().unwrap();
    let vars = super::super::super::git::GitEnvironment::allowlisted([(
        "PATH".into(),
        std::env::var_os("PATH").unwrap(),
    )]);
    let reader = Git::reader_unasked(temp.path(), vars).unwrap();
    let publication = GitBoardFence::new(GitBoardStore::opened(reader, "origin"), || {
        panic!("invalid captured head must not ask writer identity")
    });
    let moved = BoardMoved::new(
        BoardId::of_hex("0123456789abcdef").unwrap(),
        vec!["https://primary.example.test".into()],
    )
    .unwrap();
    assert!(matches!(
        publication.publish_fence(&BoardHead("--malicious-ref".into()), &moved),
        Err(FenceError::Store(BoardStoreError::Unavailable(_)))
    ));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
