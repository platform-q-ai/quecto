//! #2580 (L22): `infrastructure/providers` takes the wiki's internal shape:
//! vendor codecs in `anthropic/`, `openai/` and `codex/`; vendor-neutral
//! HTTP/SSE and attempt mechanics in `transport/`; the provider chain in
//! `routing/`; catalogue source adapters in `catalogue/`. Every list here is
//! an allowlist: a folder, file or root entry not named fails.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The only sub-folders, each with its exhaustive production files. A
/// folder holds nothing else but the `*_tests.rs` files its production
/// modules declare: no sub-folder, no file of another kind.
const FOLDERS: &[(&str, &[&str])] = &[
    (
        "anthropic",
        &[
            "anthropic_input.rs",
            "anthropic_sse.rs",
            "anthropic_sse_handler.rs",
            "anthropic_user_msg.rs",
            "anthropic_wire.rs",
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
            "openai_stream_error.rs",
            "openai_wire.rs",
            "provider_images.rs",
            "usage.rs",
        ],
    ),
    (
        "codex",
        &[
            "codex_input.rs",
            "codex_input_prefix.rs",
            "codex_replay.rs",
            "codex_request.rs",
            "codex_sse_handler.rs",
            "codex_sse_state.rs",
            "codex_test_support.rs",
            "codex_wire.rs",
            "mod.rs",
        ],
    ),
    (
        "transport",
        &[
            "admission_feedback.rs",
            "attempt_events.rs",
            "attempt_observer.rs",
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
            "wire_dialect.rs",
        ],
    ),
    (
        "routing",
        &[
            "mod.rs",
            "model_refusal.rs",
            "provider_factory.rs",
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

/// Every file the providers root may hold, each of which must exist: only
/// its `mod.rs`, which declares the folders (#2679: provider construction
/// moved to `routing/provider_factory.rs` and the root test suites beside
/// the code they exercise).
const ROOT_FILES: &[&str] = &["mod.rs"];

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

/// The entries of one provider folder: its production modules, and the
/// test files declared from them — by a production module, or by a test
/// file declared in turn. Anything else is refused by name: a sub-folder,
/// a file that is not Rust, or a test file nothing in the folder declares.
struct FolderContents {
    production: BTreeSet<String>,
    refused: Vec<String>,
}

/// A declaration line without its visibility: `pub`, `pub(crate)`,
/// `pub(super)`, `pub(self)` or `pub(in …)`. A line with no visibility is
/// returned as it is.
fn without_visibility(line: &str) -> &str {
    match line.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => match rest.find(')') {
            Some(close) => rest[close + 1..].trim_start(),
            None => line,
        },
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => line,
    }
}

/// Whether `source` declares the module file `file`: by a `#[path]`
/// attribute line, or as a sibling `mod` of its stem, under any visibility.
/// A commented-out line declares nothing.
fn declares(source: &str, file: &str) -> bool {
    let stem = file.trim_end_matches(".rs");
    let path = format!("#[path = \"{file}\"]");
    let module = format!("mod {stem};");
    source
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with(&path) || without_visibility(line).starts_with(&module))
}

#[test]
fn a_module_is_declared_under_any_visibility_or_by_its_path() {
    let file = "x_tests.rs";
    for source in [
        "mod x_tests;",
        "pub mod x_tests;",
        "pub(crate) mod x_tests;",
        "pub(super) mod x_tests;",
        "pub(in crate::infrastructure::providers) mod x_tests;",
        "#[cfg(test)]\n    pub(crate) mod x_tests;",
        "#[path = \"x_tests.rs\"]\nmod tests;",
    ] {
        assert!(declares(source, file), "{source:?} declares {file}");
    }
}

#[test]
fn a_commented_or_other_declaration_declares_nothing() {
    let file = "x_tests.rs";
    for source in [
        "// #[path = \"x_tests.rs\"]\nmod tests;",
        "// mod x_tests;",
        "// pub(crate) mod x_tests;",
        "mod y_tests;",
        "pub(crate) mod x_tests_more;",
        "publish mod x_tests;",
        "pub(crate mod x_tests;",
    ] {
        assert!(!declares(source, file), "{source:?} declares nothing");
    }
}

fn folder_contents(folder: &Path) -> FolderContents {
    let entries = names(folder);
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).expect("read a module");
    let production: BTreeSet<String> = entries
        .iter()
        .filter(|(name, is_dir)| !is_dir && name.ends_with(".rs") && !name.ends_with("_tests.rs"))
        .map(|(name, _)| name.clone())
        .collect();
    let tests: Vec<&String> = entries
        .iter()
        .filter(|(name, is_dir)| !is_dir && name.ends_with("_tests.rs"))
        .map(|(name, _)| name)
        .collect();
    let mut sources: Vec<String> = production.iter().map(|name| read(name)).collect();
    let mut declared: BTreeSet<&String> = BTreeSet::new();
    loop {
        let found: Vec<&String> = tests
            .iter()
            .copied()
            .filter(|test| !declared.contains(test))
            .filter(|test| sources.iter().any(|source| declares(source, test)))
            .collect();
        if found.is_empty() {
            break;
        }
        for test in found {
            sources.push(read(test));
            declared.insert(test);
        }
    }
    let refused = entries
        .iter()
        .filter(
            |(name, is_dir)| match (is_dir, name.ends_with("_tests.rs")) {
                (true, _) => true,
                (false, true) => !declared.contains(name),
                (false, false) => !name.ends_with(".rs"),
            },
        )
        .map(|(name, _)| name.clone())
        .collect();
    FolderContents {
        production,
        refused,
    }
}

#[test]
fn the_providers_root_holds_only_its_listed_files_and_folders() {
    let folders: BTreeSet<&str> = FOLDERS.iter().map(|(folder, _)| *folder).collect();
    let entries = names(&providers());
    let stray: Vec<&String> = entries
        .iter()
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
    let missing: Vec<&&str> = ROOT_FILES
        .iter()
        .chain(&folders)
        .filter(|listed| !entries.iter().any(|(name, _)| name == **listed))
        .collect();
    assert!(
        missing.is_empty(),
        "listed providers-root entries that do not exist: {missing:?}"
    );
}

#[test]
fn each_provider_folder_holds_exactly_its_listed_modules_and_their_tests() {
    for (folder, expected) in FOLDERS {
        let contents = folder_contents(&providers().join(folder));
        let expected: BTreeSet<String> = expected.iter().map(|file| file.to_string()).collect();
        assert_eq!(
            contents.production, expected,
            "src/infrastructure/providers/{folder}/ production files"
        );
        assert!(
            contents.refused.is_empty(),
            "src/infrastructure/providers/{folder}/ holds entries no listed module accounts for: {:?}",
            contents.refused
        );
    }
}
