//! Unit tests for the runtime reasoning-effort control (#1067):
//! `/effort` command surface, set_effort protocol command, and the
//! footer's effort display.

use super::tui_harness::TuiHarness;
use crate::infrastructure::client::Event;
use crate::interface::components::footer::Footer;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn get_state_event(model: &str, effort: Option<&str>) -> Event {
    let mut data = serde_json::json!({ "model": model });
    if let Some(effort) = effort {
        data["effort"] = serde_json::json!(effort);
    }
    Event::Response {
        id: Some("gs".into()),
        command: "get_state".into(),
        success: true,
        data: Some(data),
        error: None,
    }
}

fn command_of_type(commands: &[String], ty: &str) -> Option<serde_json::Value> {
    commands.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        (v["type"] == ty).then_some(v)
    })
}

// ── builtin command registration ────────────────────────────────────────

#[test]
fn builtin_commands_include_effort() {
    assert!(
        super::builtin_commands().iter().any(|c| c.name == "effort"),
        "builtin_commands must include the /effort command"
    );
}

// ── footer display ──────────────────────────────────────────────────────

#[test]
fn footer_apply_get_state_shows_effort_level() {
    let mut f = Footer::new();
    f.apply_get_state(&serde_json::json!({ "model": "openai-api/gpt-5.5", "effort": "high" }));
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: high"),
        "footer should show 'effort: high', got: {stripped}"
    );
}

#[test]
fn footer_shows_default_effort_when_never_set() {
    let mut f = Footer::new();
    f.apply_get_state(&serde_json::json!({ "model": "openai-api/gpt-5.5" }));
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: default"),
        "footer should show 'effort: default' when effort was never set, got: {stripped}"
    );
}

#[test]
fn footer_shows_default_effort_for_explicit_null() {
    // The agent's real wire shape for a never-set effort is an explicit
    // `"effort": null`; it must render identically to a missing key.
    let mut f = Footer::new();
    f.apply_get_state(
        &serde_json::json!({ "model": "openai-api/gpt-5.5", "effort": serde_json::Value::Null }),
    );
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: default"),
        "explicit-null effort should show 'effort: default', got: {stripped}"
    );
}

#[tokio::test]
async fn footer_updates_when_effort_changes() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    assert!(
        h.full_frame().contains("effort: medium"),
        "footer should show initial effort"
    );
    h.submit("/effort high");
    h.event(Event::Response {
        id: None,
        command: "set_effort".into(),
        success: true,
        data: Some(serde_json::json!({ "effort": "high" })),
        error: None,
    });
    assert!(
        h.full_frame().contains("effort: high"),
        "footer should update live to the new effort, frame:\n{}",
        h.full_frame()
    );
}

// ── /effort <level> direct set ──────────────────────────────────────────

#[tokio::test]
async fn effort_command_with_valid_level_sends_set_effort() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort high");
    let commands = h.drain_commands().await;
    let cmd = command_of_type(&commands, "set_effort")
        .unwrap_or_else(|| panic!("expected a set_effort command, got {commands:?}"));
    assert_eq!(cmd["effort"], "high", "set_effort should carry the level");
}

#[tokio::test]
async fn effort_command_with_invalid_level_is_rejected_listing_valid_levels() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort turbo");
    let commands = h.drain_commands().await;
    assert!(
        command_of_type(&commands, "set_effort").is_none(),
        "invalid level must not send set_effort, got {commands:?}"
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("Invalid effort level"),
        "rejection must be surfaced, frame:\n{frame}"
    );
    assert!(
        frame.contains("valid levels: none, low, medium, high, xhigh"),
        "rejection must list the valid levels for the provider, frame:\n{frame}"
    );
    assert!(
        frame.contains("effort: medium"),
        "previous effort must stay in effect, frame:\n{frame}"
    );
}

// ── /effort selector ─────────────────────────────────────────────────────

#[tokio::test]
async fn effort_selector_lists_openai_vocabulary() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort");
    // Entry-based assertion: frame substrings can't distinguish "high" from
    // "xhigh" and the footer also names a level.
    let entries = h
        .effort_selector_entries()
        .expect("bare /effort should open the effort selector");
    assert_eq!(
        entries,
        ["none", "low", "medium", "high", "xhigh"],
        "OpenAI selector must list exactly the OpenAI vocabulary"
    );
}

#[tokio::test]
async fn effort_selector_lists_anthropic_vocabulary() {
    let mut h = harness().await;
    h.event(get_state_event(
        "anthropic-api/claude-fable-5",
        Some("high"),
    ));
    h.submit("/effort");
    let entries = h
        .effort_selector_entries()
        .expect("bare /effort should open the effort selector");
    assert_eq!(
        entries,
        ["low", "medium", "high", "max"],
        "Anthropic selector must list exactly the Anthropic vocabulary"
    );
}

// ── failure handling ─────────────────────────────────────────────────────

#[tokio::test]
async fn failed_set_effort_response_keeps_previous_footer_value() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort high");
    h.event(Event::Response {
        id: None,
        command: "set_effort".into(),
        success: false,
        data: None,
        error: Some("agent busy".into()),
    });
    let frame = h.full_frame();
    assert!(
        frame.contains("Effort switch failed: agent busy"),
        "failure must be surfaced, frame:\n{frame}"
    );
    assert!(
        frame.contains("effort: medium"),
        "footer must keep the previous effort after a failed switch, frame:\n{frame}"
    );
}
