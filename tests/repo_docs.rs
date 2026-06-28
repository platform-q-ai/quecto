mod common;

use common::read_repo_file;

#[test]
fn readme_release_metadata_matches_workspace_package() {
    let readme = read_repo_file("README.md");
    let manifest = read_repo_file("Cargo.toml");
    let version = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = "))
        .map(|value| value.trim_matches('"'))
        .expect("root Cargo.toml should declare a package version");

    assert!(
        readme.contains(&format!("Current version: **{version}**")),
        "README current version should match root package version {version}"
    );
    assert!(
        !readme.contains("CHANGELOG.md"),
        "README should not link to a missing changelog"
    );
}

#[test]
fn readme_license_section_matches_private_repo_status() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("## License"));
    assert!(readme.contains("LicenseRef-Proprietary"));
    assert!(readme.contains("private repository"));
    assert!(!readme.contains("## License\n\nMIT"));
}

#[test]
fn readme_runtime_details_match_current_code() {
    let readme = read_repo_file("README.md");

    assert!(
        readme.contains("\"max_context_tokens\": 300000"),
        "README config example should use the current default max_context_tokens"
    );
    assert!(
        readme.contains("\"context_collapse_after_turns\": 50"),
        "README config example should document the current collapse threshold"
    );
    assert!(
        !readme.contains("\"max_context_tokens\": 1000000"),
        "README should not document the old 1M context default"
    );
    assert!(
        !readme.contains("QUECTO_* environment variables (including API keys) are stripped"),
        "bash currently inherits the process environment; README must not claim QUECTO_* env stripping"
    );
    assert!(
        !readme.contains("exec child environment is allowlisted by default"),
        "bash currently inherits the process environment; README must not claim env allowlisting"
    );
}

#[test]
fn readme_uds_protocol_lists_current_commands_and_events() {
    let readme = read_repo_file("README.md");

    assert!(readme.contains("get_subagents"));
    assert!(readme.contains("subagent_notification"));
    assert!(readme.contains("subagent_state_changed"));
    assert!(readme.contains("workflow_state"));
    assert!(
        !readme.contains("AgentCommand` enum (15 variants"),
        "README command count is stale; AgentCommand has get_subagents too"
    );
}

#[test]
fn agent_cmd_docs_match_tool_schema() {
    // #844: the docs' agent_cmd command surface must match the shipped tool
    // schema. After the #841 consolidation `get_messages_tail` is no longer a
    // first-class agent_cmd command — it survives only as a deprecated UDS
    // protocol alias of `get_messages` with an optional `count`.
    let src = read_repo_file("src/infrastructure/tools/agent_cmd.rs");
    let block = src
        .split("const SUPPORTED_COMMANDS: &[&str] = &[")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("agent_cmd.rs should declare SUPPORTED_COMMANDS");
    let commands: Vec<String> = block
        .split(',')
        .map(str::trim)
        // Only accept entries that round-trip as quoted string literals, so an
        // inline comment or a future entry containing a comma cannot produce a
        // bogus command name (this guard must outlive refactors).
        .filter_map(|entry| {
            entry
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect();

    assert!(
        commands.iter().any(|c| c == "get_messages"),
        "agent_cmd should support get_messages"
    );
    assert!(
        !commands.iter().any(|c| c == "get_messages_tail"),
        "get_messages_tail must not be a first-class agent_cmd command"
    );

    // README agent_cmd tool row must list exactly the supported commands and
    // must not advertise the removed get_messages_tail command.
    let readme = read_repo_file("README.md");
    let row = readme
        .lines()
        .find(|line| line.contains("Send commands to spawned UDS subagents"))
        .expect("README should document the agent_cmd tool");
    for cmd in &commands {
        assert!(
            row.contains(&format!("`{cmd}`")),
            "README agent_cmd row missing supported command `{cmd}`"
        );
    }
    assert!(
        !row.contains("get_messages_tail"),
        "README agent_cmd row must not list the removed get_messages_tail command"
    );

    // Converse: every backtick-wrapped token on the row must be a supported
    // command (no stale/removed extras), so a future drift can't slip through.
    // `count` is a documented parameter of `get_messages`, not a command.
    const ROW_NON_COMMAND_TOKENS: &[&str] = &["count"];
    let command_list = row
        .split_once("subagents:")
        .map(|(_, rest)| rest)
        .expect("README agent_cmd row should introduce the command list with `subagents:`");
    for token in command_list
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|t| !ROW_NON_COMMAND_TOKENS.contains(t))
    {
        assert!(
            commands.iter().any(|c| c == token),
            "README agent_cmd row lists `{token}` which is not a supported command"
        );
    }

    // docs/subagents.md documents the agent_cmd tool surface only — it must not
    // reference get_messages_tail at all; use get_messages with optional count.
    let subagents = read_repo_file("docs/subagents.md");
    assert!(
        !subagents.contains("get_messages_tail"),
        "docs/subagents.md must not reference get_messages_tail (use get_messages with count)"
    );
    assert!(
        subagents.contains("get_messages"),
        "docs/subagents.md should document get_messages"
    );
    // The AC requires documenting the optional `count` semantics (omit = all,
    // N = last N), not merely the command name.
    let subagents_lower = subagents.to_lowercase();
    assert!(
        subagents.contains("count"),
        "docs/subagents.md must document the optional `count` parameter of get_messages"
    );
    assert!(
        subagents_lower.contains("all") && subagents_lower.contains("last"),
        "docs/subagents.md must explain count semantics (omit = all, N = last N)"
    );

    // docs/sessions.md UDS inspection examples must not present get_messages_tail.
    let sessions = read_repo_file("docs/sessions.md");
    assert!(
        !sessions.contains("get_messages_tail"),
        "docs/sessions.md must not present get_messages_tail (use get_messages with count)"
    );

    // The AC requires the deprecated GetMessagesTail alias to be KEPT and
    // LABELLED where the UDS protocol is described — so these checks are
    // unconditional (a vacuous "if present" guard would let a future edit
    // silently drop the documented alias).
    let uds = read_repo_file("docs/uds-protocol.md");
    assert!(
        uds.contains("get_messages_tail"),
        "docs/uds-protocol.md must keep documenting the get_messages_tail alias"
    );
    assert!(
        uds.lines()
            .any(|l| l.contains("get_messages_tail") && l.to_lowercase().contains("deprecated")),
        "docs/uds-protocol.md must label the get_messages_tail alias deprecated"
    );
    // The README UDS protocol command table likewise keeps the alias labelled.
    let readme_alias_line = readme
        .lines()
        .find(|line| line.contains("`get_messages_tail`"))
        .expect("README must keep documenting the get_messages_tail UDS alias");
    assert!(
        readme_alias_line.to_lowercase().contains("deprecated"),
        "README must label the get_messages_tail UDS alias deprecated"
    );
}

#[test]
fn run_tui_prewarms_cold_binary_before_exec() {
    // #808: run-tui.sh must pay the cold-binary cost before launching the TUI,
    // POSIX-safely (no failure if `quecto` is not yet on PATH).
    let script = read_repo_file("scripts/run-tui.sh");
    let warm_idx = script
        .find("quecto --version")
        .expect("run-tui.sh must pre-warm `quecto --version`");
    let exec_idx = script
        .find("exec quecto-tui")
        .expect("run-tui.sh must exec quecto-tui");
    assert!(
        warm_idx < exec_idx,
        "the pre-warm must run before `exec quecto-tui`"
    );
    assert!(
        script.contains("|| true"),
        "the pre-warm must not fail the script if quecto is not yet on PATH (use `|| true`)"
    );
}

#[test]
fn readmes_document_cold_start_and_mitigations() {
    // #808: both READMEs must document the cold-binary first launch and both
    // mitigations (run-tui.sh pre-warm + the 30s direct-launch deadline).
    for path in ["README.md", "quecto-tui/README.md"] {
        let content = read_repo_file(path);
        let lower = content.to_lowercase();
        assert!(
            lower.contains("cold") && lower.contains("first launch"),
            "{path} must document the cold-binary first-launch slowdown"
        );
        assert!(
            lower.contains("run-tui.sh"),
            "{path} must mention run-tui.sh pre-warming the binary"
        );
        assert!(
            content.contains("30s") || lower.contains("30 second") || lower.contains("30-second"),
            "{path} must document the 30s direct-launch deadline"
        );
    }
}

#[test]
fn obsolete_development_planning_artifacts_are_removed() {
    const OBSOLETE_DOCS: &[&str] = &[
        "docs/quecto-mcp-prd.md",
        "docs/scenarios/architecture_contract_guardrails.md",
        "docs/scenarios/gpt54_1m_context.md",
        "docs/scenarios/gpt55_standard_default.md",
        "docs/scenarios/openai_oauth_gpt54_responses_cache.md",
        "docs/scenarios/resume_pruning_and_provider_resilience.md",
        "docs/scenarios/subagent_completion_notifications_passive.md",
        "docs/scenarios/subagent_notification_dedupe.md",
        "docs/scenarios/subagent_notification_provenance.md",
        "docs/scenarios/tui_context_window_display.md",
    ];

    for path in OBSOLETE_DOCS {
        assert!(
            !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(path)
                .exists(),
            "obsolete planning artifact should be removed: {path}"
        );
    }
}
