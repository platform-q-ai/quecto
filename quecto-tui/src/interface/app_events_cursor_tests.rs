use super::app_events_test_support::test_app;
use super::*;

fn rendered_chat(app: &mut App) -> String {
    app.master_session
        .chat
        .render(80)
        .join("\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

#[tokio::test]
async fn master_tool_start_finalizes_streaming_assistant_cursor() {
    let mut app = test_app().await;
    app.handle_event(Event::Token {
        token: "I will check".into(),
    });
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "tool-1".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command":"echo hi"}),
    });
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "tool-1".into(),
        tool_name: "bash".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"hi"}]}),
        is_error: false,
    });
    app.handle_event(Event::Token {
        token: "Done".into(),
    });

    let mut plain = rendered_chat(&mut app);
    assert!(
        plain.contains("I will check"),
        "prior text should render: {plain}"
    );
    assert!(
        plain.contains("Done"),
        "new text segment should render: {plain}"
    );
    assert!(
        plain.contains('▌'),
        "new text segment should stream with a cursor: {plain}"
    );
    assert!(
        !plain.contains("I will check▌"),
        "tool start should clear cursor from interrupted text: {plain}"
    );

    app.handle_event(Event::TurnEnd {
        message: serde_json::json!({}),
    });
    plain = rendered_chat(&mut app);
    assert!(
        !plain.contains('▌'),
        "TurnEnd should clear the final streaming cursor: {plain}"
    );

    app.handle_event(Event::Token {
        token: "Again".into(),
    });
    app.handle_event(Event::AgentStart);
    app.handle_event(Event::AgentEnd);
    plain = rendered_chat(&mut app);
    assert!(
        !plain.contains('▌'),
        "AgentEnd should clear the final streaming cursor: {plain}"
    );
}
