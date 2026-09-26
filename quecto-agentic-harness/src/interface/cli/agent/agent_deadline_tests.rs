//! #2168: `--max-time` stops the run at the deadline; nothing it started
//! goes on acting after it.
use super::*;
use crate::application::agent_loop::AgentLoopConfig;
use crate::composition::runtime::build_agent_provider;
use crate::infrastructure::config::Config;
use crate::infrastructure::security::sandbox::Sandbox;
use std::time::{Duration, Instant};

/// An agent whose provider, at `api_base`, answers with a bash call.
fn agent_at(api_base: &str, base_dir: &std::path::Path) -> AgentLoopImpl {
    let config_file = base_dir.join("config.json");
    std::fs::write(
        &config_file,
        format!(r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"{api_base}"}}}}}}"#),
    )
    .unwrap();
    let config = Config::load(config_file.to_str().unwrap()).unwrap();
    let provider = build_agent_provider(&config, base_dir, &reqwest::Client::new()).unwrap();
    let workspace = base_dir.to_path_buf();
    let sandbox = Sandbox::new(Some(workspace.clone()));
    let registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        crate::composition::find::build_find_tool(
            std::sync::Arc::new(workspace.clone()),
            std::sync::Arc::new(sandbox.clone()),
        ),
        workspace,
        sandbox,
        Default::default(),
    );
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(3)
}

/// A tool still running at the deadline is stopped with the run: the call
/// returns at the deadline, and the command never finishes its work.
#[test]
fn a_tool_running_at_the_deadline_is_stopped_with_the_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let marker = tmp.path().join("finished");
    let server_rt = tokio::runtime::Runtime::new().unwrap();
    let server = server_rt.block_on(wiremock::MockServer::start());
    let arguments = serde_json::json!({
        "command": format!("sleep 3 && touch {}", marker.display())
    })
    .to_string();
    let reply = serde_json::json!({
        "id": "r", "object": "chat.completion",
        "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
            "role": "assistant", "content": null,
            "tool_calls": [{"id": "c1", "type": "function",
                "function": {"name": "bash", "arguments": arguments}}]}}]
    });
    server_rt.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(reply))
            .mount(&server),
    );
    let mut agent = agent_at(&server.uri(), tmp.path());
    let rt = crate::interface::cli::build_tokio_runtime().unwrap();
    let mut messages = vec![Message::user("run it")];

    let started = Instant::now();
    let result = run_with_deadline(&rt, &mut agent, &mut messages, 1);
    let elapsed = started.elapsed();

    assert!(
        matches!(result, DeadlineResult::TimedOut),
        "the run outlived its deadline"
    );
    assert!(
        elapsed < Duration::from_millis(2500),
        "returned after {elapsed:?}"
    );
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        !marker.exists(),
        "the command went on acting after the deadline"
    );
}

/// A provider that never answers is abandoned at the deadline.
#[test]
fn a_provider_that_never_answers_is_abandoned_at_the_deadline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Accepts, then holds each connection without answering for 10 s.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().take(4) {
            held.push(stream);
        }
        std::thread::sleep(Duration::from_secs(10));
    });
    let mut agent = agent_at(&format!("http://{address}"), tmp.path());
    let rt = crate::interface::cli::build_tokio_runtime().unwrap();
    let mut messages = vec![Message::user("hello")];

    let started = Instant::now();
    let result = run_with_deadline(&rt, &mut agent, &mut messages, 1);
    let elapsed = started.elapsed();

    assert!(matches!(result, DeadlineResult::TimedOut));
    assert!(
        elapsed < Duration::from_millis(2500),
        "returned after {elapsed:?}"
    );
}
