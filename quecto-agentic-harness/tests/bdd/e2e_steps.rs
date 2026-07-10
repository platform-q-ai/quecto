use super::*;

// Agent CLI — Headless One-Shot Mode Steps
// ===========================================================================

#[given("a temp base directory")]
fn given_temp_base_directory(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

#[given("a config file with an OpenAI provider pointing at a mock server")]
fn given_config_with_openai_mock(world: &mut QuectoWorld) {
    // Start a wiremock server and leak it so it stays alive for the scenario.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    rewrite_config_to_uri(world, &uri);
    world.mock_provider_kind = Some("openai".to_string());

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given("a mocked OpenAI workspace is configured")]
fn given_mocked_openai_workspace(world: &mut QuectoWorld) {
    configure_mock_provider_workspace(world, "openai");
}

#[given("a mocked Anthropic workspace is configured")]
fn given_mocked_anthropic_workspace(world: &mut QuectoWorld) {
    configure_mock_provider_workspace(world, "anthropic");
}

fn configure_mock_provider_workspace(world: &mut QuectoWorld, provider: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");
    rewrite_config_to_provider_uri(world, provider, &uri);
    world.mock_provider_kind = Some(provider.to_string());

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the mock LLM returns a text response {string}")]
fn given_mock_llm_text_response(world: &mut QuectoWorld, content: String) {
    // Verify a mock server was previously configured (from the config step).
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — ensure a config step ran first"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // wiremock doesn't support reconnecting to an existing server.
        // Start a new server, mount the mock, and rewrite the config to point at it.
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let response_body = serde_json::json!({
            "id": "chatcmpl-test",
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
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

/// #1047: content big enough that an un-tailed `turn_end`/`agent_end` line
/// would exceed the protocol event line cap
/// (`quecto::interface::cli::protocol::EVENT_LINE_CAP_BYTES`).
#[given("the mock LLM returns a text response larger than the event line cap")]
fn given_mock_llm_oversized_text_response(world: &mut QuectoWorld) {
    let content = "x".repeat(2 * quecto::interface::cli::protocol::EVENT_LINE_CAP_BYTES);
    given_mock_llm_text_response(world, content);
}

#[given(expr = "the mock provider returns a text response {string}")]
fn given_mock_provider_text_response(world: &mut QuectoWorld, content: String) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — ensure a mock provider workspace step ran first"
    );
    let provider = world
        .mock_provider_kind
        .as_deref()
        .unwrap_or("openai")
        .to_string();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let (path, response_body) = match provider.as_str() {
            "anthropic" => (
                "/v1/messages",
                serde_json::json!({
                    "id": "msg_mock",
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "text", "text": content }],
                    "stop_reason": "end_turn",
                    "usage": { "input_tokens": 10, "output_tokens": 5 }
                }),
            ),
            _ => ("/chat/completions", openai_text_json(&content)),
        };

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(path))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;

        rewrite_config_to_provider_uri(world, &provider, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given("the mock LLM returns an HTTP 500 error")]
fn given_mock_llm_500_error(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given("no config file exists")]
fn given_no_config_file(world: &mut QuectoWorld) {
    // Delete config.json if the temp dir already has one (from prior Given steps).
    let config_path = base_path(world).join("config.json");
    if config_path.exists() {
        std::fs::remove_file(&config_path).expect("remove config.json");
    }
}

#[given("a config file with no API keys")]
fn given_config_no_api_keys(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let config_json = r#"{
  "providers": {
    "openai": { "api_key": "" },
    "anthropic": { "api_key": "" }
  }
}"#;
    std::fs::write(base.join("config.json"), config_json).expect("write config");
}

/// Generic step: "When I run quecto agent ..." with arbitrary flags.
/// Parses the full argument string after "quecto " using shell-like splitting.
#[when(expr = "I run quecto agent -m {string}")]
fn when_run_agent_with_message(world: &mut QuectoWorld, message: String) {
    if world.run_agent_via_subprocess_with_env_base_dir {
        let raw_args = format!("agent -m '{}'", message);
        spawn_quecto_subprocess(world, &raw_args);
        world.exit_code = world.subprocess_exit_code.unwrap_or(-1);
        world.stdout = world.subprocess_stdout.clone().unwrap_or_default();
        world.stderr = world.subprocess_stderr.clone().unwrap_or_default();
        world.run_agent_via_subprocess_with_env_base_dir = false;
        return;
    }

    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I run quecto agent with no flags")]
fn when_run_agent_no_flags(world: &mut QuectoWorld) {
    let args = vec!["quecto".to_string(), "agent".to_string()];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --system {string} -m {string}")]
fn when_run_agent_with_system_and_message(
    world: &mut QuectoWorld,
    system: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--system".to_string(),
        system,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --model {word} -m {string}")]
fn when_run_agent_with_model_and_message(world: &mut QuectoWorld, model: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        model,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --model {string} -s - -m {string}")]
fn when_run_agent_with_model_ephemeral_session(
    world: &mut QuectoWorld,
    model: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        model,
        "-s".to_string(),
        "-".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I set QUECTO_BASE_DIR to the temp directory")]
fn when_set_quecto_base_dir_env(world: &mut QuectoWorld) {
    // Do not mutate process-global env in BDD runner.
    // Instead, mark the next "run quecto agent" step to run as a subprocess
    // with QUECTO_BASE_DIR set in child env.
    world.run_agent_via_subprocess_with_env_base_dir = true;
}

#[then(expr = "stdout should contain {string}")]
fn then_stdout_contains(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stdout.contains(&expected),
        "expected stdout to contain '{}', got: {}",
        expected,
        world.stdout
    );
}

#[then(expr = "stderr should contain {string}")]
fn then_stderr_contains_e2e(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stderr.contains(&expected),
        "expected stderr to contain '{}', got: {}",
        expected,
        world.stderr
    );
}

/// Assert that stdout is not empty (structural check for non-deterministic output).
#[then("stdout should not be empty")]
fn then_stdout_not_empty(world: &mut QuectoWorld) {
    assert!(
        !world.stdout.trim().is_empty(),
        "expected non-empty stdout, got empty.\nstderr: {}",
        world.stderr
    );
}

// ===========================================================================
// E2E Tool Use + E2E Session Steps
// ===========================================================================

/// Helper: build the OpenAI-format JSON for a tool call response.
fn openai_tool_call_json(tool_name: &str, args_json: &str) -> serde_json::Value {
    serde_json::json!({
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
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
}

/// Convert a mocked OpenAI chat-completion JSON body into the minimal SSE
/// stream the in-repo parser accepts. Auto-mocked UDS runs stream since
/// #1060 (completion events are bounded, so scenario assertions reconstruct
/// response text from token events — which only exist when the provider
/// actually streams), so every mocked completion must go over the wire as
/// SSE rather than a plain JSON body the SSE pump would read as zero deltas.
fn openai_json_to_sse(body: &serde_json::Value) -> String {
    let msg = &body["choices"][0]["message"];
    let mut out = String::new();
    if let Some(content) = msg["content"].as_str() {
        if !content.is_empty() {
            let chunk = serde_json::json!({"choices":[{"delta":{"content": content}}]});
            out.push_str(&format!("data: {chunk}\n\n"));
        }
    }
    if let Some(calls) = msg["tool_calls"].as_array() {
        for (i, call) in calls.iter().enumerate() {
            let chunk = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
                "index": i,
                "id": call["id"],
                "function": {
                    "name": call["function"]["name"],
                    "arguments": call["function"]["arguments"],
                }
            }]}}]});
            out.push_str(&format!("data: {chunk}\n\n"));
        }
    }
    if let Some(usage) = body.get("usage") {
        let chunk = serde_json::json!({"choices": [], "usage": usage});
        out.push_str(&format!("data: {chunk}\n\n"));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

/// Anthropic twin of [`openai_json_to_sse`]: convert a mocked Messages-API
/// JSON body into the minimal Anthropic SSE event sequence the in-repo
/// parser accepts.
fn anthropic_json_to_sse(body: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = body["content"].as_array() {
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    let chunk =
                        serde_json::json!({"delta":{"type":"text_delta","text": block["text"]}});
                    out.push_str(&format!("event: content_block_delta\ndata: {chunk}\n\n"));
                }
                Some("tool_use") => {
                    let start = serde_json::json!({"content_block":{
                        "type":"tool_use","id": block["id"], "name": block["name"]}});
                    out.push_str(&format!("event: content_block_start\ndata: {start}\n\n"));
                    let input =
                        serde_json::to_string(&block["input"]).unwrap_or_else(|_| "{}".into());
                    let delta = serde_json::json!({"delta":{
                        "type":"input_json_delta","partial_json": input}});
                    out.push_str(&format!("event: content_block_delta\ndata: {delta}\n\n"));
                    out.push_str("event: content_block_stop\ndata: {}\n\n");
                }
                _ => {}
            }
        }
    }
    out.push_str("event: message_stop\ndata: {}\n\n");
    out
}

/// Wiremock responder serving the SSE encoding to streaming requests
/// (`"stream":true` in the body — set by both providers' streaming paths)
/// and the plain JSON completion to non-streaming ones. One responder per
/// mock keeps priorities and `up_to_n_times` sequencing identical while the
/// same mounted mock serves UDS (streaming) and subprocess/REPL
/// (non-streaming) scenarios correctly.
struct DualEncodingResponder {
    json: serde_json::Value,
    sse: String,
}

impl wiremock::Respond for DualEncodingResponder {
    fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        if body.contains("\"stream\":true") {
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(self.sse.clone())
        } else {
            wiremock::ResponseTemplate::new(200).set_body_json(self.json.clone())
        }
    }
}

/// Build a [`DualEncodingResponder`] for an OpenAI-shaped completion body.
fn openai_dual(body: serde_json::Value) -> DualEncodingResponder {
    let sse = openai_json_to_sse(&body);
    DualEncodingResponder { json: body, sse }
}

/// Build a [`DualEncodingResponder`] for an Anthropic-shaped message body.
fn anthropic_dual(body: serde_json::Value) -> DualEncodingResponder {
    let sse = anthropic_json_to_sse(&body);
    DualEncodingResponder { json: body, sse }
}

/// Helper: build the OpenAI-format JSON for a text response.
fn openai_text_json(content: &str) -> serde_json::Value {
    serde_json::json!({
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
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
}

fn extract_quoted_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let quote = rest.find(['\'', '"'])?;
    let quote_char = rest.as_bytes()[quote] as char;
    let after = &rest[quote + 1..];
    let end = after.find(quote_char)?;
    Some(&after[..end])
}

fn extract_after_marker(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)? + marker.len();
    let rest = text[start..].trim();
    let value = rest
        .split(['.', ',', ';'])
        .next()
        .unwrap_or(rest)
        .trim()
        .trim_matches(['\'', '"']);
    (!value.is_empty()).then(|| value.to_string())
}

fn first_path_with_suffix(text: &str, suffixes: &[&str]) -> Option<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.'))
        .find(|token| suffixes.iter().any(|suffix| token.ends_with(suffix)))
        .map(ToString::to_string)
}

fn requested_markers(text: &str) -> Vec<String> {
    let mut markers = Vec::new();
    for raw in text.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '.' | ':' | ';' | ')' | '(' | '\'' | '"')
    }) {
        let token = raw.trim();
        if token.len() >= 3
            && (token.contains('_')
                || token.contains('-')
                || token
                    .chars()
                    .filter(|c| c.is_ascii_alphabetic())
                    .all(|c| c.is_ascii_uppercase()))
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            markers.push(token.to_string());
        }
        if token.len() >= 2 && token.chars().all(|c| c.is_ascii_digit()) {
            markers.push(token.to_string());
        }
        if [".txt", ".md", ".rs", ".json"]
            .iter()
            .any(|suffix| token.ends_with(suffix))
        {
            markers.push(token.to_string());
        }
    }
    for marker in [
        "turquoise",
        "kiwi river",
        "orange cloud",
        "pineapple-72",
        "ember-77",
        "mango-9081",
        "kiwi-11",
        "melon-22",
        "entry-token-44",
        "grape-313",
        "55321",
        "7319",
        "Arrr!",
    ] {
        if text.contains(marker) {
            markers.push(marker.to_string());
        }
    }
    markers.sort();
    markers.dedup();
    markers
}

pub(crate) fn final_text_for_prompt(prompt: &str) -> String {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("current workflow template") || lower.contains("shown in your system prompt")
    {
        return "Feature 77".to_string();
    }
    if lower.contains("src/*.rs") {
        return "app.rs lib.rs".to_string();
    }
    if lower.contains("*.md") {
        return "readme.md guide.md".to_string();
    }
    if lower.contains("repl-list-a.txt") || lower.contains("repl-list-b.txt") {
        return "repl-list-a.txt repl-list-b.txt".to_string();
    }
    if lower.contains("list-a.txt") || lower.contains("list-b.txt") {
        return "list-a.txt list-b.txt".to_string();
    }
    if lower.contains("alpha.txt") || lower.contains("beta.txt") || lower.contains("list the files")
    {
        return "alpha.txt beta.txt".to_string();
    }
    let markers = requested_markers(prompt);
    if markers.is_empty() {
        let mut fallback = "OK turquoise kiwi river orange cloud pineapple-72 ember-77 mango-9081 kiwi-11 melon-22 entry-token-44 grape-313 55321 7319".to_string();
        if prompt.to_ascii_lowercase().contains("spawn") {
            fallback.push_str(" spawn");
        }
        fallback
    } else {
        markers.join(" ")
    }
}

fn workflow_call(action: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("action".to_string(), serde_json::json!(action));
    if let Some(extra_obj) = extra.as_object() {
        for (key, value) in extra_obj {
            obj.insert(key.clone(), value.clone());
        }
    }
    openai_tool_call_json("workflow", &serde_json::Value::Object(obj).to_string())
}

fn tool_call_for_prompt(prompt: &str) -> Option<(&'static str, String)> {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("run command") || lower.contains("run the command") {
        let command = extract_quoted_after(prompt, "command ")
            .map(ToString::to_string)
            .or_else(|| extract_after_marker(prompt, "Run command "));
        if let Some(command) = command {
            return Some((
                "bash",
                serde_json::json!({ "command": command }).to_string(),
            ));
        }
    }

    if lower.contains("replace") || lower.contains("edit") || lower.contains("change") {
        if let Some(path) = first_path_with_suffix(prompt, &[".txt", ".json", ".rs", ".md"]) {
            let (old_text, new_text) = if lower.contains("debug") && lower.contains("release") {
                ("debug", "release")
            } else if lower.contains("alpha") && lower.contains("beta") {
                ("alpha", "beta")
            } else if lower.contains("middle") && lower.contains("center") {
                ("middle", "center")
            } else if lower.contains("state=old") && lower.contains("state=new") {
                ("state=old", "state=new")
            } else {
                return None;
            };
            return Some((
                "edit",
                serde_json::json!({ "path": path, "oldText": old_text, "newText": new_text })
                    .to_string(),
            ));
        }
    }

    if lower.contains("append") {
        if let Some(path) = first_path_with_suffix(prompt, &[".txt", ".log"]) {
            let content = if lower.contains("line2") {
                "\nline2"
            } else if lower.contains("second") {
                "\nsecond"
            } else if lower.contains("seed42") {
                "\nSEED42"
            } else {
                "\nappended"
            };
            return Some((
                "bash",
                serde_json::json!({ "command": format!("printf '{}' >> {}", content, path) })
                    .to_string(),
            ));
        }
    }

    if lower.contains("read") {
        if let Some(path) = first_path_with_suffix(prompt, &[".txt", ".json", ".rs", ".md"]) {
            return Some(("read", serde_json::json!({ "path": path }).to_string()));
        }
    }

    if lower.contains("create") {
        if let Some(path) = first_path_with_suffix(prompt, &[".txt", ".json", ".md"]) {
            let content = if let Some(value) = extract_after_marker(prompt, "content ") {
                value
            } else if let Some(value) = extract_after_marker(prompt, "containing exactly ") {
                value
            } else if let Some(value) = extract_quoted_after(prompt, "containing ") {
                value.to_string()
            } else {
                final_text_for_prompt(prompt)
            };
            return Some((
                "write",
                serde_json::json!({ "path": path, "content": content }).to_string(),
            ));
        }
    }

    if lower.contains("workflow tool") || lower.contains("using the workflow tool") {
        let args = if lower.contains("select_template") || lower.contains("select template") {
            serde_json::json!({ "action": "select_template", "template": "feature", "issueNumber": 42, "issueTitle": "Mock issue" })
        } else if lower.contains("check_guards") {
            serde_json::json!({ "action": "check_guards", "command": "git commit" })
        } else if lower.contains("check step") || lower.contains("check every step") {
            serde_json::json!({ "action": "check", "step": 1 })
        } else if lower.contains("skip step") {
            serde_json::json!({ "action": "skip", "step": 5 })
        } else if lower.contains("set_issue") {
            serde_json::json!({ "action": "set_issue", "issueNumber": 99, "issueTitle": "Test issue" })
        } else if lower.contains("clear_issue") {
            serde_json::json!({ "action": "clear_issue" })
        } else if lower.contains("reset") {
            serde_json::json!({ "action": "reset" })
        } else if lower.contains("list_templates") {
            serde_json::json!({ "action": "list_templates" })
        } else {
            serde_json::json!({ "action": "status" })
        };
        return Some(("workflow", args.to_string()));
    }

    None
}

fn responses_for_prompt(prompt: &str) -> Vec<serde_json::Value> {
    let lower = prompt.to_ascii_lowercase();
    let mut responses = Vec::new();

    if lower.contains("create two files") || lower.contains("create files") {
        let markers = requested_markers(prompt);
        let first_content = markers
            .first()
            .cloned()
            .unwrap_or_else(|| "hello".to_string());
        let second_content = markers
            .get(1)
            .cloned()
            .unwrap_or_else(|| "world".to_string());
        let paths: Vec<String> = prompt
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.')
            })
            .filter(|token| token.ends_with(".txt") || token.ends_with(".json"))
            .map(ToString::to_string)
            .collect();
        if paths.len() >= 2 {
            responses.push(openai_tool_call_json(
                "write",
                &serde_json::json!({ "path": paths[0], "content": first_content }).to_string(),
            ));
            responses.push(openai_tool_call_json(
                "write",
                &serde_json::json!({ "path": paths[1], "content": second_content }).to_string(),
            ));
        }
    } else if lower.contains("read") && lower.contains("append") {
        let paths: Vec<String> = prompt
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.')
            })
            .filter(|token| token.ends_with(".txt") || token.ends_with(".json"))
            .map(ToString::to_string)
            .collect();
        if paths.len() >= 2 {
            responses.push(openai_tool_call_json(
                "read",
                &serde_json::json!({ "path": paths[0] }).to_string(),
            ));
            responses.push(openai_tool_call_json(
                "bash",
                &serde_json::json!({ "command": format!("printf '\nSEED42' >> {}", paths[1]) })
                    .to_string(),
            ));
        }
    } else if lower.contains("read") && lower.contains("create") {
        let paths: Vec<String> = prompt
            .split_whitespace()
            .map(|token| {
                token.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.')
            })
            .filter(|token| token.ends_with(".txt") || token.ends_with(".json"))
            .map(ToString::to_string)
            .collect();
        if paths.len() >= 2 {
            let content = requested_markers(prompt)
                .into_iter()
                .find(|marker| marker.chars().any(|c| c.is_ascii_digit()))
                .unwrap_or_else(|| {
                    if paths.get(1).is_some_and(|path| path == "result.txt") {
                        "42".to_string()
                    } else {
                        "2468".to_string()
                    }
                });
            responses.push(openai_tool_call_json(
                "read",
                &serde_json::json!({ "path": paths[0] }).to_string(),
            ));
            responses.push(openai_tool_call_json(
                "write",
                &serde_json::json!({ "path": paths[1], "content": content }).to_string(),
            ));
        }
    } else if lower.contains("workflow tool") || lower.contains("using the workflow tool") {
        if lower.contains("select_template")
            || lower.contains("select_template feature")
            || lower.contains("select template")
            || lower.contains("current workflow template")
            || lower.contains("check_guards")
            || lower.contains("check every step")
            || lower.contains("check step")
            || lower.contains("skip step")
            || lower.contains("reset")
        {
            responses.push(workflow_call(
                "select_template",
                serde_json::json!({ "template": "feature", "issueNumber": 77, "issueTitle": "Auth regression" }),
            ));
        }
        if lower.contains("list_templates") {
            responses.push(workflow_call("list_templates", serde_json::json!({})));
        }
        if lower.contains("all steps complete") {
            for step in 1..=19 {
                responses.push(workflow_call("check", serde_json::json!({ "step": step })));
            }
            responses.push(workflow_call("status", serde_json::json!({})));
        } else if lower.contains("check every step from 1 through 7") {
            for step in 1..=7 {
                responses.push(workflow_call("check", serde_json::json!({ "step": step })));
            }
            responses.push(workflow_call(
                "check_guards",
                serde_json::json!({ "command": "git commit" }),
            ));
        } else if lower.contains("try to check step 3") {
            responses.push(workflow_call("check", serde_json::json!({ "step": 3 })));
        } else if lower.contains("skip step 5") {
            responses.push(workflow_call("skip", serde_json::json!({ "step": 5 })));
            responses.push(workflow_call("status", serde_json::json!({})));
        } else if lower.contains("check_guards") {
            responses.push(workflow_call(
                "check_guards",
                serde_json::json!({ "command": "git commit" }),
            ));
        } else if lower.contains("set_issue") {
            responses.push(workflow_call(
                "set_issue",
                serde_json::json!({ "issueNumber": 99, "issueTitle": "Fix auth" }),
            ));
        }
        let scripted_many_checks = lower.contains("all steps complete")
            || lower.contains("check every step from 1 through 7");
        if !scripted_many_checks
            && (lower.contains("check step 1") || lower.contains("check every step"))
        {
            responses.push(workflow_call("check", serde_json::json!({ "step": 1 })));
        }
        if !scripted_many_checks
            && (lower.contains("check step 2") || lower.contains("progress done 2"))
        {
            responses.push(workflow_call("check", serde_json::json!({ "step": 2 })));
        }
        if lower.contains("clear_issue") {
            responses.push(workflow_call("clear_issue", serde_json::json!({})));
        }
        if lower.contains("reset") {
            responses.push(workflow_call("reset", serde_json::json!({})));
        }
    }

    if let Some((tool, args)) = tool_call_for_prompt(prompt) {
        if responses.is_empty() {
            responses.push(openai_tool_call_json(tool, &args));
        }
    }
    responses.push(openai_text_json(&final_text_for_prompt(prompt)));
    responses
}

pub(crate) fn mount_auto_mock_responses_for_messages(world: &mut QuectoWorld, messages: &[String]) {
    if !world.auto_mock_manual_llm || messages.is_empty() {
        return;
    }

    if messages
        .iter()
        .any(|message| message.contains("UDS_TOKENS_OK"))
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = wiremock::MockServer::start().await;
            let new_uri = server.uri();
            let sse_body = "data: {\"id\":\"chatcmpl-auto\",\"choices\":[{\"delta\":{\"content\":\"UDS_\"}}]}\n\n\
                            data: {\"id\":\"chatcmpl-auto\",\"choices\":[{\"delta\":{\"content\":\"TOKENS_OK\"}}]}\n\n\
                            data: [DONE]\n\n";
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(sse_body),
                )
                .mount(&server)
                .await;
            rewrite_config_to_uri(world, &new_uri);
            rewrite_config_to_provider_uri(world, "anthropic", &new_uri);
            std::mem::forget(server);
        });
        std::mem::forget(rt);
        return;
    }

    let responses: Vec<serde_json::Value> = messages
        .iter()
        .flat_map(|message| responses_for_prompt(message))
        .collect();
    let anthropic_responses: Vec<serde_json::Value> = messages
        .iter()
        .map(|message| anthropic_text_json(&final_text_for_prompt(message)))
        .collect();
    let marker_responses: Vec<(String, String)> = messages
        .iter()
        .flat_map(|message| {
            requested_markers(message)
                .into_iter()
                .filter(|marker| marker.starts_with("UDS_"))
                .map(|marker| (marker.clone(), marker))
        })
        .collect();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        for (i, (marker, text)) in marker_responses.iter().rev().enumerate() {
            let priority = u8::try_from(i + 1).expect("too many auto mock marker responses");
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .and(wiremock::matchers::body_string_contains(marker))
                .respond_with(openai_dual(openai_text_json(text)))
                .with_priority(priority)
                .mount(&server)
                .await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/v1/messages"))
                .and(wiremock::matchers::body_string_contains(marker))
                .respond_with(anthropic_dual(anthropic_text_json(text)))
                .with_priority(priority)
                .mount(&server)
                .await;
        }
        let last = responses.len().saturating_sub(1);
        for (i, body) in responses.into_iter().enumerate() {
            let mock = wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(openai_dual(body))
                .with_priority(u8::try_from(i + 50).expect("too many auto mock responses"));
            if i < last {
                mock.up_to_n_times(1).mount(&server).await;
            } else {
                mock.mount(&server).await;
            }
        }
        let anthropic_last = anthropic_responses.len().saturating_sub(1);
        for (i, body) in anthropic_responses.into_iter().enumerate() {
            let mock = wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/v1/messages"))
                .respond_with(anthropic_dual(body))
                .with_priority(u8::try_from(i + 50).expect("too many auto mock responses"));
            if i < anthropic_last {
                mock.up_to_n_times(1).mount(&server).await;
            } else {
                mock.mount(&server).await;
            }
        }
        rewrite_config_to_uri(world, &new_uri);
        rewrite_config_to_provider_uri(world, "anthropic", &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

/// Helper: rewrite config to point at a new wiremock URI (shared pattern).
///
/// Preserves any existing config fields (e.g. health, channels) by reading
/// the current config, merging the provider/workspace fields, and writing back.
pub(crate) fn rewrite_config_to_uri(world: &mut QuectoWorld, new_uri: &str) {
    rewrite_config_to_provider_uri(world, "openai", new_uri);
}

pub(crate) fn rewrite_config_to_provider_uri(
    world: &mut QuectoWorld,
    provider: &str,
    new_uri: &str,
) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    let config_path = base.join("config.json");

    // Read existing config or start from empty object.
    let mut config: serde_json::Value = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    // Merge provider and workspace settings.
    let api_key = if provider == "anthropic" {
        "sk-ant-test-key"
    } else {
        "sk-test-key"
    };
    config["providers"][provider]["api_key"] = serde_json::json!(api_key);
    config["providers"][provider]["api_base"] = serde_json::json!(new_uri);
    config["agents"]["defaults"]["workspace"] = serde_json::json!(workspace.display().to_string());

    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(&config_path, &config_json).expect("rewrite config");

    // Also update the custom config path if one was set (for --config flag scenarios).
    if let Some(ref custom_path) = world.custom_config_path {
        std::fs::write(custom_path, &config_json).expect("rewrite custom config");
    }

    world._wiremock_server_uri = Some(new_uri.to_string());
    world.mock_provider_kind = Some(provider.to_string());
}

/// Helper: mount a two-response wiremock sequence — first a tool call, then a text response.
/// Uses priority: tool call at priority 2 with up_to_n_times(1), text at priority 1 (default).
fn mount_tool_then_text_sequence(
    world: &mut QuectoWorld,
    tool_name: &str,
    args_json: &str,
    final_text: &str,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // First call: return tool call (higher priority, consumed once)
        let tool_body = openai_tool_call_json(tool_name, args_json);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(tool_body))
            .up_to_n_times(1)
            .with_priority(1) // higher priority (lower number = higher priority in wiremock)
            .mount(&server)
            .await;

        // Second call onward: return text response (lower priority)
        let text_body = openai_text_json(final_text);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Given: e2e workspace file creation ---

#[given(expr = "a file {string} in the e2e workspace with content {string}")]
fn given_file_in_e2e_workspace(world: &mut QuectoWorld, filename: String, content: String) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let path = workspace.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, content).expect("write file");
}

// --- Given: wiremock tool-call mocks ---

#[given(expr = "the mock LLM first returns a tool call for {string} with args:")]
fn given_mock_llm_tool_call(world: &mut QuectoWorld, step: &gherkin::Step, tool_name: String) {
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);
    // Store for later pairing with the "then returns text" step.
    world.pending_tool_call = Some((tool_name, args_json));
}

#[given(expr = "the mock LLM then returns a text response {string}")]
fn given_mock_llm_then_text_response(world: &mut QuectoWorld, content: String) {
    if let Some((tool_name, args_json)) = world.pending_tool_call.take() {
        mount_tool_then_text_sequence(world, &tool_name, &args_json, &content);
    } else {
        // No pending tool call — just mount a plain text response (same as existing step).
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = wiremock::MockServer::start().await;
            let new_uri = server.uri();
            let body = openai_text_json(&content);
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            rewrite_config_to_uri(world, &new_uri);
            std::mem::forget(server);
        });
        std::mem::forget(rt);
    }
}

// Multi-turn tool call sequence from a table:
//   | call | read_file  | {"path":"source.txt"} |
//   | call | write_file | {"path":"copy.txt","content":"data"} |
//   | text | Done       |                       |
#[given("the mock LLM returns a tool call sequence:")]
fn given_mock_llm_tool_call_sequence(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");

    // Collect responses in order.
    let mut responses: Vec<serde_json::Value> = Vec::new();
    for row in &table.rows {
        let kind = &row[0];
        match kind.as_str() {
            "call" => {
                let tool_name = &row[1];
                let args_json = &row[2];
                responses.push(openai_tool_call_json(tool_name, args_json));
            }
            "text" => {
                let content = &row[1];
                responses.push(openai_text_json(content));
            }
            _ => panic!("Unknown sequence row kind: {kind}"),
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // Mount each response with decreasing priority and up_to_n_times(1).
        // Priority 1 = highest (first consumed), then 2, etc. Last one has no limit.
        let last = responses.len() - 1;
        for (i, body) in responses.into_iter().enumerate() {
            let mock = wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
                .with_priority(
                    u8::try_from(i + 1).expect("too many mock responses for u8 priority"),
                );
            if i < last {
                mock.up_to_n_times(1).mount(&server).await;
            } else {
                mock.mount(&server).await;
            }
        }

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Given: pre-existing session ---

#[given(expr = "a pre-existing session {string} with {int} messages")]
fn given_pre_existing_session(world: &mut QuectoWorld, key: String, count: usize) {
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    // Build a session file with the requested number of user/assistant message pairs.
    let mut messages = Vec::new();
    for i in 0..count {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        let content = format!("message {}", i + 1);
        messages.push(serde_json::json!({
            "role": role,
            "content": content
        }));
    }
    let session_file = serde_json::json!({
        "key": key,
        "messages": messages
    });
    // The filename uses : -> _ replacement and .json suffix.
    let filename = key.replace(':', "_") + ".json";
    std::fs::write(
        sessions_dir.join(&filename),
        serde_json::to_string_pretty(&session_file).unwrap(),
    )
    .expect("write session file");
}

// --- When: run agent with session flags ---

#[when(expr = "I run quecto agent --no-session -m {string}")]
fn when_run_agent_no_session(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--no-session".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I run quecto agent --no-session -s mysession -m \"hello\"")]
fn when_run_agent_no_session_with_s(world: &mut QuectoWorld) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--no-session".to_string(),
        "-s".to_string(),
        "mysession".to_string(),
        "-m".to_string(),
        "hello".to_string(),
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// --- #402: --disable-tool flag ---

#[when(expr = "I run quecto agent --disable-tool {word} -m {string}")]
fn when_run_agent_disable_tool(world: &mut QuectoWorld, tool: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--disable-tool".to_string(),
        tool,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --disable-tool {word} --disable-tool {word} -m {string}")]
fn when_run_agent_disable_two_tools(
    world: &mut QuectoWorld,
    tool1: String,
    tool2: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--disable-tool".to_string(),
        tool1,
        "--disable-tool".to_string(),
        tool2,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ===========================================================================
// #416: --effort flag
// ===========================================================================

#[when(expr = "I run quecto agent --effort {word} -m {string}")]
fn when_run_agent_effort(world: &mut QuectoWorld, effort: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--effort".to_string(),
        effort,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --no-sandbox -m {string}")]
fn when_run_agent_no_sandbox(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--no-sandbox".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent --no-sandbox --no-session -m {string}")]
fn when_run_agent_no_sandbox_no_session(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--no-sandbox".to_string(),
        "--no-session".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I run quecto help")]
fn when_run_quecto_help(world: &mut QuectoWorld) {
    let args = vec!["quecto".to_string(), "help".to_string()];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent -s {word} -m {string}")]
fn when_run_agent_named_session(world: &mut QuectoWorld, session: String, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent -s {word} --system {string} -m {string}")]
fn when_run_agent_session_system(
    world: &mut QuectoWorld,
    session: String,
    system: String,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--system".to_string(),
        system,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// --- Then: e2e workspace file assertions ---

#[then(expr = "the file {string} should exist in the e2e workspace")]
fn then_file_exists_in_e2e_workspace(world: &mut QuectoWorld, filename: String) {
    let base = base_path(world);
    let path = base.join("workspace").join(&filename);
    assert!(
        path.exists(),
        "expected file '{}' to exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the file {string} in the e2e workspace should contain {string}")]
fn then_file_in_e2e_workspace_contains(
    world: &mut QuectoWorld,
    filename: String,
    expected: String,
) {
    let base = base_path(world);
    let path = base.join("workspace").join(&filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read '{}' at {}: {}", filename, path.display(), e));
    assert!(
        content.contains(&expected),
        "expected '{}' to contain '{}', got: {}",
        filename,
        expected,
        content
    );
}

// --- Then: session file assertions ---

#[then(expr = "a session file should exist for key {string}")]
fn then_session_file_exists(world: &mut QuectoWorld, key: String) {
    let base = base_path(world);
    let filename = key.replace(':', "_") + ".json";
    let path = base.join("sessions").join(&filename);
    assert!(
        path.exists(),
        "expected session file for key '{}' at {}",
        key,
        path.display()
    );
}

#[then(expr = "the session {string} should contain {int} conversation messages")]
fn then_session_has_n_messages(world: &mut QuectoWorld, key: String, expected: usize) {
    // The pinned spill manifest is context machinery (#951/#1046), not part
    // of the persisted conversation under test.
    let session = load_session_from_disk(world, &key);
    let conversation: Vec<_> = session.messages.iter().filter(|m| !m.is_manifest).collect();
    assert_eq!(
        conversation.len(),
        expected,
        "expected session '{}' to have {} messages, got {} (messages: {:?})",
        key,
        expected,
        conversation.len(),
        conversation
            .iter()
            .map(|m| format!("{}:{}", m.role, &m.content[..m.content.len().min(40)]))
            .collect::<Vec<_>>()
    );
}

#[then(expr = "the session {string} should contain at least {int} messages")]
fn then_session_has_at_least_n_messages(world: &mut QuectoWorld, key: String, expected: usize) {
    let session = load_session_from_disk(world, &key);
    assert!(
        session.messages.len() >= expected,
        "expected session '{}' to have at least {} messages, got {}",
        key,
        expected,
        session.messages.len()
    );
}

#[then(expr = "the session {string} should not contain text {string}")]
fn then_session_not_contain_text(world: &mut QuectoWorld, key: String, text: String) {
    let session = load_session_from_disk(world, &key);
    let all_content: String = session
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !all_content.contains(&text),
        "expected session '{}' to NOT contain '{}', but found it in: {}",
        key,
        text,
        all_content
    );
}

#[then("no session files should exist")]
fn then_no_session_files(world: &mut QuectoWorld) {
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    if !sessions_dir.exists() {
        return; // No sessions dir = no session files, as expected.
    }
    let entries: Vec<_> = std::fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        entries.is_empty(),
        "expected no session files, but found: {:?}",
        entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
    );
}

#[then(expr = "the session {string} should not include a system role message")]
fn then_session_no_system_messages(world: &mut QuectoWorld, key: String) {
    let session = load_session_from_disk(world, &key);
    // The pinned spill manifest is a persisted system message by design
    // (#951/#1046); this step is about the *user-supplied* system prompt.
    let system_count = session
        .messages
        .iter()
        .filter(|m| m.role == "system" && !m.is_manifest)
        .count();
    assert_eq!(
        system_count, 0,
        "expected no system messages in session '{}', found {}",
        key, system_count
    );
}

/// Assert that a session contains at least one message with role "tool".
#[then(expr = "the session {string} should contain a tool role message")]
fn then_session_has_tool_message(world: &mut QuectoWorld, key: String) {
    let session = load_session_from_disk(world, &key);
    let tool_count = session.messages.iter().filter(|m| m.role == "tool").count();
    assert!(
        tool_count > 0,
        "expected session '{}' to contain a tool role message, found none",
        key
    );
}

/// Assert that a session's message contents include the given text.
#[then(expr = "the session {string} should contain text {string}")]
fn then_session_contains_text(world: &mut QuectoWorld, key: String, text: String) {
    let session = load_session_from_disk(world, &key);
    let all_content: String = session
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_content.contains(&text),
        "expected session '{}' to contain '{}', but it was not found in: {}",
        key,
        text,
        all_content
    );
}

/// Create a pre-existing session that includes a tool call and tool result.
#[given(expr = "a pre-existing session {string} with tool call history for {string}")]
fn given_session_with_tool_history(world: &mut QuectoWorld, key: String, tool_name: String) {
    let base = base_path(world);
    let sessions_dir = base.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let session_file = serde_json::json!({
        "key": key,
        "messages": [
            { "role": "user", "content": "Use the tool" },
            {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": format!("call_{}", tool_name),
                    "name": tool_name,
                    "arguments": "{\"path\":\"test.txt\"}"
                }]
            },
            { "role": "tool", "content": "tool result data" },
            { "role": "assistant", "content": "Done with tool" }
        ]
    });
    let filename = key.replace(':', "_") + ".json";
    std::fs::write(
        sessions_dir.join(&filename),
        serde_json::to_string_pretty(&session_file).unwrap(),
    )
    .expect("write session with tool history");
}

/// Helper: load a session file from disk and parse it into a simple struct.
struct SessionOnDisk {
    messages: Vec<MessageOnDisk>,
}

struct MessageOnDisk {
    role: String,
    content: String,
    is_manifest: bool,
}

fn load_session_from_disk(world: &QuectoWorld, key: &str) -> SessionOnDisk {
    let base = base_path(world);
    let filename = key.replace(':', "_") + ".json";
    let path = base.join("sessions").join(&filename);
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read session '{}' at {}: {}",
            key,
            path.display(),
            e
        )
    });
    let messages = session_messages_from_json(&data)
        .unwrap_or_else(|e| panic!("failed to parse session '{}': {}", key, e));
    SessionOnDisk { messages }
}

fn session_messages_from_json(data: &str) -> Result<Vec<MessageOnDisk>, serde_json::Error> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
        return Ok(messages_from_session_value(&json));
    }
    if !data.trim_start().starts_with("{\"type\":") {
        return Ok(Vec::new());
    }

    let mut messages = Vec::new();
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: serde_json::Value = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record in BDD helper");
                break;
            }
            Err(err) => return Err(err),
        };
        parsed_any = true;
        match record["type"].as_str() {
            Some("snapshot") => messages = messages_from_session_value(&record),
            Some("append") => messages.extend(messages_from_session_value(&record)),
            _ => {}
        }
    }
    Ok(messages)
}

fn messages_from_session_value(json: &serde_json::Value) -> Vec<MessageOnDisk> {
    let container = json
        .get("messages")
        .and_then(|messages| messages.as_array())
        .or_else(|| {
            json.get("snapshot")
                .and_then(|snapshot| snapshot.get("messages"))
                .and_then(|messages| messages.as_array())
        });
    container
        .map(|messages| {
            messages
                .iter()
                .map(|m| MessageOnDisk {
                    role: m["role"].as_str().unwrap_or("").to_string(),
                    content: m["content"].as_str().unwrap_or("").to_string(),
                    is_manifest: m["is_manifest"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ===========================================================================
// E2E Agentic Loop Steps (parallel tool calls)
// ===========================================================================

/// Build an OpenAI-format JSON response with multiple parallel tool calls.
fn openai_parallel_tool_calls_json(calls: &[(String, String)]) -> serde_json::Value {
    let tool_calls: Vec<serde_json::Value> = calls
        .iter()
        .enumerate()
        .map(|(i, (name, args))| {
            serde_json::json!({
                "id": format!("call_{}_{}", name, i),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args
                }
            })
        })
        .collect();
    serde_json::json!({
        "id": "chatcmpl-parallel",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": tool_calls
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
}

#[given("the mock LLM returns parallel tool calls then text:")]
fn given_parallel_tool_calls(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    // Each row has pairs: tool_name, args_json, tool_name, args_json, ...
    let mut calls = Vec::new();
    for row in &table.rows {
        let mut i = 0;
        while i + 1 < row.len() {
            let name = row[i].clone();
            let args = row[i + 1].clone();
            if !name.is_empty() {
                calls.push((name, args));
            }
            i += 2;
        }
    }
    world.pending_parallel_calls = Some(calls);
}

#[given(expr = "the final text is {string}")]
fn given_final_text_for_parallel(world: &mut QuectoWorld, content: String) {
    let calls = world
        .pending_parallel_calls
        .take()
        .expect("no pending parallel calls");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        // First response: parallel tool calls (higher priority)
        let body = openai_parallel_tool_calls_json(&calls);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;

        // Second response: final text (lower priority)
        let text_body = openai_text_json(&content);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .with_priority(2)
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ===========================================================================
// E2E Safety and Limits Steps
// ===========================================================================

#[given("restrict_to_workspace is enabled in the config")]
fn given_restrict_to_workspace_enabled(world: &mut QuectoWorld) {
    let base = base_path(world);
    let config_str = std::fs::read_to_string(base.join("config.json")).expect("read config");
    let mut config: serde_json::Value = serde_json::from_str(&config_str).expect("parse config");
    config["agents"]["defaults"]["restrict_to_workspace"] = serde_json::Value::Bool(true);
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("rewrite config");
}

#[given(expr = "the config sets max_tool_iterations to {int}")]
fn given_config_max_tool_iterations(world: &mut QuectoWorld, max_iterations: u32) {
    let base = base_path(world);
    let config_str = std::fs::read_to_string(base.join("config.json")).expect("read config");
    let mut config: serde_json::Value = serde_json::from_str(&config_str).expect("parse config");
    config["agents"]["defaults"]["max_tool_iterations"] = serde_json::json!(max_iterations);
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("rewrite config");
}

#[given(expr = "the mock LLM always returns a tool call for {string} with args:")]
fn given_mock_llm_always_tool_call(
    world: &mut QuectoWorld,
    step: &gherkin::Step,
    tool_name: String,
) {
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let body = openai_tool_call_json(&tool_name, &args_json);
        // Mount with no limit — every request gets a tool call
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given(expr = "the mock LLM takes {int} seconds to respond")]
fn given_mock_llm_delayed_response(world: &mut QuectoWorld, delay_secs: u64) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = openai_text_json("Delayed response");
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(body)
                    .set_delay(std::time::Duration::from_secs(delay_secs)),
            )
            .mount(&server)
            .await;

        rewrite_config_to_uri(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- When steps for --max-iterations and --max-time ---

#[when(expr = "I run quecto agent -s {word} --max-iterations {int} -m {string}")]
fn when_run_agent_max_iterations(
    world: &mut QuectoWorld,
    session: String,
    max_iterations: u32,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--max-iterations".to_string(),
        max_iterations.to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto agent -s {word} --max-time {int} -m {string}")]
fn when_run_agent_max_time(
    world: &mut QuectoWorld,
    session: String,
    max_time: u64,
    message: String,
) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "-s".to_string(),
        session,
        "--max-time".to_string(),
        max_time.to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ===========================================================================
// E2E Provider Wiring Steps
// ===========================================================================

/// Helper: build the Anthropic Messages API response JSON for a text response.
fn anthropic_text_json(content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "text", "text": content }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })
}

/// Helper: write a config file with the given provider entries.
fn write_provider_config(
    world: &mut QuectoWorld,
    openai: serde_json::Value,
    anthropic: serde_json::Value,
) {
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config = serde_json::json!({
        "providers": {
            "openai": openai,
            "anthropic": anthropic,
        },
        "agents": {
            "defaults": {
                "workspace": workspace.display().to_string(),
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write config");
}

#[given("a config file with an Anthropic provider pointing at a mock server")]
fn given_config_with_anthropic_mock(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    // Store in anthropic-specific field, NOT _wiremock_server_uri (OpenAI)
    world.wiremock_anthropic_uri = Some(uri.clone());

    ensure_temp_dir(world);
    write_provider_config(
        world,
        serde_json::json!({"api_key": "", "api_base": ""}),
        serde_json::json!({"api_key": "sk-ant-test", "api_base": uri}),
    );

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given("a config file with both OpenAI and Anthropic providers pointing at mock servers")]
fn given_config_with_both_providers(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (openai_uri, anthropic_uri) = rt.block_on(async {
        let s1 = wiremock::MockServer::start().await;
        let s2 = wiremock::MockServer::start().await;
        let u1 = s1.uri();
        let u2 = s2.uri();
        std::mem::forget(s1);
        std::mem::forget(s2);
        (u1, u2)
    });

    ensure_temp_dir(world);
    world._wiremock_server_uri = Some(openai_uri.clone());
    world.wiremock_anthropic_uri = Some(anthropic_uri.clone());

    write_provider_config(
        world,
        serde_json::json!({"api_key": "sk-test-key", "api_base": openai_uri}),
        serde_json::json!({"api_key": "sk-ant-test", "api_base": anthropic_uri}),
    );

    std::mem::forget(rt);
}

/// Helper: rewrite config with a new OpenAI URI, preserving Anthropic if present.
fn rewrite_openai_in_config(world: &mut QuectoWorld, new_uri: &str) {
    let anthropic_uri = world.wiremock_anthropic_uri.as_deref().unwrap_or("");
    let anthropic_key = if anthropic_uri.is_empty() {
        ""
    } else {
        "sk-ant-test"
    };
    write_provider_config(
        world,
        serde_json::json!({"api_key": "sk-test-key", "api_base": new_uri}),
        serde_json::json!({"api_key": anthropic_key, "api_base": anthropic_uri}),
    );
    world._wiremock_server_uri = Some(new_uri.to_string());
}

/// Helper: rewrite config with a new Anthropic URI, preserving OpenAI if present.
fn rewrite_anthropic_in_config(world: &mut QuectoWorld, new_uri: &str) {
    let openai_uri = world._wiremock_server_uri.as_deref().unwrap_or("");
    let openai_key = if openai_uri.is_empty() {
        ""
    } else {
        "sk-test-key"
    };
    write_provider_config(
        world,
        serde_json::json!({"api_key": openai_key, "api_base": openai_uri}),
        serde_json::json!({"api_key": "sk-ant-test", "api_base": new_uri}),
    );
    world.wiremock_anthropic_uri = Some(new_uri.to_string());
}

#[given(expr = "the Anthropic mock returns an HTTP {int} error")]
fn given_anthropic_mock_error(world: &mut QuectoWorld, status: u16) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_string("Error"))
            .mount(&server)
            .await;

        rewrite_anthropic_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

#[given(expr = "the Anthropic mock returns a text response {string}")]
fn given_anthropic_mock_text_response(world: &mut QuectoWorld, content: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = anthropic_text_json(&content);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        rewrite_anthropic_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Credential store integration steps ---

fn test_openai_oauth_jwt(account_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{}"}}}}"#,
        account_id
    ));
    format!("{}.{}.sig", header, payload)
}

#[given(expr = "a config file with OpenAI api_key {string} pointing at a mock server")]
fn given_config_with_openai_custom_key(world: &mut QuectoWorld, api_key: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    world._wiremock_server_uri = Some(uri.clone());

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key, "api_base": uri }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.display().to_string()
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write config");

    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(
    expr = "a config file with OpenAI OAuth and an OpenAI-compatible provider {string} pointing at a mock server"
)]
fn given_config_with_openai_compatible_mock(world: &mut QuectoWorld, prefix: String) {
    let uri = "http://127.0.0.1:1".to_string();

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": "" },
            "openai_compatible": {
                "endpoints": [
                    { "prefix": prefix, "api_key": "sk-spark", "api_base": uri }
                ]
            }
        },
        "agents": { "defaults": { "workspace": workspace.display().to_string() } }
    });
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("write config");
}

#[given(
    expr = "a config file with OpenAI api_key {string}, disable_codex_routing true, and a mock server"
)]
fn given_config_with_openai_disable_codex(world: &mut QuectoWorld, api_key: String) {
    let uri = "http://127.0.0.1:1".to_string();

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config = serde_json::json!({
        "providers": {
            "openai": {
                "api_key": api_key,
                "api_base": uri,
                "auth_method": "api_key",
                "disable_codex_routing": true
            }
        },
        "agents": { "defaults": { "workspace": workspace.display().to_string() } }
    });
    std::fs::write(
        base.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("write config");
}

#[given(expr = "the credential store has a valid token {string} for provider {string}")]
fn given_credential_store_valid_token(world: &mut QuectoWorld, token: String, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: None, // no expiry = always valid
            refresh_token: None,
            account_id: None,
        })
        .expect("store credential");
}

#[given(
    expr = "the credential store has a valid OpenAI OAuth token with ChatGPT account id {string}"
)]
fn given_credential_store_valid_openai_oauth(world: &mut QuectoWorld, account_id: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: test_openai_oauth_jwt(&account_id),
            method: AuthMethod::OAuth,
            expires_at: Some(4_102_444_800),
            refresh_token: Some("refresh-token".to_string()),
            account_id: Some(account_id),
        })
        .expect("store credential");
}

#[given(expr = "the credential store has an expired token {string} for provider {string}")]
fn given_credential_store_expired_token(world: &mut QuectoWorld, token: String, provider: String) {
    let base = base_path(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider,
            token,
            method: AuthMethod::Token,
            expires_at: Some(0), // epoch = always expired
            refresh_token: None,
            account_id: None,
        })
        .expect("store credential");
}

#[given(expr = "the mock expects Authorization header {string} and returns {string}")]
fn given_mock_expects_auth_header(
    world: &mut QuectoWorld,
    expected_header: String,
    response_content: String,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = openai_text_json(&response_content);
        // Mount mock that ONLY matches the expected Authorization header.
        // If the wrong token is sent, wiremock returns 404, causing failure.
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .and(wiremock::matchers::header(
                "Authorization",
                expected_header.as_str(),
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        // Read existing config to preserve api_key, only replace api_base
        let base = base_path(world);
        let config_str =
            std::fs::read_to_string(base.join("config.json")).expect("read existing config");
        let mut config: serde_json::Value =
            serde_json::from_str(&config_str).expect("parse config");
        if config["providers"].get("openai").is_some() {
            config["providers"]["openai"]["api_base"] = serde_json::Value::String(new_uri.clone());
        }
        if let Some(endpoints) = config
            .get_mut("providers")
            .and_then(|providers| providers.get_mut("openai_compatible"))
            .and_then(|openai_compatible| openai_compatible.get_mut("endpoints"))
            .and_then(|endpoints| endpoints.as_array_mut())
        {
            for endpoint in endpoints {
                endpoint["api_base"] = serde_json::Value::String(new_uri.clone());
            }
        }
        std::fs::write(
            base.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .expect("rewrite config");
        world._wiremock_server_uri = Some(new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// --- Auth error steps ---

#[given(expr = "the OpenAI mock returns an HTTP {int} error")]
fn given_openai_mock_http_error(world: &mut QuectoWorld, status: u16) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_string("Error"))
            .mount(&server)
            .await;

        rewrite_openai_in_config(world, &new_uri);
        std::mem::forget(server);
    });
    std::mem::forget(rt);
}

// ===========================================================================
// E2E Subprocess Protocol Steps
// ===========================================================================

/// Resolve the `quecto` binary the e2e tests spawn.
///
/// Prefer `CARGO_BIN_EXE_quecto`: Cargo sets it at test-compile time and
/// guarantees the `quecto` binary is (re)built before this integration test
/// runs, so the suite always exercises the *current source* — never a stale
/// `target/` artifact and never an installed `~/.cargo/bin/quecto`.
///
/// Fall back to the path relative to the test executable
/// (`target/debug/deps/bdd-*` → `target/debug/quecto`) for the rare case of
/// running the prebuilt test binary directly, outside `cargo test`.
fn quecto_binary_path() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_quecto") {
        return PathBuf::from(p);
    }
    let test_exe = std::env::current_exe().expect("get current exe");
    let deps_dir = test_exe.parent().expect("deps dir");
    let debug_dir = deps_dir.parent().expect("debug dir");
    debug_dir.join("quecto")
}

/// Maximum wall-clock time (seconds) a subprocess may run before
/// the BDD test kills it. Prevents the suite from hanging forever.
const SUBPROCESS_TIMEOUT_SECS: u64 = 30;

/// Spawn quecto as a real subprocess, capturing output.
/// Sets QUECTO_BASE_DIR to the temp dir if cli_context has one,
/// otherwise inherits from the environment (for env-var tests).
/// Kills the child after [`SUBPROCESS_TIMEOUT_SECS`] if it has
/// not exited.
fn spawn_quecto_subprocess(world: &mut QuectoWorld, raw_args: &str) {
    spawn_quecto_subprocess_with_stdin(world, raw_args, None);
}

/// Spawn quecto as a real subprocess, optionally writing stdin.
fn spawn_quecto_subprocess_with_stdin(
    world: &mut QuectoWorld,
    raw_args: &str,
    stdin_data: Option<&str>,
) {
    let args = shell_split(raw_args);
    let mut auto_messages = Vec::new();
    if let Some(idx) = args
        .iter()
        .rposition(|arg| arg == "-m" || arg == "--message")
        && let Some(message) = args.get(idx + 1)
    {
        let system = args
            .iter()
            .position(|arg| arg == "--system")
            .and_then(|idx| args.get(idx + 1))
            .cloned()
            .unwrap_or_default();
        auto_messages.push(format!("{system} {message}"));
    }
    if let Some(stdin) = stdin_data {
        auto_messages.extend(
            stdin
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('/'))
                .map(ToString::to_string),
        );
    }
    mount_auto_mock_responses_for_messages(world, &auto_messages);

    let binary = quecto_binary_path();
    assert!(
        binary.exists(),
        "quecto binary not found at {}",
        binary.display()
    );
    let mut cmd = std::process::Command::new(&binary);
    cmd.args(&args);
    if stdin_data.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // If cli_context.base_dir is set, pass it explicitly.
    // Otherwise the env var is already set (e.g. by the
    // "I set QUECTO_BASE_DIR" step) and the child inherits it.
    if let Some(ref base) = world.cli_context.base_dir {
        cmd.env("QUECTO_BASE_DIR", base.to_string_lossy().as_ref());
    }

    let mut child = cmd.spawn().expect("spawn quecto subprocess");
    if let Some(data) = stdin_data
        && let Some(mut stdin) = child.stdin.take()
    {
        use std::io::Write as _;
        stdin
            .write_all(data.as_bytes())
            .expect("write stdin to subprocess");
    }
    let start = std::time::Instant::now();
    let deadline = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let poll_interval = std::time::Duration::from_millis(50);

    // Poll until the child exits or the deadline is reached.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().expect("collect subprocess output");
                world.subprocess_exit_code = Some(status.code().unwrap_or(-1));
                world.subprocess_stdout = Some(String::from_utf8_lossy(&out.stdout).into_owned());
                world.subprocess_stderr = Some(String::from_utf8_lossy(&out.stderr).into_owned());
                return;
            }
            Ok(None) => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "subprocess timed out after {}s \
                         (args: {})",
                        SUBPROCESS_TIMEOUT_SECS, raw_args
                    );
                }
                std::thread::sleep(poll_interval);
            }
            Err(e) => {
                panic!("failed to wait on subprocess: {e}");
            }
        }
    }
}

#[when(regex = r"^I spawn quecto as a subprocess with args: (.+)$")]
fn when_spawn_subprocess(world: &mut QuectoWorld, raw_args: String) {
    spawn_quecto_subprocess(world, &raw_args);
}

#[when("I spawn quecto as a subprocess with no args and stdin:")]
fn when_spawn_subprocess_no_args_stdin(world: &mut QuectoWorld, step: &gherkin::Step) {
    let stdin_data = step.docstring.as_ref().expect("missing docstring").trim();
    let stdin_with_newline = format!("{}\n", stdin_data);
    spawn_quecto_subprocess_with_stdin(world, "", Some(&stdin_with_newline));
    world.exit_code = world.subprocess_exit_code.unwrap_or(-1);
    world.stdout = world.subprocess_stdout.clone().unwrap_or_default();
    world.stderr = world.subprocess_stderr.clone().unwrap_or_default();
}

// "And the mock LLM returns a text response" after a When step
// is interpreted as a When step by cucumber-rs.
#[when(expr = "the mock LLM returns a text response {string}")]
fn when_mock_llm_text_response(world: &mut QuectoWorld, content: String) {
    given_mock_llm_text_response(world, content);
}

#[then(expr = "the subprocess exit code should be {int}")]
fn then_subprocess_exit_code(world: &mut QuectoWorld, expected: i32) {
    let actual = world
        .subprocess_exit_code
        .expect("no subprocess was spawned");
    assert_eq!(
        actual,
        expected,
        "expected subprocess exit code {}, got {}.\nstdout: {}\nstderr: {}",
        expected,
        actual,
        world.subprocess_stdout.as_deref().unwrap_or(""),
        world.subprocess_stderr.as_deref().unwrap_or("")
    );
}

#[then(expr = "the subprocess stdout should contain {string}")]
fn then_subprocess_stdout_contains(world: &mut QuectoWorld, expected: String) {
    let stdout = world
        .subprocess_stdout
        .as_ref()
        .expect("no subprocess was spawned");
    assert!(
        stdout.contains(&expected),
        "expected subprocess stdout to contain '{}', got: {}",
        expected,
        stdout
    );
}

#[then(expr = "the subprocess stderr should contain {string}")]
fn then_subprocess_stderr_contains(world: &mut QuectoWorld, expected: String) {
    let stderr = world
        .subprocess_stderr
        .as_ref()
        .expect("no subprocess was spawned");
    assert!(
        stderr.contains(&expected),
        "expected subprocess stderr to contain '{}', got: {}",
        expected,
        stderr
    );
}

// ===========================================================================
// E2E Real LLM Steps
// ===========================================================================

/// Resolve provider API keys: check env vars first, then fall back to `.env`.
fn resolve_api_key_from_env_or_dotenv(vars: &[&str]) -> Option<String> {
    for var in vars {
        if let Ok(key) = std::env::var(var) {
            if !key.trim().is_empty() {
                return Some(key);
            }
        }
    }
    if let Ok(contents) = std::fs::read_to_string(".env") {
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            for var in vars {
                if let Some(value) = line.strip_prefix(&format!("{var}=")) {
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    None
}

fn resolve_openai_api_key() -> String {
    resolve_api_key_from_env_or_dotenv(&["OPENAI_API_KEY"])
        .unwrap_or_else(|| panic!("OPENAI_API_KEY must be set (via env var or .env file)"))
}

fn resolve_anthropic_api_key() -> String {
    resolve_api_key_from_env_or_dotenv(&["ANTHROPIC_API_KEY"])
        .unwrap_or_else(|| panic!("ANTHROPIC_API_KEY must be set (via env var or .env file)"))
}

fn default_quecto_base_dir_for_smoke() -> PathBuf {
    std::env::var("QUECTO_BASE_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".quecto")))
        .unwrap_or_else(|| PathBuf::from(".quecto"))
}

fn copy_openai_oauth_credential_to_smoke_base(base: &Path) {
    let source_store = CredentialStore::new(default_quecto_base_dir_for_smoke());
    let Some(credential) = source_store.get("openai").ok().flatten() else {
        panic!("OpenAI OAuth credential is required for Codex provider smoke");
    };
    assert_eq!(
        credential.method,
        AuthMethod::OAuth,
        "OpenAI OAuth credential is required for Codex provider smoke"
    );
    CredentialStore::new(base)
        .store(credential)
        .expect("copy OpenAI OAuth credential to smoke base dir");
}

/// Set up a workspace configured to use a real OpenAI endpoint.
/// Reads OPENAI_API_KEY from the environment or from `.env` file at repo root.
/// Uses serde_json to avoid JSON injection from special chars in the key.
#[given("a real LLM workspace is configured")]
fn given_real_llm_workspace(world: &mut QuectoWorld) {
    if std::env::var("QUECTO_REAL_LLM").unwrap_or_default() != "1" {
        configure_mock_provider_workspace(world, "openai");
        if let Some(uri) = world._wiremock_server_uri.clone() {
            rewrite_config_to_provider_uri(world, "anthropic", &uri);
        }
        world.auto_mock_manual_llm = true;
        return;
    }

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key = resolve_openai_api_key();

    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy()
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write real LLM config");
}

/// Set up a real-LLM workspace with web fetch enabled.
#[given("a real LLM workspace is configured with web fetch enabled")]
fn given_real_llm_workspace_web_fetch(world: &mut QuectoWorld) {
    if std::env::var("QUECTO_REAL_LLM").unwrap_or_default() != "1" {
        configure_mock_provider_workspace(world, "openai");
        world.auto_mock_manual_llm = true;
        return;
    }

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key = resolve_openai_api_key();

    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy()
            }
        },
        "tools": {
            "web": {
                "fetch": { "enabled": true }
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write real LLM config");
}

/// Set up a real-LLM workspace with workflow enabled.
#[given("a real LLM workspace is configured with workflow enabled")]
fn given_real_llm_workspace_workflow(world: &mut QuectoWorld) {
    if std::env::var("QUECTO_REAL_LLM").unwrap_or_default() != "1" {
        configure_mock_provider_workspace(world, "openai");
        world._workflow_enabled = true;
        world.auto_mock_manual_llm = true;
        return;
    }

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key = resolve_openai_api_key();

    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy()
            }
        },
        "workflow": {
            "enabled": true
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write real LLM config");
}

#[given("an OpenAI provider smoke workspace is configured")]
fn given_openai_provider_smoke_workspace(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key = resolve_openai_api_key();
    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy(),
                "max_tokens": 4,
                "temperature": 0.0
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write smoke config");
}

#[given("an Anthropic provider smoke workspace is configured")]
fn given_anthropic_provider_smoke_workspace(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let api_key = resolve_anthropic_api_key();
    let config = serde_json::json!({
        "providers": {
            "anthropic": { "api_key": api_key }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy(),
                "max_tokens": 4,
                "temperature": 0.0
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write smoke config");
}

#[given("a Codex provider smoke workspace is configured")]
fn given_codex_provider_smoke_workspace(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    copy_openai_oauth_credential_to_smoke_base(&base);

    let config = serde_json::json!({
        "agents": {
            "defaults": {
                "workspace": workspace.to_string_lossy(),
                "max_tokens": 4,
                "temperature": 0.0
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write smoke config");
}

/// Run the agent against the real OpenAI endpoint with a cheap model, bounded iterations,
/// and a wall-clock timeout to prevent hung HTTP requests from blocking the suite.
#[when(expr = "I run the real LLM agent with message {string}")]
fn when_run_real_llm_agent(world: &mut QuectoWorld, message: String) {
    mount_auto_mock_responses_for_messages(world, std::slice::from_ref(&message));
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "gpt-5.2".to_string(),
        "--max-iterations".to_string(),
        "5".to_string(),
        "--max-time".to_string(),
        "60".to_string(),
        "-s".to_string(),
        "-".to_string(), // ephemeral session
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

/// Run the real LLM agent with a named session (for persistence tests).
#[when(expr = "I run the real LLM agent with session {word} and message {string}")]
fn when_run_real_llm_agent_session(world: &mut QuectoWorld, session: String, message: String) {
    mount_auto_mock_responses_for_messages(world, std::slice::from_ref(&message));
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "gpt-5.2".to_string(),
        "--max-iterations".to_string(),
        "5".to_string(),
        "--max-time".to_string(),
        "60".to_string(),
        "-s".to_string(),
        session,
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

/// Run the real LLM agent with a system prompt.
#[when(expr = "I run the real LLM agent with system {string} and message {string}")]
fn when_run_real_llm_agent_system(world: &mut QuectoWorld, system: String, message: String) {
    mount_auto_mock_responses_for_messages(world, &[format!("{system} {message}")]);
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "gpt-5.2".to_string(),
        "--max-iterations".to_string(),
        "5".to_string(),
        "--max-time".to_string(),
        "60".to_string(),
        "--system".to_string(),
        system,
        "-s".to_string(),
        "-".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}
#[when(expr = "I run the OpenAI provider smoke agent with message {string}")]
fn when_run_openai_provider_smoke_agent(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "openai-api/gpt-4o-mini".to_string(),
        "--max-iterations".to_string(),
        "1".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
        "-s".to_string(),
        "-".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run the Anthropic provider smoke agent with message {string}")]
fn when_run_anthropic_provider_smoke_agent(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "anthropic-api/claude-sonnet-4-5".to_string(),
        "--max-iterations".to_string(),
        "1".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
        "-s".to_string(),
        "-".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run the Codex provider smoke agent with message {string}")]
fn when_run_codex_provider_smoke_agent(world: &mut QuectoWorld, message: String) {
    let args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--model".to_string(),
        "openai-oauth/gpt-5.3-codex".to_string(),
        "--max-iterations".to_string(),
        "1".to_string(),
        "--max-time".to_string(),
        "30".to_string(),
        "--system".to_string(),
        "You are concise.".to_string(),
        "-s".to_string(),
        "-".to_string(),
        "-m".to_string(),
        message,
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

/// Start a wiremock server that captures requests and returns a text response.
/// Stores a leaked reference so we can inspect received requests later.
#[given(expr = "a mock LLM that captures requests and returns text {string}")]
fn given_mock_llm_captures_and_returns(world: &mut QuectoWorld, content: String) {
    ensure_temp_dir(world);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        let body = openai_text_json(&content);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        rewrite_config_to_uri(world, &new_uri);
        // Leak the server and keep a static ref for request inspection.
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        world.wiremock_server_ref = Some(leaked);
    });
    std::mem::forget(rt);
}

/// Assert that the LLM received at least one request containing a system
/// message with the given substring.
#[then(expr = "the LLM should have received a system message containing {string}")]
fn then_llm_received_system_message(world: &mut QuectoWorld, expected: String) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    assert!(
        !requests.is_empty(),
        "expected at least one request to the LLM"
    );
    let found = requests.iter().any(|req| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        body["messages"]
            .as_array()
            .map(|msgs| {
                msgs.iter().any(|m| {
                    m["role"] == "system"
                        && m["content"]
                            .as_str()
                            .map(|c| c.contains(&expected))
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected a system message containing '{}' in LLM requests",
        expected
    );
}

/// Assert that the LLM did not receive any request containing a system
/// message with the given substring.
#[then(expr = "the LLM should not have received a system message containing {string}")]
fn then_llm_did_not_receive_system_message(world: &mut QuectoWorld, unexpected: String) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    assert!(
        !requests.is_empty(),
        "expected at least one request to the LLM"
    );
    let found = requests.iter().any(|req| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        body["messages"]
            .as_array()
            .map(|msgs| {
                msgs.iter().any(|m| {
                    m["role"] == "system"
                        && m["content"]
                            .as_str()
                            .map(|c| c.contains(&unexpected))
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });
    assert!(
        !found,
        "expected no system message containing '{}' in LLM requests",
        unexpected
    );
}

/// Assert that the LLM did NOT receive any system message.
#[then("the LLM should not have received a system message")]
fn then_llm_no_system_message(world: &mut QuectoWorld) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    let has_system = requests.iter().any(|req| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        body["messages"]
            .as_array()
            .map(|msgs| msgs.iter().any(|m| m["role"] == "system"))
            .unwrap_or(false)
    });
    assert!(
        !has_system,
        "expected no system messages in LLM requests, but found one"
    );
}

#[then("the LLM system message should only contain the datetime preamble")]
fn then_llm_system_message_only_preamble(world: &mut QuectoWorld) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    let system_contents: Vec<String> = requests
        .iter()
        .filter_map(|req| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            body["messages"].as_array().and_then(|msgs| {
                msgs.iter()
                    .find(|m| m["role"] == "system")
                    .and_then(|m| m["content"].as_str().map(String::from))
            })
        })
        .collect();
    assert!(
        !system_contents.is_empty(),
        "expected a system message with datetime preamble, but none found"
    );
    for content in &system_contents {
        assert!(
            content.starts_with("Current date and time: "),
            "system message should start with datetime preamble, got: {}",
            &content[..content.len().min(100)]
        );
        // Beyond the datetime line, the only standard content is the Quecto
        // capability-docs retrieval preamble (always appended by
        // build_system_prompt). User prompts should not leak into the
        // system message.
        let extra = content.lines().skip(1).collect::<Vec<_>>().join("\n");
        let extra = extra.trim();
        assert!(
            extra.is_empty() || extra.contains("Quecto's own capability docs are embedded"),
            "system message should contain only the standard preamble, got: {:?}",
            content.lines().collect::<Vec<_>>()
        );
    }
}

#[then("the LLM system message datetime preamble should include day-of-week, time, and timezone")]
fn then_llm_system_message_preamble_rich_format(world: &mut QuectoWorld) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    let system_contents: Vec<String> = requests
        .iter()
        .filter_map(|req| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            body["messages"].as_array().and_then(|msgs| {
                msgs.iter()
                    .find(|m| m["role"] == "system")
                    .and_then(|m| m["content"].as_str().map(String::from))
            })
        })
        .collect();
    assert!(
        !system_contents.is_empty(),
        "expected a system message with datetime preamble, but none found"
    );
    let preamble = &system_contents[0];
    assert!(
        preamble.starts_with("Current date and time: "),
        "preamble should start with 'Current date and time: ', got: {}",
        preamble
    );
    // The quecto preamble is intentionally richer than provider-injected dates.
    // It includes day-of-week (e.g. "Saturday"), full time with seconds,
    // and timezone — critical for cron scheduling and time-aware tasks.
    // Duplication with provider-side "Current date:" metadata is expected
    // and documented (issue #104).
    let days = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    let has_day = days.iter().any(|d| preamble.contains(d));
    assert!(
        has_day,
        "preamble should include day-of-week (e.g. 'Saturday'), got: {}",
        preamble
    );

    // Should contain a time component with AM/PM (e.g. "06:55:58 PM")
    let time_pattern = regex::Regex::new(r"\d{1,2}:\d{2}:\d{2}\s+(AM|PM)").unwrap();
    assert!(
        time_pattern.is_match(preamble),
        "preamble should include time with seconds and AM/PM, got: {}",
        preamble
    );

    // Should contain a timezone identifier (e.g. "GMT", "UTC", "+00:00", "EST")
    // The format uses %Z which produces timezone abbreviations.
    let after_ampm = preamble
        .find("AM")
        .or_else(|| preamble.find("PM"))
        .map(|pos| &preamble[pos..])
        .unwrap_or("");
    assert!(
        after_ampm.len() > 3,
        "preamble should include timezone after AM/PM, got: {}",
        preamble
    );
}

// ===========================================================================
// --config flag steps (Issue #300)
// ===========================================================================

#[given("a config file at a custom path with an OpenAI provider pointing at a mock server")]
fn given_config_at_custom_path(world: &mut QuectoWorld) {
    // Start a wiremock server and leak it so it stays alive for the scenario.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    ensure_temp_dir(world);
    let base = base_path(world);

    // Write config to a non-standard path (not base_dir/config.json)
    let custom_path = base.join("custom-config.json");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace dir");

    let config_json = serde_json::json!({
        "providers": {
            "openai": {
                "api_key": "sk-test-key",
                "api_base": uri
            }
        },
        "agents": {
            "defaults": {
                "workspace": workspace.display().to_string()
            }
        }
    });
    std::fs::write(
        &custom_path,
        serde_json::to_string_pretty(&config_json).unwrap(),
    )
    .expect("write custom config");

    world.custom_config_path = Some(custom_path.to_string_lossy().to_string());
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[when(regex = r"^I run quecto agent --config (.+)$")]
fn when_run_agent_with_config_flag(world: &mut QuectoWorld, rest: String) {
    // Replace <custom-config-path> placeholder with the actual path, if set.
    let resolved = if rest.starts_with("<custom-config-path>") {
        let config_path = world
            .custom_config_path
            .as_ref()
            .expect("custom config path not set");
        rest.replacen("<custom-config-path>", config_path, 1)
    } else {
        rest.clone()
    };
    let mut args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--config".to_string(),
    ];
    args.extend(shell_split(&resolved));
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

// ===========================================================================
// Extension wiring assertions (#318 Part 2)
// ===========================================================================

/// Assert that at least one LLM request included a tool definition with the given name.
#[then(expr = "the LLM request should have included tool {string}")]
fn then_llm_request_included_tool(world: &mut QuectoWorld, tool_name: String) {
    let server = world
        .wiremock_server_ref
        .expect("no capturing mock LLM configured — use 'a mock LLM that captures requests'");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let requests = rt.block_on(async { server.received_requests().await });
    std::mem::forget(rt);
    let requests = requests.expect("request recording not enabled");
    assert!(
        !requests.is_empty(),
        "expected at least one request to the LLM"
    );
    let found = requests.iter().any(|req| {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        body["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .any(|t| t["function"]["name"].as_str() == Some(tool_name.as_str()))
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected LLM request to include tool '{}', but it was not found in tool definitions",
        tool_name
    );
}

// ===========================================================================
// E2E Real LLM UDS Steps
// ===========================================================================
//
// These steps exercise the UDS agent with real Anthropic/OpenAI OAuth
// credentials.  They use real socket bind (production path) and wait for
// each prompt to complete before sending the next.

/// Set up a workspace for real-LLM UDS tests.
///
/// Prefers a static Anthropic API key (env `ANTHROPIC_API_KEY`) so these tests run in CI without a local
/// `quecto auth login`. Falls back to copying `~/.quecto/credentials.json` and
/// `auth_method: "oauth"` for subscription-based local runs. The default model
/// is `anthropic/claude-haiku-4-5` for fast, cheap tests.
#[given("a real LLM UDS workspace is configured")]
fn given_real_llm_uds_workspace(world: &mut QuectoWorld) {
    if std::env::var("QUECTO_REAL_LLM").unwrap_or_default() != "1" {
        configure_mock_provider_workspace(world, "openai");
        world.auto_mock_manual_llm = true;
        return;
    }

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let has_api_key = std::env::var("ANTHROPIC_API_KEY").is_ok();

    // API-key auth (CI-friendly): the key is supplied via env override at load
    // time, so no credentials.json is needed in the isolated base dir.
    // OAuth auth (local subscription): copy the home credential store.
    let anthropic = if has_api_key {
        serde_json::json!({ "api_key": "", "api_base": "" })
    } else {
        let home_creds = dirs::home_dir()
            .expect("no home dir")
            .join(".quecto")
            .join("credentials.json");
        if !home_creds.exists() {
            panic!(
                "set ANTHROPIC_API_KEY, or run \
                 'quecto auth login' (credentials.json not found)"
            );
        }
        let dest = base.join("credentials.json");
        std::fs::copy(&home_creds, &dest).expect("copy credentials.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600))
                .expect("set credentials permissions");
        }
        serde_json::json!({ "api_key": "", "api_base": "", "auth_method": "oauth" })
    };

    let config = serde_json::json!({
        "providers": { "anthropic": anthropic },
        "agents": {
            "defaults": {
                "model": "anthropic/claude-haiku-4-5",
                "workspace": workspace.to_string_lossy()
            }
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write real LLM UDS config");
}

/// Start the real LLM UDS agent in multi-client mode (real socket bind).
#[when("I start the real LLM UDS agent")]
fn when_start_real_llm_uds(world: &mut QuectoWorld) {
    world.no_session = true;
    world._workflow_enabled = false;
    if world.auto_mock_manual_llm {
        world._uds_streaming_enabled = true;
        world._real_llm_uds = false;
    } else {
        world._uds_streaming_enabled = true;
        world._real_llm_uds = true;
    }
}

// Real-LLM UDS execution is handled by `execute_real_llm_uds` in `uds_steps.rs`.
// The flag `world._real_llm_uds` is set by "I start the real LLM UDS agent" and
// detected by `execute_uds` which delegates to the real-LLM executor.

// ─── Real-LLM UDS Then steps ────────────────────────────────────────────────

/// Assert that a completed agent response contains a specific string. Completion
/// payloads are bounded, so response text is reconstructed from token events.
#[then(expr = "the agent_end messages should contain {string}")]
fn then_agent_end_contains(world: &mut QuectoWorld, expected: String) {
    let mut completed = false;
    let mut response = String::new();
    for line in &world.agent_events {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
            match event["type"].as_str() {
                Some("token") => response.push_str(event["token"].as_str().unwrap_or_default()),
                Some("agent_end") => completed = true,
                _ => {}
            }
        }
    }
    assert!(
        completed && response.contains(&expected),
        "expected completed agent response to contain {expected:?}; response={response:?}\nevents: {:#?}",
        world.agent_events,
    );
}

/// Assert that an agent_error event was emitted.
#[then("the agent output should contain an agent_error event")]
fn then_agent_output_has_error_event(world: &mut QuectoWorld) {
    let found = world.agent_events.iter().any(|l| {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
            let t = v["type"].as_str().unwrap_or("");
            let cmd = v["command"].as_str().unwrap_or("");
            t == "response" && cmd == "agent_error"
        } else {
            false
        }
    });
    assert!(
        found,
        "expected an agent_error event in output\nevents: {:#?}",
        world.agent_events,
    );
}

/// Assert that the agent_error event mentions a specific string.
#[then(expr = "the agent_error event should mention {string}")]
fn then_agent_error_mentions(world: &mut QuectoWorld, expected: String) {
    let found = world.agent_events.iter().any(|l| {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
            let t = v["type"].as_str().unwrap_or("");
            let cmd = v["command"].as_str().unwrap_or("");
            if t == "response" && cmd == "agent_error" {
                let err = v["error"].as_str().unwrap_or("");
                return err.contains(&expected);
            }
        }
        false
    });
    assert!(
        found,
        "expected agent_error event mentioning {expected:?}\nevents: {:#?}",
        world.agent_events,
    );
}

// ─── Real-LLM get_state assertions ──────────────────────────────────────────

/// Assert that the get_state response messageCount is at least N.
#[then(expr = "the get_state response messageCount should be at least {int}")]
fn then_get_state_message_count_at_least(world: &mut QuectoWorld, minimum: usize) {
    let resp = uds_steps::find_agent_response(world, "get_state").expect("no get_state response");
    let count = resp["data"]["messageCount"]
        .as_u64()
        .expect("messageCount not a number") as usize;
    assert!(
        count >= minimum,
        "expected messageCount >= {minimum}, got {count}"
    );
}

// ─── Real-LLM get_messages assertions ───────────────────────────────────────

/// Assert that the get_messages response contains a user message with specific text.
#[then(expr = "the get_messages response should include a user message containing {string}")]
fn then_get_messages_has_user_message(world: &mut QuectoWorld, expected: String) {
    let resp =
        uds_steps::find_agent_response(world, "get_messages").expect("no get_messages response");
    let msgs = resp["data"]["messages"]
        .as_array()
        .expect("messages not an array");
    let found = msgs.iter().any(|m| {
        m["role"].as_str() == Some("user")
            && m["content"]
                .as_str()
                .map(|c| c.contains(&expected))
                .unwrap_or(false)
    });
    assert!(
        found,
        "expected a user message containing {expected:?}\nmessages: {msgs:?}"
    );
}

/// Assert that the get_messages response contains at least one assistant message.
#[then("the get_messages response should include an assistant message")]
fn then_get_messages_has_assistant(world: &mut QuectoWorld) {
    let resp =
        uds_steps::find_agent_response(world, "get_messages").expect("no get_messages response");
    let msgs = resp["data"]["messages"]
        .as_array()
        .expect("messages not an array");
    let found = msgs.iter().any(|m| m["role"].as_str() == Some("assistant"));
    assert!(
        found,
        "expected at least one assistant message\nmessages: {msgs:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Real-LLM Workflow V2 UDS steps (#568–#577)
// ═══════════════════════════════════════════════════════════════════════════

#[given("a real LLM UDS workspace is configured with workflow enabled")]
fn given_real_llm_uds_workflow_workspace(world: &mut QuectoWorld) {
    if std::env::var("QUECTO_REAL_LLM").unwrap_or_default() != "1" {
        configure_mock_provider_workspace(world, "openai");
        world._workflow_enabled = true;
        world.auto_mock_manual_llm = true;
        return;
    }

    ensure_temp_dir(world);
    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let home_creds = dirs::home_dir()
        .expect("no home dir")
        .join(".quecto")
        .join("credentials.json");
    if !home_creds.exists() {
        panic!("~/.quecto/credentials.json not found — run 'quecto auth login' first");
    }
    let dest = base.join("credentials.json");
    std::fs::copy(&home_creds, &dest).expect("copy credentials.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600))
            .expect("set credentials permissions");
    }

    let config = serde_json::json!({
        "providers": {
            "openai": {
                "api_key": ""
            }
        },
        "agents": {
            "defaults": {
                "model": "openai/gpt-5.4",
                "workspace": workspace.to_string_lossy()
            }
        },
        "workflow": {
            "auto_continue": false,
            "completion_nudge": false
        }
    });
    let config_json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(base.join("config.json"), config_json).expect("write workflow config");

    world._workflow_enabled = true;
}

#[when("I start the real LLM UDS workflow agent")]
fn when_start_real_llm_uds_workflow(world: &mut QuectoWorld) {
    world.no_session = true;
    if world.auto_mock_manual_llm {
        world._uds_streaming_enabled = true;
        world._real_llm_uds = false;
    } else {
        world._uds_streaming_enabled = true;
        world._real_llm_uds = true;
    }
    world._workflow_enabled = true;
    world.system_prompt = Some(
        "You are a coding assistant. Use the workflow tool when asked. \
         Follow instructions precisely and reply with the exact marker words requested."
            .to_string(),
    );
}

// ─── Workflow Then steps ────────────────────────────────────────────────────

#[then(expr = "the get_state response {string} should have workflow mode {string}")]
fn then_get_state_workflow_mode(world: &mut QuectoWorld, id: String, expected_mode: String) {
    let resp = find_response_by_id(world, &id).unwrap_or_else(|| {
        panic!(
            "no get_state response with id {id:?}\nevents: {:#?}",
            world.agent_events
        )
    });
    let mode = resp["data"]["workflow"]["mode"]
        .as_str()
        .unwrap_or("(missing)");
    assert_eq!(
        mode, expected_mode,
        "expected workflow mode {expected_mode:?}, got {mode:?}"
    );
}

#[then(expr = "the get_state response {string} should have {int} available templates")]
fn then_get_state_template_count(world: &mut QuectoWorld, id: String, count: usize) {
    let resp = find_response_by_id(world, &id).unwrap_or_else(|| {
        panic!(
            "no get_state response with id {id:?}\nevents: {:#?}",
            world.agent_events
        )
    });
    let templates = resp["data"]["workflow"]["available_templates"]
        .as_array()
        .expect("available_templates not an array");
    assert_eq!(
        templates.len(),
        count,
        "expected {count} templates, got {}",
        templates.len()
    );
}

#[then(expr = "the get_state response {string} should have workflow template {string}")]
fn then_get_state_workflow_template(world: &mut QuectoWorld, id: String, expected: String) {
    let resp = find_response_by_id(world, &id)
        .unwrap_or_else(|| panic!("no get_state response with id {id:?}"));
    let tpl_id = resp["data"]["workflow"]["active_template"]["id"]
        .as_str()
        .unwrap_or("(missing)");
    assert_eq!(tpl_id, expected);
}

#[then(expr = "the get_state response {string} should have workflow progress done {int}")]
fn then_get_state_workflow_progress(world: &mut QuectoWorld, id: String, expected: u64) {
    let resp = find_response_by_id(world, &id)
        .unwrap_or_else(|| panic!("no get_state response with id {id:?}"));
    let done = resp["data"]["workflow"]["progress"]["done"]
        .as_u64()
        .unwrap_or(0);
    assert_eq!(
        done, expected,
        "expected progress.done={expected}, got {done}"
    );
}

#[then(expr = "the get_state response {string} should not have workflow")]
fn then_get_state_no_workflow(world: &mut QuectoWorld, id: String) {
    let resp = find_response_by_id(world, &id)
        .unwrap_or_else(|| panic!("no get_state response with id {id:?}"));
    assert!(
        resp["data"]["workflow"].is_null(),
        "expected no workflow in get_state, got: {}",
        resp["data"]["workflow"]
    );
}

#[then(expr = "the agent output should contain a workflow_state event with mode {string}")]
fn then_workflow_event_mode(world: &mut QuectoWorld, expected: String) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .map(|v| {
                v["type"].as_str() == Some("workflow_state")
                    && v["mode"].as_str() == Some(&expected)
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected workflow_state event with mode={expected:?}\nevents: {:#?}",
        world.agent_events
    );
}

#[then(expr = "the agent output should contain a workflow_state event with template {string}")]
fn then_workflow_event_template(world: &mut QuectoWorld, expected: String) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .map(|v| {
                v["type"].as_str() == Some("workflow_state")
                    && v["activeTemplate"]["id"].as_str() == Some(&expected)
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected workflow_state event with template={expected:?}\nevents: {:#?}",
        world.agent_events
    );
}

#[then(expr = "the agent output should contain a workflow_state event with progress done {int}")]
fn then_workflow_event_progress(world: &mut QuectoWorld, expected: u64) {
    let found = world.agent_events.iter().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .map(|v| {
                v["type"].as_str() == Some("workflow_state")
                    && v["progress"]["done"].as_u64() == Some(expected)
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected workflow_state event with progress.done={expected}\nevents: {:#?}",
        world.agent_events
    );
}

fn find_response_by_id(world: &QuectoWorld, id: &str) -> Option<serde_json::Value> {
    world.agent_events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v["type"].as_str() == Some("response") && v["id"].as_str() == Some(id) {
            Some(v)
        } else {
            None
        }
    })
}
