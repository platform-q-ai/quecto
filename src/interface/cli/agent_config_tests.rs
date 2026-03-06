// Issue #300: --config flag tests

use super::CliContext;
use crate::interface::cli::run_with_output;

#[test]
fn test_agent_config_flag_loads_custom_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = tmp.path().join("custom.json");
    std::fs::write(&cfg, "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        config_path: Some(cfg.clone()),
        ..Default::default()
    };
    let args = vec![
        "quecto".into(),
        "agent".into(),
        "--config".into(),
        cfg.to_str().unwrap().into(),
        "-m".into(),
        "Hi".into(),
    ];
    let out = run_with_output(args, &ctx);
    assert!(
        !out.stderr.contains("config not found"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_config_flag_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        ..Default::default()
    };
    let out = run_with_output(
        vec!["quecto".into(), "agent".into(), "--config".into()],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--config requires"));
}

#[test]
fn test_agent_config_flag_nonexistent_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        ..Default::default()
    };
    let args = vec![
        "quecto".into(),
        "agent".into(),
        "--config".into(),
        "/tmp/no-such-config.json".into(),
        "-m".into(),
        "hi".into(),
    ];
    let out = run_with_output(args, &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("config not found"),
        "stderr: {}",
        out.stderr
    );
}
