use crate::interface::cli::{CliContext, run_with_output};

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected output to contain '{needle}', got:\n{haystack}"
        );
    }
}

#[test]
fn test_status_shows_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "agents": { "defaults": { "model": "gpt-5.4" } },
        "providers": {
            "openai": { "api_key": "sk-test" },
            "anthropic": { "api_key": "" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_contains_all(
        &out.stdout,
        &[
            "quecto Status",
            "Config:",
            "Workspace:",
            "Model:",
            "gpt-5.4",
            "OpenAI API:",
            "configured",
            "Anthropic API:",
            "not set",
        ],
    );
}

#[test]
fn test_status_respects_global_config_flag() {
    let base = tempfile::TempDir::new().unwrap();
    let custom_dir = tempfile::TempDir::new().unwrap();
    let custom_config = custom_dir.path().join("custom.json");
    std::fs::write(
        &custom_config,
        r#"{"agents":{"defaults":{"model":"custom/status-model"}}}"#,
    )
    .unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"agents":{"defaults":{"model":"base/status-model"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        ..Default::default()
    };

    let out = run_with_output(
        vec![
            "quecto".into(),
            "--config".into(),
            custom_config.display().to_string(),
            "status".into(),
        ],
        &ctx,
    );

    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains(&format!("Config:    {}", custom_config.display())),
        "stdout: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("custom/status-model"),
        "stdout: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("base/status-model"),
        "stdout: {}",
        out.stdout
    );
}

#[test]
fn test_status_no_config_uses_defaults() {
    // Zero-config: status with no config file succeeds on defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto Status"));
    assert!(!out.stderr.contains("config not found"));
}

#[test]
fn test_status_redacts_api_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-super-secret-12345" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(!out.stdout.contains("sk-super-secret-12345"));
    assert!(out.stdout.contains("configured"));
}

#[test]
fn test_status_both_providers_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test-openai" },
            "anthropic": { "api_key": "sk-ant-test" }
        }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    let configured_count = out.stdout.matches("configured").count();
    assert_eq!(configured_count, 2, "stdout: {}", out.stdout);
}

#[test]
fn test_status_explicit_missing_config_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let missing_config = tmp.path().join("missing.json");
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };

    let out = run_with_output(
        vec![
            "quecto".into(),
            "--config".into(),
            missing_config.display().to_string(),
            "status".into(),
        ],
        &ctx,
    );

    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .contains(&format!("config not found: {}", missing_config.display())),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_status_invalid_config_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{ not valid json ").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to load config"),
        "stderr: {}",
        out.stderr
    );
}
