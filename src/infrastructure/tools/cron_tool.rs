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

        // Validate deliver_to format if provided.
        if let Some(ref target) = deliver_to {
            crate::domain::channel::validate_deliver_to(target)?;
        }

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
            last_run_at: 0,
            created_at: now_unix_secs(),
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

            // Diagnostics
            if job.last_run_at > 0 {
                out.push_str(&format!(
                    "  last_run: {}\n",
                    format_timestamp(job.last_run_at)
                ));
            } else {
                out.push_str("  last_run: never\n");
            }
            if let Some(ref err) = job.last_error {
                out.push_str(&format!("  last_error: {}\n", err));
            }
            if job.created_at > 0 {
                out.push_str(&format!(
                    "  created: {}\n",
                    format_timestamp(job.created_at)
                ));
            }
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

/// Current time as Unix seconds.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Format a Unix timestamp as a simple UTC datetime string.
fn format_timestamp(secs: u64) -> String {
    // Use chrono for clean formatting.
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("{}s", secs))
}

impl Tool for CronTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cron".to_string(),
            description: "Manage scheduled cron jobs (add, remove, list, enable, disable)"
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["add","remove","list","enable","disable"],"description":"The cron action to perform"},"name":{"type":"string","description":"Job name (for add/remove/enable/disable)"},"message":{"type":"string","description":"The message/prompt to execute (for add)"},"interval_seconds":{"type":"integer","description":"Interval in seconds (for add with interval)"},"cron_expression":{"type":"string","description":"Cron expression (for add with cron schedule)"},"deliver_to":{"type":"string","description":"Optional delivery target in format 'telegram:<chat_id>', e.g. 'telegram:12345'"}},"required":["action"]}"#.to_string(),
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

    // -----------------------------------------------------------------------
    // Fix 2: created_at should be set automatically when adding a job
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_add_job_sets_created_at() {
        let (tool, _tmp) = test_tool();
        tool.execute(
            r#"{"action":"add","name":"Timestamped","message":"test","interval_seconds":60}"#,
        )
        .await
        .unwrap();
        let store = Arc::new(FileCronStore::new(_tmp.path()));
        let job = store.find_by_name("Timestamped").unwrap().unwrap();
        assert!(
            job.created_at > 0,
            "created_at should be set to current timestamp, got {}",
            job.created_at
        );
    }

    // -----------------------------------------------------------------------
    // Fix 3: list output includes diagnostics
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_shows_last_error() {
        let (tool, _tmp) = test_tool();
        tool.execute(r#"{"action":"add","name":"Failing","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        // Simulate an error on the job
        let store = FileCronStore::new(_tmp.path());
        let job = store.find_by_name("Failing").unwrap().unwrap();
        store
            .set_last_error(&job.id, Some("timeout".to_string()))
            .unwrap();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(
            result.content.contains("last_error"),
            "list output should include last_error field, got: {}",
            result.content
        );
        assert!(
            result.content.contains("timeout"),
            "list output should show the actual error value, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_shows_last_run_at() {
        let (tool, _tmp) = test_tool();
        tool.execute(r#"{"action":"add","name":"Runner","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        let store = FileCronStore::new(_tmp.path());
        let job = store.find_by_name("Runner").unwrap().unwrap();
        store.set_last_run_at(&job.id, 1_700_000_000).unwrap();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(
            result.content.contains("last_run"),
            "list output should include last_run info, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_list_shows_created_at() {
        let (tool, _tmp) = test_tool();
        tool.execute(r#"{"action":"add","name":"Dated","message":"test","interval_seconds":60}"#)
            .await
            .unwrap();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(
            result.content.contains("created"),
            "list output should include created_at info, got: {}",
            result.content
        );
    }

    // -----------------------------------------------------------------------
    // Fix 4: deliver_to validation at add-time
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_add_rejects_invalid_deliver_to() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Bad","message":"test","interval_seconds":60,"deliver_to":"current"}"#,
            )
            .await
            .unwrap();
        assert!(
            result.is_error,
            "should reject invalid deliver_to 'current'"
        );
        assert!(
            result.content.contains("telegram:"),
            "error should suggest valid format, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_add_accepts_valid_deliver_to() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Good","message":"test","interval_seconds":60,"deliver_to":"telegram:12345"}"#,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "should accept valid deliver_to 'telegram:12345', got error: {}",
            result.content
        );
    }

    // -----------------------------------------------------------------------
    // Fix 5: tool description includes deliver_to format examples
    // -----------------------------------------------------------------------

    #[test]
    fn test_definition_includes_deliver_to_example() {
        let (tool, _tmp) = test_tool();
        let def = tool.definition();
        assert!(
            def.parameters_schema.contains("telegram:") || def.description.contains("telegram:"),
            "tool definition should include deliver_to format example (telegram:chat_id), got schema: {}",
            def.parameters_schema
        );
    }
}
