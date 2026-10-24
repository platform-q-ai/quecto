//! The agent `board` tool on a board moved to a board server (#2705 B13):
//! every op is refused as `board_moved`, naming the URLs, and nothing is
//! written.
use serde_json::{Value, json};
use std::sync::Arc;

use super::tests::{site, tool};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::ports::{BoardStoreError, ProjectBoardStore};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::tools::ports::Tool;
use crate::composition::bootstrap::project_board_operations::fixtures::{QueryListings, assembled};
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::site::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// A board moved to a board server at one URL.
fn moved_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let id = BoardId::of_hex("0123456789abcdef").unwrap();
    let moved = BoardMoved::new(id, vec!["https://board.example:7443".into()]).unwrap();
    store.fail_every_call(BoardStoreError::Moved(moved));
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

#[tokio::test]
async fn every_op_on_a_moved_board_answers_board_moved() {
    let tool = tool(site(Some(moved_board)));
    for request in [
        r#"{"op":"show","task":"a"}"#,
        r#"{"op":"create","task":"a","title":"A","status":"ready"}"#,
        r#"{"op":"list"}"#,
        r#"{"op":"list_ready"}"#,
        r#"{"op":"clear_set_aside","path":"tasks/a.md","object":"0000000000000000000000000000000000000000"}"#,
    ] {
        let result = tool.execute(request).await.unwrap();
        let answer: Value = serde_json::from_str(&result.content).unwrap();
        assert!(result.is_error, "{answer}");
        assert_eq!(answer["error"]["code"], json!("board_moved"), "{answer}");
        assert_eq!(
            answer["error"]["urls"],
            json!(["https://board.example:7443"])
        );
        let message = answer["error"]["message"].as_str().unwrap();
        assert!(message.contains("https://board.example:7443"), "{message}");
    }
}
