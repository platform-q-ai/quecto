// /heartbeat command handler for the REPL.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use super::{CMD_HEARTBEAT, ReplLoop};

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    pub(super) fn handle_heartbeat(&mut self, input: &str) {
        let rest = input.strip_prefix(CMD_HEARTBEAT).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "show" => self.heartbeat_show(),
            "add" => self.heartbeat_add(args_str),
            "remove" => self.heartbeat_remove(args_str),
            "enable" => self.heartbeat_enable(),
            "disable" => self.heartbeat_disable(),
            "interval" => self.heartbeat_interval(args_str),
            "status" => self.heartbeat_status(),
            _ => self.heartbeat_usage(),
        }
    }

    fn heartbeat_md_path(&self) -> PathBuf {
        let config = self.load_config();
        let workspace = config.map(|c| c.workspace_path()).unwrap_or_else(|| {
            self.session
                .base_dir
                .join("workspace")
                .to_string_lossy()
                .to_string()
        });
        PathBuf::from(workspace).join("HEARTBEAT.md")
    }

    fn heartbeat_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /heartbeat <subcommand>");
        let _ = writeln!(self.writer, "  show       Show current heartbeat tasks");
        let _ = writeln!(self.writer, "  add        Add a new task");
        let _ = writeln!(self.writer, "  remove     Remove a task by text");
        let _ = writeln!(self.writer, "  enable     Enable heartbeat in config");
        let _ = writeln!(self.writer, "  disable    Disable heartbeat in config");
        let _ = writeln!(self.writer, "  interval   Set heartbeat interval (seconds)");
        let _ = writeln!(self.writer, "  status     Show heartbeat configuration");
    }

    fn heartbeat_show(&mut self) {
        let path = self.heartbeat_md_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(self.writer, "No heartbeat tasks configured");
                return;
            }
        };

        let tasks = crate::application::heartbeat::parse_heartbeat(&content);
        if tasks.is_empty() {
            let _ = writeln!(self.writer, "No heartbeat tasks configured");
            return;
        }

        let _ = writeln!(self.writer, "Heartbeat tasks:");
        for task in &tasks {
            if task.use_spawn {
                let _ = writeln!(self.writer, "  - {} [spawn]", task.message);
            } else {
                let _ = writeln!(self.writer, "  - {}", task.message);
            }
        }
        let _ = writeln!(self.writer, "{} tasks", tasks.len());
    }

    fn heartbeat_add(&mut self, args_str: &str) {
        let args_str = args_str.trim();
        let use_spawn = args_str.starts_with("--spawn");
        let task_text = if use_spawn {
            args_str.strip_prefix("--spawn").unwrap_or("").trim()
        } else {
            args_str
        };

        if task_text.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let path = self.heartbeat_md_path();

        // Read existing content or start fresh
        let mut content = std::fs::read_to_string(&path).unwrap_or_default();

        if use_spawn {
            // Ensure there's a spawn section header
            if !content.to_lowercase().contains("spawn") {
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str("## Long Tasks (use spawn)\n");
            }
            content.push_str(&format!("- {}\n", task_text));
        } else {
            // Find insertion point: before any ## header, or at end
            let insert_pos = content.find("##").unwrap_or(content.len());
            let line = format!("- {}\n", task_text);
            content.insert_str(insert_pos, &line);
        }

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::write(&path, &content) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Task added: {}", task_text);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_remove(&mut self, args_str: &str) {
        let needle = args_str.trim();
        if needle.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let path = self.heartbeat_md_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(self.writer, "Error: task '{}' not found", needle);
                return;
            }
        };

        let mut found = false;
        let mut new_lines = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if !found
                && trimmed.starts_with("- ")
                && trimmed
                    .strip_prefix("- ")
                    .is_some_and(|t| t.trim() == needle)
            {
                found = true;
                continue; // Skip this line
            }
            new_lines.push(line);
        }

        if !found {
            let _ = writeln!(self.writer, "Error: task '{}' not found", needle);
            return;
        }

        let new_content = new_lines.join("\n") + "\n";
        match std::fs::write(&path, new_content) {
            Ok(()) => {
                let _ = writeln!(self.writer, "Task removed: {}", needle);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_enable(&mut self) {
        self.set_heartbeat_enabled(true);
    }

    fn heartbeat_disable(&mut self) {
        self.set_heartbeat_enabled(false);
    }

    fn set_heartbeat_enabled(&mut self, enabled: bool) {
        let config_path = self.session.base_dir.join("config.json");
        match self.read_config_json(&config_path) {
            Ok(mut config) => {
                if config.get("heartbeat").is_none() {
                    config["heartbeat"] = serde_json::json!({});
                }
                config["heartbeat"]["enabled"] = serde_json::Value::Bool(enabled);
                match self.write_config_json(&config_path, &config) {
                    Ok(()) => {
                        let action = if enabled { "enabled" } else { "disabled" };
                        let _ = writeln!(self.writer, "Heartbeat {}", action);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_interval(&mut self, args_str: &str) {
        let val_str = args_str.trim();
        let seconds: u32 = match val_str.parse() {
            Ok(0) => {
                let _ = writeln!(self.writer, "Error: interval must be at least 1 second");
                return;
            }
            Ok(n) => n,
            Err(_) => {
                let _ = writeln!(self.writer, "Error: invalid interval '{}'", val_str);
                return;
            }
        };

        let config_path = self.session.base_dir.join("config.json");
        match self.read_config_json(&config_path) {
            Ok(mut config) => {
                if config.get("heartbeat").is_none() {
                    config["heartbeat"] = serde_json::json!({});
                }
                config["heartbeat"]["interval"] =
                    serde_json::Value::Number(serde_json::Number::from(seconds));
                match self.write_config_json(&config_path, &config) {
                    Ok(()) => {
                        let _ = writeln!(self.writer, "Heartbeat interval set to {}s", seconds);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn heartbeat_status(&mut self) {
        let config = self.load_config();
        let (hb_enabled, hb_interval) = config
            .as_ref()
            .map(|c| (c.heartbeat.enabled, c.heartbeat.interval))
            .unwrap_or((true, 30));

        let status = if hb_enabled { "enabled" } else { "disabled" };
        let _ = writeln!(self.writer, "Heartbeat: {} ({}s)", status, hb_interval);

        let path = self.heartbeat_md_path();
        let task_count = std::fs::read_to_string(&path)
            .map(|c| crate::application::heartbeat::parse_heartbeat(&c).len())
            .unwrap_or(0);
        let _ = writeln!(self.writer, "{} task(s) configured", task_count);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::domain::error::DomainError;
    use crate::domain::message::LlmResponse;
    use crate::domain::provider::{ChatRequest, LlmProvider};
    use crate::infrastructure::persistence::session_store::FileSessionStore;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    use crate::interface::repl::{CMD_HEARTBEAT, ReplLoop, ReplSession};

    // -- helpers --

    #[derive(Debug)]
    struct StubProvider;

    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn chat(
            &self,
            _request: ChatRequest<'_>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>,
        > {
            Box::pin(async {
                Ok(LlmResponse {
                    content: Some("stub".to_string()),
                    tool_calls: vec![],
                    usage: None,
                })
            })
        }
    }

    fn make_repl(base_dir: &std::path::Path) -> ReplLoop<Cursor<Vec<u8>>, Vec<u8>> {
        let provider: Arc<dyn LlmProvider> = Arc::new(StubProvider);
        let registry = ToolRegistryImpl::new();
        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider,
            tool_registry: Box::new(registry),
            model: "test".to_string(),
            max_tokens: 1024,
            temperature: 0.0,
            spill_store: None,
            session_key: String::new(),
            context_collapse_after_turns: u32::MAX,
            max_context_tokens: 190_000,
        });
        let session_store = FileSessionStore::new(base_dir);
        let session = ReplSession {
            agent,
            messages: Vec::new(),
            session_store,
            session_key: "test:hb".to_string(),
            ephemeral: true,
            system_prompt: None,
            base_dir: base_dir.to_path_buf(),
        };
        ReplLoop::new(Cursor::new(Vec::new()), Vec::new(), false, session)
    }

    fn output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
        String::from_utf8(repl.writer.clone()).unwrap()
    }

    /// Write a config.json that sets workspace to `<base_dir>/workspace`.
    fn write_config(base_dir: &std::path::Path) {
        let workspace = base_dir.join("workspace");
        let config = serde_json::json!({
            "agents": {
                "defaults": {
                    "workspace": workspace.to_string_lossy()
                }
            }
        });
        std::fs::write(
            base_dir.join("config.json"),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
    }

    // -- tests --

    #[test]
    fn test_heartbeat_usage() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat(CMD_HEARTBEAT);
        let out = output(&repl);
        assert!(out.contains("Usage"), "expected usage text, got: {out}");
        assert!(out.contains("show"));
        assert!(out.contains("add"));
        assert!(out.contains("remove"));
    }

    #[test]
    fn test_heartbeat_show_no_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat show");
        let out = output(&repl);
        assert!(
            out.contains("No heartbeat tasks"),
            "expected no-tasks message, got: {out}"
        );
    }

    #[test]
    fn test_heartbeat_show_with_tasks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("HEARTBEAT.md"),
            "- Check disk space\n- Run backups\n",
        )
        .unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat show");
        let out = output(&repl);
        assert!(out.contains("Check disk space"), "got: {out}");
        assert!(out.contains("Run backups"), "got: {out}");
        assert!(out.contains("2 tasks"), "got: {out}");
    }

    #[test]
    fn test_heartbeat_add_regular() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat add Check logs daily");
        let out = output(&repl);
        assert!(out.contains("Task added: Check logs daily"), "got: {out}");

        let content = std::fs::read_to_string(workspace.join("HEARTBEAT.md")).unwrap();
        assert!(content.contains("- Check logs daily"));
    }

    #[test]
    fn test_heartbeat_add_spawn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat add --spawn Run heavy analysis");
        let out = output(&repl);
        assert!(out.contains("Task added: Run heavy analysis"), "got: {out}");

        let content = std::fs::read_to_string(workspace.join("HEARTBEAT.md")).unwrap();
        assert!(
            content.contains("spawn"),
            "expected spawn section header, got: {content}"
        );
        assert!(content.contains("- Run heavy analysis"));
    }

    #[test]
    fn test_heartbeat_add_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat add");
        let out = output(&repl);
        assert!(
            out.contains("Error: missing task description"),
            "got: {out}"
        );
    }

    #[test]
    fn test_heartbeat_remove() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("HEARTBEAT.md"),
            "- Check disk space\n- Run backups\n",
        )
        .unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat remove Check disk space");
        let out = output(&repl);
        assert!(out.contains("Task removed: Check disk space"), "got: {out}");

        let content = std::fs::read_to_string(workspace.join("HEARTBEAT.md")).unwrap();
        assert!(!content.contains("Check disk space"));
        assert!(content.contains("Run backups"));
    }

    #[test]
    fn test_heartbeat_remove_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("HEARTBEAT.md"), "- Existing task\n").unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat remove Nonexistent task");
        let out = output(&repl);
        assert!(
            out.contains("Error: task 'Nonexistent task' not found"),
            "got: {out}"
        );
    }

    #[test]
    fn test_heartbeat_remove_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat remove");
        let out = output(&repl);
        assert!(
            out.contains("Error: missing task description"),
            "got: {out}"
        );
    }

    #[test]
    fn test_heartbeat_enable() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat enable");
        let out = output(&repl);
        assert!(out.contains("Heartbeat enabled"), "got: {out}");

        // Verify config was updated
        let content = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["heartbeat"]["enabled"], true);
    }

    #[test]
    fn test_heartbeat_disable() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat disable");
        let out = output(&repl);
        assert!(out.contains("Heartbeat disabled"), "got: {out}");

        let content = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["heartbeat"]["enabled"], false);
    }

    #[test]
    fn test_heartbeat_interval() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat interval 120");
        let out = output(&repl);
        assert!(out.contains("Heartbeat interval set to 120s"), "got: {out}");

        let content = std::fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["heartbeat"]["interval"], 120);
    }

    #[test]
    fn test_heartbeat_interval_invalid() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat interval abc");
        let out = output(&repl);
        assert!(out.contains("Error: invalid interval 'abc'"), "got: {out}");
    }

    #[test]
    fn test_heartbeat_interval_zero() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat interval 0");
        let out = output(&repl);
        assert!(
            out.contains("Error: interval must be at least 1 second"),
            "got: {out}"
        );
    }

    #[test]
    fn test_heartbeat_status() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(tmp.path());
        // Create workspace with a task
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("HEARTBEAT.md"), "- Monitor CPU\n").unwrap();

        let mut repl = make_repl(tmp.path());
        repl.handle_heartbeat("/heartbeat status");
        let out = output(&repl);
        assert!(out.contains("enabled"), "got: {out}");
        assert!(out.contains("1 task(s) configured"), "got: {out}");
    }
}
