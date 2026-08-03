use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

async fn request_tool_catalogue(h: &mut TuiHarness) -> String {
    h.app_mut().handle_key(crate::shell::keys::Key::Ctrl('t'));
    let commands = h.drain_commands().await.join("\n");
    let value: serde_json::Value = serde_json::from_str(
        commands
            .lines()
            .find(|line| line.contains("get_tool_catalogue"))
            .expect("get_tool_catalogue command"),
    )
    .expect("valid command json");
    value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("command id")
        .to_owned()
}

#[tokio::test]
async fn ctrl_t_fresh_legacy_profile_enabled_overrides_stale_cached_scope_on_replace() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue_event(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::None),
            profile_enabled: Some(false),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha",
                "profileEnabled": true
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("[PC] alpha"),
        "fresh legacy profileEnabled did not override stale cached scope:\n{frame}"
    );
    assert!(!frame.contains("[--] alpha"), "{frame}");
}

#[tokio::test]
async fn tool_policy_changed_fresh_legacy_profile_enabled_overrides_stale_cached_scope_on_merge() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue_event(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::None),
            profile_enabled: Some(false),
            ..Default::default()
        }]);

    h.app_mut()
        .merge_tool_policy_results(vec![crate::protocol::client::ToolPolicyResult {
            after: Some(crate::protocol::client::ToolCatalogueEntry {
                stable_id: "tool-alpha".into(),
                name: "alpha".into(),
                profile_enabled: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("[PC] alpha"),
        "fresh legacy profileEnabled did not override stale cached scope on merge:\n{frame}"
    );
    assert!(!frame.contains("[--] alpha"), "{frame}");
}

#[tokio::test]
async fn ctrl_t_preserves_cached_legacy_profile_enabled_false_when_incoming_profile_fields_absent_on_replace()
 {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue_event(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_enabled: Some(false),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("[--] alpha"),
        "cached legacy profileEnabled=false was not preserved on replace when incoming profile fields were absent:\n{frame}"
    );
    assert!(!frame.contains("[PC] alpha"), "{frame}");
}

#[tokio::test]
async fn tool_policy_changed_preserves_cached_legacy_profile_enabled_false_when_incoming_profile_fields_absent_on_merge()
 {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue_event(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_enabled: Some(false),
            ..Default::default()
        }]);

    h.app_mut()
        .merge_tool_policy_results(vec![crate::protocol::client::ToolPolicyResult {
            after: Some(crate::protocol::client::ToolCatalogueEntry {
                stable_id: "tool-alpha".into(),
                name: "alpha".into(),
                ..Default::default()
            }),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("[--] alpha"),
        "cached legacy profileEnabled=false was not preserved on merge when incoming profile fields were absent:\n{frame}"
    );
    assert!(!frame.contains("[PC] alpha"), "{frame}");
}

#[tokio::test]
async fn ctrl_t_preserves_cached_scope_only_when_incoming_profile_fields_absent() {
    let mut h = harness().await;
    h.app_mut()
        .merge_tool_catalogue_event(vec![crate::protocol::client::ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(crate::protocol::client::ToolScope::Child),
            ..Default::default()
        }]);

    let request_id = request_tool_catalogue(&mut h).await;
    h.app_mut().handle_response(
        Some(request_id),
        "get_tool_catalogue".into(),
        true,
        Some(serde_json::json!({
            "tools": [{
                "stableId": "tool-alpha",
                "name": "alpha"
            }]
        })),
        None,
    );

    let frame = crate::components::ansi::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("[-C] alpha"),
        "cached scope was not preserved when incoming profile fields were absent:\n{frame}"
    );
}
