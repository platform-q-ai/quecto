use super::*;
use std::io::Write;

#[test]
fn test_deserialize_full_config() {
    let json = r#"{
            "agents": {
                "defaults": {
                    "model": "gpt-4",
                    "max_tokens": 4096
                }
            },
            "providers": {
                "openai": {
                    "api_key": "sk-test-123"
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.agents.defaults.model, "gpt-4");
    assert_eq!(config.agents.defaults.max_tokens, 4096);
    assert_eq!(config.providers.openai.api_key, "sk-test-123");
}

#[test]
fn test_deserialize_empty_uses_defaults() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config.agents.defaults.model, "gpt-5.5");
    assert_eq!(config.agents.defaults.max_tokens, 8192);
    assert!((config.agents.defaults.temperature - 0.7).abs() < f32::EPSILON);
    assert_eq!(config.agents.defaults.workspace, "~/.quecto/workspace");
    assert_eq!(config.agents.defaults.max_tool_iterations, 999_999);
    assert!(config.agents.defaults.restrict_to_workspace);
}

#[test]
fn test_agent_defaults_has_no_command_allowlist() {
    let defaults = AgentDefaults::default();
    assert_eq!(defaults.command_allowlist, None);
}

#[test]
fn test_command_allowlist_deserializes_from_config() {
    let json = r#"{
            "agents": {
                "defaults": {
                    "command_allowlist": ["echo", "ls", "cat"]
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.agents.defaults.command_allowlist,
        Some(vec![
            "echo".to_string(),
            "ls".to_string(),
            "cat".to_string()
        ])
    );
}

#[test]
fn test_deserialize_legacy_exec_fields_ignored() {
    // Old configs may still carry the removed nsjail/network keys; serde
    // ignores unknown fields, so they deserialize without error.
    let json = r#"{
            "tools": {
                "exec": {
                    "isolation": "nsjail",
                    "nsjail_binary": "/usr/bin/nsjail",
                    "network_passthrough": true
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    // Sandbox confinement is independent and still defaults on.
    assert!(config.agents.defaults.restrict_to_workspace);
}

fn workflow_config_with_steps(steps: &str) -> String {
    format!(
        r#"{{"workflow":{{"templates":[{{"id":"test","label":"Test","description":"d","steps":[{steps}]}}]}}}}"#
    )
}

#[test]
fn test_load_workflow_step_from_string_reference() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("steps")).unwrap();
    std::fs::write(
        dir.path().join("steps/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"reuse me"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(r#""steps/shared""#),
    )
    .unwrap();

    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    let step = &config.workflow.templates[0].steps[0];
    assert_eq!(
        (step.key.as_str(), step.label.as_str()),
        ("shared", "Shared")
    );
    assert_eq!(step.phase, "green");
    assert_eq!(step.guidance.as_deref(), Some("reuse me"));
}

#[test]
fn test_load_workflow_step_reference_applies_individual_overrides() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"base"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(r#"{"ref":"shared.json","key":"second","phase":"review"}"#),
    )
    .unwrap();

    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    let step = &config.workflow.templates[0].steps[0];
    assert_eq!(
        (step.key.as_str(), step.label.as_str()),
        ("second", "Shared")
    );
    assert_eq!(step.phase, "review");
    assert_eq!(step.guidance.as_deref(), Some("base"));
}

#[test]
fn test_load_workflow_step_reference_overriding_key_label_phase_still_loads_the_file() {
    // The guidance assertion can only pass if shared.json was actually read,
    // so this pins that an all-structural-override reference still resolves
    // (it previously fell through to the inline branch and skipped the file).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"key":"base","label":"Base","phase":"green","guidance":"from the file"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(
            r#"{"ref":"shared.json","key":"new","label":"New","phase":"review"}"#,
        ),
    )
    .unwrap();

    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    let step = &config.workflow.templates[0].steps[0];
    assert_eq!((step.key.as_str(), step.label.as_str()), ("new", "New"));
    assert_eq!(step.phase, "review");
    assert_eq!(step.guidance.as_deref(), Some("from the file"));
}

#[test]
fn test_load_workflow_step_reference_with_all_overrides_still_fails_on_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(
            r#"{"ref":"missing.json","key":"new","label":"New","phase":"review","guidance":"g"}"#,
        ),
    )
    .unwrap();

    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("missing.json"), "{error}");
}

#[test]
fn test_load_workflow_step_reference_with_wrong_typed_override_names_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"key":"base","label":"Base","phase":"green"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(r#"{"ref":"shared.json","key":5}"#),
    )
    .unwrap();

    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("failed to load workflow step"),
        "{error}"
    );
    assert!(error.to_string().contains("shared.json"), "{error}");
    assert!(error.to_string().contains("invalid type"), "{error}");
}

#[test]
fn test_resolve_workflow_steps_with_bare_config_filename_uses_current_directory() {
    // Path::new("config.json").parent() is Some(""), not None; the resolver
    // must fall back to "." or every reference fails on canonicalizing "".
    let mut value: serde_json::Value =
        serde_json::from_str(&workflow_config_with_steps(r#""nope""#)).unwrap();
    let error = resolve_workflow_step_entries(&mut value, Path::new("config.json")).unwrap_err();
    assert!(error.to_string().contains("nope.json"), "{error}");
}

#[test]
fn test_load_rejects_too_many_workflow_templates_before_resolving_references() {
    let dir = tempfile::tempdir().unwrap();
    let templates = (0..=crate::domain::workflow::MAX_TEMPLATE_COUNT)
        .map(|i| format!(r#"{{"id":"t{i}","label":"T","description":"d","steps":["missing"]}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        format!(r#"{{"workflow":{{"templates":[{templates}]}}}}"#),
    )
    .unwrap();

    // No "missing.json" exists: the count check must fire before any file load.
    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("too many workflow templates"),
        "{error}"
    );
}

#[test]
fn test_load_rejects_too_many_workflow_steps_before_resolving_references() {
    let dir = tempfile::tempdir().unwrap();
    let steps = vec![r#""missing""#; crate::domain::workflow::MAX_STEPS_PER_TEMPLATE + 1].join(",");
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(&steps)).unwrap();

    // No "missing.json" exists: the count check must fire before any file load.
    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("template 'test' has too many steps"),
        "{error}"
    );
}

#[test]
fn test_load_rejects_oversized_workflow_step_file() {
    let dir = tempfile::tempdir().unwrap();
    let skeleton = r#"{"key":"big","label":"Big","phase":"green","guidance":""}"#;
    let padding = "g".repeat(MAX_WORKFLOW_STEP_FILE_BYTES as usize - skeleton.len() + 1);
    std::fs::write(
        dir.path().join("big.json"),
        skeleton.replace(r#""guidance":"""#, &format!(r#""guidance":"{padding}""#)),
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""big""#)).unwrap();

    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("step file is too large"),
        "{error}"
    );
    assert!(error.to_string().contains("big.json"), "{error}");
}

#[test]
fn test_load_workflow_step_file_at_exact_size_cap_loads() {
    let dir = tempfile::tempdir().unwrap();
    let skeleton = r#"{"key":"big","label":"Big","phase":"green","guidance":""}"#;
    let padding = "g".repeat(MAX_WORKFLOW_STEP_FILE_BYTES as usize - skeleton.len());
    let content = skeleton.replace(r#""guidance":"""#, &format!(r#""guidance":"{padding}""#));
    assert_eq!(content.len() as u64, MAX_WORKFLOW_STEP_FILE_BYTES);
    std::fs::write(dir.path().join("big.json"), content).unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""big""#)).unwrap();

    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    assert_eq!(config.workflow.templates[0].steps[0].key, "big");
}

#[test]
fn test_load_invalid_typed_field_reports_line_and_column() {
    // Configs without step references must keep serde's line/column context
    // (the reference resolver's Value round-trip would otherwise strip it).
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp,
        "{{\n  \"agents\": {{\n    \"defaults\": {{\n      \"max_tokens\": \"not_a_number\"\n    }}\n  }}\n}}"
    )
    .unwrap();
    let error = Config::load(tmp.path().to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("line 4"), "{error}");
}

#[test]
fn test_load_workflow_inline_step_remains_compatible() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp,
        "{}",
        workflow_config_with_steps(
            r#"{"key":"inline","label":"Inline","phase":"red","guidance":"today"}"#
        )
    )
    .unwrap();
    let config = Config::load(tmp.path().to_str().unwrap()).unwrap();
    assert_eq!(config.workflow.templates[0].steps[0].key, "inline");
    assert_eq!(
        config.workflow.templates[0].steps[0].guidance.as_deref(),
        Some("today")
    );
}

#[test]
fn test_load_workflow_inline_step_preserves_unknown_metadata_including_ref() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp,
        "{}",
        workflow_config_with_steps(
            r#"{"key":"inline","label":"Inline","phase":"red","owner":"team","ref":"ticket-1"}"#
        )
    )
    .unwrap();
    let config = Config::load(tmp.path().to_str().unwrap()).unwrap();
    assert_eq!(config.workflow.templates[0].steps[0].key, "inline");
}

#[test]
fn test_load_workflow_reference_rejects_paths_outside_config_directory() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    for reference in ["../outside", "/tmp/outside"] {
        std::fs::write(
            &config_path,
            workflow_config_with_steps(&format!(r#""{reference}""#)),
        )
        .unwrap();
        let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("must remain within"));
    }
}

#[cfg(unix)]
#[test]
fn test_load_workflow_reference_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    symlink(outside.path(), dir.path().join("linked.json")).unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""linked""#)).unwrap();
    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("escapes config directory"));
}

#[test]
fn test_load_workflow_reference_errors_name_the_offending_file() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(r#""steps/missing""#),
    )
    .unwrap();

    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("steps/missing.json"));
}

#[test]
fn test_load_invalid_workflow_step_json_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.json"), "{not json").unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""broken""#)).unwrap();
    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("broken.json"));
}

#[test]
fn test_load_rejects_unknown_fields_in_new_workflow_step_shapes() {
    let cases = [r#"{"ref":"shared","guidence":"typo"}"#];
    for entry in cases {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("shared.json"),
            r#"{"key":"x","label":"X","phase":"red"}"#,
        )
        .unwrap();
        let config_path = dir.path().join("config.json");
        std::fs::write(&config_path, workflow_config_with_steps(entry)).unwrap();
        let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("unknown field `guidence`"));
    }

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bad.json"),
        r#"{"key":"x","label":"X","phase":"red","guidence":"typo"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""bad""#)).unwrap();
    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("bad.json"));
    assert!(error.to_string().contains("unknown field `guidence`"));
}

#[test]
fn test_load_rejects_recursive_workflow_step_reference() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("recursive.json"), r#"{"ref":"other"}"#).unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(&config_path, workflow_config_with_steps(r#""recursive""#)).unwrap();

    let error = Config::load(config_path.to_str().unwrap()).unwrap_err();
    assert!(error.to_string().contains("recursive.json"));
    assert!(
        error
            .to_string()
            .contains("recursive references are not allowed")
    );
}

#[test]
fn test_resolved_duplicate_step_keys_are_rejected_by_engine() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("shared.json"),
        r#"{"key":"same","label":"Same","phase":"red"}"#,
    )
    .unwrap();
    let config_path = dir.path().join("config.json");
    std::fs::write(
        &config_path,
        workflow_config_with_steps(r#""shared", "shared.json""#),
    )
    .unwrap();
    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    let error = crate::domain::workflow::WorkflowEngine::new(config.workflow, true).unwrap_err();
    assert!(error.to_string().contains("duplicate step key 'same'"));
}

#[test]
fn test_load_from_file() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp,
        r#"{{ "agents": {{ "defaults": {{ "model": "claude-opus-4-5" }} }} }}"#
    )
    .unwrap();
    let config = Config::load(tmp.path().to_str().unwrap()).unwrap();
    assert_eq!(config.agents.defaults.model, "claude-opus-4-5");
    // defaults still applied for missing fields
    assert_eq!(config.agents.defaults.max_tokens, 8192);
}

#[test]
fn test_load_missing_file_returns_default() {
    // Zero-config: a missing config file yields the default config rather
    // than an error (no onboarding step required).
    let config = Config::load("/nonexistent/path/config.json").unwrap();
    assert_eq!(
        config.agents.defaults.model,
        Config::default().agents.defaults.model
    );
}

#[test]
fn test_load_missing_file_with_env_applies_overrides_on_default() {
    // Env overrides apply on top of defaults even with no config file.
    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_MODEL".to_string(),
        "env/model".to_string(),
    );
    let config = Config::load_with_env("/nonexistent/path/config.json", &env).unwrap();
    assert_eq!(config.agents.defaults.model, "env/model");
}

#[test]
fn test_load_invalid_json() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "not valid json {{").unwrap();
    let result = Config::load(tmp.path().to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("failed to parse config"));
}

#[test]
fn test_env_override_model() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(
        tmp,
        r#"{{ "agents": {{ "defaults": {{ "model": "gpt-4" }} }} }}"#
    )
    .unwrap();

    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_MODEL".to_string(),
        "claude-opus-4-5".to_string(),
    );

    let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
    assert_eq!(config.agents.defaults.model, "claude-opus-4-5");
}

#[test]
fn test_env_override_max_tokens() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "{{}}").unwrap();

    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_MAX_TOKENS".to_string(),
        "2048".to_string(),
    );

    let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
    assert_eq!(config.agents.defaults.max_tokens, 2048);
}

#[test]
fn test_env_override_provider_key() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "{{}}").unwrap();

    let mut env = HashMap::new();
    env.insert("OPENAI_API_KEY".to_string(), "sk-from-env".to_string());

    let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
    assert_eq!(config.providers.openai.api_key, "sk-from-env");
}

#[test]
fn test_workspace_path_tilde_expansion() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let ws = config.workspace_path();
    assert!(ws.starts_with('/'), "should start with /: {ws}");
    assert!(
        ws.ends_with(".quecto/workspace"),
        "should end with .quecto/workspace: {ws}"
    );
}

#[test]
fn test_workspace_path_absolute_no_expansion() {
    let json = r#"{
            "agents": {
                "defaults": {
                    "workspace": "/opt/quecto/workspace"
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.workspace_path(), "/opt/quecto/workspace");
}

#[test]
fn test_env_override_invalid_number_ignored() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    write!(tmp, "{{}}").unwrap();

    let mut env = HashMap::new();
    env.insert(
        "QUECTO_AGENTS_DEFAULTS_MAX_TOKENS".to_string(),
        "not_a_number".to_string(),
    );

    let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
    // Should keep the default since parse failed
    assert_eq!(config.agents.defaults.max_tokens, 8192);
}

#[test]
fn test_provider_entry_debug_redacts_api_key() {
    let entry = ProviderEntry {
        api_key: "sk-secret-key-12345".to_string(),
        api_base: "https://api.openai.com/v1".to_string(),
        disable_codex_routing: false,
    };
    let debug = format!("{:?}", entry);
    assert!(!debug.contains("sk-secret-key-12345"));
}

#[test]
fn test_default_max_session_messages() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config.agents.defaults.max_session_messages, 200);
}

#[test]
fn test_default_max_context_tokens() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config.agents.defaults.max_context_tokens, 200_000);
}

#[test]
fn test_default_context_collapse_after_tool_calls_is_50() {
    // #1017: collapse triggers after a configurable number of tool calls,
    // default 50 — pin the default in code, not only in docs.
    assert_eq!(default_context_collapse_after_tool_calls(), 50);
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config.agents.defaults.context_collapse_after_tool_calls, 50);
    assert_eq!(
        AgentDefaults::default().context_collapse_after_tool_calls,
        50
    );
}

#[test]
fn test_context_collapse_legacy_turns_alias_deserializes() {
    // Pre-#1017 config files used `context_collapse_after_turns`; the serde
    // alias keeps them working.
    let json = r#"{
            "agents": { "defaults": { "context_collapse_after_turns": 12 } }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.agents.defaults.context_collapse_after_tool_calls, 12);
}

#[test]
fn test_deserialize_max_session_messages_override() {
    let json = r#"{
            "agents": { "defaults": { "max_session_messages": 12 } }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.agents.defaults.max_session_messages, 12);
}

#[test]
fn test_openai_compatible_endpoints_deserialize() {
    let json = r#"{
            "providers": {
                "openai_compatible": {
                    "endpoints": [
                        {
                            "prefix": "spark",
                            "api_base": "http://127.0.0.1:8000/v1",
                            "api_key": "sk-spark",
                            "allow_remote_http": true
                        }
                    ]
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    let endpoint = &config.providers.openai_compatible.endpoints[0];
    assert_eq!(endpoint.prefix, "spark");
    assert_eq!(endpoint.api_base, "http://127.0.0.1:8000/v1");
    assert_eq!(endpoint.api_key, "sk-spark");
    assert!(endpoint.allow_remote_http);
}

#[test]
fn test_openai_disable_codex_routing_deserializes() {
    let json = r#"{
            "providers": {
                "openai": {
                    "api_key": "sk-custom",
                    "api_base": "http://127.0.0.1:8000/v1",
                    "disable_codex_routing": true
                }
            }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.providers.openai.disable_codex_routing);
}

#[test]
fn test_legacy_config_with_removed_sections_still_deserializes() {
    // Guard against regressions: existing config.json files may contain
    // telegram, heartbeat, gateway, health, voice, and cron sections that
    // were removed in #317. serde's default handling must silently ignore
    // these unknown fields.
    let json = r#"{
            "agents": { "defaults": { "model": "gpt-4" } },
            "channels": { "telegram": { "enabled": true, "token": "123:ABC" } },
            "heartbeat": { "enabled": true, "interval": 300 },
            "gateway": { "host": "0.0.0.0", "port": 8080 },
            "health": { "enabled": true, "port": 9090 },
            "voice": { "groq": { "api_key": "gsk-test" } }
        }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.agents.defaults.model, "gpt-4");
}
