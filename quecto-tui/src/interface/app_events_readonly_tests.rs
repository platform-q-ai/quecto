use super::*;
use crate::infrastructure::terminal::Terminal;
use tokio::io::AsyncReadExt;

async fn test_app() -> App {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-readonly-events-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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

#[tokio::test]
async fn spawn_start_tracks_read_only_observer_args() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-ro".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"reviewer","read_only":true}),
    });
    assert!(app.subagent_local["reviewer"].info.read_only);

    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-disabled".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"observer","disable_tools":["edit","write"]}),
    });
    assert!(app.subagent_local["observer"].info.read_only);
}

#[tokio::test]
async fn spawn_start_does_not_mark_partial_disable_as_observer() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-partial".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"writer","disable_tools":["write"]}),
    });
    assert!(!app.subagent_local["writer"].info.read_only);
}
