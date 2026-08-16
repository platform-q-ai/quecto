use super::*;

#[test]
fn leading_config_dispatches_to_following_agent_subcommand() {
    let base = tempfile::TempDir::new().unwrap();
    let custom = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-base"}}}"#,
    )
    .unwrap();
    let config_path = custom.path().join("custom.json");
    std::fs::write(
        &config_path,
        r#"{"providers":{"openai":{"api_key":""}},"agents":{"defaults":{"model":"openai/gpt-5.2"}}}"#,
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
            config_path.display().to_string(),
            "agent".into(),
            "-m".into(),
            "hello".into(),
        ],
        &ctx,
    );

    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("no LLM providers"), "{}", out.stderr);
    assert!(!out.stdout.contains("unknown flag '-m'"));
    assert!(!out.stderr.contains("unknown flag '-m'"));
}
