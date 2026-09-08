//! Fake-provider end-to-end run through the real agent loop and swarm tool.
use quecto::application::agent_loop::AgentLoopImpl;
use quecto::domain::{
    agent::AgentLoop,
    message::{LlmResponse, Message, ToolCall},
};
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::{
    registry::ToolRegistryImpl,
    swarm::{SwarmConfig, SwarmTool},
    swarm_bridge::SwarmContext,
};
use std::sync::Arc;

fn action(id: usize, source: &str) -> LlmResponse {
    let mut response = text_response("");
    response.content = None;
    response.tool_calls.push(ToolCall {
        id: format!("swarm-{id}"),
        name: "swarm".into(),
        arguments: serde_json::json!({"op":"run","code":source}).to_string(),
    });
    response
}

#[tokio::test]
async fn fake_provider_decomposes_resolves_blocker_and_verifies_swarm() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Arc::new(directory.path().to_path_buf());
    let context = SwarmContext {
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    context.create_run(&serde_json::json!({"goal":"ship","constraints":[],"criteria":[{"id":"tests","kind":"command","description":"pass"}],"member_limit":1,"deadline":deadline}),
        &quecto::domain::swarm::ProcessIdentity { pid: std::process::id(), started: quecto::infrastructure::tools::swarm_bridge::process_start(std::process::id()).unwrap() }, None).unwrap();
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context));
    let provider = Arc::new(MockProvider::new(vec![
        action(
            1,
            "from swarm import board\nboard.amend('ship feature',[],[{'id':'tests','kind':'command','description':'acceptance passes'},{'id':'review','kind':'review','description':'independent review'}],'define verification')\na=board.task_create('build','implement',['acceptance passes'],[])\nboard.task_create('review-task','review',['independent review'],[a['id']])\nc=board.claim(a['id']); board.block(a['id'],c['token'],'need schema')",
        ),
        action(
            2,
            "from swarm import board\nt=board.task(1)\nboard.send('schema','coordinator','schema approved')\nboard.ack(board.inbox()[0]['id'])\nopen('acceptance.log','w').write('PASS revision abc')\nboard.submit(1,t['token'],[{'artifact':'acceptance.log','revision':'abc'}])\nboard.verify_task(1,t['token'],'abc')",
        ),
        action(
            3,
            "from swarm import board\nc=board.claim(2)\nopen('review.md','w').write('Reviewed revision abc: approved')\nboard.submit(2,c['token'],[{'artifact':'review.md','revision':'abc'}])\nboard.verify_task(2,c['token'],'abc')\nboard.evidence('tests','acceptance.log','abc','command',True)\nboard.evidence('review','review.md','abc','review',True)\nboard.complete('abc')",
        ),
        text_response("Verified completion at abc"),
    ]));
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(tool));
    let mut agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry)));
    let result = agent
        .process(&mut vec![Message::user("Complete the bounded swarm")])
        .await
        .unwrap();
    assert!(result.response.contains("Verified completion"));
    let summary = SwarmContext {
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    }
    .summary()
    .unwrap();
    assert_eq!(summary["status"], "succeeded");
    assert_eq!(summary["counts"]["completed"], 2);
    assert!(
        summary["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "blocked")
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("acceptance.log")).unwrap(),
        "PASS revision abc"
    );
    assert_eq!(provider.request_count(), 4);
}

#[derive(Debug)]
struct MockProvider {
    responses: std::sync::Mutex<std::collections::VecDeque<LlmResponse>>,
    requests: std::sync::atomic::AtomicUsize,
}

impl MockProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into()),
            requests: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn request_count(&self) -> usize {
        self.requests.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl quecto::domain::provider::LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "swarm-fake"
    }
    fn chat(
        &self,
        _: quecto::domain::provider::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, quecto::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        self.requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider request");
        Box::pin(async move { Ok(response) })
    }
}

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn test_config(
    provider: Arc<MockProvider>,
    tool_registry: Box<ToolRegistryImpl>,
) -> quecto::application::agent_loop::AgentLoopConfig {
    quecto::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry,
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    }
}
