use super::*;

impl App {
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => self.handle_agent_start(),
            Event::Token { token } => self.chat.append_token(&token),
            Event::TurnStart => {}
            Event::TurnEnd { message, .. } => self.handle_turn_end(message),
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => self.handle_tool_start(tool_call_id, tool_name, args),
            Event::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => self.handle_tool_end(tool_call_id, tool_name, result, is_error),
            Event::AgentEnd { .. } if self.agent_state.end() => self.handle_agent_end(),
            Event::AgentEnd { .. } => {}
            Event::Response {
                command,
                success,
                data,
                error,
                ..
            } => self.handle_response(command, success, data, error),
            Event::SubagentStateChanged { subagents } => self.update_subagent_bar(subagents),
            Event::WorkflowState {
                agent_id,
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            } => self.handle_workflow_state(WorkflowStateEvent {
                agent_id,
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            }),
            _ => {}
        }
    }

    fn handle_agent_start(&mut self) {
        self.agent_state.start();
        self.footer.set_streaming(true);
        self.spinner = Some(Spinner::new("Working... (Esc to interrupt)"));
    }

    fn handle_agent_end(&mut self) {
        self.footer.set_streaming(false);
        self.spinner = None;
        self.chat.finalize_assistant();
    }

    fn handle_turn_end(&mut self, message: serde_json::Value) {
        self.chat.finalize_assistant();
        let Some(usage) = message.get("usage") else {
            return;
        };
        let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = usage.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        if let Some(window) = message
            .get("maxContextTokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
        {
            self.footer.update_context_usage(input, window);
            self.context_stats_requested = true;
        } else if total > 0 && !self.context_stats_requested {
            self.context_stats_requested = true;
            self.send_session_stats();
        }
    }

    fn handle_tool_start(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) {
        let args_str = if args.is_object() || args.is_array() {
            serde_json::to_string(&args).unwrap_or_default()
        } else {
            args.to_string()
        };
        self.update_tool_spinner(&tool_name, &args, &args_str);
        // Mark the awaited sub-agent so its row shows a per-row "awaiting"
        // indicator instead of the shared spinner line.
        if tool_name == "agent_cmd" && args.get("command").and_then(|v| v.as_str()) == Some("await")
        {
            self.awaited_agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            self.rebuild_subagent_bar();
        }
        let is_spawn = tool_name == "spawn";
        if !suppress_tool_box(&tool_name, &args) {
            self.chat.start_tool(tool_call_id, tool_name, args_str);
        }
        if is_spawn {
            self.track_starting_subagent(&args);
        }
    }

    fn update_tool_spinner(&mut self, tool_name: &str, args: &serde_json::Value, args_str: &str) {
        let Some(spinner) = &mut self.spinner else {
            return;
        };
        let msg = match tool_name {
            "spawn" => format!("Spawning {}...", sanitized_arg(args, "agent_id", "agent")),
            // For `await`, keep a generic, stable message — the awaited agent is
            // marked on its own sub-agent row, so the shared line stays put.
            "agent_cmd" if args.get("command").and_then(|v| v.as_str()) == Some("await") => {
                "Working... (Esc to interrupt)".to_string()
            }
            "agent_cmd" => format!(
                "{} → {}...",
                sanitized_arg(args, "command", "?"),
                sanitized_arg(args, "agent_id", "?")
            ),
            _ => format!("{} {}...", tool_name, truncate_args(args_str)),
        };
        spinner.set_message(&msg);
    }

    fn track_starting_subagent(&mut self, args: &serde_json::Value) {
        let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) else {
            return;
        };
        let sanitized: String = agent_id.chars().filter(|c| !c.is_control()).collect();
        self.subagent_local.insert(
            sanitized.clone(),
            TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
                agent_id: sanitized,
                status: "starting".to_string(),
                last_tool: None,
                last_error: None,
                pid: 0,
                parent_id: None,
                workflow: None,
            }),
        );
        self.rebuild_subagent_bar();
    }

    fn handle_tool_end(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        result: serde_json::Value,
        is_error: bool,
    ) {
        let result_text = crate::infrastructure::client::extract_result_text(&result);
        self.chat
            .complete_tool(&tool_call_id, &result_text, is_error, None);
        if tool_name == "spawn" && !is_error {
            self.mark_spawned_subagent_running(&result_text);
        }
        if is_subagent_tool(&tool_name) {
            self.send_command(Command::GetSubagents { id: None });
        }
        if self.awaited_agent_id.is_some() {
            self.awaited_agent_id = None;
            self.rebuild_subagent_bar();
        }
        if let Some(spinner) = &mut self.spinner {
            spinner.set_message("Working... (Esc to interrupt)");
        }
    }

    fn mark_spawned_subagent_running(&mut self, result_text: &str) {
        let Some(start) = result_text.find('\'') else {
            return;
        };
        let Some(end) = result_text[start + 1..].find('\'') else {
            return;
        };
        let agent_id = &result_text[start + 1..start + 1 + end];
        let sanitized: String = agent_id.chars().filter(|c| !c.is_control()).collect();
        if let Some(entry) = self.subagent_local.get_mut(&sanitized) {
            entry.info.status = "running".to_string();
        }
        self.rebuild_subagent_bar();
    }

    fn handle_response(
        &mut self,
        command: String,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        match command.as_str() {
            "get_state" if success => self.handle_get_state(data),
            "set_model" if success => self.notify("Model switched", NotifyLevel::Success),
            "set_model" => self.notify_response_error("Model switch failed", error),
            "set_workflow_automation" if success => self.handle_workflow_automation(data),
            "set_workflow_automation" => {
                self.notify_response_error("Workflow automation update failed", error)
            }
            "get_session_stats" if success => {
                if let Some(data) = data {
                    self.show_session_stats(&data);
                }
            }
            "list_sessions" if success => {
                if let Some(data) = data {
                    self.open_resume_selector(&data);
                }
            }
            "list_sessions" => self.notify_response_error("Could not list sessions", error),
            "resume_session" if success => self.handle_resume_success(data),
            "resume_session" => self.notify_response_error("Resume failed", error),
            "get_messages" if success => {
                if let Some(data) = data {
                    self.replace_chat_with_messages(&data);
                }
            }
            "clear_history" if success => {}
            "get_subagents" if success => self.handle_get_subagents(data),
            "agent_error" => self.handle_agent_error(error),
            _ => {}
        }
    }

    fn handle_get_state(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
            let sanitized: String = model.chars().filter(|c| !c.is_control()).collect();
            self.footer.set_model(&sanitized);
            self.current_model = Some(sanitized);
        }
        if let Some(max_ctx) = data.get("maxContextTokens").and_then(|v| v.as_u64()) {
            self.footer.set_context_window(max_ctx as usize);
            self.context_stats_requested = true;
        }
        if let Some(wf) = data.get("workflow") {
            self.workflow_bar = workflow_bar::parse_workflow_event(wf);
            self.sync_workflow_automation(wf);
        }
    }

    fn sync_workflow_automation(&mut self, data: &serde_json::Value) {
        let automation = data.get("automation").unwrap_or(data);
        if let Some(value) = automation.get("autoContinue").and_then(|v| v.as_bool()) {
            self.workflow_auto_continue = value;
        }
        if let Some(value) = automation.get("completionNudge").and_then(|v| v.as_bool()) {
            self.workflow_completion_nudge = value;
        }
    }

    fn handle_workflow_automation(&mut self, data: Option<serde_json::Value>) {
        if let Some(data) = data {
            self.sync_workflow_automation(&data);
        }
        let auto = if self.workflow_auto_continue {
            "ON"
        } else {
            "OFF"
        };
        let nudge = if self.workflow_completion_nudge {
            "ON"
        } else {
            "OFF"
        };
        self.notify(
            &format!("Workflow automation: auto-continue {auto}, completion nudge {nudge}"),
            NotifyLevel::Info,
        );
    }

    fn handle_resume_success(&mut self, data: Option<serde_json::Value>) {
        let session = data
            .as_ref()
            .and_then(|d| d.get("session").and_then(|v| v.as_str()))
            .unwrap_or("session");
        self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
        self.send_command(Command::GetMessages {
            id: Some("resume-messages".into()),
        });
        self.send_session_stats();
    }

    fn handle_get_subagents(&mut self, data: Option<serde_json::Value>) {
        let Some(data) = data else { return };
        let Some(arr) = data.get("subagents") else {
            return;
        };
        if let Ok(infos) = serde_json::from_value::<
            Vec<crate::infrastructure::client::SubagentInfoEvent>,
        >(arr.clone())
        {
            self.update_subagent_bar(infos);
        }
    }

    fn handle_agent_error(&mut self, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.chat.add_entry(ChatEntry::Status {
            text: format!("Error: {}", msg),
        });
        self.agent_state.reset();
        self.footer.set_streaming(false);
        self.spinner = None;
    }

    fn notify_response_error(&mut self, prefix: &str, error: Option<String>) {
        let msg = error.unwrap_or_else(|| "unknown error".into());
        self.notify(&format!("{prefix}: {msg}"), NotifyLevel::Error);
    }

    fn handle_workflow_state(&mut self, workflow: WorkflowStateEvent) {
        let WorkflowStateEvent {
            agent_id,
            steps,
            progress,
            active_issue,
            mode,
            active_template,
            available_templates,
        } = workflow;
        // Ignore workflow_state events forwarded up from a sub-agent (PRD Stage
        // B): they belong to a child, not the connected agent, and must not
        // overwrite the parent's own workflow bar — doing so makes the bar
        // flicker between children and judders the layout while awaiting several
        // workflow-running children. The child's progress is shown on its own
        // row via get_subagents instead.
        if let Some(id) = agent_id.as_deref() {
            if self.subagent_local.contains_key(id) {
                return;
            }
        }
        let mut event = serde_json::json!({
            "steps": steps,
            "progress": progress,
        });
        if let Some(issue) = active_issue {
            event["activeIssue"] = issue;
        }
        if let Some(m) = mode {
            event["mode"] = serde_json::json!(m);
        }
        if let Some(tpl) = active_template {
            event["activeTemplate"] = tpl;
        }
        if let Some(templates) = available_templates {
            event["availableTemplates"] = serde_json::json!(templates);
        }
        self.workflow_bar = workflow_bar::parse_workflow_event(&event);
    }
}

struct WorkflowStateEvent {
    agent_id: Option<String>,
    steps: Vec<serde_json::Value>,
    progress: serde_json::Value,
    active_issue: Option<serde_json::Value>,
    mode: Option<String>,
    active_template: Option<serde_json::Value>,
    available_templates: Option<Vec<serde_json::Value>>,
}

fn sanitized_arg(args: &serde_json::Value, key: &str, fallback: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(fallback)
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::terminal::Terminal;
    use tokio::io::AsyncReadExt;

    async fn test_app() -> App {
        let dir = std::env::temp_dir().join(format!(
            "quecto-tui-app-events-test-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket_path = dir.join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
        });
        let client = Client::connect(&socket_path).await.unwrap();
        App::new(Terminal::new(), client)
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[tokio::test]
    async fn handles_agent_lifecycle_and_token_events() {
        let mut app = test_app().await;
        app.handle_event(Event::AgentStart);
        assert!(app.agent_state.is_running());
        assert!(app.spinner.is_some());
        app.handle_event(Event::Token {
            token: "hello".into(),
        });
        app.handle_event(Event::TurnStart);
        app.handle_event(Event::AgentEnd { messages: vec![] });
        assert!(!app.agent_state.is_running());
        assert!(app.spinner.is_none());
    }

    #[tokio::test]
    async fn handles_turn_end_usage_with_context_window_and_stats_fallback() {
        let mut app = test_app().await;
        app.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "usage": {"input": 10, "output": 5, "total": 15},
                "maxContextTokens": 100
            }),
            tool_results: vec![],
        });
        assert!(app.context_stats_requested);

        let mut app = test_app().await;
        app.handle_event(Event::TurnEnd {
            message: serde_json::json!({"usage": {"total": 1}}),
            tool_results: vec![],
        });
        assert!(app.context_stats_requested);
    }

    #[tokio::test]
    async fn handles_tool_start_and_end_for_spawn_and_regular_tools() {
        let mut app = test_app().await;
        app.spinner = Some(Spinner::new("Working"));
        app.handle_event(Event::ToolExecutionStart {
            tool_call_id: "spawn-1".into(),
            tool_name: "spawn".into(),
            args: serde_json::json!({"agent_id":"worker-1"}),
        });
        assert!(app.subagent_local.contains_key("worker-1"));

        app.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "spawn-1".into(),
            tool_name: "spawn".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"Subagent 'worker-1' is running"}]}),
            is_error: false,
        });
        assert_eq!(app.subagent_local["worker-1"].info.status, "running");

        app.handle_event(Event::ToolExecutionStart {
            tool_call_id: "read-1".into(),
            tool_name: "read".into(),
            args: serde_json::json!({"path":"file.txt"}),
        });
        app.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "read-1".into(),
            tool_name: "read".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"contents"}]}),
            is_error: false,
        });
    }

    #[tokio::test]
    async fn handles_agent_cmd_spinner_and_subagent_refresh() {
        let mut app = test_app().await;
        app.spinner = Some(Spinner::new("Working"));
        app.handle_event(Event::ToolExecutionStart {
            tool_call_id: "cmd-1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"agent_id":"worker-1", "command":"await"}),
        });
        app.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "cmd-1".into(),
            tool_name: "agent_cmd".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"done"}]}),
            is_error: false,
        });
    }

    #[tokio::test]
    async fn handles_response_variants() {
        let mut app = test_app().await;
        app.handle_event(Event::Response {
            id: None,
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model":"test-model",
                "maxContextTokens": 123,
                "workflow": {"steps": [], "progress": {"done": 0, "total": 0}}
            })),
            error: None,
        });
        assert_eq!(app.current_model.as_deref(), Some("test-model"));

        for command in ["set_model", "list_sessions", "resume_session"] {
            app.handle_event(Event::Response {
                id: None,
                command: command.into(),
                success: false,
                data: None,
                error: Some("nope".into()),
            });
        }

        app.handle_event(Event::Response {
            id: None,
            command: "get_session_stats".into(),
            success: true,
            data: Some(serde_json::json!({"messageCount": 2, "totalTokens": 10})),
            error: None,
        });
        app.handle_event(Event::Response {
            id: None,
            command: "list_sessions".into(),
            success: true,
            data: Some(serde_json::json!({"sessions": []})),
            error: None,
        });
        app.handle_event(Event::Response {
            id: None,
            command: "resume_session".into(),
            success: true,
            data: Some(serde_json::json!({"session":"cli:test"})),
            error: None,
        });
        app.handle_event(Event::Response {
            id: None,
            command: "get_messages".into(),
            success: true,
            data: Some(serde_json::json!({"messages": [{"role":"user", "content":"hi"}]})),
            error: None,
        });
    }

    #[tokio::test]
    async fn forwarded_child_workflow_state_does_not_clobber_parent_bar() {
        let mut app = test_app().await;
        // Register a child subagent.
        app.handle_event(Event::SubagentStateChanged {
            subagents: vec![crate::infrastructure::client::SubagentInfoEvent {
                agent_id: "child".into(),
                status: "running".into(),
                last_tool: None,
                last_error: None,
                pid: 0,
                parent_id: None,
                workflow: None,
            }],
        });
        // A workflow_state forwarded up from the child (agent_id = "child", a
        // known subagent) must NOT touch the parent's own workflow bar.
        app.handle_event(Event::WorkflowState {
            agent_id: Some("child".into()),
            steps: vec![],
            progress: serde_json::json!({"done": 3, "total": 5}),
            active_issue: Some(serde_json::json!({"number": 7, "title": "child"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        });
        assert!(
            app.workflow_bar.issue_number.is_none(),
            "a forwarded child event must not set the parent's workflow bar"
        );

        // The connected agent's own event (no agent_id) does update the bar.
        app.handle_event(Event::WorkflowState {
            agent_id: None,
            steps: vec![],
            progress: serde_json::json!({"done": 1, "total": 2}),
            active_issue: Some(serde_json::json!({"number": 9, "title": "parent"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: None,
        });
        assert_eq!(app.workflow_bar.issue_number, Some(9));
    }

    #[tokio::test]
    async fn handles_subagent_workflow_and_error_events() {
        let mut app = test_app().await;
        let info = crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "a1".into(),
            status: "running".into(),
            last_tool: Some("read".into()),
            last_error: None,
            pid: 42,
            parent_id: None,
            workflow: None,
        };
        app.handle_event(Event::SubagentStateChanged {
            subagents: vec![info.clone()],
        });
        app.handle_event(Event::Response {
            id: None,
            command: "get_subagents".into(),
            success: true,
            data: Some(serde_json::json!({"subagents": [{
                "agentId": "a1",
                "status": "running",
                "lastTool": "read",
                "lastError": null,
                "pid": 42
            }]})),
            error: None,
        });
        app.handle_event(Event::WorkflowState {
            agent_id: None,
            steps: vec![],
            progress: serde_json::json!({"done": 0, "total": 0}),
            active_issue: Some(serde_json::json!({"number": 1, "title": "Issue"})),
            mode: Some("active".into()),
            active_template: None,
            available_templates: Some(vec![]),
        });
        app.handle_event(Event::Response {
            id: None,
            command: "agent_error".into(),
            success: false,
            data: None,
            error: Some("boom".into()),
        });
        assert!(!app.agent_state.is_running());
    }

    #[test]
    fn sanitized_arg_strips_control_chars_and_uses_fallback() {
        let args = serde_json::json!({"agent_id":"a\u{0007}b"});
        assert_eq!(sanitized_arg(&args, "agent_id", "x"), "ab");
        assert_eq!(sanitized_arg(&args, "missing", "x"), "x");
    }
}
