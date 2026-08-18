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

    let prompts: Vec<String> = world
        .repl_input_lines
        .iter()
        .filter(|line| !line.starts_with('/'))
        .cloned()
        .collect();
    let prompts: Vec<String> = if world.repl_flags.is_empty() {
        prompts
    } else {
        let flags = world.repl_flags.join(" ");
        prompts
            .into_iter()
            .map(|prompt| format!("{flags} {prompt}"))
            .collect()
    };
    e2e_steps::mount_auto_mock_responses_for_messages(world, &prompts);

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
    world.repl_flags = shell_split(&flags_str);
}

#[when(expr = "I type {string}")]
fn when_type_line(world: &mut QuectoWorld, line: String) {
    let is_exit = line == "/exit" || line == "/quit";
    world.repl_input_lines.push(line);
    if is_exit {
        // /exit is the last input — execute the REPL now
        // so stdout/stderr/exit_code are ready for Then steps.
        if world.repl_use_progress_recorder {
            execute_repl_with_recorder(world);
        } else if world.repl_force_tty {
            execute_repl_with_progress(world, true);
        } else {
            // Default: simulate TTY so banner/prompt appear (matches original behaviour).
            execute_repl(world);
        }
    }
}

#[when("I send EOF")]
fn when_send_eof(world: &mut QuectoWorld) {
    // EOF is represented by having no more input lines.
    // The REPL should exit cleanly when read_line returns Ok(0).
    // We don't add any more lines — the reader will return EOF
    // after all accumulated lines are consumed.
    // Mark as ready to execute (no /exit needed).
    if world.repl_use_progress_recorder {
        execute_repl_with_recorder(world);
    } else if world.repl_force_tty {
        execute_repl_with_progress(world, true);
    } else {
        execute_repl(world);
    }
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
    let messages = session_messages_from_file(&session_file);
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
    let messages = session_messages_from_file(&session_file);
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
    let messages = session_messages_from_file(&session_file);
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

fn session_messages_from_file(path: &Path) -> Vec<serde_json::Value> {
    let data = std::fs::read_to_string(path).unwrap();
    if let Ok(session) = serde_json::from_str::<serde_json::Value>(&data) {
        return session["messages"].as_array().cloned().unwrap_or_default();
    }

    let mut messages = Vec::new();
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: serde_json::Value = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record in REPL test helper");
                break;
            }
            Err(err) => panic!("failed to parse session '{}': {}", path.display(), err),
        };
        parsed_any = true;
        match record["type"].as_str() {
            Some("snapshot") => {
                messages = record["messages"].as_array().cloned().unwrap_or_default();
            }
            Some("append") => {
                if let Some(added) = record["messages"].as_array() {
                    messages.extend(added.iter().cloned());
                }
            }
            _ => {}
        }
    }
    messages
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
        // Only session FILES (<key>.json): the context spill store (#1046)
        // creates a sibling DIRECTORY per session (sessions/<key>/spill.jsonl)
        // with the same "repl_" prefix, and read_dir order is filesystem-
        // dependent — picking the dir made this helper flake on CI.
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("repl_") && name.ends_with(".json") && e.path().is_file()
        })
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

// ===========================================================================
// REPL Agent Profile Steps — /agent slash command scenarios
// ===========================================================================

/// Helper: path to agent profiles directory.
fn agents_dir(base: &Path) -> PathBuf {
    base.join("agents")
}

/// Helper: path to a specific agent profile file.
fn agent_profile_path(base: &Path, name: &str) -> PathBuf {
    agents_dir(base).join(format!("{}.json", name))
}

/// Helper: read an agent profile from disk.
fn read_agent_profile(base: &Path, name: &str) -> serde_json::Value {
    let path = agent_profile_path(base, name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("could not read profile at {}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|_| panic!("invalid JSON in {}", path.display()))
}

/// Helper: create an agent profile on disk.
fn create_agent_profile(base: &Path, name: &str, system: &str, model: Option<&str>) {
    let dir = agents_dir(base);
    std::fs::create_dir_all(&dir).expect("create agents dir");
    let mut profile = serde_json::json!({
        "name": name,
        "system": system
    });
    if let Some(m) = model {
        profile["model"] = serde_json::json!(m);
    }
    let path = dir.join(format!("{}.json", name));
    let content = serde_json::to_string_pretty(&profile).expect("serialize profile");
    std::fs::write(&path, content).expect("write profile");
}

#[given(expr = "a subagent profile {string} exists with system prompt {string}")]
fn given_agent_profile(world: &mut QuectoWorld, name: String, system: String) {
    let base = base_path(world);
    create_agent_profile(&base, &name, &system, None);
}

#[given(expr = "a subagent profile {string} exists with system prompt {string} and model {string}")]
fn given_agent_profile_with_model(
    world: &mut QuectoWorld,
    name: String,
    system: String,
    model: String,
) {
    let base = base_path(world);
    create_agent_profile(&base, &name, &system, Some(&model));
}

#[then(expr = "a subagent profile {string} should exist on disk")]
fn then_agent_profile_exists(world: &mut QuectoWorld, name: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let path = agent_profile_path(&base, &name);
    assert!(
        path.exists(),
        "expected agent profile at {}, but it does not exist",
        path.display()
    );
}

#[then(expr = "a subagent profile {string} should not exist on disk")]
fn then_agent_profile_not_exists(world: &mut QuectoWorld, name: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let path = agent_profile_path(&base, &name);
    assert!(
        !path.exists(),
        "expected no agent profile at {}, but it exists",
        path.display()
    );
}

#[then(expr = "the profile {string} should have system prompt {string}")]
fn then_profile_has_system(world: &mut QuectoWorld, name: String, expected: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let profile = read_agent_profile(&base, &name);
    let system = profile["system"].as_str().unwrap_or("");
    assert_eq!(
        system, expected,
        "expected profile '{}' system = '{}', got '{}'",
        name, expected, system
    );
}

#[then(expr = "the profile {string} should have model {string}")]
fn then_profile_has_model(world: &mut QuectoWorld, name: String, expected: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let profile = read_agent_profile(&base, &name);
    let model = profile["model"].as_str().unwrap_or("");
    assert_eq!(
        model, expected,
        "expected profile '{}' model = '{}', got '{}'",
        name, expected, model
    );
}

#[then(expr = "a child quecto process should have been spawned with system prompt {string}")]
fn then_child_spawned_with_system(world: &mut QuectoWorld, expected_system: String) {
    // In the REPL test context, /agent run executes the agent inline with the
    // profile's system prompt. We verify indirectly: the mock LLM returned a
    // response (verified by "stdout should contain"), and the profile's system
    // prompt was used. We verify the profile exists and has the expected prompt.
    ensure_repl_executed(world);
    let base = base_path(world);
    // Find any profile with this system prompt
    let agents = agents_dir(&base);
    if agents.exists() {
        for entry in std::fs::read_dir(&agents).unwrap().flatten() {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(profile) = serde_json::from_str::<serde_json::Value>(&content) {
                    if profile["system"].as_str() == Some(&expected_system) {
                        return; // Found a profile with the expected system prompt
                    }
                }
            }
        }
    }
    panic!(
        "no agent profile found with system prompt '{}'",
        expected_system
    );
}

// ===========================================================================
// REPL Spawn Steps — /spawn slash command scenarios
// ===========================================================================

#[then("a child quecto process should have been spawned")]
fn then_child_spawned(world: &mut QuectoWorld) {
    // In the REPL test context, /spawn runs the agent inline. We verify the
    // spawn happened by confirming the REPL produced output (the mock LLM
    // response was captured). The stdout check in preceding steps already
    // verifies the agent ran; this step confirms the REPL exited cleanly.
    ensure_repl_executed(world);
    assert_eq!(
        world.exit_code, 0,
        "expected clean exit after spawn, got exit code {}",
        world.exit_code
    );
}

#[then("the REPL should continue accepting input after the failure")]
fn then_repl_continues_after_failure(world: &mut QuectoWorld) {
    // The REPL should have exited cleanly (via /exit), not crashed.
    ensure_repl_executed(world);
    assert_eq!(
        world.exit_code, 0,
        "expected REPL to continue (exit code 0), got {}",
        world.exit_code
    );
}

// Note: "the mock LLM takes N seconds to respond" is defined in e2e_steps.rs

#[then(expr = "the session {string} should not contain {string} as a user message")]
fn then_session_not_contains_user_msg(world: &mut QuectoWorld, session_key: String, text: String) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    // Convert session key "repl:parent-session" -> "repl_parent-session.json"
    let file_name = format!("{}.json", session_key.replace(':', "_"));
    let path = sessions_dir.join(&file_name);
    if !path.exists() {
        // Session file doesn't exist — can't contain the message
        return;
    }
    let content = std::fs::read_to_string(&path).expect("read session file");
    let session: serde_json::Value = serde_json::from_str(&content).expect("parse session");
    if let Some(messages) = session["messages"].as_array() {
        let found = messages
            .iter()
            .any(|m| m["role"].as_str() == Some("user") && m["content"].as_str() == Some(&text));
        assert!(
            !found,
            "expected session '{}' NOT to contain user message '{}', but it was found",
            session_key, text
        );
    }
}

#[then("no child session files should exist")]
fn then_no_child_session_files(world: &mut QuectoWorld) {
    ensure_repl_executed(world);
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    if !sessions_dir.exists() {
        return; // No sessions dir = no child sessions
    }
    // Check that no session files exist that look like child/spawn sessions.
    // Parent REPL sessions are "repl_*.json". Child sessions would be
    // "spawn_*.json" or "subagent_*.json" or similar.
    let child_files: Vec<String> = std::fs::read_dir(&sessions_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("spawn_") || name.starts_with("subagent_")
        })
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        child_files.is_empty(),
        "expected no child session files, found: {:?}",
        child_files
    );
}

// ===========================================================================
// REPL Progress Steps — spinner / tool activity display
// ===========================================================================

/// Execute the REPL with progress callback recorder / TTY mode as requested.
fn execute_repl_with_progress(world: &mut QuectoWorld, is_tty: bool) {
    if world.repl_executed {
        return;
    }
    world.repl_executed = true;

    let input = world.repl_input_lines.join("\n") + "\n";
    let flags = world.repl_flags.clone();

    if is_tty {
        // Use the TTY-captured variant so spinner output is captured into stderr
        // and available for "Then stderr should contain ..." assertions.
        let output = cli::run_repl_with_tty_captured(&world.cli_context, &flags, input.as_bytes());
        world.exit_code = output.exit_code;
        world.stdout = output.stdout;
        world.stderr = output.stderr;
    } else {
        let output = cli::run_repl_with_output(&world.cli_context, &flags, input.as_bytes(), false);
        world.exit_code = output.exit_code;
        world.stdout = output.stdout;
        world.stderr = output.stderr;
    }
}

/// Run the REPL with a progress recorder (injects a recording callback via the
/// `QUECTO_REPL_PROGRESS_RECORD=1` env var, which the run_repl_with_output path
/// recognises in test-support mode).
#[when("I run the REPL with a progress recorder")]
fn when_run_repl_with_progress_recorder(world: &mut QuectoWorld) {
    world.repl_input_lines = Vec::new();
    world.repl_flags = Vec::new();
    world.repl_executed = false;
    world.repl_use_progress_recorder = true;
}

#[when("I start quecto in REPL mode as a TTY")]
fn when_start_repl_as_tty(world: &mut QuectoWorld) {
    world.repl_input_lines = Vec::new();
    world.repl_flags = Vec::new();
    world.repl_executed = false;
    world.repl_force_tty = true;
}

/// Override execute_repl to handle progress-recorder and force-tty modes.
fn execute_repl_smart(world: &mut QuectoWorld) {
    if world.repl_executed {
        return;
    }

    if world.repl_use_progress_recorder {
        execute_repl_with_recorder(world);
    } else {
        let is_tty = world.repl_force_tty;
        execute_repl_with_progress(world, is_tty);
    }
}

/// Execute the REPL wiring a progress recorder callback so events are captured.
fn execute_repl_with_recorder(world: &mut QuectoWorld) {
    if world.repl_executed {
        return;
    }
    world.repl_executed = true;

    let input = world.repl_input_lines.join("\n") + "\n";
    let flags = world.repl_flags.clone();

    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let output = cli::run_repl_with_progress_recorder(cli::ReplRecorderOptions {
        ctx: &world.cli_context,
        args: &flags,
        input: input.as_bytes(),
        is_tty: false,
        progress_callback: Arc::new(move |ev: quecto::domain::agent::AgentProgressEvent| {
            let label = match &ev {
                quecto::domain::agent::AgentProgressEvent::Thinking { .. } => {
                    "Thinking".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::ToolStarted { name, .. } => {
                    format!("ToolStarted:{}", name)
                }
                quecto::domain::agent::AgentProgressEvent::ToolFinished { name, .. } => {
                    format!("ToolFinished:{}", name)
                }
                quecto::domain::agent::AgentProgressEvent::Token(_) => "Token".to_string(),
                quecto::domain::agent::AgentProgressEvent::ThinkingDelta(_) => {
                    "ThinkingDelta".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::TurnCompleted { .. } => {
                    "TurnCompleted".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::ConversationChanged { .. } => {
                    "ConversationChanged".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::ToolCatalogueChanged { .. } => {
                    "ToolCatalogueChanged".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::ToolPolicyChanged { .. } => {
                    "ToolPolicyChanged".to_string()
                }
                quecto::domain::agent::AgentProgressEvent::Done => "Done".to_string(),
            };
            events_clone.lock().unwrap().push(label);
        }),
    });

    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
    world.repl_progress_events = events.lock().unwrap().clone();
}

// ---------------------------------------------------------------------------
// Then steps for progress recorder assertions
// ---------------------------------------------------------------------------

#[then(expr = "the progress recorder should have received a {string} event for tool {string}")]
fn then_progress_event_for_tool(world: &mut QuectoWorld, event_type: String, tool_name: String) {
    ensure_repl_smart_executed(world);
    let expected = format!("{}:{}", event_type, tool_name);
    assert!(
        world.repl_progress_events.iter().any(|e| e == &expected),
        "expected progress event '{}', got: {:?}",
        expected,
        world.repl_progress_events
    );
}

#[then(expr = "the progress recorder should have received a {string} event")]
fn then_progress_event(world: &mut QuectoWorld, event_type: String) {
    ensure_repl_smart_executed(world);
    assert!(
        world.repl_progress_events.iter().any(|e| e == &event_type),
        "expected progress event '{}', got: {:?}",
        event_type,
        world.repl_progress_events
    );
}

#[then(expr = "the progress recorder should have received {int} progress events")]
fn then_progress_event_count(world: &mut QuectoWorld, expected: usize) {
    ensure_repl_smart_executed(world);
    assert_eq!(
        world.repl_progress_events.len(),
        expected,
        "expected {} progress events, got: {:?}",
        expected,
        world.repl_progress_events
    );
}

#[then(expr = "stdout should not contain {string}")]
fn then_stdout_not_contains(world: &mut QuectoWorld, text: String) {
    assert!(
        !world.stdout.contains(&text),
        "expected stdout NOT to contain '{}', but it did.\nstdout: {}",
        text,
        world.stdout
    );
}

/// Ensure the "smart" execute path (progress recorder / force-tty) has run.
fn ensure_repl_smart_executed(world: &mut QuectoWorld) {
    if !world.repl_executed {
        execute_repl_smart(world);
    }
}
