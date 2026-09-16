//! Epic #1929 follow-up (#1666): composition is the only place that
//! constructs the concrete runtime graph. Infrastructure's inbound event
//! sources (reapers, socket monitors, cleanup workers) and the interface's
//! dispatch loops invoke application use cases through handles composition
//! injects; none of them constructs a use case or assembles a graph.
//!
//! What is checked, per production file under `src/infrastructure` and
//! `src/interface`:
//! - the inventory is tree-derived: every `pub struct` under any
//!   `use_cases/` folder of the application, whatever its field shape, and
//!   the hand-written [`USE_CASES`] / [`LEGACY_RECORDS`] lists must equal it;
//! - `use … as` and `type … =` aliases are resolved to their targets first,
//!   so an alias of a use case is matched like the use case;
//! - a use case (or alias) may be named only as a handle — in a type
//!   position — never followed by `::new(`, `::default(`, `::from(`, a
//!   struct literal, or made constructible through `impl From<…> for` it;
//! - the legacy use cases that live outside `use_cases/` folders are an
//!   exact, non-growing baseline ([`LEGACY_CONSTRUCTIONS`]) tracked by #1666.
//!
//! The text scan does not see a construction spread over two lines with
//! the path on one and `::new(` on the next, a struct literal at the start
//! of a line (indistinguishable from a pattern), spaced `::` or `Self::new`
//! inside an `impl UseCase`. The parse-based scan at the end of this file
//! (D10 #1979) closes those on the syntax tree; a constructor reached
//! through a macro invocation's tokens stays the one documented evasion of
//! both scans.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// Every use case and ports bundle declared under an application
/// `use_cases` folder.
const USE_CASES: &[&str] = &[
    "ClearConversation",
    "CompensateFailedLaunch",
    "CompensateFailedLaunchPorts",
    "DepartingChildren",
    "ExecuteHarnessShutdown",
    "ExecuteHarnessShutdownPorts",
    "ExportSessionReport",
    "FinalizeEnvironmentMember",
    "FindUseCase",
    "HarnessShutdownTransaction",
    "KillDelegatedAgent",
    "KillDelegatedAgentPorts",
    "KillEnvironment",
    "ListEnvironmentsQuery",
    "ListRetainedContext",
    "ListSessions",
    "ObserveOwnedChildExit",
    "OwnerConclusionPorts",
    "PrepareHarnessShutdown",
    "ReadHistory",
    "RecallContext",
    "RecoverMessage",
    "ResumeSavedSession",
    "RetainContext",
    "RewindConversation",
    "SaveSession",
    "SettleDelegatedChild",
    "SettleDelegatedChildPorts",
    "StartFreshConversation",
    "SynchronizeTranscript",
    "TerminateAllDelegatedAgents",
    "TerminateAllDelegatedAgentsPorts",
    "TerminateDelegatedAgent",
    "WebFetchUseCase",
];

/// Request/response records declared beside the pre-#1929 find and
/// web-fetch use cases (their capability has no `dto` folder yet, #1666).
/// A port adapter builds these as the use case's inputs and outputs; they
/// are values, not graph nodes, and are the only structs of the tree an
/// adapter may construct. Exact: a new record joins here or the test fails.
const LEGACY_RECORDS: &[&str] = &[
    "FetchRequest",
    "FindOutput",
    "FindPathsRequest",
    "FindRequest",
    "FindResult",
    "HttpStatus",
    "KilledEnvironment",
    "ParsedHttpUrl",
];

/// The use cases declared outside a `use_cases/` folder that the interface
/// and infrastructure still construct themselves (pre-#1929, tracked by
/// #1666): (file, construction). Exact and non-growing: a construction
/// that moves to composition leaves the list; nothing may join it.
const LEGACY_CONSTRUCTIONS: &[(&str, &str)] = &[
    (
        "src/interface/catalogue_runtime.rs",
        "ComposeProviderRuntimeUseCase::new(",
    ),
    (
        "src/interface/catalogue_runtime.rs",
        "ResolveModelSelectionUseCase::new(",
    ),
    (
        "src/interface/cli/uds_models.rs",
        "QueryCatalogueUseCase::new(",
    ),
    (
        "src/infrastructure/tools/spawn.rs",
        "SubagentLaunchUseCase::new(",
    ),
];

fn ident_at_start(rest: &str) -> String {
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Every `pub struct` under any `use_cases/` folder of the application
/// tree, whatever its fields or generics.
fn use_case_tree_inventory() -> BTreeSet<String> {
    let mut files = Vec::new();
    walk(Path::new("src/application"), &mut files);
    let mut names = BTreeSet::new();
    for path in files.iter().filter(|path| {
        path.contains("/use_cases/") && !path.ends_with("_tests.rs") && !path.contains("fakes")
    }) {
        for (_, line) in production_code(path) {
            if let Some(rest) = line.trim_start().strip_prefix("pub struct ") {
                let name = ident_at_start(rest);
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }
    names
}

/// The use cases and ports bundles: the tree inventory minus the legacy
/// records, both hand lists asserted against the tree.
fn use_case_names() -> BTreeSet<String> {
    let tree = use_case_tree_inventory();
    let records: BTreeSet<String> = LEGACY_RECORDS.iter().map(|s| s.to_string()).collect();
    let use_cases: BTreeSet<String> = USE_CASES.iter().map(|s| s.to_string()).collect();
    assert!(
        records.is_subset(&tree),
        "LEGACY_RECORDS names a struct the tree no longer declares: shrink the list"
    );
    let expected: BTreeSet<String> = tree.difference(&records).cloned().collect();
    assert_eq!(
        use_cases, expected,
        "the use-case inventory changed (every `pub struct` under a use_cases/ folder, minus LEGACY_RECORDS); update USE_CASES"
    );
    use_cases
}

/// The raw aliases a file declares: `use …::X as Y;` (any visibility —
/// `pub use`, `pub(crate) use`, `pub(super) use`), a grouped `{X as Y}`,
/// and `type Y = …::X;` (with or without generics), mapped alias → target.
fn raw_aliases_in(code: &[(usize, String)]) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    for (_, line) in code {
        let trimmed = line.trim_start();
        let is_use = trimmed.starts_with("use ")
            || trimmed.starts_with("pub use ")
            || (trimmed.starts_with("pub(") && trimmed.contains(" use "));
        if is_use {
            for (index, _) in line.match_indices(" as ") {
                let target = line[..index]
                    .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let alias = ident_at_start(&line[index + 4..]);
                if !alias.is_empty() {
                    aliases.insert(alias, target);
                }
            }
        }
        let type_decl = trimmed
            .strip_prefix("pub(crate) type ")
            .or_else(|| trimmed.strip_prefix("pub(super) type "))
            .or_else(|| trimmed.strip_prefix("pub type "))
            .or_else(|| trimmed.strip_prefix("type "));
        if let Some(rest) = type_decl {
            let alias = ident_at_start(rest);
            if let Some((_, target)) = rest.split_once('=') {
                let target = target.trim().trim_end_matches(';');
                let target = target.split('<').next().unwrap_or_default();
                let target = target.rsplit("::").next().unwrap_or_default().to_string();
                if !alias.is_empty() && !target.is_empty() {
                    aliases.insert(alias, target);
                }
            }
        }
    }
    aliases
}

/// The raw aliases of every production file of the crate, computed once:
/// an alias declared in one file and used in another (a `pub(crate) use
/// … as Hidden;` re-export) resolves wherever it is spelled. Fail-closed:
/// every alias of the crate counts in every file, whatever its scope.
fn crate_raw_aliases() -> &'static BTreeMap<String, String> {
    use std::sync::OnceLock;
    static ALIASES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    ALIASES.get_or_init(|| {
        let mut aliases = BTreeMap::new();
        for path in production_files() {
            aliases.extend(raw_aliases_in(&production_code(&path)));
            // The syntax tree sees a rename inside a multiline `use {…}`
            // group, which the line scan cannot.
            aliases.extend(syn_aliases(&std::fs::read_to_string(&path).unwrap()));
        }
        aliases
    })
}

/// The aliases of `code` plus the crate-wide ones, resolved transitively to
/// an inventory name (alias → target).
fn aliases_in(code: &[(usize, String)], names: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut aliases = crate_raw_aliases().clone();
    aliases.extend(raw_aliases_in(code));
    resolve_aliases(&aliases, names)
}

/// Resolve chains (an alias of an alias) to the inventory name.
fn resolve_aliases(
    aliases: &BTreeMap<String, String>,
    names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut resolved = BTreeMap::new();
    for alias in aliases.keys() {
        let mut target = alias.clone();
        for _ in 0..aliases.len() + 1 {
            match aliases.get(&target) {
                Some(next) => target = next.clone(),
                None => break,
            }
        }
        if names.contains(&target) && &target != alias {
            resolved.insert(alias.clone(), target);
        }
    }
    resolved
}

fn at_word_boundary_before(line: &str, index: usize) -> bool {
    line[..index]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
}

/// `line` constructs `name`: `name::new(` / `name::default(` / `name::from(`,
/// a struct literal `name {` in expression position (after `=`, `(`, `,`,
/// `[` or `>`), or an `impl From<…> for name` that makes `.into()` build it.
fn constructs(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("impl") && trimmed.contains("From<") {
        // `for <path>` names the constructed type, possibly fully
        // qualified: compare its last segment.
        if let Some(index) = trimmed.find("> for ") {
            let target = trimmed[index + "> for ".len()..]
                .split(|c: char| c == '{' || c == '<' || c.is_whitespace())
                .next()
                .unwrap_or_default();
            return target.rsplit("::").next() == Some(name);
        }
    }
    line.match_indices(name).any(|(index, _)| {
        if !at_word_boundary_before(line, index) {
            return false;
        }
        let after = &line[index + name.len()..];
        if after.starts_with("::new(")
            || after.starts_with("::default(")
            || after.starts_with("::from(")
        {
            return true;
        }
        after.starts_with(" {")
            && line[..index]
                .trim_end()
                .ends_with(['=', '(', ',', '[', '>'])
    })
}

/// Every construction of an inventory name (or an alias of one) in the
/// production files under `layer`.
fn constructions_in(layer: &str, names: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    for path in production_files()
        .into_iter()
        .filter(|path| path.starts_with(layer))
    {
        let code = production_code(&path);
        let aliases = aliases_in(&code, names);
        for (line_no, line) in &code {
            for name in names {
                if constructs(line, name) {
                    found.push(format!(
                        "{path}:{line_no} constructs `{name}`: `{}`",
                        line.trim()
                    ));
                }
            }
            for (alias, target) in &aliases {
                if constructs(line, alias) {
                    found.push(format!(
                        "{path}:{line_no} constructs `{target}` through alias `{alias}`: `{}`",
                        line.trim()
                    ));
                }
            }
        }
    }
    found
}

/// (a) No production file under `src/infrastructure` or `src/interface`
/// constructs a use case or its ports bundle, directly, through an alias,
/// or through a `From` impl; the fully qualified `use_cases::…::new(`
/// spelling is refused outright. Adapters hold `Arc<UseCase>` handles (or
/// a builder alias composition fills) and invoke them.
#[test]
fn infrastructure_and_interface_never_construct_a_use_case() {
    let names = use_case_names();
    let mut violations = constructions_in("src/infrastructure", &names);
    violations.extend(constructions_in("src/interface", &names));
    assert!(
        violations.is_empty(),
        "use cases are constructed in composition only:\n{}",
        violations.join("\n")
    );
    for layer in ["src/infrastructure", "src/interface"] {
        for path in production_files()
            .into_iter()
            .filter(|path| path.starts_with(layer))
        {
            for (line_no, line) in production_code(&path) {
                assert!(
                    !(line.contains("use_cases::") && line.contains("::new(")),
                    "{path}:{line_no} names `use_cases::…::new(`: `{}`",
                    line.trim()
                );
            }
        }
    }
}

/// (b) Composition is the only layer outside the application itself that
/// constructs a use case: the application may compose a sibling use case
/// from the ports it was built with, and composition builds the rest.
/// Nothing else in the crate (entry points, lib) does.
#[test]
fn composition_is_the_only_layer_that_constructs_use_cases() {
    let names = use_case_names();
    let mut outside = Vec::new();
    let mut in_composition = 0usize;
    for path in production_files() {
        let code = production_code(&path);
        let aliases = aliases_in(&code, &names);
        for (line_no, line) in &code {
            let built = names.iter().any(|name| constructs(line, name))
                || aliases.keys().any(|alias| constructs(line, alias));
            if !built {
                continue;
            }
            if path.starts_with("src/composition/") {
                in_composition += 1;
            } else if !path.starts_with("src/application/") {
                outside.push(format!("{path}:{line_no}: `{}`", line.trim()));
            }
        }
    }
    assert!(outside.is_empty(), "{}", outside.join("\n"));
    assert!(
        in_composition >= 10,
        "composition builds the graph ({in_composition} constructions seen)"
    );
    // The teardown, lifecycle and environment graphs each have a home.
    for (file, builder) in [
        (
            "src/composition/subagent_teardown.rs",
            "pub fn build_teardown_graph",
        ),
        (
            "src/composition/subagent_termination.rs",
            "pub fn install_termination_owners",
        ),
        (
            "src/composition/subagent_lifecycle.rs",
            "pub fn build_lifecycle_use_cases",
        ),
        (
            "src/composition/environments.rs",
            "pub fn build_environment_control",
        ),
        (
            "src/composition/environments.rs",
            "pub fn build_member_finalizer",
        ),
    ] {
        assert!(
            production_code(file)
                .iter()
                .any(|(_, l)| l.contains(builder)),
            "{file} declares `{builder}`"
        );
    }
}

/// The use cases declared outside a `use_cases/` folder (`…UseCase`
/// structs of the pre-#1929 catalogue, provider-runtime and launch
/// modules) that the interface and infrastructure still construct are
/// exactly [`LEGACY_CONSTRUCTIONS`]: each is present once, and no other
/// `…UseCase::new(` / `::default(` appears in either layer.
#[test]
fn legacy_use_case_constructions_outside_use_cases_folders_do_not_grow() {
    let mut seen = BTreeSet::new();
    let mut unexpected = Vec::new();
    for layer in ["src/infrastructure", "src/interface"] {
        for path in production_files()
            .into_iter()
            .filter(|path| path.starts_with(layer))
        {
            for (line_no, line) in production_code(&path) {
                for (index, _) in line.match_indices("UseCase::") {
                    let after = &line[index + "UseCase::".len()..];
                    if !(after.starts_with("new(") || after.starts_with("default(")) {
                        continue;
                    }
                    let name_start = line[..index]
                        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .map_or(0, |i| i + 1);
                    let construction = &line[name_start..index + "UseCase::".len() + 4];
                    match LEGACY_CONSTRUCTIONS
                        .iter()
                        .find(|(file, needle)| *file == path && construction.starts_with(needle))
                    {
                        Some(entry) => {
                            seen.insert(*entry);
                        }
                        None => unexpected.push(format!(
                            "{path}:{line_no} constructs a use case outside the #1666 baseline: `{}`",
                            line.trim()
                        )),
                    }
                }
            }
        }
    }
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
    let baseline: BTreeSet<(&str, &str)> = LEGACY_CONSTRUCTIONS.iter().copied().collect();
    assert_eq!(
        seen, baseline,
        "a baselined legacy construction moved or disappeared; update LEGACY_CONSTRUCTIONS"
    );
}

/// The interface declares the handles it needs as plain structs and a
/// builder alias, constructed by composition and passed down through
/// `CliComposition`; it never names composition and never assembles a
/// graph. The old graph module is gone without a re-export shim.
#[test]
fn interface_declares_handles_and_composition_fills_them() {
    assert!(
        !Path::new("src/interface/cli/uds_teardown_graph.rs").exists(),
        "the interface graph module was retired"
    );
    let handles = production_code("src/interface/cli/uds_teardown_handles.rs");
    for declared in [
        "pub struct TeardownLoopInputs {",
        "pub struct TeardownHandles {",
        "pub fleet: Arc<TerminateAllDelegatedAgents>,",
        "pub controller: Arc<SubagentTeardownController>,",
    ] {
        assert!(
            handles.iter().any(|(_, l)| l.contains(declared)),
            "uds_teardown_handles.rs declares `{declared}`"
        );
    }
    for forbidden in ["::new(", "crate::composition", "TeardownGraph"] {
        assert!(
            !handles.iter().any(|(_, l)| l.contains(forbidden)),
            "uds_teardown_handles.rs holds handles only, never `{forbidden}`"
        );
    }
    let cli = production_code("src/interface/cli/mod.rs");
    assert!(
        cli.iter()
            .any(|(_, l)| l.contains("pub type TeardownHandlesBuilder ="))
    );
    assert!(
        cli.iter()
            .any(|(_, l)| l.contains("pub teardown_graph: TeardownHandlesBuilder,")),
        "CliComposition carries the handles builder"
    );
    let composition = production_code("src/composition/subagent_teardown.rs");
    assert!(composition.iter().any(|(_, l)| {
        l.contains("pub fn build_teardown_graph(inputs: TeardownLoopInputs) -> TeardownHandles")
    }));
    // The per-loop handles are built by the composition-provided builder
    // the loop was handed, with the loop's own runtime inputs.
    let multi = production_code("src/interface/cli/uds_multi.rs");
    assert!(
        multi
            .iter()
            .any(|(_, l)| l.contains("build(super::uds_teardown_handles::TeardownLoopInputs {")),
        "the loop calls composition's builder with its runtime inputs"
    );
    // Infrastructure's inbound event sources hold handles or slots.
    let spawn = production_code("src/infrastructure/tools/spawn_lifecycle.rs");
    assert!(
        spawn
            .iter()
            .any(|(_, l)| l.contains("pub fn with_lifecycle_slot("))
    );
    let wiring = production_code("src/infrastructure/tools/subagent_teardown_wiring.rs");
    assert!(
        wiring
            .iter()
            .any(|(_, l)| l.contains("pub struct SubagentLifecycleSlot("))
    );
    assert!(
        !wiring
            .iter()
            .any(|(_, l)| l.contains("fn build_lifecycle_use_cases")),
        "the wiring module declares handles, composition builds them"
    );
    let cleanup = production_code("src/infrastructure/tools/subagent_cleanup.rs");
    assert!(cleanup.iter().any(|(_, l)| {
        l.contains(
            "pub type MemberFinalizer = fn(EnvironmentRegistry) -> FinalizeEnvironmentMember;",
        )
    }));
    let native = production_code("src/infrastructure/extensions/native.rs");
    assert!(
        native.iter().any(|(_, l)| {
            l.contains(".with_lifecycle_slot(termination_slots.lifecycle.clone())")
        })
    );
    assert!(native.iter().any(|(_, l)| {
        l.contains(".with_environment_control_slot(termination_slots.environments.clone())")
    }));
}

/// The scan itself: each evasion shape is recognised, and handle positions
/// are not.
#[test]
fn construction_scan_recognises_aliases_literals_and_from_impls() {
    let names: BTreeSet<String> = ["KillEnvironment".to_string()].into_iter().collect();
    let code = |src: &str| -> Vec<(usize, String)> {
        src.lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.to_string()))
            .collect()
    };
    let aliased = code(
        "use crate::application::environments::use_cases::KillEnvironment as Y;\nlet k = Y::new(a, b, c);",
    );
    let aliases = aliases_in(&aliased, &names);
    assert_eq!(
        aliases.get("Y").map(String::as_str),
        Some("KillEnvironment")
    );
    assert!(constructs(&aliased[1].1, "Y"));
    let typed = code(
        "type Q = crate::application::environments::use_cases::KillEnvironment;\nlet k = Q::default();",
    );
    assert_eq!(
        aliases_in(&typed, &names).get("Q").map(String::as_str),
        Some("KillEnvironment")
    );
    assert!(constructs(&typed[1].1, "Q"));
    let grouped = code(
        "use crate::application::environments::use_cases::{KillEnvironment as K, ListEnvironmentsQuery};",
    );
    assert_eq!(
        aliases_in(&grouped, &names).get("K").map(String::as_str),
        Some("KillEnvironment")
    );
    assert!(constructs(
        "impl From<Parts> for KillEnvironment {",
        "KillEnvironment"
    ));
    assert!(constructs(
        "impl From<Parts> for crate::application::environments::use_cases::KillEnvironment {",
        "KillEnvironment"
    ));
    assert!(constructs(
        "impl<T> From<T> for KillEnvironment where T: Into<Parts> {",
        "KillEnvironment"
    ));
    assert!(constructs(
        "let k = KillEnvironment::from(parts);",
        "KillEnvironment"
    ));
    assert!(constructs(
        "    let ports = KillDelegatedAgentPorts {",
        "KillDelegatedAgentPorts"
    ));
    assert!(constructs(
        "    Arc::new(KillEnvironment {",
        "KillEnvironment"
    ));
    for handle in [
        "    kill: Option<Arc<KillEnvironment>>,",
        "fn kill(&self, uc: &KillEnvironment) {",
        "    KillEnvironment { .. } => (),",
        "    Command::KillEnvironment { id } => id,",
        "impl From<KillEnvironment> for Other {",
        "impl Debug for KillEnvironment {",
    ] {
        assert!(!constructs(handle, "KillEnvironment"), "{handle}");
    }
}

// ─── D10 (#1979): the parse-based construction scan ──────────────────────
//
// The line scan above documents its evasions (a construction spread over
// lines, a struct literal at the start of a line, spaced `::`). This scan
// parses each production file and resolves constructions on the syntax
// tree instead, so whitespace, line breaks, `Self::new(` inside an
// `impl UseCase`, `<UseCase>::new(`, `<UseCase as Trait>::from(` and a
// constructor passed as a value (`.unwrap_or_else(UseCase::default)`) are
// all seen. `use … as` renames and `type` aliases are resolved
// transitively and fail-closed (every rename in the file counts, whatever
// its scope). Tokens inside macro invocations are not parsed by either
// scan; that evasion stays documented.

/// The constructor names a graph node is built through.
const CONSTRUCTORS: &[&str] = &["new", "default", "from"];

struct AstConstructions<'a> {
    names: &'a BTreeSet<String>,
    aliases: BTreeMap<String, String>,
    impl_types: Vec<String>,
    found: Vec<String>,
}

impl AstConstructions<'_> {
    /// The inventory name `ident` stands for, through any alias chain.
    fn resolve(&self, ident: &str) -> Option<String> {
        let mut current = ident.to_string();
        for _ in 0..=self.aliases.len() {
            if self.names.contains(&current) {
                return Some(current);
            }
            current = self.aliases.get(&current)?.clone();
        }
        None
    }

    /// `Self` stands for the enclosing impl's type; anything else for itself.
    fn segment_name(&self, ident: &str) -> Option<String> {
        if ident == "Self" {
            self.impl_types.iter().find_map(|ty| self.resolve(ty))
        } else {
            self.resolve(ident)
        }
    }

    fn type_name(&self, ty: &syn::Type) -> Option<String> {
        match ty {
            syn::Type::Paren(ty) => self.type_name(&ty.elem),
            syn::Type::Group(ty) => self.type_name(&ty.elem),
            syn::Type::Path(ty) => match &ty.qself {
                Some(qself) => self.type_name(&qself.ty),
                None => self.segment_name(&ty.path.segments.last()?.ident.to_string()),
            },
            _ => None,
        }
    }

    /// The inventory name a path names, when it is a constructor path:
    /// `Name::new`, `alias::default`, `Self::new`, `<Name>::from`,
    /// `<Name as Trait>::new`.
    fn constructor_owner(&self, qself: Option<&syn::QSelf>, path: &syn::Path) -> Option<String> {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let last = segments.last()?;
        if !CONSTRUCTORS.contains(&last.as_str()) {
            return None;
        }
        match qself {
            Some(qself) => self.type_name(&qself.ty),
            None => segments
                .len()
                .checked_sub(2)
                .and_then(|owner| self.segment_name(&segments[owner])),
        }
    }

    fn record(&mut self, what: String) {
        if !self.found.contains(&what) {
            self.found.push(what);
        }
    }
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

fn path_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Every rename (`… as Y`, any visibility, grouped or not, multiline or
/// not) and `type Y = …::X` alias of a parsed file, alias → target.
fn syn_aliases_of(file: &syn::File) -> BTreeMap<String, String> {
    use syn::visit::Visit;
    struct Aliases(BTreeMap<String, String>);
    impl<'ast> Visit<'ast> for Aliases {
        fn visit_use_rename(&mut self, item: &'ast syn::UseRename) {
            self.0
                .insert(item.rename.to_string(), item.ident.to_string());
        }
        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            if let syn::Type::Path(ty) = &*item.ty
                && let Some(last) = ty.path.segments.last()
            {
                self.0
                    .insert(item.ident.to_string(), last.ident.to_string());
            }
            syn::visit::visit_item_type(self, item);
        }
    }
    let mut aliases = Aliases(BTreeMap::new());
    aliases.visit_file(file);
    aliases.0
}

fn syn_aliases(source: &str) -> BTreeMap<String, String> {
    syn_aliases_of(&syn::parse_file(source).expect("production source parses"))
}

impl<'ast> syn::visit::Visit<'ast> for AstConstructions<'_> {
    fn visit_file(&mut self, file: &'ast syn::File) {
        self.aliases.extend(syn_aliases_of(file));
        syn::visit::visit_file(self, file);
    }

    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs = match item {
            syn::Item::Mod(item) => &item.attrs,
            syn::Item::Fn(item) => &item.attrs,
            syn::Item::Impl(item) => &item.attrs,
            syn::Item::Type(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs,
            syn::Item::Const(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs,
            _ => {
                syn::visit::visit_item(self, item);
                return;
            }
        };
        if !is_cfg_test(attrs) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
        if !is_cfg_test(&method.attrs) {
            syn::visit::visit_impl_item_fn(self, method);
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let self_ty = match &*item.self_ty {
            syn::Type::Path(ty) => ty.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        };
        if let Some((_, trait_path, _)) = &item.trait_
            && trait_path
                .segments
                .last()
                .is_some_and(|s| s.ident == "From")
            && let Some(name) = self_ty.as_deref().and_then(|ty| self.resolve(ty))
        {
            self.record(format!("`impl From<…> for {name}`"));
        }
        let previous = self.impl_types.clone();
        self.impl_types = self_ty.into_iter().collect();
        syn::visit::visit_item_impl(self, item);
        self.impl_types = previous;
    }

    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        if let Some(name) = self.constructor_owner(expr.qself.as_ref(), &expr.path) {
            self.record(format!("`{}` constructs {name}", path_string(&expr.path)));
        }
        syn::visit::visit_expr_path(self, expr);
    }

    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        let owner = match &expr.qself {
            Some(qself) => self.type_name(&qself.ty),
            None => expr
                .path
                .segments
                .last()
                .and_then(|s| self.segment_name(&s.ident.to_string())),
        };
        if let Some(name) = owner {
            self.record(format!(
                "`{} {{ … }}` constructs {name}",
                path_string(&expr.path)
            ));
        }
        syn::visit::visit_expr_struct(self, expr);
    }
}

/// Every construction of an inventory name the parsed `source` contains,
/// test items excluded; `seed` are the aliases declared elsewhere (the
/// crate-wide map in production, a planted re-export in fixtures).
fn ast_constructions_with(
    source: &str,
    names: &BTreeSet<String>,
    seed: &BTreeMap<String, String>,
) -> Vec<String> {
    use syn::visit::Visit;
    let file = syn::parse_file(source).expect("production source parses");
    let mut scan = AstConstructions {
        names,
        aliases: seed.clone(),
        impl_types: Vec::new(),
        found: Vec::new(),
    };
    scan.visit_file(&file);
    scan.found
}

/// [`ast_constructions_with`] seeded with every alias of the crate.
fn ast_constructions(source: &str, names: &BTreeSet<String>) -> Vec<String> {
    ast_constructions_with(source, names, crate_raw_aliases())
}

/// (c) The parse-based scan: no production file outside `src/composition`
/// and `src/application` constructs a use case, however the construction
/// is spelled or laid out.
#[test]
fn only_composition_and_the_application_construct_use_cases_on_the_syntax_tree() {
    let names = use_case_names();
    let mut outside = Vec::new();
    let mut in_composition = 0usize;
    for path in production_files() {
        let source = std::fs::read_to_string(&path).unwrap();
        let found = ast_constructions(&source, &names);
        if path.starts_with("src/composition/") {
            in_composition += found.len();
        } else if !path.starts_with("src/application/") {
            outside.extend(found.into_iter().map(|what| format!("{path}: {what}")));
        }
    }
    assert!(
        outside.is_empty(),
        "use cases are constructed in composition only (parse-based scan):\n{}",
        outside.join("\n")
    );
    assert!(
        in_composition >= 10,
        "composition builds the graph ({in_composition} constructions seen on the tree)"
    );
}

/// The parse-based scan itself: every evasion shape the line scan
/// documents is caught, and handle positions are spared.
#[test]
fn ast_construction_scan_catches_multiline_self_qualified_and_aliased_constructions() {
    let names: BTreeSet<String> = ["ListSessions", "KillEnvironment"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let caught = [
        "fn f() { let x = ListSessions\n    ::new(\n        store,\n    ); }",
        "fn f() { let x = ListSessions :: new(store); }",
        "impl ListSessions { fn alt() -> Self { Self::new(store) } }",
        "impl ListSessions { fn alt() -> Self { Self { store } } }",
        "fn f() { let x = <ListSessions>::new(store); }",
        "fn f() { let x = <ListSessions as Build>::from(parts); }",
        "use crate::application::sessions::use_cases::ListSessions as Q;\nfn f() { let x = Q::new(store); }",
        "use crate::application::sessions::use_cases::{ListSessions as A, ReadHistory};\ntype B = A;\nfn f() { let x = B::default(); }",
        "type Alias = crate::application::sessions::use_cases::ListSessions;\nfn f() { let x = Arc::new(Alias { store }); }",
        "fn f() { let x = handle.unwrap_or_else(ListSessions::default); }",
        "fn f() -> ListSessions {\n    ListSessions {\n        store,\n    }\n}",
        "struct Parts;\nimpl From<Parts> for ListSessions { fn from(p: Parts) -> Self { todo() } }",
        "fn f() { let k = Arc::new(\n    KillEnvironment::new(\n        a,\n    ),\n); }",
    ];
    for source in caught {
        assert!(
            !ast_constructions(source, &names).is_empty(),
            "must catch:\n{source}"
        );
    }
    let spared = [
        "struct H { list: Arc<ListSessions> }",
        "fn f(uc: &ListSessions) -> usize { uc.list_all().len() }",
        "fn f(e: Event) { match e { Event::ListSessions { id } => id, ListSessions { .. } => 0 } }",
        "impl Debug for ListSessions { fn fmt(&self) {} }",
        "impl From<ListSessions> for Other { fn from(x: ListSessions) -> Self { Other } }",
        "use crate::application::sessions::use_cases::ListSessions;\nfn f() { let x = ListSessionsController::new(uc); }",
        "#[cfg(test)]\nfn rig() { let x = ListSessions::new(store); }",
        "#[cfg(test)]\nmod tests { fn rig() { let x = ListSessions::new(store); } }",
        "impl Thing { #[cfg(test)] fn rig() { ListSessions::new(store); } }",
        "fn f() { let x = Other::new(ListSessions::name()); }",
    ];
    for source in spared {
        let found = ast_constructions(source, &names);
        assert!(found.is_empty(), "must spare:\n{source}\nfound {found:?}");
    }
}

/// F1 (#1998 review): an alias declared in one file and used in another —
/// `pub(crate) use …::ResumeSavedSession as Hidden;` in `uds_models.rs`,
/// `super::uds_models::Hidden::new(…)` in `uds_reload.rs` — is caught by
/// both scans once the alias map is crate-wide.
#[test]
fn cross_file_alias_re_exports_are_resolved_by_both_scans() {
    let names: BTreeSet<String> = ["ResumeSavedSession".to_string()].into_iter().collect();
    let declaring =
        "pub(crate) use crate::application::sessions::use_cases::ResumeSavedSession as Hidden;";
    let using = "fn build() { let r = super::uds_models::Hidden::new(a, a, a, a, false); }";
    let code = |src: &str| -> Vec<(usize, String)> {
        src.lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.to_string()))
            .collect()
    };
    // The line scan: the raw alias comes from the declaring file only.
    let raw = raw_aliases_in(&code(declaring));
    assert_eq!(
        raw.get("Hidden").map(String::as_str),
        Some("ResumeSavedSession")
    );
    let resolved = resolve_aliases(&raw, &names);
    // The line scan sees the bare alias; the qualified `super::…::Hidden`
    // spelling is the tree scan's (a `::`-qualified name is a path there).
    assert!(
        resolved
            .keys()
            .any(|alias| constructs("let r = Hidden::new(a, a, a, a, false);", alias)),
        "the line scan resolves the cross-file alias"
    );
    // The tree scan: seeded with the other file's alias.
    assert!(
        !ast_constructions_with(using, &names, &raw).is_empty(),
        "the tree scan resolves the cross-file alias"
    );
    // Without the other file's alias neither would see it — the reason the
    // map is crate-wide.
    assert!(ast_constructions_with(using, &names, &BTreeMap::new()).is_empty());
    // `pub(super) use` and `pub use` spellings are aliases too, and a rename
    // inside a multiline group is seen on the syntax tree.
    for decl in [
        "pub(super) use crate::application::sessions::use_cases::ResumeSavedSession as H2;",
        "pub use crate::application::sessions::use_cases::{ResumeSavedSession as H3};",
    ] {
        assert!(!raw_aliases_in(&code(decl)).is_empty(), "{decl}");
    }
    let grouped = "pub(crate) use crate::application::sessions::use_cases::{\n    ListSessions,\n    ResumeSavedSession as H4,\n};";
    assert_eq!(
        syn_aliases(grouped).get("H4").map(String::as_str),
        Some("ResumeSavedSession")
    );
}
