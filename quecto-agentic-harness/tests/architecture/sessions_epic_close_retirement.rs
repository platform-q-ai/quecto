//! Epic close of the sessions capability (#1968, D10 #1979), part two:
//! the interface's parse/map/present rule (a binding-agnostic persistence
//! reach predicate with negative fixtures), the admitted raw-key sites, the
//! retirement and disposition inventories, and the documentation lockstep.
//! The inventories and helpers live in `sessions_epic_close.rs`; every list
//! here is decrease-only by review policy.

use std::collections::BTreeSet;
use std::path::Path;

use super::sessions_epic_close::{
    PORT_IMPLEMENTORS, files_under, production_files_calling, set, use_case_tree,
};
use super::teardown_authority::{production_code, production_files};

const INTERFACE_SESSION_MODULES: &[&str] = &[
    "src/interface/uds/sessions/controller.rs",
    "src/interface/uds/sessions/read_history_controller.rs",
    "src/interface/uds/sessions/recover_message_controller.rs",
    "src/interface/uds/sessions/export_report_controller.rs",
    "src/interface/uds/sessions/synchronize_transcript_controller.rs",
    "src/interface/uds/sessions/rewind_conversation_controller.rs",
    "src/interface/cli/uds_dispatch_session.rs",
    "src/interface/cli/uds_session_history.rs",
    "src/interface/cli/uds_session_message_range.rs",
    "src/interface/cli/uds_latest_report.rs",
    "src/interface/cli/uds_sync.rs",
    "src/interface/cli/uds_snapshots.rs",
    "src/interface/cli/uds_session_handles.rs",
    "src/interface/cli/uds_turn_accounting.rs",
    "src/interface/cli/uds_session_switch_runtime.rs",
    "src/interface/cli/retention_handles.rs",
    "src/interface/cli/uds_lifecycle.rs",
    "src/interface/cli/agent/run_session.rs",
];

/// Adapter and layout names no interface production file may spell.
const INTERFACE_FORBIDDEN_ADAPTERS: &[&str] = &[
    "FileSessionStore",
    "FileContextSpillStore",
    "FlatSessionLayout",
    "FileSessionExport",
    "ProcessClockIdentityGenerator",
    "RegistryDelegatedRoster",
    "WorkflowEngineRunSource",
    "RegistryRosterSource",
    "SessionIdentity::from_persisted_key(",
    "SessionIdentity::fresh_chat(",
];

/// Orchestration verbs the session handlers admit, request and present
/// around but never sequence themselves.
const RAW_KEY_CONVERSION_SITES: &[&str] = &[
    // #2009 catalogue maps persisted opaque keys at the adapter boundary.
    "src/infrastructure/persistence/session_home_catalogue.rs",
    "src/application/agent_loop.rs",
    "src/application/sessions/use_cases/read_history.rs",
    "src/infrastructure/persistence/session_store.rs",
    // R2-L3: the per-record read (and its one conversion) moved out of the
    // directory walk into `session_store_list_record.rs`; the walk itself
    // no longer converts (the summary carries its identity).
    "src/infrastructure/persistence/session_store_list_record.rs",
    "src/infrastructure/tools/recall.rs",
];

/// The tool registry's startup key (exact): the tools capability's raw
/// `session_key` input predates the identity and is admitted here with its
/// reason — `build_key("cli", name)` is the same bytes `SessionIdentity::
/// named_cli` produces for a name the flag parser already admitted, and the
/// registry runs on the one-shot path too, where no identity is resolved.
const STARTUP_KEY_SITES: &[&str] = &["src/interface/cli/agent/agent_tool_registry.rs"];

/// Disposition inventory (D10): the interface modules that stayed, each in
/// its allowlisted owner role, with no sessions persistence call.
const DISPOSITION: &[(&str, &str)] = &[
    ("src/interface/cli/uds_cancel_history.rs", "cancellation"),
    ("src/interface/cli/uds_session_controls.rs", "control"),
    ("src/interface/cli/uds_session_notify.rs", "notification"),
    ("src/interface/cli/uds_session_suspension.rs", "suspension"),
    ("src/interface/cli/uds_session_usage.rs", "stats"),
];

/// Files retired by the epic; none may reappear.
const RETIRED_PATHS: &[&str] = &[
    "src/interface/cli/agent_session_identity.rs",
    "src/interface/cli/uds/uds_session_load.rs",
    "src/interface/cli/uds_busy_sync.rs",
    "src/interface/cli/uds_snapshot_export.rs",
];

/// Symbols retired by the epic; none may survive in production code or be
/// presented by the docs as current.
const RETIRED_NAMES: &[&str] = &[
    "ConversationSnapshotData",
    "resolve_uds_session_key",
    "generate_chat_key",
    "generate_chat_identity",
    "persist_current_session",
    "snapshot_subagent_roster",
    "reset_to_with_spill_store",
    "ExportRootSlot",
    "scrub_ephemeral_spill",
    "scrub_session_spill_sync",
    "note_persisted_roster_is_history",
    "GetMessageLookup",
    "ownership_stamp_path",
    "key_to_filename",
    // D10: the tracker's raw session-key copy and the raw startup input.
    "fn set_session_key(&mut self, session_key: String)",
    "pub fn new(model: String, session_key: String)",
];

/// The session commands the protocol reference documents, each under its
/// own `### `command`` heading.
const SESSION_COMMANDS: &[&str] = &[
    "list_sessions",
    "search_session_metadata",
    "resume_session",
    "new_session",
    "clear_history",
    "rewind_to",
    "persist_session",
    "get_messages",
    "get_message",
    "sync",
    "get_report",
    "get_session_stats",
    "get_state",
];

fn reaches_a_persistence_method(line: &str) -> bool {
    let keyed = [
        ".claim(&",
        ".release(&",
        ".load(&",
        ".exists(&",
        ".list(&",
        ".append(&",
        ".recall(&",
        ".clear(&",
        ".list_entries(&",
        ".has_entries(&",
        ".scrub_sync(&",
        ".save(&",
    ];
    keyed.iter().any(|needle| {
        line.match_indices(needle)
            .any(|(index, _)| !line[index + needle.len()..].starts_with("mut "))
    }) || line.contains(".save_delta(")
        || line.contains(".save_clean_delta(")
}

/// The persistence methods of the two store ports.
const PERSISTENCE_METHODS: &[&str] = &[
    "claim",
    "release",
    "load",
    "save",
    "save_delta",
    "save_clean_delta",
    "exists",
    "list",
    "append",
    "recall",
    "list_entries",
    "has_entries",
    "clear",
    "scrub_sync",
];

/// The field names a store handle travels under.
const STORE_FIELDS: &[&str] = &["store", "spill_store", "session_store"];

/// Every persistence-method call of `source` whose receiver is a store —
/// a field chain through `store` / `spill_store` / `session_store`, or a
/// local bound from one (`let owner = handles.store.clone();`) — found on
/// the syntax tree, so neither a `let target = &id;` argument nor a line
/// break before the `&` hides it (F2 of the #1998 review). `cfg(test)`
/// items are skipped; locals are collected file-wide, fail-closed.
fn store_reaches_in_source(source: &str) -> Vec<String> {
    use syn::visit::Visit;
    fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_ident("cfg")
                && attr
                    .parse_args::<syn::Path>()
                    .is_ok_and(|path| path.is_ident("test"))
        })
    }
    /// Does `expr` reach a store: a store field, a marked local, or an
    /// expression built on one (method chain, call argument, reference,
    /// await, `?`, parens, block tail)?
    fn touches_store(expr: &syn::Expr, marked: &BTreeSet<String>) -> bool {
        match expr {
            syn::Expr::Field(field) => {
                matches!(&field.member, syn::Member::Named(name) if STORE_FIELDS.contains(&name.to_string().as_str()))
                    || touches_store(&field.base, marked)
            }
            syn::Expr::Path(path) => path
                .path
                .get_ident()
                .is_some_and(|ident| marked.contains(&ident.to_string())),
            syn::Expr::MethodCall(call) => {
                touches_store(&call.receiver, marked)
                    || call.args.iter().any(|arg| touches_store(arg, marked))
            }
            syn::Expr::Call(call) => call.args.iter().any(|arg| touches_store(arg, marked)),
            syn::Expr::Reference(reference) => touches_store(&reference.expr, marked),
            syn::Expr::Await(awaited) => touches_store(&awaited.base, marked),
            syn::Expr::Try(tried) => touches_store(&tried.expr, marked),
            syn::Expr::Paren(paren) => touches_store(&paren.expr, marked),
            syn::Expr::Unary(unary) => touches_store(&unary.expr, marked),
            syn::Expr::Cast(cast) => touches_store(&cast.expr, marked),
            syn::Expr::Block(block) => block.block.stmts.last().is_some_and(
                |stmt| matches!(stmt, syn::Stmt::Expr(expr, _) if touches_store(expr, marked)),
            ),
            _ => false,
        }
    }
    fn pat_idents(pat: &syn::Pat, out: &mut Vec<String>) {
        match pat {
            syn::Pat::Ident(ident) => out.push(ident.ident.to_string()),
            syn::Pat::Type(typed) => pat_idents(&typed.pat, out),
            syn::Pat::Reference(reference) => pat_idents(&reference.pat, out),
            syn::Pat::Tuple(tuple) => tuple.elems.iter().for_each(|p| pat_idents(p, out)),
            _ => {}
        }
    }
    struct Scan {
        marked: BTreeSet<String>,
        found: Vec<String>,
        collecting: bool,
    }
    impl<'ast> Visit<'ast> for Scan {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            let attrs = match item {
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Fn(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
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
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if self.collecting
                && let Some(init) = &local.init
                && touches_store(&init.expr, &self.marked)
            {
                let mut names = Vec::new();
                pat_idents(&local.pat, &mut names);
                self.marked.extend(names);
            }
            syn::visit::visit_local(self, local);
        }
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            let method = call.method.to_string();
            if !self.collecting
                && PERSISTENCE_METHODS.contains(&method.as_str())
                && touches_store(&call.receiver, &self.marked)
            {
                self.found
                    .push(format!("`.{method}(…)` on a store receiver"));
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let file = syn::parse_file(source).expect("source parses");
    let mut scan = Scan {
        marked: BTreeSet::new(),
        found: Vec::new(),
        collecting: true,
    };
    // Two collecting passes so a local bound from another marked local
    // resolves whatever the declaration order, then the reach pass.
    scan.visit_file(&file);
    scan.visit_file(&file);
    scan.collecting = false;
    scan.visit_file(&file);
    scan.found
}

#[test]
fn store_reach_scan_sees_renamed_receivers_and_arguments_without_an_ampersand() {
    for rogue in [
        // F2 plant 1: the argument carries no `&` at the call site.
        "async fn f(handles: &SessionHandles, id: SessionIdentity) { let target = &id; handles.store.load(target).await; handles.store.release(target); }",
        // F2 plant 2: the `&` sits on the next line, the receiver is a renamed local.
        "fn f(handles: &SessionHandles, k: SessionIdentity) { let owner = handles.store.clone(); owner.load(\n&k); }",
        "fn f(h: &SessionHandles) { let s = h.store.clone(); let t = s; t.claim(&id()); }",
        "async fn f(r: &RetentionHandles, i: &SessionIdentity) { r.spill_store.recall(i, &SpillId::new(x)).await; }",
        "async fn f(ctx: &Ctx) { Arc::clone(&ctx.sessions.store).save_delta(&session, 3).await; }",
        "fn f(ctx: &Ctx) { ctx.session_store.exists(&id); }",
        "async fn f(h: &SessionHandles) { let n = h.store.list(&SessionListQuery::All).await; }",
    ] {
        let found = store_reaches_in_source(rogue);
        assert!(!found.is_empty(), "must catch:\n{rogue}");
    }
    for benign in [
        "async fn f(ctx: &mut Ctx) { ctx.save_session.save(&mut messages, SaveTrigger::Routine).await; }",
        "fn f(sessions: &SessionHandles) -> Inputs { Inputs { store: sessions.store.clone(), spill_store: retention.as_ref().map(|h| h.store.clone()) } }",
        "fn f(flag: &AtomicBool) { flag.store(true, Ordering::SeqCst); let v = cell.load(Ordering::SeqCst); }",
        "fn f(mut v: Vec<u8>, mut w: Vec<u8>) { v.append(&mut w); v.clear(); }",
        "fn f(store: &CredentialStore) { store.store(Credential::default()); }",
        "fn f(ctx: &Ctx) { let page = ctx.sessions.read_history.page(m, p, 5, None); }",
        "#[cfg(test)] fn rig(h: &SessionHandles) { h.store.load(&id()); }",
        "fn f(history: &History) { let x = history.recall(page_size); }",
    ] {
        let found = store_reaches_in_source(benign);
        assert!(found.is_empty(), "must spare:\n{benign}\nfound {found:?}");
    }
}

#[test]
fn persistence_reach_predicate_catches_rogue_calls_and_spares_requests() {
    for rogue in [
        "ctx.sessions.store.claim(&target).await?;",
        "let loaded = handles.store.load(&identity).await;",
        "store.release(&old_identity);",
        "if store.exists(&id) {",
        "let rows = s.list(&SessionListQuery::All).await?;",
        "self.store.save_delta(&session, 3).await",
        "spill.append(&identity, &entry).await",
        "retention.store.recall(&identity, &SpillId::new(id)).await",
        "store.clear(&identity).await?;",
        "layout_store.scrub_sync(&SessionIdentity::ephemeral());",
        "ctx.session_store.save(&session).await",
    ] {
        assert!(reaches_a_persistence_method(rogue), "must catch: {rogue}");
    }
    for benign in [
        "ctx.save_session.save(&mut messages, SaveTrigger::Routine).await",
        "messages.append(&mut other);",
        "seen.clear();",
        "let value = cell.load(Ordering::SeqCst);",
        "let page = history.recall(page_size);",
        "if path.exists() {",
        "let sessions = ctx.discovery.list(requested).await;",
    ] {
        assert!(
            !reaches_a_persistence_method(benign),
            "must spare: {benign}"
        );
    }
}

/// Inventory: every sessions use case is declared exactly once, under the
/// capability's use_cases folder, and the exact list is the tree's.
/// Interface: the session modules parse, map and present only. No
/// interface production line reaches a persistence method, whatever the
/// binding; none names an adapter, implements a persistence port or
/// converts a raw key.
#[test]
fn interface_session_modules_parse_map_and_present_only() {
    for file in INTERFACE_SESSION_MODULES {
        assert!(
            Path::new(file).is_file(),
            "missing interface session module {file}"
        );
    }
    let interface: Vec<String> = production_files()
        .into_iter()
        .filter(|p| {
            p.starts_with("src/interface/cli/") || p.starts_with("src/interface/uds/sessions/")
        })
        .collect();
    let reaches: Vec<String> = interface
        .iter()
        .flat_map(|path| {
            production_code(path)
                .into_iter()
                .filter(|(_, line)| reaches_a_persistence_method(line))
                .map(move |(n, line)| format!("{path}:{n}: {}", line.trim()))
        })
        .collect();
    assert!(
        reaches.is_empty(),
        "interface reaches a persistence method directly: {reaches:#?}"
    );
    // The syntax-tree scan: the same rule with the receiver resolved —
    // a store field chain or a local bound from one — so no argument
    // spelling or line break hides a reach.
    let tree_reaches: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .flat_map(|path| {
            store_reaches_in_source(&std::fs::read_to_string(&path).unwrap())
                .into_iter()
                .map(move |what| format!("{path}: {what}"))
        })
        .collect();
    assert!(
        tree_reaches.is_empty(),
        "interface reaches a store on the syntax tree: {tree_reaches:#?}"
    );
    let whole_interface: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .collect();
    for needle in INTERFACE_FORBIDDEN_ADAPTERS {
        let owners: Vec<String> = whole_interface
            .iter()
            .filter(|path| {
                production_code(path)
                    .iter()
                    .any(|(_, line)| line.contains(needle))
            })
            .cloned()
            .collect();
        assert!(
            owners.is_empty(),
            "interface names `{needle}` in {owners:?}"
        );
    }
    // The sessions edge names only the capability's DTOs and use cases.
    for path in files_under("src/interface/uds/sessions")
        .into_iter()
        .filter(|p| !p.ends_with("_tests.rs"))
    {
        for (n, line) in production_code(&path) {
            if line.contains("crate::application::sessions::") {
                assert!(
                    line.contains("::dto") || line.contains("::use_cases"),
                    "{path}:{n} imports beyond the capability's DTOs/use cases: {}",
                    line.trim()
                );
            }
            assert!(
                !line.contains("crate::interface::cli") && !line.contains("crate::infrastructure"),
                "{path}:{n} reaches outside the sessions edge: {}",
                line.trim()
            );
        }
    }
}

/// Infrastructure implements the ports at exact sites and never constructs
/// a use case; the durable store's holders are an exact set.
/// Admitted raw-key sites are exact and decrease-only.
#[test]
fn raw_key_conversion_and_startup_key_sites_are_exact() {
    assert_eq!(
        production_files_calling("SessionIdentity::from_persisted_key("),
        set(RAW_KEY_CONVERSION_SITES),
        "the raw-key conversion sites are exact (decrease-only)"
    );
    assert_eq!(
        production_files_calling("Session::build_key("),
        set(STARTUP_KEY_SITES),
        "the raw startup-key formation sites are exact (decrease-only)"
    );
    // The recall tool converts at the tools port and nowhere else in
    // infrastructure; the raw tools port is the tools capability's.
    let converters: BTreeSet<String> = production_files_calling("from_persisted_key(")
        .into_iter()
        .filter(|p| p.starts_with("src/infrastructure/tools/"))
        .collect();
    assert_eq!(converters, set(&["src/infrastructure/tools/recall.rs"]));
    // The loop's runtime inputs are typed: no interface handle struct
    // carries a raw session key.
    let handles = production_code("src/interface/cli/uds_session_handles.rs");
    assert!(
        handles
            .iter()
            .any(|(_, line)| line.contains("pub identity: SessionIdentity,")),
        "SessionLoopInputs carries the typed identity"
    );
    assert!(
        !handles
            .iter()
            .any(|(_, line)| line.contains("session_key: String")),
        "SessionLoopInputs carries no raw key"
    );
}

/// Retirement: exact paths and names are gone from production code, and
/// the docs present none of them as current.
#[test]
fn retired_paths_and_names_are_absent() {
    for path in RETIRED_PATHS {
        assert!(!Path::new(path).exists(), "{path} was retired by the epic");
    }
    // Built by concatenation so this file does not match its own needle.
    let singular_root = ["src/application/", "session"].concat();
    assert!(
        !Path::new(&singular_root).exists(),
        "the singular root is gone"
    );
    assert!(!Path::new(&format!("{singular_root}.rs")).exists());
    for name in RETIRED_NAMES {
        let survivors = production_files_calling(name);
        assert!(survivors.is_empty(), "`{name}` survives in {survivors:?}");
    }
    // The tracker keeps no raw session key at all: every `sessionKey` the
    // interface presents comes from the active session's identity.
    let tracker = production_code("src/interface/cli/uds_session.rs");
    assert!(
        !tracker.iter().any(|(_, line)| {
            line.trim() == "session_key: String," || line.contains("fn session_key(")
        }),
        "AgentSession holds no raw session-key copy and exposes none"
    );
    let docs: Vec<String> = files_under("docs")
        .into_iter()
        .chain(files_under("../docs"))
        .filter(|p| p.ends_with(".md"))
        .collect();
    for name in [
        "ConversationSnapshotData",
        "agent_session_identity",
        "uds_session_load",
        "uds_busy_sync",
        "persist_current_session",
        &["application/", "session/"].concat(),
    ] {
        let stale: Vec<&String> = docs
            .iter()
            .filter(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains(name)))
            .collect();
        assert!(
            stale.is_empty(),
            "docs still name retired `{name}`: {stale:?}"
        );
    }
}

/// Disposition inventory (non-empty, exact): the interface modules that
/// stayed remain in their allowlisted owner roles with no sessions
/// persistence call; `agent_session_identity.rs` was retired by D7.
#[test]
fn disposition_inventory_is_exact_and_persistence_free() {
    assert!(!DISPOSITION.is_empty());
    for (file, role) in DISPOSITION {
        assert!(Path::new(file).is_file(), "{file} stays in the interface");
        let source = std::fs::read_to_string(file).unwrap();
        let header: String = source.lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(
            header.to_lowercase().contains(role),
            "{file} states its `{role}` owner role in its header"
        );
        let code = production_code(file);
        for needle in [
            "SessionStore",
            "ContextSpillStore",
            "SaveSession",
            "use_cases::",
            "active_session",
            "from_persisted_key",
            "FlatSessionLayout",
        ] {
            assert!(
                !code.iter().any(|(_, line)| line.contains(needle)),
                "{file} names `{needle}`: the disposition modules hold no persistence"
            );
        }
        let reaches: Vec<String> = code
            .iter()
            .filter(|(_, line)| reaches_a_persistence_method(line))
            .map(|(n, line)| format!("{file}:{n}: {}", line.trim()))
            .collect();
        assert!(reaches.is_empty(), "{reaches:#?}");
    }
    assert!(
        !Path::new("src/interface/cli/agent_session_identity.rs").exists(),
        "agent_session_identity.rs was retired by D7 (#1976)"
    );
    assert!(
        production_files_calling("agent_session_identity").is_empty(),
        "nothing declares or names the retired identity module"
    );
}

/// Documentation lockstep: the docs name every use case of the tree, every
/// port, every session command, the one layout and the workspace seam.
#[test]
fn documentation_names_every_use_case_port_command_and_the_seam() {
    let sessions = std::fs::read_to_string("docs/sessions.md").unwrap();
    let map = std::fs::read_to_string("docs/architecture/harness-architecture-map.md").unwrap();
    let matrix =
        std::fs::read_to_string("docs/architecture/protocol-capability-matrix.md").unwrap();
    let protocol = std::fs::read_to_string("docs/uds-protocol.md").unwrap();
    let cookbooks = std::fs::read_to_string("docs/contributor-cookbooks.md").unwrap();
    for use_case in use_case_tree() {
        assert!(
            sessions.contains(&format!("`{use_case}`")),
            "docs/sessions.md must name the use case `{use_case}`"
        );
        assert!(
            map.contains(&use_case),
            "docs/architecture/harness-architecture-map.md must name `{use_case}`"
        );
    }
    for (port, _) in PORT_IMPLEMENTORS {
        assert!(
            sessions.contains(&format!("`{port}`")),
            "docs/sessions.md must name the port `{port}`"
        );
    }
    for command in SESSION_COMMANDS {
        assert!(
            protocol.contains(&format!("### `{command}`")),
            "docs/uds-protocol.md documents `{command}` under its own heading"
        );
    }
    for needle in [
        "`FlatSessionLayout`",
        "`SessionIdentity`",
        "`ActiveSessionState`",
        "`ConversationLedger`",
        "composition/sessions.rs",
        "composition/active_session.rs",
        "composition/session_report.rs",
        "composition/retention.rs",
        "#2009",
        "Home metadata",
        "global transcript store",
        "Legacy records",
        "unavailable",
        "`SessionHomeCatalogue`",
        "`WorkspaceDiscovery`",
        "Tool::set_session_key",
    ] {
        assert!(
            sessions.contains(needle),
            "docs/sessions.md must state {needle}"
        );
    }
    assert!(
        matrix.contains("#1968") && matrix.contains("Unchanged"),
        "the protocol capability matrix records the unchanged session protocol at epic close"
    );
    assert!(
        protocol.contains("#1968"),
        "docs/uds-protocol.md names the epic that fixed the session command owners"
    );
    assert!(
        cookbooks.contains("sessions") && cookbooks.contains("composition"),
        "the contributor cookbooks point at the sessions capability's composition rule"
    );
    let feature = std::fs::read_to_string("tests/features/tui_clean_architecture.feature").unwrap();
    assert!(
        feature.contains("@issue-1979"),
        "the TUI architecture feature pins the unchanged session protocol at epic close"
    );
}
