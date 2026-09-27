//! #2213: the low-water mark. Crossing a dial prunes down to the low-water
//! mark, so the turns that follow append without rewriting the prompt
//! prefix; below the dial nothing changes.

use super::*;
use crate::application::context_pruning::estimate_message_tokens;
use crate::application::context_pruning::messages::collapse_conversation_messages_over_limit;
use crate::application::context_pruning::{
    collapse_tool_results_over_limit, estimate_total_tokens,
};
use crate::domain::message::{Message, Role};

/// A spilled assistant message of about 100 tokens in `turn`.
fn turn_message(turn: u32) -> Message {
    let mut msg = Message::assistant(format!("turn {turn} {}", "x".repeat(400)), vec![]);
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
    msg
}

/// The in-flight prompt (exempt) followed by `n` demotable turns.
fn session(n: u32) -> Vec<Message> {
    let mut messages = vec![Message::user("current prompt")];
    messages.extend((1..=n).map(turn_message));
    messages
}

fn spilled_tool_result(i: u32) -> Message {
    let mut msg = Message::tool(format!("call-{i}"), format!("output {i}"));
    msg.tool_name = Some("bash".to_string());
    msg.spill_id = Some(format!("turn{i}:bash:0"));
    msg
}

fn live_tool_results(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == Role::Tool && !m.is_collapsed)
        .count()
}

/// True when a pass changed any message already in the list: the prompt
/// prefix the provider cached from the previous request no longer matches.
fn rewrote_prefix(before: &[Message], after: &[Message]) -> bool {
    before.len() != after.len()
        || before
            .iter()
            .zip(after)
            .any(|(b, a)| b.content != a.content || b.is_collapsed != a.is_collapsed)
}

#[test]
fn low_water_is_the_named_fraction_of_the_limit_rounded_up() {
    assert_eq!(LOW_WATER_PERCENT, 75);
    assert_eq!(low_water(0), 0);
    assert_eq!(low_water(1), 1, "a one-item dial keeps its one item");
    assert_eq!(low_water(3), 3, "ceil(2.25)");
    assert_eq!(low_water(4), 3);
    assert_eq!(low_water(50), 38, "ceil(37.5)");
    assert_eq!(low_water(100), 75);
    assert_eq!(low_water(200_000), 150_000);
}

#[test]
fn low_water_never_exceeds_the_limit_and_never_overflows() {
    for limit in (0..=1_000).chain([usize::MAX - 1, usize::MAX]) {
        let mark = low_water(limit);
        assert!(mark <= limit, "low_water({limit}) = {mark}");
        let exact = limit as u128 * LOW_WATER_PERCENT as u128;
        assert!(mark as u128 * 100 >= exact, "rounded up: {limit}");
        assert!(
            (mark as u128).saturating_sub(1) * 100 < exact.max(1),
            "{limit}"
        );
    }
}

#[test]
fn crossing_the_ceiling_prunes_down_to_the_low_water_mark() {
    let mut messages = session(20);
    let budget = estimate_total_tokens(&messages) - 1;

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);

    assert!(!outcome.over_budget);
    assert!(
        estimate_total_tokens(&messages) <= low_water(budget),
        "{} over the low-water mark {}",
        estimate_total_tokens(&messages),
        low_water(budget)
    );
    assert!(
        outcome.collapsed_to_stubs > 1,
        "a batch, not just the overflow"
    );
    assert_eq!(outcome.dropped, 0, "stubbing alone reaches the mark");
    // Rung 1 stops at the mark: the newest message is still full, and
    // un-stubbing the last stub would put the total back above the mark.
    let original = session(20);
    assert!(!messages.last().is_some_and(|m| m.is_collapsed));
    let last_stub = messages.iter().rposition(|m| m.is_collapsed).unwrap();
    let restored = estimate_total_tokens(&messages) - estimate_message_tokens(&messages[last_stub])
        + estimate_message_tokens(&original[last_stub]);
    assert!(restored > low_water(budget), "stubbed past the mark");
}

#[test]
fn a_total_exactly_at_the_ceiling_changes_nothing_and_one_over_prunes_to_low_water() {
    let mut at = session(20);
    let total = estimate_total_tokens(&at);
    let before = at.clone();
    let outcome = enforce_context_ceiling_ladder(&mut at, total, 0);
    assert!(
        !rewrote_prefix(&before, &at),
        "at the ceiling nothing moves"
    );
    assert_eq!(outcome.collapsed_to_stubs + outcome.dropped, 0);

    let mut over = session(20);
    let outcome = enforce_context_ceiling_ladder(&mut over, total - 1, 0);
    assert!(outcome.collapsed_to_stubs > 0);
    assert!(estimate_total_tokens(&over) <= low_water(total - 1));
}

#[test]
fn a_total_between_the_low_water_mark_and_the_ceiling_changes_nothing() {
    let mut messages = session(20);
    let total = estimate_total_tokens(&messages);
    // The low-water mark of this budget is below the total; the ceiling is not.
    let budget = total + total / 10;
    assert!(low_water(budget) < total && total <= budget);
    let before = messages.clone();

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);

    assert!(!rewrote_prefix(&before, &messages));
    assert_eq!(outcome.collapsed_to_stubs + outcome.dropped, 0);
    assert!(!outcome.over_budget);
}

#[test]
fn two_consecutive_over_ceiling_appends_rewrite_the_prefix_once() {
    let mut messages = session(10);
    let budget = estimate_total_tokens(&messages) + 50;
    let mut rewrites = 0;
    for turn in [11, 12] {
        messages.push(turn_message(turn));
        let before = messages.clone();
        enforce_context_ceiling_ladder(&mut messages, budget, 0);
        rewrites += usize::from(rewrote_prefix(&before, &messages));
    }
    assert_eq!(rewrites, 1, "the second append lands in the headroom");
}

#[test]
fn an_unreachable_low_water_mark_drops_only_down_to_the_ceiling() {
    // The exempt in-flight prompt alone is 97% of the budget: stubbing every
    // old turn is not enough to meet the ceiling, and the low-water mark
    // cannot be reached at all. The drop rung then drops only down to the
    // ceiling, keeping the newest stubs, instead of deleting every stub.
    let old = session(4);
    let old_tokens = estimate_total_tokens(&old[1..]);
    let budget = old_tokens * 5;
    let prompt = Message::user("p".repeat(4 * (budget * 97 / 100)));
    let prompt_content = prompt.content.clone();
    let mut messages = vec![prompt];
    messages.extend(old.into_iter().skip(1));

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);

    assert!(!outcome.over_budget, "the ceiling itself is met");
    assert_eq!(messages[0].content, prompt_content, "the prompt is exempt");
    assert!(outcome.dropped >= 1, "stubbing alone missed the ceiling");
    let survivors: Vec<u32> = messages[1..].iter().filter_map(|m| m.turn).collect();
    assert!(!survivors.is_empty(), "stubs survive an unreachable mark");
    assert_eq!(survivors.len() + outcome.dropped, 4);
    let newest: Vec<u32> = (outcome.dropped as u32 + 1..=4).collect();
    assert_eq!(survivors, newest, "the oldest stubs go first");
    assert!(messages[1..].iter().all(|m| m.is_collapsed));
    // Minimal: the oldest dropped stub would not have fit back in.
    let one_stub = estimate_message_tokens(&messages[1]);
    assert!(estimate_total_tokens(&messages) + one_stub > budget);
}

#[test]
fn a_reachable_low_water_mark_is_the_drop_target_when_stubbing_misses_the_ceiling() {
    // Four stubs alone are still over a budget of 100; only the tiny prompt
    // is exempt, so dropping reaches the mark and drops down to it, not
    // just under the ceiling.
    let mut messages = session(4);
    let budget = 100;
    assert!(estimate_total_tokens(&messages[..1]) <= low_water(budget));

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);

    assert!(outcome.dropped >= 1, "stubbing alone missed the ceiling");
    assert!(
        estimate_total_tokens(&messages) <= low_water(budget),
        "{} over the mark",
        estimate_total_tokens(&messages)
    );
}

#[test]
fn stubbing_that_meets_the_ceiling_drops_nothing_even_above_the_mark() {
    // Four pinned turns dominate: stubbing turns 1 and 2 meets the ceiling
    // but not the low-water mark. The drop rung is a last resort for the
    // ceiling, so both stubs stay.
    let mut messages = session(6);
    let budget = estimate_total_tokens(&messages) - 1;

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 4);

    assert_eq!(outcome.collapsed_to_stubs, 2);
    assert_eq!(outcome.dropped, 0, "no drop while stubbing met the ceiling");
    assert!(!outcome.over_budget);
    let total = estimate_total_tokens(&messages);
    assert!(low_water(budget) < total && total <= budget, "{total}");
}

#[test]
fn pinned_recent_turns_are_kept_when_they_dominate_the_budget() {
    let mut messages = session(6);
    let pinned: Vec<String> = messages[5..].iter().map(|m| m.content.clone()).collect();
    // Room for the prompt and the two pinned turns only.
    let budget = estimate_total_tokens(&messages[..1]) + estimate_total_tokens(&messages[5..]);

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 2);

    assert!(!outcome.over_budget);
    let kept: Vec<String> = messages
        .iter()
        .filter(|m| !m.is_collapsed && m.turn.is_some())
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(kept, pinned, "the pinned tail is never demoted");
}

#[test]
fn the_tool_count_dial_collapses_down_to_its_low_water_mark_when_crossed() {
    let mut messages: Vec<Message> = (0..50).map(spilled_tool_result).collect();
    assert_eq!(collapse_tool_results_over_limit(&mut messages, 50), 0);

    messages.push(spilled_tool_result(50));
    assert_eq!(collapse_tool_results_over_limit(&mut messages, 50), 13);
    assert_eq!(live_tool_results(&messages), low_water(50));
    assert!(
        messages[..13].iter().all(|m| m.is_collapsed),
        "oldest first"
    );
}

#[test]
fn two_consecutive_tool_results_over_the_count_dial_rewrite_the_prefix_once() {
    let mut messages: Vec<Message> = (0..50).map(spilled_tool_result).collect();
    let mut rewrites = 0;
    for i in [50, 51] {
        messages.push(spilled_tool_result(i));
        let before = messages.clone();
        collapse_tool_results_over_limit(&mut messages, 50);
        rewrites += usize::from(rewrote_prefix(&before, &messages));
    }
    assert_eq!(rewrites, 1);
}

#[test]
fn the_message_count_dial_collapses_down_to_its_low_water_mark_when_crossed() {
    let mut messages = session(8);
    assert_eq!(
        collapse_conversation_messages_over_limit(&mut messages, 8, 0),
        0
    );

    messages.push(turn_message(9));
    assert_eq!(
        collapse_conversation_messages_over_limit(&mut messages, 8, 0),
        3
    );
    let live = messages
        .iter()
        .filter(|m| m.role == Role::Assistant && !m.is_collapsed)
        .count();
    assert_eq!(live, low_water(8));

    messages.push(turn_message(10));
    assert_eq!(
        collapse_conversation_messages_over_limit(&mut messages, 8, 0),
        0,
        "the next message lands in the headroom"
    );
}

/// An assistant turn that called `calls` tools, followed by their results.
fn tool_turn(first: u32, calls: u32) -> Vec<Message> {
    let mut turn = vec![Message::assistant("calling tools", vec![])];
    turn.extend((first..first + calls).map(spilled_tool_result));
    turn
}

#[test]
fn the_tool_count_dial_never_collapses_results_the_model_has_not_seen() {
    // Limit 10: two older results, then one turn returning nine in parallel.
    // Crossing asks for 3 (11 down to 8), but only the two older results
    // were seen by the model; the fresh nine stay in full.
    let mut messages = tool_turn(0, 1);
    messages.extend(tool_turn(1, 1));
    messages.extend(tool_turn(2, 9));

    assert_eq!(collapse_tool_results_over_limit(&mut messages, 10), 2);
    let fresh = &messages[messages.len() - 9..];
    assert!(fresh.iter().all(|m| !m.is_collapsed), "fresh results stay");
    assert!(messages[1].is_collapsed && messages[3].is_collapsed);
}

/// The #2213 review probe: the prompt, an old seen result, the assistant's
/// tool call and the fresh result it has not seen yet.
fn in_flight_tool_call() -> Vec<Message> {
    let mut old = spilled_tool_result(1);
    old.content = "old ".repeat(40);
    let call = crate::domain::message::ToolCall {
        id: "call-2".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    };
    let mut fresh = spilled_tool_result(2);
    fresh.content = "fresh ".repeat(200);
    let mut messages = vec![
        Message::user("current prompt"),
        Message::assistant("reading", vec![]),
        old,
        Message::assistant("calling bash", vec![call]),
        fresh,
    ];
    for (msg, turn) in messages[1..].iter_mut().zip([1, 1, 2, 2]) {
        msg.turn = Some(turn);
    }
    messages
}

#[test]
fn the_ladder_never_stubs_a_result_the_model_has_not_seen() {
    let mut messages = in_flight_tool_call();
    let budget = estimate_total_tokens(&messages) - 20;

    enforce_context_ceiling_ladder(&mut messages, budget, 0);

    assert!(messages[2].is_collapsed, "the seen result is stubbed");
    assert!(!messages[4].is_collapsed, "the fresh result stays in full");
    assert!(messages[4].content.starts_with("fresh "));
}

#[test]
fn the_drop_rung_keeps_the_in_flight_tool_call_and_its_result() {
    let mut messages = in_flight_tool_call();

    let outcome = enforce_context_ceiling_ladder(&mut messages, 10, 0);

    assert!(outcome.over_budget, "the in-flight pair alone is over 10");
    let tail = &messages[messages.len() - 2..];
    assert_eq!(tail[0].tool_calls.len(), 1, "the tool call stays");
    assert!(tail[1].content.starts_with("fresh "), "its result stays");
    assert!(!tail[1].is_collapsed);
}

/// Review 3 M3 (#2226): a long nudge phase pushes the context past its
/// ceiling. The ladder may stub the agent's answer, but never removes it,
/// so the answer stays the report every history page names, and its text
/// stays recallable by id.
#[test]
fn the_ceiling_never_removes_the_agents_report() {
    use crate::application::sessions::history_paging::newest_window;
    use crate::domain::turn_origin::{TurnOrigin, instruction, progress_nudge};
    let mut answer = Message::assistant(format!("ANSWER {}", "a".repeat(400)), vec![]);
    answer.turn = Some(1);
    answer.turn_origin = TurnOrigin::Instruction;
    answer.spill_id = Some("turn1:msg:assistant".into());
    let answer_id = answer.id();
    let mut messages = vec![instruction("task".into()), answer];
    for turn in 2..=40 {
        let mut nudge = progress_nudge("Workflow incomplete.".into());
        nudge.turn = Some(turn);
        let mut reply = turn_message(turn);
        reply.turn_origin = TurnOrigin::ProgressNudge;
        messages.extend([nudge, reply]);
    }
    messages.push(progress_nudge("Workflow incomplete.".into()));
    let budget = estimate_total_tokens(&messages) / 4;

    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 2);

    assert!(outcome.dropped > 0, "the ladder removed messages");
    let kept = messages.iter().find(|m| m.id() == answer_id);
    assert!(kept.is_some(), "the answer is never removed");
    let page = newest_window(&messages, "", 4);
    assert_eq!(
        page.report.map(|report| report.id),
        Some(answer_id.to_string()),
        "the page still names the answer, not a nudge reply"
    );
}

/// A long nudge phase after `answer`, then a nudge in flight.
fn nudged_after(answer: Message, turns: u32) -> Vec<Message> {
    use crate::domain::turn_origin::{TurnOrigin, instruction, progress_nudge};
    let mut messages = vec![instruction("task".into()), answer];
    for turn in 2..=turns {
        let mut nudge = progress_nudge("Workflow incomplete.".into());
        nudge.turn = Some(turn);
        let mut reply = turn_message(turn);
        reply.turn_origin = TurnOrigin::ProgressNudge;
        messages.extend([nudge, reply]);
    }
    messages.push(progress_nudge("Workflow incomplete.".into()));
    messages
}

/// Review 4 probe C1 (M2): a report that was never spilled cannot be
/// stubbed, only removed; keeping it would hold the context over its
/// ceiling for good, so the ceiling may remove it and is met.
#[test]
fn r4_c1_an_unspilled_huge_report_never_holds_the_ceiling_unmet() {
    let mut answer = Message::assistant(format!("ANSWER {}", "a".repeat(1_000_000)), vec![]);
    answer.turn = Some(1);
    answer.turn_origin = crate::domain::turn_origin::TurnOrigin::Instruction;
    answer.spill_id = None;
    let mut messages = nudged_after(answer, 40);
    let budget = 50_000;
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 2);
    assert!(!outcome.over_budget, "the ceiling is met");
    assert!(estimate_total_tokens(&messages) <= budget);
    let again = enforce_context_ceiling_ladder(&mut messages, budget, 2);
    assert!(!again.over_budget && again.dropped == 0);
}

/// Review 4 probe C2 (L1), #2246 review finding 4: the report kept is one
/// of a finished turn, never a reply of the turn in flight, and it is kept
/// over newer messages. Here the finished report is older than every nudge
/// reply (an unmarked answer outranks them), and the in-flight reply would
/// outrank it were it a candidate: the ceiling must keep the old answer
/// while it removes the newer nudge replies.
#[test]
fn r4_c2_the_kept_report_is_of_a_finished_turn() {
    use crate::domain::turn_origin::{TurnOrigin, progress_nudge};
    let mut answer = Message::assistant(format!("LEGACY {}", "a".repeat(400)), vec![]);
    answer.turn = Some(1);
    answer.spill_id = Some("turn1:msg:legacy".into());
    let answer_id = answer.id();
    let mut messages = vec![Message::user("task"), answer];
    let mut nudge_replies = Vec::new();
    for turn in 2..=40 {
        let mut reply = turn_message(turn);
        reply.turn_origin = TurnOrigin::ProgressNudge;
        nudge_replies.push(reply.id());
        messages.extend([progress_nudge("Workflow incomplete.".into()), reply]);
    }
    messages.push(progress_nudge("Workflow incomplete.".into()));
    let mut status = turn_message(1);
    status.content = format!("in flight {}", "s".repeat(400));
    messages.push(status);
    let budget = estimate_total_tokens(&messages) / 4;
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 1);
    assert!(outcome.dropped > 0);
    let kept = messages.iter().find(|m| m.id() == answer_id);
    assert!(kept.is_some(), "the finished report is kept");
    assert!(
        nudge_replies
            .iter()
            .any(|id| messages.iter().all(|m| m.id() != *id)),
        "newer nudge replies were removed while the older report stayed"
    );
}

/// #2246 review finding 2: a harness note between the task and the answer
/// (the child waited for a sub-agent) opens the answer's turn. Through the
/// nudge phase after it, the answer is stubbed, never removed.
#[test]
fn a_note_before_the_answer_never_exposes_it_to_removal() {
    use crate::domain::turn_origin::{TurnOrigin, harness_note, instruction};
    let mut answer = Message::assistant(format!("ANSWER {}", "a".repeat(400)), vec![]);
    answer.turn = Some(1);
    answer.turn_origin = TurnOrigin::Instruction;
    answer.spill_id = Some("turn1:msg:assistant".into());
    let answer_id = answer.id();
    let mut messages = nudged_after(answer, 40);
    let note = harness_note("<subagent_notification/>".into(), &messages[..1]);
    assert_eq!(note.turn_origin, TurnOrigin::Instruction);
    messages.insert(1, note);
    assert_eq!(messages[0].content, instruction("task".into()).content);
    let budget = estimate_total_tokens(&messages) / 4;
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert!(outcome.dropped > 0, "the ladder removed messages");
    let kept = messages.iter().find(|m| m.id() == answer_id);
    assert!(
        kept.is_some_and(|m| m.is_collapsed),
        "stubbed, never removed"
    );
}

/// #2246 review finding 2: a transcript whose latest turn is finished (a
/// resumed session pruned before its next prompt) keeps that turn's answer,
/// opened by a note, as a stub rather than removing it.
#[test]
fn a_finished_latest_turn_keeps_its_answer_under_the_ceiling() {
    use crate::domain::turn_origin::{TurnOrigin, harness_note, instruction};
    let mut messages = vec![instruction("task".into())];
    for turn in 1..=40 {
        let mut step = turn_message(turn);
        step.turn_origin = TurnOrigin::Instruction;
        messages.push(step);
    }
    messages.push(harness_note("<subagent_notification/>".into(), &messages));
    let mut answer = turn_message(1);
    answer.content = format!("ANSWER {}", "a".repeat(400));
    answer.turn_origin = TurnOrigin::Instruction;
    let answer_id = answer.id();
    messages.push(answer);
    let outcome = enforce_context_ceiling_ladder(&mut messages, 1, 0);
    assert!(outcome.dropped > 0, "the ladder removed messages");
    let kept = messages.iter().find(|m| m.id() == answer_id);
    assert!(
        kept.is_some_and(|m| m.is_collapsed),
        "stubbed, never removed"
    );
}
