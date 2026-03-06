use super::*;

// ===========================================================================
// Config Steps (Given)
// ===========================================================================

#[given(expr = "a config file at {string} with content:")]
fn given_config_file_at_path(world: &mut QuectoWorld, step: &gherkin::Step, _path: String) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

#[given(expr = "an environment variable {string} set to {string}")]
fn given_env_var(world: &mut QuectoWorld, key: String, value: String) {
    world.env_overrides.insert(key, value);
}

#[given(expr = "a config file with model {string}")]
fn given_config_file_with_model(world: &mut QuectoWorld, model: String) {
    let content = format!(
        r#"{{
  "agents": {{
    "defaults": {{
      "model": "{model}"
    }}
  }}
}}"#
    );
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

#[given(expr = "a config with workspace {string}")]
fn given_config_with_workspace(world: &mut QuectoWorld, workspace: String) {
    let content = format!(
        r#"{{
  "agents": {{
    "defaults": {{
      "workspace": "{workspace}"
    }}
  }}
}}"#
    );
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

#[given("a config file with a workspace directory")]
fn given_config_file_with_workspace_directory(world: &mut QuectoWorld) {
    let content = r#"{
  "agents": {
    "defaults": {
      "workspace": "workspace"
    }
  }
}"#;
    ensure_temp_dir(world);
    let config_file = base_path(world).join("config.json");
    std::fs::write(&config_file, content).expect("failed to write config file");
    world.config_path = Some(config_file.to_string_lossy().to_string());
}

// ===========================================================================
// Onboard Steps (Given)
// ===========================================================================

#[given(expr = "no config file exists at {string}")]
fn given_no_config(world: &mut QuectoWorld, _path: String) {
    // Create a fresh temp dir with no config file
    let td = TempDir::new().expect("failed to create temp dir");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
    // Verify no config exists
    assert!(!base_path(world).join("config.json").exists());
}

#[given(expr = "a config file already exists at {string}")]
fn given_config_already_exists(world: &mut QuectoWorld, _path: String) {
    let td = TempDir::new().expect("failed to create temp dir");
    // Create a config file
    std::fs::write(td.path().join("config.json"), "{}").expect("failed to write");
    world.cli_context.base_dir = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

// ===========================================================================
// Config Steps (When)
// ===========================================================================

#[when("I load the config")]
fn when_load_config(world: &mut QuectoWorld) {
    let path = world
        .config_path
        .as_ref()
        .expect("config_path must be set before loading");
    let config =
        Config::load_with_env(path, &world.env_overrides).expect("Config::load_with_env failed");
    world.config = Some(config);
}

#[when("I resolve the workspace path")]
fn when_resolve_workspace(world: &mut QuectoWorld) {
    let path = world
        .config_path
        .as_ref()
        .expect("config_path must be set before resolving workspace");
    let config = Config::load(path).expect("Config::load failed");
    world.resolved_workspace = Some(config.workspace_path());
}

// ===========================================================================
// Config Steps (Then)
// ===========================================================================

#[then(expr = "the model should be {string}")]
fn then_model_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.model, expected);
}

#[then(expr = "the max_tokens should be {int}")]
fn then_max_tokens_should_be(world: &mut QuectoWorld, expected: u32) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.max_tokens, expected);
}

#[then(expr = "the max_session_messages should be {int}")]
fn then_max_session_messages_should_be(world: &mut QuectoWorld, expected: usize) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.max_session_messages, expected);
}

#[then(expr = "the OpenAI API key should be {string}")]
fn then_openai_key_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.providers.openai.api_key, expected);
}

#[then(expr = "the temperature should be {float}")]
fn then_temperature_should_be(world: &mut QuectoWorld, expected: f32) {
    let config = world.config.as_ref().expect("config not loaded");
    assert!(
        (config.agents.defaults.temperature - expected).abs() < f32::EPSILON,
        "expected temperature {}, got {}",
        expected,
        config.agents.defaults.temperature
    );
}

#[then(expr = "the workspace should be {string}")]
fn then_workspace_should_be(world: &mut QuectoWorld, expected: String) {
    let config = world.config.as_ref().expect("config not loaded");
    assert_eq!(config.agents.defaults.workspace, expected);
}

#[then(expr = "the workspace path should start with {string}")]
fn then_workspace_starts_with(world: &mut QuectoWorld, prefix: String) {
    let ws = world
        .resolved_workspace
        .as_ref()
        .expect("resolved_workspace not set");
    assert!(
        ws.starts_with(&prefix),
        "expected workspace '{}' to start with '{}'",
        ws,
        prefix
    );
}

#[then(expr = "the workspace path should end with {string}")]
fn then_workspace_ends_with(world: &mut QuectoWorld, suffix: String) {
    let ws = world
        .resolved_workspace
        .as_ref()
        .expect("resolved_workspace not set");
    assert!(
        ws.ends_with(&suffix),
        "expected workspace '{}' to end with '{}'",
        ws,
        suffix
    );
}

// ===========================================================================
// CLI Steps
// ===========================================================================

#[when("I run quecto with no arguments")]
fn when_run_no_args(world: &mut QuectoWorld) {
    let output = cli::run_with_output(vec!["quecto".to_string()], &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when(expr = "I run quecto with arguments {string}")]
fn when_run_with_args(world: &mut QuectoWorld, args_str: String) {
    let mut args = vec!["quecto".to_string()];
    args.extend(shell_split(&args_str));
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[then(expr = "the exit code should be {int}")]
fn then_exit_code(world: &mut QuectoWorld, expected: i32) {
    assert_eq!(
        world.exit_code, expected,
        "expected exit code {}, got {}.\nstdout: {}\nstderr: {}",
        expected, world.exit_code, world.stdout, world.stderr
    );
}

#[then(expr = "the output should contain {string}")]
fn then_output_contains(world: &mut QuectoWorld, expected: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        combined.contains(&expected),
        "expected output to contain '{}', got:\nstdout: {}\nstderr: {}",
        expected,
        world.stdout,
        world.stderr
    );
}

#[then(expr = "the output should not contain {string}")]
fn then_output_not_contains(world: &mut QuectoWorld, unexpected: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    assert!(
        !combined.contains(&unexpected),
        "expected output NOT to contain '{}', got:\nstdout: {}\nstderr: {}",
        unexpected,
        world.stdout,
        world.stderr
    );
}

#[then(expr = "the stderr should contain {string}")]
fn then_stderr_contains(world: &mut QuectoWorld, expected: String) {
    assert!(
        world.stderr.contains(&expected),
        "expected stderr to contain '{}', got: {}",
        expected,
        world.stderr
    );
}

#[then(expr = "the output should match {string}")]
fn then_output_matches(world: &mut QuectoWorld, pattern: String) {
    let combined = format!("{}{}", world.stdout, world.stderr);
    let re = regex::Regex::new(&pattern).expect("invalid regex pattern");
    assert!(
        re.is_match(&combined),
        "expected output to match '{}', got:\n{}",
        pattern,
        combined
    );
}

// ===========================================================================
// Onboard Steps (Then)
// ===========================================================================

#[then(expr = "a config file should exist at {string}")]
fn then_config_file_exists(world: &mut QuectoWorld, _path: String) {
    let config_path = base_path(world).join("config.json");
    assert!(
        config_path.exists(),
        "config file should exist at {}",
        config_path.display()
    );
}

#[then(expr = "a workspace directory should exist at {string}")]
fn then_workspace_dir_exists(world: &mut QuectoWorld, _path: String) {
    let ws_path = base_path(world).join("workspace");
    assert!(
        ws_path.is_dir(),
        "workspace dir should exist at {}",
        ws_path.display()
    );
}

#[then(expr = "the workspace should contain {string}")]
fn then_workspace_contains_file(world: &mut QuectoWorld, filename: String) {
    let file_path = base_path(world).join("workspace").join(&filename);
    assert!(
        file_path.exists(),
        "workspace should contain '{}' at {}",
        filename,
        file_path.display()
    );
}

#[then(expr = "the config should have model {string}")]
fn then_config_should_have_model(world: &mut QuectoWorld, expected: String) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert_eq!(config.agents.defaults.model, expected);
}

#[then(expr = "the config should have max_tokens {int}")]
fn then_config_should_have_max_tokens(world: &mut QuectoWorld, expected: u32) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert_eq!(config.agents.defaults.max_tokens, expected);
}

#[then(expr = "the config should have temperature {float}")]
fn then_config_should_have_temperature(world: &mut QuectoWorld, expected: f32) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    assert!(
        (config.agents.defaults.temperature - expected).abs() < f32::EPSILON,
        "expected temperature {}, got {}",
        expected,
        config.agents.defaults.temperature
    );
}

#[then(expr = "the config should have restrict_to_workspace {word}")]
fn then_config_should_have_restrict(world: &mut QuectoWorld, expected: String) {
    let config_path = base_path(world).join("config.json");
    let config = Config::load(config_path.to_str().unwrap()).expect("load config");
    let expected_bool = expected == "true";
    assert_eq!(
        config.agents.defaults.restrict_to_workspace, expected_bool,
        "expected restrict_to_workspace {}, got {}",
        expected_bool, config.agents.defaults.restrict_to_workspace
    );
}

// ===========================================================================
// Config steps for CLI/Observability scenarios (restored from deleted voice_steps.rs)
// ===========================================================================

#[given("a valid config with OpenAI API key set")]
fn given_valid_config_with_openai(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-123" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given("a config with OpenAI api_key set and Anthropic not set")]
fn given_config_openai_set_anthropic_not(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-key-456" },
            "anthropic": { "api_key": "" }
        }
    }"#;
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
}

#[given(expr = "a config with OpenAI api_key {string} set")]
fn given_config_with_specific_openai_key(world: &mut QuectoWorld, api_key: String) {
    ensure_temp_dir(world);
    let config = serde_json::json!({
        "providers": {
            "openai": { "api_key": api_key }
        }
    });
    let config_path = base_path(world).join("config.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap())
        .expect("write config");
}
