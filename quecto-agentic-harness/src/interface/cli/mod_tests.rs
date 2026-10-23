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
    // #2024 S5: the help text names every broker action and the flags the
    // runbooks tell an agent to pass.
    for needle in [
        "admission-broker run|status|reset|install-service|uninstall-service",
        "--directory <dir>",
        "--config <file>",
        "--accept-missing-ledger",
        "install-service|uninstall-service [--dry-run]",
    ] {
        assert!(help.stdout.contains(needle), "help text misses {needle}");
    }

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
        turn_session: crate::composition::bootstrap::agent_turn::build_turn_session,
        retention: crate::composition::sessions::build_retention_handles,
        fresh_session_identity: crate::composition::sessions::build_fresh_session_identity,
        configuration: crate::composition::configuration::build_configuration_handles,
        admission: crate::composition::admission::build_admission_handles,
        auth: crate::composition::bootstrap::auth::build_auth_handles,
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
        run_end_fleet: crate::composition::subagent_teardown::build_run_end_fleet,
        swarm_board: crate::composition::swarm::build_swarm_board_handles,
        swarm_board_wire: crate::composition::swarm::board_wire(),
        swarm_board_log: crate::composition::swarm::board_op_log,
        project_board: crate::composition::bootstrap::project_board::build_project_board_handles,
        project_board_recording: board::BoardToolRecordingBuilder(
            crate::composition::bootstrap::project_board::session_board_recording_decided,
        ),
        board_cli_recording: super::board::CliRecordingBuilder(
            crate::composition::bootstrap::project_board::cli_board_recording,
        ),
        claude_member: crate::composition::claude_member::build_claude_member_handles,
    }
}

/// A real run's context carries every builder `main` composed (a command
/// whose builder is missing refuses as uncomposed), the process's working
/// directory and stdin, and live output.
#[test]
fn a_real_runs_context_carries_every_composed_builder() {
    let ctx = composed_context(test_composition(), Some("/cfg.json".into()), true);
    assert_eq!(ctx.config_path, Some(PathBuf::from("/cfg.json")));
    assert_eq!(ctx.cwd, std::env::current_dir().ok());
    assert!(ctx.cwd.is_some());
    assert_eq!(ctx.stdin_is_tty, Some(true));
    assert_eq!(
        composed_context(test_composition(), None, false).stdin_is_tty,
        Some(false)
    );
    assert!(ctx.live_output);
    let composed = [
        (
            "web_fetch_tool_factory",
            ctx.web_fetch_tool_factory.is_some(),
        ),
        ("teardown_graph", ctx.teardown_graph.is_some()),
        ("kill_tool", ctx.kill_tool.is_some()),
        ("sessions", ctx.sessions.is_some()),
        ("retention", ctx.retention.is_some()),
        (
            "fresh_session_identity",
            ctx.fresh_session_identity.is_some(),
        ),
        ("configuration", ctx.configuration.is_some()),
        ("admission", ctx.admission.is_some()),
        ("auth", ctx.auth.is_some()),
        ("catalogue", ctx.catalogue.is_some()),
        ("provider_runtime", ctx.provider_runtime.is_some()),
        (
            "tool_policy_persistence",
            ctx.tool_policy_persistence.is_some(),
        ),
        ("container_configs", ctx.container_configs.is_some()),
        ("container_doctor", ctx.container_doctor.is_some()),
        ("environment_registry", ctx.environment_registry.is_some()),
        ("container_inventory", ctx.container_inventory.is_some()),
        ("container_init", ctx.container_init.is_some()),
        ("container_status", ctx.container_status.is_some()),
        ("run_end_fleet", ctx.run_end_fleet.is_some()),
        ("claude_member", ctx.claude_member.is_some()),
        ("swarm_board", ctx.swarm_board.is_some()),
        ("swarm_board_log", ctx.swarm_board_log.is_some()),
        (
            "project_board_recording",
            ctx.project_board_recording.is_some(),
        ),
        ("board_cli_recording", ctx.board_cli_recording.is_some()),
    ];
    let present: Vec<_> = composed
        .iter()
        .filter_map(|(name, present)| present.then_some(*name))
        .collect();
    let every: Vec<_> = composed.iter().map(|(name, _)| *name).collect();
    assert_eq!(present, every, "every builder is composed");
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

/// The test-support execution-state and ledger-hint probes are part of the
/// lib's public surface for the BDD suites; pin their shapes here too.
#[tokio::test]
async fn test_support_probes_report_live_and_completed_execution_state() {
    use crate::domain::agents::value_objects::agent::AgentProgressEvent;
    let started = AgentProgressEvent::ToolStarted {
        tool_call_id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    };
    let live = crate::interface::uds::test_support::live_execution_state_for_events(
        std::slice::from_ref(&started),
    );
    assert_eq!(live["execution"]["phase"], "runningTool");
    assert_eq!(live["execution"]["tools"]["started"], 1);
    let done = crate::interface::uds::test_support::completed_live_execution_state(&[started]);
    assert_eq!(done["execution"]["phase"], "idle");
    assert_eq!(done["messageCount"], 0);
    let hints = crate::interface::uds::test_support::ledger_hint_lines_for_turn_events(
        &[AgentProgressEvent::Done],
        &crate::interface::uds::controllers::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[])
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

/// From a linked git worktree with no overlay of its own, `status` names
/// the main worktree's overlay and says it is shared; trusting it there
/// approves that one file, and `config set` never writes it from here.
#[test]
fn a_linked_worktree_reports_and_applies_the_main_worktrees_overlay() {
    let layout = crate::infrastructure::config::main_worktree::tests::layout();
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"agents":{"defaults":{"model":"global-model"}}}"#,
    )
    .unwrap();
    let shared = layout.main.join(".quecto").join("config.json");
    std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
    std::fs::write(&shared, r#"{"agents":{"defaults":{"model":"main-model"}}}"#).unwrap();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        cwd: Some(layout.linked.clone()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        ..Default::default()
    };
    let out = run_with_output(args("status"), &ctx);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let line = |state: &str| {
        format!(
            "Overlay:   {} ({state}, from the main worktree)",
            shared.display()
        )
    };
    assert!(out.stdout.contains(&line("untrusted")), "{}", out.stdout);
    assert!(
        out.stdout.contains("Model:     global-model"),
        "{}",
        out.stdout
    );

    let trust = run_with_output(args("config trust"), &ctx);
    assert_eq!(trust.exit_code, 0, "{}", trust.stderr);
    let out = run_with_output(args("status"), &ctx);
    assert!(out.stdout.contains(&line("trusted")), "{}", out.stdout);
    assert!(
        out.stdout.contains("Model:     main-model"),
        "{}",
        out.stdout
    );

    let set = run_with_output(args("config set agents.defaults.model \"x\""), &ctx);
    assert_ne!(set.exit_code, 0, "{}", set.stdout);
    assert!(
        set.stderr.contains("shared with this linked worktree"),
        "{}",
        set.stderr
    );
    assert!(!layout.linked.join(".quecto").exists(), "nothing is copied");
}

/// #2660: `quecto help` lists every op `quecto board` takes, in its
/// command-line spelling.
#[test]
fn help_lists_every_board_op_the_command_line_takes() {
    let help = run_with_output(args("help"), &CliContext::default());
    for op in [
        "bootstrap",
        "list-ready",
        "list-takeable",
        "update-draft",
        "take-over",
        "link-pr",
        "unlink-pr",
        "clear-set-aside",
        "propose-plan",
        "edit-plan",
        "accept-plan",
        "discard-plan",
        "end-run",
    ] {
        assert!(help.stdout.contains(op), "{op} missing:\n{}", help.stdout);
    }
}
