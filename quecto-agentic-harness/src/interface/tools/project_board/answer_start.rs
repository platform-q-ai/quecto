//! What `start`'s answer tells its caller to do next (owner-approved
//! split of the Pocket start prompt): the words moved verbatim from that
//! prompt into the answers, each where it applies, so the prompt can be
//! one line. A started task's answer carries the steps for running it, a
//! `launch_failed` the undo steps, a `contended` the retry, and every
//! other refusal of a start the stop. Only the task's id and the run's
//! number are filled in; every other word is the prompt's.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::operations::{BoardOutcome, BoardRequest};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::FrontEnd;

/// Whether `front_end`'s answers carry steps: the agent tool's alone, as
/// the steps tell an agent what to call next (review round 1, #2567).
pub fn carried_by(front_end: FrontEnd) -> bool {
    match front_end {
        FrontEnd::Tool => true,
        FrontEnd::Cli => false,
    }
}

/// The steps an answered `outcome` carries: a started task's, or one a
/// board server began for its client to launch (B8).
pub fn started(outcome: &BoardOutcome) -> Option<String> {
    match outcome {
        BoardOutcome::Started(started) => Some(running(
            &started.task.fields().id,
            started.run.fields().number,
        )),
        BoardOutcome::Begun(begun) => Some(running(&begun.launch.task, begun.launch.run)),
        _ => None,
    }
}

/// The op whose answers carry steps.
const START_OP: &str = "start";

/// The step a `contended` start answers with.
pub const CONTENDED: &str =
    "on \"contended\", call it again, up to three times, then say so and stop.";

/// The step every other refusal of a start answers with.
pub const REFUSED: &str = "If start is refused for any other reason, say why in a sentence and \
                           stop; do not start the task any other way.";

/// The step added to the prompt's (#2666 round 1): a succeeded run's
/// commits are confirmed on the remote before its coordinator's container
/// can go, as `end_run` refuses a success over any that is not.
pub const PUSH_STEP: &str = "If the run succeeded, before you close it make sure every commit its evidence names is \
     on a branch of the board's remote (git fetch <remote>, then git branch -r --contains \
     <commit>): have the coordinator push any that is not. end_run refuses \"succeeded\" \
     (\"not_pushed\") while one is not. If the container cannot push, stop the coordinator \
     with agent_cmd kill: that keeps its environment while the run's completion commit is \
     not on that remote, and the retention reason names the environment and its checkout; \
     fetch the commit from that checkout, push it, then remove the environment with \
     kill_container. Ending the session without that kill does not keep it.";

/// The steps for running run `run` of task `task`, once it has started.
pub fn running(task: &Slug, run: u32) -> String {
    assert!(run >= 1, "a started run is numbered from 1");
    let task = task.as_str();
    format!(
        "Once it has started, you hold the task and you end its run as your playbook's \
         \"Board task plans and swarm runs\" says. Renew the claim with renew(task \"{task}\") \
         once renew_after has passed; while agent_cmd swarm_control status shows the run \
         running, do nothing else. On \"not_holder\" from renew or end_run, stop the \
         coordinator with agent_cmd kill, do not end the run or move the task, and tell the \
         user here. When the coordinator reports, read its report with get_messages and \
         export what the run must keep. {PUSH_STEP} A run paused as blocked waits for the user: ask them \
         here before you close it. Close a run paused with a proposed outcome (a cancelled run \
         is final already). For a run paused with no outcome, resume it with agent_cmd \
         swarm_control resume and ask the coordinator to end it with its outcome, or to cancel \
         it with cancel_run; wait for its report, check the status again, and close the run if \
         it is paused with an outcome. Only then stop the coordinator with agent_cmd kill (one \
         already stopping is fine) and call end_run(task \"{task}\", run {run}) with the \
         outcome from the status or the close's receipt, never from the coordinator's words: \
         \"succeeded\" for succeeded; \"stopped\" for blocked or cancelled, or when the \
         coordinator cannot answer at all; \"failed\" for failed or budget-exhausted; \"lost\" \
         only when nobody can account for the run. On \"run_live\", kill the agent it names and \
         call end_run again; after three tries, tell the user here and stop. On \
         \"claim_expired\", renew(task \"{task}\") and call it again. Then, if the run \
         succeeded, link_pr(task \"{task}\", pr) for the run's pull request, if it opened one; \
         the run's end has moved the task. Say how it went."
    )
}

/// The step added for an undo a board server refused (#2705 B8).
pub const REFUSED_UNDO: &str =
    "with run_state \"refused\", show(task), then end_run(task, run, outcome failed)";

/// The steps for undoing a start of task `task` whose coordinator did
/// not launch, by the `launch_failed` answer's fields.
pub fn undo(task: &Slug) -> String {
    let task = task.as_str();
    format!(
        "On \"launch_failed\", undo by its fields: stop any coordinator still_running names \
         with agent_cmd kill; with run_state \"open\", call end_run(task \"{task}\", the run it \
         names, outcome \"failed\"), which moves the task back to ready; {REFUSED_UNDO}; say \
         why the launch failed, and stop."
    )
}

/// The steps a refusal of `request` with `error` carries: only a start's,
/// each refusal its own.
pub fn refused(request: &BoardRequest, error: &ProjectBoardError) -> Option<String> {
    use ProjectBoardError as E;
    let BoardRequest::Start(start) = request else {
        return None;
    };
    Some(match error {
        E::Contended { .. } => CONTENDED.to_string(),
        E::LaunchFailed { .. } => undo(&start.task),
        E::Store(_)
        | E::Rule(_)
        | E::Clock(_)
        | E::Invalid(_)
        | E::NotFound(_)
        | E::Exists(_)
        | E::IdInUse { .. }
        | E::SetAsideFile { .. }
        | E::Duplicate { .. }
        | E::Quarantined { .. }
        | E::UnknownReference(_)
        | E::ClosesCycle { .. }
        | E::InvalidWriter(_)
        | E::PrStateUnknown(_)
        | E::PullRequests(_)
        | E::TaskAhead { .. }
        | E::ClockOutOfRange(_)
        | E::ClockAhead { .. }
        | E::Refused(_)
        | E::NotSetAside { .. }
        | E::NoPullRequestHost(_)
        | E::Stale { .. }
        | E::NoLauncher
        | E::Busy(_)
        | E::Unanswered(_)
        | E::RunLive { .. }
        | E::Brief { .. } => REFUSED.to_string(),
    })
}

/// The op `text` names when the strict read refused it (a key named
/// twice, a request past its bytes) but it is a start, read as plain JSON
/// names it: so its refusal still carries the stop (review round 2,
/// #2567). `None` for any other op, and when the op itself cannot be read
/// (not JSON, not an object (an array included), or `op` named twice).
pub fn start_in_unread(text: &str) -> Option<&'static str> {
    #[derive(serde::Deserialize)]
    struct Named {
        op: Option<String>,
    }
    // Only an object is a request: serde would read an array's first
    // element as `op` (review round 3, #2567).
    let object = text.trim_start().starts_with('{');
    let named = object
        .then(|| serde_json::from_str::<Named>(text).ok())??
        .op?;
    match named.as_str() {
        START_OP => Some(START_OP),
        _ => None,
    }
}

/// The step a refusal of `op` made before the board answered carries
/// (an invalid request, or a front end's own refusal): only a start's.
pub fn refused_before_board(op: Option<&str>) -> Option<&'static str> {
    match op {
        Some(START_OP) => Some(REFUSED),
        Some(_) | None => None,
    }
}

#[cfg(test)]
#[path = "answer_start_tests.rs"]
mod tests;
