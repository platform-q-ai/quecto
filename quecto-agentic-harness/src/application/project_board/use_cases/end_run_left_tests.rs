//! #2674 round 1: a one-shot exit while its swarm ran or was paused kept
//! no board, so the orphaned run's stop recorded every item as todo. The
//! board the run had reached as its owner ended its coordinator is kept,
//! and a stop, failure or loss records from it; a success never does.
use super::*;

/// Run 1 of `board-store`'s swarm as far as it got: kept as left, never
/// as ended.
struct LeftOnly(std::sync::Mutex<Option<SwarmRunView>>);

impl SwarmRunReports for LeftOnly {
    fn ended_run(&self, _: &Slug, _: u32) -> Option<SwarmRunView> {
        None
    }

    fn left_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView> {
        match (task.as_str(), run) {
            ("board-store", 1) => self.0.lock().unwrap().clone(),
            _ => None,
        }
    }

    fn recorded(&self, _: &Slug, _: u32) {
        assert!(self.0.lock().unwrap().take().is_some(), "recorded once");
    }
}

fn ender_left(store: &MemoryBoardStore) -> EndRun {
    let states = [
        SwarmItemState::Completed,
        SwarmItemState::Claimed,
        SwarmItemState::Ready,
    ];
    let left = swarm(SwarmEnd::Cancelled, states);
    EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        Remote::holding_all(),
        None,
        Some(Arc::new(LeftOnly(std::sync::Mutex::new(Some(left))))),
    )
    .unwrap()
}

/// #2705 T5 review round 2: a board kept while its run still ran or was
/// paused is read as cancelled for its items alone; how the stop stopped
/// is the caller's `stop`, so blocked keeps the task with its holder, and
/// none given releases it.
#[test]
fn a_left_board_never_overrides_the_callers_stop() {
    use crate::domain::project_board::services::run_outcomes::StopKind;
    for (stop, want) in [
        (Some(StopKind::Blocked), TaskStatus::Blocked),
        (None, TaskStatus::Ready),
    ] {
        let (store, _) = stranded();
        let request = RunEnd {
            stop,
            ..ending(RunEndOutcome::Stopped)
        };
        let ended = ender_left(&store).execute(&request).unwrap();
        assert_eq!(ended.report, RunReport::Kept, "{stop:?}");
        assert_eq!(ended.status, want, "{stop:?}");
        assert_eq!(store.read().unwrap().tasks[0].fields().status, want);
    }
}

#[test]
fn a_stop_records_the_board_its_run_had_reached() {
    for outcome in [
        RunEndOutcome::Stopped,
        RunEndOutcome::Failed,
        RunEndOutcome::Lost,
    ] {
        let (store, _) = stranded();
        let ended = ender_left(&store).execute(&ending(outcome)).unwrap();
        assert_eq!(ended.report, RunReport::Kept, "{outcome:?}");
        let states: Vec<ItemState> = ended
            .run
            .fields()
            .items
            .iter()
            .map(|item| item.state)
            .collect();
        assert_eq!(states, [ItemState::Done, ItemState::Doing, ItemState::Todo]);
        assert_eq!(
            ended.run.fields().items[0].evidence,
            ["docs/api.md at abc123"],
            "{outcome:?}"
        );
    }
}

#[test]
fn a_success_is_never_recorded_from_a_board_its_run_had_only_reached() {
    let (store, _) = stranded();
    assert_eq!(
        ender_left(&store).execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Rule(BoardRuleError::ItemsNotDone {
            run: 1,
            items: vec![slug("api"), slug("store"), slug("docs")],
            report: SwarmReport::NoneKept,
        }))
    );
}
