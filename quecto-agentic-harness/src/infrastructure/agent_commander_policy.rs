//! SPIKE: what code would do with Jev's answers — thresholds and composition.
use serde_json::{Value, json};

use super::stall::stall_action;
use super::{ACT_CONFIDENCE, AgentRole, CommanderEvent, PARENT_HANDLES, STOP_CONFIDENCE, Worker};

/// Statuses whose configuration failures only the owner can fix.
fn owner_fixable_status(status: Option<u16>) -> bool {
    matches!(status, Some(400..=404))
}

/// Events that earn a digest line. A failed tool call alone is routine:
/// it reaches the owner only when urgent.
fn digest_worthy(event: &CommanderEvent) -> bool {
    matches!(
        event,
        CommanderEvent::TurnEnd { .. }
            | CommanderEvent::ProviderFailure { .. }
            | CommanderEvent::SubagentNotice { .. }
    )
}

/// How far an escalation reaches the owner; composition only ever raises it.
pub(super) fn escalation_rank(action: &str) -> u8 {
    match action {
        "interrupt_owner" => 3,
        "promote_in_tui" => 2,
        "add_to_digest" => 1,
        _ => 0,
    }
}

/// An HTML page in place of the API's JSON error: a CDN challenge or proxy.
fn is_html_page(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("<!doctype html") || lower.contains("<html")
}

impl Worker {
    /// What code would do with the answers (dry run: recorded only).
    pub(super) fn would_do(event: &CommanderEvent, role: &AgentRole, answers: &Value) -> Value {
        let choice = |id: &str| -> Option<(String, f64)> {
            let a = answers.get(id)?;
            Some((
                a.get("choice")?.as_str()?.to_string(),
                a.get("confidence")?.as_f64()?,
            ))
        };
        let mut actions = serde_json::Map::new();
        if let Some((c, conf)) = choice("turn_end") {
            let act = if conf < ACT_CONFIDENCE {
                "none_uncertain"
            } else {
                match c.as_str() {
                    "complete" => "none",
                    "waiting_on_others" => "none",
                    "needs_input" => "notify_asker_needs_input",
                    "cut_off" => "auto_continue",
                    "stopped_early" => "nudge_continue",
                    _ => "none",
                }
            };
            actions.insert("5".into(), json!(act));
        }
        if let Some((c, conf)) = choice("child_state") {
            let act = if conf < ACT_CONFIDENCE {
                "plain_idle_note_uncertain"
            } else {
                match c.as_str() {
                    "done" => "tell_parent_done",
                    "waiting_on_subagents" => "plain_idle_note",
                    "question_for_parent" => "tell_parent_question",
                    "blocked" => "tell_parent_blocked_badge",
                    "failed" => "tell_parent_failed_badge",
                    "partial" => "nudge_child_continue",
                    _ => "plain_idle_note",
                }
            };
            actions.insert("6".into(), json!(act));
        }
        if let Some((c, conf)) = choice("provider_error") {
            let act = if conf < ACT_CONFIDENCE {
                "keep_rule_based_handling"
            } else {
                match c.as_str() {
                    "fixable_by_model" => "reprompt_with_error",
                    "configuration" => {
                        let (status, error) = match event {
                            CommanderEvent::ProviderFailure {
                                http_status, error, ..
                            } => (*http_status, error.as_str()),
                            _ => (None, ""),
                        };
                        if is_html_page(error) {
                            // A CDN or proxy page, not the provider's API verdict.
                            "keep_rule_based_handling"
                        } else if owner_fixable_status(status) || conf >= STOP_CONFIDENCE {
                            "stop_and_tell_owner"
                        } else {
                            "keep_rule_based_handling"
                        }
                    }
                    "context_overflow" => "compact_and_retry",
                    "policy_refusal" => "stop_no_retry",
                    "transient" => "retry_with_backoff",
                    _ => "keep_rule_based_handling",
                }
            };
            actions.insert("12".into(), json!(act));
        }
        if let Some(noul) = answers
            .pointer("/owner_needed/noul")
            .and_then(Value::as_f64)
        {
            let urgency = answers
                .pointer("/urgency/score")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            // Asked of child turn ends and of notices the parent receives.
            let parent_handles = answers
                .pointer("/parent_can_handle/noul")
                .and_then(Value::as_f64)
                .is_some_and(|p| p >= PARENT_HANDLES);
            let act = if parent_handles {
                "leave_to_parent"
            } else if noul >= 0.8 && urgency >= 2.0 {
                "interrupt_owner"
            } else if noul >= 0.8 {
                "promote_in_tui"
            } else if noul >= 0.5 && digest_worthy(event) {
                "add_to_digest"
            } else {
                "none"
            };
            actions.insert("22".into(), json!(act));
            // Composition (policy in code): a configuration failure only the
            // owner can fix, or a child that is blocked/failed, reaches the
            // owner whatever the standalone escalation judgment said.
            let stall = stall_action(answers, role);
            let terminal_on_root = matches!(role, AgentRole::Root)
                && matches!(event, CommanderEvent::ProviderFailure { outcome, .. } if outcome == "terminal");
            let composed = match (
                actions.get("12").and_then(Value::as_str),
                actions.get("6").and_then(Value::as_str),
                stall,
            ) {
                (Some("stop_and_tell_owner"), _, _) => Some("interrupt_owner"),
                (_, Some("tell_parent_blocked_badge" | "tell_parent_failed_badge"), _)
                    if !parent_handles =>
                {
                    Some("promote_in_tui")
                }
                (_, _, Some("tell_owner_stalled")) => Some("promote_in_tui"),
                // The root's run stopped: the owner is the only one left to act.
                _ if terminal_on_root => Some("promote_in_tui"),
                (Some("stop_no_retry"), _, _) => Some("add_to_digest"),
                // A stuck child is its parent's to replace, not a digest line.
                (_, _, Some("tell_parent_stalled_suggest_replacement"))
                    if matches!(act, "none" | "add_to_digest") =>
                {
                    Some("leave_to_parent")
                }
                _ => None,
            };
            if let Some(composed) = composed
                .filter(|c| *c == "leave_to_parent" || escalation_rank(c) > escalation_rank(act))
            {
                actions.insert("22_composed".into(), json!(composed));
            }
        }
        if let Some(action) = stall_action(answers, role) {
            actions.insert("stall".into(), json!(action));
        }
        Value::Object(actions)
    }
}
