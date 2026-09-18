//! `/setup` dispatch (#2024 S6): the slash command composes the walkthrough
//! text and re-enters the ordinary user-turn path — visible in the transcript,
//! sent as a `prompt` — while a malformed variant only toasts the usage.

use crate::setup::{SETUP_FROM_MASTER, SETUP_USAGE, SetupArea, setup_walkthrough_prompt};
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
    h.app_mut().handle_submit("/setup\tmodel   prov/m-1 ");
    let expected = setup_walkthrough_prompt(&SetupArea::Model("prov/m-1".into()));
    assert_eq!(h.active_user_entries(), vec![expected]);
}

#[tokio::test]
async fn setup_while_master_streams_queues_a_follow_up() {
    use crate::protocol::client::Event;
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(Event::Token {
        token: "working".into(),
    });
    h.app_mut().handle_submit("/setup admission");
    let expected = setup_walkthrough_prompt(&SetupArea::Admission);
    assert_eq!(h.active_user_entries(), vec![expected.clone()]);
    let cmds = h.drain_commands().await;
    let types: Vec<String> = cmds
        .iter()
        .map(|c| serde_json::from_str::<serde_json::Value>(c).expect("json")["type"].to_string())
        .collect();
    assert_eq!(types, vec!["\"follow_up\""], "{cmds:?}");
    let follow_up: serde_json::Value = serde_json::from_str(&cmds[0]).expect("follow_up json");
    assert_eq!(follow_up["message"], expected);
}

#[tokio::test]
async fn setup_with_a_focused_subagent_refuses_and_sends_nothing() {
    let mut h = TuiHarness::new().await;
    h.event(crate::shell::app::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) =
        crate::shell::app::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(crate::shell::app::tui_harness::subagents_changed(vec![
        crate::shell::app::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), child_rx.recv()).await;

    h.app_mut().handle_submit("/setup");
    assert!(
        h.active_user_entries().is_empty(),
        "no user turn on the child"
    );
    assert!(h.try_drain_commands().is_empty(), "nothing to master");
    let child = tokio::time::timeout(std::time::Duration::from_millis(50), child_rx.recv()).await;
    assert!(
        !matches!(&child, Ok(Some(line)) if line.contains("prompt") || line.contains("follow_up")),
        "child must not receive the walkthrough: {child:?}"
    );
    assert_eq!(
        h.notification_messages(),
        vec![SETUP_FROM_MASTER.to_string()],
        "the S2-style refusal names where to run it from"
    );
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
