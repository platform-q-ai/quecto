use super::super::cov_tests::{
    BufReplOpts, make_buf_repl, make_buf_repl_for_agent_cov, out, out_for_agent_cov,
    rt_for_agent_cov,
};

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

#[test]
fn bufreader_agent_list_and_parse_error_edge_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());

    std::fs::write(tmp.path().join("agents"), "not a dir").unwrap();
    repl.handle_agent("/agent list", &runtime);
    assert!(out_for_agent_cov(&repl).contains("No subagent profiles"));

    repl.writer.clear();
    repl.handle_agent("/agent create helper --system hi", &runtime);
    assert!(out_for_agent_cov(&repl).contains("Error:"));

    std::fs::remove_file(tmp.path().join("agents")).unwrap();
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(tmp.path().join("agents/alpha.json"), "{}").unwrap();
    std::fs::write(tmp.path().join("agents/beta.txt"), "ignore").unwrap();
    repl.writer.clear();
    repl.handle_agent("/agent list", &runtime);
    let output = out_for_agent_cov(&repl);
    assert!(output.contains("alpha"), "{output}");
    assert!(!output.contains("beta"), "{output}");

    repl.writer.clear();
    repl.handle_agent("/agent create helper --system", &runtime);
    assert!(out_for_agent_cov(&repl).contains("--system requires a value"));

    repl.writer.clear();
    repl.handle_agent("/agent edit helper --model", &runtime);
    assert!(out_for_agent_cov(&repl).contains("--model requires a value"));
}

#[test]
fn bufreader_agent_edit_invalid_json_profile_rewrites_default() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = rt_for_agent_cov();
    let mut repl = make_buf_repl_for_agent_cov(tmp.path());
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("broken.json"), "not-json").unwrap();

    repl.handle_agent("/agent edit broken --system fixed --model m", &runtime);

    let output = out_for_agent_cov(&repl);
    assert!(output.contains("Agent 'broken' updated"), "{output}");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(agents_dir.join("broken.json")).unwrap())
            .unwrap();
    assert_eq!(saved["system"], "fixed");
    assert_eq!(saved["model"], "m");
}

#[test]
fn bufreader_run_drives_agent_profile_lifecycle_with_real_bufreader_type() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = b"/agent list\n/agent create helper --system Be brief --model local/test\n/agent show helper\n/agent edit helper --system Updated --model local/next\n/agent run helper do work\n/agent remove helper\n/agent unknown\n/exit\n";
    let mut repl = make_buf_repl(tmp.path(), input.as_slice(), BufReplOpts::default());

    assert_eq!(repl.run(), 0);

    let output = out(&repl);
    assert!(
        output.contains("No subagent profiles configured"),
        "{output}"
    );
    assert!(output.contains("Agent 'helper' created"), "{output}");
    assert!(output.contains("Agent: helper"), "{output}");
    assert!(output.contains("System: Be brief"), "{output}");
    assert!(output.contains("Model: local/test"), "{output}");
    assert!(output.contains("Agent 'helper' updated"), "{output}");
    assert!(output.contains("stub response"), "{output}");
    assert!(output.contains("Agent 'helper' removed"), "{output}");
    assert!(output.contains("Usage: /agent <subcommand>"), "{output}");
    assert!(!tmp.path().join("agents/helper.json").exists());
}
