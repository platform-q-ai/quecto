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
    );
    let uds = startup_prompt::compose(
        instructions.as_deref(),
        explicit.as_deref(),
        false,
        "Extension marker",
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
