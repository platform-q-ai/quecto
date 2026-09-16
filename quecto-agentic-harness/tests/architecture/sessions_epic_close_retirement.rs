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
    "impl SessionStore for",
    "impl ContextSpillStore for",
    "impl SessionExportPort for",
    "SessionIdentity::from_persisted_key(",
    "SessionIdentity::fresh_chat(",
];

/// Orchestration verbs the session handlers admit, request and present
/// around but never sequence themselves.
const RAW_KEY_CONVERSION_SITES: &[&str] = &[
    "src/application/agent_loop.rs",
    "src/application/sessions/use_cases/read_history.rs",
    "src/infrastructure/persistence/session_store.rs",
    "src/infrastructure/persistence/session_store_list.rs",
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
        "let sessions = ctx.list_sessions.list_all().await;",
    ] {
        assert!(
            !reaches_a_persistence_method(benign),
            "must spare: {benign}"
        );
    }
}

/// Inventory: every sessions use case is declared exactly once, under the
/// capability's use_cases folder, and the exact list is the tree's.
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
        "#1966",
        "no scope field",
        "no workspace behaviour",
        "repository-layout adapter",
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
