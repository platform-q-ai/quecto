//! SPIKE: the #22 escalation questions.
use serde_json::{Value, json};

/// #22 for a sub-agent: its parent reads every report first.
pub(super) fn parent_question(questions: &mut serde_json::Map<String, Value>) {
    questions.insert(
        "parent_can_handle".into(),
        json!({
            "type": "noul",
            "instructions": "This agent is a sub-agent. Its parent agent reads this event and can relaunch or re-scope the sub-agent, retry, answer its questions, fix its inputs, or do the work itself. Can the parent agent deal with this event without involving the human owner?",
            "criteria": {
                "true": "The parent agent can resolve or route this itself: a blocked or failed task it can relaunch, a changed target it can re-point, a question it can answer, or a finished or partial result it can use",
                "false": "Only the human can resolve it: an owner decision or approval, missing credentials or access, or a risky or harmful outcome the parent cannot undo"
            }
        }),
    );
}

/// #22: does a human need to see this now, what kind, how urgent.
pub(super) fn escalation_questions(questions: &mut serde_json::Map<String, Value>) {
    questions.insert(
        "owner_needed".into(),
        json!({
            "type": "noul",
            "instructions": "The owner is a human who supervises these coding agents but is not watching every event. Does the owner need to see this event now, rather than later or never?",
            "criteria": {
                "true": "Something only the owner can resolve or should know promptly: a decision or approval only the owner can give, a blocker agents cannot clear, a failure that stops the work, or a risky or surprising outcome",
                "false": "Routine progress, a finished step, or something the agents can handle themselves"
            }
        }),
    );
    questions.insert(
        "owner_kind".into(),
        json!({
            "type": "choice",
            "instructions": "If this event were raised with the owner, what kind of matter would it be?",
            "criteria": {
                "owner_decision": "A choice only the owner can make (scope, priorities, trade-offs, preferences)",
                "approval": "The agent needs permission before doing something (risky, destructive, outward-facing, or costly)",
                "blocker": "Work cannot continue until something outside the agents is fixed (credentials, configuration, access, environment)",
                "agent_answerable": "A question or problem another agent or the agent itself can resolve without the owner",
                "information": "Useful to know, but no action is needed from anyone"
            }
        }),
    );
    questions.insert(
        "urgency".into(),
        json!({
            "type": "score",
            "instructions": "How urgently does the owner need to act on this event?",
            "criteria": [
                "No action needed from the owner at all",
                "Can wait for the owner's next routine check-in or a daily digest",
                "The owner should look within the hour; work is slowed or waiting",
                "The owner should look now; work is stopped, at risk, or something harmful may happen"
            ]
        }),
    );
}

/// #22 for a notice about a sub-agent: the receiving agent is its parent.
pub(super) fn receiving_parent_question(questions: &mut serde_json::Map<String, Value>) {
    questions.insert(
        "parent_can_handle".into(),
        json!({
            "type": "noul",
            "instructions": "This agent received a notice about one of its own sub-agents. It can inspect the sub-agent's output, prompt or steer it, relaunch it with a narrower or different brief, or kill it. Can this agent deal with the notice itself, without involving the human owner?",
            "criteria": {
                "true": "This agent can resolve it: a sub-agent that finished, stalled, failed on its task or its context, or crashed once can be read, steered, relaunched or replaced",
                "false": "Only the human can resolve it: credentials, billing or access that stopped the sub-agent, a failure that keeps recurring across relaunches, or a risky or harmful outcome"
            }
        }),
    );
}
