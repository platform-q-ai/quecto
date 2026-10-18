//! #2580 (L22): `infrastructure/providers` takes the wiki's internal shape:
//! vendor codecs in `anthropic/`, `openai/` and `codex/`; vendor-neutral
//! HTTP/SSE and attempt mechanics in `transport/`; the provider chain in
//! `routing/`; catalogue source adapters in `catalogue/`.
use std::path::{Path, PathBuf};

/// Each folder's production modules, by file (a folder module is `mod.rs`).
const PLACEMENTS: &[(&str, &[&str])] = &[
    (
        "openai",
        &[
            "mod.rs",
            "openai_endpoint_router.rs",
            "openai_images.rs",
            "openai_request.rs",
            "openai_sse.rs",
            "openai_sse_parser.rs",
            "provider_images.rs",
            "usage.rs",
        ],
    ),
    (
        "codex",
        &[
            "mod.rs",
            "codex_input.rs",
            "codex_replay.rs",
            "codex_request.rs",
            "codex_sse_handler.rs",
            "codex_sse_state.rs",
            "codex_test_support.rs",
        ],
    ),
    (
        "transport",
        &[
            "mod.rs",
            "admission_feedback.rs",
            "attempt_events.rs",
            "attempt_observer.rs",
            "attempt_output.rs",
            "attempt_profile.rs",
            "attempt_transport.rs",
            "attempt_transport_passive.rs",
            "diagnostic_sse.rs",
            "input_prefix.rs",
            "single_attempt_client.rs",
            "sse_common.rs",
            "sse_end.rs",
            "sse_limits.rs",
            "sse_progress.rs",
            "stream_idle.rs",
            "transport_diagnostics.rs",
        ],
    ),
    (
        "routing",
        &[
            "mod.rs",
            "model_refusal.rs",
            "provider_runtime.rs",
            "provider_runtime_admission.rs",
            "refreshable.rs",
            "retry.rs",
            "router.rs",
        ],
    ),
];

/// The production files the providers root may hold: its own `mod.rs`, and
/// `refresh_wiring.rs` until auth A3 (#2589) moves it to `infrastructure/auth/`.
const ROOT_FILES: &[&str] = &["mod.rs", "refresh_wiring.rs"];

fn providers() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure/providers")
}

#[test]
fn provider_modules_live_in_their_folders() {
    let missing: Vec<String> = PLACEMENTS
        .iter()
        .flat_map(|(folder, files)| files.iter().map(move |file| format!("{folder}/{file}")))
        .filter(|path| !providers().join(path).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "expected in src/infrastructure/providers/: {missing:?}"
    );
}

#[test]
fn the_providers_root_holds_only_its_module_file() {
    let entries = std::fs::read_dir(providers()).expect("read the providers folder");
    let stray: Vec<String> = entries
        .map(|entry| {
            entry
                .expect("read an entry")
                .file_name()
                .into_string()
                .expect("UTF-8 name")
        })
        .filter(|name| name.ends_with(".rs") && !name.ends_with("_tests.rs"))
        .filter(|name| !ROOT_FILES.contains(&name.as_str()))
        .collect();
    assert!(
        stray.is_empty(),
        "flat production files at the providers root: {stray:?}"
    );
}
