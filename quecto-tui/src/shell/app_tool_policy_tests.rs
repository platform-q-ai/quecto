use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn ctrl_t_opens_tool_policy_modal_and_apply_sends_mutations() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::None),
            ..Default::default()
        }]);

    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(crate::components::ansi::strip_ansi(&frame).contains("Tool Policy"));
    assert!(crate::components::ansi::strip_ansi(&frame).contains("[--] alpha"));

    h.app_mut().handle_key(crate::shell::keys::Key::Char(' '));
    h.app_mut().handle_key(crate::shell::keys::Key::Enter);
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
    assert!(sent.contains("\"toolId\":\"tool-alpha\""), "{sent}");
    assert!(sent.contains("\"scope\":\"parent\""), "{sent}");
}

#[tokio::test]
async fn ctrl_t_with_empty_catalogue_opens_modal_after_catalogue_update() {
    let mut h = harness().await;

    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"type\":\"get_tool_catalogue\""), "{sent}");

    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::None),
            ..Default::default()
        }]);

    let frame = h.app_mut().compose_frame().join("\n");
    assert!(crate::components::ansi::strip_ansi(&frame).contains("Tool Policy"));
    assert!(crate::components::ansi::strip_ansi(&frame).contains("[--] alpha"));
}

#[tokio::test]
async fn ctrl_t_with_empty_catalogue_opens_modal_after_get_tool_catalogue_response() {
    let mut h = harness().await;

    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"type\":\"get_tool_catalogue\""), "{sent}");

    h.app_mut().handle_response(
        Some("tool-policy-catalogue".into()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha",
                "profileScope": "none"
            }]
        })),
        None,
    );

    let frame = h.app_mut().compose_frame().join("\n");
    assert!(crate::components::ansi::strip_ansi(&frame).contains("Tool Policy"));
    assert!(crate::components::ansi::strip_ansi(&frame).contains("[--] alpha"));
}

#[tokio::test]
async fn ctrl_t_apply_without_changes_does_not_persist_effective_downstream_scope() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: None,
            effective_scope: Some(crate::protocol::client::ToolScope::Child),
            ..Default::default()
        }]);

    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(crate::components::ansi::strip_ansi(&frame).contains("[--] alpha"));

    h.app_mut().handle_key(crate::shell::keys::Key::Enter);
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
    assert!(sent.contains("\"toolId\":\"tool-alpha\""), "{sent}");
    assert!(sent.contains("\"scope\":\"none\""), "{sent}");
    assert!(!sent.contains("\"scope\":\"child\""), "{sent}");
}

#[tokio::test]
async fn help_mentions_ctrl_t_tool_policy_shortcut() {
    let mut h = harness().await;
    h.app_mut().show_help();
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(
        crate::components::ansi::strip_ansi(&frame)
            .contains("Ctrl+T         Open tool policy selector")
    );
}
