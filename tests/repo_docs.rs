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
