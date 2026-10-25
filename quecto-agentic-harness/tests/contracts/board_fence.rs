//! Shared BoardFence publication contract, exercised with owned bare remotes.
use quecto::application::project_board::ports::{
    BoardHead,
    fence::{FenceError, FenceReport},
};

pub fn published(result: Result<FenceReport, FenceError>, before: &BoardHead) -> BoardHead {
    assert!(
        matches!(&result, Ok(FenceReport::Published { previous, head }) if previous == before && head != before),
        "one newly published caller commit with captured parent required: {result:?}"
    );
    match result.unwrap() {
        FenceReport::Published { head, .. } => head,
        other => panic!("published, got {other:?}"),
    }
}
