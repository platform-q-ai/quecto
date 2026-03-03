// Cron tool: create and manage scheduled tasks from the agent.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Current time as Unix seconds (for display purposes).
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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
        let run_once = args
            .get("run_once")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let schedule = if let Some(expr) = args.get("cron_expression").and_then(|v| v.as_str()) {
            CronSchedule::Cron {
                expression: expr.to_string(),
            }
        } else if let Some(secs) = args.get("interval_seconds").and_then(|v| v.as_u64()) {
            if secs == 0 {
                return Err("interval_seconds must be greater than 0".to_string());
            }
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
            run_once,
        };

        // Atomic check-and-insert to avoid TOCTOU race between find_by_name + add.
        let added = self.store.add_if_absent(job).map_err(|e| e.to_string())?;
        if !added {
            return Err(format!("job '{}' already exists", name));
        }
        Ok(format!("Job '{}' added successfully.", name))
    }

    fn handle_remove(&self, args: &serde_json::Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("missing field: name")?;

        let job = self
            .store
            .find_by_name(name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("job '{}' not found", name))?;
        self.store.remove(&job.id).map_err(|e| e.to_string())?;
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
            let last_run = if job.last_run_at == 0 {
                "never".to_string()
            } else {
                format!("{}s ago", now_unix_secs().saturating_sub(job.last_run_at))
            };
            let once_tag = if job.run_once { " [one-shot]" } else { "" };
            let mut line = format!(
                "- {}{} [{}] ({}) last_run: {}",
                job.name, once_tag, status, sched, last_run
            );
            if let Some(ref err) = job.last_error {
                line.push_str(&format!(" last_error: {}", err));
            }
            if let Some(ref target) = job.deliver_to {
                line.push_str(&format!(" deliver_to: {}", target));
            }
            line.push('\n');
            out.push_str(&line);
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

        let job = self
            .store
            .find_by_name(name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("job '{}' not found", name))?;
        self.store
            .set_enabled(&job.id, enabled)
            .map_err(|e| e.to_string())?;
        let action = if enabled { "enabled" } else { "disabled" };
        Ok(format!("Job '{}' {}.", name, action))
    }
}

impl Tool for CronTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cron".into(),
            description: "Manage scheduled cron jobs (add, remove, list, enable, disable)"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["add","remove","list","enable","disable"],"description":"The cron action to perform"},"name":{"type":"string","description":"Job name (for add/remove/enable/disable)"},"message":{"type":"string","description":"The message/prompt to execute (for add)"},"interval_seconds":{"type":"integer","description":"Interval in seconds (for add with interval)"},"cron_expression":{"type":"string","description":"Cron expression (for add with cron schedule)"},"deliver_to":{"type":"string","description":"Optional delivery target in format 'telegram:<chat_id>', e.g. 'telegram:123456789'"},"run_once":{"type":"boolean","description":"If true, job auto-removes after one successful execution (for reminders/delayed actions)"}},"required":["action"]}"#.into(),
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
                    image_blocks: vec![],
                }),
                Err(e) => Ok(ToolResult {
                    content: e,
                    is_error: true,
                    image_blocks: vec![],
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
    async fn test_add_run_once_job() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Reminder","message":"Call dentist","interval_seconds":1800,"run_once":true}"#,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("added"));
        // Verify the stored job has run_once set
        let store = tool.store.clone();
        let job = store.find_by_name("Reminder").unwrap().unwrap();
        assert!(job.run_once, "job should have run_once=true");
    }

    #[tokio::test]
    async fn test_add_job_without_run_once_defaults_to_false() {
        let (tool, _tmp) = test_tool();
        let result = tool
            .execute(
                r#"{"action":"add","name":"Recurring","message":"check","interval_seconds":60}"#,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let store = tool.store.clone();
        let job = store.find_by_name("Recurring").unwrap().unwrap();
        assert!(!job.run_once, "job should default to run_once=false");
    }

    #[tokio::test]
    async fn test_list_shows_one_shot_for_run_once_jobs() {
        let (tool, _tmp) = test_tool();
        tool.execute(
            r#"{"action":"add","name":"Reminder","message":"test","interval_seconds":1800,"run_once":true}"#,
        )
        .await
        .unwrap();
        let result = tool.execute(r#"{"action":"list"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("one-shot"),
            "list output should show 'one-shot' for run_once jobs, got: {}",
            result.content
        );
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
