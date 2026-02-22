use super::*;
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
fn test_onboard_creates_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("onboard"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto is ready"));
    assert!(tmp.path().join("config.json").exists());
    assert!(tmp.path().join("workspace").exists());
}

#[test]
fn test_onboard_existing_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("onboard"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Config already exists"));
}

#[test]
fn test_status_shows_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "agents": { "defaults": { "model": "gpt-5.2" } },
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
            "gpt-5.2",
            "OpenAI API:",
            "configured",
            "Anthropic API:",
            "not set",
        ],
    );
}

#[test]
fn test_status_no_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("config not found"));
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
fn test_skills_no_subcommand() {
    let out = run_with_output(args("skills"), &CliContext::default());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing subcommand"));
}

#[test]
fn test_skills_unknown_subcommand() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills foobar"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown subcommand"));
}

#[test]
fn test_skills_list_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills list"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("No skills installed"));
}

#[test]
fn test_skills_list_with_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: weather\ndescription: Weather forecasts\n---\nBody",
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills list"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("weather"));
    assert!(out.stdout.contains("Weather forecasts"));
}

#[test]
fn test_skills_remove_missing_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills remove"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing skill name"));
}

#[test]
fn test_skills_remove_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills remove nonexistent"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not found"));
}

#[test]
fn test_skills_install_not_implemented() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not yet implemented"));
}

#[test]
fn test_status_shows_telegram_and_heartbeat() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": { "openai": { "api_key": "sk-test" } },
        "channels": { "telegram": { "enabled": true, "token": "123:ABC" } },
        "heartbeat": { "enabled": true, "interval": 300 }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Telegram:"));
    assert!(out.stdout.contains("enabled"));
    assert!(out.stdout.contains("Heartbeat:"));
    assert!(out.stdout.contains("300s"));
}

#[test]
fn test_status_disabled_telegram_and_heartbeat() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{
        "providers": { "openai": { "api_key": "sk-test" } },
        "channels": { "telegram": { "enabled": false } },
        "heartbeat": { "enabled": false }
    }"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("disabled"));
}

#[test]
fn test_gateway_no_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    // cmd_gateway_run uses eprintln directly, so we test through run_with_output
    // (which routes "gateway" to a hint message since the real gateway path
    // goes through run() -> cmd_gateway_run)
    let out = run_with_output(args("gateway"), &ctx);
    assert_eq!(out.exit_code, 0);
}

// ===================================================================
// skills remove success + edge cases
// ===================================================================

#[test]
fn test_skills_remove_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("workspace").join("skills").join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Test\n---\nBody",
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills remove my-skill"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("removed successfully"));
    assert!(!skill_dir.exists());
}

#[test]
fn test_skills_remove_invalid_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills remove ../evil"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not found"));
}

// ===================================================================
// status with both providers configured
// ===================================================================

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
    // Both should show "configured"
    let configured_count = out.stdout.matches("configured").count();
    assert_eq!(configured_count, 2, "stdout: {}", out.stdout);
}

// ===================================================================
// cmd_gateway_run() no-config path
// ===================================================================

#[test]
fn test_cmd_gateway_run_no_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    // cmd_gateway_run uses eprintln, so we can't capture stderr via CliOutput.
    // But we can verify the exit code.
    let code = cmd_gateway_run(&ctx);
    assert_eq!(code, 1);
}
