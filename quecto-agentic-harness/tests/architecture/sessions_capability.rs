//! The sessions capability (#1968, D1 #1970): exact-inventory ratchets for
//! the plural capability, its one construction site, and the single owner
//! of the flat storage layout. Affirmative throughout: each check states the
//! set that is allowed and asserts the observed set equals it, so an unknown
//! future owner fails by not being listed.
//!
//! - R1: the canonical files of the capability exist; the singular
//!   `application/session` path, every singular import and any alias
//!   re-export are absent.
//! - R5: the sessions use case, its controller and the file store are
//!   constructed only in `composition/sessions.rs`; interface and
//!   infrastructure hold injected handles.
//! - R7: exactly one infrastructure `FlatSessionLayout` joins `sessions`,
//!   calls the sanitizer and forms the `.json`/`.owner`/`spill.jsonl`
//!   names; the layout is created at an exact, non-growing set of sites.
//! - Retirement: no interface production code lists the store directly.
//! - Ceilings: the per-owner line ceilings are non-empty and decrease-only.

use std::collections::BTreeSet;
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// The files the plural capability is made of (R1).
const CANONICAL_FILES: &[&str] = &[
    "src/application/sessions/mod.rs",
    "src/application/sessions/ports.rs",
    "src/application/sessions/dto/mod.rs",
    "src/application/sessions/use_cases/mod.rs",
    "src/application/sessions/use_cases/list_sessions.rs",
    "src/composition/sessions.rs",
    "src/interface/cli/uds_session_handles.rs",
    "src/interface/uds/sessions/controller.rs",
    "src/infrastructure/persistence/session_layout.rs",
    "src/domain/session_identity.rs",
];

/// The ports of the capability, each with its contract suite (R2 subset).
const SESSION_PORTS: &[(&str, &str)] = &[
    ("SessionStore", "tests/contracts/session_store.rs"),
    (
        "ContextSpillStore",
        "tests/contracts/context_spill_store.rs",
    ),
];

/// Where a sessions graph node may be constructed (R5): composition only.
const CONSTRUCTION_SITE: &str = "src/composition/sessions.rs";
const COMPOSED_CONSTRUCTORS: &[&str] = &[
    "ListSessions::new(",
    "ListSessionsController::new(",
    "FileSessionStore::new(",
];

/// The exact sites that create a `FlatSessionLayout` (R7). Decrease-only:
/// the two interface sites (the ephemeral spill scrub and the tool
/// runtime's spill store) retire when the retention adapter moves behind
/// the sessions composition (D9); nothing may join this list.
const LAYOUT_CREATION_SITES: &[&str] = &[
    "src/composition/sessions.rs",
    "src/interface/shared.rs",
    "src/interface/tool_runtime.rs",
];

/// The one file of the persistence tree that forms session paths, and the
/// exact callers of the shared filename sanitizer (R7). `audit_log.rs`
/// names its own audit files with the same sanitizer; it forms no session
/// path.
const LAYOUT_OWNER: &str = "src/infrastructure/persistence/session_layout.rs";
const SANITIZER_CALLERS: &[&str] = &[
    "src/infrastructure/persistence/audit_log.rs",
    "src/infrastructure/persistence/session_layout.rs",
];

/// Decrease-only line ceilings of the list/owner files (file, ceiling).
/// Lower a ceiling when a file shrinks; never raise or remove one to pass.
const LINE_CEILINGS: &[(&str, usize)] = &[
    ("src/application/sessions/ports.rs", 135),
    ("src/application/sessions/use_cases/list_sessions.rs", 46),
    ("src/composition/sessions.rs", 38),
    ("src/domain/session_identity.rs", 133),
    ("src/infrastructure/persistence/context_spill.rs", 376),
    ("src/infrastructure/persistence/session_layout.rs", 83),
    ("src/infrastructure/persistence/session_ownership.rs", 229),
    ("src/infrastructure/persistence/session_store.rs", 687),
    ("src/infrastructure/persistence/session_store_list.rs", 99),
    ("src/interface/cli/uds_dispatch_query.rs", 217),
    ("src/interface/uds/sessions/controller.rs", 36),
];

fn files_under(root: &str) -> Vec<String> {
    let mut files = Vec::new();
    walk(Path::new(root), &mut files);
    files
}

fn files_containing(files: &[String], needle: &str) -> BTreeSet<String> {
    files
        .iter()
        .filter(|path| std::fs::read_to_string(path).is_ok_and(|source| source.contains(needle)))
        .cloned()
        .collect()
}

fn production_files_calling(needle: &str) -> BTreeSet<String> {
    production_files()
        .into_iter()
        .filter(|path| {
            production_code(path)
                .iter()
                .any(|(_, line)| line.contains(needle))
        })
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn plural_capability_files_exist_and_the_singular_path_is_gone() {
    for file in CANONICAL_FILES {
        assert!(Path::new(file).is_file(), "missing canonical file {file}");
    }
    assert!(
        !Path::new("src/application/session").exists(),
        "the singular application/session path must not survive the rename"
    );
    let declarations: Vec<_> = production_code("src/application/mod.rs")
        .into_iter()
        .filter(|(_, line)| line.trim() == "pub mod sessions;" || line.trim() == "pub mod session;")
        .map(|(_, line)| line.trim().to_string())
        .collect();
    assert_eq!(
        declarations,
        vec!["pub mod sessions;".to_string()],
        "application/mod.rs exposes `sessions` exactly once and no singular `session`"
    );
}

#[test]
fn no_singular_import_alias_or_re_export_survives() {
    let mut files = files_under("src");
    files.extend(files_under("tests"));
    files.extend(files_under("docs"));
    files.extend(files_under("../docs"));
    // Built by concatenation so this ratchet does not match its own needle.
    let singular_import = ["application::", "session::"].concat();
    let singular_path = ["application/", "session/"].concat();
    let singular_paths: BTreeSet<String> = files
        .iter()
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|source| {
                source
                    .lines()
                    .any(|line| line.contains(&singular_import) || line.contains(&singular_path))
            })
        })
        .cloned()
        .collect();
    assert!(
        singular_paths.is_empty(),
        "singular `application::session` paths remain in {singular_paths:?}"
    );
    let aliases = files_containing(&files_under("src"), "sessions as session");
    assert!(
        aliases.is_empty(),
        "no alias may forward the plural capability under the singular name: {aliases:?}"
    );
    let re_exports: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|path| {
            production_code(path).iter().any(|(_, line)| {
                line.trim_start()
                    .starts_with("pub use crate::application::sessions")
            })
        })
        .collect();
    assert!(
        re_exports.is_empty(),
        "no module re-exports the sessions capability: {re_exports:?}"
    );
}

#[test]
fn ports_are_declared_only_in_the_capability_ports_file_and_contracted() {
    let sessions_files: Vec<String> = files_under("src/application/sessions")
        .into_iter()
        .filter(|path| !path.ends_with("_tests.rs"))
        .collect();
    let mut declared = BTreeSet::new();
    for path in &sessions_files {
        for (_, line) in production_code(path) {
            if let Some(rest) = line.trim_start().strip_prefix("pub trait ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                assert_eq!(
                    path, "src/application/sessions/ports.rs",
                    "port {name} must be declared in the capability's ports.rs, found in {path}"
                );
                declared.insert(name);
            }
        }
    }
    let expected: BTreeSet<String> = SESSION_PORTS
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(declared, expected, "the sessions port inventory changed");
    for (name, contract) in SESSION_PORTS {
        assert!(
            Path::new(contract).is_file(),
            "port {name} needs its contract {contract}"
        );
        let registry = std::fs::read_to_string("tests/contracts.rs").unwrap();
        let module = contract.trim_start_matches("tests/");
        assert!(
            registry.contains(&format!("#[path = \"{module}\"]")),
            "tests/contracts.rs must register {module}"
        );
    }
    for extra in [
        "tests/contracts/session_layout.rs",
        "tests/contracts/session_list_scale.rs",
    ] {
        assert!(Path::new(extra).is_file(), "missing {extra}");
    }
}

#[test]
fn sessions_graph_nodes_are_constructed_only_in_composition() {
    for constructor in COMPOSED_CONSTRUCTORS {
        let sites = production_files_calling(constructor);
        assert_eq!(
            sites,
            set(&[CONSTRUCTION_SITE]),
            "{constructor} may appear only in {CONSTRUCTION_SITE}"
        );
    }
    let layout_sites = production_files_calling("FlatSessionLayout::new(");
    assert_eq!(
        layout_sites,
        set(LAYOUT_CREATION_SITES),
        "FlatSessionLayout creation sites changed; the list is decrease-only"
    );
    // The builder reaches the interface only through the entry point.
    let main = std::fs::read_to_string("src/main.rs").unwrap();
    assert!(
        main.contains("sessions: quecto::composition::sessions::build_session_handles"),
        "main.rs hands the sessions builder to CliComposition"
    );
    let cli = std::fs::read_to_string("src/interface/cli/mod.rs").unwrap();
    assert!(cli.contains("pub sessions: SessionHandlesBuilder,"));
    for interface_file in production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
    {
        let names_composition = production_code(&interface_file)
            .iter()
            .any(|(_, line)| line.contains("crate::composition::sessions"));
        assert!(
            !names_composition,
            "{interface_file} must not name composition::sessions"
        );
    }
}

#[test]
fn one_layout_owns_the_sessions_join_sanitizer_and_file_names() {
    let persistence: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/infrastructure/persistence/"))
        .collect();
    for needle in [
        "join(\"sessions\")",
        "\"spill.jsonl\"",
        ".owner\"",
        ".json\"",
    ] {
        let owners: BTreeSet<String> = persistence
            .iter()
            .filter(|path| {
                production_code(path)
                    .iter()
                    .any(|(_, line)| line.contains(needle))
            })
            .cloned()
            .collect();
        assert_eq!(
            owners,
            set(&[LAYOUT_OWNER]),
            "{needle} is formed only by the layout"
        );
    }
    let sanitizer_callers = production_files_calling("sanitize_session_key(");
    let expected: BTreeSet<String> = set(SANITIZER_CALLERS)
        .into_iter()
        .chain(std::iter::once(
            "src/infrastructure/persistence/filename.rs".to_string(),
        ))
        .collect();
    assert_eq!(sanitizer_callers, expected, "sanitizer callers are exact");
    // No caller outside persistence forms a session path.
    let outside: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|p| !p.starts_with("src/infrastructure/persistence/"))
        .filter(|path| {
            production_code(path).iter().any(|(_, line)| {
                line.contains("join(\"sessions\")") || line.contains("spill.jsonl")
            })
        })
        .collect();
    assert!(
        outside.is_empty(),
        "session paths formed outside persistence: {outside:?}"
    );
}

#[test]
fn interface_never_lists_the_store_directly() {
    let direct: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .filter(|path| {
            production_code(path).iter().any(|(_, line)| {
                line.contains("session_store.list(") || line.contains(".list(None")
            })
        })
        .collect();
    assert!(
        direct.is_empty(),
        "interface lists the store directly in {direct:?}"
    );
    let query = std::fs::read_to_string("src/interface/cli/uds_dispatch_query.rs").unwrap();
    assert!(
        query.contains("list_sessions.list_all()"),
        "the list_sessions command is answered through the composed controller"
    );
}

#[test]
fn owner_line_ceilings_are_non_empty_and_respected() {
    assert!(!LINE_CEILINGS.is_empty());
    for (file, ceiling) in LINE_CEILINGS {
        let lines = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read {file}: {e}"))
            .lines()
            .count();
        assert!(
            lines <= *ceiling,
            "{file} has {lines} lines, above its decrease-only ceiling {ceiling}"
        );
        assert!(
            *ceiling <= 750,
            "{file}: ceilings never exceed the quality gate"
        );
    }
}
