//! A board server's runs as its client starts and ends them (epic #2705
//! B8): the [`BoardOperations`] port over the remote adapter, as
//! [`super::project_board_operations::LocalBoardOperations`] is over the
//! local use case.
//!
//! - **start**: begun on the server, whose answer names what to launch;
//!   the coordinator launched here ([`LaunchBegunRun`]), the start undone
//!   on the server when the launch fails. The answer is the server's with
//!   the launched coordinator in place of what it was launched with; a
//!   refusal of the begin is the server's as it came.
//! - **end_run**: sent with what this session saw of the run
//!   ([`RunFacts`]); a swarm board it kept is dropped once the server
//!   recorded it.
//! - **left runs**: each run this session's starts launched that the
//!   server shows still running.
//!
//! Every other request is the remote adapter's as it is.
use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::application::project_board::dto::{
    BoardAnswered, BoardExecution, BoardOutcome, BoardRequest, ListSummaries, OpNoting,
    ProjectBoardError, ReadRequest, RunEnd, RunEndOutcome, TaskStart,
};
use crate::application::project_board::ports::{
    BoardOperations, BoardStoreError, RunLiveness, StartedRuns, SwarmRunReports,
};
use crate::application::project_board::run_facts::RunFacts;
use crate::application::project_board::start::StartFacts;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::{LEFT_RUN_END, LaunchBegunRun};
use crate::infrastructure::http::board_remote::RemoteBoardOperations;
use crate::infrastructure::http::board_remote_runs::{
    MALFORMED_BEGIN, RemoteBoardRuns, begun_of, launched_answer, recorded_kept, served,
};
use crate::infrastructure::http::board_service::limits::MAX_BODY_BYTES;

/// Why a start whose call was cancelled before its write writes nothing:
/// a local cancelled store's own words.
const CANCELLED_UNWRITTEN: &str = "the call was cancelled before its write";

/// What this session's swarms are to its runs on a board server.
pub struct RunSwarms {
    pub started: Option<Arc<dyn StartedRuns>>,
    pub liveness: Option<Arc<dyn RunLiveness>>,
    pub reports: Option<Arc<dyn SwarmRunReports>>,
}

pub struct RemoteRuns {
    remote: Arc<RemoteBoardOperations>,
    launch: LaunchBegunRun,
    swarms: RunSwarms,
}

impl RemoteRuns {
    pub fn new(
        remote: Arc<RemoteBoardOperations>,
        launch: LaunchBegunRun,
        swarms: RunSwarms,
    ) -> Self {
        Self {
            remote,
            launch,
            swarms,
        }
    }

    /// `start` begun on the server and launched here, noted as a local
    /// start is (its write's facts are the server's).
    fn started(
        &self,
        read: ReadRequest<'_>,
        start: &TaskStart,
        noting: OpNoting,
    ) -> BoardExecution {
        let version = Some(u64::from(start.plan_version));
        let mut seen = StartFacts {
            plan_version: version,
            ..StartFacts::default()
        };
        let noted = noting == OpNoting::Noted;
        let mut facts = noted.then(OpFacts::default);
        let seeing = noted.then_some(&mut seen);
        let result = self.launched(read, start, (noting, &mut facts), seeing);
        if let Some(facts) = facts.as_mut() {
            facts.plan_version = version;
            facts.start = Some(seen);
        }
        BoardExecution { result, facts }
    }

    fn launched(
        &self,
        read: ReadRequest<'_>,
        start: &TaskStart,
        (noting, facts): (OpNoting, &mut Option<OpFacts>),
        seen: Option<&mut StartFacts>,
    ) -> Result<BoardAnswered, ProjectBoardError> {
        self.launch.may_launch()?;
        // A call cancelled before its write writes nothing, as locally.
        match self.launch.cancelled() && false {
            false => {}
            true => return Err(ProjectBoardError::Refused(CANCELLED_UNWRITTEN.into())),
        }
        let begun = self.remote.begin(read, noting);
        if let (Some(facts), Some(written)) = (facts.as_mut(), begun.facts) {
            *facts = written;
        }
        let answered = begun.result?;
        // A refusal of the begin is the server's, as it rendered it.
        let Some(value) = served(&answered).filter(|value| value["ok"] == json!(true)) else {
            return Ok(answered);
        };
        let malformed =
            || ProjectBoardError::Store(BoardStoreError::Unavailable(MALFORMED_BEGIN.into()));
        let begun = begun_of(start, value).ok_or_else(malformed)?;
        let (begun, coordinator) = self.launch.launch(begun, seen)?;
        Ok(BoardAnswered {
            outcome: BoardOutcome::Served(launched_answer(begun.written, &coordinator)),
            clock_note: None,
        })
    }

    /// `end` sent with what this session saw of its run.
    fn ended(
        &self,
        read: ReadRequest<'_>,
        end: &RunEnd,
        asked: (ListSummaries, OpNoting),
    ) -> BoardExecution {
        let swarms = &self.swarms;
        let (liveness, reports) = (swarms.liveness.as_deref(), swarms.reports.as_deref());
        let facts = RunFacts::gathered((&end.task, end.run), liveness, reports);
        // Within the request's bytes, the rest of the body left room for.
        let succeeds = end.outcome == RunEndOutcome::Succeeded;
        let facts = facts.fitting(succeeds, MAX_BODY_BYTES as usize - 64 * 1024);
        let wire = serde_json::to_value(&facts).unwrap_or(Value::Null);
        let execution =
            self.remote
                .execute_with(read, asked, Map::from_iter([("facts".into(), wire)]));
        // A board kept here that the server recorded need not be kept.
        let recorded = execution.result.as_ref().is_ok_and(recorded_kept);
        if let (Some(reports), Some(_), true) = (reports, &facts.reports, recorded) {
            reports.recorded(&end.task, end.run);
        }
        execution
    }
}

impl BoardOperations for RemoteRuns {
    fn execute(
        &self,
        read: ReadRequest<'_>,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> BoardExecution {
        match read.request {
            BoardRequest::Start(start) => self.started(read, start, noting),
            BoardRequest::EndRun(end) => self.ended(read, end, (summaries, noting)),
            _ => self.remote.execute(read, summaries, noting),
        }
    }

    /// Each run this session's starts launched that the server shows still
    /// running, once, to be ended as a local session's left runs are.
    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError> {
        let started = self
            .swarms
            .started
            .as_ref()
            .map(|started| started.started());
        let runs = RemoteBoardRuns(Arc::clone(&self.remote));
        let mut ends = Vec::new();
        let mut asked = BTreeSet::new();
        for (task, run) in started.unwrap_or_default() {
            if asked.insert((task.clone(), run)) && runs.running(&task, run)? {
                ends.push(RunEnd {
                    task,
                    run,
                    outcome: LEFT_RUN_END,
                    expected_updated: None,
                    stop: None,
                });
            }
        }
        assert!(
            ends.iter().all(|end| end.outcome == LEFT_RUN_END),
            "a left run is ended one way"
        );
        Ok(ends)
    }
}
