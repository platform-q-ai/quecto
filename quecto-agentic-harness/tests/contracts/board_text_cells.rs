//! Contract for [`BoardTextCells`] (S4): the markdown adapter holds a line
//! exactly when the git store's codec writes it in a run's text cell, so a
//! fold-back that keeps only held lines is never refused for one.
use quecto::application::project_board::ports::BoardTextCells;
use quecto::domain::project_board::entities::run::{Run, RunFields, RunItem};
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use quecto::infrastructure::persistence::project_board::artifacts::encode;
use quecto::infrastructure::persistence::project_board::artifacts::run::RunCodec;
use quecto::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;

/// A folded run whose one task item holds `line` as its owner and its
/// evidence.
fn run_holding(line: &str) -> Run {
    let at = Timestamp::parse("2026-10-10T10:00:00Z").unwrap();
    Run::new(RunFields {
        task: Slug::parse("board-store").unwrap(),
        number: 1,
        plan_version: 1,
        started: at.clone(),
        ended: Some(at),
        outcome: Some(RunOutcome::Succeeded),
        cost: Some(1),
        items: vec![RunItem {
            id: Slug::parse("api").unwrap(),
            state: ItemState::Done,
            owner: Some(line.into()),
            evidence: vec![line.into()],
        }],
        environment: None,
    })
    .unwrap()
}

#[test]
fn a_line_is_held_exactly_when_the_run_codec_writes_it() {
    let cells = MarkdownTextCells;
    for line in [
        "src/api.rs at 4f2a9c1",
        "[link](https://github.com/o/r/pull/7) at 4f2a9c1",
        "worker-1",
        "costs $5 at 4f2a9c1",
        "ada@example.com at 4f2a9c1",
        "https://github.com/o/r/pull/7 at 4f2a9c1",
        "http://example.com at 4f2a9c1",
        "a | b at `x`",
        "*",
    ] {
        let written = encode::<RunCodec>(&run_holding(line));
        assert_eq!(cells.holds(line), written.is_ok(), "{line:?}: {written:?}");
    }
}
