//! `/setup` dispatch (#2024 S6): the slash command composes the walkthrough
//! text and re-enters the ordinary user-turn path — visible in the transcript,
//! sent as a `prompt` — while a malformed variant only toasts the usage.

use crate::setup::{SETUP_USAGE, SetupArea, setup_walkthrough_prompt};
use crate::shell::app::tui_harness::TuiHarness;

#[tokio::test]
async fn setup_submits_the_walkthrough_as_the_user_turn() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_submit("/setup");
    let expected = setup_walkthrough_prompt(&SetupArea::All);
    assert_eq!(h.active_user_entries(), vec![expected.clone()]);
    let cmds = h.drain_commands().await;
    let prompt: serde_json::Value = serde_json::from_str(&cmds[0]).expect("prompt json");
    assert_eq!(prompt["type"], "prompt");
    assert_eq!(prompt["message"], expected);
}

#[tokio::test]
async fn setup_model_variant_names_the_model() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_submit("/setup   model   prov/m-1 ");
    let expected = setup_walkthrough_prompt(&SetupArea::Model("prov/m-1".into()));
    assert_eq!(h.active_user_entries(), vec![expected]);
}

#[tokio::test]
async fn setup_usage_toasts_and_submits_nothing() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_submit("/setup nope");
    assert!(h.active_user_entries().is_empty());
    assert!(h.try_drain_commands().is_empty());
    assert_eq!(h.notification_messages(), vec![SETUP_USAGE.to_string()]);
}

#[tokio::test]
async fn setup_prefix_does_not_capture_other_commands() {
    // `/setupx` is not a registered command: rejected as unknown, no prompt.
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_submit("/setupx");
    assert!(h.active_user_entries().is_empty());
    assert!(h.try_drain_commands().is_empty());
    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m.contains("Unknown slash command"))
    );
}
