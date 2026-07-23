//! Step definitions for `tui_app_behaviors.feature`.
//!
//! These scenarios exercise the real App routing paths via the public headless
//! TUI harness exposed under the `test-harness` feature. They intentionally
//! assert observable frames, notifications, and serialized commands rather than
//! private fields.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::ansi::strip_ansi;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::interface::keys::Key;
use quecto_tui::interface::utils::visible_width;

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

#[given(expr = "sub-agent {string} uses model {string} with effort {string}")]
fn given_subagent_uses_model_effort(
    world: &mut TuiWorld,
    id: String,
    model: String,
    effort: String,
) {
    drive(world, |h| {
        h.route(
            &id,
            Event::Response {
                id: None,
                command: "get_state".into(),
                success: true,
                data: Some(serde_json::json!({
                    "model": model,
                    "effort": effort,
                    "effortLevels": if model.starts_with("anthropic-api/") {
                        serde_json::json!(["low", "medium", "high", "max"])
                    } else {
                        serde_json::json!(["none", "low", "medium", "high", "xhigh"])
                    },
                    "maxContextTokens": 100_000,
                })),
                error: None,
            },
        );
    });
    world.tui_last_commands.clear();
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

#[when(
    expr = "an interactive session stats response arrives for {string} with cost {string} and tokens {int} input {int} output"
)]
fn when_interactive_stats_arrives(
    world: &mut TuiWorld,
    session_key: String,
    cost_label: String,
    input_tokens: u64,
    output_tokens: u64,
) {
    let cost = cost_label
        .trim_start_matches('$')
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("invalid cost label {cost_label:?}: {e}"));
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("stats-chat".into()),
            command: "get_session_stats".into(),
            success: true,
            data: Some(serde_json::json!({
                "sessionKey": session_key,
                "totalMessages": 9,
                "tokens": { "input": input_tokens, "output": output_tokens },
                "cost": cost,
                "contextTokens": 8_000,
                "maxContextTokens": 100_000,
            })),
            error: None,
        });
    });
}

#[given(expr = "the master chat already contains {string}")]
fn given_master_chat_already_contains(world: &mut TuiWorld, text: String) {
    drive(world, |h| {
        h.add_user_message(&text);
    });
}

#[when("a resumed messages response arrives with a non-array messages field")]
fn when_resumed_messages_non_array(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("resume-messages".into()),
            command: "get_messages".into(),
            success: true,
            data: Some(serde_json::json!({ "messages": "bad" })),
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

#[when("I request the model selector")]
fn when_request_model_selector(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Ctrl('l'));
    });
    world.tui_last_commands = drain_commands(world);
}

#[when(expr = "the model list response contains {string} and {string}")]
fn when_model_list_response_contains(world: &mut TuiWorld, first: String, second: String) {
    let request = command_of_type(&world.tui_last_commands, "list_models").unwrap_or_else(|| {
        panic!(
            "model selector should request list_models, got {:?}",
            world.tui_last_commands
        )
    });
    let id = json_field(request, "id");
    drive(world, |h| {
        h.event(Event::Response {
            id,
            command: "list_models".into(),
            success: true,
            data: Some(serde_json::json!({
                "models": [
                    { "id": first, "provider": "OpenAI API", "auth": "api" },
                    { "id": second, "provider": "Anthropic API", "auth": "api" }
                ]
            })),
            error: None,
        });
    });
}

#[when(expr = "the model list response fails with {string}")]
fn when_model_list_response_fails(world: &mut TuiWorld, error: String) {
    let request = command_of_type(&world.tui_last_commands, "list_models").unwrap_or_else(|| {
        panic!(
            "model selector should request list_models, got {:?}",
            world.tui_last_commands
        )
    });
    let id = json_field(request, "id");
    drive(world, |h| {
        h.event(Event::Response {
            id,
            command: "list_models".into(),
            success: false,
            data: None,
            error: Some(error),
        });
    });
}

#[when(expr = "I filter the model selector with {string}")]
fn when_filter_model_selector(world: &mut TuiWorld, query: String) {
    drive(world, |h| {
        for ch in query.chars() {
            h.press(Key::Char(ch));
        }
    });
}

#[when("I accept the selected model")]
fn when_accept_selected_model(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain_commands(world);
}

// ── Effort control (#1067) ─────────────────────────────────────────────

fn apply_get_state(world: &mut TuiWorld, model: String, effort: serde_json::Value) {
    // Mirror the agent's real get_state shape: it always reports the
    // provider's valid effort vocabulary in `effortLevels` (#1067) — the
    // TUI's single source of truth for validation and the selector.
    let levels: &[&str] = if model.contains("anthropic") || model.contains("claude") {
        &["low", "medium", "high", "max"]
    } else {
        &["none", "low", "medium", "high", "xhigh"]
    };
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("gs".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": model,
                "effort": effort,
                "effortLevels": levels,
            })),
            error: None,
        });
    });
}

/// Trigger form: the arriving state IS the behaviour under test (footer
/// display scenarios).
#[when(expr = "a get_state response arrives with model {string} and effort {string}")]
fn when_get_state_with_effort(world: &mut TuiWorld, model: String, effort: String) {
    apply_get_state(world, model, serde_json::json!(effort));
}

/// Context form: the same state arrival used as a precondition for a later
/// `/effort` action (same handler, Given semantics).
#[given(expr = "the agent reports model {string} with effort {string}")]
fn given_agent_reports_model_with_effort(world: &mut TuiWorld, model: String, effort: String) {
    apply_get_state(world, model, serde_json::json!(effort));
}

/// The agent's real wire shape for a never-set effort is an explicit
/// `"effort": null` (never a missing key).
#[when(expr = "a get_state response arrives with model {string} and a null effort")]
fn when_get_state_with_null_effort(world: &mut TuiWorld, model: String) {
    apply_get_state(world, model, serde_json::Value::Null);
}

#[given(expr = "I have submitted the master prompt {string}")]
fn given_submitted_master_prompt(world: &mut TuiWorld, prompt: String) {
    drive(world, |h| {
        h.submit(&prompt);
    });
    world.tui_last_commands = drain_commands(world);
}

/// Submit a prompt that is handled locally (rejected or selector-opening):
/// uses the NON-polling drain, because the shared submit step's bounded poll
/// blocks a runtime worker for its full window when no command ever arrives,
/// starving concurrent scenarios (and their 3s notification lifetimes).
#[when(expr = "I submit the master prompt {string} expecting no agent command")]
fn when_submit_master_prompt_no_command(world: &mut TuiWorld, prompt: String) {
    drive(world, |h| {
        h.submit(&prompt);
    });
    world.tui_last_commands = drive(world, |h| h.try_drain_commands());
}

/// Open the effort selector overlay. No command drain: bare `/effort` is a
/// purely local action (see the non-polling rationale above).
#[when("I open the effort selector via the /effort prompt")]
#[given("I have opened the effort selector via the /effort prompt")]
fn when_open_effort_selector(world: &mut TuiWorld) {
    drive(world, |h| {
        h.submit("/effort");
    });
    world.tui_last_commands = drive(world, |h| h.try_drain_commands());
}

#[then(expr = "the footer shows effort level {string}")]
fn then_footer_shows_effort(world: &mut TuiWorld, level: String) {
    let frame = drive(world, |h| h.full_frame());
    let needle = format!("effort: {level}");
    assert!(
        frame.contains(&needle),
        "footer should show {needle:?}, got:\n{frame}"
    );
}

/// "default" is deliberately NOT an effort level: the footer's placeholder
/// for the effective config/provider default gets its own step so the
/// level-valued step is never fed out-of-vocabulary values.
#[then("the footer shows the effective default effort")]
fn then_footer_shows_default_effort(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("effort: default"),
        "footer should show the effective default effort, got:\n{frame}"
    );
}

#[then(expr = "a set effort command is sent for {string}")]
fn then_set_effort_command_sent_for(world: &mut TuiWorld, expected: String) {
    let cmd = command_of_type(&world.tui_last_commands, "set_effort").unwrap_or_else(|| {
        panic!(
            "expected set_effort command, got {:?}",
            world.tui_last_commands
        )
    });
    let value: serde_json::Value = serde_json::from_str(cmd).expect("set_effort command json");
    assert_eq!(
        value.get("effort").and_then(|v| v.as_str()),
        Some(expected.as_str()),
        "selected effort should be sent in set_effort command: {cmd}"
    );
}

#[then("no set effort command is sent")]
fn then_no_set_effort_command_sent(world: &mut TuiWorld) {
    // Non-polling drain: the submit step already waited for late sends, and a
    // 400ms empty poll here would push the rejection notification past its
    // 3s lifetime before the next step can assert it.
    let mut commands = world.tui_last_commands.clone();
    commands.extend(drive(world, |h| h.try_drain_commands()));
    world.tui_last_commands = commands;
    assert!(
        command_of_type(&world.tui_last_commands, "set_effort").is_none(),
        "no set_effort command should be sent, got {:?}",
        world.tui_last_commands
    );
}

/// One logical outcome — "rejected with a message listing the valid levels" —
/// asserted level-list-aware: the list after "valid levels:" must match the
/// expected vocabulary token-for-token (substring checks would let "xhigh"
/// stand in for "high").
#[then(expr = "the app reports an invalid effort level listing {string}")]
fn then_app_reports_invalid_effort(world: &mut TuiWorld, expected_csv: String) {
    // Raw pushed messages, not the rendered popup: the popup's 3s display
    // lifetime races concurrent-scenario scheduling; the behaviour under test
    // is the rejection content, not the popup's decay.
    let messages = drive(world, |h| h.notification_messages());
    let notification = messages
        .iter()
        .find(|m| m.contains("Invalid effort level"))
        .unwrap_or_else(|| panic!("expected an invalid-effort notification, got: {messages:?}"));
    let listed = notification
        .split_once("valid levels:")
        .map(|(_, rest)| rest.trim())
        .unwrap_or_else(|| panic!("notification lists no valid levels:\n{notification}"));
    let listed: Vec<&str> = listed.split(',').map(str::trim).collect();
    let expected: Vec<&str> = expected_csv.split(',').map(str::trim).collect();
    assert_eq!(
        listed, expected,
        "rejection must list exactly the provider vocabulary, got:\n{notification}"
    );
}

#[when(expr = "the set effort response succeeds with effort {string}")]
fn when_set_effort_response_succeeds(world: &mut TuiWorld, effort: String) {
    let id = command_of_type(&world.tui_last_commands, "set_effort")
        .and_then(|cmd| json_field(cmd, "id"));
    drive(world, |h| {
        h.event(Event::Response {
            id,
            command: "set_effort".into(),
            success: true,
            data: Some(serde_json::json!({ "effort": effort })),
            error: None,
        });
    });
}

#[when(expr = "the set effort response fails with {string}")]
fn when_set_effort_response_fails(world: &mut TuiWorld, error: String) {
    let id = command_of_type(&world.tui_last_commands, "set_effort")
        .and_then(|cmd| json_field(cmd, "id"));
    drive(world, |h| {
        h.event(Event::Response {
            id,
            command: "set_effort".into(),
            success: false,
            data: None,
            error: Some(error),
        });
    });
}

#[then("the effort selector is visible")]
fn then_effort_selector_visible(world: &mut TuiWorld) {
    let entries = drive(world, |h| h.effort_selector_entries());
    assert!(
        entries.is_some(),
        "effort selector should be open after bare /effort"
    );
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains("Select Effort"),
        "effort selector overlay should render, frame:\n{frame}"
    );
}

/// Asserts against the selector's OWN entry list — not frame substrings,
/// where the footer also names a level and "high" is a substring of "xhigh".
#[then(expr = "the effort selector lists exactly {string}")]
fn then_effort_selector_lists_exactly(world: &mut TuiWorld, csv: String) {
    let expected: Vec<String> = csv.split(',').map(|l| l.trim().to_string()).collect();
    let entries =
        drive(world, |h| h.effort_selector_entries()).expect("effort selector should be open");
    assert_eq!(
        entries, expected,
        "effort selector must list exactly the provider vocabulary"
    );
}

#[when(expr = "I filter the effort selector with {string}")]
fn when_filter_effort_selector(world: &mut TuiWorld, query: String) {
    drive(world, |h| {
        for ch in query.chars() {
            h.press(Key::Char(ch));
        }
    });
}

#[when("I accept the selected effort")]
fn when_accept_selected_effort(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain_commands(world);
}

#[when(expr = "I choose effort {string} from the effort selector")]
fn when_choose_effort_from_selector(world: &mut TuiWorld, effort: String) {
    drive(world, |h| {
        h.submit("/effort");
        for ch in effort.chars() {
            h.press(Key::Char(ch));
        }
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain_commands(world);
}

#[when(expr = "I request effort {string} for the selected sub-agent")]
fn when_request_effort_for_selected_subagent(world: &mut TuiWorld, effort: String) {
    drive(world, |h| {
        h.submit(&format!("/effort {effort}"));
    });
    world.tui_last_commands = drive(world, |h| h.try_drain_commands());
}

#[then(expr = "sub-agent {string} receives effort {string}")]
fn then_subagent_receives_effort(world: &mut TuiWorld, id: String, effort: String) {
    assert!(
        command_of_type(&world.tui_last_commands, "set_effort").is_none(),
        "selected sub-agent effort should use the child connection, not the master command stream: {:?}",
        world.tui_last_commands
    );
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let rx = world
        .tui_subagent_commands
        .as_mut()
        .expect("sub-agent command receiver");
    let deadline = std::time::Duration::from_secs(2);
    let commands = handle.block_on(async {
        let mut commands = Vec::new();
        loop {
            match tokio::time::timeout(deadline, rx.recv()).await {
                Ok(Some(line)) => {
                    let is_expected = json_field(&line, "type").as_deref() == Some("set_effort");
                    commands.push(line);
                    if is_expected {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        commands
    });
    let cmd = command_of_type(&commands, "set_effort")
        .unwrap_or_else(|| panic!("expected sub-agent set_effort command, got {commands:?}"));
    assert_eq!(
        json_field(cmd, "effort").as_deref(),
        Some(effort.as_str()),
        "sub-agent command should carry selected effort: {cmd}"
    );
    drive(world, |h| {
        h.route(
            &id,
            Event::Response {
                id: Some("se".into()),
                command: "set_effort".into(),
                success: true,
                data: Some(serde_json::json!({ "effort": effort })),
                error: None,
            },
        );
    });
}

#[then("no set effort command is sent to the master")]
fn then_no_set_effort_command_to_master(world: &mut TuiWorld) {
    assert!(
        command_of_type(&world.tui_last_commands, "set_effort").is_none(),
        "master command stream should not receive set_effort: {:?}",
        world.tui_last_commands
    );
}

#[then(expr = "the app reports invalid effort {string} with supported levels {string}")]
fn then_app_reports_invalid_effort_for_level(
    world: &mut TuiWorld,
    effort: String,
    expected_csv: String,
) {
    let messages = drive(world, |h| h.notification_messages());
    let notification = messages
        .iter()
        .find(|m| m.contains("Invalid effort level"))
        .unwrap_or_else(|| {
            panic!("expected invalid effort notification for {effort}, got: {messages:?}")
        });
    assert!(
        notification.contains(&format!("Invalid effort level \"{effort}\"")),
        "notification should name rejected effort, got:\n{notification}"
    );
    let listed = notification
        .split_once("valid levels:")
        .map(|(_, rest)| rest.trim())
        .unwrap_or_else(|| panic!("notification lists no valid levels:\n{notification}"));
    let listed: Vec<&str> = listed.split(',').map(str::trim).collect();
    let expected: Vec<&str> = expected_csv.split(',').map(str::trim).collect();
    assert_eq!(
        listed, expected,
        "invalid effort should list selected sub-agent levels"
    );
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
                    { "id": "u1", "role": "user", "content": "first prompt" },
                    { "id": "a1", "role": "assistant", "content": "first answer" },
                    { "id": "u2", "role": "user", "content": "most recent prompt" },
                    { "id": "a2", "role": "assistant", "content": "most recent answer" }
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

#[then(expr = "the footer shows context {string} without cost {string}")]
fn then_footer_shows_context_without_cost(world: &mut TuiWorld, context: String, cost: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&context) && !frame.contains(&cost),
        "footer should show context {context:?} without cost {cost:?}, got:\n{frame}"
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
    // Assert against the raw notification messages, not the rendered popup:
    // `notification_text()` respects the 3s display lifetime, so under
    // concurrent-scenario scheduling delays the popup can expire between the
    // When that raises it and this Then, yielding a spurious empty match
    // (#1067). `notification_messages()` is expiry-independent.
    let notification = drive(world, |h| h.notification_messages().join("\n"));
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
    // #1061: rewind targets the message's STABLE id, not a page-local index. The
    // most recent user turn ("most recent prompt") is id "u2".
    assert_eq!(
        value.get("messageId").and_then(|v| v.as_str()),
        Some("u2"),
        "the default selected target should be the most recent user turn: {rewind}"
    );
    assert!(
        value.get("messageIndex").is_none(),
        "rewind must not send a page-local index: {rewind}"
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

#[then(expr = "the master follow-up command is sent with message {string}")]
fn then_master_follow_up_sent(world: &mut TuiWorld, message: String) {
    let follow_up = command_of_type(&world.tui_last_commands, "follow_up").unwrap_or_else(|| {
        panic!(
            "expected follow-up command, got {:?}",
            world.tui_last_commands
        )
    });
    let value: serde_json::Value = serde_json::from_str(follow_up).expect("follow-up command json");
    assert_eq!(
        value.get("message").and_then(|v| v.as_str()),
        Some(message.as_str()),
        "streaming master submit should queue a follow-up: {follow_up}"
    );
    assert!(
        value.get("streamingBehavior").is_none(),
        "follow-up command must not carry steer streaming behavior: {follow_up}"
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

#[then(expr = "the app master session shows {string}")]
fn then_app_master_session_shows(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&expected),
        "master session should show {expected:?}, got:\n{frame}"
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

#[then(expr = "a set model command is sent for {string}")]
fn then_set_model_command_sent_for(world: &mut TuiWorld, expected: String) {
    let cmd = command_of_type(&world.tui_last_commands, "set_model").unwrap_or_else(|| {
        panic!(
            "expected set_model command, got {:?}",
            world.tui_last_commands
        )
    });
    let value: serde_json::Value = serde_json::from_str(cmd).expect("set_model command json");
    assert_eq!(
        value.get("model").and_then(|v| v.as_str()),
        Some(expected.as_str()),
        "selected model should be sent in set_model command: {cmd}"
    );
}

#[then(expr = "the footer shows the master model {string}")]
fn then_footer_shows_master_model(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&expected),
        "master footer should show selected model {expected:?}, got:\n{frame}"
    );
}

#[then("the model selector is visible")]
fn then_model_selector_visible(world: &mut TuiWorld) {
    let is_open = drive(world, |h| h.model_selector_open());
    let frame = drive(world, |h| h.full_frame());
    assert!(
        is_open && frame.contains("Select Model"),
        "model selector should be open and visible, open={is_open}, frame:\n{frame}"
    );
}

// ── Model routing to focused sub-agent (#1085) ─────────────────────────

#[given(expr = "the master uses model {string}")]
fn given_master_uses_model(world: &mut TuiWorld, model: String) {
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("gs-master".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": model,
                "effort": "medium",
                "effortLevels": ["none", "low", "medium", "high", "xhigh"],
                "maxContextTokens": 100_000,
            })),
            error: None,
        });
    });
}

#[given(expr = "a TUI viewing sub-agent {string} without a ready connection")]
fn given_viewing_subagent_without_ready_connection(world: &mut TuiWorld, id: String) {
    // Select a tracked sub-agent that has no socket path so connect-on-select
    // never installs `active_cmd_tx` — the same not-ready path as effort (#1084).
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut h = rt.block_on(async {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(quecto_tui::interface::app::tui_harness::spawn_start(&id));
        h.event(quecto_tui::interface::app::tui_harness::subagents_changed(
            vec![quecto_tui::interface::app::tui_harness::subagent(
                &id, "idle", None,
            )],
        ));
        h.select(Some(&id));
        h
    });
    h.try_drain_commands();
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_subagent_commands = None;
    world.tui_viewed_agent = Some(id);
    world.tui_last_commands.clear();
}

#[when(expr = "I choose model {string} from the model selector")]
fn when_choose_model_from_selector(world: &mut TuiWorld, model: String) {
    drive(world, |h| {
        h.press(Key::Ctrl('l'));
    });
    let list_cmds = drain_commands(world);
    let request = command_of_type(&list_cmds, "list_models")
        .unwrap_or_else(|| panic!("model selector should request list_models, got {list_cmds:?}"));
    let id = json_field(request, "id");
    drive(world, |h| {
        h.event(Event::Response {
            id,
            command: "list_models".into(),
            success: true,
            data: Some(serde_json::json!({
                "models": [
                    { "id": model, "provider": "Test", "auth": "api" }
                ]
            })),
            error: None,
        });
        for ch in model.chars() {
            h.press(Key::Char(ch));
        }
        h.press(Key::Enter);
    });
    world.tui_last_commands = drive(world, |h| h.try_drain_commands());
}

#[when(expr = "sub-agent {string} acknowledges and reports model {string}")]
fn when_subagent_acknowledges_and_reports_model(world: &mut TuiWorld, id: String, model: String) {
    // Production set_model acks with data: None (uds.rs AgentEvent::ok). The
    // TUI then requests get_state; that authoritative response, not the bare
    // acknowledgement, updates the focused session's footer and selector.
    drive(world, |h| {
        h.route(
            &id,
            Event::Response {
                id: Some("sm".into()),
                command: "set_model".into(),
                success: true,
                data: None,
                error: None,
            },
        );
        h.route(
            &id,
            Event::Response {
                id: Some("resync".into()),
                command: "get_state".into(),
                success: true,
                data: Some(serde_json::json!({
                    "model": model,
                    "effort": "low",
                    "effortLevels": ["low", "medium", "high", "max"],
                })),
                error: None,
            },
        );
    });
}

#[when(expr = "a master model switch succeeds for {string}")]
fn when_master_model_switch_succeeds(world: &mut TuiWorld, model: String) {
    // Late master-stream set_model success while a child is focused — must not
    // clobber the focused child's displayed model/footer (#1085).
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("sm-master".into()),
            command: "set_model".into(),
            success: true,
            data: Some(serde_json::json!({ "model": model })),
            error: None,
        });
    });
}

fn drain_subagent_commands_of_type(world: &mut TuiWorld, ty: &str) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let rx = world
        .tui_subagent_commands
        .as_mut()
        .expect("sub-agent command receiver");
    let deadline = std::time::Duration::from_secs(2);
    handle.block_on(async {
        let mut commands = Vec::new();
        loop {
            match tokio::time::timeout(deadline, rx.recv()).await {
                Ok(Some(line)) => {
                    let is_expected = json_field(&line, "type").as_deref() == Some(ty);
                    commands.push(line);
                    if is_expected {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        commands
    })
}

#[then(expr = "sub-agent {string} receives model {string}")]
fn then_subagent_receives_model(world: &mut TuiWorld, _id: String, model: String) {
    // Observe-only: assert the child UDS received set_model. Acknowledgement
    // is a separate When step when footer/post-ack behaviour is under test.
    assert!(
        command_of_type(&world.tui_last_commands, "set_model").is_none(),
        "selected sub-agent model should use the child connection, not the master command stream: {:?}",
        world.tui_last_commands
    );
    let commands = drain_subagent_commands_of_type(world, "set_model");
    let cmd = command_of_type(&commands, "set_model")
        .unwrap_or_else(|| panic!("expected sub-agent set_model command, got {commands:?}"));
    assert_eq!(
        json_field(cmd, "model").as_deref(),
        Some(model.as_str()),
        "sub-agent command should carry selected model: {cmd}"
    );
}

#[then("no set model command is sent to the master")]
fn then_no_set_model_command_to_master(world: &mut TuiWorld) {
    assert!(
        command_of_type(&world.tui_last_commands, "set_model").is_none(),
        "master command stream should not receive set_model: {:?}",
        world.tui_last_commands
    );
}

#[then("no set model command is sent")]
fn then_no_set_model_command_sent(world: &mut TuiWorld) {
    let mut commands = world.tui_last_commands.clone();
    commands.extend(drive(world, |h| h.try_drain_commands()));
    world.tui_last_commands = commands;
    assert!(
        command_of_type(&world.tui_last_commands, "set_model").is_none(),
        "no set_model command should be sent, got {:?}",
        world.tui_last_commands
    );
}

#[then(expr = "the footer shows the sub-agent model {string}")]
fn then_footer_shows_subagent_model(world: &mut TuiWorld, model: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&model),
        "selected sub-agent footer should show model {model:?}, got:\n{frame}"
    );
}

#[then(expr = "the master session still shows model {string}")]
fn then_master_session_still_shows_model(world: &mut TuiWorld, model: String) {
    // Probe the master's OWN session footer without switching focus: while a
    // child is focused, `current_model` tracks the active (child) session for
    // the selector marker, so only the master session footer is authoritative.
    let footer = drive(world, |h| h.master_footer_text());
    assert!(
        footer.contains(&model),
        "master session footer must still show model {model:?}, got:\n{footer}"
    );
}

#[then(expr = "the app notification does not include {string}")]
fn then_notification_does_not_include(world: &mut TuiWorld, unexpected: String) {
    let messages = drive(world, |h| h.notification_messages());
    assert!(
        !messages.iter().any(|m| m.contains(&unexpected)),
        "notification must not include {unexpected:?}, got: {messages:?}"
    );
}

// ── TUI tool execution rendering (`tui_tool_execution_rendering.feature`) ──

fn tool_start(tool_call_id: &str, tool_name: &str, args: serde_json::Value) -> Event {
    Event::ToolExecutionStart {
        tool_call_id: tool_call_id.into(),
        tool_name: tool_name.into(),
        args,
    }
}

fn tool_success(tool_call_id: &str, tool_name: &str, text: &str) -> Event {
    Event::ToolExecutionEnd {
        tool_call_id: tool_call_id.into(),
        tool_name: tool_name.into(),
        result: serde_json::json!({ "content": [{ "type": "text", "text": text }] }),
        is_error: false,
    }
}

#[given("a fresh TUI tool rendering harness")]
fn given_fresh_tool_rendering_harness(world: &mut TuiWorld) {
    init_fresh(world);
}

#[given(regex = r#"^a TUI tool rendering viewport that is (\d+) display columns wide$"#)]
fn given_tui_tool_rendering_viewport(world: &mut TuiWorld, width: usize) {
    world.tui_tool_viewport_width = Some(width);
}

#[when("a tool result contains an uninterrupted long value")]
fn when_tool_result_contains_uninterrupted_long_value(world: &mut TuiWorld) {
    let width = world
        .tui_tool_viewport_width
        .expect("tool viewport width set");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut h = rt.block_on(TuiHarness::sized(width, 30));
    h.event(tool_start(
        "bdd-generic",
        "custom_tool",
        serde_json::json!({ "query": "demo" }),
    ));
    h.event(tool_success(
        "bdd-generic",
        "custom_tool",
        "alpha-beta-gamma-delta-epsilon-zeta",
    ));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

#[when(expr = "a bash tool call runs command {string} with {int} output lines")]
fn when_bash_tool_call_runs(world: &mut TuiWorld, command: String, line_count: u32) {
    let output = (1..=line_count)
        .map(|n| format!("line-{n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let raw = drive(world, |h| {
        h.event(tool_start(
            "bdd-bash",
            "bash",
            serde_json::json!({ "command": command }),
        ));
        h.event(tool_success("bdd-bash", "bash", &output));
        h.full_frame_raw()
    });
    world.tui_tool_rendered_raw = Some(raw);
}

#[when(expr = "a read tool call previews path {string} with controlled content")]
fn when_read_tool_call_previews_controlled_content(world: &mut TuiWorld, path: String) {
    let content = "safe\tvalue\n\u{1b}]0;pwned-title\u{7}\nsecond safe line";
    drive(world, |h| {
        h.event(tool_start(
            "bdd-read",
            "read",
            serde_json::json!({ "path": path }),
        ));
        h.event(tool_success("bdd-read", "read", content));
    });
}

#[when(expr = "a workflow tool call checks step {int} with multiline result")]
fn when_workflow_tool_call_checks_step(world: &mut TuiWorld, step_num: u32) {
    drive(world, |h| {
        h.event(Event::ToolExecutionStart {
            tool_call_id: "bdd-workflow".into(),
            tool_name: "workflow".into(),
            args: serde_json::json!({ "action": "check", "step": step_num }),
        });
        h.event(Event::ToolExecutionEnd {
            tool_call_id: "bdd-workflow".into(),
            tool_name: "workflow".into(),
            result: serde_json::json!({ "content": [{ "type": "text", "text": "Step 2 checked.\nextra detail" }] }),
            is_error: false,
        });
    });
}

#[when("I expand tool output in the TUI")]
fn when_expand_tool_output_in_tui(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Ctrl('o'));
    });
}

#[then(expr = "the tool rendering shows {string}")]
fn then_tool_rendering_shows(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&expected),
        "tool rendering should show {expected:?}, got:\n{frame}"
    );
}

#[then("the tool rendering includes the complete uninterrupted long value")]
fn then_tool_rendering_includes_complete_long_value(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    let joined_tool_pane_lines: String = frame
        .lines()
        .filter_map(|line| {
            line.split_once('│')
                .map(|(_, pane)| pane.trim().trim_start_matches('│').trim())
        })
        .collect();
    assert!(
        joined_tool_pane_lines.contains("alpha-beta-gamma-delta-epsilon-zeta"),
        "tool rendering should include the complete long value, got:\n{frame}"
    );
}

#[then(expr = "the tool rendering hides {string}")]
fn then_tool_rendering_hides(world: &mut TuiWorld, unexpected: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        !frame.contains(&unexpected),
        "tool rendering should hide {unexpected:?}, got:\n{frame}"
    );
}

#[then("the raw tool frame does not contain terminal title escape controls")]
fn then_raw_tool_frame_has_no_title_escapes(world: &mut TuiWorld) {
    let raw = drive(world, |h| h.full_frame_raw());
    assert!(
        !raw.contains("\u{1b}]") && !raw.contains("\u{9d}"),
        "raw rendered frame must not contain OSC/title controls, got:\n{raw:?}"
    );
}

#[when("a failed bash tool call is rendered")]
fn when_failed_bash_tool_call_is_rendered(world: &mut TuiWorld) {
    let raw = drive(world, |h| {
        h.event(tool_start(
            "bdd-failed-bash",
            "bash",
            serde_json::json!({ "command": "false" }),
        ));
        h.event(Event::ToolExecutionEnd {
            tool_call_id: "bdd-failed-bash".into(),
            tool_name: "bash".into(),
            result: serde_json::json!({ "content": [{ "type": "text", "text": "command failed" }] }),
            is_error: true,
        });
        h.full_frame_raw()
    });
    world.tui_tool_rendered_raw = Some(raw);
}

#[then("the tool block uses the terminal default background")]
fn then_tool_block_uses_terminal_default_background(world: &mut TuiWorld) {
    let violations = explicit_background_sgr(raw_tool_frame(world));
    assert!(
        violations.is_empty(),
        "explicit backgrounds: {violations:?}"
    );
}

#[then("the tool block has a visible boundary")]
fn then_tool_block_has_visible_boundary(world: &mut TuiWorld) {
    let plain = strip_ansi(raw_tool_frame(world));
    assert!(
        plain
            .lines()
            .any(|line| line.contains("│ ✓ $ printf theme"))
    );
}

#[then("the tool block shows an error symbol and status text")]
fn then_tool_block_shows_error_symbol_and_status_text(world: &mut TuiWorld) {
    let plain = strip_ansi(raw_tool_frame(world));
    assert!(
        plain.contains("✗ $ false"),
        "error symbol missing: {plain:?}"
    );
    assert!(
        plain.contains("command failed"),
        "error text missing: {plain:?}"
    );
}

fn raw_tool_frame(world: &TuiWorld) -> &str {
    world
        .tui_tool_rendered_raw
        .as_deref()
        .expect("tool rendering captured by the When step")
}

fn explicit_background_sgr(raw: &str) -> Vec<u16> {
    raw.split("\u{1b}[")
        .skip(1)
        .filter_map(|segment| segment.split_once('m').map(|(params, _)| params))
        .flat_map(|params| {
            params
                .replace(':', ";")
                .split(';')
                .filter_map(|part| part.parse().ok())
                .collect::<Vec<u16>>()
        })
        .filter(|code| *code == 48 || (40..=47).contains(code) || (100..=107).contains(code))
        .collect()
}

#[then("every tool rendering line should fit within the viewport")]
fn then_every_tool_rendering_line_fits_viewport(world: &mut TuiWorld) {
    let width = world
        .tui_tool_viewport_width
        .expect("tool viewport width set");
    let frame = drive(world, |h| h.full_frame());
    for line in frame.lines() {
        assert!(
            visible_width(line) <= width,
            "tool rendering line must fit within {width} display columns, got {}: {line:?}\nframe:\n{frame}",
            visible_width(line)
        );
    }
}

fn workflow_rule_lines(frame: &str) -> Vec<String> {
    frame
        .lines()
        .filter(|line| {
            strip_ansi(line)
                .rsplit_once("│ ")
                .is_some_and(|(_, segment)| {
                    !segment.is_empty() && segment.chars().all(|c| c == '─')
                })
        })
        .map(str::to_string)
        .collect()
}

#[given(expr = "a fresh TUI app harness at width {int}")]
fn given_fresh_harness_at_width(world: &mut TuiWorld, width: usize) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::sized(width, 30));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_last_commands.clear();
}

#[when(
    expr = "workflow state reports issue {int} with step {int} {string} in phase {string} out of {int}"
)]
fn when_workflow_state_reports_step(
    world: &mut TuiWorld,
    issue: u32,
    current_step: u32,
    label: String,
    phase: String,
    total: u32,
) {
    let steps = (1..=total)
        .map(|idx| {
            serde_json::json!({
                "index": idx,
                "label": if idx == current_step { label.as_str() } else { "Other step" },
                "phase": if idx == current_step { phase.as_str() } else { "done" },
                "done": idx < current_step,
            })
        })
        .collect::<Vec<_>>();

    drive(world, |h| {
        h.event(Event::WorkflowState {
            agent_id: None,
            steps,
            progress: serde_json::json!({
                "done": current_step.saturating_sub(1),
                "total": total,
                "percent": current_step.saturating_sub(1).saturating_mul(100).checked_div(total).unwrap_or(0),
            }),
            active_issue: Some(serde_json::json!({
                "number": issue,
                "title": "BDD coverage wave",
            })),
            mode: Some("active".to_string()),
            active_template: None,
            available_templates: None,
        });
    });
}

#[then(expr = "the workflow bar shows {string}")]
fn then_workflow_bar_shows(world: &mut TuiWorld, expected: String) {
    let pane = drive(world, |h| h.main_pane());
    assert!(
        pane.contains(&expected),
        "workflow bar should show {expected:?} in main pane, got:\n{pane}"
    );
}

#[then(expr = "the bottom stack does not show workflow text {string}")]
fn then_bottom_stack_hides_workflow_text(world: &mut TuiWorld, unexpected: String) {
    let bottom = drive(world, |h| h.bottom_stack());
    assert!(
        !bottom.contains(&unexpected),
        "workflow text {unexpected:?} should render in the main pane, not bottom stack:\n{bottom}"
    );
}

#[then("every workflow frame row fits the terminal width")]
fn then_workflow_rows_fit_terminal_width(world: &mut TuiWorld) {
    let expected_width = drive(world, |h| h.terminal_width());
    let frame = drive(world, |h| h.full_frame());
    let rows = workflow_rule_lines(&frame);
    assert!(
        !rows.is_empty(),
        "workflow rule rows should render:\n{frame}"
    );
    for row in rows {
        let width = visible_width(&row);
        assert!(
            width <= expected_width,
            "workflow row width {width} should fit terminal width {expected_width}:\n{row}\nframe:\n{frame}"
        );
    }
}

#[then("the workflow bar preserves left padding after the divider")]
fn then_workflow_bar_preserves_left_padding(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    let row = workflow_rule_lines(&frame)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("workflow rule row should render:\n{frame}"));
    assert!(
        strip_ansi(&row).contains("│ ─"),
        "workflow rule should start after the normal gutter/padding, got:\n{row}\nframe:\n{frame}"
    );
}

// ── #1050: master history backfill on --socket attach ──────────────────

/// Deliver a successful master `get_messages` response with the attach-backfill
/// request id, mirroring the kernel payload the TUI requests on socket attach.
fn deliver_master_backfill(world: &mut TuiWorld, user: &str, assistant: &str) {
    let data = serde_json::json!({
        "messages": [
            { "role": "user", "content": user },
            { "role": "assistant", "content": assistant },
        ]
    });
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("attach-backfill".into()),
            command: "get_messages".into(),
            success: true,
            data: Some(data),
            error: None,
        });
    });
    world.tui_last_master_backfill = Some((user.to_string(), assistant.to_string()));
}

fn deliver_empty_master_backfill(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("attach-backfill".into()),
            command: "get_messages".into(),
            success: true,
            data: Some(serde_json::json!({ "messages": [] })),
            error: None,
        });
    });
}

/// Domain Given: TUI is attached to a running agent (socket-attach posture).
/// The outbound get_messages request is covered by unit tests; BDD drives the
/// response path that renders prior durable history into the master session.
#[given("a TUI attached to a running agent")]
fn given_tui_attached_to_running_agent(world: &mut TuiWorld) {
    init_fresh(world);
}

#[given(expr = "the master has already streamed the live token {string}")]
fn given_master_already_streamed_live_token(world: &mut TuiWorld, token: String) {
    drive(world, |h| {
        h.event(Event::AgentStart);
        h.event(Event::Token { token });
    });
}

#[given(expr = "the master backfill history {string} then {string} has already arrived")]
fn given_master_backfill_history_has_arrived(
    world: &mut TuiWorld,
    user: String,
    assistant: String,
) {
    deliver_master_backfill(world, &user, &assistant);
}

#[given("an empty master backfill history has already arrived")]
fn given_empty_master_backfill_has_arrived(world: &mut TuiWorld) {
    deliver_empty_master_backfill(world);
}

#[when(expr = "the master backfill history {string} then {string} arrives")]
fn when_master_backfill_history_arrives(world: &mut TuiWorld, user: String, assistant: String) {
    deliver_master_backfill(world, &user, &assistant);
}

#[when("the same master backfill history arrives again")]
fn when_same_master_backfill_history_arrives_again(world: &mut TuiWorld) {
    let (user, assistant) = world
        .tui_last_master_backfill
        .clone()
        .expect("a prior master backfill must have been delivered first");
    deliver_master_backfill(world, &user, &assistant);
}

#[then(expr = "the app master session still shows {string}")]
fn then_app_master_session_still_shows(world: &mut TuiWorld, text: String) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(&text),
        "master history backfill must preserve live content {text:?}, got:\n{frame}"
    );
}

#[then(expr = "{string} appears above {string} in the master session")]
fn then_appears_above_in_master_session(world: &mut TuiWorld, upper: String, lower: String) {
    let frame = drive(world, |h| h.full_frame());
    let up = frame.find(&upper);
    let lo = frame.find(&lower);
    assert!(
        matches!((up, lo), (Some(u), Some(l)) if u < l),
        "history {upper:?} must render ABOVE {lower:?} in the master session, got:\n{frame}"
    );
}

#[then(expr = "the app master session shows {string} exactly once")]
fn then_app_master_session_shows_exactly_once(world: &mut TuiWorld, text: String) {
    let frame = drive(world, |h| h.full_frame());
    assert_eq!(
        frame.matches(&text).count(),
        1,
        "re-delivered master history must not duplicate {text:?}, got:\n{frame}"
    );
}
