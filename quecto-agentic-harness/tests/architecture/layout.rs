//! Filename-based, immediate-child layout ratchets for #2358.
//! Include mod.rs and cfg(test) support names; exclude only *_tests.rs.
use std::collections::BTreeSet;
use std::io;
use std::path::Path;
// Explicit source-policy pair: relative path and exact expected count.
struct Budget<'a>(&'a str, usize);
struct Capability<'a> {
    path: &'a str,
    allowed_roles: &'a [&'a str],
}
#[derive(PartialEq, Eq)]
enum Violation {
    BudgetMismatch {
        path: String,
        actual: usize,
        expected: usize,
    },
    StrayEntry(String),
    Inspection {
        path: String,
        reason: String,
    },
    UndeclaredRole(String),
    EmptyRole(String),
    StalePlacement(String),
    UnlistedPlacement(String),
}
impl std::fmt::Debug for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetMismatch {
                path,
                actual,
                expected,
            } => {
                write!(f, "{path}: actual {actual}, expected {expected}; ")?;
                if actual < expected {
                    write!(f, "lower {path} to {actual} in BUDGETS in this PR. {WIKI}")
                } else {
                    write!(
                        f,
                        "move growth into the target capability/role directory in an authorized layout slice; do not raise BUDGETS. {WIKI}"
                    )
                }
            }
            Self::StrayEntry(path) => write!(
                f,
                "{path}: migrated shape violation; move {path} into a declared role directory; keep only mod.rs and mod_tests.rs at the capability root. {WIKI}"
            ),
            Self::UndeclaredRole(path) => {
                let (capability, name) = path.rsplit_once('/').expect("role has capability parent");
                write!(
                    f,
                    "{path}: undeclared role directory; add {name} to {capability} allowed_roles in MIGRATED or remove the directory. {WIKI}"
                )
            }
            Self::EmptyRole(path) => write!(
                f,
                "{path}: empty role; put a .rs file anywhere beneath {path} or remove the unused role directory. {WIKI}"
            ),
            Self::StalePlacement(path) => write!(
                f,
                "{path}: stale transitional placement; remove {path} from PLACEMENTS when its directory has been retired. {WIKI}"
            ),
            Self::UnlistedPlacement(path) => write!(
                f,
                "{path}: unlisted immediate capability directory; add a placement row with this capability's wiki section and classify it as target, transitional, or testing, or move the directory into a listed placement. {WIKI}"
            ),
            Self::Inspection { path, reason } => write!(
                f,
                "{path}: inspection failed ({reason}); restore {path} as a readable real directory/file tree; descendant symlinks are not inspected. {WIKI}"
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
const BUDGETS: &[Budget<'_>] = &[
    Budget("domain", 71),
    Budget("application", 45),
    Budget("interface", 6),
    Budget("infrastructure", 28),
    Budget("composition", 24),
    Budget("interface/cli", 106),
    Budget("infrastructure/tools", 122),
    Budget("infrastructure/persistence", 36),
    Budget("application/swarm", 23),
    Budget("domain/swarm", 15),
];
// L0 moves nothing: future slices explicitly opt capabilities into strict shape.
const MIGRATED: &[Capability<'_>] = &[];
#[derive(PartialEq, Eq)]
enum Classification {
    Target,
    Transitional,
    Testing,
}
struct Placement<'a> {
    layer: &'a str,
    name: &'a str,
    _citation: &'a str,
    classification: Classification,
}
// Compile-time expansion: explicit names and wiki ownership, never runtime name generation.
macro_rules! placement_table {
    ($(($layer:literal, $citation:literal, $kind:ident) => [$($name:literal),*];)*) => {
        const PLACEMENTS: &[Placement<'static>] = &[$($(Placement {
            layer: $layer, name: $name, _citation: $citation,
            classification: Classification::$kind,
        },)*)*];
    };
}
placement_table! {
    ("domain", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#domain", Target) => ["conversation", "admission", "environments", "catalogue", "tool_policy", "sessions", "agents", "audit", "inference", "commander", "identity", "shared", "external_agent", "swarm", "workflow"];
    ("application", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#application", Target) => ["admission", "agent_turn", "audit", "catalogue", "configuration", "environments", "extensions", "external_agent", "provider_runtime", "providers", "search", "sessions", "subagents", "swarm", "tools", "workflow", "agent_commander", "shared"];
    ("interface", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#interface", Target) => ["cli", "repl", "tools", "uds"];
    ("infrastructure", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#infrastructure", Target) => ["admission", "auth", "config", "extensions", "external_agents", "http", "persistence", "processes", "providers", "search", "security", "tools", "workspace", "judgment", "observability", "time"];
    ("composition", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#composition", Target) => ["bootstrap", "runtime", "logging", "shutdown"];
    ("domain", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#capability-coverage-and-naming", Transitional) => ["environment_registry"];
    ("application", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#capability-coverage-and-naming", Transitional) => ["agent_loop"];
    ("infrastructure", "https://github.com/platform-q-ai/quecto/wiki/Agentic-Harness-Target-Architecture#capability-coverage-and-naming", Testing) => ["test_support"];
}
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
// Canonicalize only the caller root; check descendants without following links.
fn checked_directory(root: &Path, relative: &str) -> io::Result<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        real_directory(&path)?;
    }
    Ok(path)
}
fn canonical_root(root: &Path) -> io::Result<std::path::PathBuf> {
    let root = std::fs::canonicalize(root)?;
    real_directory(&root)?;
    Ok(root)
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
                self.failures.push(Violation::Inspection {
                    path: relative.into(),
                    reason: error.to_string(),
                })
            })
            .ok()
    }
    // Visit all descendants, including those after a Rust file: any inspection error fails closed.
    fn role_has_rust(&mut self, relative: &str) -> Option<bool> {
        let entries = self.inspect(relative)?;
        let mut populated = false;
        let mut complete = true;
        for entry in entries {
            match entry.1 {
                Kind::File => populated |= regular_rust(&entry),
                Kind::Directory => match self.role_has_rust(&format!("{relative}/{}", entry.0)) {
                    Some(found) => populated |= found,
                    None => complete = false,
                },
            }
        }
        complete.then_some(populated)
    }
    fn layout(&mut self, budgets: &[Budget<'_>], migrated: &[Capability<'_>]) {
        for budget in budgets {
            if let Some(entries) = self.inspect(budget.0) {
                let actual = entries
                    .iter()
                    .filter(|entry| match entry.0.strip_suffix("_tests.rs") {
                        Some(_) => false,
                        None => regular_rust(entry),
                    })
                    .count();
                if actual == budget.1 {
                    continue;
                }
                self.failures.push(Violation::BudgetMismatch {
                    path: budget.0.into(),
                    actual,
                    expected: budget.1,
                });
            }
        }
        for capability in migrated {
            if let Some(entries) = self.inspect(capability.path) {
                for entry in entries {
                    let path = format!("{}/{}", capability.path, entry.0);
                    match entry.1 {
                        Kind::File if matches!(entry.0.as_str(), "mod.rs" | "mod_tests.rs") => {}
                        Kind::Directory if capability.allowed_roles.contains(&entry.0.as_str()) => {
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
        let paths: BTreeSet<_> = rows.iter().map(|row| (row.layer, row.name)).collect();
        for layer in LAYERS {
            if let Some(entries) = self.inspect(layer) {
                for row in rows.iter().filter(|row| {
                    row.layer == *layer && row.classification == Classification::Transitional
                }) {
                    if entries
                        .iter()
                        .any(|entry| entry.0 == row.name && matches!(entry.1, Kind::Directory))
                    {
                        continue;
                    }
                    self.failures
                        .push(Violation::StalePlacement(format!("{layer}/{}", row.name)));
                }
                for entry in entries {
                    if matches!(entry.1, Kind::Directory) {
                        let path = format!("{layer}/{}", entry.0);
                        if paths.contains(&(*layer, entry.0.as_str())) {
                            continue;
                        }
                        self.failures.push(Violation::UnlistedPlacement(path));
                    }
                }
            }
        }
    }
}
fn validate(root: &Path, budgets: &[Budget<'_>], migrated: &[Capability<'_>]) -> Vec<Violation> {
    validate_all(root, budgets, migrated, None, &scan)
}
// One canonicalization boundary for a complete inspection, including placements.
fn validate_all(
    root: &Path,
    budgets: &[Budget<'_>],
    migrated: &[Capability<'_>],
    rows: Option<&[Placement<'_>]>,
    scanner: &impl Fn(&Path) -> io::Result<Vec<Entry>>,
) -> Vec<Violation> {
    let root = match canonical_root(root) {
        Ok(root) => root,
        Err(error) => {
            return vec![Violation::Inspection {
                path: root.display().to_string(),
                reason: error.to_string(),
            }];
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
        Violation::Inspection { path, .. } => reported.insert(path.clone()),
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
    let keys: BTreeSet<_> = BUDGETS
        .iter()
        .map(|row| ("budget", "", row.0))
        .chain(MIGRATED.iter().map(|row| ("migrated", "", row.path)))
        .chain(rows.iter().map(|row| ("placement", row.layer, row.name)))
        .collect();
    assert_eq!(
        keys.len(),
        BUDGETS.len() + MIGRATED.len() + rows.len(),
        "duplicate source-policy row"
    );
    let failures = validate_all(&root, BUDGETS, MIGRATED, Some(rows), &scan);
    assert!(
        failures.is_empty(),
        "layout policy violations: {failures:#?}"
    );
}
#[path = "layout/fixtures.rs"]
mod fixtures;
