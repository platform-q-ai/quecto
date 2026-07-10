//! #1072 steps: mid-run pruning vs positional watermarks.
//!
//! These scenarios pin the behavioural acceptance criteria of issue #1072:
//! a run whose history is demoted or shrunk mid-run by the #1046 ladder must
//! still (a) report exactly the messages it appended (the per-run ledger,
//! never a positional slice) and (b) mark the durable prefix dirty so
//! persistence reconciles — including when demotion is a pure in-place stub
//! collapse that changes no message identity.

use super::*;
use quecto::application::agent_loop::AgentLoopConfig;

fn push_history_turns(world: &mut QuectoWorld, count: usize, content: &str) {
    let start = world
        .watermark_history
        .iter()
        .filter_map(|m| m.turn)
        .max()
        .unwrap_or(0)
        + 1;
    for i in 0..count as u32 {
        let turn = start + i;
        let mut msg = Message::assistant(content, vec![]);
        msg.turn = Some(turn);
        msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
        world.watermark_history.push(msg);
    }
}

// ─── Givens ──────────────────────────────────────────────────────────────────

#[given(
    expr = "a spilled conversation history of {int} prior turns each exceeding the pruning budget"
)]
fn given_big_spilled_history(world: &mut QuectoWorld, count: usize) {
    // ~2400 ASCII chars ≈ 600 tokens per message — each alone over the small
    // budgets these scenarios configure.
    let big = "lorem ipsum dolor sit amet ".repeat(90);
    push_history_turns(world, count, &big);
}

#[given(expr = "a spilled conversation history of {int} further small prior turns")]
fn given_small_spilled_history(world: &mut QuectoWorld, count: usize) {
    push_history_turns(world, count, "a small earlier reply");
}

#[given(expr = "the pruning agent context budget is {int} tokens")]
fn given_pruning_budget(world: &mut QuectoWorld, budget: usize) {
    world.watermark_budget = budget;
}

#[given("the provider rejects the next request as malformed")]
fn given_provider_rejects_malformed(world: &mut QuectoWorld) {
    let mock = super::agent_loop_steps::ensure_mock_llm(world);
    mock.push_error_response(
        "provider error (400): invalid_request_error: tool_use input is malformed",
    );
}

// ─── When ────────────────────────────────────────────────────────────────────

#[when(expr = "the user sends {string} through the pruning agent")]
fn when_user_sends_through_pruning_agent(world: &mut QuectoWorld, text: String) {
    let provider = world.mock_llm.clone().expect("mock LLM not configured") as Arc<dyn LlmProvider>;
    let mut registry = ToolRegistryImpl::new();
    for tool in world.mock_tools.values() {
        registry.register(tool.clone());
    }
    let budget = if world.watermark_budget == 0 {
        190_000
    } else {
        world.watermark_budget
    };
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        // No spill store: the pre-run history already carries spill_ids (so
        // ladder rung 1 can stub in place) and a store would insert a spill
        // manifest message, perturbing the prefix these scenarios pin.
        spill_store: None,
        session_key: "bdd-1072".to_string(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: budget,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });

    let mut messages = world.watermark_history.clone();
    world.watermark_pre_run = messages.clone();
    messages.push(Message::user(text));
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages));
    world.agent_result = Some(result.expect("agent process failed"));
    world.watermark_post_run = messages;
}

// ─── Thens ───────────────────────────────────────────────────────────────────

#[then("fewer pre-run messages survive than were present before the run")]
fn then_pre_run_messages_shrank(world: &mut QuectoWorld) {
    let pre = &world.watermark_pre_run;
    // The live #1072 panic shape requires a PHYSICAL shrink: post-run length
    // strictly below the pre-turn length (pre-run history + the pushed
    // prompt). In-place stub demotion alone must NOT satisfy this Then — it
    // would let the scenario pass without exercising positional-slice safety.
    assert!(
        world.watermark_post_run.len() < pre.len() + 1,
        "mid-run pruning must shrink the conversation below its pre-turn \
         length: {} pre-turn (incl. prompt) -> {} post-run",
        pre.len() + 1,
        world.watermark_post_run.len()
    );
    let survivors = world
        .watermark_post_run
        .iter()
        .filter(|m| {
            pre.iter()
                .any(|p| p.content == m.content && p.turn == m.turn)
        })
        .count();
    assert!(
        survivors < pre.len(),
        "mid-run pruning should have removed pre-run messages: {survivors} of {} survive",
        pre.len()
    );
}

#[then(
    expr = "the run's appended messages are exactly the assistant tool call, the tool result and the final reply {string}"
)]
fn then_appended_exactly_tool_turn_and_final(world: &mut QuectoWorld, final_reply: String) {
    let result = world.agent_result.as_ref().expect("no agent result");
    let appended = &result.appended_messages;
    let roles: Vec<Role> = appended.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![Role::Assistant, Role::Tool, Role::Assistant],
        "appended ledger must carry exactly the run's own messages, got roles {roles:?} with contents {:?}",
        appended.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    assert!(
        !appended[0].tool_calls.is_empty(),
        "first appended message must be the assistant tool call"
    );
    assert_eq!(
        appended[2].content, final_reply,
        "last appended message must be the final assistant reply"
    );
}

#[then("the agent result marks the durable prefix dirty")]
fn then_agent_result_marks_prefix_dirty(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        result.durable_prefix_dirty,
        "mid-run demotion (stub or drop) must mark the durable prefix dirty"
    );
}

#[then("the oversized pre-run messages are collapsed to recall stubs in place")]
fn then_oversized_collapsed_in_place(world: &mut QuectoWorld) {
    let pre = &world.watermark_pre_run;
    let big: Vec<&Message> = pre.iter().filter(|m| m.content.len() > 1000).collect();
    assert!(!big.is_empty(), "scenario needs oversized pre-run messages");
    for original in big {
        let post = world
            .watermark_post_run
            .iter()
            .find(|m| m.turn == original.turn && m.role == original.role)
            .unwrap_or_else(|| panic!("turn {:?} message missing post-run", original.turn));
        assert!(
            post.is_collapsed && post.content.contains("recall("),
            "turn {:?} must be stub-demoted in place, got: {}",
            original.turn,
            post.content
        );
    }
}

#[then("no pre-run message is removed from the conversation")]
fn then_no_pre_run_message_removed(world: &mut QuectoWorld) {
    for original in &world.watermark_pre_run {
        assert!(
            world
                .watermark_post_run
                .iter()
                .any(|m| m.turn == original.turn && m.role == original.role),
            "pre-run message for turn {:?} was removed — this scenario is stub-only",
            original.turn
        );
    }
}

#[then("the run's appended messages include the malformed-request feedback")]
fn then_appended_includes_malformed_feedback(world: &mut QuectoWorld) {
    let result = world.agent_result.as_ref().expect("no agent result");
    assert!(
        result
            .appended_messages
            .iter()
            .any(|m| m.role == Role::User
                && m.content.contains("rejected by the provider as malformed")),
        "the #931 malformed-request feedback message was appended to the \
         conversation mid-run and must appear in the run's appended ledger; \
         got {:?}",
        result
            .appended_messages
            .iter()
            .map(|m| (m.role.clone(), &m.content))
            .collect::<Vec<_>>()
    );
}
