use super::*;

fn args(command: &str) -> Vec<String> {
    std::iter::once("quecto".to_owned())
        .chain(command.split_whitespace().map(str::to_owned))
        .collect()
}

#[test]
fn live_cli_commands_keep_help_version_and_unknown_dispatch() {
    let help = run_with_output(args("help"), &CliContext::default());
    assert_eq!(help.exit_code, 0);
    assert!(help.stdout.contains("Usage: quecto [command]"));
    assert!(help.stdout.contains("agent"));

    let version = run_with_output(args("--version"), &CliContext::default());
    assert_eq!(version.exit_code, 0);
    assert!(version.stdout.contains(env!("CARGO_PKG_VERSION")));

    let unknown = run_with_output(args("not-a-command"), &CliContext::default());
    assert_eq!(unknown.exit_code, 1);
    assert!(unknown.stderr.contains("Unknown command: not-a-command"));
}

fn test_composition() -> CliComposition {
    CliComposition {
        web_fetch_tool_factory: crate::composition::web_fetch::build,
        teardown_graph: crate::composition::subagent_teardown::build_teardown_graph,
        kill_tool: crate::composition::subagent_termination::install_termination_owners,
        sessions: crate::composition::sessions::build_session_handles,
        retention: crate::composition::sessions::build_retention_handles,
        fresh_session_identity: crate::composition::sessions::build_fresh_session_identity,
        configuration: crate::composition::configuration::build_configuration_handles,
        admission: crate::composition::admission::build_admission_handles,
        catalogue: crate::composition::catalogue::build_catalogue_handles,
        provider_runtime: crate::composition::runtime::build_agent_provider,
        tool_policy_persistence: crate::composition::tool_policy::build_tool_policy_persistence,
        container_configs:
            crate::composition::container_configs::build_agent_container_config_handles,
        container_doctor: crate::composition::environments::build_container_doctor,
        environment_registry: quecto::composition::environments::build_environment_registry,
        container_inventory: quecto::composition::environments::build_container_inventory,
        container_init: crate::composition::standard_container::build_standard_container_init,
        container_status: crate::composition::standard_container::build_container_status,
    }
}

#[test]
fn real_run_dispatches_non_repl_commands() {
    let composition = test_composition();
    assert_eq!(run(args("version"), composition), 0);
    assert_eq!(run(args("definitely-not-a-command"), composition), 1);
}

#[test]
fn public_run_accepts_required_web_fetch_factory_when_fetch_is_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.json");
    std::fs::write(
        &config,
        r#"{
            "providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}},
            "tools":{"web":{"fetch":{"enabled":true}}}
        }"#,
    )
    .unwrap();
    let result = std::panic::catch_unwind(|| {
        run(
            vec![
                "quecto".into(),
                "--config".into(),
                config.display().to_string(),
                "agent".into(),
                "--no-session".into(),
                "--message".into(),
                "hello".into(),
                "--max-iterations".into(),
                "1".into(),
            ],
            test_composition(),
        )
    });
    assert!(
        result.is_ok(),
        "public run must not panic with fetch enabled"
    );
}

/// The blocking pool keeps a resident thread: two sequential blocking tasks
/// run on the same OS thread instead of each creating (and retiring) one.
#[test]
fn harness_runtime_keeps_a_resident_blocking_thread() {
    let runtime = super::build_tokio_runtime().unwrap();
    let blocking_thread = || async {
        tokio::task::spawn_blocking(|| std::thread::current().id())
            .await
            .unwrap()
    };
    let first = runtime.block_on(blocking_thread());
    std::thread::sleep(std::time::Duration::from_millis(20));
    let second = runtime.block_on(blocking_thread());
    assert_eq!(first, second, "the warm thread is reused, never retired");
}

/// The test-support execution-state and ledger-hint probes are part of the
/// lib's public surface for the BDD suites; pin their shapes here too.
#[tokio::test]
async fn test_support_probes_report_live_and_completed_execution_state() {
    use crate::domain::agent::AgentProgressEvent;
    let started = AgentProgressEvent::ToolStarted {
        tool_call_id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    };
    let live = super::live_execution_state_for_events(std::slice::from_ref(&started));
    assert_eq!(live["execution"]["phase"], "runningTool");
    assert_eq!(live["execution"]["tools"]["started"], 1);
    let done = super::completed_live_execution_state(&[started]);
    assert_eq!(done["execution"]["phase"], "idle");
    assert_eq!(done["messageCount"], 0);
    let hints = super::ledger_hint_lines_for_turn_events(
        &[AgentProgressEvent::Done],
        &crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[])
            .active_session,
    )
    .await;
    assert!(
        hints.iter().all(|line| line.is_object()),
        "every emitted line is a JSON event: {hints:?}"
    );
}

/// The interface never probes for a config itself (#1966): without
/// composition's configuration builder a config-loading command refuses to
/// run rather than quietly reading the global file.
#[test]
fn config_loading_commands_refuse_to_run_without_the_configuration_capability() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("configuration capability not composed"),
        "{}",
        out.stderr
    );
}

/// The composed context merges a trusted `./.quecto/config.json` from the
/// hermetic cwd over the base directory's file and reports both (#2024);
/// a retired `./config.json` beside it is warned about, never loaded.
#[test]
fn composed_context_layers_the_working_directory_overlay_over_the_global_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("project");
    std::fs::create_dir_all(cwd.join(".quecto")).unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"agents":{"defaults":{"model":"global-model"}}}"#,
    )
    .unwrap();
    std::fs::write(
        cwd.join(".quecto").join("config.json"),
        r#"{"agents":{"defaults":{"model":"local-model"}}}"#,
    )
    .unwrap();
    std::fs::write(
        cwd.join("config.json"),
        r#"{"agents":{"defaults":{"model":"legacy-model"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        cwd: Some(cwd.clone()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_configs: Some(
            crate::composition::container_configs::build_agent_container_config_handles,
        ),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(
            crate::composition::tool_policy::build_tool_policy_persistence,
        ),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains(&format!(
            "Config:    {}",
            tmp.path().join("config.json").display()
        )),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Model:     global-model"),
        "untrusted overlay is not applied: {}",
        out.stdout
    );
    assert!(out.stderr.contains("not trusted"), "{}", out.stderr);
    assert!(out.stderr.contains("no longer loaded"), "{}", out.stderr);

    let trust = run_with_output(args("config trust"), &ctx);
    assert_eq!(trust.exit_code, 0, "{}", trust.stderr);
    let out = run_with_output(args("status"), &ctx);
    assert!(
        out.stdout.contains(&format!(
            "Overlay:   {} (trusted)",
            cwd.join(".quecto").join("config.json").display()
        )),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Model:     local-model"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_repl_runs_setup_commands_refuses_the_rest_and_no_arguments_at_all_means_the_repl() {
    let dir = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(dir.path().to_path_buf()),
        cwd: Some(dir.path().to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        ..Default::default()
    };
    // Piped input: no banner, no prompt; a command's stdout and stderr
    // both reach the writer; a non-zero command code becomes the exit
    // code; `exit` ends the loop before the lines after it.
    let output = run_repl_with_output(
        &ctx,
        &[],
        b"\nstatus\nagent -m hi\n/help\nexit\nstatus\n",
        false,
    );
    assert_eq!(output.exit_code, 0, "{}", output.stdout);
    assert_eq!(output.stdout.matches("quecto Status\n").count(), 1);
    assert!(
        output.stdout.contains(&format!(
            "Config:    {}",
            dir.path().join("config.json").display()
        )),
        "{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("Unsupported REPL command"),
        "{}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("Setup & Configuration"),
        "{}",
        output.stdout
    );

    // A terminal gets the banner and the prompt; a command the REPL
    // delegates but that fails (`models` with no argument it knows)
    // leaves the tty session's exit code at 0.
    let output = run_repl_with_output(&ctx, &[], b"models --nope\n", true);
    assert!(output.stdout.starts_with("quecto v"), "{}", output.stdout);
    assert!(output.stdout.contains("> "), "{}", output.stdout);
    assert_eq!(output.exit_code, 0, "{}", output.stdout);
    let piped = run_repl_with_output(&ctx, &[], b"models --nope\n", false);
    assert_ne!(piped.exit_code, 0, "{}", piped.stdout);

    // Agent flags on the REPL are gone.
    let refused = run_repl_with_output(&ctx, &["-m".to_string()], b"", false);
    assert_eq!(refused.exit_code, 1);
    assert!(
        refused.stderr.contains("use `quecto agent`"),
        "{}",
        refused.stderr
    );

    // No arguments: the REPL over empty input exits at once.
    let bare = run_with_output(vec!["quecto".to_string()], &ctx);
    assert_eq!(bare.exit_code, 0);
    assert!(bare.stdout.is_empty(), "{}", bare.stdout);
}
