use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

async fn request_tool_catalogue(h: &mut TuiHarness) -> String {
    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    extract_command_id(&h.drain_commands().await.join("\n"))
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

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
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

    let _request_id = request_tool_catalogue(&mut h).await;

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
async fn incremental_catalogue_event_during_pending_ctrl_t_does_not_open_or_consume() {
    let mut h = harness().await;

    let request_id = request_tool_catalogue(&mut h).await;
    assert!(
        request_id.starts_with("tool-policy-catalogue-"),
        "{request_id}"
    );

    h.app_mut()
        .handle_event(crate::protocol::client::Event::ToolCatalogueChanged {
            changed_tools: vec!["stale".into()],
            before: Vec::new(),
            after: vec![crate::protocol::client::ToolCatalogueEntry {
                stable_id: "tool-stale".into(),
                name: "stale".into(),
                profile_scope: Some(crate::protocol::client::ToolScope::Both),
                ..Default::default()
            }],
            reason: "register_tool".into(),
        });

    let event_frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !event_frame.contains("Tool Policy"),
        "incremental event opened pending policy modal:\n{event_frame}"
    );

    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-fresh",
                "name": "fresh",
                "profileScope": "child"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Tool Policy"),
        "fresh response did not open modal:\n{frame}"
    );
    assert!(frame.contains("[-C] fresh"), "fresh tool missing:\n{frame}");
    assert!(
        !frame.contains("stale"),
        "incremental stale tool leaked into modal:\n{frame}"
    );
}

#[tokio::test]
async fn stale_get_tool_catalogue_response_does_not_open_or_consume_pending_ctrl_t_request() {
    let mut h = harness().await;

    let request_id = request_tool_catalogue(&mut h).await;
    assert!(
        request_id.starts_with("tool-policy-catalogue-"),
        "{request_id}"
    );

    h.app_mut().handle_response(
        Some("unrelated-tool-policy-catalogue".into()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-stale",
                "name": "stale",
                "profileScope": "both"
            }]
        })),
        None,
    );

    let stale_frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !stale_frame.contains("Tool Policy"),
        "stale foreign response opened policy modal:\n{stale_frame}"
    );
    assert!(
        !stale_frame.contains("stale"),
        "stale foreign response replaced pending catalogue:\n{stale_frame}"
    );

    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-fresh",
                "name": "fresh",
                "profileScope": "child"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Tool Policy"),
        "matching response did not open modal:\n{frame}"
    );
    assert!(
        frame.contains("[-C] fresh"),
        "fresh catalogue missing:\n{frame}"
    );
    assert!(
        !frame.contains("stale"),
        "stale catalogue leaked into modal:\n{frame}"
    );
}

#[tokio::test]
async fn overlapping_ctrl_t_refreshes_ignore_earlier_response_and_open_latest() {
    let mut h = harness().await;

    let first_id = request_tool_catalogue(&mut h).await;

    let second_id = request_tool_catalogue(&mut h).await;

    assert_ne!(first_id, second_id, "Ctrl+T refresh ids must be unique");

    h.app_mut().handle_response(
        Some(first_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-stale",
                "name": "stale",
                "profileScope": "both"
            }]
        })),
        None,
    );

    let stale_frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !stale_frame.contains("Tool Policy"),
        "earlier response opened policy modal:\n{stale_frame}"
    );
    assert!(
        !stale_frame.contains("stale"),
        "earlier response replaced latest pending catalogue:\n{stale_frame}"
    );

    h.app_mut().handle_response(
        Some(second_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-fresh",
                "name": "fresh",
                "profileScope": "child"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Tool Policy"),
        "latest response did not open modal:\n{frame}"
    );
    assert!(
        frame.contains("[-C] fresh"),
        "fresh catalogue missing:\n{frame}"
    );
    assert!(
        !frame.contains("stale"),
        "stale catalogue leaked into modal:\n{frame}"
    );
}

fn extract_command_id(sent: &str) -> String {
    sent.lines()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|value| {
            value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| panic!("missing command id in {sent}"))
}

#[tokio::test]
async fn ctrl_t_with_empty_catalogue_opens_modal_after_get_tool_catalogue_response() {
    let mut h = harness().await;

    let request_id = request_tool_catalogue(&mut h).await;

    h.app_mut().handle_response(
        Some(request_id.clone()),
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
async fn ctrl_t_with_cached_catalogue_waits_for_fresh_get_tool_catalogue_response() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::None),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    let stale_frame = h.app_mut().compose_frame().join("\n");
    assert!(!crate::components::ansi::strip_ansi(&stale_frame).contains("Tool Policy"));

    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha",
                "profileScope": "child"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(frame.contains("Tool Policy"));
    assert!(frame.contains("[-C] alpha"));
    assert!(!frame.contains("[--] alpha"));
}

#[tokio::test]
async fn ctrl_t_success_without_tool_catalogue_data_does_not_open_stale_cached_catalogue() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-stale".into(),
            name: "stale".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::Both),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        None,
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        !frame.contains("Tool Policy"),
        "missing catalogue payload opened stale cached modal:\n{frame}"
    );
    assert!(
        !frame.contains("stale"),
        "stale cached tool remained visible after missing catalogue payload:\n{frame}"
    );
}

#[tokio::test]
async fn ctrl_t_empty_catalogue_response_clears_stale_cache_and_opens_empty_policy_state() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-stale".into(),
            name: "stale".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::Both),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({ "tools": [] })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("Tool Policy"),
        "empty valid catalogue did not open policy modal:\n{frame}"
    );
    assert!(
        frame.contains("No matching items"),
        "empty policy state not shown:\n{frame}"
    );
    assert!(
        !frame.contains("stale"),
        "stale cached tool remained visible after empty fresh catalogue:\n{frame}"
    );
}

#[tokio::test]
async fn ctrl_t_fresh_catalogue_response_removes_stale_absent_tools() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-stale".into(),
            name: "stale".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::Both),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-fresh",
                "name": "fresh",
                "profileScope": "none"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(frame.contains("Tool Policy"));
    assert!(frame.contains("[--] fresh"));
    assert!(
        !frame.contains("stale"),
        "stale absent tool remained visible:\n{frame}"
    );

    h.app_mut().handle_key(crate::shell::keys::Key::Enter);
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"toolId\":\"tool-fresh\""), "{sent}");
    assert!(
        !sent.contains("tool-stale"),
        "stale absent tool remained mutable:\n{sent}"
    );
}

#[tokio::test]
async fn ctrl_t_seeds_and_applies_legacy_profile_enabled_when_profile_scope_absent() {
    let mut h = harness().await;
    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha",
                "profileEnabled": true,
                "effectiveScope": "child"
            }]
        })),
        None,
    );
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(crate::components::ansi::strip_ansi(&frame).contains("[PC] alpha"));

    h.app_mut().handle_key(crate::shell::keys::Key::Enter);
    let sent = h.drain_commands().await.join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
    assert!(sent.contains("\"toolId\":\"tool-alpha\""), "{sent}");
    assert!(sent.contains("\"scope\":\"both\""), "{sent}");
    assert!(!sent.contains("\"scope\":\"child\""), "{sent}");
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

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id.clone()),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha",
                "profileScope": null,
                "effectiveScope": "child"
            }]
        })),
        None,
    );
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

#[tokio::test]
async fn tool_policy_modal_help_mentions_bulk_shortcuts() {
    let mut h = harness().await;
    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
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

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(frame.contains("Ctrl+Shift+A allow all"), "{frame}");
    assert!(frame.contains("Ctrl+Shift+D disable matches"), "{frame}");
    assert!(!frame.contains("disable visible"), "{frame}");
}
