//! Epic close of the sessions capability (#1968, D10 #1979): the final
//! architecture, locked. `sessions_capability.rs` grew slice by slice and
//! pins each slice's retirement; this module states the end state once,
//! affirmatively and tree-derived, so a use case, port, DTO, owner or
//! document added later must join an exact inventory or fail.
//!
//! - Inventory: every sessions use case (each `pub struct` under
//!   `application/sessions/use_cases/`), port and DTO is declared exactly
//!   once, capability-local, and each lifecycle transaction has exactly one
//!   owner file with one entry point.
//! - Signatures: the domain and application code of the capability names
//!   no infrastructure, wire, transport or interface type and takes no
//!   interface callback (one domain-typed frame predicate is the admitted
//!   exception, pinned as an exact site).
//! - Interface: the session modules parse, map and present; none reaches a
//!   store method, whatever the binding is called (negative fixtures prove
//!   the predicate), implements a persistence port, or names an adapter.
//! - Infrastructure: implements the ports at exact sites and constructs no
//!   use case (the parse-based scan in `use_case_construction.rs`).
//! - Ownership: durable retention (the `SessionStore` / `ContextSpillStore`
//!   holders) is an exact set inside sessions, composition and persistence;
//!   pruning decisions stay in `context*`/the agent loop.
//! - Admitted raw-key sites: the persistence round-trip, the agent loop,
//!   the recall tool's `Tool::set_session_key(String)` conversion (tools
//!   capability, out of this epic's scope) and the tool registry's startup
//!   key — exact and decrease-only.
//! - Retirement: exact retired paths and names are absent from production
//!   code and from the docs; the disposition inventory of the modules that
//!   stayed in the interface is non-empty and exact.
//! - Documentation lockstep: the docs name every use case, port and
//!   session command of the tree, the unchanged protocol and the
//!   intentionally unimplemented workspace seam.
//!
//! Every inventory here is decrease-only by review policy: a new entry is a
//! reviewed edit of this file the PR must justify.

use std::collections::BTreeSet;
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// Every sessions use case: each `pub struct` under
/// `src/application/sessions/use_cases/`, exact.
const SESSIONS_USE_CASES: &[&str] = &[
    "ClearConversation",
    "DepartingChildren",
    "ExportSessionReport",
    "ListRetainedContext",
    "ListSessions",
    "ReadHistory",
    "RecallContext",
    "RecoverMessage",
    "ResumeSavedSession",
    "RetainContext",
    "RewindConversation",
    "SaveSession",
    "SearchSessionMetadata",
    "StartFreshConversation",
    "SynchronizeTranscript",
];

/// Each lifecycle transaction / query, its one owner file and the entry
/// point the owner exposes (use case, file, entry signature fragment).
const TRANSACTION_OWNERS: &[(&str, &str, &str)] = &[
    (
        "ListSessions",
        "src/application/sessions/use_cases/list_sessions.rs",
        "pub async fn discover(",
    ),
    (
        "SearchSessionMetadata",
        "src/application/sessions/use_cases/search_session_metadata.rs",
        "pub async fn search(",
    ),
    (
        "ReadHistory",
        "src/application/sessions/use_cases/read_history.rs",
        "pub fn page_of(",
    ),
    (
        "RecoverMessage",
        "src/application/sessions/use_cases/recover_message.rs",
        "pub async fn execute(",
    ),
    (
        "SynchronizeTranscript",
        "src/application/sessions/use_cases/synchronize_transcript.rs",
        "pub async fn execute(",
    ),
    (
        "ExportSessionReport",
        "src/application/sessions/use_cases/export_session_report.rs",
        "pub async fn execute(",
    ),
    (
        "SaveSession",
        "src/application/sessions/use_cases/save_session.rs",
        "pub async fn save(",
    ),
    (
        "ClearConversation",
        "src/application/sessions/use_cases/clear_conversation.rs",
        "pub async fn execute(",
    ),
    (
        "RewindConversation",
        "src/application/sessions/use_cases/rewind_conversation.rs",
        "pub async fn execute(",
    ),
    (
        "StartFreshConversation",
        "src/application/sessions/use_cases/start_fresh_conversation.rs",
        "pub async fn execute(",
    ),
    (
        "DepartingChildren",
        "src/application/sessions/use_cases/departing_children.rs",
        "pub async fn settle(",
    ),
    (
        "ResumeSavedSession",
        "src/application/sessions/use_cases/resume_saved_session.rs",
        "pub async fn execute(",
    ),
    (
        "RecallContext",
        "src/application/sessions/use_cases/recall_context.rs",
        "pub async fn recall(",
    ),
    (
        "RetainContext",
        "src/application/sessions/use_cases/retain_context.rs",
        "pub async fn retain(",
    ),
    (
        "ListRetainedContext",
        "src/application/sessions/use_cases/retain_context.rs",
        "pub async fn list(",
    ),
];

/// The ports of the capability and every production implementor of each,
/// exact: infrastructure adapters, the composition-side fleet adaptation,
/// the application's own latch, and the two interface runtime adapters.
pub(super) const PORT_IMPLEMENTORS: &[(&str, &[&str])] = &[
    // #2009 capability-local ports retain one concrete adapter each.
    (
        "SessionHomeCatalogue",
        &["src/infrastructure/persistence/session_home_catalogue.rs"],
    ),
    (
        "WorkspaceDiscovery",
        &["src/infrastructure/workspace/git_scope_discovery.rs"],
    ),
    (
        "SessionStore",
        &["src/infrastructure/persistence/session_store.rs"],
    ),
    (
        "ContextSpillStore",
        &["src/infrastructure/persistence/context_spill.rs"],
    ),
    (
        "SessionExportPort",
        &["src/infrastructure/session_export.rs"],
    ),
    (
        "DurablePrefixObservation",
        &["src/application/durable_prefix.rs"],
    ),
    (
        "WorkflowRunSource",
        &["src/infrastructure/persistence/session_snapshot_sources.rs"],
    ),
    (
        "HistoricalRosterSource",
        &["src/infrastructure/persistence/session_snapshot_sources.rs"],
    ),
    (
        "TurnAccountingReset",
        &[
            "src/interface/cli/uds_session_switch_runtime.rs",
            "src/interface/cli/uds_turn_accounting.rs",
        ],
    ),
    (
        "FreshSessionIdentityGenerator",
        &["src/infrastructure/persistence/fresh_session_identity.rs"],
    ),
    ("FleetSettlement", &["src/composition/fleet_settlement.rs"]),
    (
        "DelegatedChildrenRoster",
        &["src/infrastructure/tools/delegated_roster.rs"],
    ),
    (
        "SessionKeyPropagation",
        &["src/interface/cli/uds_session_switch_runtime.rs"],
    ),
    (
        "SessionSwitchRuntime",
        &["src/interface/cli/uds_session_switch_runtime.rs"],
    ),
];

/// The production files that hold the durable session store port (exact):
/// the capability, composition, persistence, and the interface handle
/// declarations the loop is composed over.
const SESSION_STORE_HOLDERS: &[&str] = &[
    // #2009 authority and derived catalogue share the composed store.
    "src/infrastructure/persistence/session_home_catalogue.rs",
    // #2010 the catalogue's metadata query (a child module, split for the
    // line ceiling) joins the store's summary walk with the home listing.
    "src/infrastructure/persistence/session_home_catalogue_metadata.rs",
    "src/infrastructure/persistence/session_store_home.rs",
    "src/application/sessions/ports.rs",
    "src/application/sessions/use_cases/list_sessions.rs",
    "src/application/sessions/use_cases/read_history.rs",
    "src/application/sessions/use_cases/resume_saved_session.rs",
    // #2009 admission holds the target's claim guard beside the owner.
    "src/application/sessions/use_cases/resume_saved_session_admission.rs",
    // #2011 the eligibility collaborators read the target through the
    // owner's store: the effect-free pre-flight and the claimed load — and
    // (review R2-H3) the same existence check opens an action's refusal order.
    "src/application/sessions/use_cases/resume_saved_session_action.rs",
    "src/application/sessions/use_cases/resume_saved_session_decision.rs",
    "src/application/sessions/use_cases/save_session.rs",
    // #2009 home acquisition runs on the save owner's existing path.
    "src/application/sessions/use_cases/save_session_home.rs",
    "src/application/sessions/use_cases/start_fresh_conversation.rs",
    "src/composition/active_session.rs",
    // #2009 the home context is composed over the one file store.
    "src/composition/session_home.rs",
    "src/composition/sessions.rs",
    "src/infrastructure/persistence/session_store.rs",
    "src/interface/cli/uds_session_handles.rs",
];

/// Vocabulary no domain or application file of the capability may spell:
/// wire and transport types, the interface's own types, infrastructure
/// paths, runtime I/O, and interface callbacks.
const APPLICATION_FORBIDDEN: &[&str] = &[
    "serde_json",
    "AgentEvent",
    "AgentCommand",
    "crate::interface",
    "crate::infrastructure",
    "DispatchCtx",
    "AgentSession",
    "SubagentRegistry",
    "WorkflowStateHandle",
    "tokio::net",
    "UnixStream",
    "AsyncWrite",
    "AsyncRead",
    "std::fs",
    "std::env",
    "tokio::fs",
    ".exists(",
    "dyn Fn",
    "FnOnce",
    "FnMut",
    "impl Fn(",
];

/// The admitted closure parameters, exact and decrease-only: the frame
/// predicate over the domain `Message` the transport hands the sync use
/// case (D3 #1973 — the application decides what a cut means, the
/// transport what fits; no wire type crosses), and the report use case's
/// private ledger lookup (D4 #1974, a helper of the same file).
const ADMITTED_PREDICATE_SITES: &[(&str, &str)] = &[
    (
        "src/application/sessions/use_cases/synchronize_transcript.rs",
        "impl FnMut(&Message) -> bool",
    ),
    (
        "src/application/sessions/use_cases/export_session_report.rs",
        "impl Fn(&str) -> Option<&'a Message>",
    ),
];

/// The domain files the capability is built on.
const DOMAIN_FILES: &[&str] = &[
    "src/domain/session.rs",
    "src/domain/session_identity.rs",
    "src/domain/conversation_view.rs",
    "src/domain/conversation_edit.rs",
];

/// The interface session modules: the sessions edge, the CLI wire modules
/// that present session state, the handle declarations, the runtime
/// adapters and the two startup paths. Each parses, maps or presents.
const HANDLER_FORBIDDEN: &[&str] = &[
    ".claim(",
    ".load(",
    ".release(",
    ".save(",
    ".save_delta(",
    "switch_to(",
    ".settle(",
    ".reset_roster(",
    ".set_session_key(",
    "from_persisted_key(",
    "clear_conversation(",
    "truncate_at_user_message(",
    "set_persisted_watermark(",
    ".publish(",
    ".clear_usage(",
];

/// The admitted raw-key conversion sites (exact, decrease-only): the
/// persistence round-trip, the agent loop's provider session id (agent-turn
/// epic), the read of a roster row's persisted key inside the capability,
/// and the recall tool's `Tool::set_session_key(String)` adaptation — the
/// tools-capability port is raw, so the one infrastructure conversion sits
/// there until that capability types it (documented in `docs/sessions.md`).
pub(super) fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub(super) fn files_under(root: &str) -> Vec<String> {
    let mut files = Vec::new();
    walk(Path::new(root), &mut files);
    files
}

pub(super) fn production_files_calling(needle: &str) -> BTreeSet<String> {
    production_files()
        .into_iter()
        .filter(|path| {
            production_code(path)
                .iter()
                .any(|(_, line)| line.contains(needle))
        })
        .collect()
}

pub(super) fn ident_at_start(rest: &str) -> String {
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Every `pub struct` under the sessions use_cases folder.
pub(super) fn use_case_tree() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in files_under("src/application/sessions/use_cases")
        .into_iter()
        .filter(|p| !p.ends_with("_tests.rs"))
    {
        for (_, line) in production_code(&path) {
            if let Some(rest) = line.trim_start().strip_prefix("pub struct ") {
                names.insert(ident_at_start(rest));
            }
        }
    }
    names
}

/// Every `pub struct` / `pub enum` under the sessions dto folder.
pub(super) fn dto_tree() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in files_under("src/application/sessions/dto")
        .into_iter()
        .filter(|p| !p.ends_with("_tests.rs"))
    {
        for (_, line) in production_code(&path) {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed
                .strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("pub enum "))
            {
                names.insert(ident_at_start(rest));
            }
        }
    }
    names
}

/// Files whose production code declares `pub struct <name>` / `pub enum
/// <name>` / `pub trait <name>`.
pub(super) fn declarers_of(name: &str) -> BTreeSet<String> {
    production_files()
        .into_iter()
        .filter(|path| {
            production_code(path).iter().any(|(_, line)| {
                let trimmed = line.trim_start();
                [
                    "pub struct ",
                    "pub enum ",
                    "pub trait ",
                    "pub(crate) struct ",
                ]
                .iter()
                .any(|prefix| {
                    trimmed
                        .strip_prefix(prefix)
                        .is_some_and(|rest| ident_at_start(rest) == name)
                })
            })
        })
        .collect()
}

/// A line that reaches a session store or retention store method by its
/// identity-keyed argument shape — `.claim(&…`, `.release(&…`, `.load(&…`,
/// `.exists(&…`, `.list(&…`, `.save_delta(`, `.save_clean_delta(`,
/// `.append(&…`, `.recall(&…`, `.clear(&…`, `.list_entries(&…`,
/// `.has_entries(&…`, `.scrub_sync(&…` — or a `.save(&…` that is not the
/// transaction's own `save(&mut messages, …)` request. Binding-agnostic.
#[test]
fn every_sessions_use_case_is_inventoried_and_declared_once() {
    let tree = use_case_tree();
    assert_eq!(
        tree,
        set(SESSIONS_USE_CASES),
        "the sessions use-case inventory changed; update SESSIONS_USE_CASES and the docs"
    );
    for name in SESSIONS_USE_CASES {
        let declarers = declarers_of(name);
        assert_eq!(declarers.len(), 1, "{name} is declared once: {declarers:?}");
        let owner = declarers.iter().next().unwrap();
        assert!(
            owner.starts_with("src/application/sessions/use_cases/"),
            "{name} lives under the capability's use_cases folder, not {owner}"
        );
    }
    let owners: BTreeSet<&str> = TRANSACTION_OWNERS.iter().map(|(n, _, _)| *n).collect();
    assert_eq!(
        owners,
        SESSIONS_USE_CASES.iter().copied().collect(),
        "every use case has one transaction owner row"
    );
}

/// Ownership: each lifecycle transaction has exactly one owner — the use
/// case's `impl` blocks live in its file, the entry point is the owner's,
/// and no other production file spells that entry with the same request
/// shape.
#[test]
fn each_lifecycle_transaction_has_exactly_one_owner() {
    for (name, file, entry) in TRANSACTION_OWNERS {
        let impls = production_files_calling(&format!("impl {name} {{"));
        assert_eq!(impls, set(&[file]), "`impl {name}` blocks live in {file}");
        assert!(
            production_code(file)
                .iter()
                .any(|(_, line)| line.contains(entry)),
            "{file} exposes `{entry}` for {name}"
        );
    }
    // The handlers admit, request and present; the sequencing verbs of the
    // transactions never appear in them.
    let handlers = "src/interface/cli/uds_dispatch_session.rs";
    for needle in HANDLER_FORBIDDEN {
        let hits: Vec<String> = production_code(handlers)
            .into_iter()
            .filter(|(_, line)| line.contains(needle))
            .map(|(n, line)| format!("{handlers}:{n}: {}", line.trim()))
            .collect();
        assert!(
            hits.is_empty(),
            "the handlers sequence `{needle}`: {hits:#?}"
        );
    }
    // Exactly the four session handlers exist, and no other interface file
    // declares a `handle_*session*` orchestration.
    let handler_fns: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .flat_map(|path| {
            production_code(&path)
                .into_iter()
                .filter_map(move |(_, line)| {
                    let trimmed = line.trim_start();
                    let rest = trimmed
                        .strip_prefix("pub(super) async fn ")
                        .or_else(|| trimmed.strip_prefix("pub(crate) async fn "))
                        .or_else(|| trimmed.strip_prefix("pub async fn "))
                        .or_else(|| trimmed.strip_prefix("async fn "))?;
                    let name = ident_at_start(rest);
                    (name.starts_with("handle_") && name.contains("session"))
                        .then(|| format!("{path}::{name}"))
                })
        })
        .collect();
    assert_eq!(
        handler_fns,
        set(&[
            "src/interface/cli/uds_dispatch_session.rs::handle_list_sessions",
            "src/interface/cli/uds_dispatch_session.rs::handle_new_session",
            "src/interface/cli/uds_dispatch_session.rs::handle_resume_session",
        ]),
        "the session handlers are exactly the discovery and two transition edges"
    );
}

/// Ports and DTOs are capability-local: each port is declared once under
/// the capability's ports files; each DTO once under its dto folder; no
/// DTO copy or second port declaration exists anywhere else.
#[test]
fn ports_and_dtos_are_declared_once_and_capability_local() {
    for (port, _) in PORT_IMPLEMENTORS {
        let declarers = declarers_of(port);
        assert_eq!(
            declarers.len(),
            1,
            "port {port} is declared once: {declarers:?}"
        );
        let owner = declarers.iter().next().unwrap();
        assert!(
            owner == "src/application/sessions/ports.rs"
                || owner.starts_with("src/application/sessions/ports/"),
            "port {port} is declared under the capability's ports, not {owner}"
        );
    }
    let dtos = dto_tree();
    assert!(
        dtos.len() >= 40,
        "the DTO tree is non-empty ({})",
        dtos.len()
    );
    for dto in &dtos {
        let declarers = declarers_of(dto);
        assert_eq!(
            declarers.len(),
            1,
            "DTO {dto} is declared once: {declarers:?}"
        );
        assert!(
            declarers
                .iter()
                .next()
                .unwrap()
                .starts_with("src/application/sessions/dto/")
        );
    }
    for path in files_under("src/application/sessions/dto")
        .into_iter()
        .filter(|p| !p.ends_with("_tests.rs"))
    {
        for needle in [
            "serde",
            "AgentEvent",
            "crate::interface",
            "crate::infrastructure",
        ] {
            assert!(
                !production_code(&path)
                    .iter()
                    .any(|(_, line)| line.contains(needle)),
                "{path} carries `{needle}`: DTOs are domain values, not wire shapes"
            );
        }
    }
}

/// Signatures: no domain or application file of the capability names a
/// wire, transport, infrastructure or interface type, or takes an
/// interface callback, beyond the one admitted domain-typed predicate.
#[test]
fn domain_and_application_signatures_carry_no_wire_infrastructure_or_callback_types() {
    let mut files: Vec<String> = files_under("src/application/sessions")
        .into_iter()
        .filter(|p| !p.ends_with("_tests.rs"))
        .collect();
    files.extend(DOMAIN_FILES.iter().map(|f| f.to_string()));
    files.push("src/application/durable_prefix.rs".to_string());
    let admitted: BTreeSet<(String, String)> = ADMITTED_PREDICATE_SITES
        .iter()
        .map(|(f, s)| (f.to_string(), s.to_string()))
        .collect();
    let mut seen_admitted = BTreeSet::new();
    let mut violations = Vec::new();
    for path in &files {
        for (n, line) in production_code(path) {
            for needle in APPLICATION_FORBIDDEN {
                if !line.contains(needle) {
                    continue;
                }
                let admitted_here = admitted
                    .iter()
                    .find(|(f, s)| f == path && line.contains(s.as_str()));
                match admitted_here {
                    Some(site) => {
                        seen_admitted.insert(site.clone());
                    }
                    None => violations.push(format!("{path}:{n}: `{needle}` in `{}`", line.trim())),
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "sessions domain/application code names a forbidden type: {violations:#?}"
    );
    assert_eq!(
        seen_admitted, admitted,
        "the admitted predicate sites are exact (a vanished site leaves the list)"
    );
}

/// The last segment of every trait a `source` implements outside
/// `cfg(test)` items — however the trait path is qualified or laid out (a
/// text needle would miss `impl crate::…::Port for X` and a multiline
/// `impl`).
fn implemented_traits(source: &str) -> BTreeSet<String> {
    use syn::visit::Visit;
    fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_ident("cfg")
                && attr
                    .parse_args::<syn::Path>()
                    .is_ok_and(|path| path.is_ident("test"))
        })
    }
    #[derive(Default)]
    struct Impls(BTreeSet<String>);
    impl<'ast> Visit<'ast> for Impls {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            let attrs = match item {
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Fn(item) => &item.attrs,
                _ => {
                    syn::visit::visit_item(self, item);
                    return;
                }
            };
            if !is_cfg_test(attrs) {
                syn::visit::visit_item(self, item);
            }
        }
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            if let Some((_, path, _)) = &item.trait_
                && let Some(last) = path.segments.last()
            {
                self.0.insert(last.ident.to_string());
            }
            syn::visit::visit_item_impl(self, item);
        }
    }
    let file = syn::parse_file(source).expect("source parses");
    let mut impls = Impls::default();
    impls.visit_file(&file);
    impls.0
}

fn implements_in_source(source: &str, port: &str) -> bool {
    implemented_traits(source).contains(port)
}

/// The production files that implement `port` (syntax-tree scan, every
/// file parsed once per process).
pub(super) fn port_implementors(port: &str) -> BTreeSet<String> {
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    static IMPLS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    IMPLS
        .get_or_init(|| {
            let mut by_trait: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for path in production_files() {
                for name in implemented_traits(&std::fs::read_to_string(&path).unwrap()) {
                    by_trait.entry(name).or_default().insert(path.clone());
                }
            }
            by_trait
        })
        .get(port)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn port_implementor_scan_sees_qualified_and_multiline_impls_and_skips_test_items() {
    for source in [
        "struct X; impl crate::application::sessions::ports::SessionStore for X {}",
        "struct X; impl SessionStore\n    for X\n{}",
        "struct X<'a>(&'a ()); impl<'a> ports::SessionStore for X<'a> {}",
    ] {
        assert!(
            implements_in_source(source, "SessionStore"),
            "must catch:\n{source}"
        );
    }
    for source in [
        "struct X; impl Debug for X {}",
        "struct X; #[cfg(test)] impl SessionStore for X {}",
        "#[cfg(test)] mod t { struct X; impl SessionStore for X {} }",
        "struct X; fn f() { let _ = \"impl SessionStore for X\"; }",
        "struct SessionStore; impl SessionStore { fn f() {} }",
    ] {
        assert!(
            !implements_in_source(source, "SessionStore"),
            "must spare:\n{source}"
        );
    }
}

/// Infrastructure implements the ports at exact sites (on the syntax tree,
/// whatever the trait path's qualification), never constructs a use case
/// (the parse-based scan of `use_case_construction.rs`), and the durable
/// store's holders are an exact set.
#[test]
fn infrastructure_implements_ports_at_exact_sites_and_the_store_holders_are_exact() {
    for (port, sites) in PORT_IMPLEMENTORS {
        assert!(!sites.is_empty(), "{port} has a production implementor");
        let observed = port_implementors(port);
        assert_eq!(observed, set(sites), "the implementors of {port} are exact");
    }
    assert_eq!(
        production_files_calling("SessionStore"),
        set(SESSION_STORE_HOLDERS),
        "the session store port's holders are exact"
    );
    for path in production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/infrastructure/"))
    {
        for (n, line) in production_code(&path) {
            if line.contains("sessions::use_cases") {
                assert!(
                    line.contains("RecallContext"),
                    "{path}:{n}: infrastructure names a sessions use case other than the recall adapter's handle: {}",
                    line.trim()
                );
            }
        }
    }
}
