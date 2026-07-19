use super::super::cov_tests::{make_buf_repl_for_agent_cov, out_for_agent_cov, rt_for_agent_cov};

#[test]
fn bufreader_agent_commands_cover_profile_lifecycle_and_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());

    assert_eq!(repl.agents_dir(), tmp.path().join("agents"));

    repl.handle_agent("/agent list", &runtime);
    assert!(out_for_agent_cov(&repl).contains("No subagent profiles"));

    repl.writer.clear();
    repl.handle_agent(
        "/agent create helper --system Be concise --model local/test",
        &runtime,
    );
    assert!(out_for_agent_cov(&repl).contains("Agent 'helper' created"));

    repl.writer.clear();
    repl.handle_agent("/agent list", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("Subagent profiles:"), "{output}");
    assert!(output.contains("helper"), "{output}");

    repl.writer.clear();
    repl.handle_agent("/agent show helper", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("Agent: helper"), "{output}");
    assert!(output.contains("System: Be concise"), "{output}");
    assert!(output.contains("Model: local/test"), "{output}");

    repl.writer.clear();
    repl.handle_agent(
        "/agent edit helper --system Updated --model local/other",
        &runtime,
    );
    assert!(out_for_agent_cov(&repl).contains("Agent 'helper' updated"));
    let saved: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join("agents/helper.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["system"], "Updated");
    assert_eq!(saved["model"], "local/other");

    repl.writer.clear();
    repl.handle_agent("/agent run helper Do the thing", &runtime);
    assert!(out_for_agent_cov(&repl).contains("stub response"));
    assert_eq!(
        repl.session.messages.last().unwrap().content,
        "stub response"
    );

    repl.writer.clear();
    repl.handle_agent("/agent remove helper", &runtime);
    assert!(out_for_agent_cov(&repl).contains("Agent 'helper' removed"));
    assert!(!tmp.path().join("agents/helper.json").exists());

    repl.writer.clear();
    repl.handle_agent("/agent show ../bad", &runtime);
    assert!(out_for_agent_cov(&repl).contains("invalid agent name"));

    repl.writer.clear();
    repl.handle_agent("/agent frobnicate", &runtime);
    assert!(out_for_agent_cov(&repl).contains("Usage: /agent"));
}
