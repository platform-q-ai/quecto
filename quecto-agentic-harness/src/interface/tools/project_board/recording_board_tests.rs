//! #2663: every record of a call names the board it reached.
use super::*;

/// An adapter that breaks the port's contract: it gives back no facts,
/// even of a noted request.
struct Factless;

impl BoardOperations for Factless {
    fn execute(&self, _: ReadRequest<'_>, _: ListSummaries, _: OpNoting) -> BoardExecution {
        BoardExecution {
            result: Err(ProjectBoardError::Contended { attempts: 1 }),
            facts: None,
        }
    }

    fn left_runs(
        &self,
    ) -> Result<Vec<crate::application::project_board::dto::RunEnd>, ProjectBoardError> {
        Ok(vec![])
    }
}

/// #2705 B6 review: a noted request that gives back no facts is flagged,
/// never filled in as a request that saw nothing.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a noted board request gave back no facts")]
fn a_recorded_call_flags_an_adapter_that_gives_back_no_facts() {
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    let read = ReadRequest {
        request: &BoardRequest::ListReady,
        text: r#"{"op": "list_ready"}"#,
        front_end: FrontEnd::Tool,
    };
    let _ = call.executed(&Factless, read, ListSummaries::Omitted);
}

/// #2663 review round 1: every record of a call names the board it
/// reached: its `board_op`, its start's `board_start`, and each fold-back
/// it noted (a start's undo's, an end's).
#[test]
fn every_record_of_a_call_names_its_board() {
    use crate::application::project_board::dto::{FoldBackFacts, FoldedBack};
    use crate::application::project_board::start::NotedFoldBack;
    use crate::domain::project_board::value_objects::slug::Slug;
    let records = Arc::new(Records::default());
    let board = BoardId::of_hex("0123456789abcdef").unwrap();
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    call.on_board(Some(&board));
    let task = Slug::parse("a").unwrap();
    let folded = || NotedFoldBack {
        task: task.clone(),
        run: 1,
        result: Ok(FoldedBack::Ended(1)),
        facts: FoldBackFacts::default(),
        elapsed: std::time::Duration::ZERO,
    };
    {
        let open = call.open.as_mut().unwrap();
        open.record.op = OpKind::Start;
        open.record.task = Some(TaskRef::of(&task));
        open.start = Some(StartFacts {
            fold_back: Some(folded()),
            ..StartFacts::default()
        });
        open.fold_back = Some(folded());
    }
    call.finish(&json!({"ok": true}), false);
    let op = serde_json::to_value(&records.taken()[0]).unwrap();
    assert_eq!(op["board"], json!("0123456789abcdef"), "{op}");
    let start = serde_json::to_value(&records.starts()[0]).unwrap();
    assert_eq!(start["board"], json!("0123456789abcdef"), "{start}");
    let fold_backs = records.fold_backs();
    assert_eq!(fold_backs.len(), 2, "the undo's and the end's");
    for fold_back in fold_backs {
        let line = serde_json::to_value(&fold_back).unwrap();
        assert_eq!(line["board"], json!("0123456789abcdef"), "{line}");
    }
}
