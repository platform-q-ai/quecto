use super::super::cov_tests::{make_buf_repl_for_agent_cov, out_for_agent_cov, rt_for_agent_cov};

#[test]
fn bufreader_spawn_commands_cover_usage_errors_profile_and_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());

    repl.handle_spawn("/spawn --help", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("Usage: /spawn [flags] <task>"), "{output}");
    assert!(output.contains("--agent <name>"), "{output}");

    repl.writer.clear();
    repl.handle_spawn("/spawn", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("missing task description"), "{output}");

    repl.writer.clear();
    repl.handle_spawn("/spawn --model local/test do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(
        output.contains("--model is not supported in REPL mode"),
        "{output}"
    );

    repl.writer.clear();
    repl.handle_spawn("/spawn --agent missing do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("agent 'missing' not found"), "{output}");

    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("bad.json"), b"not-json").unwrap();
    repl.writer.clear();
    repl.handle_spawn("/spawn --agent bad do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("invalid profile for 'bad'"), "{output}");

    std::fs::write(
        agents_dir.join("helper.json"),
        serde_json::json!({"name":"helper","system":"Be terse"}).to_string(),
    )
    .unwrap();
    repl.writer.clear();
    repl.handle_spawn("/spawn --agent helper do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].content, "stub response");
}
