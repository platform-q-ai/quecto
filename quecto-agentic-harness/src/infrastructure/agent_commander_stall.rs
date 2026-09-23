//! Stall watch: code notices a run of failing tool calls cheaply; Jev then
//! judges whether the agent is stuck or iterating normally.
use std::collections::VecDeque;

use serde_json::{Value, json};

use super::{ACT_CONFIDENCE, AgentRole, head};

/// Tool calls kept for the judgment.
const WINDOW: usize = 12;
/// Failures in the window (and since the last check) that trigger a check.
const FAILURES: usize = 6;
/// Only failures this recent count: slow, sparse failures are not a stall.
const SPAN_SECS: f64 = 900.0;

#[derive(Debug, Clone)]
struct Call {
    ts: f64,
    tool: String,
    ok: bool,
    result: String,
}

#[derive(Debug, Default)]
pub(super) struct StallWatch {
    calls: VecDeque<Call>,
    failures_since_check: usize,
}

impl StallWatch {
    /// Records one tool call; true when this call should trigger a check.
    pub(super) fn record(&mut self, ts: f64, tool: &str, ok: bool, result: &str) -> bool {
        self.calls.push_back(Call {
            ts,
            tool: tool.to_string(),
            ok,
            result: head(result, 300),
        });
        while self.calls.len() > WINDOW {
            self.calls.pop_front();
        }
        if ok {
            return false;
        }
        self.failures_since_check += 1;
        let recent_failures = self
            .calls
            .iter()
            .filter(|call| !call.ok && ts - call.ts <= SPAN_SECS)
            .count();
        let check = recent_failures >= FAILURES && self.failures_since_check >= FAILURES;
        if check {
            self.failures_since_check = 0;
        }
        check
    }

    /// The window as sent to Jev, oldest first.
    pub(super) fn window(&self) -> Vec<Value> {
        let newest = self.calls.back().map_or(0.0, |call| call.ts);
        self.calls
            .iter()
            .map(|call| {
                json!({
                    "tool": call.tool,
                    "ok": call.ok,
                    "result": if call.ok { Value::Null } else { json!(call.result) },
                    "seconds_before_latest": (newest - call.ts).round(),
                })
            })
            .collect()
    }
}

pub(super) fn progress_question(questions: &mut serde_json::Map<String, Value>) {
    questions.insert(
        "progress".into(),
        json!({
            "type": "choice",
            "instructions": "This agent's recent tool calls are listed in `recent_tool_calls`, oldest first (`ok: false` means the call failed). Is the agent making progress, or is it stuck?",
            "criteria": {
                "making_progress": "The failures are a normal part of the work: expected red tests while fixing a bug, exploring and correcting paths or arguments, then moving on; the approach changes and calls succeed in between",
                "stuck": "The agent keeps failing without converging: edits that do not apply or change nothing, re-running the same failing command without changing anything relevant, or cycling between approaches while nothing succeeds"
            }
        }),
    );
}

/// What the supervisor would do with the stall judgment, if one was asked.
pub(super) fn stall_action(answers: &Value, role: &AgentRole) -> Option<&'static str> {
    let answer = answers.get("progress")?;
    let choice = answer.get("choice")?.as_str()?;
    let confidence = answer.get("confidence")?.as_f64()?;
    Some(match (choice, role) {
        (_, _) if confidence < ACT_CONFIDENCE => "none_uncertain",
        ("stuck", AgentRole::Child { .. }) => "tell_parent_stalled_suggest_replacement",
        ("stuck", AgentRole::Root) => "tell_owner_stalled",
        _ => "none",
    })
}

#[cfg(test)]
#[path = "agent_commander_stall_tests.rs"]
mod tests;
