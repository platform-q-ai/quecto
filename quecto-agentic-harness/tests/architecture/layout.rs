//! Filename-based, immediate-child layout ratchets for #2358.
//! Include mod.rs and cfg(test) support names; exclude only *_tests.rs.

use std::collections::BTreeSet;
use std::io;
use std::path::{Component, Path};

#[derive(Debug)]
struct FlatBudget<'a> {
    path: &'a str,
    maximum: usize,
}

#[derive(Debug)]
struct MigratedCapability<'a> {
    path: &'a str,
    allowed_roles: &'a [&'a str],
}

#[derive(Debug, PartialEq, Eq)]
enum Violation {
    FlatBudgetExceeded {
        path: String,
        actual: usize,
        maximum: usize,
    },
    UnexpectedMigratedEntry {
        path: String,
    },
    InvalidPolicy {
        path: String,
        reason: String,
    },
    InspectionFailed {
        path: String,
        reason: String,
    },
    UnlistedPlacement {
        path: String,
    },
}

const LAYERS: &[&str] = &[
    "domain",
    "application",
    "interface",
    "infrastructure",
    "composition",
];
const WIKI: &str =
    "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture";
const BUDGETS: &[FlatBudget<'_>] = &[
    FlatBudget {
        path: "domain",
        maximum: 71,
    },
    FlatBudget {
        path: "application",
        maximum: 45,
    },
    FlatBudget {
        path: "interface",
        maximum: 6,
    },
    FlatBudget {
        path: "infrastructure",
        maximum: 28,
    },
    FlatBudget {
        path: "composition",
        maximum: 24,
    },
    FlatBudget {
        path: "interface/cli",
        maximum: 106,
    },
    FlatBudget {
        path: "infrastructure/tools",
        maximum: 122,
    },
    FlatBudget {
        path: "infrastructure/persistence",
        maximum: 36,
    },
    FlatBudget {
        path: "application/swarm",
        maximum: 23,
    },
    FlatBudget {
        path: "domain/swarm",
        maximum: 15,
    },
];
// L0 moves nothing: future slices explicitly opt capabilities into strict shape.
const MIGRATED: &[MigratedCapability<'_>] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Classification {
    Target,
    Transitional,
    Testing,
}
#[derive(Debug, Clone)]
struct Placement<'a> {
    layer: &'a str,
    name: &'a str,
    citation: String,
    classification: Classification,
}

fn placements() -> Vec<Placement<'static>> {
    let targets: &[(&str, &[&str])] = &[
        (
            "domain",
            &[
                "conversation",
                "admission",
                "environments",
                "catalogue",
                "tool_policy",
                "sessions",
                "agents",
                "audit",
                "inference",
                "commander",
                "identity",
                "shared",
                "external_agent",
                "swarm",
                "workflow",
            ],
        ),
        (
            "application",
            &[
                "admission",
                "agent_turn",
                "audit",
                "catalogue",
                "configuration",
                "environments",
                "extensions",
                "external_agent",
                "provider_runtime",
                "providers",
                "search",
                "sessions",
                "subagents",
                "swarm",
                "tools",
                "workflow",
                "agent_commander",
                "shared",
            ],
        ),
        ("interface", &["cli", "repl", "tools", "uds"]),
        (
            "infrastructure",
            &[
                "admission",
                "auth",
                "config",
                "extensions",
                "external_agents",
                "http",
                "persistence",
                "processes",
                "providers",
                "search",
                "security",
                "tools",
                "workspace",
                "judgment",
                "observability",
                "time",
            ],
        ),
        (
            "composition",
            &["bootstrap", "runtime", "logging", "shutdown"],
        ),
    ];
    let mut rows = Vec::new();
    for &(layer, names) in targets {
        for &name in names {
            rows.push(Placement {
                layer,
                name,
                citation: format!("{WIKI}#{layer}"),
                classification: Classification::Target,
            });
        }
    }
    for (layer, name, classification) in [
        (
            "domain",
            "environment_registry",
            Classification::Transitional,
        ),
        ("application", "agent_loop", Classification::Transitional),
        ("infrastructure", "test_support", Classification::Testing),
    ] {
        rows.push(Placement {
            layer,
            name,
            citation: format!("{WIKI}#capability-coverage-and-naming"),
            classification,
        });
    }
    rows
}

fn safe_path(path: &str) -> bool {
    path.chars().next().is_some()
        && path.split('/').all(|part| {
            part.chars().next().is_some()
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/'))
        && path
            .split('/')
            .next()
            .is_some_and(|layer| LAYERS.contains(&layer))
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    File,
    Directory,
}
#[derive(Debug)]
struct Entry {
    name: String,
    kind: Kind,
}

fn scan(path: &Path) -> io::Result<Vec<Entry>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name().into_string().map_err(|name| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("non-UTF8 entry {name:?}"),
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", entry.path().display()))
            })?;
            let kind = match (file_type.is_file(), file_type.is_dir()) {
                (true, false) => Kind::File,
                (false, true) => Kind::Directory,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{}: expected regular file or directory",
                            entry.path().display()
                        ),
                    ));
                }
            };
            entries.push(Entry { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected directory (symlinks are not allowed)",
        ))
    }
}

// Every traversed component must be a real directory, never a symlink.
fn checked_directory(root: &Path, relative: &str) -> io::Result<std::path::PathBuf> {
    let mut current = std::path::PathBuf::new();
    for component in root.components().chain(Path::new(relative).components()) {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)?.file_type().is_dir() {
            continue;
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: expected real directory", current.display()),
        ));
    }
    Ok(current)
}

// Capability-local roles from the cited target tree, not a layer-wide union.
// A cohesive capability with no role children in the wiki may remain roleless.
fn wiki_roles(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "domain/conversation"
        | "domain/workflow"
        | "domain/swarm"
        | "domain/admission"
        | "domain/environments"
        | "domain/catalogue"
        | "domain/tool_policy"
        | "domain/sessions"
        | "domain/agents" => Some(&["entities", "value_objects", "services", "events"]),
        "domain/inference" | "domain/commander" => Some(&["value_objects", "services", "events"]),
        "domain/identity" => Some(&["value_objects"]),
        "domain/shared" => Some(&["error", "validation"]),
        "application/agent_turn"
        | "application/sessions"
        | "application/catalogue"
        | "application/configuration"
        | "application/subagents"
        | "application/environments"
        | "application/swarm"
        | "application/external_agent"
        | "application/admission"
        | "application/workflow"
        | "application/agent_commander" => Some(&["use_cases", "ports", "dto"]),
        "application/shared" => Some(&["transaction", "error"]),
        "interface/cli" | "interface/repl" => Some(&["controllers", "presenters", "dto"]),
        "interface/uds" => Some(&["framing", "wire", "mapping", "controllers", "presenters"]),
        "infrastructure/providers" => Some(&["anthropic", "openai", "codex", "transport"]),
        "infrastructure/tools" => Some(&["native", "uds", "registry", "isolation"]),
        "infrastructure/persistence" => Some(&["file", "sqlite", "records", "locks", "migrations"]),
        "infrastructure/processes" => Some(&["local", "containers", "monitoring"]),
        "infrastructure/config" => Some(&["schema", "loaders", "mapping", "writer"]),
        "domain/external_agent"
        | "domain/audit"
        | "application/extensions"
        | "application/search"
        | "application/audit"
        | "application/tools"
        | "application/providers"
        | "application/provider_runtime"
        | "interface/tools"
        | "infrastructure/judgment"
        | "infrastructure/admission"
        | "infrastructure/external_agents"
        | "infrastructure/extensions"
        | "infrastructure/search"
        | "infrastructure/auth"
        | "infrastructure/http"
        | "infrastructure/workspace"
        | "infrastructure/observability"
        | "infrastructure/time"
        | "infrastructure/security"
        | "composition/bootstrap"
        | "composition/runtime"
        | "composition/logging"
        | "composition/shutdown" => Some(&[]),
        _ => None,
    }
}

fn inspect(
    root: &Path,
    relative: &str,
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
    failures: &mut Vec<Violation>,
) -> Option<Vec<Entry>> {
    match checked_directory(root, relative).and_then(|path| scanner(&path)) {
        Ok(entries) => Some(entries),
        Err(error) => {
            failures.push(Violation::InspectionFailed {
                path: relative.into(),
                reason: error.to_string(),
            });
            None
        }
    }
}

fn validate(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[MigratedCapability<'_>],
) -> Vec<Violation> {
    validate_with(root, budgets, migrated, &scan)
}

fn validate_with(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[MigratedCapability<'_>],
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    for budget in budgets {
        if safe_path(budget.path) && seen.insert(budget.path) {
            if let Some(entries) = inspect(root, budget.path, scanner, &mut failures) {
                let actual = entries
                    .iter()
                    .filter(|entry| {
                        matches!(entry.kind, Kind::File)
                            && Path::new(&entry.name)
                                .extension()
                                .is_some_and(|ext| ext == "rs")
                            && entry.name.strip_suffix("_tests.rs").is_none()
                    })
                    .count();
                if actual <= budget.maximum {
                    if actual < budget.maximum {
                        eprintln!(
                            "layout ratchet {}: actual {actual}, maximum {}; lower the table in this PR",
                            budget.path, budget.maximum
                        );
                    }
                } else {
                    failures.push(Violation::FlatBudgetExceeded {
                        path: budget.path.into(),
                        actual,
                        maximum: budget.maximum,
                    });
                }
            }
        } else {
            failures.push(Violation::InvalidPolicy {
                path: budget.path.into(),
                reason: "expected unique safe layer-relative budget path".into(),
            });
        }
    }
    seen.clear();
    for capability in migrated {
        let roles: BTreeSet<_> = capability.allowed_roles.iter().copied().collect();
        let available = wiki_roles(capability.path);
        let valid_roles = roles.len() == capability.allowed_roles.len()
            && available.is_some_and(|available| roles.iter().all(|role| available.contains(role)));
        if safe_path(capability.path)
            && capability.path.split('/').count() == 2
            && seen.insert(capability.path)
            && valid_roles
        {
            if let Some(entries) = inspect(root, capability.path, scanner, &mut failures) {
                for entry in entries {
                    let permitted = match entry.kind {
                        Kind::File => entry.name == "mod.rs",
                        Kind::Directory => roles.contains(entry.name.as_str()),
                    };
                    if permitted {
                        continue;
                    }
                    failures.push(Violation::UnexpectedMigratedEntry {
                        path: format!("{}/{}", capability.path, entry.name),
                    });
                }
            }
        } else {
            failures.push(Violation::InvalidPolicy {
                path: capability.path.into(),
                reason: "expected unique capability path and explicit unique role names".into(),
            });
        }
    }
    failures
}

fn validate_placements(
    root: &Path,
    rows: &[Placement<'_>],
    migrated: &[MigratedCapability<'_>],
) -> Vec<Violation> {
    let mut failures = Vec::new();
    let mut paths = BTreeSet::new();
    for row in rows {
        let path = format!("{}/{}", row.layer, row.name);
        let section = match row.classification {
            Classification::Target => row.layer,
            Classification::Transitional | Classification::Testing => {
                "capability-coverage-and-naming"
            }
        };
        let valid_citation = row.citation == format!("{WIKI}#{section}");
        let classified = matches!(
            row.classification,
            Classification::Target | Classification::Transitional | Classification::Testing
        );
        if safe_path(&path)
            && path.split('/').count() == 2
            && valid_citation
            && classified
            && paths.insert(path.clone())
        {
            continue;
        }
        failures.push(Violation::InvalidPolicy {
            path,
            reason: "expected unique placement with known layer, safe name and wiki section".into(),
        });
    }
    for capability in migrated {
        if paths.contains(capability.path) {
            continue;
        }
        failures.push(Violation::InvalidPolicy {
            path: capability.path.into(),
            reason: "migrated capability must have a cited placement".into(),
        });
    }
    for layer in LAYERS {
        if let Some(entries) = inspect(root, layer, &scan, &mut failures) {
            for entry in entries {
                if matches!(entry.kind, Kind::Directory) {
                    let path = format!("{layer}/{}", entry.name);
                    if paths.contains(&path) {
                        continue;
                    }
                    failures.push(Violation::UnlistedPlacement { path });
                }
            }
        }
    }
    failures
}

#[test]
fn source_tree_layout_obeys_checked_in_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut failures = validate(&root, BUDGETS, MIGRATED);
    failures.extend(validate_placements(&root, &placements(), MIGRATED));
    assert!(
        failures.is_empty(),
        "layout policy violations: {failures:#?}"
    );
}

#[test]
fn adding_flat_file_exceeds_ratchet() {
    let tree = tempfile::tempdir().expect("create isolated layout fixture");
    let domain = tree.path().join("domain");
    std::fs::create_dir(&domain).expect("create domain layer");
    std::fs::write(domain.join("mod.rs"), "").expect("write counted module root");
    let budgets = [FlatBudget {
        path: "domain",
        maximum: 1,
    }];
    assert_eq!(validate(tree.path(), &budgets, &[]), Vec::new());

    std::fs::write(domain.join("extra.rs"), "").expect("add flat production file");
    assert_eq!(
        validate(tree.path(), &budgets, &[]),
        vec![Violation::FlatBudgetExceeded {
            path: "domain".into(),
            actual: 2,
            maximum: 1,
        }],
        "adding an immediate flat .rs file must exceed the unchanged budget"
    );
}

#[test]
fn stray_file_in_migrated_capability_fails_shape() {
    let tree = tempfile::tempdir().expect("create isolated layout fixture");
    let capability = tree.path().join("application/sessions");
    std::fs::create_dir_all(&capability).expect("create migrated capability");
    std::fs::write(capability.join("mod.rs"), "").expect("write permitted module root");
    for role in ["use_cases", "ports", "dto"] {
        std::fs::create_dir(capability.join(role)).expect("create permitted role");
    }
    let migrated = [MigratedCapability {
        path: "application/sessions",
        allowed_roles: &["use_cases", "ports", "dto"],
    }];
    assert_eq!(validate(tree.path(), &[], &migrated), Vec::new());

    std::fs::write(capability.join("stray.rs"), "").expect("add stray direct file");
    assert_eq!(
        validate(tree.path(), &[], &migrated),
        vec![Violation::UnexpectedMigratedEntry {
            path: "application/sessions/stray.rs".into(),
        }],
        "a migrated root permits only explicit role directories and mod.rs"
    );
}

#[test]
fn budget_counts_only_immediate_rust_files_and_excludes_only_test_suffix() {
    let root = tempfile::tempdir().unwrap();
    let domain = root.path().join("domain");
    std::fs::create_dir_all(domain.join("nested.rs")).unwrap();
    for name in [
        "mod.rs",
        "cfg_test_support.rs",
        "one.rs",
        "one_tests.rs",
        "_tests.rs",
        "README.md",
    ] {
        std::fs::write(domain.join(name), "").unwrap();
    }
    std::fs::write(domain.join("nested.rs/deep.rs"), "").unwrap();
    assert!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain",
                maximum: 3
            }],
            &[]
        )
        .is_empty()
    );
    assert_eq!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain",
                maximum: 2
            }],
            &[]
        ),
        vec![Violation::FlatBudgetExceeded {
            path: "domain".into(),
            actual: 3,
            maximum: 2
        }]
    );
    assert!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain",
                maximum: 4
            }],
            &[]
        )
        .is_empty()
    );
}

#[test]
fn migrated_roots_allow_only_mod_and_declared_role_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("application/sessions");
    std::fs::create_dir_all(path.join("use_cases")).unwrap();
    std::fs::write(path.join("mod.rs"), "").unwrap();
    std::fs::write(path.join("use_cases/test_support.rs"), "").unwrap();
    let policy = [MigratedCapability {
        path: "application/sessions",
        allowed_roles: &["use_cases"],
    }];
    assert!(validate(root.path(), &[], &policy).is_empty());
    for name in ["flat_tests.rs", "support.rs", "README.md", "ports"] {
        std::fs::write(path.join(name), "").unwrap();
    }
    std::fs::create_dir(path.join("handles")).unwrap();
    std::fs::create_dir(path.join("helpers")).unwrap();
    let failures = validate(root.path(), &[], &policy);
    assert_eq!(failures.len(), 6, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Violation::UnexpectedMigratedEntry { .. }))
    );
}

#[test]
fn migrated_roles_are_capability_specific_and_sparse() {
    let root = tempfile::tempdir().unwrap();
    for (path, role) in [
        ("domain/sessions", "entities"),
        ("application/sessions", "use_cases"),
        ("interface/cli", "controllers"),
        ("infrastructure/persistence", "records"),
    ] {
        std::fs::create_dir_all(root.path().join(path).join(role)).unwrap();
        assert!(
            validate(
                root.path(),
                &[],
                &[MigratedCapability {
                    path,
                    allowed_roles: &[role]
                }]
            )
            .is_empty()
        );
        assert!(matches!(
            validate(
                root.path(),
                &[],
                &[MigratedCapability {
                    path,
                    allowed_roles: &["handles"]
                }]
            )
            .as_slice(),
            [Violation::InvalidPolicy { .. }]
        ));
    }
    std::fs::create_dir_all(root.path().join("composition/bootstrap")).unwrap();
    assert!(
        validate(
            root.path(),
            &[],
            &[MigratedCapability {
                path: "composition/bootstrap",
                allowed_roles: &[]
            }]
        )
        .is_empty()
    );
    for (path, roles) in [
        ("domain/identity", &["entities"][..]),
        ("domain/sessions", &["ports"][..]),
        ("application/sessions", &["services"][..]),
        ("interface/cli", &["handlers"][..]),
        ("composition/bootstrap", &["wiring"][..]),
        ("domain/sessions", &["events", "events"][..]),
    ] {
        assert!(matches!(
            validate(
                root.path(),
                &[],
                &[MigratedCapability {
                    path,
                    allowed_roles: roles
                }]
            )
            .as_slice(),
            [Violation::InvalidPolicy { .. }]
        ));
    }
}

#[test]
fn policy_paths_are_safe_unique_and_inspection_is_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        "",
        "/domain",
        "domain/../application",
        "domain/./x",
        "domain//x",
        "other/x",
        "domain\\x",
    ] {
        assert!(matches!(
            validate(root.path(), &[FlatBudget { path, maximum: 0 }], &[]).as_slice(),
            [Violation::InvalidPolicy { .. }]
        ));
    }
    assert!(matches!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain",
                maximum: 0
            }],
            &[]
        )
        .as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    std::fs::write(root.path().join("domain"), "").unwrap();
    assert!(matches!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain",
                maximum: 0
            }],
            &[]
        )
        .as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    std::fs::remove_file(root.path().join("domain")).unwrap();
    std::fs::create_dir(root.path().join("domain")).unwrap();
    assert!(matches!(
        validate(
            root.path(),
            &[
                FlatBudget {
                    path: "domain",
                    maximum: 0
                },
                FlatBudget {
                    path: "domain",
                    maximum: 0
                }
            ],
            &[]
        )
        .as_slice(),
        [Violation::InvalidPolicy { .. }]
    ));
    let failure = validate_with(
        root.path(),
        &[FlatBudget {
            path: "domain",
            maximum: 0,
        }],
        &[],
        &|_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "entry metadata denied",
            ))
        },
    );
    assert!(
        matches!(failure.as_slice(), [Violation::InspectionFailed { reason, .. }] if reason.contains("entry metadata denied"))
    );
}

#[test]
fn placement_policy_covers_future_targets_but_rejects_unknown_directories() {
    let root = tempfile::tempdir().unwrap();
    for layer in LAYERS {
        std::fs::create_dir(root.path().join(layer)).unwrap();
    }
    let rows = placements();
    assert!(validate_placements(root.path(), &rows, &[]).is_empty());
    std::fs::create_dir(root.path().join("application/sessions")).unwrap();
    assert!(validate_placements(root.path(), &rows, &[]).is_empty());
    std::fs::write(root.path().join("application/legacy.rs"), "").unwrap();
    std::fs::create_dir(root.path().join("application/helpers")).unwrap();
    assert_eq!(
        validate_placements(root.path(), &rows, &[]),
        vec![Violation::UnlistedPlacement {
            path: "application/helpers".into()
        }]
    );
}

#[test]
fn placement_rows_require_safe_unique_paths_and_real_wiki_sections() {
    let root = tempfile::tempdir().unwrap();
    for layer in LAYERS {
        std::fs::create_dir(root.path().join(layer)).unwrap();
    }
    let row = Placement {
        layer: "application",
        name: "sessions",
        citation: format!("{WIKI}#application"),
        classification: Classification::Target,
    };
    assert!(matches!(
        validate_placements(root.path(), &[row.clone(), row.clone()], &[]).as_slice(),
        [Violation::InvalidPolicy { .. }]
    ));
    for (layer, name, citation) in [
        ("unknown", "sessions", row.citation.clone()),
        ("application", "../sessions", row.citation.clone()),
        ("application", "sessions", String::new()),
        ("application", "sessions", format!("{WIKI}#domain")),
        ("application", "sessions", format!("{WIKI}#invented")),
    ] {
        assert!(matches!(
            validate_placements(
                root.path(),
                &[Placement {
                    layer,
                    name,
                    citation,
                    classification: Classification::Target
                }],
                &[]
            )
            .as_slice(),
            [Violation::InvalidPolicy { .. }]
        ));
    }
    assert!(matches!(
        validate_placements(
            root.path(),
            &[row],
            &[MigratedCapability {
                path: "application/uncited",
                allowed_roles: &[]
            }]
        )
        .as_slice(),
        [Violation::InvalidPolicy { .. }]
    ));
}

#[cfg(unix)]
#[test]
fn filesystem_special_entries_and_non_utf8_names_fail_closed() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("domain")).unwrap();
    std::fs::write(root.path().join("target.rs"), "").unwrap();
    symlink(
        root.path().join("target.rs"),
        root.path().join("domain/link.rs"),
    )
    .unwrap();
    assert!(scan(&root.path().join("domain")).is_err());
    std::fs::remove_file(root.path().join("domain/link.rs")).unwrap();
    let socket_path = root.path().join("domain/socket");
    let socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    assert!(scan(&root.path().join("domain")).is_err());
    drop(socket);
    std::fs::remove_file(socket_path).unwrap();
    std::fs::write(
        root.path()
            .join("domain")
            .join(std::ffi::OsString::from_vec(vec![0xff])),
        "",
    )
    .unwrap();
    assert!(scan(&root.path().join("domain")).is_err());
    symlink(root.path().join("domain"), root.path().join("application")).unwrap();
    assert!(scan(&root.path().join("application")).is_err());
}

#[test]
fn migrated_entry_types_and_duplicate_capabilities_are_checked() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("domain/sessions");
    std::fs::create_dir_all(path.join("mod.rs")).unwrap();
    std::fs::write(path.join("events"), "").unwrap();
    let capability = || MigratedCapability {
        path: "domain/sessions",
        allowed_roles: &["events"],
    };
    let failures = validate(root.path(), &[], &[capability()]);
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Violation::UnexpectedMigratedEntry { .. }))
    );
    assert!(
        validate(root.path(), &[], &[capability(), capability()])
            .iter()
            .any(|failure| matches!(failure, Violation::InvalidPolicy { .. }))
    );
    for path in [
        "domain",
        "domain/sessions/nested",
        "domain/../sessions",
        "other/sessions",
    ] {
        assert!(matches!(
            validate(
                root.path(),
                &[],
                &[MigratedCapability {
                    path,
                    allowed_roles: &[]
                }]
            )
            .as_slice(),
            [Violation::InvalidPolicy { .. }]
        ));
    }
}

#[cfg(unix)]
#[test]
fn ancestor_symlinks_cannot_escape_the_inspected_root() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("swarm")).unwrap();
    symlink(outside.path(), root.path().join("domain")).unwrap();
    assert!(matches!(
        validate(
            root.path(),
            &[FlatBudget {
                path: "domain/swarm",
                maximum: 0
            }],
            &[]
        )
        .as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    assert!(matches!(
        validate(
            root.path(),
            &[],
            &[MigratedCapability {
                path: "domain/swarm",
                allowed_roles: &[]
            }]
        )
        .as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    symlink(outside.path(), root.path().join("linked_root")).unwrap();
    std::fs::create_dir(outside.path().join("domain")).unwrap();
    assert!(matches!(
        validate(
            &root.path().join("linked_root"),
            &[FlatBudget {
                path: "domain",
                maximum: 0
            }],
            &[]
        )
        .as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
}

#[test]
fn missing_or_file_substituted_migrated_and_layer_roots_fail() {
    let root = tempfile::tempdir().unwrap();
    let policy = [MigratedCapability {
        path: "domain/sessions",
        allowed_roles: &[],
    }];
    assert!(matches!(
        validate(root.path(), &[], &policy).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    std::fs::create_dir(root.path().join("domain")).unwrap();
    std::fs::write(root.path().join("domain/sessions"), "").unwrap();
    assert!(matches!(
        validate(root.path(), &[], &policy).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    let failures = validate_placements(root.path(), &placements(), &[]);
    assert_eq!(failures.len(), 4, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Violation::InspectionFailed { .. }))
    );
    std::fs::write(root.path().join("application"), "").unwrap();
    assert_eq!(
        validate_placements(root.path(), &placements(), &[]).len(),
        4
    );
}
