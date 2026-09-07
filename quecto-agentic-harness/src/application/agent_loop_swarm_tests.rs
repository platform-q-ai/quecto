//! Fake-provider end-to-end run through the real agent loop and swarm tool.
use super::{MockProvider, MockRegistry, test_config, text_response};
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::{
    agent::AgentLoop,
    message::{LlmResponse, Message, ToolCall},
};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::{
    swarm::SwarmConfig, swarm_bridge::SwarmContext, swarm_test_support,
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
    let tool = swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    );
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
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(tool));
    let mut agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry)));
    let result = agent
        .process(&mut vec![Message::user("Complete the bounded swarm")])
        .await
        .unwrap();
    assert!(result.response.contains("Verified completion"));
    let summary = SwarmContext {
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
