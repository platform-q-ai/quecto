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

#[test]
fn test_resolve_workspace_for_skills_relative_config() {
    // A relative workspace path in config is joined onto the base dir.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"agents":{"defaults":{"workspace":"relws"}}}"#,
    )
    .unwrap();
    let result = super::resolve_workspace_for_skills(tmp.path());
    assert_eq!(result, tmp.path().join("relws"));
}

#[test]
fn test_resolve_workspace_for_skills_absolute_config() {
    // An absolute workspace path in config is returned unchanged.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"agents":{"defaults":{"workspace":"/tmp/quecto-abs-ws"}}}"#,
    )
    .unwrap();
    let result = super::resolve_workspace_for_skills(tmp.path());
    assert_eq!(result, std::path::PathBuf::from("/tmp/quecto-abs-ws"));
}

#[test]
fn test_resolve_workspace_for_skills_invalid_config_falls_back() {
    // A present-but-unparseable config falls back to <base>/workspace.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{ not valid ").unwrap();
    let result = super::resolve_workspace_for_skills(tmp.path());
    assert_eq!(result, tmp.path().join("workspace"));
}

// ===================================================================
// cmd_status: config load failure branch
// ===================================================================

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

// ===================================================================
// skills install: download + frontmatter validation branches
// ===================================================================

/// Start a wiremock server that answers every GET with `status`/`body`.
fn mock_any_get(status: u16, body: String) -> wiremock::MockServer {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        server
    })
}

#[test]
fn test_skills_install_download_not_found() {
    // Empty server → both main and master branches 404 → "skill not found".
    let tmp = tempfile::TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async { wiremock::MockServer::start().await });
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to download skill"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_download_server_error() {
    // 500 from both branches → last_error carries the HTTP status.
    let tmp = tempfile::TempDir::new().unwrap();
    let server = mock_any_get(500, "boom".to_string());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to download skill"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_download_connection_error() {
    // Unreachable port → request error branch.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some("http://127.0.0.1:1".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to download skill"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_body_too_large() {
    // Body exceeds the 256 KiB cap → download aborts.
    let tmp = tempfile::TempDir::new().unwrap();
    let big = "x".repeat(300 * 1024);
    let server = mock_any_get(200, big);
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to download skill"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_no_frontmatter() {
    // Body without `---` delimiters → invalid frontmatter.
    let tmp = tempfile::TempDir::new().unwrap();
    let server = mock_any_get(200, "no frontmatter here".to_string());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("invalid SKILL.md frontmatter"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_empty_description_fails_validation() {
    // Parses (name+description keys present) but description is empty →
    // validate_frontmatter rejects it.
    let tmp = tempfile::TempDir::new().unwrap();
    let server = mock_any_get(
        200,
        "---\nname: weather\ndescription:\n---\nBody".to_string(),
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("invalid SKILL.md frontmatter"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_skills_install_name_mismatch() {
    // Valid frontmatter, but the declared name differs from the requested one.
    let tmp = tempfile::TempDir::new().unwrap();
    let server = mock_any_get(
        200,
        "---\nname: other\ndescription: Something\n---\nBody".to_string(),
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("invalid SKILL.md name"),
        "stderr: {}",
        out.stderr
    );
}

// ===================================================================
// skills install / remove: filesystem error arms (Unix permission based)
// ===================================================================

#[test]
fn test_skills_install_create_dir_all_fails_when_skills_path_is_file() {
    // A regular file occupying the `skills` path makes create_dir_all fail.
    let tmp = tempfile::TempDir::new().unwrap();
    let ws_skills = tmp.path().join("workspace").join("skills");
    std::fs::create_dir_all(ws_skills.parent().unwrap()).unwrap();
    std::fs::write(&ws_skills, "i am a file, not a dir").unwrap();
    let server = mock_any_get(
        200,
        "---\nname: weather\ndescription: Weather forecasts\n---\nBody".to_string(),
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to create skills directory"),
        "stderr: {}",
        out.stderr
    );
}

#[cfg(unix)]
#[test]
fn test_skills_install_create_dir_fails_in_readonly_parent() {
    use std::os::unix::fs::PermissionsExt;
    // A read-only `skills` dir lets create_dir_all succeed (already exists) but
    // makes create_dir(skill_dir) fail with permission denied.
    let tmp = tempfile::TempDir::new().unwrap();
    let ws_skills = tmp.path().join("workspace").join("skills");
    std::fs::create_dir_all(&ws_skills).unwrap();
    std::fs::set_permissions(&ws_skills, std::fs::Permissions::from_mode(0o555)).unwrap();
    let server = mock_any_get(
        200,
        "---\nname: weather\ndescription: Weather forecasts\n---\nBody".to_string(),
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        github_raw_base_url: Some(server.uri()),
        ..Default::default()
    };
    let out = run_with_output(args("skills install user/repo/weather"), &ctx);
    // Restore perms so TempDir cleanup works.
    std::fs::set_permissions(&ws_skills, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to create skill directory"),
        "stderr: {}",
        out.stderr
    );
}

#[cfg(unix)]
#[test]
fn test_skills_remove_dir_fails_in_readonly_parent() {
    use std::os::unix::fs::PermissionsExt;
    // A read-only parent dir makes remove_dir_all of the skill fail.
    let tmp = tempfile::TempDir::new().unwrap();
    let ws_skills = tmp.path().join("workspace").join("skills");
    let skill_dir = ws_skills.join("weather");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "body").unwrap();
    std::fs::set_permissions(&ws_skills, std::fs::Permissions::from_mode(0o555)).unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("skills remove weather"), &ctx);
    // Restore perms so TempDir cleanup works.
    std::fs::set_permissions(&ws_skills, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("failed to remove skill"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_download_skill_markdown_within_runtime_errors() {
    // download_skill_markdown refuses to spin up a nested runtime.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        super::download_skill_markdown("http://example.invalid", "owner", "repo", "weather")
    });
    match result {
        Err(msg) => assert!(msg.contains("async runtime"), "got: {msg}"),
        Ok(_) => panic!("expected error inside runtime"),
    }
}
