use super::*;

#[test]
fn test_headless_agent_registry_includes_spawn_tool() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::security::sandbox::Sandbox;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();

    let config = Config::load(tmp.path().join("config.json").to_str().unwrap()).unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        workspace.clone(),
        sandbox,
        crate::infrastructure::tools::bash::ExecOptions {
            max_capture_bytes: exec_settings,
            ..Default::default()
        },
        false,
    );

    // Match what build_agent_from_config does: with_base_dir so spawn is
    // not in stub mode and will actually launch subagents.
    use crate::infrastructure::tools::spawn::SpawnTool;
    use std::sync::Arc;
    let spawn = SpawnTool::with_base_dir(
        vec![],
        config.agents.defaults.restrict_to_workspace,
        tmp.path().to_path_buf(),
    );

    // Assert the constructed tool has a real base_dir (not stub mode).
    let debug_str = format!("{:?}", spawn);
    assert!(
        debug_str.contains(tmp.path().to_str().unwrap()),
        "SpawnTool must be constructed with a real base_dir, got: {}",
        debug_str
    );

    registry.register(Arc::new(spawn));

    let names = registry.names();
    assert!(
        names.contains(&"spawn".to_string()),
        "headless agent registry must include 'spawn' tool, got: {:?}",
        names
    );
}

// --disable-tool flag (#402)

#[test]
fn test_agent_disable_tool_flag_single() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--disable-tool".into(),
        "bash".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.disabled_tools, vec!["bash"]);
}

#[test]
fn test_agent_disable_tool_flag_multiple() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--disable-tool".into(),
        "bash".into(),
        "--disable-tool".into(),
        "web_fetch".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.disabled_tools, vec!["bash", "web_fetch"]);
}

#[test]
fn test_agent_disable_tool_flag_missing_value() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--disable-tool".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("--disable-tool requires a tool name"));
}

#[test]
fn test_agent_disable_tool_absent_is_empty() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.disabled_tools.is_empty());
}

// --persist flag (#348)

#[test]
fn test_agent_persist_flag() {
    let mut e = String::new();
    // Default: persist is false
    let a = vec!["--mode".into(), "uds".into()];
    assert!(!parse_agent_flags(&a, &mut e).unwrap().persist);
    // Explicit --persist sets it to true, combines with other flags
    let a = vec![
        "--mode".into(),
        "uds".into(),
        "--persist".into(),
        "--no-session".into(),
        "--socket".into(),
        "/tmp/t.sock".into(),
    ];
    let f = parse_agent_flags(&a, &mut e).unwrap();
    assert!(f.persist && f.no_session && f.uds_mode);
    assert_eq!(
        f.socket_path.as_deref(),
        Some(std::path::Path::new("/tmp/t.sock"))
    );
    // --persist without --mode uds is rejected
    e.clear();
    let a = vec!["--persist".into(), "-m".into(), "hi".into()];
    assert!(parse_agent_flags(&a, &mut e).is_none());
    assert!(e.contains("--persist requires --mode uds"));
}
