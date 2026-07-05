//! Step definitions for `tui_app_behaviors.feature`.
//!
//! These scenarios exercise the real App routing paths via the public headless
//! TUI harness exposed under the `test-harness` feature. They intentionally
//! assert observable frames, notifications, and serialized commands rather than
//! private fields.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::interface::keys::Key;

async fn build_fresh_harness() -> TuiHarness {
    TuiHarness::new().await
}

fn init_fresh(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(build_fresh_harness());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_last_commands.clear();
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

fn json_field(line: &str, field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.get(field)?.as_str().map(str::to_string)
}

fn command_of_type<'a>(commands: &'a [String], ty: &str) -> Option<&'a String> {
    commands
        .iter()
        .find(|line| json_field(line, "type").as_deref() == Some(ty))
}

fn parse_k_tokens(label: &str) -> u64 {
    label
        .strip_suffix('k')
        .unwrap_or(label)
        .parse::<u64>()
        .unwrap_or_else(|e| panic!("invalid context label {label:?}: {e}"))
        * 1_000
}

#[given("a fresh TUI app harness")]
fn given_fresh_harness(world: &mut TuiWorld) {
    init_fresh(world);
}

#[given("the master assistant is currently streaming")]
fn given_master_streaming(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(Event::AgentStart);
        h.event(Event::Token {
            token: "working".into(),
        });
    });
}

#[when(
    expr = "a quiet session stats footer response arrives with cost {string} and context {string}"
)]
fn when_quiet_stats_arrives(world: &mut TuiWorld, cost_label: String, context_label: String) {
    let cost = cost_label
        .trim_start_matches('$')
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("invalid cost label {cost_label:?}: {e}"));
    let context_tokens = parse_k_tokens(&context_label);
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("stats-footer".into()),
            command: "get_session_stats".into(),
            success: true,
            data: Some(serde_json::json!({
                "sessionKey": "cli:default",
                "totalMessages": 3,
                "tokens": { "input": 10, "output": 20 },
                "cost": cost,
                "contextTokens": context_tokens,
                "maxContextTokens": 100_000,
            })),
            error: None,
        });
    });
}

#[when(expr = "a model switch response fails with {string}")]
fn when_model_switch_fails(world: &mut TuiWorld, error: String) {
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("sm".into()),
            command: "set_model".into(),
            success: false,
            data: None,
            error: Some(error),
        });
    });
}

#[when("I request rewind history with two prior user turns")]
fn when_request_rewind_history(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Escape);
        h.press(Key::Escape);
    });
    let commands = drain_commands(world);
    let get_messages = command_of_type(&commands, "get_messages")
        .unwrap_or_else(|| panic!("rewind open should send get_messages, got {commands:?}"));
    let id = json_field(get_messages, "id")
        .unwrap_or_else(|| panic!("get_messages should carry rewind request id: {get_messages}"));

    drive(world, |h| {
        h.event(Event::Response {
            id: Some(id),
            command: "get_messages".into(),
            success: true,
            data: Some(serde_json::json!({
                "messages": [
                    { "role": "user", "content": "first prompt" },
                    { "role": "assistant", "content": "first answer" },
                    { "role": "user", "content": "most recent prompt" },
                    { "role": "assistant", "content": "most recent answer" }
                ]
            })),
            error: None,
        });
    });
    world.tui_last_commands = commands;
}

#[when("I choose the most recent rewind target")]
fn when_choose_rewind_target(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain_commands(world);
}

#[when("the rewind apply response succeeds")]
fn when_rewind_apply_succeeds(world: &mut TuiWorld) {
    let rewind = command_of_type(&world.tui_last_commands, "rewind_to").unwrap_or_else(|| {
        panic!(
            "expected stored rewind_to command, got {:?}",
            world.tui_last_commands
        )
    });
    let id = json_field(rewind, "id")
        .unwrap_or_else(|| panic!("rewind_to should carry an id: {rewind}"));
    drive(world, |h| {
        h.event(Event::Response {
            id: Some(id),
            command: "rewind_to".into(),
            success: true,
            data: None,
            error: None,
        });
    });
    let mut commands = world.tui_last_commands.clone();
    commands.extend(drain_commands(world));
    world.tui_last_commands = commands;
}

#[when(expr = "I submit the master prompt {string}")]
fn when_submit_master_prompt(world: &mut TuiWorld, prompt: String) {
    drive(world, |h| {
        h.submit(&prompt);
    });
    world.tui_last_commands = drain_commands(world);
}

#[when(expr = "sub-agent {string} streams token {string}")]
fn when_subagent_streams_token(world: &mut TuiWorld, id: String, token: String) {
    drive(world, |h| {
        h.route(&id, Event::Token { token });
    });
}

#[when(expr = "sub-agent {string} reports model {string} and context {string}")]
fn when_subagent_reports_state(
    world: &mut TuiWorld,
    id: String,
    model: String,
    context_label: String,
) {
    let context_tokens = parse_k_tokens(&context_label);
    drive(world, |h| {
        h.route(
            &id,
            Event::Response {
                id: None,
                command: "get_state".into(),
                success: true,
                data: Some(serde_json::json!({
                    "model": model,
                    "maxContextTokens": 100_000,
                })),
                error: None,
            },
        );
        h.route(
            &id,
            Event::TurnEnd {
                message: serde_json::json!({
                    "contextTokens": context_tokens,
                    "maxContextTokens": 100_000,
                }),
            },
        );
    });
}

#[then(expr = "the footer shows cost {string} and context {string}")]
fn then_footer_shows_cost_and_context(world: &mut TuiWorld, cost: String, context: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&cost) && frame.contains(&context),
        "footer should show cost {cost:?} and context {context:?}, got:\n{frame}"
    );
}

#[then("the chat transcript does not show a session stats notification")]
fn then_no_session_stats_notification(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        !frame.contains("Session: cli:default") && !frame.contains("Tokens: ↑10 ↓20"),
        "stats-footer response must not add the verbose stats chat line, got:\n{frame}"
    );
}

#[then(expr = "the app notification includes {string}")]
fn then_notification_includes(world: &mut TuiWorld, expected: String) {
    let notification = drive(world, |h| h.notification_text());
    assert!(
        notification.contains(&expected),
        "notification should include {expected:?}, got:\n{notification}"
    );
}

#[then("a rewind command is sent for the most recent user turn")]
fn then_rewind_command_sent(world: &mut TuiWorld) {
    let rewind = command_of_type(&world.tui_last_commands, "rewind_to").unwrap_or_else(|| {
        panic!(
            "expected rewind_to command, got {:?}",
            world.tui_last_commands
        )
    });
    let value: serde_json::Value = serde_json::from_str(rewind).expect("rewind command json");
    assert_eq!(
        value.get("messageIndex").and_then(|v| v.as_u64()),
        Some(2),
        "the default selected target should be the most recent user turn: {rewind}"
    );
}

#[then("a rewind refresh command is sent")]
fn then_rewind_refresh_sent(world: &mut TuiWorld) {
    let refresh = world.tui_last_commands.iter().any(|line| {
        json_field(line, "type").as_deref() == Some("get_messages")
            && json_field(line, "id").as_deref() == Some("rewind-refresh")
    });
    assert!(
        refresh,
        "successful rewind should request a conversation refresh, got {:?}",
        world.tui_last_commands
    );
}

#[then(expr = "the master prompt command includes streaming behavior {string}")]
fn then_master_prompt_has_streaming_behavior(world: &mut TuiWorld, behavior: String) {
    let prompt = command_of_type(&world.tui_last_commands, "prompt")
        .unwrap_or_else(|| panic!("expected prompt command, got {:?}", world.tui_last_commands));
    let value: serde_json::Value = serde_json::from_str(prompt).expect("prompt command json");
    assert_eq!(
        value.get("streamingBehavior").and_then(|v| v.as_str()),
        Some(behavior.as_str()),
        "streaming master submit should steer: {prompt}"
    );
}

#[then(expr = "the master chat shows {string}")]
fn then_master_chat_shows(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&expected),
        "master chat should show {expected:?}, got:\n{frame}"
    );
}

#[then(expr = "the selected sub-agent session shows {string}")]
fn then_selected_subagent_session_shows(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&expected),
        "selected sub-agent session should show {expected:?}, got:\n{frame}"
    );
}

#[then(expr = "the app master session does not show {string}")]
fn then_master_session_does_not_show(world: &mut TuiWorld, unexpected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        !frame.contains(&unexpected),
        "master session must not show sub-agent-only content {unexpected:?}, got:\n{frame}"
    );
}

#[then(expr = "the footer shows the sub-agent model {string} and context {string}")]
fn then_footer_shows_subagent_model_and_context(
    world: &mut TuiWorld,
    model: String,
    context: String,
) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&model) && frame.contains(&context),
        "selected sub-agent footer should show model {model:?} and context {context:?}, got:\n{frame}"
    );
}
