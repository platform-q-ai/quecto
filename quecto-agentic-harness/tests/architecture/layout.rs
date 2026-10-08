//! Filename-based, immediate-child layout ratchets for #2358.
//! Include mod.rs and cfg(test) support names; exclude only *_tests.rs.
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
struct FlatBudget<'a> {
    path: &'a str,
    expected: usize,
}
struct Capability<'a>(&'a str, &'a [&'a str]);
#[derive(PartialEq, Eq, Debug)]
enum Violation {
    FlatBudgetMismatch {
        path: String,
        actual: usize,
        expected: usize,
    },
    StrayEntry(String),
    Inspection(String, String),
    UndeclaredRole(String),
    EmptyRole(String),
    StalePlacement(String),
    UnlistedPlacement(String),
}
impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::FlatBudgetMismatch {
                path,
                actual,
                expected,
            } => {
                let repair = if actual < expected {
                    format!("lower {path} to {actual} in BUDGETS in this PR. {WIKI}")
                } else {
                    format!(
                        "move growth into the target capability/role directory in an authorized layout slice; do not raise BUDGETS. {WIKI}"
                    )
                };
                format!("{path}: actual {actual}, expected {expected}; {repair}")
            }
            Self::StrayEntry(path) => format!(
                "{path}: migrated shape violation; move {path} into a declared role directory; keep only mod.rs and mod_tests.rs at the capability root. {WIKI}"
            ),
            Self::UndeclaredRole(path) => {
                let (capability, name) = path.rsplit_once('/').expect("role has capability parent");
                format!(
                    "{path}: undeclared role directory; add {name} to {capability} allowed_roles in MIGRATED or remove the directory. {WIKI}"
                )
            }
            Self::EmptyRole(path) => format!(
                "{path}: empty role; put a .rs file anywhere beneath {path} or remove the unused role directory. {WIKI}"
            ),
            Self::StalePlacement(path) => format!(
                "{path}: stale transitional placement; remove {path} from PLACEMENTS when its directory has been retired. {WIKI}"
            ),
            Self::UnlistedPlacement(path) => format!(
                "{path}: unlisted immediate capability directory; add a placement row with this capability's wiki section and classify it as target, transitional, or testing, or move the directory into a listed placement. {WIKI}"
            ),
            Self::Inspection(path, reason) => format!(
                "{path}: inspection failed ({reason}); restore {path} as a readable real directory/file tree; descendant symlinks are not inspected. {WIKI}"
            ),
        };
        f.write_str(&message)
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
    FlatBudget { path: "domain", expected: 14 },
    FlatBudget { path: "application", expected: 41 },
    FlatBudget { path: "interface", expected: 6 },
    FlatBudget { path: "infrastructure", expected: 28 },
    FlatBudget { path: "composition", expected: 24 },
    FlatBudget { path: "interface/cli", expected: 106 },
    FlatBudget { path: "infrastructure/tools", expected: 122 },
    FlatBudget { path: "infrastructure/persistence", expected: 36 },
    FlatBudget { path: "application/swarm", expected: 23 },
    FlatBudget { path: "domain/swarm", expected: 15 },
];
// Layout slices opt capabilities into strict shape as they migrate (#2357).
#[rustfmt::skip] // One readable source-policy row per capability.
const MIGRATED: &[Capability<'_>] = &[
    Capability("domain/conversation", &["value_objects", "services"]),
    Capability("domain/tool_policy", &["value_objects", "services"]),
    Capability("domain/catalogue", &["value_objects"]),
    Capability("domain/sessions", &["entities", "value_objects", "services"]),
    Capability("domain/inference", &["value_objects", "services", "events"]),
    Capability("domain/admission", &["value_objects", "services"]),
    Capability("domain/agents", &["entities", "value_objects", "services"]),
    Capability("domain/environments", &["entities", "services"]),
];
// Wiki target-source-tree anchors define allowed future names; Transitional rows must exist.
// https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#target-source-tree
type Placement<'a> = (&'a str, &'a [&'a str]);
#[rustfmt::skip]
const PLACEMENTS: &[Placement<'_>] = &[
    ("domain", &["conversation", "admission", "environments", "catalogue", "tool_policy", "sessions", "agents", "audit", "inference", "commander", "identity", "shared", "external_agent", "swarm", "workflow"]),
    ("application", &["admission", "agent_turn", "audit", "catalogue", "configuration", "environments", "extensions", "external_agent", "provider_runtime", "providers", "search", "sessions", "subagents", "swarm", "tools", "workflow", "agent_commander", "shared", "agent_loop"]),
    ("interface", &["cli", "repl", "tools", "uds"]),
    ("infrastructure", &["admission", "auth", "config", "extensions", "external_agents", "http", "persistence", "processes", "providers", "search", "security", "tools", "workspace", "judgment", "observability", "time", "test_support"]),
    ("composition", &["bootstrap", "runtime", "logging", "shutdown"]),
 ];
const TRANSITIONAL: &[(&str, &str)] = &[("application", "agent_loop")];
enum Kind {
    File,
    Directory,
}
struct Entry(String, Kind);
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn real_directory(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(invalid(format!("{}: not a real directory", path.display())))
    }
}
fn scan(path: &Path) -> io::Result<Vec<Entry>> {
    real_directory(path)?;
    std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("non-UTF-8 entry name"))?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            let kind = if metadata.is_file() {
                std::fs::File::open(entry.path())
                    .map_err(|error| io::Error::new(error.kind(), format!("{name}: {error}")))?;
                Kind::File
            } else if metadata.is_dir() {
                Kind::Directory
            } else {
                return Err(invalid(format!("{name}: symlink or non-regular entry")));
            };
            Ok(Entry(name, kind))
        })
        .collect()
}
fn regular_rust(entry: &Entry) -> bool {
    matches!(entry.1, Kind::File) && entry.0.ends_with(".rs")
}
fn checked_directory(root: &Path, relative: &str) -> io::Result<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        real_directory(&path)?;
    }
    Ok(path)
}
struct Inspector<'a, F> {
    root: &'a Path,
    scanner: &'a F,
    failures: Vec<Violation>,
}
impl<F: Fn(&Path) -> io::Result<Vec<Entry>>> Inspector<'_, F> {
    fn inspect(&mut self, relative: &str) -> Option<Vec<Entry>> {
        checked_directory(self.root, relative)
            .and_then(|path| (self.scanner)(&path))
            .map_err(|error| {
                self.failures
                    .push(Violation::Inspection(relative.into(), error.to_string()))
            })
            .ok()
    }
    // Inspect every descendant after success or failure; try_fold would short-circuit.
    #[expect(clippy::manual_try_fold)]
    fn role_has_rust(&mut self, relative: &str) -> Option<bool> {
        self.inspect(relative)?
            .into_iter()
            .fold(Some(false), |found, entry| {
                let next = match entry.1 {
                    Kind::File => Some(regular_rust(&entry)),
                    Kind::Directory => self.role_has_rust(&format!("{relative}/{}", entry.0)),
                };
                found.zip(next).map(|(found, next)| found || next)
            })
    }
    fn layout(&mut self, budgets: &[FlatBudget<'_>], migrated: &[Capability<'_>]) {
        for budget in budgets {
            if let Some(entries) = self.inspect(budget.path) {
                let actual = entries
                    .iter()
                    .filter(|entry| match entry.0.strip_suffix("_tests.rs") {
                        Some(_) => false,
                        None => regular_rust(entry),
                    })
                    .count();
                if actual == budget.expected {
                    continue;
                }
                self.failures.push(Violation::FlatBudgetMismatch {
                    path: budget.path.into(),
                    actual,
                    expected: budget.expected,
                });
            }
        }
        for capability in migrated {
            if let Some(entries) = self.inspect(capability.0) {
                for entry in entries {
                    let path = format!("{}/{}", capability.0, entry.0);
                    match entry.1 {
                        Kind::File if matches!(entry.0.as_str(), "mod.rs" | "mod_tests.rs") => {}
                        Kind::Directory if capability.1.contains(&entry.0.as_str()) => {
                            if let Some(false) = self.role_has_rust(&path) {
                                self.failures.push(Violation::EmptyRole(path));
                            }
                        }
                        Kind::Directory => self.failures.push(Violation::UndeclaredRole(path)),
                        _ => self.failures.push(Violation::StrayEntry(path)),
                    }
                }
            }
        }
    }
    fn placements(&mut self, rows: &[Placement<'_>]) {
        for layer in LAYERS {
            if let Some(entries) = self.inspect(layer) {
                for (_, name) in TRANSITIONAL.iter().filter(|(owner, name)| {
                    owner == layer
                        && rows
                            .iter()
                            .any(|(layer, names)| layer == owner && names.contains(name))
                }) {
                    if entries
                        .iter()
                        .any(|entry| entry.0 == *name && matches!(entry.1, Kind::Directory))
                    {
                        continue;
                    }
                    self.failures
                        .push(Violation::StalePlacement(format!("{layer}/{name}")));
                }
                for entry in entries {
                    if matches!(entry.1, Kind::Directory) {
                        let path = format!("{layer}/{}", entry.0);
                        if rows.iter().any(|(owner, names)| {
                            owner == layer && names.contains(&entry.0.as_str())
                        }) {
                            continue;
                        }
                        self.failures.push(Violation::UnlistedPlacement(path));
                    }
                }
            }
        }
    }
}
fn validate(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[Capability<'_>],
) -> Vec<Violation> {
    validate_all(root, budgets, migrated, None, &scan)
}
fn validate_all(
    root: &Path,
    budgets: &[FlatBudget<'_>],
    migrated: &[Capability<'_>],
    rows: Option<&[Placement<'_>]>,
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let root = match std::fs::canonicalize(root).and_then(|root| {
        real_directory(&root)?;
        Ok(root)
    }) {
        Ok(root) => root,
        Err(error) => {
            return vec![Violation::Inspection(
                root.display().to_string(),
                error.to_string(),
            )];
        }
    };
    let mut inspector = Inspector {
        root: &root,
        scanner,
        failures: Vec::new(),
    };
    inspector.layout(budgets, migrated);
    if let Some(rows) = rows {
        inspector.placements(rows);
    }
    let mut reported = BTreeSet::new();
    inspector.failures.retain(|failure| match failure {
        Violation::Inspection(path, _) => reported.insert(path.clone()),
        _ => true,
    });
    inspector.failures
}
fn placements(root: &Path, rows: &[Placement<'_>]) -> Vec<Violation> {
    validate_all(root, &[], &[], Some(rows), &scan)
}
#[test]
fn source_tree_layout_obeys_checked_in_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let rows = PLACEMENTS;
    let keys: BTreeSet<_> =
        BUDGETS
            .iter()
            .map(|row| ("budget", "", row.path))
            .chain(MIGRATED.iter().map(|row| ("migrated", "", row.0)))
            .chain(rows.iter().flat_map(|(layer, names)| {
                names.iter().map(move |name| ("placement", *layer, *name))
            }))
            .collect();
    assert_eq!(
        keys.len(),
        BUDGETS.len() + MIGRATED.len() + rows.iter().map(|(_, names)| names.len()).sum::<usize>(),
        "duplicate source-policy row"
    );
    let failures = validate_all(&root, BUDGETS, MIGRATED, Some(rows), &scan);
    assert!(
        failures.is_empty(),
        "layout policy violations: {failures:#?}"
    );
}
#[path = "layout/environments.rs"]
mod environments;
#[path = "layout/fixtures.rs"]
mod fixtures;

const AGENT_MODULES: &[&str] = &[
    "agent",
    "subagent",
    "subagent_launch",
    "subagent_teardown",
    "parent_control",
    "harness_lifetime",
    "child_end",
    "child_session",
    "unread_report",
];
const EXTERNAL_AGENT_FILES: &[&str] = &["mod.rs", "backend.rs", "stream.rs", "turn.rs", "usage.rs"];

fn domain() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain")
}

#[test]
fn agent_modules_live_under_domain_agents() {
    let expected_paths: Vec<_> = [
        "value_objects/agent.rs",
        "value_objects/subagent_launch.rs",
        "services/subagent.rs",
        "services/subagent_teardown.rs",
        "entities/parent_control.rs",
        "value_objects/harness_lifetime.rs",
        "value_objects/child_end.rs",
        "services/child_session.rs",
        "services/unread_report.rs",
    ]
    .iter()
    .map(|path| domain().join("agents").join(path))
    .collect();
    assert!(
        expected_paths.iter().all(|path| path.is_file()),
        "expected in src/domain/agents/: {expected_paths:?}"
    );
}

#[test]
fn agent_modules_are_absent_from_domain_root() {
    let stray: Vec<_> = AGENT_MODULES
        .iter()
        .filter(|name| domain().join(format!("{name}.rs")).exists())
        .collect();
    assert!(stray.is_empty(), "still at src/domain root: {stray:?}");
}

#[test]
fn external_agent_stays_a_sibling_domain_capability() {
    let missing: Vec<_> = EXTERNAL_AGENT_FILES
        .iter()
        .filter(|file| !domain().join("external_agent").join(file).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "expected in src/domain/external_agent/: {missing:?}"
    );
    assert!(
        !domain().join("agents/external_agent").exists(),
        "external_agent must not nest under domain/agents (#2356 wiki e61de22)"
    );
}

// Binding L5 whole-file moves; UserKind is the separately approved extraction.
#[rustfmt::skip]
const L5_MOVES: &[(&str, &str)] = &[
    ("message.rs", "conversation/value_objects/message.rs"),
    ("message_tests.rs", "conversation/value_objects/message_tests.rs"),
    ("message_cov_tests.rs", "conversation/value_objects/message_cov_tests.rs"),
    ("conversation_view.rs", "conversation/services/conversation_view.rs"),
    ("conversation_view_tests.rs", "conversation/services/conversation_view_tests.rs"),
    ("conversation_edit.rs", "conversation/services/conversation_edit.rs"),
    ("conversation_edit_tests.rs", "conversation/services/conversation_edit_tests.rs"),
    ("turn_origin.rs", "conversation/services/turn_origin.rs"),
    ("turn_origin_tests.rs", "conversation/services/turn_origin_tests.rs"),
    ("visible_thinking.rs", "conversation/services/visible_thinking.rs"),
    ("visible_thinking_tests.rs", "conversation/services/visible_thinking_tests.rs"),
    ("conversation/image_input.rs", "conversation/services/image_input.rs"),
    ("conversation/image_input_tests.rs", "conversation/services/image_input_tests.rs"),
    ("conversation/image_tokens.rs", "conversation/value_objects/image_tokens.rs"),
    ("conversation/image_tokens_tests.rs", "conversation/value_objects/image_tokens_tests.rs"),
    ("conversation/reply_requirement.rs", "conversation/services/reply_requirement.rs"),
    ("conversation/reply_requirement_tests.rs", "conversation/services/reply_requirement_tests.rs"),
    ("conversation/stored_images.rs", "conversation/value_objects/stored_images.rs"),
    ("conversation/stored_images_tests.rs", "conversation/value_objects/stored_images_tests.rs"),
    ("conversation/user_images.rs", "conversation/value_objects/user_images.rs"),
    ("conversation/user_images_tests.rs", "conversation/value_objects/user_images_tests.rs"),
    ("conversation/watermark.rs", "conversation/services/watermark.rs"),
    ("conversation/watermark_tests.rs", "conversation/services/watermark_tests.rs"),
    ("conversation/watermark_split_tests.rs", "conversation/services/watermark_split_tests.rs"),
    ("conversation/watermark_cut.rs", "conversation/services/watermark_cut.rs"),
    ("conversation/watermark_cut_tests.rs", "conversation/services/watermark_cut_tests.rs"),
    ("tool.rs", "tool_policy/value_objects/tool.rs"),
    ("tool_tests.rs", "tool_policy/value_objects/tool_tests.rs"),
    ("tool_descriptor.rs", "tool_policy/value_objects/tool_descriptor.rs"),
    ("tool_descriptor_tests.rs", "tool_policy/value_objects/tool_descriptor_tests.rs"),
    ("tool_id.rs", "tool_policy/value_objects/tool_id.rs"),
    ("tool_id_tests.rs", "tool_policy/value_objects/tool_id_tests.rs"),
    ("extension_tool.rs", "tool_policy/value_objects/extension_tool.rs"),
    ("extension_tool_tests.rs", "tool_policy/value_objects/extension_tool_tests.rs"),
    ("tool_policy.rs", "tool_policy/services/tool_policy.rs"),
    ("tool_policy_tests.rs", "tool_policy/services/tool_policy_tests.rs"),
    ("tool_policy_catalogue.rs", "tool_policy/services/tool_policy_catalogue.rs"),
    ("tool_policy_catalogue_tests.rs", "tool_policy/services/tool_policy_catalogue_tests.rs"),
    ("catalogue.rs", "catalogue/value_objects/catalogue.rs"),
    ("catalogue_tests.rs", "catalogue/value_objects/catalogue_tests.rs"),
    ("catalogue_window_tests.rs", "catalogue/value_objects/catalogue_window_tests.rs"),
    ("catalogue/effort_vocabulary.rs", "catalogue/value_objects/effort_vocabulary.rs"),
    ("catalogue/retired.rs", "catalogue/value_objects/retired.rs"),
    ("catalogue/retired_tests.rs", "catalogue/value_objects/retired_tests.rs"),
    ("state_snapshot.rs", "sessions/value_objects/state_snapshot.rs"),
    ("state_snapshot_tests.rs", "sessions/value_objects/state_snapshot_tests.rs"),
    ("state_snapshot_reader.rs", "sessions/value_objects/state_snapshot_reader.rs"),
];

#[test]
fn l5_all_47_moves_have_destinations_and_retire_sources() {
    assert_eq!(L5_MOVES.len(), 47);
    let root = domain();
    let mut failures = Vec::new();
    for (old, new) in L5_MOVES {
        if root.join(new).is_file() {
        } else {
            failures.push(format!("missing destination {new}"));
        }
        match root.join(old).try_exists() {
            Ok(false) => {}
            Ok(true) => failures.push(format!("retained source {old}")),
            Err(error) => failures.push(format!("cannot inspect {old}: {error}")),
        }
    }
    assert!(failures.is_empty(), "L5 exact moves: {failures:#?}");
}

#[test]
fn l5_moved_tests_remain_siblings_of_their_production_module() {
    let root = domain();
    let mut failures = Vec::new();
    for (_, test) in L5_MOVES
        .iter()
        .filter(|(_, new)| new.ends_with("_tests.rs"))
    {
        let parent = Path::new(test).parent().expect("test has role parent");
        let production = L5_MOVES
            .iter()
            .filter(|(old, new)| {
                matches!(old.strip_suffix("_tests.rs"), None)
                    && Path::new(new).parent() == Some(parent)
            })
            .any(|(_, production)| {
                let stem = Path::new(production).file_stem().unwrap().to_str().unwrap();
                Path::new(test)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(&format!("{stem}_"))
                    && root.join(production).is_file()
            });
        if production && root.join(test).is_file() {
        } else {
            failures.push(format!("missing production/test sibling pair {test}"));
        }
    }
    assert!(failures.is_empty(), "L5 sibling pairs: {failures:#?}");
}

#[test]
fn l5_new_roles_have_at_least_two_approved_production_modules() {
    let root = domain();
    let mut failures = Vec::new();
    for role in [
        "conversation/value_objects",
        "conversation/services",
        "tool_policy/value_objects",
        "tool_policy/services",
        "catalogue/value_objects",
    ] {
        let approved: Vec<_> = L5_MOVES
            .iter()
            .filter(|(old, new)| {
                matches!(old.strip_suffix("_tests.rs"), None)
                    && Path::new(new).parent() == Some(Path::new(role))
            })
            .map(|(_, new)| *new)
            .chain(
                ["conversation/value_objects/user_kind.rs"]
                    .into_iter()
                    .filter(|new| Path::new(new).parent() == Some(Path::new(role))),
            )
            .collect();
        assert!(approved.len() >= 2, "invalid role allowlist {role}");
        let actual = approved
            .iter()
            .filter(|path| root.join(path).is_file())
            .count();
        if actual >= 2 {
        } else {
            failures.push(format!(
                "{role}: approved production modules {actual}, minimum 2"
            ));
        }
    }
    assert!(failures.is_empty(), "L5 role population: {failures:#?}");
}

#[test]
fn l5_user_kind_and_snapshot_placement_is_explicit() {
    let root = domain();
    let expected = [
        "conversation/value_objects/user_kind.rs",
        "sessions/value_objects/state_snapshot.rs",
        "sessions/value_objects/state_snapshot_reader.rs",
        "sessions/value_objects/state_snapshot_tests.rs",
    ];
    let missing: Vec<_> = expected
        .into_iter()
        .filter_map(|path| {
            if root.join(path).is_file() {
                None
            } else {
                Some(path)
            }
        })
        .collect();
    assert!(
        missing.is_empty(),
        "missing approved UserKind/session snapshot placement: {missing:?}"
    );
}
