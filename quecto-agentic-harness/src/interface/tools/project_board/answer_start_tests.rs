//! The start answers' steps are the Pocket start prompt's words, moved
//! unchanged: only the task's id and the run's number are filled in.
use super::{CONTENDED, PUSH_STEP, REFUSED, refused, refused_before_board, running, undo};
use crate::application::project_board::dto::{ProjectBoardError, RunAfterUndo, TaskStart};
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::value_objects::slug::Slug;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// The Pocket relay's start prompt for task `a`, plan version 1, as it
/// read before the split (quecto-pocket `relay/src/domain/board_start.rs`).
const PROMPT: &str = "Start the swarm for board task a on its accepted plan, version 1: call \
     the board tool's start(task \"a\", plan_version 1), exactly as written; on \"contended\", \
     call it again, up to three times, then say so and stop. If start is refused for any other \
     reason, say why in a sentence and stop; do not start the task any other way. On \
     \"launch_failed\", undo by its fields: stop any coordinator still_running names with \
     agent_cmd kill; with run_state \"open\", call end_run(task \"a\", the run it names, \
     outcome \"failed\"), which moves the task back to ready; say why the launch failed, and \
     stop. Once it has started, you hold the task and you end its \
     run as your playbook's \"Board task plans and swarm runs\" says. Renew the claim with \
     renew(task \"a\") once renew_after has passed; while agent_cmd swarm_control status \
     shows the run running, do nothing else. On \"not_holder\" from renew or end_run, stop the \
     coordinator with agent_cmd kill, do not end the run or move the task, and tell the user \
     here. When the coordinator reports, read its report with get_messages and export what \
     the run must keep. A run paused as blocked waits for the user: ask them here before you \
     close it. Close a run paused with a proposed outcome (a cancelled run is final already). \
     For a run paused with no outcome, resume it with agent_cmd swarm_control resume and ask \
     the coordinator to end it with its outcome, or to cancel it with cancel_run; wait for its \
     report, check the status again, and close the run if it is paused with an outcome. Only \
     then stop the coordinator with agent_cmd kill (one already stopping is fine) and call \
     end_run(task \"a\", the run start answered) with the outcome from the status or the \
     close's receipt, never from the coordinator's words: \"succeeded\" for succeeded; \
     \"stopped\" for blocked or cancelled, or when the coordinator cannot answer at all; \
     \"failed\" for failed or budget-exhausted; \"lost\" only when nobody can account for the \
     run. On \"run_live\", kill the agent it names and call end_run again; after three tries, \
     tell the user here and stop. On \"claim_expired\", renew(task \"a\") and call it again. \
     Then, if the run succeeded, link_pr(task \"a\", pr) for the run's pull request, if it \
     opened one; the run's end has moved the task. Say how it went.";

/// Every step is a span of the prompt, word for word, the run number
/// read back as the prompt put it; taken out, they leave only the call
/// itself, so nothing of the prompt was dropped or reworded.
#[test]
fn the_steps_are_the_prompts_words_moved_unchanged() {
    let task = slug("a");
    let run = running(&task, 7);
    assert!(run.contains("end_run(task \"a\", run 7) with"), "{run}");
    // The one step added since (#2666 round 1), taken out whole.
    assert_eq!(run.matches(PUSH_STEP).count(), 1, "{run}");
    let run = run.replace(
        "end_run(task \"a\", run 7)",
        "end_run(task \"a\", the run start answered)",
    );
    let run = run.replace(&format!("{PUSH_STEP} "), "");
    let undo = undo(&task);
    let mut rest = PROMPT.to_string();
    for step in [CONTENDED, REFUSED, undo.as_str(), run.as_str()] {
        assert_eq!(PROMPT.matches(step).count(), 1, "verbatim, once: {step}");
        rest = rest.replace(step, "");
    }
    assert_eq!(
        rest.trim_end(),
        "Start the swarm for board task a on its accepted plan, version 1: call the board \
         tool's start(task \"a\", plan_version 1), exactly as written;"
    );
}

#[test]
fn the_fixed_steps_read_exactly() {
    assert_eq!(
        CONTENDED,
        "on \"contended\", call it again, up to three times, then say so and stop."
    );
    assert_eq!(
        REFUSED,
        "If start is refused for any other reason, say why in a sentence and stop; do not \
         start the task any other way."
    );
    assert_eq!(
        undo(&slug("board-store")),
        "On \"launch_failed\", undo by its fields: stop any coordinator still_running names \
         with agent_cmd kill; with run_state \"open\", call end_run(task \"board-store\", the \
         run it names, outcome \"failed\"), which moves the task back to ready; say why the \
         launch failed, and stop."
    );
    assert_eq!(
        PUSH_STEP,
        "If the run succeeded, before you close it make sure every commit its evidence names is \
         on a branch of the board's remote (git fetch <remote>, then git branch -r --contains \
         <commit>): have the coordinator push any that is not. end_run refuses \"succeeded\" \
         (\"not_pushed\") while one is not. If the container cannot push, stop the coordinator \
         with agent_cmd kill: that keeps its environment while the run's completion commit is \
         not on that remote, and the retention reason names the environment and its checkout; \
         fetch the commit from that checkout, push it, then remove the environment with \
         kill_container. Ending the session without that kill does not keep it."
    );
    let steps = running(&slug("board-store"), 3);
    assert!(
        steps.contains(&format!(
            "export what the run must keep. {PUSH_STEP} A run paused"
        )),
        "before the close: {steps}"
    );
    assert!(
        steps.starts_with("Once it has started, you hold the task"),
        "{steps}"
    );
    assert!(steps.ends_with("Say how it went."), "{steps}");
    assert_eq!(steps.matches("\"board-store\"").count(), 4, "{steps}");
    assert!(
        !steps.contains("{task}") && !steps.contains("{run}"),
        "{steps}"
    );
}

/// Only a start's refusals carry steps: a contended one the retry, a
/// failed launch the undo, every other the stop.
#[test]
fn each_refusal_of_a_start_carries_its_own_step() {
    let start = BoardRequest::Start(TaskStart {
        task: slug("a"),
        plan_version: 1,
    });
    let contended = ProjectBoardError::Contended { attempts: 5 };
    assert_eq!(refused(&start, &contended).as_deref(), Some(CONTENDED));
    let failed = ProjectBoardError::LaunchFailed {
        reason: "no container runtime".into(),
        run: 1,
        run_state: RunAfterUndo::Folded,
        released: Some(true),
        still_running: None,
    };
    assert_eq!(refused(&start, &failed), Some(undo(&slug("a"))));
    for other in [
        ProjectBoardError::NoLauncher,
        ProjectBoardError::NotFound(slug("a")),
        ProjectBoardError::Brief {
            code: "coordinator_model",
            reason: "words".into(),
        },
    ] {
        assert_eq!(
            refused(&start, &other).as_deref(),
            Some(REFUSED),
            "{other:?}"
        );
    }
    let complete = BoardRequest::Complete(slug("a"));
    assert_eq!(refused(&complete, &contended), None);
    assert_eq!(refused_before_board(Some("start")), Some(REFUSED));
    assert_eq!(refused_before_board(Some("end_run")), None);
    assert_eq!(refused_before_board(None), None);
}
