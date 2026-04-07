use crate::interface::cli::{CliContext, run_with_output};

fn mock_github_raw_skill(owner: &str, repo: &str, skill: &str, body: &str) -> wiremock::MockServer {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(format!(
                "/{owner}/{repo}/main/{skill}/SKILL.md"
            )))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        server
    })
}

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
fn test_skills_install_missing_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing skill path"));
}

#[test]
fn test_skills_install_invalid_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install invalid-path"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("invalid skill path"));
}

#[test]
fn test_skills_install_already_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("already exists"));
}

#[test]
fn test_skills_install_creates_skill_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let server = mock_github_raw_skill(
        "user",
        "repo",
        "weather",
        "---\nname: weather\ndescription: Weather forecasts\n---\nWeather content",
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("installed"));

    let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
    assert!(skill_dir.is_dir());
    assert!(skill_dir.join("SKILL.md").is_file());
    let content = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(content.contains("Weather forecasts"));
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

// --- parse_github_skill_path tests ---

#[test]
fn test_parse_github_skill_path_valid() {
    let result = super::parse_github_skill_path("owner/repo/skill");
    assert_eq!(result, Some(("owner", "repo", "skill")));
}

#[test]
fn test_parse_github_skill_path_too_few_parts() {
    assert!(super::parse_github_skill_path("owner/repo").is_none());
    assert!(super::parse_github_skill_path("owner").is_none());
    assert!(super::parse_github_skill_path("").is_none());
}

#[test]
fn test_parse_github_skill_path_too_many_parts() {
    assert!(super::parse_github_skill_path("a/b/c/d").is_none());
}

#[test]
fn test_parse_github_skill_path_invalid_owner() {
    assert!(super::parse_github_skill_path(".hidden/repo/skill").is_none());
}

// --- is_valid_github_slug tests ---

#[test]
fn test_is_valid_github_slug_valid() {
    assert!(super::is_valid_github_slug("owner"));
    assert!(super::is_valid_github_slug("my-repo"));
    assert!(super::is_valid_github_slug("my_repo"));
    assert!(super::is_valid_github_slug("my.repo"));
    assert!(super::is_valid_github_slug("Owner123"));
}

#[test]
fn test_is_valid_github_slug_invalid() {
    assert!(!super::is_valid_github_slug(""));
    assert!(!super::is_valid_github_slug("."));
    assert!(!super::is_valid_github_slug(".."));
    assert!(!super::is_valid_github_slug(".hidden"));
    assert!(!super::is_valid_github_slug("trail."));
    assert!(!super::is_valid_github_slug("has space"));
    assert!(!super::is_valid_github_slug("has/slash"));
}

// --- resolve_workspace_for_skills ---

#[test]
fn test_resolve_workspace_for_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let result = super::resolve_workspace_for_skills(tmp.path());
    assert!(result.ends_with("workspace"));
}
