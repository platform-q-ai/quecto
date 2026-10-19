//! #2580 (L22): `infrastructure/providers` takes the wiki's internal shape:
//! vendor codecs in `anthropic/`, `openai/` and `codex/`; vendor-neutral
//! HTTP/SSE and attempt mechanics in `transport/`; the provider chain in
//! `routing/`; catalogue source adapters in `catalogue/`. Every list here is
//! an allowlist: a folder, file or root entry not named fails.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The only sub-folders, each with its exhaustive production files (a
/// path relative to the folder; tests are the `*_tests.rs` files a listed
/// module declares).
const FOLDERS: &[(&str, &[&str])] = &[
    (
        "anthropic",
        &[
            "anthropic_sse.rs",
            "anthropic_sse_handler.rs",
            "anthropic_user_msg.rs",
            "claude_code.rs",
            "effort.rs",
            "mod.rs",
            "normalize.rs",
            "request_body.rs",
            "test_support.rs",
            "usage.rs",
        ],
    ),
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
            "codex_input.rs",
            "codex_replay.rs",
            "codex_request.rs",
            "codex_sse_handler.rs",
            "codex_sse_state.rs",
            "codex_test_support.rs",
            "mod.rs",
        ],
    ),
    (
        "transport",
        &[
            "admission_feedback.rs",
            "attempt_events.rs",
            "attempt_observer.rs",
            "attempt_output.rs",
            "attempt_profile.rs",
            "attempt_transport.rs",
            "attempt_transport_passive.rs",
            "diagnostic_sse.rs",
            "input_prefix.rs",
            "mod.rs",
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
    (
        "catalogue",
        &[
            "catalogue_discovery.rs",
            "catalogue_inputs.rs",
            "catalogue_refresh_inputs.rs",
            "catalogue_registry.rs",
            "mod.rs",
            "model_registry.rs",
            "model_registry_builtin_tables.rs",
            "model_registry_file.rs",
        ],
    ),
];

/// Every file the providers root may hold:
/// - its `mod.rs` and the module's own tests;
/// - the cross-vendor suites `mod.rs` declares (each drives every vendor
///   through the factory or the shared stream bounds, so none belongs to
///   one folder; #2580 records why each stays);
/// - `refresh_wiring.rs` and its tests until auth A3 (#2589) moves them to
///   `infrastructure/auth/`.
const ROOT_FILES: &[&str] = &[
    "mod.rs",
    "mod_tests.rs",
    "attempt_observation_tests.rs",
    "issue_996_efficiency_tests.rs",
    "output_cap_eof_tests.rs",
    "output_cap_provider_tests.rs",
    "providers_cov2_tests.rs",
    "sse_wire_cov_tests.rs",
    "stream_end_2249_tests.rs",
    "stream_idle_provider_tests.rs",
    "stream_progress_provider_tests.rs",
    "tool_schema_adapter_tests.rs",
    "refresh_wiring.rs",
    "refresh_wiring_tests.rs",
];

fn providers() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure/providers")
}

fn names(path: &Path) -> Vec<(String, bool)> {
    std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .map(|entry| {
            let entry = entry.expect("read an entry");
            let name = entry.file_name().into_string().expect("a UTF-8 name");
            let is_dir = entry.file_type().expect("an entry type").is_dir();
            (name, is_dir)
        })
        .collect()
}

/// Every production file under `folder`, relative to it, at any depth.
fn production_files(folder: &Path, prefix: &str, found: &mut BTreeSet<String>) {
    for (name, is_dir) in names(folder) {
        let relative = format!("{prefix}{name}");
        if is_dir {
            production_files(&folder.join(&name), &format!("{relative}/"), found);
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
            found.insert(relative);
        }
    }
}

#[test]
fn the_providers_root_holds_only_its_listed_files_and_folders() {
    let folders: BTreeSet<&str> = FOLDERS.iter().map(|(folder, _)| *folder).collect();
    let stray: Vec<String> = names(&providers())
        .into_iter()
        .filter(|(name, is_dir)| match is_dir {
            true => !folders.contains(name.as_str()),
            false => !ROOT_FILES.contains(&name.as_str()),
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        stray.is_empty(),
        "unlisted entries at the providers root: {stray:?}"
    );
}

#[test]
fn each_provider_folder_holds_exactly_its_listed_modules() {
    for (folder, expected) in FOLDERS {
        let mut found = BTreeSet::new();
        production_files(&providers().join(folder), "", &mut found);
        let expected: BTreeSet<String> = expected.iter().map(|file| file.to_string()).collect();
        assert_eq!(
            found, expected,
            "src/infrastructure/providers/{folder}/ production files"
        );
    }
}
