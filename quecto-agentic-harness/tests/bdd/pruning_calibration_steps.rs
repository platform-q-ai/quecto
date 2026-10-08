//! Steps for the calibrated context ceiling (#2212).
//!
//! The ceiling decides on the per-class estimate (prose, digit-bearing runs,
//! non-ASCII), scaled by the provider's own count of the prompt once one is
//! reported. These scenarios run the pruning agent of #1072 over history
//! whose estimate is known, and read which pre-run messages it stubbed.

use super::*;
use quecto::domain::conversation::value_objects::message::UsageInfo;

/// `seq` output joined by spaces (the #2212 QA content): about 2000
/// characters per turn.
fn seq_output(turn: u32) -> String {
    (turn * 10_000..turn * 10_000 + 334)
        .map(|n| format!("{n} "))
        .collect()
}

fn push_spilled_history(world: &mut QuectoWorld, count: u32, content: impl Fn(u32) -> String) {
    for turn in 1..=count {
        let mut msg = Message::assistant(content(turn), vec![]);
        msg.turn = Some(turn);
        msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
        world.watermark_history.push(msg);
    }
}

#[given(expr = "a spilled conversation history of {int} prior turns of seq output")]
fn given_seq_history(world: &mut QuectoWorld, count: u32) {
    push_spilled_history(world, count, seq_output);
}

#[given(expr = "a spilled conversation history of {int} prior turns of prose")]
fn given_prose_history(world: &mut QuectoWorld, count: u32) {
    // About 2000 characters, no digits: 500 tokens by any count.
    push_spilled_history(world, count, |_| {
        "the quick brown fox jumps over the lazy dog ".repeat(45)
    });
}

fn push_tool_call(world: &mut QuectoWorld, tool_name: String, usage: Option<UsageInfo>) {
    let mock = super::agent_loop_steps::ensure_mock_llm(world);
    mock.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{tool_name}"),
            name: tool_name,
            arguments: "{}".to_string(),
        }],
        usage,
        stop_reason: None,
        thinking_blocks: vec![],
    });
}

#[given(expr = "the LLM returns a tool call for {string} reporting {int} context tokens")]
fn given_llm_tool_call_reporting(world: &mut QuectoWorld, tool_name: String, reported: u32) {
    let usage = UsageInfo {
        prompt_tokens: reported,
        completion_tokens: 5,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: Some(reported),
        cost: None,
    };
    push_tool_call(world, tool_name, Some(usage));
}

#[given(expr = "the LLM returns a tool call for {string} reporting no usage")]
fn given_llm_tool_call_without_usage(world: &mut QuectoWorld, tool_name: String) {
    push_tool_call(world, tool_name, None);
}

/// The pre-run messages no longer in context in full: archived by a cut,
/// or stubbed or dropped by the emergency ladder (#2414).
fn pre_run_stubs(world: &QuectoWorld) -> usize {
    world
        .watermark_pre_run
        .iter()
        .filter(|original| {
            !world.watermark_post_run.iter().any(|m| {
                m.turn == original.turn
                    && m.role == original.role
                    && m.content == original.content
                    && !m.is_collapsed
            })
        })
        .count()
}

#[then("some pre-run messages are archived or stubbed")]
fn then_some_pre_run_stubbed(world: &mut QuectoWorld) {
    assert!(
        pre_run_stubs(world) > 0,
        "the transcript is over the budget: the run must prune it"
    );
}

#[then("no pre-run message is archived or stubbed")]
fn then_no_pre_run_stubbed(world: &mut QuectoWorld) {
    assert_eq!(pre_run_stubs(world), 0, "the transcript fits the budget");
}

#[then("a flat four-characters-per-token count of the pre-run history fits the budget")]
fn then_ascii_quarter_fits(world: &mut QuectoWorld) {
    let flat: usize = world
        .watermark_pre_run
        .iter()
        .map(|m| m.content.len().div_ceil(4))
        .sum();
    let dense =
        quecto::application::context_pruning::estimate_total_tokens(&world.watermark_pre_run);
    // Room left for the prompt and the tool definitions.
    assert!(
        flat * 10 <= world.watermark_budget * 9,
        "ASCII/4 counts {flat} of {}: that estimate would not have pruned",
        world.watermark_budget
    );
    assert!(
        dense > world.watermark_budget,
        "the per-class estimate is {dense}"
    );
}
