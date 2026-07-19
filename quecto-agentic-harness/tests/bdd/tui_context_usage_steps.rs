//! Steps for `tui_context_usage.feature`.
//!
//! Two real production surfaces are exercised:
//!  * The TUI footer context gauge, driven by real `TurnEnd` events and the
//!    real `get_session_stats` response path through the headless harness.
//!  * The real agent loop, to prove `AgentResult.context_tokens` reflects the
//!    provider-reported context occupancy when usage is available (with the
//!    active pruned-conversation estimate kept separate for pruning).

use super::*;
use quecto::application::agent_loop::AgentLoopConfig;
use quecto::application::context_pruning::estimate_total_tokens;
use quecto::domain::message::UsageInfo;
use quecto::interface::cli::protocol::SessionStats as WireSessionStats;
use quecto::interface::cli::uds_session::{AgentSession, compute_session_stats_with_usage};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::TuiHarness;

const WINDOW: u64 = 200_000;
const AGENT_WINDOW: usize = 190_000;
const PROVIDER_INPUT_TOKENS: u32 = 280_000;

fn with_harness<R>(world: &mut QuectoWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

/// Drive a real TurnEnd carrying inline context usage into the TUI footer.
fn turn_end_with_context(world: &mut QuectoWorld, context_tokens: u64, window: u64) {
    with_harness(world, |h| {
        h.event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "contextTokens": context_tokens,
                "maxContextTokens": window,
            }),
        });
    });
}

/// Run one real agent-loop turn whose provider reports input usage above the
/// configured context window, and stash the result for later assertions.
fn run_context_turn(world: &mut QuectoWorld, streaming: bool) {
    let provider = Arc::new(MockLlmProvider::new());
    provider.push_response(LlmResponse {
        content: Some("hello".to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: PROVIDER_INPUT_TOKENS,
            completion_tokens: 7,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_tokens: None,
            cost: None,
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    });
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider as Arc<dyn LlmProvider>,
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: AGENT_WINDOW,
        progress_callback: None,
        streaming,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut messages = vec![Message::user("Hi")];
    let result = rt
        .block_on(agent.process(&mut messages))
        .expect("agent run should succeed");
    world.tui_ctx_window = Some(agent.max_context_tokens());
    world.tui_ctx_messages = messages;
    world.tui_ctx_agent_result = Some(result);
}

// ═══════════════════════════════════════════════════════════════════════════
//  Inline TurnEnd footer scenarios
// ═══════════════════════════════════════════════════════════════════════════

#[given("the agent completes a response using 5000 input tokens")]
fn given_5000_tokens(world: &mut QuectoWorld) {
    // Recorded implicitly; the TurnEnd in the When carries the value.
    world.tui_ctx_window = Some(WINDOW as usize);
}

#[given("the context window is 200k tokens")]
fn given_window_200k(world: &mut QuectoWorld) {
    world.tui_ctx_window = Some(WINDOW as usize);
}

#[when("the TurnEnd event includes usage data")]
fn when_turn_end_usage(world: &mut QuectoWorld) {
    turn_end_with_context(world, 5_000, WINDOW);
}

#[then(regex = r#"^the footer should show "2\.5%/200k"$"#)]
fn then_footer_shows_2_5(world: &mut QuectoWorld) {
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        footer.contains("2.5%") && footer.contains("200k"),
        "footer should show 2.5% of the 200k window, got:\n{footer}"
    );
}

#[given("the agent has processed multiple turns")]
fn given_multiple_turns(world: &mut QuectoWorld) {
    // First turn establishes an initial active conversation size.
    turn_end_with_context(world, 5_000, WINDOW);
}

#[when("each TurnEnd event includes current active conversation size")]
fn when_each_turn_end(world: &mut QuectoWorld) {
    // A later, larger active conversation size replaces (not accumulates) the gauge.
    turn_end_with_context(world, 8_000, WINDOW);
}

#[then(
    "the footer should reflect the latest active conversation size, not cumulative provider input"
)]
fn then_reflects_latest(world: &mut QuectoWorld) {
    let footer = with_harness(world, |h| h.bottom_stack());
    // 8000/200000 = 4.0% (latest), NOT 13000/200000 = 6.5% (cumulative).
    assert!(
        footer.contains("4.0%"),
        "footer should show the latest 4.0%, got:\n{footer}"
    );
    assert!(
        !footer.contains("6.5%"),
        "footer must not accumulate turns into 6.5%, got:\n{footer}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Provider usage vs conversation-size gauge (real agent loop)
// ═══════════════════════════════════════════════════════════════════════════

#[given("a completed agent turn reports provider input usage above the configured context window")]
fn given_nonstreaming_turn(world: &mut QuectoWorld) {
    run_context_turn(world, false);
    let result = world.tui_ctx_agent_result.as_ref().unwrap();
    assert!(
        u64::from(result.input_tokens) > world.tui_ctx_window.unwrap() as u64,
        "provider input usage should exceed the window"
    );
}

#[given(
    "a completed streamed agent turn reports provider input usage above the configured context window"
)]
fn given_streaming_turn(world: &mut QuectoWorld) {
    run_context_turn(world, true);
    let result = world.tui_ctx_agent_result.as_ref().unwrap();
    assert!(
        u64::from(result.input_tokens) > world.tui_ctx_window.unwrap() as u64,
        "provider input usage should exceed the window"
    );
}

#[given("the active pruned conversation estimate remains below the configured context window")]
fn given_estimate_below_window(world: &mut QuectoWorld) {
    let estimate = estimate_total_tokens(&world.tui_ctx_messages);
    let window = world.tui_ctx_window.expect("window");
    assert!(
        estimate < window,
        "the active pruned estimate ({estimate}) should stay below the window ({window})"
    );
}

#[when("the agent emits TurnEnd and session stats for the TUI")]
fn when_emits_turn_end_and_stats(world: &mut QuectoWorld) {
    let result = world.tui_ctx_agent_result.as_ref().expect("agent result");
    let window = world.tui_ctx_window.expect("window");
    // Mirror the production emit mapping (uds_cancel.rs): contextTokens comes
    // from the agent's provider-truth gauge value, and maxContextTokens remains
    // the enforced pruning/window budget.
    turn_end_with_context(world, result.context_tokens as u64, window as u64);
}

#[then("contextTokens should equal the provider-reported context occupancy")]
fn then_context_equals_provider_truth(world: &mut QuectoWorld) {
    let result = world.tui_ctx_agent_result.as_ref().expect("agent result");
    let estimate = estimate_total_tokens(&world.tui_ctx_messages);
    assert_eq!(
        result.context_tokens, PROVIDER_INPUT_TOKENS as usize,
        "AgentResult.context_tokens must use provider-reported context occupancy, not the active-message estimate ({estimate})"
    );
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        footer.contains("147.4%") || footer.contains("280k/190k"),
        "the gauge must reflect the over-window provider-reported occupancy, got:\n{footer}"
    );
}

#[then("maxContextTokens should equal the configured context window")]
fn then_max_equals_window(world: &mut QuectoWorld) {
    assert_eq!(
        world.tui_ctx_window,
        Some(AGENT_WINDOW),
        "maxContextTokens should equal the configured context window"
    );
}

#[then("the provider token usage should drive both the context gauge and usage totals")]
fn then_provider_usage_drives_gauge_and_totals(world: &mut QuectoWorld) {
    let result = world.tui_ctx_agent_result.as_ref().expect("agent result");
    assert_eq!(
        result.input_tokens, PROVIDER_INPUT_TOKENS,
        "the provider input usage should remain intact in the usage totals"
    );
    assert_eq!(
        result.context_tokens as u64,
        u64::from(result.input_tokens),
        "the context gauge should match provider-reported occupancy"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Session stats accumulation
// ═══════════════════════════════════════════════════════════════════════════

#[given("multiple LLM calls return input, output, cache, and cost usage")]
fn given_multiple_llm_calls(world: &mut QuectoWorld) {
    let mut session = AgentSession::new("test-model".to_string(), "cli:default".to_string());
    // Two turns' worth of provider usage accumulate into the session totals.
    session.record_usage(1_200, 340, 500, 20, 4_200);
    session.record_usage(800, 160, 300, 10, 1_800);
    let messages = vec![
        Message::user("first"),
        Message::assistant("reply one", vec![]),
        Message::user("second"),
        Message::assistant("reply two", vec![]),
    ];
    let stats: WireSessionStats = compute_session_stats_with_usage(
        "cli:default",
        &messages,
        session.usage_snapshot(),
        1_234,
        AGENT_WINDOW,
    );
    world.tui_session_stats_json = Some(serde_json::to_value(&stats).expect("serialize stats"));
}

#[when("the TUI requests session stats")]
fn when_tui_requests_stats(world: &mut QuectoWorld) {
    let data = world
        .tui_session_stats_json
        .clone()
        .expect("session stats json");
    with_harness(world, |h| {
        h.event(Event::Response {
            id: Some("stats".to_string()),
            command: "get_session_stats".to_string(),
            success: true,
            data: Some(data),
            error: None,
        });
    });
}

#[then("the stats response should include non-zero token totals and cost")]
fn then_stats_non_zero(world: &mut QuectoWorld) {
    let data = world.tui_session_stats_json.as_ref().expect("stats json");
    let total = data
        .get("tokens")
        .and_then(|t| t.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cost = data.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
    assert!(total > 0, "token totals should be non-zero, got {total}");
    assert!(cost > 0.0, "cost should be non-zero, got {cost}");
}

#[then("the TUI should display those token totals and cost instead of zeros")]
fn then_tui_displays_totals(world: &mut QuectoWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    // show_session_stats renders "... Tokens: ↑2000 ↓500 | Cost: $0.0060".
    assert!(
        frame.contains("↑2000") && frame.contains("↓500"),
        "the TUI should display accumulated input/output token totals, frame:\n{frame}"
    );
    assert!(
        frame.contains("$0.0060"),
        "the TUI should display the accumulated cost, frame:\n{frame}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Warning / error colours
// ═══════════════════════════════════════════════════════════════════════════

#[given("context usage exceeds 70%")]
fn given_usage_over_70(world: &mut QuectoWorld) {
    // 150000/200000 = 75%.
    turn_end_with_context(world, 150_000, WINDOW);
}

#[given("context usage exceeds 90%")]
fn given_usage_over_90(world: &mut QuectoWorld) {
    // 190000/200000 = 95%.
    turn_end_with_context(world, 190_000, WINDOW);
}

#[when("the footer renders")]
fn when_footer_renders(world: &mut QuectoWorld) {
    // Force a raw (ANSI-preserving) render and stash it for the colour assertion.
    let raw = with_harness(world, |h| h.full_frame_raw());
    world.tui_footer_streaming_render = vec![raw];
}

#[then("the usage should be displayed in warning color")]
fn then_warning_color(world: &mut QuectoWorld) {
    let raw = world.tui_footer_streaming_render.join("");
    assert!(
        raw.contains("\u{1b}[33m"),
        "context usage over 70% should render in the yellow warning colour"
    );
}

#[then("the usage should be displayed in error color")]
fn then_error_color(world: &mut QuectoWorld) {
    let raw = world.tui_footer_streaming_render.join("");
    assert!(
        raw.contains("\u{1b}[31m"),
        "context usage over 90% should render in the red error colour"
    );
}
