// /cron command handler for the REPL.

use std::io::{BufRead, Write};

use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::infrastructure::persistence::cron_store::FileCronStore;

use super::parsers::parse_cron_add_args;
use super::{CMD_CRON, ReplLoop};

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    pub(super) fn handle_cron(&mut self, input: &str) {
        let rest = input.strip_prefix(CMD_CRON).unwrap_or("").trim();
        let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
        let subcmd = parts.first().copied().unwrap_or("");
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        match subcmd {
            "list" => self.cron_list(),
            "add" => self.cron_add(args_str),
            "remove" => self.cron_remove(args_str),
            "enable" => self.cron_enable(args_str),
            "disable" => self.cron_disable(args_str),
            _ => self.cron_usage(),
        }
    }

    fn cron_store(&self) -> FileCronStore {
        FileCronStore::new(&self.session.base_dir)
    }

    fn cron_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /cron <subcommand>");
        let _ = writeln!(self.writer, "  add      Add a new cron job");
        let _ = writeln!(self.writer, "  list     List all cron jobs");
        let _ = writeln!(self.writer, "  remove   Remove a cron job");
        let _ = writeln!(self.writer, "  enable   Enable a cron job");
        let _ = writeln!(self.writer, "  disable  Disable a cron job");
    }

    fn cron_list(&mut self) {
        let store = self.cron_store();
        match store.list() {
            Ok(jobs) if jobs.is_empty() => {
                let _ = writeln!(self.writer, "No scheduled jobs");
            }
            Ok(jobs) => {
                let _ = writeln!(self.writer, "Scheduled jobs:");
                for job in &jobs {
                    let schedule_str = match &job.schedule {
                        CronSchedule::Interval { seconds } => format!("every {}s", seconds),
                        CronSchedule::Cron { expression } => format!("cron: {}", expression),
                    };
                    let status = if job.enabled { "enabled" } else { "disabled" };
                    let _ = writeln!(
                        self.writer,
                        "  {} — {} [{}]",
                        job.name, schedule_str, status
                    );
                }
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn cron_add(&mut self, args_str: &str) {
        match parse_cron_add_args(args_str) {
            Ok(parsed) => {
                let store = self.cron_store();
                let job = CronJob {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: parsed.name.clone(),
                    message: parsed.message,
                    schedule: parsed.schedule,
                    enabled: true,
                    deliver_to: parsed.deliver_to,
                    last_error: None,
                    last_run_at: 0,
                    run_once: false,
                };
                // Atomic check-and-insert to avoid TOCTOU race.
                match store.add_if_absent(job) {
                    Ok(true) => {
                        let _ = writeln!(self.writer, "Job '{}' created", parsed.name);
                    }
                    Ok(false) => {
                        let _ =
                            writeln!(self.writer, "Error: job '{}' already exists", parsed.name);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
            }
        }
    }

    fn cron_remove(&mut self, args_str: &str) {
        let name = args_str.trim();
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing job name");
            return;
        }
        let store = self.cron_store();
        match store.find_by_name(name) {
            Ok(Some(job)) => match store.remove(&job.id) {
                Ok(()) => {
                    let _ = writeln!(self.writer, "Job '{}' removed", name);
                }
                Err(e) => {
                    let _ = writeln!(self.writer, "Error: {}", e);
                }
            },
            Ok(None) => {
                let _ = writeln!(self.writer, "Error: job '{}' not found", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }

    fn cron_enable(&mut self, args_str: &str) {
        self.cron_set_enabled(args_str.trim(), true);
    }

    fn cron_disable(&mut self, args_str: &str) {
        self.cron_set_enabled(args_str.trim(), false);
    }

    fn cron_set_enabled(&mut self, name: &str, enabled: bool) {
        if name.is_empty() {
            let _ = writeln!(self.writer, "Error: missing job name");
            return;
        }
        let store = self.cron_store();
        match store.find_by_name(name) {
            Ok(Some(job)) => {
                let action = if enabled { "enabled" } else { "disabled" };
                match store.set_enabled(&job.id, enabled) {
                    Ok(()) => {
                        let _ = writeln!(self.writer, "Job '{}' {}", name, action);
                    }
                    Err(e) => {
                        let _ = writeln!(self.writer, "Error: {}", e);
                    }
                }
            }
            Ok(None) => {
                let _ = writeln!(self.writer, "Error: job '{}' not found", name);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::domain::error::DomainError;
    use crate::domain::message::LlmResponse;
    use crate::domain::provider::{ChatRequest, LlmProvider};
    use crate::infrastructure::persistence::session_store::FileSessionStore;
    use crate::infrastructure::security::sandbox::Sandbox;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;
    use std::io::Cursor;
    use std::sync::Arc;

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

    fn make_test_repl(base_dir: &std::path::Path) -> ReplLoop<Cursor<Vec<u8>>, Vec<u8>> {
        let sandbox = Sandbox::new(None, false);
        let workspace = base_dir.join("workspace");
        let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider: Arc::new(StubProvider),
            tool_registry: Box::new(registry),
            model: "test-model".to_string(),
            max_tokens: 100,
            temperature: 0.0,
            spill_store: None,
            session_key: String::new(),
            context_collapse_after_turns: 3,
            max_context_tokens: 100_000,
        });

        let session = super::super::ReplSession {
            agent,
            messages: Vec::new(),
            session_store: FileSessionStore::new(base_dir),
            session_key: "test:key".to_string(),
            ephemeral: false,
            system_prompt: None,
            base_dir: base_dir.to_path_buf(),
        };

        ReplLoop::new(Cursor::new(Vec::new()), Vec::new(), false, session)
    }

    fn get_output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
        String::from_utf8(repl.writer.clone()).unwrap()
    }

    #[test]
    fn test_cron_usage() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron");
        let out = get_output(&repl);
        assert!(out.contains("Usage"), "expected usage text, got: {out}");
    }

    #[test]
    fn test_cron_list_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron list");
        let out = get_output(&repl);
        assert!(
            out.contains("No scheduled"),
            "expected empty list message, got: {out}"
        );
    }

    #[test]
    fn test_cron_add_and_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());

        repl.handle_cron("/cron add myjob --interval 60 --message Check status");
        let out = get_output(&repl);
        assert!(
            out.contains("Job 'myjob' created"),
            "expected created message, got: {out}"
        );

        // Clear writer and list
        repl.writer.clear();
        repl.handle_cron("/cron list");
        let out = get_output(&repl);
        assert!(out.contains("myjob"), "expected job in listing, got: {out}");
        assert!(
            out.contains("every 60s"),
            "expected schedule in listing, got: {out}"
        );
        assert!(
            out.contains("enabled"),
            "expected enabled status, got: {out}"
        );
    }

    #[test]
    fn test_cron_add_missing_args() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron add");
        let out = get_output(&repl);
        assert!(
            out.contains("Error"),
            "expected error for missing args, got: {out}"
        );
    }

    #[test]
    fn test_cron_remove() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());

        repl.handle_cron("/cron add deljob --interval 120 --message Test removal");
        repl.writer.clear();

        repl.handle_cron("/cron remove deljob");
        let out = get_output(&repl);
        assert!(
            out.contains("removed"),
            "expected removed message, got: {out}"
        );

        // Verify it's gone
        repl.writer.clear();
        repl.handle_cron("/cron list");
        let out = get_output(&repl);
        assert!(
            out.contains("No scheduled"),
            "expected empty after removal, got: {out}"
        );
    }

    #[test]
    fn test_cron_remove_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron remove ghost");
        let out = get_output(&repl);
        assert!(
            out.contains("not found"),
            "expected not found error, got: {out}"
        );
    }

    #[test]
    fn test_cron_remove_missing_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron remove");
        let out = get_output(&repl);
        assert!(
            out.contains("missing job name"),
            "expected missing name error, got: {out}"
        );
    }

    #[test]
    fn test_cron_enable_disable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());

        repl.handle_cron("/cron add togglejob --interval 300 --message Toggle test");
        repl.writer.clear();

        // Disable
        repl.handle_cron("/cron disable togglejob");
        let out = get_output(&repl);
        assert!(
            out.contains("disabled"),
            "expected disabled message, got: {out}"
        );

        // Verify disabled in listing
        repl.writer.clear();
        repl.handle_cron("/cron list");
        let out = get_output(&repl);
        assert!(
            out.contains("disabled"),
            "expected disabled status in listing, got: {out}"
        );

        // Enable
        repl.writer.clear();
        repl.handle_cron("/cron enable togglejob");
        let out = get_output(&repl);
        assert!(
            out.contains("enabled"),
            "expected enabled message, got: {out}"
        );

        // Verify enabled in listing
        repl.writer.clear();
        repl.handle_cron("/cron list");
        let out = get_output(&repl);
        assert!(
            out.contains("enabled"),
            "expected enabled status in listing, got: {out}"
        );
    }

    #[test]
    fn test_cron_enable_missing_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron enable");
        let out = get_output(&repl);
        assert!(
            out.contains("missing job name"),
            "expected missing name error, got: {out}"
        );
    }

    #[test]
    fn test_cron_enable_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());
        repl.handle_cron("/cron enable nonexistent");
        let out = get_output(&repl);
        assert!(
            out.contains("not found"),
            "expected not found error, got: {out}"
        );
    }

    #[test]
    fn test_cron_add_duplicate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut repl = make_test_repl(tmp.path());

        repl.handle_cron("/cron add dupjob --interval 60 --message First");
        repl.writer.clear();

        repl.handle_cron("/cron add dupjob --interval 120 --message Second");
        let out = get_output(&repl);
        assert!(
            out.contains("already exists"),
            "expected duplicate error, got: {out}"
        );
    }
}
