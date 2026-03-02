// /spawn command handler for the REPL.

use std::io::{BufRead, Write};

use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;

use super::parsers::parse_spawn_args;
use super::{CMD_SPAWN, ReplLoop};

impl<R: BufRead, W: Write> ReplLoop<R, W> {
    pub(super) fn handle_spawn(&mut self, input: &str, rt: &tokio::runtime::Runtime) {
        let rest = input.strip_prefix(CMD_SPAWN).unwrap_or("").trim();

        if rest.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        let parsed = match parse_spawn_args(rest) {
            Ok(p) => p,
            Err(msg) => {
                let _ = writeln!(self.writer, "Error: {}", msg);
                return;
            }
        };

        if parsed.help {
            self.spawn_usage();
            return;
        }

        if parsed.task.is_empty() {
            let _ = writeln!(self.writer, "Error: missing task description");
            return;
        }

        if parsed.model.is_some() {
            let _ = writeln!(
                self.writer,
                "Error: --model is not supported in REPL mode (agent uses the model from startup)"
            );
            return;
        }

        // Resolve system prompt: from --agent profile or --system flag
        let system = if let Some(ref agent_name) = parsed.agent {
            let Some(path) = self.validated_agent_path(agent_name) else {
                return;
            };
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(profile) => profile["system"].as_str().map(String::from),
                    Err(_) => {
                        let _ =
                            writeln!(self.writer, "Error: invalid profile for '{}'", agent_name);
                        return;
                    }
                },
                Err(_) => {
                    let _ = writeln!(self.writer, "Error: agent '{}' not found", agent_name);
                    return;
                }
            }
        } else {
            parsed.system.clone()
        };

        // Build an ephemeral message list for the spawn (session isolation).
        let mut spawn_messages = Vec::new();

        if let Some(ref prompt) = system {
            spawn_messages.push(Message::system(prompt.clone()));
        }

        spawn_messages.push(Message::user(parsed.task.clone()));

        // Run with optional timeout
        let result = if let Some(max_secs) = parsed.max_time {
            let timeout = std::time::Duration::from_secs(max_secs);
            rt.block_on(async {
                match tokio::time::timeout(timeout, self.session.agent.process(&mut spawn_messages))
                    .await
                {
                    Ok(r) => r,
                    Err(_) => Err(crate::domain::error::DomainError::Other(
                        "spawn timed out".to_string(),
                    )),
                }
            })
        } else {
            rt.block_on(self.session.agent.process(&mut spawn_messages))
        };

        match result {
            Ok(r) => {
                self.session
                    .messages
                    .push(Message::assistant(r.response.clone(), vec![]));
                let _ = writeln!(self.writer, "{}", r.response);
            }
            Err(e) => {
                let _ = writeln!(self.writer, "Error: {e}");
            }
        }
    }

    pub(super) fn spawn_usage(&mut self) {
        let _ = writeln!(self.writer, "Usage: /spawn [flags] <task>");
        let _ = writeln!(
            self.writer,
            "  --agent <name>       Use a named agent profile"
        );
        let _ = writeln!(self.writer, "  --system <prompt>    Set a system prompt");
        let _ = writeln!(
            self.writer,
            "  --model <model>      Not supported in REPL mode"
        );
        let _ = writeln!(
            self.writer,
            "  --max-time <secs>    Set a timeout in seconds"
        );
        let _ = writeln!(self.writer, "  --help               Show this help");
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::Cursor;
    use std::pin::Pin;
    use std::sync::Arc;

    use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use crate::domain::error::DomainError;
    use crate::domain::message::LlmResponse;
    use crate::domain::provider::{ChatRequest, LlmProvider};
    use crate::infrastructure::persistence::session_store::FileSessionStore;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    use super::super::{ReplLoop, ReplSession};

    #[derive(Debug)]
    struct StubProvider;

    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn chat(
            &self,
            _request: ChatRequest<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
            Box::pin(async {
                Ok(LlmResponse {
                    content: Some("stub response".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })
            })
        }
    }

    /// Build a minimal `ReplLoop` backed by in-memory buffers and a stub provider.
    fn make_repl() -> (ReplLoop<Cursor<Vec<u8>>, Vec<u8>>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let provider: Arc<dyn LlmProvider> = Arc::new(StubProvider);
        let registry = ToolRegistryImpl::new();
        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider,
            tool_registry: Box::new(registry),
            model: "test-model".to_string(),
            max_tokens: 1024,
            temperature: 0.7,
            spill_store: None,
            session_key: String::new(),
            context_collapse_after_turns: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
        });
        let session_store = FileSessionStore::new(tmp.path());
        let session = ReplSession {
            agent,
            messages: Vec::new(),
            session_store,
            session_key: "test:spawn".to_string(),
            ephemeral: true,
            system_prompt: None,
            base_dir: tmp.path().to_path_buf(),
        };
        let reader = Cursor::new(Vec::new());
        let writer = Vec::new();
        let repl = ReplLoop::new(reader, writer, false, session);
        (repl, tmp)
    }

    fn output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
        String::from_utf8(repl.writer.clone()).unwrap()
    }

    #[test]
    fn test_spawn_usage() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn --help", &rt);
        let out = output(&repl);
        assert!(out.contains("Usage"), "expected Usage in output: {out}");
        assert!(out.contains("--agent"));
        assert!(out.contains("--system"));
        assert!(out.contains("--max-time"));
        assert!(out.contains("--help"));
    }

    #[test]
    fn test_spawn_empty_task() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn", &rt);
        let out = output(&repl);
        assert!(
            out.contains("missing task description"),
            "expected 'missing task description' in output: {out}"
        );
    }

    #[test]
    fn test_spawn_model_rejected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn --model gpt-5 some task", &rt);
        let out = output(&repl);
        assert!(
            out.contains("not supported in REPL mode"),
            "expected REPL mode rejection in output: {out}"
        );
    }

    #[test]
    fn test_spawn_simple_task() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn Do something", &rt);
        let out = output(&repl);
        assert!(
            out.contains("stub response"),
            "expected agent response in output: {out}"
        );
        // The result should also be injected into parent session messages.
        assert_eq!(repl.session.messages.len(), 1);
        assert_eq!(repl.session.messages[0].content, "stub response");
    }

    #[test]
    fn test_spawn_with_system() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn --system 'Be concise' summarize this", &rt);
        let out = output(&repl);
        assert!(
            out.contains("stub response"),
            "expected agent response in output: {out}"
        );
    }

    #[test]
    fn test_spawn_with_agent_profile() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, tmp) = make_repl();

        // Create agents directory and a profile file.
        let agents_dir = tmp.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let profile = serde_json::json!({
            "name": "helper",
            "system": "You are a helpful assistant"
        });
        std::fs::write(
            agents_dir.join("helper.json"),
            serde_json::to_string_pretty(&profile).unwrap(),
        )
        .unwrap();

        repl.handle_spawn("/spawn --agent helper do the thing", &rt);
        let out = output(&repl);
        assert!(
            out.contains("stub response"),
            "expected agent response in output: {out}"
        );
        // Result injected into parent session.
        assert_eq!(repl.session.messages.len(), 1);
    }

    #[test]
    fn test_spawn_with_agent_not_found() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn --agent nonexistent do something", &rt);
        let out = output(&repl);
        assert!(
            out.contains("not found"),
            "expected 'not found' in output: {out}"
        );
    }

    #[test]
    fn test_spawn_parse_error() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut repl, _tmp) = make_repl();
        repl.handle_spawn("/spawn --agent", &rt);
        let out = output(&repl);
        assert!(
            out.contains("Error"),
            "expected error message in output: {out}"
        );
        assert!(
            out.contains("--agent requires a value"),
            "expected '--agent requires a value' in output: {out}"
        );
    }
}
