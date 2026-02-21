use super::*;

// ===========================================================================
// REPL Steps — Interactive Conversational Mode
// ===========================================================================

/// Helper: execute the REPL with accumulated input lines and store results.
fn execute_repl(world: &mut QuectoWorld) {
    if world.repl_executed {
        return;
    }
    world.repl_executed = true;

    let input = world.repl_input_lines.join("\n") + "\n";
    let flags = world.repl_flags.clone();
    // Simulate TTY mode so banner/prompt are included in output
    let output = cli::run_repl_with_output(&world.cli_context, &flags, input.as_bytes(), true);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ---------------------------------------------------------------------------
// When steps
// ---------------------------------------------------------------------------

#[when("I start quecto in REPL mode")]
fn when_start_repl(world: &mut QuectoWorld) {
    // Reset REPL state for this scenario
    world.repl_input_lines = Vec::new();
    world.repl_flags = Vec::new();
    world.repl_executed = false;
}

#[when(expr = "I start quecto in REPL mode with flags {string}")]
fn when_start_repl_with_flags(world: &mut QuectoWorld, flags_str: String) {
    world.repl_input_lines = Vec::new();
    world.repl_executed = false;

    // Parse the flags string into individual args
    let mut flags = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = ' ';

    for ch in flags_str.chars() {
        if in_quotes {
            if ch == quote_char {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            in_quotes = true;
            quote_char = ch;
        } else if ch == ' ' {
            if !current.is_empty() {
                flags.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        flags.push(current);
    }

    world.repl_flags = flags;
}

#[when(expr = "I type {string}")]
fn when_type_line(world: &mut QuectoWorld, line: String) {
    let is_exit = line == "/exit" || line == "/quit";
    world.repl_input_lines.push(line);
    if is_exit {
        // /exit is the last input — execute the REPL now
        // so stdout/stderr/exit_code are ready for Then steps.
        execute_repl(world);
    }
}

#[when("I send EOF")]
fn when_send_eof(world: &mut QuectoWorld) {
    // EOF is represented by having no more input lines.
    // The REPL should exit cleanly when read_line returns Ok(0).
    // We don't add any more lines — the reader will return EOF
    // after all accumulated lines are consumed.
    // Mark as ready to execute (no /exit needed).
    execute_repl(world);
}

// ---------------------------------------------------------------------------
// Given: sequential mock responses (table form)
// ---------------------------------------------------------------------------

#[given("the mock LLM returns sequential responses:")]
fn given_sequential_responses(world: &mut QuectoWorld, step: &gherkin::Step) {
    if let Some(table) = &step.table {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = wiremock::MockServer::start().await;
            let new_uri = server.uri();
            let rows: Vec<&str> = table.rows.iter().map(|r| r[0].trim()).collect();

            // Mount responses with priority-based sequencing
            for (i, content) in rows.iter().enumerate() {
                let body = serde_json::json!({
                    "id": format!("chatcmpl-seq-{}", i),
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": content
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 5,
                        "total_tokens": 15
                    }
                });

                let priority = i as u8 + 1;
                if i < rows.len() - 1 {
                    // First N-1 responses: fire once each
                    wiremock::Mock::given(wiremock::matchers::method("POST"))
                        .and(wiremock::matchers::path("/chat/completions"))
                        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                        .up_to_n_times(1)
                        .with_priority(priority)
                        .mount(&server)
                        .await;
                } else {
                    // Last response: unlimited
                    wiremock::Mock::given(wiremock::matchers::method("POST"))
                        .and(wiremock::matchers::path("/chat/completions"))
                        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                        .with_priority(priority)
                        .mount(&server)
                        .await;
                }
            }

            // Rewrite config
            let base = base_path(world);
            let workspace = base.join("workspace");
            let config_json = format!(
                r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
                new_uri = new_uri,
                workspace = workspace.display()
            );
            std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
            world._wiremock_server_uri = Some(new_uri);
            std::mem::forget(server);
        });
        std::mem::forget(rt);
    }
}

// ---------------------------------------------------------------------------
// Given: tool call + text response combo for REPL
// ---------------------------------------------------------------------------

#[given(expr = "the mock LLM returns a tool call for {string} with args {string}")]
fn given_mock_tool_call(world: &mut QuectoWorld, tool_name: String, args: String) {
    world.pending_tool_call = Some((tool_name, args));
}

#[given(expr = "then the mock LLM returns a text response {string}")]
fn given_then_text_response(world: &mut QuectoWorld, content: String) {
    let (tool_name, args_json) = world
        .pending_tool_call
        .take()
        .expect("pending_tool_call should be set by previous step");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // First response: tool call
        let tool_body = serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": format!("call_{}", tool_name),
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": args_json
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(tool_body))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;

        // Second response: text
        let text_body = serde_json::json!({
            "id": "chatcmpl-text",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&server)
            .await;

        let base = base_path(world);
        let workspace = base.join("workspace");
        let config_json = format!(
            r#"{{
  "providers": {{
    "openai": {{ "api_key": "sk-test-key", "api_base": "{new_uri}" }}
  }},
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#,
            new_uri = new_uri,
            workspace = workspace.display()
        );
        std::fs::write(base.join("config.json"), config_json).expect("rewrite config");
        world._wiremock_server_uri = Some(new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ---------------------------------------------------------------------------
// Then steps
// ---------------------------------------------------------------------------

/// For REPL scenarios: ensure the REPL has been executed before checking output.
fn ensure_repl_executed(world: &mut QuectoWorld) {
    if !world.repl_executed {
        execute_repl(world);
    }
}

// Note: stdout/stderr/exit_code assertions reuse the existing e2e_steps:
// - "stdout should contain {string}" — e2e_steps.rs
// - "the exit code should be 0"     — config_steps.rs
// These work because execute_repl() sets world.stdout, world.stderr, world.exit_code.

/// "Then quecto should enter interactive REPL mode" — for the cli.feature scenario
#[then("quecto should enter interactive REPL mode")]
fn then_enters_repl(world: &mut QuectoWorld) {
    // When run with no arguments via run_with_output, we get the REPL hint.
    // Verify exit code 0 (REPL mode entered, not an error).
    assert_eq!(
        world.exit_code, 0,
        "expected exit code 0 for REPL mode, got {}. stderr: {}",
        world.exit_code, world.stderr
    );
}

/// "Then a session file for {string} should exist in the base directory"
#[then(expr = "a session file for {string} should exist in the base directory")]
fn then_session_file_exists(world: &mut QuectoWorld, session_name: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");

    // Session key for REPL is "repl:<name>", file is "repl_<name>.json"
    let file_name = format!("repl_{}.json", session_name);
    let path = sessions_dir.join(&file_name);
    assert!(
        path.exists(),
        "expected session file at {}, but it does not exist.\nFiles in sessions/: {:?}",
        path.display(),
        std::fs::read_dir(&sessions_dir).ok().map(|entries| entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>())
    );
}

/// "And the session should contain the user message {string}"
#[then(expr = "the session should contain the user message {string}")]
fn then_session_contains_user_msg(world: &mut QuectoWorld, expected: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");

    // Find any session file and check
    let session_file = find_repl_session_file(&sessions_dir);
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_file).unwrap()).unwrap();
    let messages = session["messages"].as_array().unwrap();
    let found = messages
        .iter()
        .any(|m| m["role"].as_str() == Some("user") && m["content"].as_str() == Some(&expected));
    assert!(
        found,
        "expected user message '{}' in session, messages: {:?}",
        expected, messages
    );
}

/// "And the session should contain the assistant message {string}"
#[then(expr = "the session should contain the assistant message {string}")]
fn then_session_contains_assistant_msg(world: &mut QuectoWorld, expected: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");

    let session_file = find_repl_session_file(&sessions_dir);
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_file).unwrap()).unwrap();
    let messages = session["messages"].as_array().unwrap();
    let found = messages.iter().any(|m| {
        m["role"].as_str() == Some("assistant")
            && m["content"].as_str().is_some_and(|c| c.contains(&expected))
    });
    assert!(
        found,
        "expected assistant message containing '{}' in session, messages: {:?}",
        expected, messages
    );
}

/// "And the session should contain N user message(s)"
#[then(expr = "the session should contain {int} user message")]
fn then_session_user_message_count(world: &mut QuectoWorld, expected: usize) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");

    let session_file = find_repl_session_file(&sessions_dir);
    let session: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_file).unwrap()).unwrap();
    let messages = session["messages"].as_array().unwrap();
    let count = messages
        .iter()
        .filter(|m| m["role"].as_str() == Some("user"))
        .count();
    assert_eq!(
        count, expected,
        "expected {} user message(s) in session, found {}. messages: {:?}",
        expected, count, messages
    );
}

/// Helper: find the REPL session file in the sessions directory.
fn find_repl_session_file(sessions_dir: &Path) -> PathBuf {
    if !sessions_dir.exists() {
        panic!(
            "sessions directory does not exist at {}",
            sessions_dir.display()
        );
    }
    let entries: Vec<_> = std::fs::read_dir(sessions_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("repl_"))
        .collect();

    assert!(
        !entries.is_empty(),
        "no REPL session files found in {}",
        sessions_dir.display()
    );

    entries[0].path()
}

// Note: REPL execution is triggered automatically when "I type /exit" or
// "I type /quit" is called, so stdout/stderr/exit_code are populated
// before any Then steps run. EOF scenarios use "I send EOF" which also
// triggers execution immediately.
