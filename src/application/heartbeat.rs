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
}
