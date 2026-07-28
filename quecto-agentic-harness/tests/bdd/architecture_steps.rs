use super::*;

#[then(expr = "the source file {string} should contain {string}")]
fn then_source_file_contains(_world: &mut QuectoWorld, path: String, needle: String) {
    let content = std::fs::read_to_string(&path).expect("read source file");
    assert!(
        content.contains(&needle),
        "expected '{}' to contain '{}', but it did not",
        path,
        needle
    );
}

#[then(expr = "the source file {string} should not contain {string}")]
fn then_source_file_not_contains(_world: &mut QuectoWorld, path: String, needle: String) {
    let content = std::fs::read_to_string(&path).expect("read source file");
    assert!(
        !content.contains(&needle),
        "expected '{}' to not contain '{}', but it did",
        path,
        needle
    );
}

#[then("the application source should not contain runtime I/O patterns")]
fn then_application_has_no_runtime_io(_world: &mut QuectoWorld) {
    let mut files = Vec::new();
    collect_rs_files(Path::new("src/application"), &mut files);

    let forbidden = [
        "std::fs::",
        "tokio::fs::",
        "std::env::",
        "dirs::",
        ".exists(",
    ];

    for file_content in &files {
        let (file_path, _) = file_content
            .split_once(":\n")
            .expect("split path from file content");

        for line in file_content.lines().skip(1) {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]" {
                break;
            }
            if trimmed.starts_with("//") {
                continue;
            }

            for pattern in &forbidden {
                assert!(
                    !trimmed.contains(pattern),
                    "application runtime I/O pattern found in {}: {}",
                    file_path,
                    trimmed
                );
            }
        }
    }
}

fn collect_rs_files(dir: &Path, files: &mut Vec<String>) {
    if !dir.exists() {
        return;
    }

    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = std::fs::read_to_string(&path).expect("read file");
            files.push(format!("{}:\n{}", path.display(), content));
        }
    }
}

#[then("authoritative CI should lint with --workspace flag")]
fn then_ci_lints_workspace(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("../.github/workflows/ci.yml")
        .expect("read ../.github/workflows/ci.yml");
    // Find the actual clippy invocation line (not echo/comment lines).
    let has_workspace_clippy = content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("- run: cargo clippy") && trimmed.contains("--workspace")
    });
    assert!(
        has_workspace_clippy,
        "authoritative CI must invoke `cargo clippy --workspace` to lint all workspace members"
    );
}

#[then("authoritative CI should run the mocked e2e suite")]
fn then_ci_runs_mock_e2e(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("../.github/workflows/ci.yml")
        .expect("read ../.github/workflows/ci.yml");
    assert!(
        content.lines().any(|line| {
            line.contains("run-bdd-shards.sh")
                && line.contains("mock-llm-bdd")
                && line.contains("--tag mock-llm")
        }),
        "authoritative CI must run the zero-cost mocked e2e suite"
    );
}

#[then("the pre-push script should not probe for a provider key to auto-run the paid suite")]
fn then_pre_push_no_key_autorun(_world: &mut QuectoWorld) {
    let content =
        std::fs::read_to_string("../scripts/pre-push.sh").expect("read ../scripts/pre-push.sh");
    // Behaviour under test: a `.env` provider key must NOT auto-trigger the paid
    // suite. The old design probed key presence (`OPENAI_API_KEY`) to fold a
    // `REAL_LLM_STATE=run` decision
    // and then auto-run real-LLM. Assert that whole probe mechanism is gone:
    // pre-push.sh no longer inspects a provider key at all to decide what runs.
    // Keying off the absence of key inspection (not a `="run"` literal) means a
    // renamed/rewritten probe can't slip through as a false green.
    for needle in ["OPENAI_API_KEY", "REAL_LLM_STATE"] {
        assert!(
            !content.contains(needle),
            "pre-push.sh must not inspect `{needle}` to select the e2e lane — a .env key \
             must never auto-enable the paid real-LLM suite (gate it behind QUECTO_RUN_REAL_LLM \
             and let the suite/shards script load credentials)"
        );
    }
}

#[then("the mocked e2e suite should cover the curated real-LLM capability checklist")]
fn then_mock_e2e_preserves_coverage(_world: &mut QuectoWorld) {
    // The mocked copy is a CONSOLIDATED suite (per docs/real-llm-mocking-plan.md),
    // not a 1:1 file-per-file mirror: PR #780's WireMock helpers let one
    // deterministic feature cover the behaviours many prompt-dependent @real-llm
    // scenarios exercise. The two suites also use different phrasings (live NL
    // prompts vs WireMock helper steps), so a literal scenario-by-scenario diff
    // is impossible. This guard is therefore a HAND-MAINTAINED behavioural-
    // capability checklist, not an automatic drift detector: each capability is
    // anchored to a marker in BOTH the live @real-llm suite and the mocked copy,
    // so dropping a capability from either side trips the check. When a NEW
    // @real-llm capability is added, extend `required` below.
    let read_features = |prefix: &str| -> (String, Vec<String>) {
        let mut joined = String::new();
        let mut files = Vec::new();
        for entry in std::fs::read_dir("tests/features").expect("read tests/features") {
            let path = entry.expect("dir entry").path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name.starts_with(prefix) && name.ends_with(".feature") {
                joined.push_str(&std::fs::read_to_string(&path).expect("read feature"));
                joined.push('\n');
                files.push(name);
            }
        }
        (joined, files)
    };

    let (mock, mock_files) = read_features("e2e_mock_llm");
    let (real_raw, real_files) = read_features("e2e_real_llm");
    let real = real_raw.to_lowercase();
    assert!(
        !mock_files.is_empty(),
        "no e2e_mock_llm*.feature files found — the zero-cost mocked e2e copy is missing"
    );
    assert!(
        !real_files.is_empty(),
        "no e2e_real_llm*.feature files found — the live suite this checklist mirrors is missing"
    );

    // Every mocked e2e scenario must carry the @mock-llm tag so the pre-push
    // mock lane (QUECTO_TAG=mock-llm) actually selects it, and so it never gets
    // mistaken for / counted against the live @real-llm lane. Inspect tag LINES
    // (lines starting with `@`), not prose, so a doc-comment mentioning
    // "@real-llm" doesn't trip the check.
    let tag_lines: Vec<&str> = mock
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('@'))
        .collect();
    assert!(
        tag_lines
            .iter()
            .any(|l| l.split_whitespace().any(|t| t == "@mock-llm")),
        "mocked e2e features must be tagged @mock-llm so the default pre-push lane selects them"
    );
    assert!(
        !tag_lines
            .iter()
            .any(|l| l.split_whitespace().any(|t| t == "@real-llm")),
        "the mocked e2e copy must not carry @real-llm tags (that would make it paid/skipped)"
    );

    // Each capability is anchored on BOTH sides: `real_marker` (matched
    // case-insensitively against the live suite) proves the live suite still
    // exercises the behaviour, and `mock_marker` proves the deterministic copy
    // reproduces it. A capability silently dropped from either suite fails here.
    let required: [(&str, &str, &str); 8] = [
        (
            "plain text / token response",
            "token",
            "the mock LLM returns a text response",
        ),
        ("file write tool-call", "write", r#"tool call for "write""#),
        ("file read tool-call", "read", r#"tool call for "read""#),
        ("file edit tool-call", "edit", r#"tool call for "edit""#),
        ("shell exec tool-call", "shell", r#"tool call for "bash""#),
        ("multi-step tool-call loop", "multi", "tool call sequence"),
        ("system-prompt influence", "--system", "--system"),
        (
            "session memory across turns",
            "session",
            "remembers context across session turns",
        ),
    ];
    let mut missing_real = Vec::new();
    let mut missing_mock = Vec::new();
    for (label, real_marker, mock_marker) in required {
        if !real.contains(real_marker) {
            missing_real.push(label);
        }
        if !mock.contains(mock_marker) {
            missing_mock.push(label);
        }
    }
    assert!(
        missing_real.is_empty(),
        "the live @real-llm suite ({real_files:?}) no longer exercises checklisted \
         capabilities: {missing_real:?} — update the checklist if the behaviour was \
         intentionally removed"
    );
    assert!(
        missing_mock.is_empty(),
        "mocked e2e suite ({mock_files:?}) is missing behavioural coverage present in the \
         @real-llm suite: {missing_mock:?}"
    );
}

#[then("the retired live behavioral e2e suite should be tagged manual-only")]
fn then_live_behavioral_suite_is_manual_only(_world: &mut QuectoWorld) {
    let mut real_files = Vec::new();
    let mut old_tag_locations = Vec::new();
    let mut missing_manual_tag_locations = Vec::new();
    let mut missing_mock_tag_locations = Vec::new();

    for entry in std::fs::read_dir("tests/features").expect("read tests/features") {
        let path = entry.expect("dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("e2e_real_llm") || !name.ends_with(".feature") {
            continue;
        }

        real_files.push(name.to_string());
        let content = std::fs::read_to_string(&path).expect("read feature");
        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if !trimmed.starts_with('@') {
                continue;
            }
            let next_non_empty = lines
                .iter()
                .skip(idx + 1)
                .map(|l| l.trim())
                .find(|l| !l.is_empty());
            let Some(next_non_empty) = next_non_empty else {
                continue;
            };
            if !next_non_empty.starts_with("Scenario") {
                continue;
            }

            let tags: Vec<&str> = trimmed.split_whitespace().collect();
            if tags.contains(&"@real-llm") {
                old_tag_locations.push(format!("{name}:{}", idx + 1));
            }
            if !tags.contains(&"@manual-real-llm") {
                missing_manual_tag_locations.push(format!("{name}:{}", idx + 1));
            }
            if !tags.contains(&"@mock-llm") {
                missing_mock_tag_locations.push(format!("{name}:{}", idx + 1));
            }
        }
    }

    assert!(
        !real_files.is_empty(),
        "no e2e_real_llm*.feature files found to classify as manual-only"
    );
    assert!(
        old_tag_locations.is_empty(),
        "retired live behavioral scenarios must use @manual-real-llm, not @real-llm: {old_tag_locations:?}"
    );
    assert!(
        missing_manual_tag_locations.is_empty(),
        "retired live behavioral tag lines must include @manual-real-llm: {missing_manual_tag_locations:?}"
    );
    assert!(
        missing_mock_tag_locations.is_empty(),
        "retired live behavioral tag lines must also include @mock-llm for the zero-cost mirror: {missing_mock_tag_locations:?}"
    );
}

#[then("provider smoke scenarios should not be tagged as mocked or manual real LLM")]
fn then_provider_smoke_is_not_automocked(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("tests/features/provider_smoke.feature")
        .expect("read provider_smoke.feature");
    let mut bad_tag_lines = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with('@') || !trimmed.split_whitespace().any(|t| t == "@provider-smoke")
        {
            continue;
        }
        let has_automock_tag = trimmed
            .split_whitespace()
            .any(|t| t == "@mock-llm" || t == "@manual-real-llm");
        if has_automock_tag {
            bad_tag_lines.push(format!("provider_smoke.feature:{}", idx + 1));
        }
    }

    assert!(
        bad_tag_lines.is_empty(),
        "provider smoke scenarios must stay out of @mock-llm/@manual-real-llm automock lanes: {bad_tag_lines:?}"
    );
}

#[given("the harness normal build configuration is inspected")]
fn given_harness_normal_build_configuration_inspected(_world: &mut QuectoWorld) {}

#[when("retired installer support is classified")]
fn when_retired_installer_support_classified(_world: &mut QuectoWorld) {}

#[then("normal builds should exclude the retired installer")]
fn then_normal_builds_exclude_retired_installer(_world: &mut QuectoWorld) {
    assert!(
        !Path::new("src/infrastructure/tools/ensure_tool.rs").exists(),
        "retired tool installer source should not remain in normal builds"
    );

    let tools_mod = std::fs::read_to_string("src/infrastructure/tools/mod.rs")
        .expect("read tools module declarations");
    assert!(
        !tools_mod
            .lines()
            .any(|line| line.trim() == "pub mod ensure_tool;"),
        "normal builds should not declare the retired tool installer module"
    );
}

#[then("normal builds should exclude its archive dependencies")]
fn then_normal_builds_exclude_retired_installer_archive_dependencies(_world: &mut QuectoWorld) {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("read harness Cargo.toml");
    let normal_dependencies = manifest_section(&manifest, "dependencies");
    for package in ["flate2", "tar"] {
        assert!(
            !manifest_section_contains_dependency(normal_dependencies, package),
            "normal builds must not include the retired installer archive dependency `{package}`"
        );
    }
}

#[given("the harness search tools are inspected")]
fn given_harness_search_tools_inspected(_world: &mut QuectoWorld) {}

#[when("their missing-binary handling is checked")]
fn when_missing_binary_handling_checked(_world: &mut QuectoWorld) {}

#[given("the harness dependency manifest is inspected")]
fn given_harness_dependency_manifest_inspected(_world: &mut QuectoWorld) {}

#[when("platform-specific dependencies are classified")]
fn when_platform_specific_dependencies_classified(_world: &mut QuectoWorld) {}

#[then("each search tool should keep direct install guidance")]
fn then_each_search_tool_keeps_direct_install_guidance(_world: &mut QuectoWorld) {
    let tmp = TempDir::new().expect("create search-tool workspace");
    let workspace = Arc::new(tmp.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some(workspace.as_ref().clone()), true));

    let grep_tool = quecto::infrastructure::tools::grep::GrepTool::with_rg_binary(
        workspace.clone(),
        sandbox.clone(),
        "definitely-missing-rg-for-dependency-hygiene".to_string(),
    );
    let grep_error = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(grep_tool.execute(r#"{"pattern":"needle"}"#))
        .expect_err("missing rg should surface as a domain tool error");
    let grep_message = grep_error.to_string();
    assert!(
        grep_message.contains("rg not found on PATH")
            && grep_message.contains("github.com/BurntSushi/ripgrep#installation"),
        "grep should keep user-facing ripgrep installation guidance when rg is missing, got: {}",
        grep_message
    );

    let find_tool = quecto::infrastructure::tools::find::FindTool::with_fd_binary(
        workspace,
        sandbox,
        "definitely-missing-fd-for-dependency-hygiene".to_string(),
    );
    let find_error = tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(find_tool.execute(r#"{"pattern":"*.rs"}"#))
        .expect_err("missing fd should surface as a domain tool error");
    let find_message = find_error.to_string();
    assert!(
        find_message.contains("fd not found on PATH")
            && find_message.contains("github.com/sharkdp/fd#installation"),
        "find should keep user-facing fd installation guidance when fd is missing, got: {}",
        find_message
    );
}

#[then("text normalization should be scoped to macOS builds")]
fn then_text_normalization_scoped_to_macos_builds(_world: &mut QuectoWorld) {
    let manifest = std::fs::read_to_string("Cargo.toml").expect("read harness Cargo.toml");
    let normal_dependencies = manifest_section(&manifest, "dependencies");
    assert!(
        !manifest_section_contains_dependency(normal_dependencies, "unicode-normalization"),
        "unicode-normalization should not be an unconditional normal-build dependency"
    );

    let macos_dependencies = manifest_section(
        &manifest,
        "target.'cfg(target_os = \"macos\")'.dependencies",
    );
    assert!(
        manifest_section_contains_dependency(macos_dependencies, "unicode-normalization"),
        "unicode-normalization should remain available for macOS-only path normalization"
    );
}

fn manifest_section<'a>(manifest: &'a str, section: &str) -> &'a str {
    let header = format!("[{section}]");
    let Some(start) = manifest.find(&header) else {
        return "";
    };
    let after_header = &manifest[start + header.len()..];
    let end = after_header.find("\n[").unwrap_or(after_header.len());
    &after_header[..end]
}

fn manifest_section_contains_dependency(section: &str, dependency: &str) -> bool {
    section.lines().any(|line| {
        let trimmed = line.trim_start();
        !trimmed.starts_with('#')
            && (trimmed.starts_with(&format!("{dependency} ="))
                || trimmed.starts_with(&format!("{dependency}=")))
    })
}
