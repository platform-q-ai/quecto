//! Filename-based, immediate-child layout ratchets for #2358.
//! Include mod.rs and cfg(test) support names; exclude only *_tests.rs.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

#[derive(Debug)]
struct FlatBudget<'a> {
    path: &'a str,
    // Original RED seam name retained: this is an exact baseline, not a ceiling.
    maximum: usize,
}

#[derive(Debug)]
struct MigratedCapability<'a> {
    path: &'a str,
    allowed_roles: &'a [&'a str],
}

#[derive(PartialEq, Eq)]
enum Violation {
    FlatBudgetExceeded {
        path: String,
        actual: usize,
        maximum: usize,
    },
    UnexpectedMigratedEntry {
        path: String,
    },
    InspectionFailed {
        path: String,
        reason: String,
    },
    UnlistedPlacement {
        path: String,
    },
}

impl std::fmt::Debug for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FlatBudgetExceeded {
                path,
                actual,
                maximum,
            } => {
                write!(f, "{path}: actual {actual}, expected {maximum}; ")?;
                if actual < maximum {
                    write!(f, "lower {path} to {actual} in BUDGETS in this PR. {WIKI}")
                } else {
                    write!(
                        f,
                        "move growth into the target capability/role directory in an authorized layout slice; do not raise BUDGETS. {WIKI}"
                    )
                }
            }
            Self::UnexpectedMigratedEntry { path } => write!(
                f,
                "{path}: migrated shape violation; keep mod.rs and mod_tests.rs at the capability root, move implementation into this capability's declared role directories; for a present empty role, add an immediate regular .rs file to the role or remove the unused role directory. {WIKI}"
            ),
            Self::UnlistedPlacement { path } => write!(
                f,
                "{path}: unlisted immediate capability directory; add a placement row with this capability's wiki section and classify it as target, transitional, or testing, or move the directory into a listed placement. {WIKI}"
            ),
            Self::InspectionFailed { path, reason } => write!(
                f,
                "{path}: inspection failed ({reason}); restore a readable real directory/file tree; descendant symlinks are not inspected. {WIKI}"
            ),
        }
    }
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
#[rustfmt::skip] // One readable source-policy row per path.
const BUDGETS: &[FlatBudget<'_>] = &[
    FlatBudget { path: "domain", maximum: 71 },
    FlatBudget { path: "application", maximum: 45 },
    FlatBudget { path: "interface", maximum: 6 },
    FlatBudget { path: "infrastructure", maximum: 28 },
    FlatBudget { path: "composition", maximum: 24 },
    FlatBudget { path: "interface/cli", maximum: 106 },
    FlatBudget { path: "infrastructure/tools", maximum: 122 },
    FlatBudget { path: "infrastructure/persistence", maximum: 36 },
    FlatBudget { path: "application/swarm", maximum: 23 },
    FlatBudget { path: "domain/swarm", maximum: 15 },
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
    let targets = [
        (
            "domain",
            "conversation admission environments catalogue tool_policy sessions agents audit inference commander identity shared external_agent swarm workflow",
        ),
        (
            "application",
            "admission agent_turn audit catalogue configuration environments extensions external_agent provider_runtime providers search sessions subagents swarm tools workflow agent_commander shared",
        ),
        ("interface", "cli repl tools uds"),
        (
            "infrastructure",
            "admission auth config extensions external_agents http persistence processes providers search security tools workspace judgment observability time",
        ),
        ("composition", "bootstrap runtime logging shutdown"),
    ];
    let mut rows = Vec::new();
    for (layer, names) in targets {
        for name in names.split_whitespace() {
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

// The caller root is canonicalized once; only relative descendants are checked.
fn checked_directory(root: &Path, relative: &str) -> io::Result<std::path::PathBuf> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
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

fn canonical_root(root: &Path) -> Result<std::path::PathBuf, Vec<Violation>> {
    std::fs::canonicalize(root).map_err(|error| {
        vec![Violation::InspectionFailed {
            path: root.display().to_string(),
            reason: error.to_string(),
        }]
    })
}

fn regular_rust(entry: &Entry) -> bool {
    matches!(entry.kind, Kind::File) && entry.name.ends_with(".rs")
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
    validate_all(root, budgets, migrated, None, scanner)
}

// One canonicalization boundary for a complete inspection, including placements.
fn validate_all(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[MigratedCapability<'_>],
    rows: Option<&[Placement<'_>]>,
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let root = match canonical_root(root) {
        Ok(root) => root,
        Err(failures) => return failures,
    };
    let mut failures = inspect_layout(&root, budgets, migrated, scanner);
    if let Some(rows) = rows {
        failures.extend(inspect_placements(&root, rows, scanner));
    }
    failures
}

fn inspect_layout(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[MigratedCapability<'_>],
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let mut failures = Vec::new();
    for budget in budgets {
        if let Some(entries) = inspect(root, budget.path, scanner, &mut failures) {
            let actual = entries
                .iter()
                .filter(|entry| {
                    regular_rust(entry) && entry.name.strip_suffix("_tests.rs").is_none()
                })
                .count();
            if actual == budget.maximum {
                continue;
            }
            failures.push(Violation::FlatBudgetExceeded {
                path: budget.path.into(),
                actual,
                maximum: budget.maximum,
            });
        }
    }
    for capability in migrated {
        if let Some(entries) = inspect(root, capability.path, scanner, &mut failures) {
            for entry in entries {
                let path = format!("{}/{}", capability.path, entry.name);
                match entry.kind {
                    Kind::File if matches!(entry.name.as_str(), "mod.rs" | "mod_tests.rs") => {}
                    Kind::Directory if capability.allowed_roles.contains(&entry.name.as_str()) => {
                        if let Some(children) = inspect(root, &path, scanner, &mut failures) {
                            if children.iter().any(regular_rust) {
                                continue;
                            }
                            failures.push(Violation::UnexpectedMigratedEntry { path });
                        }
                    }
                    _ => failures.push(Violation::UnexpectedMigratedEntry { path }),
                }
            }
        }
    }
    failures
}

fn validate_placements(root: &Path, rows: &[Placement<'_>]) -> Vec<Violation> {
    validate_all(root, &[], &[], Some(rows), &scan)
}

fn inspect_placements(
    root: &Path,
    rows: &[Placement<'_>],
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let mut failures = Vec::new();
    let paths: BTreeSet<_> = rows
        .iter()
        .map(|row| {
            // Citation/classification describe ownership, not a self-policy schema.
            let _ = (&row.citation, row.classification);
            format!("{}/{}", row.layer, row.name)
        })
        .collect();
    for layer in LAYERS {
        if let Some(entries) = inspect(root, layer, scanner, &mut failures) {
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
    let rows = placements();
    let keys: Vec<_> = BUDGETS
        .iter()
        .map(|row| ("budget", row.path.to_owned()))
        .chain(MIGRATED.iter().map(|row| ("migrated", row.path.to_owned())))
        .chain(
            rows.iter()
                .map(|row| ("placement", format!("{}/{}", row.layer, row.name))),
        )
        .collect();
    assert_eq!(
        keys.iter().collect::<BTreeSet<_>>().len(),
        keys.len(),
        "duplicate source-policy row"
    );
    let failures = validate_all(&root, BUDGETS, MIGRATED, Some(&rows), &scan);
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
        std::fs::write(capability.join(role).join("fixture.rs"), "")
            .expect("populate permitted role");
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

// Small fixture helpers keep the filesystem scenarios visible without repeated setup.
fn fixture(directories: &[&str], files: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for directory in directories {
        std::fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    for file in files {
        let path = root.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    root
}

fn budget(path: &str, maximum: usize) -> FlatBudget<'_> {
    FlatBudget { path, maximum }
}

fn capability<'a>(path: &'a str, allowed_roles: &'a [&'a str]) -> MigratedCapability<'a> {
    MigratedCapability {
        path,
        allowed_roles,
    }
}

fn mismatch(path: &str, actual: usize, maximum: usize) -> Violation {
    Violation::FlatBudgetExceeded {
        path: path.into(),
        actual,
        maximum,
    }
}

fn unexpected(path: impl Into<String>) -> Violation {
    Violation::UnexpectedMigratedEntry { path: path.into() }
}

fn flat(root: &Path, path: &str, expected: usize) -> Vec<Violation> {
    validate(root, &[budget(path, expected)], &[])
}

fn shape(root: &Path, path: &str, roles: &[&str]) -> Vec<Violation> {
    validate(root, &[], &[capability(path, roles)])
}

#[test]
fn budget_counts_only_immediate_rust_files_and_excludes_only_test_suffix() {
    let root = fixture(
        &["domain/nested.rs"],
        &[
            "domain/mod.rs",
            "domain/cfg_test_support.rs",
            "domain/one.rs",
            "domain/one_tests.rs",
            "domain/_tests.rs",
            "domain/README.md",
            "domain/nested.rs/deep.rs",
        ],
    );
    for expected in [2, 3, 4] {
        let failures = flat(root.path(), "domain", expected);
        let wanted = if expected == 3 {
            vec![]
        } else {
            vec![mismatch("domain", 3, expected)]
        };
        assert_eq!(failures, wanted, "exact baseline {expected}");
    }
}

#[test]
fn migrated_roots_allow_only_mod_and_declared_role_directories() {
    let root = fixture(
        &[],
        &[
            "application/sessions/mod.rs",
            "application/sessions/mod_tests.rs",
            "application/sessions/use_cases/test_support.rs",
        ],
    );
    let path = root.path().join("application/sessions");
    let policy = [capability("application/sessions", &["use_cases"])];
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
    for (path, role) in [
        ("domain/sessions", "events"),
        ("application/sessions", "ports"),
        ("interface/cli", "presenters"),
        ("infrastructure/providers", "anthropic"),
    ] {
        let root = fixture(&[], &[&format!("{path}/{role}/one.rs")]);
        let directory = root.path().join(path);
        let roles = [role];
        let migrated = [capability(path, &roles)];
        assert!(validate(root.path(), &[], &migrated).is_empty());
        std::fs::create_dir(directory.join("handles")).unwrap();
        assert_eq!(
            validate(root.path(), &[], &migrated),
            vec![unexpected(format!("{path}/handles"))]
        );
    }
    let root = fixture(&["composition/bootstrap"], &[]);
    assert!(shape(root.path(), "composition/bootstrap", &[]).is_empty());
}

#[test]
fn inspection_failures_and_restored_directory() {
    let root = tempfile::tempdir().unwrap();
    let budget = [budget("domain", 0)];
    assert!(matches!(
        validate(root.path(), &budget, &[]).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    std::fs::write(root.path().join("domain"), "").unwrap();
    assert!(matches!(
        validate(root.path(), &budget, &[]).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    std::fs::remove_file(root.path().join("domain")).unwrap();
    std::fs::create_dir(root.path().join("domain")).unwrap();
    assert!(validate(root.path(), &budget, &[]).is_empty());
    let failures = validate_with(root.path(), &budget, &[], &|_| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "entry metadata denied",
        ))
    });
    assert!(
        matches!(failures.as_slice(), [Violation::InspectionFailed { reason, .. }] if reason.contains("entry metadata denied"))
    );
}

#[test]
fn placement_policy_covers_future_targets_but_rejects_unknown_directories() {
    for (known, unknown) in [
        ("application/sessions", "application/helpers"),
        ("domain/sessions", "domain/unknown"),
    ] {
        let root = fixture(LAYERS, &[]);
        let rows = placements();
        assert!(validate_placements(root.path(), &rows).is_empty());
        std::fs::create_dir(root.path().join(known)).unwrap();
        std::fs::write(root.path().join("application/legacy.rs"), "").unwrap();
        assert!(validate_placements(root.path(), &rows).is_empty());
        std::fs::create_dir(root.path().join(unknown)).unwrap();
        assert_eq!(
            validate_placements(root.path(), &rows),
            vec![Violation::UnlistedPlacement {
                path: unknown.into()
            }]
        );
    }
}

#[cfg(unix)]
#[test]
fn filesystem_special_entries_and_non_utf8_names_fail_closed() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let root = fixture(&["domain"], &["target.rs"]);
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
fn migrated_entry_types_are_checked() {
    let root = fixture(&["domain/sessions/mod.rs"], &["domain/sessions/events"]);
    let failures = validate(
        root.path(),
        &[],
        &[capability("domain/sessions", &["events"])],
    );
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Violation::UnexpectedMigratedEntry { .. }))
    );
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
        flat(root.path(), "domain/swarm", 0).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    assert!(matches!(
        shape(root.path(), "domain/swarm", &[]).as_slice(),
        [Violation::InspectionFailed { .. }]
    ));
    symlink(outside.path(), root.path().join("linked_root")).unwrap();
    std::fs::create_dir(outside.path().join("domain")).unwrap();
    assert!(flat(&root.path().join("linked_root"), "domain", 0).is_empty());
}

#[test]
fn missing_or_file_substituted_migrated_and_layer_roots_fail() {
    let root = tempfile::tempdir().unwrap();
    let policy = [capability("domain/sessions", &[])];
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
    let failures = validate_placements(root.path(), &placements());
    assert_eq!(failures.len(), 4, "{failures:?}");
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, Violation::InspectionFailed { .. }))
    );
    std::fs::write(root.path().join("application"), "").unwrap();
    assert_eq!(validate_placements(root.path(), &placements()).len(), 4);
}

#[test]
fn amendment_exact_decrease_fails() {
    let root = fixture(&[], &["domain/mod.rs"]);
    assert_eq!(flat(root.path(), "domain", 2).len(), 1);
}

#[test]
fn amendment_empty_role_fails_but_one_rust_file_passes() {
    for files in [
        &[][..],
        &["README.md"][..],
        &["nested/deep.rs"][..],
        &["one.rs"][..],
        &["tests_tests.rs"][..],
    ] {
        let paths: Vec<_> = files
            .iter()
            .map(|name| format!("domain/sessions/entities/{name}"))
            .collect();
        let names: Vec<_> = paths.iter().map(String::as_str).collect();
        let root = fixture(&["domain/sessions/entities"], &names);
        let failures = validate(
            root.path(),
            &[],
            &[capability("domain/sessions", &["entities"])],
        );
        if files
            .iter()
            .any(|name| matches!(*name, "one.rs" | "tests_tests.rs"))
        {
            assert!(failures.is_empty(), "{files:?}");
        } else {
            assert_eq!(
                failures,
                vec![unexpected("domain/sessions/entities")],
                "{files:?}"
            );
        }
    }
}

#[test]
fn amendment_count_neutral_rename_and_replacement_pass() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("domain")).unwrap();
    std::fs::write(root.path().join("domain/old.rs"), "old").unwrap();
    let budgets = [budget("domain", 1)];
    assert!(validate(root.path(), &budgets, &[]).is_empty());
    std::fs::rename(
        root.path().join("domain/old.rs"),
        root.path().join("domain/new.rs"),
    )
    .unwrap();
    assert!(validate(root.path(), &budgets, &[]).is_empty());
    std::fs::write(root.path().join("domain/new.rs"), "replacement").unwrap();
    assert!(validate(root.path(), &budgets, &[]).is_empty());
}

#[cfg(unix)]
#[test]
fn amendment_role_symlink_and_read_failure_fail_closed() {
    let root = fixture(&["domain/sessions/entities"], &["outside.rs"]);
    let role = root.path().join("domain/sessions/entities");
    std::os::unix::fs::symlink(root.path().join("outside.rs"), role.join("linked.rs")).unwrap();
    let migrated = [capability("domain/sessions", &["entities"])];
    assert!(matches!(validate(root.path(), &[], &migrated).as_slice(),
        [Violation::InspectionFailed { path, reason }] if path == "domain/sessions/entities" && reason.contains("linked.rs")));
    let failures = validate_with(root.path(), &[], &migrated, &|path| {
        if path.ends_with("entities") {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "role read denied",
            ));
        }
        scan(path)
    });
    assert!(
        matches!(failures.as_slice(), [Violation::InspectionFailed { path, reason }] if path == "domain/sessions/entities" && reason.contains("role read denied"))
    );
}

#[test]
fn amendment_diagnostics_name_concrete_adjustments() {
    for (failure, fragments) in [
        (
            mismatch("domain", 2, 1),
            &["BUDGETS", "capability", WIKI][..],
        ),
        (
            mismatch("domain", 1, 2),
            &["actual 1, expected 2", "lower domain to 1 in BUDGETS"][..],
        ),
        (
            Violation::UnlistedPlacement {
                path: "domain/widgets".into(),
            },
            &["add a placement row with this capability's wiki section"][..],
        ),
        (
            unexpected("domain/sessions/entities"),
            &["add an immediate regular .rs file to the role"][..],
        ),
    ] {
        let message = format!("{failure:?}");
        for fragment in fragments {
            assert!(message.contains(fragment), "{message}");
        }
    }
}

#[cfg(unix)]
#[test]
fn unified_inspection_accepts_caller_symlink_for_layout_and_placements() {
    let root = tempfile::tempdir().unwrap();
    for layer in LAYERS {
        std::fs::create_dir_all(root.path().join("real").join(layer)).unwrap();
    }
    std::fs::create_dir(root.path().join("real/domain/sessions")).unwrap();
    std::os::unix::fs::symlink(root.path().join("real"), root.path().join("alias")).unwrap();
    assert!(flat(&root.path().join("alias"), "domain", 0).is_empty());
    assert!(
        validate_all(
            &root.path().join("alias"),
            &[budget("domain", 0)],
            &[],
            Some(&placements()),
            &scan
        )
        .is_empty()
    );
}
