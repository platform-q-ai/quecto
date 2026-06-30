use super::*;
use crate::infrastructure::terminal::Terminal;
use tokio::io::AsyncReadExt;

async fn test_app() -> App {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-app-events-cursor-test-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    });
    let client = Client::connect(&socket_path).await.unwrap();
    App::new(Terminal::new(), client)
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

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
        tool_results: vec![],
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
    app.handle_event(Event::AgentEnd { messages: vec![] });
    plain = rendered_chat(&mut app);
    assert!(
        !plain.contains('▌'),
        "AgentEnd should clear the final streaming cursor: {plain}"
    );
}
