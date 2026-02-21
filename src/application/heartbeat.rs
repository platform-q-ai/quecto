// Heartbeat service: reads HEARTBEAT.md, parses tasks, dispatches.

use std::path::Path;

use crate::domain::error::DomainError;

/// A parsed heartbeat task from HEARTBEAT.md.
#[derive(Debug, Clone)]
pub struct HeartbeatTask {
    /// The task description / message to send to the agent.
    pub message: String,
    /// Whether this task should be spawned as a subagent.
    pub use_spawn: bool,
}

/// Result of a heartbeat run.
#[derive(Debug)]
pub struct HeartbeatResult {
    pub tasks_found: usize,
    pub tasks_executed: usize,
    pub ok: bool,
}

impl HeartbeatResult {
    pub fn status(&self) -> &str {
        if self.ok {
            "HEARTBEAT_OK"
        } else {
            "HEARTBEAT_FAIL"
        }
    }
}

/// Parse HEARTBEAT.md content into a list of tasks.
/// Lines starting with `- ` are tasks. If under a section header
/// containing "spawn", they are marked as `use_spawn`.
pub fn parse_heartbeat(content: &str) -> Vec<HeartbeatTask> {
    let mut tasks = Vec::new();
    let mut in_spawn_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("##") {
            in_spawn_section = trimmed.to_lowercase().contains("spawn");
            continue;
        }
        if let Some(task_text) = trimmed.strip_prefix("- ") {
            let task_text = task_text.trim();
            if !task_text.is_empty() {
                tasks.push(HeartbeatTask {
                    message: task_text.to_string(),
                    use_spawn: in_spawn_section,
                });
            }
        }
    }

    tasks
}

/// Read HEARTBEAT.md from the workspace and parse tasks.
/// Returns an empty list if the file doesn't exist.
pub async fn load_tasks(workspace: impl AsRef<Path>) -> Result<Vec<HeartbeatTask>, DomainError> {
    let path = workspace.as_ref().join("HEARTBEAT.md");
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| DomainError::Other(format!("failed to read HEARTBEAT.md: {}", e)))?;
    Ok(parse_heartbeat(&content))
}

/// Result of dispatching a single heartbeat task.
#[derive(Debug)]
pub struct HeartbeatTaskResult {
    /// The original task message.
    pub message: String,
    /// Whether the task was dispatched via spawn (subagent).
    pub dispatched_via_spawn: bool,
    /// The response from the agent (or indication of spawn).
    pub response: String,
}

/// Execute a heartbeat tick: load tasks from workspace, dispatch each
/// through the agent (or via spawn for `use_spawn` tasks).
///
/// Returns the list of dispatched task results, or an empty list
/// if no HEARTBEAT.md exists.
pub async fn execute_heartbeat_tick(
    workspace: &Path,
    agent: &dyn crate::domain::agent::AgentLoop,
) -> Result<Vec<HeartbeatTaskResult>, DomainError> {
    let tasks = load_tasks(workspace).await?;
    if tasks.is_empty() {
        return Ok(vec![]);
    }

    let mut results = Vec::new();
    for task in &tasks {
        let result = dispatch_task(agent, task).await;
        results.push(result);
    }
    Ok(results)
}

/// Dispatch a single heartbeat task.
async fn dispatch_task(
    agent: &dyn crate::domain::agent::AgentLoop,
    task: &HeartbeatTask,
) -> HeartbeatTaskResult {
    if task.use_spawn {
        // For spawn tasks, we send the message through the agent
        // with a system prompt instructing spawn tool usage.
        // The agent loop will invoke the spawn tool if available.
        let mut messages = vec![crate::domain::message::Message {
            role: crate::domain::message::Role::User,
            content: format!("Spawn a subagent to handle this task: {}", task.message),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        match agent.process(&mut messages).await {
            Ok(result) => HeartbeatTaskResult {
                message: task.message.clone(),
                dispatched_via_spawn: true,
                response: result.response,
            },
            Err(e) => HeartbeatTaskResult {
                message: task.message.clone(),
                dispatched_via_spawn: true,
                response: format!("spawn error: {}", e),
            },
        }
    } else {
        let mut messages = vec![crate::domain::message::Message {
            role: crate::domain::message::Role::User,
            content: task.message.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        match agent.process(&mut messages).await {
            Ok(result) => HeartbeatTaskResult {
                message: task.message.clone(),
                dispatched_via_spawn: false,
                response: result.response,
            },
            Err(e) => HeartbeatTaskResult {
                message: task.message.clone(),
                dispatched_via_spawn: false,
                response: format!("error: {}", e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_tasks() {
        let content = "- Check the weather\n- Report time\n";
        let tasks = parse_heartbeat(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].message, "Check the weather");
        assert_eq!(tasks[1].message, "Report time");
        assert!(!tasks[0].use_spawn);
    }

    #[test]
    fn test_parse_spawn_section() {
        let content = "## Long Tasks (use spawn for async)\n- Search news\n- Analyze data\n";
        let tasks = parse_heartbeat(content);
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].use_spawn);
        assert!(tasks[1].use_spawn);
    }

    #[test]
    fn test_parse_mixed_sections() {
        let content = "\
- Quick task\n\
## Long Tasks (use spawn)\n\
- Slow task\n\
## Regular\n\
- Another quick task\n";
        let tasks = parse_heartbeat(content);
        assert_eq!(tasks.len(), 3);
        assert!(!tasks[0].use_spawn);
        assert!(tasks[1].use_spawn);
        assert!(!tasks[2].use_spawn);
    }

    #[test]
    fn test_parse_empty() {
        let tasks = parse_heartbeat("");
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_parse_no_tasks() {
        let content = "# Heartbeat\n\nSome description text\n";
        let tasks = parse_heartbeat(content);
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_heartbeat_result_ok() {
        let result = HeartbeatResult {
            tasks_found: 2,
            tasks_executed: 2,
            ok: true,
        };
        assert_eq!(result.status(), "HEARTBEAT_OK");
    }

    #[test]
    fn test_heartbeat_result_fail() {
        let result = HeartbeatResult {
            tasks_found: 2,
            tasks_executed: 1,
            ok: false,
        };
        assert_eq!(result.status(), "HEARTBEAT_FAIL");
    }

    #[tokio::test]
    async fn test_load_tasks_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tasks = load_tasks(tmp.path()).await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn test_load_tasks_from_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("HEARTBEAT.md"),
            "- Check weather\n- Report time\n",
        )
        .unwrap();
        let tasks = load_tasks(tmp.path()).await.unwrap();
        assert_eq!(tasks.len(), 2);
    }

    // -- Mock agent for heartbeat tick tests --

    use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
    use crate::domain::message::{Message, Role};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    struct RecordingAgent {
        response: String,
        received: Mutex<Vec<String>>,
    }

    impl RecordingAgent {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                received: Mutex::new(Vec::new()),
            }
        }
    }

    impl AgentLoop for RecordingAgent {
        fn process<'a>(
            &'a self,
            messages: &'a mut Vec<Message>,
        ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
            let user_msg = messages
                .iter()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone())
                .unwrap_or_default();
            self.received.lock().unwrap().push(user_msg);
            let resp = self.response.clone();
            Box::pin(async move { Ok(AgentResult::text(resp)) })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    #[tokio::test]
    async fn test_execute_heartbeat_tick_dispatches_tasks() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("HEARTBEAT.md"),
            "- Check health\n- Report disk\n",
        )
        .unwrap();

        let agent = RecordingAgent::new("done");
        let results = execute_heartbeat_tick(tmp.path(), &agent).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].message, "Check health");
        assert_eq!(results[1].message, "Report disk");
        assert!(!results[0].dispatched_via_spawn);

        let received = agent.received.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert!(received[0].contains("Check health"));
    }

    #[tokio::test]
    async fn test_execute_heartbeat_tick_no_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let agent = RecordingAgent::new("done");
        let results = execute_heartbeat_tick(tmp.path(), &agent).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_heartbeat_tick_spawn_tasks() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("HEARTBEAT.md"),
            "## Long Tasks (use spawn)\n- Analyze data\n",
        )
        .unwrap();

        let agent = RecordingAgent::new("spawned");
        let results = execute_heartbeat_tick(tmp.path(), &agent).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].dispatched_via_spawn);
        assert_eq!(results[0].message, "Analyze data");

        let received = agent.received.lock().unwrap();
        assert!(received[0].contains("Analyze data"));
        assert!(received[0].contains("Spawn"));
    }
}
