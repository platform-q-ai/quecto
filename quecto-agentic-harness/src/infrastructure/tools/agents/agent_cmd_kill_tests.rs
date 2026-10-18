// #1936: `agent_cmd kill` is owned by the composed selected-termination tool;
// this tool only routes the command to it and never decides a lifecycle.
use super::*;
use std::sync::{Arc, Mutex};

struct RecordingKill {
    seen: Arc<Mutex<Vec<String>>>,
}

impl Tool for RecordingKill {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd:kill".into(),
            description: String::new().into(),
            parameters_schema: "{}".into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        self.seen.lock().unwrap().push(arguments.to_owned());
        Box::pin(async move {
            Ok(ToolResult {
                content: r#"{"result":"graceful","killed":["w1"]}"#.into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[tokio::test]
async fn kill_is_delegated_verbatim_to_the_composed_tool() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let tool = AgentCmdTool::new(new_registry())
        .with_kill_tool(Arc::new(RecordingKill { seen: seen.clone() }));
    let arguments = r#"{"agent_id":"w1","command":"kill"}"#;
    let result = tool.execute(arguments).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("graceful"));
    assert_eq!(seen.lock().unwrap().as_slice(), [arguments]);
}

#[tokio::test]
async fn kill_without_a_composed_owner_is_refused() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"kill"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("kill is not available"));
    assert!(format!("{tool:?}").contains("kill: false"));
}

#[tokio::test]
async fn other_commands_never_reach_the_kill_owner() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let tool = AgentCmdTool::new(new_registry())
        .with_kill_tool(Arc::new(RecordingKill { seen: seen.clone() }));
    let result = tool
        .execute(r#"{"agent_id":"*","command":"get_subagents_all"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(seen.lock().unwrap().is_empty());
}
