// Cron tool: create and manage scheduled tasks from the agent.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Tool that lets the agent manage cron jobs (add, remove, list, enable, disable).
pub struct CronTool {
    store: Arc<dyn CronStore>,
}

impl std::fmt::Debug for CronTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CronTool").finish()
    }
}

impl CronTool {
    pub fn new(store: Arc<dyn CronStore>) -> Self {
        Self { store }
    }

    fn handle_action(&self, arguments: &str) -> Result<String, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: action")?;

        match action {
            "add" => self.handle_add(&args),
            "remove" => self.handle_remove(&args),
            "list" => self.handle_list(),
            "enable" => self.handle_set_enabled(&args, true),
            "disable" => self.handle_set_enabled(&args, false),
            _ => Err(format!("unknown action: {}", action)),
        }
    }

    fn handle_add(&self, args: &serde_json::Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("missing field: name")?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or("missing field: message")?;
        let deliver_to = args
            .get("deliver_to")
            .and_then(|v| v.as_str())
            .map(String::from);

        let schedule = if let Some(expr) = args.get("cron_expression").and_then(|v| v.as_str()) {
            CronSchedule::Cron {
                expression: expr.to_string(),
            }
        } else if let Some(secs) = args.get("interval_seconds").and_then(|v| v.as_u64()) {
            CronSchedule::Interval { seconds: secs }
        } else {
            return Err("must provide either cron_expression or interval_seconds".to_string());
        };

        let job = CronJob {
            id: name.to_lowercase().replace(' ', "-"),
            name: name.to_string(),
            message: message.to_string(),
            schedule,
            enabled: true,
            deliver_to,
            last_error: None,
        };

        self.store.add(job).map_err(|e| e.to_string())?;
        Ok(format!("Job '{}' added successfully.", name))
    }

    fn handle_remove(&self, args: &serde_json::Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("missing field: name")?;

        let id = name.to_lowercase().replace(' ', "-");
        self.store.remove(&id).map_err(|e| e.to_string())?;
        Ok(format!("Job '{}' removed.", name))
    }

    fn handle_list(&self) -> Result<String, String> {
        let jobs = self.store.list().map_err(|e| e.to_string())?;
        if jobs.is_empty() {
            return Ok("No cron jobs configured.".to_string());
        }
        let mut out = format!("{} cron job(s):\n", jobs.len());
        for job in &jobs {
            let status = if job.enabled { "enabled" } else { "disabled" };
            let sched = match &job.schedule {
                CronSchedule::Interval { seconds } => format!("every {}s", seconds),
                CronSchedule::Cron { expression } => format!("cron: {}", expression),
            };
            out.push_str(&format!("- {} [{}] ({})\n", job.name, status, sched));
        }
        Ok(out)
    }

    fn handle_set_enabled(
        &self,
        args: &serde_json::Value,
        enabled: bool,
    ) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("missing field: name")?;

        let id = name.to_lowercase().replace(' ', "-");
        self.store
            .set_enabled(&id, enabled)
            .map_err(|e| e.to_string())?;
        let action = if enabled { "enabled" } else { "disabled" };
        Ok(format!("Job '{}' {}.", name, action))
    }
}

impl Tool for CronTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cron".to_string(),
            description: "Manage scheduled cron jobs (add, remove, list, enable, disable)"
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["add","remove","list","enable","disable"],"description":"The cron action to perform"},"name":{"type":"string","description":"Job name (for add/remove/enable/disable)"},"message":{"type":"string","description":"The message/prompt to execute (for add)"},"interval_seconds":{"type":"integer","description":"Interval in seconds (for add with interval)"},"cron_expression":{"type":"string","description":"Cron expression (for add with cron schedule)"},"deliver_to":{"type":"string","description":"Optional channel:chat_id for result delivery"}},"required":["action"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            match self.handle_action(&args) {
                Ok(content) => Ok(ToolResult {
                    content,
                    is_error: false,
                }),
                Err(e) => Ok(ToolResult {
                    content: e,
                    is_error: true,
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::persistence::cron_store::FileCronStore;
    use tempfile::TempDir;

    fn test_tool() -> (CronTool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(FileCronStore::new(tmp.path()));
        (CronTool::new(store), tmp)
    }

    #[tokio::test]
    async fn test_add_interval_job() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Hourly Check","message":"Check health","interval_seconds":3600}"#,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("added"));
    }

    #[tokio::test]
    async fn test_add_cron_job() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Daily Report","message":"Generate report","cron_expression":"0 9 * * *"}"#,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("added"));
    }

    #[tokio::test]
    async fn test_list_empty() {
        let (tool, _tmp) = test_tool();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No cron jobs"));
    }

    #[tokio::test]
    async fn test_list_with_jobs() {
        let (tool, _tmp) = test_tool();
        tool.execute(
            r#"{"action":"add","name":"Test Job","message":"test","interval_seconds":60}"#,
        )
        .await
        .unwrap();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Test Job"));
    }

    #[tokio::test]
    async fn test_disable_job() {
        let (tool, _tmp) = test_tool();
        tool.execute(r#"{"action":"add","name":"My Job","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        let result = tool
            .execute(r#"{"action":"disable","name":"My Job"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("disabled"));
    }

    #[tokio::test]
    async fn test_remove_job() {
        let (tool, _tmp) = test_tool();
        tool.execute(
            r#"{"action":"add","name":"Temp Job","message":"test","interval_seconds":60}"#,
        )
        .await
        .unwrap();
        let result = tool
            .execute(r#"{"action":"remove","name":"Temp Job"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("removed"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let (tool, _tmp) = test_tool();
        let result = tool.execute(r#"{"name":"test"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let (tool, _tmp) = test_tool();
        let result = tool.execute(r#"{"action":"purge"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("unknown action"));
    }

    #[tokio::test]
    async fn test_add_missing_name() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(r#"{"action":"add","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("name"));
    }

    #[tokio::test]
    async fn test_add_missing_schedule() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(r#"{"action":"add","name":"Job","message":"test"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result
                .content
                .contains("cron_expression or interval_seconds")
        );
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let (tool, _tmp) = test_tool();
        let result = tool.execute("not json").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn test_enable_job() {
        let (tool, _tmp) = test_tool();
        tool.execute(r#"{"action":"add","name":"My Job","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        tool.execute(r#"{"action":"disable","name":"My Job"}"#)
            .await
            .unwrap();
        let result = tool
            .execute(r#"{"action":"enable","name":"My Job"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("enabled"));
    }

    #[tokio::test]
    async fn test_add_with_deliver_to() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Notify","message":"check","interval_seconds":60,"deliver_to":"telegram:123"}"#,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("added"));
    }

    #[test]
    fn test_definition() {
        let (tool, _tmp) = test_tool();
        let def = tool.definition();
        assert_eq!(def.name, "cron");
        assert!(def.description.contains("cron"));
    }

    #[test]
    fn test_debug_format() {
        let (tool, _tmp) = test_tool();
        let debug = format!("{:?}", tool);
        assert!(debug.contains("CronTool"));
    }
}
