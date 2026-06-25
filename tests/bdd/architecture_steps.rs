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

#[then("the pre-push script should lint with --workspace flag")]
fn then_pre_push_lints_workspace(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("scripts/pre-push.sh").expect("read scripts/pre-push.sh");
    // Find the actual clippy invocation line (not echo/comment lines).
    let has_workspace_clippy = content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("cargo clippy") && trimmed.contains("--workspace")
    });
    assert!(
        has_workspace_clippy,
        "pre-push.sh must invoke `cargo clippy --workspace` to lint all workspace members including quecto-tui"
    );
}

#[then("the pre-push script should run the mocked e2e suite by default")]
fn then_pre_push_runs_mock_e2e(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("scripts/pre-push.sh").expect("read scripts/pre-push.sh");

    // (a) The default e2e lane must invoke the zero-cost mocked copy (tagged
    //     @mock-llm) via run-bdd-shards.sh.
    let runs_mock = content.lines().any(|line| {
        let t = line.trim();
        t.contains("run-bdd-shards.sh") && t.contains("mock-llm")
    }) || content.contains("--tag \"mock-llm\"");
    assert!(
        runs_mock,
        "pre-push.sh must run the zero-cost mocked e2e suite (@mock-llm) by default"
    );

    // (b) The paid real-LLM lane must NOT sit on the default path: every
    //     `--real-llm` invocation must appear AFTER the explicit opt-in guard
    //     (`QUECTO_RUN_REAL_LLM`), never unconditionally. Locating it by byte
    //     offset proves the opt-in guard precedes (gates) the paid invocation
    //     rather than merely co-existing in the file.
    let optin_at = content.find("QUECTO_RUN_REAL_LLM");
    for (idx, _) in content.match_indices("--real-llm") {
        match optin_at {
            Some(guard) => assert!(
                idx > guard,
                "pre-push.sh runs `--real-llm` (offset {idx}) before/without the \
                 QUECTO_RUN_REAL_LLM opt-in guard (offset {guard}): the paid suite must \
                 not be on the default push path"
            ),
            None => panic!(
                "pre-push.sh invokes `--real-llm` with no QUECTO_RUN_REAL_LLM opt-in guard: \
                 the paid suite must not be on the default push path"
            ),
        }
    }
}

#[then("the pre-push script should not probe for a provider key to auto-run the paid suite")]
fn then_pre_push_no_key_autorun(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("scripts/pre-push.sh").expect("read scripts/pre-push.sh");
    // Behaviour under test: a `.env` provider key must NOT auto-trigger the paid
    // suite. The old design probed key presence (`OPENAI_API_KEY` /
    // `QUECTO_PROVIDERS_OPENAI_API_KEY`) to fold a `REAL_LLM_STATE=run` decision
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

#[then("the pre-push script should gate the live real-LLM suite behind an explicit opt-in flag")]
fn then_pre_push_real_llm_optin(_world: &mut QuectoWorld) {
    let content = std::fs::read_to_string("scripts/pre-push.sh").expect("read scripts/pre-push.sh");
    // The live (paid) suite must remain runnable on demand via an explicit,
    // documented opt-in env flag.
    assert!(
        content.contains("QUECTO_RUN_REAL_LLM"),
        "pre-push.sh must gate the live real-LLM suite behind an explicit opt-in (QUECTO_RUN_REAL_LLM)"
    );
}

#[then("the mocked e2e suite should preserve the real-LLM behavioural coverage")]
fn then_mock_e2e_preserves_coverage(_world: &mut QuectoWorld) {
    // The mocked copy is a CONSOLIDATED suite (per docs/real-llm-mocking-plan.md),
    // not a 1:1 file-per-file mirror: PR #780's WireMock helpers let one
    // deterministic feature cover the behaviours many prompt-dependent @real-llm
    // scenarios exercise. We therefore assert behavioural-capability parity
    // (no net coverage loss), not filename symmetry.
    let mut mock = String::new();
    let mut mock_files = Vec::new();
    for entry in std::fs::read_dir("tests/features").expect("read tests/features") {
        let path = entry.expect("dir entry").path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.starts_with("e2e_mock_llm") && name.ends_with(".feature") {
            mock.push_str(&std::fs::read_to_string(&path).expect("read mock feature"));
            mock.push('\n');
            mock_files.push(name);
        }
    }
    assert!(
        !mock_files.is_empty(),
        "no e2e_mock_llm*.feature files found — the zero-cost mocked e2e copy is missing"
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

    // Behavioural capabilities the @real-llm suite asserts that the mocked copy
    // must reproduce deterministically. Each is keyed off a genuine behavioural
    // marker (a tool-call schema, a multi-step loop, a session-memory scenario),
    // not an incidental string, so a token/empty mock file cannot pass.
    let required: [(&str, &str); 8] = [
        (
            "plain text / token response",
            "the mock LLM returns a text response",
        ),
        ("file write tool-call", r#"tool call for "write""#),
        ("file read tool-call", r#"tool call for "read""#),
        ("file edit tool-call", r#"tool call for "edit""#),
        ("shell exec tool-call", r#"tool call for "bash""#),
        ("multi-step tool-call loop", "tool call sequence"),
        ("system-prompt influence", "--system"),
        (
            "session memory across turns",
            "Scenario: Mocked agent remembers context across session turns",
        ),
    ];
    let mut missing = Vec::new();
    for (label, marker) in required {
        if !mock.contains(marker) {
            missing.push(label);
        }
    }
    assert!(
        missing.is_empty(),
        "mocked e2e suite ({mock_files:?}) is missing behavioural coverage present in the \
         @real-llm suite: {missing:?}"
    );
}
