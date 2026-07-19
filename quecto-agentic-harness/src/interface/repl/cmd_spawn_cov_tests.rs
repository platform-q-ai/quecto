use super::super::cov_tests::{
    BufReplOpts, make_buf_repl, make_buf_repl_for_agent_cov, out, out_for_agent_cov,
    rt_for_agent_cov,
};

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

#[test]
fn bufreader_spawn_parse_and_timeout_edge_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());

    repl.handle_spawn("/spawn --system Be terse do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 1);

    repl.writer.clear();
    repl.handle_spawn("/spawn --agent ../bad do it", &runtime);
    assert!(out_for_agent_cov(&repl).contains("invalid agent name"));

    repl.writer.clear();
    repl.handle_spawn("/spawn --system", &runtime);
    assert!(out_for_agent_cov(&repl).contains("--system requires a value"));

    repl.writer.clear();
    repl.handle_spawn("/spawn --max-time nope do it", &runtime);
    assert!(out_for_agent_cov(&repl).contains("invalid --max-time value"));

    repl.writer.clear();
    repl.handle_spawn("/spawn --max-time 5 do it", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 2);
}

#[test]
fn bufreader_spawn_agent_profile_without_system_still_runs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("nosys.json"), r#"{"name":"nosys"}"#).unwrap();

    repl.handle_spawn("/spawn --agent nosys do it", &runtime);

    let output = out_for_agent_cov(&repl);
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 1);
}

#[test]
fn bufreader_run_drives_spawn_usage_and_timed_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = b"/spawn --help\n/spawn --max-time 5 do it\n/exit\n";
    let mut repl = make_buf_repl(tmp.path(), input.as_slice(), BufReplOpts::default());

    assert_eq!(repl.run(), 0);

    let output = out(&repl);
    assert!(output.contains("Usage: /spawn [flags] <task>"), "{output}");
    assert!(output.contains("--max-time <secs>"), "{output}");
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 1);
}
