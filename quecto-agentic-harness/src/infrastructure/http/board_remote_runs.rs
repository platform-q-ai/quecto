//! A board server's half of its client's runs (epic #2705 B8), over
//! [`RemoteBoardOperations`]: a begun start read from the server's answer
//! for what its coordinator is launched with ([`begun_of`]), the answer to
//! a start launched here ([`launched_answer`]), a start undone on the
//! server ([`RemoteBoardRuns`]), and whether a run still runs there. No
//! clock is read and no board decoded: answers are read for the fields
//! named here only.
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::board_remote::{RemoteBoardOperations, shown};
use crate::application::project_board::dto::{
    BoardAnswered, BoardOutcome, BoardRequest, ListSummaries, OpNoting, PlanView,
    ProjectBoardError, ReadRequest, RunAfterUndo, ServedAnswer, TaskStart,
};
use crate::application::project_board::ports::{
    BegunStarts, BoardOperations, CoordinatorRequest, LaunchedCoordinator,
};
use crate::application::project_board::start::{Begun, NotedFoldBack, StartFacts, StartUndo};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{FrontEnd, StartClaim};
use crate::domain::project_board::value_objects::vocabulary::{Effort, TaskStatus};

/// Why a begun start could not be launched from its server's answer.
pub const MALFORMED_BEGIN: &str = "the board server's answer to the start is malformed: it may \
                                   have begun the run, so show the task before starting it again";

/// A board server's starts and runs, as its client undoes and asks of them.
pub struct RemoteBoardRuns(pub Arc<RemoteBoardOperations>);

impl RemoteBoardRuns {
    /// Whether run `run` of `task` is still running on the server, as its
    /// `show` says.
    ///
    /// # Errors
    /// The server could not be asked.
    pub fn running(&self, task: &Slug, run: u32) -> Result<bool, ProjectBoardError> {
        let view = PlanView {
            version: None,
            page: None,
        };
        let request = BoardRequest::Show(task.clone(), view);
        let text = json!({"op": "show", "task": task.as_str()}).to_string();
        let read = ReadRequest {
            request: &request,
            text: &text,
            front_end: FrontEnd::Cli,
        };
        let asked = self
            .0
            .execute(read, ListSummaries::Omitted, OpNoting::Unnoted);
        let answer = served(&asked.result?).unwrap_or(Value::Null);
        let mut runs = answer["runs"].as_array().into_iter().flatten();
        Ok(runs.any(|shown| shown["number"] == json!(run) && shown["state"] == json!("running")))
    }
}

/// The served answer `answered` is, as JSON.
pub fn served(answered: &BoardAnswered) -> Option<Value> {
    match &answered.outcome {
        BoardOutcome::Served(answer) => serde_json::from_str(&answer.0).ok(),
        _ => None,
    }
}

/// Whether an end's answer recorded its swarm's kept board: it ended the
/// run, and does not say its items were recorded as unreported.
pub fn recorded_kept(answered: &BoardAnswered) -> bool {
    let answer = served(answered).unwrap_or(Value::Null);
    answer["ok"] == json!(true) && answer.get("items_recorded").is_none()
}

/// The start the server's `answer` began for `start`, with what its
/// coordinator is launched with; `None` when the answer is not one.
pub fn begun_of(start: &TaskStart, answer: Value) -> Option<Begun<Value>> {
    let answer = answer.get("red").cloned()?;
    let number = |value: &Value| value.as_u64().and_then(|number| u32::try_from(number).ok());
    let text = |value: &Value| match value {
        Value::Null => Some(None),
        Value::String(text) => Some(Some(text.clone())),
        _ => None,
    };
    let launch = &answer["launch"];
    let named = answer["task"]["id"].as_str() == Some(start.task.as_str());
    let run = number(&answer["run"]["number"]).filter(|_| named)?;
    let claim = match answer["claim"].as_str()? {
        "fresh" => StartClaim::Fresh,
        "held" => StartClaim::Held,
        _ => return None,
    };
    let effort = match text(&launch["effort"])? {
        Some(effort) => Some(Effort::parse(&effort)?),
        None => None,
    };
    let launch = CoordinatorRequest {
        task: start.task.clone(),
        run,
        plan_version: number(&launch["plan_version"])?,
        brief: launch["brief"].as_str()?.to_string(),
        model: text(&launch["model"])?,
        effort,
    };
    let noted = StartFacts {
        run: Some(u64::from(run)),
        plan_version: Some(u64::from(launch.plan_version)),
        not_applied: Some(u64::try_from(answer["not_applied"].as_array()?.len()).ok()?),
        claim: Some(claim),
        ..StartFacts::default()
    };
    Some(Begun {
        launch,
        from: TaskStatus::parse(answer["from"].as_str()?)?,
        claim,
        noted,
        written: answer,
    })
}

/// The answer to a start launched here: the server's begun answer, with
/// the `coordinator` launched in place of what it was launched with.
pub fn launched_answer(begun: Value, coordinator: &LaunchedCoordinator) -> ServedAnswer {
    let launched = json!({"agent": shown(&coordinator.agent),
                          "environment_ref": shown(&coordinator.environment_ref)});
    let mut answer = Map::new();
    for (key, value) in begun.as_object().into_iter().flatten() {
        match key.as_str() {
            "from" | "launch" => {}
            "run" => {
                answer.insert(key.clone(), value.clone());
                answer.insert("coordinator".into(), launched.clone());
            }
            _ => {
                answer.insert(key.clone(), value.clone());
            }
        }
    }
    ServedAnswer(Value::Object(answer).to_string())
}

/// A begun start undone on the board server it was begun on.
impl BegunStarts for RemoteBoardRuns {
    /// The undo sent as the start it undoes, with its run, its task's
    /// former status and the launch's reason; the run left open unless the
    /// server answers how its undo left it.
    fn undo(&self, undo: &StartUndo, _: bool) -> (RunAfterUndo, Option<NotedFoldBack>) {
        let (task, plan_version) = (undo.task.clone(), undo.plan_version);
        let text = json!({"op": "start", "task": task.as_str(), "plan_version": plan_version});
        let request = BoardRequest::Start(TaskStart { task, plan_version });
        let text = text.to_string();
        let read = ReadRequest {
            request: &request,
            text: &text,
            front_end: FrontEnd::Cli,
        };
        let undone = json!({"run": undo.run, "from": undo.from.as_str(), "reason": undo.reason});
        let beside = Map::from_iter([("undo".to_string(), undone)]);
        let sent = self
            .0
            .execute_with(read, (ListSummaries::Omitted, OpNoting::Unnoted), beside);
        let answer = sent
            .result
            .ok()
            .as_ref()
            .and_then(served)
            .unwrap_or(Value::Null);
        let error = &answer["error"];
        let state = match (error["code"].as_str(), error["run_state"].as_str()) {
            (Some("launch_failed"), Some("folded")) => RunAfterUndo::Folded,
            (Some("launch_failed"), Some("ended_already")) => RunAfterUndo::EndedAlready,
            _ => RunAfterUndo::Open,
        };
        (state, None)
    }

    fn elapsed(&self) -> Option<std::time::Duration> {
        None
    }
}

#[cfg(test)]
#[path = "board_remote_runs_tests.rs"]
mod tests;
