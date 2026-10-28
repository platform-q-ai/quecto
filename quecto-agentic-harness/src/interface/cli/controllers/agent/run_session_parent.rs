//! A one-shot run under a trusted root parent's turn-outcome guard (#2708).
//!
//! The prompt is the run's external instruction. Each outer run is
//! admitted and completed through the shared runtime: a turn that ends
//! with nothing running gets the one correction as a run of its own; a
//! turn that waits on its own children waits for their next note under
//! the run's one `--max-time` deadline, and that note is the next run.
//! Completed outcomes exit 0; correction-exhausted stalled exits 1. Typed
//! diagnostics go to stderr. An unguarded run keeps its error semantics.
use std::time::{Duration, Instant};

use super::agent::{AgentOutput, DeadlineResult, run_with_deadline};
use crate::application::agent_turn::agent_loop::AgentLoopImpl;
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::agent_turn::use_cases::parent_outcome::{
    ActiveTurn, ParentOutcomeRuntime, ParentVerdict, TurnAdmission, TurnEnd,
};
use crate::domain::agents::value_objects::agent::AgentResult;
use crate::domain::agents::value_objects::turn_outcome::{
    CORRECTION_NOTE, TurnOutcome, TurnOutcomeRecord, UNKNOWN_REASON, WorkLiveness,
};
use crate::domain::conversation::services::turn_origin;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::shared::error::DomainError;
use crate::infrastructure::tools::agents::owned_work::registry_liveness;
use crate::infrastructure::tools::agents::subagent_registry::{NotificationRx, SubagentRegistry};

/// What a guarded one-shot run owns: its runtime, and the registry and
/// note channel of the children it launches.
pub(crate) struct ParentRun {
    runtime: std::sync::Arc<ParentOutcomeRuntime>,
    registry: Option<SubagentRegistry>,
    notes: Option<NotificationRx>,
    /// How long a wait without a deadline lasts with no note arriving.
    quiet: Duration,
}

/// The quiet bound of a wait without `--max-time`: no note from the run's
/// children for this long ends the run needs_user, as unknown work does.
const QUIET_LIMIT: Duration = Duration::from_secs(30 * 60);

impl ParentRun {
    pub(crate) fn of(
        runtime: Option<std::sync::Arc<ParentOutcomeRuntime>>,
        registry: Option<SubagentRegistry>,
        notes: Option<NotificationRx>,
    ) -> Option<Self> {
        runtime.map(|runtime| Self {
            runtime,
            registry,
            notes,
            quiet: QUIET_LIMIT,
        })
    }

    #[cfg(test)]
    pub(crate) fn quiet_for(self, quiet: Duration) -> Self {
        Self { quiet, ..self }
    }

    fn observe(&self) -> WorkLiveness {
        let noted = self.notes.as_ref().is_some_and(|rx| !rx.is_empty());
        registry_liveness(self.registry.as_ref()).and(WorkLiveness::of(noted))
    }
}

/// How a one-shot run ended.
pub(crate) enum Driven {
    /// Its last run's result and, when guarded, the outcome it settled on.
    Ended(
        Box<Result<AgentResult, DomainError>>,
        Option<TurnOutcomeRecord>,
    ),
    /// `--max-time` (these seconds) stopped it.
    TimedOut(u64),
}

fn ended(result: Result<AgentResult, DomainError>, settled: Option<TurnOutcomeRecord>) -> Driven {
    Driven::Ended(Box::new(result), settled)
}

/// Run the prompt `opener` (already in `messages`), guarded when `parent`
/// is: corrections and children's notes run as further runs of the same
/// episode, all under one deadline.
pub(crate) fn drive(
    rt: &tokio::runtime::Runtime,
    agent: &mut AgentLoopImpl,
    messages: &mut Vec<Message>,
    (max_time, parent): (Option<u64>, Option<ParentRun>),
    opener: uuid::Uuid,
) -> Driven {
    agent.reconcile_declaration_policy(true);
    let mut latest_text = None;
    let Some(mut parent) = parent else {
        return match max_time.map(|secs| (secs, run_with_deadline(rt, agent, messages, secs))) {
            Some((_, DeadlineResult::Completed(result))) => {
                ended(present(result, &mut latest_text), None)
            }
            Some((secs, DeadlineResult::TimedOut)) => Driven::TimedOut(secs),
            None => ended(
                present(rt.block_on(agent.process(messages)), &mut latest_text),
                None,
            ),
        };
    };
    let deadline = max_time.map(|secs| Instant::now() + Duration::from_secs(secs));
    let timed_out = || Driven::TimedOut(max_time.unwrap_or_default());
    let episode = TurnAdmission::External {
        episode_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut turn = admit(&parent, agent, episode, opener);
    loop {
        let Some(result) = run_once(rt, agent, messages, deadline) else {
            return timed_out();
        };
        let result = present(result, &mut latest_text);
        let end = match &result {
            Ok(_) => TurnEnd::Completed(parent.observe()),
            Err(_) => TurnEnd::Interrupted,
        };
        let next = match parent.runtime.complete(turn, end) {
            ParentVerdict::Unsettled => return ended(result, None),
            ParentVerdict::Correct => (TurnAdmission::Correction, CORRECTION_NOTE.to_owned()),
            ParentVerdict::Published(record) if record.outcome == TurnOutcome::Waiting => {
                match wait_for_note(rt, &mut parent, deadline) {
                    Wait::Note(note) => (TurnAdmission::WorkWake, note),
                    Wait::TimedOut => return timed_out(),
                    // Unable to tell whether work still runs (#2762 round 2).
                    Wait::Closed | Wait::Quiet => {
                        let reason = UNKNOWN_REASON.to_owned();
                        let outcome = TurnOutcome::NeedsUser;
                        let quiet = TurnOutcomeRecord {
                            outcome,
                            reason,
                            ..record
                        };
                        return ended(result, Some(quiet));
                    }
                }
            }
            ParentVerdict::Published(record) => return ended(result, Some(record)),
        };
        let note = turn_origin::harness_note(next.1, messages);
        let opener = note.id();
        messages.push(note);
        turn = admit(&parent, agent, next.0, opener);
    }
}

/// Select only this invocation's appended assistant text, never session history.
fn present(
    result: Result<AgentResult, DomainError>,
    latest: &mut Option<String>,
) -> Result<AgentResult, DomainError> {
    result.map(|mut result| {
        for message in &result.appended_messages {
            if let (crate::domain::conversation::value_objects::message::Role::Assistant, false) =
                (&message.role, message.content.trim().is_empty())
            {
                *latest = Some(message.content.clone());
            }
        }
        // Synthetic diagnostics are not provider replies and take precedence.
        if let (false, true, Some(text)) = (
            result.iteration_limit_reached,
            result.response.trim().is_empty(),
            latest.as_ref(),
        ) {
            result.response = text.clone();
        }
        result
    })
}

fn admit(
    parent: &ParentRun,
    agent: &AgentLoopImpl,
    admission: TurnAdmission,
    opener: uuid::Uuid,
) -> ActiveTurn {
    let turn = parent
        .runtime
        .admit(agent.session_key(), admission, &opener.to_string());
    turn.expect("a one-shot run admits its prompt, its correction and its children's notes")
}

/// One outer run, within what is left of the deadline (`None`: none left).
fn run_once(
    rt: &tokio::runtime::Runtime,
    agent: &mut AgentLoopImpl,
    messages: &mut Vec<Message>,
    deadline: Option<Instant>,
) -> Option<Result<AgentResult, DomainError>> {
    let Some(deadline) = deadline else {
        return Some(rt.block_on(agent.process(messages)));
    };
    let deadline = tokio::time::Instant::from_std(deadline);
    rt.block_on(async { tokio::time::timeout_at(deadline, agent.process(messages)).await })
        .ok()
}

enum Wait {
    Note(String),
    TimedOut,
    Closed,
    /// No note for the quiet bound, with no deadline before it.
    Quiet,
}

/// The next note of the run's own children, within the deadline.
fn wait_for_note(
    rt: &tokio::runtime::Runtime,
    parent: &mut ParentRun,
    deadline: Option<Instant>,
) -> Wait {
    let quiet = Instant::now() + parent.quiet;
    let Some(notes) = parent.notes.as_mut() else {
        return Wait::Closed;
    };
    let until = tokio::time::Instant::from_std(deadline.map_or(quiet, |at| at.min(quiet)));
    match rt.block_on(async { tokio::time::timeout_at(until, notes.recv()).await.ok() }) {
        Some(Some(note)) => Wait::Note(note.to_message()),
        Some(None) => Wait::Closed,
        None if deadline.is_some_and(|at| at <= quiet) => Wait::TimedOut,
        None => Wait::Quiet,
    }
}

/// Completed turns exit 0; stalled is correction exhaustion (1). Every
/// guarded settlement has a typed stderr diagnostic.
pub(crate) fn exit_code(settled: Option<&TurnOutcomeRecord>, out: &mut AgentOutput<'_>) -> i32 {
    let Some(record) = settled else {
        return 0;
    };
    let name = match record.outcome {
        TurnOutcome::Done => "done",
        TurnOutcome::NeedsUser => "needs_user",
        TurnOutcome::Waiting => "waiting",
        TurnOutcome::Stalled => "stalled",
    };
    out.stderr
        .push_str(&format!("turn outcome: {name}: {}\n", record.reason));
    0
}

#[cfg(test)]
#[path = "run_session_parent_tests.rs"]
mod issue_2708_cli_parent_tests;
