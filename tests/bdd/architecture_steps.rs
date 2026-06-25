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
