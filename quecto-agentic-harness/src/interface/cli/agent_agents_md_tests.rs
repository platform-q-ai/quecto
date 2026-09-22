use super::startup_prompt;
use crate::interface::cli::{CliContext, run_with_output};

#[test]
fn one_shot_startup_stops_on_invalid_agents_md_utf8() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::write(&path, [0xff]).unwrap();
    let context = CliContext {
        base_dir: Some(initialization_dir.path().join("config")),
        cwd: Some(initialization_dir.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        fresh_session_identity: Some(crate::composition::sessions::build_fresh_session_identity),
        ..CliContext::default()
    };

    let output = run_with_output(
        vec!["quecto".into(), "agent".into(), "-m".into(), "hello".into()],
        &context,
    );

    assert_eq!(output.exit_code, 1);
    assert!(output.stderr.contains("AGENTS.md is not valid UTF-8"));
    assert!(output.stderr.contains(&path.display().to_string()));
    assert!(!output.stderr.contains("config file not found"));
}

#[test]
fn one_shot_and_uds_share_the_same_startup_prompt_composer() {
    let instructions = Some("AGENTS marker".to_string());
    let explicit = Some("Explicit marker".to_string());

    let one_shot = startup_prompt::compose(
        instructions.as_deref(),
        explicit.as_deref(),
        false,
        "Extension marker",
        "Parent playbook marker",
    );
    let uds = startup_prompt::compose(
        instructions.as_deref(),
        explicit.as_deref(),
        false,
        "Extension marker",
        "Parent playbook marker",
    );

    assert_eq!(one_shot, uds);
    assert!(one_shot.find("AGENTS marker").unwrap() < one_shot.find("Explicit marker").unwrap());
    assert!(one_shot.find("Explicit marker").unwrap() < one_shot.find("Extension marker").unwrap());
}

#[test]
fn uds_startup_stops_on_invalid_agents_md_utf8_before_socket_loop() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::write(&path, [0xff]).unwrap();
    let socket = initialization_dir.path().join("agent.sock");
    let context = CliContext {
        base_dir: Some(initialization_dir.path().join("config")),
        cwd: Some(initialization_dir.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        fresh_session_identity: Some(crate::composition::sessions::build_fresh_session_identity),
        ..CliContext::default()
    };

    let output = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--mode".into(),
            "uds".into(),
            "--socket".into(),
            socket.display().to_string(),
        ],
        &context,
    );

    assert_eq!(output.exit_code, 1);
    assert!(output.stderr.contains("AGENTS.md is not valid UTF-8"));
    assert!(output.stderr.contains(&path.display().to_string()));
    assert!(!socket.exists(), "UDS socket loop must not start");
}

#[test]
fn uds_startup_stops_on_agents_md_read_error_before_socket_loop() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::create_dir(&path).unwrap();
    let socket = initialization_dir.path().join("agent.sock");
    let context = CliContext {
        base_dir: Some(initialization_dir.path().join("config")),
        cwd: Some(initialization_dir.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        fresh_session_identity: Some(crate::composition::sessions::build_fresh_session_identity),
        ..CliContext::default()
    };

    let output = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--mode".into(),
            "uds".into(),
            "--socket".into(),
            socket.display().to_string(),
        ],
        &context,
    );

    assert_eq!(output.exit_code, 1);
    assert!(output.stderr.contains("failed to read AGENTS.md"));
    assert!(output.stderr.contains(&path.display().to_string()));
    assert!(!socket.exists(), "UDS socket loop must not start");
}

#[test]
fn one_shot_startup_stops_on_agents_md_read_error() {
    let initialization_dir = tempfile::tempdir().unwrap();
    let path = initialization_dir.path().join("AGENTS.md");
    std::fs::create_dir(&path).unwrap();
    let context = CliContext {
        base_dir: Some(initialization_dir.path().join("config")),
        cwd: Some(initialization_dir.path().to_path_buf()),
        sessions: Some(crate::composition::sessions::build_session_handles),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        fresh_session_identity: Some(crate::composition::sessions::build_fresh_session_identity),
        ..CliContext::default()
    };

    let output = run_with_output(
        vec!["quecto".into(), "agent".into(), "-m".into(), "hello".into()],
        &context,
    );

    assert_eq!(output.exit_code, 1);
    assert!(output.stderr.contains("failed to read AGENTS.md"));
    assert!(output.stderr.contains(&path.display().to_string()));
}

#[test]
fn override_is_composed_only_for_parent_and_keeps_other_prompt_sources() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("PARENT_PLAYBOOK.md"),
        "Unique project playbook",
    )
    .unwrap();
    let context = CliContext {
        cwd: Some(directory.path().to_path_buf()),
        ..CliContext::default()
    };
    let mut errors = String::new();
    let playbook = startup_prompt::load_parent_playbook(&context, false, &mut errors).unwrap();
    assert!(errors.is_empty());
    let parent = startup_prompt::compose(
        Some("Project AGENTS"),
        Some("Explicit instructions"),
        false,
        "Extension instructions",
        &playbook,
    );
    assert!(parent.contains("Unique project playbook"));
    assert!(!parent.contains("### Route and isolate"));
    assert!(
        parent.find("Unique project playbook").unwrap() < parent.find("Project AGENTS").unwrap()
    );
    assert!(parent.find("Project AGENTS").unwrap() < parent.find("Explicit instructions").unwrap());
    assert!(
        parent.find("Explicit instructions").unwrap()
            < parent.find("Extension instructions").unwrap()
    );
    let child_playbook = startup_prompt::load_parent_playbook(&context, true, &mut errors).unwrap();
    let child = startup_prompt::compose(
        None,
        Some("Explicit child instructions"),
        true,
        "",
        &child_playbook,
    );
    assert!(!child.contains("Unique project playbook"));
    assert!(child.contains("Explicit child instructions"));
}

#[test]
fn present_invalid_override_fails_parent_startup_but_does_not_affect_child() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("PARENT_PLAYBOOK.md"), [0xff]).unwrap();
    let context = CliContext {
        cwd: Some(directory.path().to_path_buf()),
        ..CliContext::default()
    };
    let mut errors = String::new();
    assert!(startup_prompt::load_parent_playbook(&context, false, &mut errors).is_none());
    assert!(errors.contains("not valid UTF-8"));
    assert!(startup_prompt::load_parent_playbook(&context, true, &mut String::new()).is_some());
}
