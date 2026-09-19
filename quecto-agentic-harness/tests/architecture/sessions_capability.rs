//! The sessions capability (#1968, D1 #1970, D2 #1971, D3 #1973, D4 #1974, D5 #1972, D6 #1975, D7 #1976, D8 #1977, D9 #1978, D10 #1979): exact-inventory
//! ratchets for the plural capability, its composition sites, and the
//! single owner of the flat storage layout. Affirmative throughout: each
//! check states the set that is allowed and asserts the observed set equals
//! it, so an unknown future owner fails by not being listed.
//!
//! - R1: the canonical files of the capability exist; the singular
//!   `application/session` path, every singular import and any alias
//!   re-export are absent.
//! - R5: every sessions use case, controller, the file store and the one
//!   active-session state (R7a) are constructed only at their named
//!   composition file; interface and infrastructure hold injected handles.
//! - R7a/R9: one owner of live-conversation state and of history/recovery/
//!   sync/report policy: the interface neither declares a conversation
//!   ledger nor pages or ranges history, reconciles revisions, selects
//!   reports, bounds or writes exports itself, and converts no raw key
//!   into an identity.
//! - D4: the export root literal is composition's alone; no interface file
//!   names the export adapter.
//! - D5: one owner of the save transaction: the interface holds no
//!   persistence policy (watermark, dirty latch, killing exit, roster
//!   snapshot, ordinal assignment) and never writes the store directly.
//! - D6: one owner each of the clear and rewind transactions: the
//!   interface neither edits the conversation, resolves a rewind target,
//!   resets the ledger or the watermark, nor clears the retention
//!   namespace itself; the handlers admit, request and present.
//! - D7: one owner of the fresh-session transaction and of the
//!   departing-children settlement: the interface generates no session
//!   key, holds no fleet/reset ordering or refusal text, keeps no raw
//!   session-key copy on the dispatch context, and switches no identity
//!   itself; the loop's raw-key holders adopt the identity through the
//!   propagation port from exactly one adapter.
//! - D8: one owner of the resume transaction and of the startup open: the
//!   interface admits no target, builds no key, claims, loads or releases
//!   no session, holds no store on the dispatch context and spells none of
//!   the resume refusal texts; the persisted-roster policy (#1937) and the
//!   settle-before-switch order (#1938) are the application's.
//! - D9: one owner of durable retained context: the sessions capability
//!   declares the retention port, owns the recall/list/clear selection and
//!   the id append/deduplication; the context-pruning policy decides when
//!   and what to retain and consumes the narrow writer/reader handles,
//!   never a store method; the recall tool parses, formats and diagnoses
//!   over the composed use case; composition alone constructs the store,
//!   the use cases and the pruning handles; no interface file constructs
//!   the store, forms the layout or scrubs the ephemeral file itself.
//!   Every inventory is affirmative and non-empty.
//! - R7: exactly one infrastructure `FlatSessionLayout` joins `sessions`,
//!   calls the sanitizer and forms the `.json`/`.owner`/`spill.jsonl`
//!   names; the layout is created at an exact, non-growing set of sites.
//! - Retirement: no interface production code lists the store directly.
//! - D10 (#1979, epic close): the tracker holds no raw session key — the
//!   presenters read the active session's identity, the loop's startup
//!   identity is typed end to end; the exact end-state inventories live in
//!   `sessions_epic_close.rs` / `sessions_epic_close_retirement.rs`.
//! - Ceilings: the per-owner line ceilings are non-empty and decrease-only.
//!
//! `LINE_CEILINGS` and `LAYOUT_CREATION_SITES` are decrease-only by review
//! policy, not mechanically: the test asserts the current inventory/ceiling
//! holds, and a change that raises a ceiling or adds a site is a reviewed
//! edit of this file that the PR must justify.

use std::collections::BTreeSet;
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// The files the plural capability is made of (R1).
const CANONICAL_FILES: &[&str] = &[
    // #2009 extends the existing owners with home discovery, not scoped keys.
    "src/domain/session_home.rs",
    "src/application/sessions/dto/list_sessions.rs",
    "src/application/sessions/ports/session_home.rs",
    "src/application/sessions/session_home.rs",
    "src/infrastructure/persistence/session_home_catalogue.rs",
    "src/infrastructure/persistence/session_home_catalogue_index.rs",
    "src/infrastructure/persistence/session_store_home.rs",
    "src/infrastructure/persistence/session_store_list_index.rs",
    "src/infrastructure/workspace/git_scope_discovery.rs",
    "src/infrastructure/workspace/filesystem_scope.rs",
    "src/application/sessions/mod.rs",
    "src/application/sessions/ports.rs",
    "src/application/sessions/dto/mod.rs",
    "src/application/sessions/use_cases/mod.rs",
    "src/application/sessions/use_cases/list_sessions.rs",
    "src/application/sessions/use_cases/list_sessions_discover.rs",
    "src/application/sessions/use_cases/list_sessions_observations.rs",
    "src/application/sessions/use_cases/save_session_home.rs",
    "src/application/sessions/use_cases/resume_saved_session_admission.rs",
    "src/application/sessions/use_cases/resume_saved_session_startup.rs",
    "src/application/sessions/dto/resume_disposition.rs",
    "src/application/sessions/dto/startup_refusal.rs",
    // #2011 typed resume decisions extend the resume owner, not a new one.
    "src/domain/resume_decision.rs",
    "src/application/sessions/dto/resume_decision.rs",
    "src/application/sessions/dto/resume_refusal_text.rs",
    "src/application/sessions/dto/resume_refusal_code.rs",
    "src/application/sessions/dto/resume_target.rs",
    "src/domain/stable_digest.rs",
    "src/interface/cli/uds_safe_display.rs",
    "src/application/sessions/use_cases/resume_saved_session_decision.rs",
    "src/interface/uds/sessions/resume_session_controller.rs",
    "src/interface/cli/uds_dispatch_resume.rs",
    // #2010 global metadata search: one new query owner and its edges.
    "src/domain/session_metadata_search.rs",
    "src/application/sessions/dto/search_session_metadata.rs",
    "src/application/sessions/use_cases/search_session_metadata.rs",
    "src/infrastructure/persistence/session_home_catalogue_metadata.rs",
    "src/composition/session_search.rs",
    "src/interface/uds/sessions/search_session_metadata_controller.rs",
    "src/interface/cli/uds_discovery_handles.rs",
    "src/interface/cli/uds_dispatch_search.rs",
    // #2010 review round 1: the text fold, the request's numbers (typed and
    // on the wire), the rejection cache and the walk's seeding.
    "src/domain/session_metadata_text.rs",
    "src/application/sessions/dto/search_limits.rs",
    "src/interface/cli/uds_search_numbers.rs",
    "src/infrastructure/persistence/session_home_catalogue_rejections.rs",
    // Round 3 (R3-H1): naming the walk's skips, linear in the bad records.
    "src/infrastructure/persistence/session_home_catalogue_skipped.rs",
    "src/infrastructure/persistence/session_home_catalogue_seed.rs",
    "src/infrastructure/persistence/session_record_read.rs",
    "src/domain/session_path_text.rs",
    "src/domain/session_query_refusal.rs",
    "src/interface/cli/protocol_search_rescue.rs",
    "src/interface/cli/uds_freshness_json.rs",
    "src/application/sessions/use_cases/read_history.rs",
    "src/application/sessions/use_cases/recover_message.rs",
    "src/application/sessions/use_cases/export_session_report.rs",
    "src/application/sessions/use_cases/synchronize_transcript.rs",
    "src/application/sessions/use_cases/save_session.rs",
    "src/application/sessions/use_cases/clear_conversation.rs",
    "src/application/sessions/use_cases/rewind_conversation.rs",
    "src/application/sessions/use_cases/start_fresh_conversation.rs",
    "src/application/sessions/use_cases/departing_children.rs",
    "src/application/sessions/use_cases/resume_saved_session.rs",
    "src/application/sessions/use_cases/recall_context.rs",
    "src/application/sessions/use_cases/retain_context.rs",
    "src/application/sessions/ports/export.rs",
    "src/application/sessions/ports/session_runtime.rs",
    "src/application/sessions/ports/session_transition.rs",
    "src/application/sessions/dto/history.rs",
    "src/application/sessions/dto/message_recovery.rs",
    "src/application/sessions/dto/session_report.rs",
    "src/application/sessions/dto/sync.rs",
    "src/application/sessions/dto/save_session.rs",
    "src/application/sessions/dto/clear_conversation.rs",
    "src/application/sessions/dto/rewind_conversation.rs",
    "src/application/sessions/dto/start_fresh_conversation.rs",
    "src/application/sessions/dto/resume_saved_session.rs",
    "src/application/sessions/dto/retained_context.rs",
    "src/application/sessions/active_session.rs",
    "src/application/sessions/conversation_ledger.rs",
    "src/application/sessions/history_paging.rs",
    "src/application/durable_prefix.rs",
    "src/infrastructure/persistence/session_snapshot_sources.rs",
    "src/infrastructure/persistence/fresh_session_identity.rs",
    "src/infrastructure/tools/delegated_roster.rs",
    "src/composition/sessions.rs",
    "src/composition/active_session.rs",
    "src/composition/session_report.rs",
    "src/composition/fleet_settlement.rs",
    "src/composition/retention.rs",
    "src/interface/cli/uds_session_handles.rs",
    "src/interface/cli/retention_handles.rs",
    "src/infrastructure/persistence/context_spill.rs",
    "src/infrastructure/tools/recall.rs",
    "src/interface/cli/uds_sync.rs",
    "src/interface/cli/uds_turn_accounting.rs",
    "src/interface/cli/uds_session_switch_runtime.rs",
    "src/interface/uds/sessions/controller.rs",
    "src/interface/uds/sessions/read_history_controller.rs",
    "src/interface/uds/sessions/recover_message_controller.rs",
    "src/interface/uds/sessions/export_report_controller.rs",
    "src/infrastructure/session_export.rs",
    "src/interface/uds/sessions/synchronize_transcript_controller.rs",
    "src/interface/uds/sessions/rewind_conversation_controller.rs",
    "src/domain/conversation_view.rs",
    "src/domain/conversation_edit.rs",
    "src/infrastructure/persistence/session_layout.rs",
    "src/domain/session_identity.rs",
];

/// The ports of the capability, each with its contract suite (R2 subset).
const SESSION_PORTS: &[(&str, &str)] = &[
    (
        "SessionHomeCatalogue",
        "tests/contracts/session_home_catalogue.rs",
    ),
    (
        "WorkspaceDiscovery",
        "tests/contracts/workspace_discovery.rs",
    ),
    ("SessionStore", "tests/contracts/session_store.rs"),
    (
        "ContextSpillStore",
        "tests/contracts/context_spill_store.rs",
    ),
    (
        "SessionExportPort",
        "tests/contracts/session_export_port.rs",
    ),
    (
        "DurablePrefixObservation",
        "tests/contracts/durable_prefix_observation.rs",
    ),
    (
        "WorkflowRunSource",
        "tests/contracts/workflow_run_source.rs",
    ),
    (
        "HistoricalRosterSource",
        "tests/contracts/historical_roster_source.rs",
    ),
    (
        "TurnAccountingReset",
        "tests/contracts/turn_accounting_reset.rs",
    ),
    // D7 #1976: the transition ports.
    (
        "FreshSessionIdentityGenerator",
        "tests/contracts/fresh_session_identity_generator.rs",
    ),
    ("FleetSettlement", "tests/contracts/fleet_settlement.rs"),
    (
        "DelegatedChildrenRoster",
        "tests/contracts/delegated_children_roster.rs",
    ),
    (
        "SessionKeyPropagation",
        "tests/contracts/session_key_propagation.rs",
    ),
    (
        "SessionSwitchRuntime",
        "tests/contracts/session_switch_runtime.rs",
    ),
];

/// Where the capability's ports may be declared: the ports file and the
/// files under its `ports/` directory (D5 #1972).
fn is_ports_file(path: &str) -> bool {
    path == "src/application/sessions/ports.rs"
        || path.starts_with("src/application/sessions/ports/")
}

/// Where each sessions graph node may be constructed (R5, R7a): exactly one
/// composition file per constructor (constructor, file).
const COMPOSED_CONSTRUCTORS: &[(&str, &str)] = &[
    ("ListSessions::new(", "src/composition/sessions.rs"),
    (
        "ListSessionsController::new(",
        "src/composition/sessions.rs",
    ),
    // #2010 metadata search: its use case and controller beside the list
    // controller in the discovery handles.
    (
        "SearchSessionMetadata::new(",
        "src/composition/session_search.rs",
    ),
    (
        "SearchSessionMetadataController::new(",
        "src/composition/session_search.rs",
    ),
    ("FileSessionStore::new(", "src/composition/sessions.rs"),
    (
        "ActiveSessionState::new(",
        "src/composition/active_session.rs",
    ),
    ("ReadHistory::new(", "src/composition/active_session.rs"),
    ("RecoverMessage::new(", "src/composition/active_session.rs"),
    (
        "ReadHistoryController::new(",
        "src/composition/active_session.rs",
    ),
    (
        "RecoverMessageController::new(",
        "src/composition/active_session.rs",
    ),
    (
        "ExportSessionReport::new(",
        "src/composition/session_report.rs",
    ),
    (
        "ExportSessionReportController::new(",
        "src/composition/session_report.rs",
    ),
    (
        "FileSessionExport::new(",
        "src/composition/session_report.rs",
    ),
    (
        "SynchronizeTranscript::new(",
        "src/composition/active_session.rs",
    ),
    (
        "SynchronizeTranscriptController::new(",
        "src/composition/active_session.rs",
    ),
    ("SaveSession::new(", "src/composition/active_session.rs"),
    (
        "WorkflowEngineRunSource::new(",
        "src/composition/active_session.rs",
    ),
    (
        "RegistryRosterSource::new(",
        "src/composition/active_session.rs",
    ),
    (
        "ClearConversation::new(",
        "src/composition/active_session.rs",
    ),
    (
        "RewindConversation::new(",
        "src/composition/active_session.rs",
    ),
    // D7 #1976.
    (
        "StartFreshConversation::new(",
        "src/composition/active_session.rs",
    ),
    (
        "DepartingChildren::new(",
        "src/composition/active_session.rs",
    ),
    // D8 #1977.
    (
        "ResumeSavedSession::new(",
        "src/composition/active_session.rs",
    ),
    (
        "RegistryDelegatedRoster::new(",
        "src/composition/active_session.rs",
    ),
    (
        "ProcessClockIdentityGenerator::new(",
        "src/composition/sessions.rs",
    ),
    // D9 #1978: the retention store beside the session store, the
    // recall/retain/list graph over it.
    ("FileContextSpillStore::new(", "src/composition/sessions.rs"),
    ("RecallContext::new(", "src/composition/retention.rs"),
    ("RetainContext::new(", "src/composition/retention.rs"),
    ("ListRetainedContext::new(", "src/composition/retention.rs"),
    (
        "RecallTool::new(",
        "src/infrastructure/extensions/native.rs",
    ),
];

/// The one production file that may spell the export root (D4): the loop's
/// `artifacts/session-exports` is a composition runtime input, never formed
/// by a dispatch branch or a loop.
const EXPORT_ROOT_OWNER: &str = "src/composition/session_report.rs";

/// The history/recovery/sync policy vocabulary no interface production
/// file may declare or spell (R9): the read model, the paging and ranging
/// primitives, the raw-key conversion the active session retired, and the
/// epoch/revision reconciliation the sync use case owns (D3 #1973: the
/// interface never reads the ledger frontier or decides reset-or-delta).
const INTERFACE_FORBIDDEN_NEEDLES: &[&str] = &[
    "struct ConversationSnapshotData",
    "fn messages_page_json_for_id(",
    "fn position_by_wire_id(",
    "fn position_by_message_id(",
    "fn nearest_char_boundary_at_or_before(",
    "fn resolve_get_message(",
    "SessionIdentity::from_persisted_key(",
    "fn latest_report_resolving(",
    "fn export_source(",
    "ExportRootSlot",
    "static EXPORTS: tokio::sync::Semaphore",
    "session_export::",
    "artifacts/session-exports",
    "fn sync_json(",
    "fn sync_message_json(",
    ".frontier()",
    "resync =",
    // D5 (#1972): the save transaction's policy lives in the application.
    "fn persist_current_session",
    "fn snapshot_subagent_roster",
    "assign_missing_ordinals",
    "last_persisted_message_index",
    "durable_prefix_dirty",
    "killing_exit =",
    "fn inject_system_prompt(",
    "fn remove_injected_system_prompt(",
    "session_store_ordinals",
    // D6 (#1975): the clear and rewind transactions' edits, target
    // resolution, ledger reset and refusal texts live in the domain and the
    // application; the interface admits, requests and presents. (The fresh
    // session's own watermark reset and spill clear are D7 #1976's.)
    "fn clear_conversation(",
    "fn resolve_rewind_target(",
    "fn rewind_to_message_index(",
    "fn truncate_at_user_message(",
    "fn remove_spill_references(",
    "fn reset_to(",
    "rewind target not found",
    "invalid rewind target",
    "rewind requires messageId",
    "failed to save cleared session",
    "failed to save rewound session",
    // D7 (#1976): the fresh-session transaction — key generation, the
    // fleet/reset ordering and its refusal texts, the identity switch and
    // the raw session-key copy of the dispatch context — lives in the
    // application; the handler admits, requests and presents.
    "fn generate_chat_key(",
    "fn generate_chat_identity(",
    "fn resolve_uds_session_key(",
    "fresh_chat(",
    "fn settle_departing_children(",
    "fn reset_subagent_roster(",
    "fn live_delegated_rows(",
    "enum TransitionRefused",
    "could not be settled; the current session was kept",
    "subagent teardown was interrupted",
    "no fleet teardown is available",
    "remain after the teardown",
    "fn reset_to_with_spill_store(",
    ".switch_identity(",
    "session_key: &'a mut String",
    // D8 (#1977): the resume transaction — target admission and key
    // building, the claim/load/release sequence and its refusal texts, the
    // startup open, the history/workflow restore and the persisted-roster
    // policy — lives in the application; the handler and the startup paths
    // admit, request and present. No interface production line claims,
    // loads or releases a session, or holds the store on the dispatch
    // context.
    "fn note_persisted_roster_is_history(",
    "fn set_workflow_run(",
    "fn sync_message_count(",
    "fn load_session(",
    "cannot resume sessions in ephemeral mode",
    "failed to save current session",
    "session not found:",
    "failed to load session",
    "strip_prefix(\"cli:\")",
    "USER_CHAT_PREFIX",
    "session_store.claim(",
    "session_store.load(",
    "session_store.release(",
    "session_store: &'a dyn SessionStore",
    ".departing_children",
    "SessionTransition::Resume",
    // D9 (#1978): retained context — the store, its file, the recall
    // graph and the ephemeral scrub — is composed and owned outside the
    // interface; no interface line constructs the store, names the
    // adapter, forms its layout, scrubs its file or reaches a store method.
    "FileContextSpillStore",
    "context_spill::",
    "fn scrub_ephemeral_spill(",
    "scrub_session_spill_sync",
    ".scrub_sync(",
    "RecallTool::new(",
    "tools::recall::",
    ".list_entries(",
    ".has_entries(",
    "fn spill_store(",
];

/// A line that reaches a retention store method by its identity-keyed
/// argument shape — `.append(&identity, …)`, `.recall(&identity, …)`,
/// `.clear(&identity)` — whatever the binding is called (D9 #1978). The
/// `(&mut` shape of `Vec::append` and the argument-less `.clear()` are not
/// store reaches.
fn reaches_a_retention_store(line: &str) -> bool {
    [".append(&", ".recall(&", ".clear(&"].iter().any(|needle| {
        line.match_indices(needle)
            .any(|(index, _)| !line[index + needle.len()..].starts_with("mut "))
    })
}

#[test]
fn retention_store_reach_predicate_catches_a_rogue_call_and_spares_vec_and_set() {
    for rogue in [
        "retention.store.recall(&identity, &SpillId::new(id)).await",
        "store.clear(&identity).await?;",
        "handles.store.append(&state.identity().clone(), &entry).await",
        "ctx.retention.as_ref().unwrap().store.recall(&key, &id)",
    ] {
        assert!(reaches_a_retention_store(rogue), "must catch: {rogue}");
    }
    for benign in [
        "messages.append(&mut other);",
        "seen.clear();",
        "self.recall_counts.lock().unwrap().clear();",
        "let page = history.recall(page_size);",
    ] {
        assert!(!reaches_a_retention_store(benign), "must spare: {benign}");
    }
}

/// D9 (#1978): no interface production line reaches a retention store
/// method, whatever the binding is called.
#[test]
fn interface_never_reaches_a_retention_store_method() {
    let reaches: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .flat_map(|path| {
            production_code(&path)
                .into_iter()
                .filter(|(_, line)| reaches_a_retention_store(line))
                .map(move |(n, line)| format!("{path}:{n}: {}", line.trim()))
        })
        .collect();
    assert!(
        reaches.is_empty(),
        "interface reaches a retention store method: {reaches:#?}"
    );
}

/// The interface files still holding a raw persisted key the active session
/// does not stand for (exact, decrease-only): none since D8 #1977 moved the
/// resume target's admission into the application.
const RAW_KEY_CONVERSION_SITES: &[&str] = &[];

/// The exact production files that request the injected save transaction
/// (D5 #1972): the interface's persistence triggers. Decrease-only (D8
/// #1977 moved the resume's transition save into the application).
const SAVE_REQUESTERS: &[&str] = &[
    "src/interface/cli/agent/run_session.rs",
    "src/interface/cli/uds.rs",
    "src/interface/cli/uds_dispatch.rs",
    "src/interface/cli/uds_multi.rs",
    "src/interface/cli/uds_single_client.rs",
];

/// The trigger each persistence site requests: explicit, post-turn and
/// transition saves, the pre-turn prompt save, and the ordinary exits.
const SAVE_TRIGGER_SITES: &[(&str, &str)] = &[
    (
        "src/interface/cli/uds_dispatch.rs",
        "SaveTrigger::Explicit {",
    ),
    ("src/interface/cli/uds.rs", "SaveTrigger::Routine"),
    ("src/interface/cli/uds.rs", ".save_with_pending_prompt("),
    (
        "src/interface/cli/uds_single_client.rs",
        "SaveTrigger::OrdinaryExit",
    ),
    (
        "src/interface/cli/uds_multi.rs",
        "SaveTrigger::OrdinaryExit",
    ),
    (
        "src/interface/cli/agent/run_session.rs",
        "SaveTrigger::OrdinaryExit",
    ),
];

/// The exact sites that create a `FlatSessionLayout` (R7). Decrease-only:
/// D9 #1978 retired the two interface sites (the ephemeral spill scrub and
/// the tool runtime's spill store) behind the sessions composition;
/// nothing may join this list.
const LAYOUT_CREATION_SITES: &[&str] = &["src/composition/sessions.rs"];

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
// #2009: scoped discovery/admission extends existing owners. After the review
// splits (list_sessions_discover, save_session_home, resume_saved_session_admission,
// dto/resume_disposition, composition/session_home, uds_dispatch_discovery) the
// owners that remain above master are: ports.rs 148→149 (the `session_home`
// port module declaration), save_session.rs 268→282 (home context field, the
// two prepare_home calls), resume_saved_session.rs 191→208 (mandatory home
// context, admission under the claim guard at load and startup),
// dto/resume_saved_session.rs 114→123 (the `Scope` refusal and its Display),
// active_session.rs 117→126 (the home context handed to save and resume),
// composition/sessions.rs 61→70 (`build_session_handles_over`),
// session_layout.rs 83→94 (the `.home` and `home.catalogue` paths),
// uds_dispatch_session.rs 241→259 (`handle_list_sessions`),
// controller.rs 36→42 (the typed scoped result). Every pin is the delivered
// size, retains the 750-line cap and remains decrease-only.
const LINE_CEILINGS: &[(&str, usize)] = &[
    ("src/application/durable_prefix.rs", 42),
    // D7 #1976 folds the interface reset composition into `switch_to`
    // (was 277 before D7); D8 #1977 notes the switch is the transactions' alone
    // (was 287 before D8).
    ("src/application/sessions/active_session.rs", 289),
    ("src/application/sessions/conversation_ledger.rs", 295),
    ("src/application/sessions/dto/history.rs", 90),
    ("src/application/sessions/dto/message_recovery.rs", 185),
    ("src/application/sessions/dto/session_report.rs", 150),
    ("src/application/sessions/dto/sync.rs", 65),
    ("src/application/sessions/dto/save_session.rs", 68),
    ("src/application/sessions/dto/clear_conversation.rs", 50),
    ("src/application/sessions/dto/rewind_conversation.rs", 90),
    (
        "src/application/sessions/dto/start_fresh_conversation.rs",
        136,
    ),
    ("src/application/sessions/history_paging.rs", 65),
    // D7 #1976 declares and re-exports the transition ports module (was
    // 135 before D7); D9 #1978 adds the ephemeral scrub to the retention
    // port (was 142 before D9).
    ("src/application/sessions/ports.rs", 149),
    ("src/application/sessions/ports/export.rs", 30),
    // D6 #1975 adds the accounting-reset port beside the save observations
    // (was 29 before D6).
    ("src/application/sessions/ports/session_runtime.rs", 42),
    // D8 #1977 adds the workflow restore to the switch runtime port (was
    // 60 before D8).
    ("src/application/sessions/ports/session_transition.rs", 66),
    (
        "src/application/sessions/use_cases/export_session_report.rs",
        180,
    ),
    // #2009: query-local catalogue projection and Git discovery cache live in
    // the helper; this owner is the listing query and remains decrease-only.
    ("src/application/sessions/use_cases/list_sessions.rs", 57),
    // R2-M1 reads the exact authority for a record the catalogue has no row
    // for; the query-local observation cache moved to its own helper so the
    // projection stays under its ceiling (141 → 125) and the cache is pinned
    // at its delivered size.
    (
        "src/application/sessions/use_cases/list_sessions_discover.rs",
        125,
    ),
    (
        "src/application/sessions/use_cases/list_sessions_observations.rs",
        49,
    ),
    ("src/application/sessions/use_cases/read_history.rs", 90),
    ("src/application/sessions/use_cases/recover_message.rs", 140),
    (
        "src/application/sessions/use_cases/synchronize_transcript.rs",
        110,
    ),
    ("src/application/sessions/use_cases/save_session.rs", 282),
    (
        "src/application/sessions/use_cases/save_session_home.rs",
        38,
    ),
    (
        "src/application/sessions/use_cases/clear_conversation.rs",
        105,
    ),
    (
        "src/application/sessions/use_cases/rewind_conversation.rs",
        145,
    ),
    (
        "src/application/sessions/use_cases/start_fresh_conversation.rs",
        116,
    ),
    // D8 #1977 adds the history-only note of a resumed roster (was 112
    // before D8).
    (
        "src/application/sessions/use_cases/departing_children.rs",
        127,
    ),
    (
        "src/application/sessions/use_cases/resume_saved_session.rs",
        208,
    ),
    // R2-H1/H2: startup admission (the actionable refusal, the orphan-home
    // rule) is its own helper; the shared admission shrank 67 → 48.
    // #2011: the shared admission is the decision helper's now; this file
    // keeps the claim guard alone (48 → 31) and startup reads the home itself
    // (56 → 54).
    (
        "src/application/sessions/use_cases/resume_saved_session_admission.rs",
        31,
    ),
    (
        "src/application/sessions/use_cases/resume_saved_session_startup.rs",
        54,
    ),
    // #2011: the eligibility collaborators of the one resume transaction —
    // request admission, the effect-free pre-flight, the claimed re-check.
    // #2045 removes explicit resume actions; eligibility remains effect-free.
    (
        "src/application/sessions/use_cases/resume_saved_session_decision.rs",
        147,
    ),
    // #2011: the refusal text moved to its own module (123 → 117 with the
    // typed decision and refusal variants added).
    // Review R1: the target and the stable codes are their own modules
    // (117 → 91, 61 → 47).
    ("src/application/sessions/dto/resume_saved_session.rs", 91),
    ("src/application/sessions/dto/resume_refusal_text.rs", 47),
    ("src/application/sessions/dto/resume_refusal_code.rs", 25),
    ("src/application/sessions/dto/resume_target.rs", 39),
    ("src/application/sessions/dto/resume_decision.rs", 162),
    // Review R1: the digest is its own pure module (182 → 165).
    ("src/domain/resume_decision.rs", 163),
    ("src/domain/stable_digest.rs", 30),
    (
        "src/interface/uds/sessions/resume_session_controller.rs",
        55,
    ),
    ("src/interface/cli/uds_dispatch_resume.rs", 100),
    ("src/application/sessions/dto/resume_disposition.rs", 30),
    ("src/application/sessions/dto/startup_refusal.rs", 40),
    // D9 #1978: retained context.
    ("src/application/sessions/use_cases/recall_context.rs", 80),
    ("src/application/sessions/use_cases/retain_context.rs", 118),
    ("src/application/sessions/dto/retained_context.rs", 92),
    ("src/composition/retention.rs", 35),
    ("src/interface/cli/retention_handles.rs", 33),
    ("src/infrastructure/tools/recall.rs", 183),
    ("src/application/context.rs", 336),
    ("src/application/context_pruning.rs", 248),
    ("src/application/context_pruning_messages.rs", 304),
    ("src/application/agent_loop_spill.rs", 56),
    // D3 #1973, D4 #1974, D5 #1972, D6 #1975, D7 #1976 and D8 #1977 each
    // add use cases to this graph; the ceiling follows their merge (was 113
    // before D8); D10 #1979 drops the raw-key conversion (was 119).
    ("src/composition/active_session.rs", 126),
    ("src/composition/session_report.rs", 40),
    // D7 #1976 adds the fresh-identity generator builder (was 38 before D7);
    // D9 #1978 adds the retention store and graph builder (was 48 before D9).
    ("src/composition/sessions.rs", 70),
    ("src/composition/session_home.rs", 38),
    ("src/composition/fleet_settlement.rs", 39),
    (
        "src/infrastructure/persistence/session_snapshot_sources.rs",
        61,
    ),
    (
        "src/infrastructure/persistence/fresh_session_identity.rs",
        41,
    ),
    ("src/infrastructure/tools/delegated_roster.rs", 40),
    (
        "src/infrastructure/persistence/session_store_ordinals.rs",
        23,
    ),
    // D8 #1977 adds the wire-admitted user-chat constructor (was 133
    // before D8).
    ("src/domain/session_identity.rs", 148),
    // D9 #1978 moves the ephemeral scrub onto the port (was 376 before D9).
    ("src/infrastructure/persistence/context_spill.rs", 377),
    ("src/infrastructure/persistence/session_layout.rs", 94),
    ("src/infrastructure/persistence/session_ownership.rs", 229),
    // R2-H2: the empty-save delete moved beside the home sidecar it now
    // removes (`session_store_home.rs`); the store is back at 687.
    ("src/infrastructure/persistence/session_store.rs", 685),
    (
        "src/infrastructure/persistence/session_store_catalogue.rs",
        23,
    ),
    // PR #2018 perf: the summary cache moved to its own module, seeded once
    // per process from the persisted index (31 → 29; the walk 62 → 60).
    ("src/infrastructure/persistence/session_store_list.rs", 28),
    // R2-L3: the per-record read (cached summary, header parse, layout
    // check, every skip logged) is its own helper; the walk shrank 110 → 62.
    (
        "src/infrastructure/persistence/session_store_list_scan.rs",
        59,
    ),
    (
        "src/infrastructure/persistence/session_store_list_index.rs",
        48,
    ),
    // PR #2018 perf: the derived index is stamp-based and persisted; its
    // on-disk shape is its own module, both pinned at delivered size.
    // #2010 review (R1-H1): strict validation and the walk's seeding moved
    // to child modules beside the rejection cache (482 → 468). Round 2
    // (R2-H1/H2): rejections are in memory only and never seeded, so every
    // owner shrank (468 → 446, 139 → 110, 83 → 44); the one record read both
    // halves share is its own seam (`session_record_read.rs`).
    (
        "src/infrastructure/persistence/session_home_catalogue.rs",
        446,
    ),
    // Round 3: naming the walk's skips is its own owner (R3-H1), and the
    // legacy `rejected` key's presence rule lives here (R3-H6): 110 as before.
    (
        "src/infrastructure/persistence/session_home_catalogue_rejections.rs",
        110,
    ),
    (
        "src/infrastructure/persistence/session_home_catalogue_skipped.rs",
        34,
    ),
    (
        "src/infrastructure/persistence/session_home_catalogue_seed.rs",
        44,
    ),
    (
        "src/infrastructure/persistence/session_home_catalogue_index.rs",
        142,
    ),
    (
        "src/infrastructure/persistence/session_store_list_record.rs",
        95,
    ),
    ("src/infrastructure/persistence/session_record_read.rs", 48),
    ("src/infrastructure/session_export.rs", 110),
    ("src/infrastructure/session_export_records.rs", 80),
    ("src/interface/cli/agent/run_session.rs", 130),
    ("src/interface/cli/uds_dispatch.rs", 500),
    // D10 #1979 hands the presenters the active session's key (was 190).
    // #2009: the discovery presenter is its own owner; the query extraction
    // shrinks below master (was 187).
    // #2011: one re-export line for the shared safe-display helper (174 → 173).
    ("src/interface/cli/uds_dispatch_query.rs", 173),
    // #2011 review: the safe rendering is its own module (50 → 46). #2010:
    // the row is one shared function (46 → 45).
    ("src/interface/cli/uds_dispatch_discovery.rs", 42),
    // #2010 metadata search: every new owner pinned at its delivered size.
    // Review round 1 only lowered them; what it added lives in new owners
    // (`search_limits`, `session_metadata_text`, `uds_search_numbers`).
    (
        "src/application/sessions/use_cases/search_session_metadata.rs",
        157,
    ),
    (
        "src/application/sessions/dto/search_session_metadata.rs",
        80,
    ),
    ("src/application/sessions/dto/search_limits.rs", 33),
    ("src/domain/session_metadata_text.rs", 51),
    ("src/interface/cli/uds_search_numbers.rs", 62),
    ("src/domain/session_metadata_search.rs", 168),
    (
        "src/infrastructure/persistence/session_home_catalogue_metadata.rs",
        45,
    ),
    // Review round 2: the refusal, the injective path spelling, the rescue of
    // a number no `f64` holds and the bounded freshness tail are new owners.
    ("src/domain/session_path_text.rs", 23),
    ("src/domain/session_query_refusal.rs", 31),
    ("src/interface/cli/protocol_search_rescue.rs", 51),
    ("src/interface/cli/uds_freshness_json.rs", 33),
    ("src/composition/session_search.rs", 23),
    ("src/interface/cli/uds_dispatch_search.rs", 80),
    ("src/interface/cli/uds_discovery_handles.rs", 44),
    (
        "src/interface/uds/sessions/search_session_metadata_controller.rs",
        39,
    ),
    ("src/interface/cli/uds_safe_display.rs", 24),
    // #1848 reasoning-effort injection plus #2009 scoped session dispatch.
    // #2011: the resume answers are the resume presenter's (259 → 246).
    ("src/interface/cli/uds_dispatch_session.rs", 243),
    ("src/interface/cli/uds_latest_report.rs", 85),
    // D9 #1978 hands the loop its retained-context handles as an input
    // (was 305 before D9).
    // #1845 split the single-client loop out of uds_lifecycle (311 → 203);
    // the moved lines are ratcheted at their new home.
    ("src/interface/cli/uds_lifecycle.rs", 193),
    ("src/interface/cli/uds_single_client.rs", 127),
    ("src/interface/cli/uds_multi.rs", 633),
    // Same merge of D3/D4/D5/D6/D7 handles (was 111 before D7); D10 #1979
    // types the loop's identity and reads the key from the active session
    // (was 125).
    ("src/interface/cli/uds_session_handles.rs", 121),
    // D10 #1979: the tracker keeps no session key; the presenters, the
    // workflow-nudge descendant check and the agent loop's key accessor are
    // pinned at their D10 size.
    ("src/interface/cli/uds_session.rs", 628),
    ("src/interface/cli/uds_query.rs", 232),
    ("src/interface/cli/uds_workflow_nudge.rs", 109),
    ("src/application/agent_loop/agent_loop_session.rs", 22),
    ("src/interface/cli/uds_turn_accounting.rs", 40),
    // D8 #1977 adds the workflow restore (was 93 before D8).
    // #1848: the adapter holds the change-reasoning-effort use case and
    // restores the startup effort through it (was 97).
    ("src/interface/cli/uds_session_switch_runtime.rs", 103),
    // #2010 review (R1-H7): the discovery handles' alias is gone (712 → 711).
    ("src/interface/cli/uds.rs", 711),
    ("src/interface/cli/uds_session_history.rs", 205),
    ("src/interface/cli/uds_session_message_range.rs", 290),
    ("src/interface/cli/uds_snapshots.rs", 256),
    ("src/interface/cli/uds_sync.rs", 135),
    ("src/interface/uds/sessions/controller.rs", 42),
    ("src/interface/uds/sessions/export_report_controller.rs", 45),
    ("src/interface/uds/sessions/read_history_controller.rs", 90),
    (
        "src/interface/uds/sessions/recover_message_controller.rs",
        80,
    ),
    (
        "src/interface/uds/sessions/synchronize_transcript_controller.rs",
        85,
    ),
    (
        "src/interface/uds/sessions/rewind_conversation_controller.rs",
        40,
    ),
    ("src/domain/conversation_edit.rs", 100),
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
            production_code(path)
                .iter()
                .any(|(_, line)| re_exports_sessions(line))
        })
        .collect();
    assert!(
        re_exports.is_empty(),
        "no module re-exports the sessions capability: {re_exports:?}"
    );
}

/// A `pub use crate::application::sessions…` line under any visibility
/// (`pub`, `pub(crate)`, `pub(super)`, `pub(in …)`): a re-export of the
/// capability, which no module may offer (F1 of the #1998 review widened
/// this from the bare `pub use` spelling).
fn re_exports_sessions(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("pub") else {
        return false;
    };
    let rest = match rest.strip_prefix('(') {
        Some(vis) => match vis.split_once(')') {
            Some((_, after)) => after,
            None => return false,
        },
        None => rest,
    };
    rest.trim_start()
        .starts_with("use crate::application::sessions")
}

#[test]
fn re_export_needle_sees_every_visibility() {
    for line in [
        "pub use crate::application::sessions::use_cases::ListSessions;",
        "pub(crate) use crate::application::sessions::use_cases::ResumeSavedSession as Hidden;",
        "    pub(super) use crate::application::sessions::dto::HistoryPage;",
        "pub(in crate::interface) use crate::application::sessions::ports::SessionStore;",
    ] {
        assert!(re_exports_sessions(line), "{line}");
    }
    for line in [
        "use crate::application::sessions::use_cases::ListSessions;",
        "pub use crate::application::agent_loop::UsageTotals;",
        "pub(crate) use uds_session_message_range::recovered_content_json;",
    ] {
        assert!(!re_exports_sessions(line), "{line}");
    }
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
                assert!(
                    is_ports_file(path),
                    "port {name} must be declared under the capability's ports, found in {path}"
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
    for (constructor, site) in COMPOSED_CONSTRUCTORS {
        let sites = production_files_calling(constructor);
        assert_eq!(
            sites,
            set(&[site]),
            "{constructor} may appear only in {site}"
        );
    }
    let layout_sites = production_files_calling("FlatSessionLayout::new(");
    assert_eq!(
        layout_sites,
        set(LAYOUT_CREATION_SITES),
        "FlatSessionLayout creation sites changed; the list is decrease-only"
    );
    let export_root_sites = production_files_calling("artifacts/session-exports");
    assert_eq!(
        export_root_sites,
        set(&[EXPORT_ROOT_OWNER]),
        "the export root is spelled by composition only"
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

/// The one persistence adapter that is not the sessions capability's.
const ENVIRONMENT_REGISTRY_STORE: &str =
    "src/infrastructure/persistence/environment_registry_store.rs";

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
            // The environments capability's durable registry (#2024 S4d)
            // is one `<base_dir>/environments.json` document, not a
            // session projection: it names its own file and never a
            // session path.
            .filter(|path| path.as_str() != ENVIRONMENT_REGISTRY_STORE)
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
    for needle in ["join(\"sessions\")", "\"spill.jsonl\"", ".owner\""] {
        assert!(
            !production_code(ENVIRONMENT_REGISTRY_STORE)
                .iter()
                .any(|(_, line)| line.contains(needle)),
            "{ENVIRONMENT_REGISTRY_STORE} forms no session path ({needle})"
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
    // No caller outside persistence forms a session path: any path literal
    // of the layout's vocabulary (a `sessions` segment being joined, an
    // `.owner` stamp, the spill file, the sanitizer) is refused whatever
    // the surrounding expression.
    let outside: Vec<String> = production_files()
        .into_iter()
        .filter(|p| !p.starts_with("src/infrastructure/persistence/"))
        .flat_map(|path| {
            production_code(&path)
                .into_iter()
                .filter(|(_, line)| forms_session_path(line))
                .map(move |(n, line)| format!("{path}:{n}: {}", line.trim()))
        })
        .collect();
    assert!(
        outside.is_empty(),
        "session paths formed outside persistence: {outside:#?}"
    );
}

/// A line that spells part of the flat layout: a `sessions` path segment
/// (`join("sessions")`, `join("sessions/…")`, `"…/sessions/…"`), the
/// ownership stamp suffix, the spill file, or the sanitizer.
fn forms_session_path(line: &str) -> bool {
    let joins_sessions = line.contains("join(\"sessions")
        || line.contains("/sessions/")
        || line.contains("\"sessions/");
    joins_sessions
        || line.contains(".owner\"")
        || line.contains(".owner`")
        || line.contains("spill.jsonl")
        || line.contains("sanitize_session_key(")
}

/// A line that calls `list` on something that is (or is typed by) the
/// session store port: the field, a binding named after it, or a `dyn
/// SessionStore` handle — whatever the alias.
fn lists_a_session_store(line: &str, file_names_store: bool) -> bool {
    if !line.contains(".list(") {
        return false;
    }
    line.contains("session_store")
        || line.contains(".list(None")
        || line.contains("SessionListQuery")
        || (file_names_store && line.contains("store.list("))
}

/// A line that requests the injected save transaction (`x.save_session`
/// followed by a call), as opposed to a handle copy (`.save_session.clone()`)
/// or the field/constructor sites of the handles and composition.
fn requests_a_save(line: &str) -> bool {
    line.contains(".save_session") && !line.contains(".clone()")
}

/// A line that writes through something typed by the session store port:
/// `save`, `save_delta` or `save_clean_delta` on the field, a binding named
/// after it, or a `dyn SessionStore` handle.
fn saves_a_session_store(line: &str, file_names_store: bool) -> bool {
    let writes = line.contains(".save(")
        || line.contains(".save_delta(")
        || line.contains(".save_clean_delta(");
    if !writes {
        return false;
    }
    line.contains("session_store") || (file_names_store && line.contains("store."))
}

/// D5 retirement (#1972): every persistence trigger of the interface goes
/// through the injected `SaveSession`; no interface production line writes
/// the store, and the exact set of save requesters is known.
#[test]
fn interface_never_saves_the_store_directly() {
    let direct: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .flat_map(|path| {
            let code = production_code(&path);
            let names_store = code
                .iter()
                .any(|(_, line)| line.contains("SessionStore") || line.contains("session_store"));
            code.into_iter()
                .filter(move |(_, line)| saves_a_session_store(line, names_store))
                .map(move |(n, line)| format!("{path}:{n}: {}", line.trim()))
        })
        .collect();
    assert!(
        direct.is_empty(),
        "interface writes the store directly in {direct:#?}"
    );
    let requesters: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|path| {
            production_code(path)
                .iter()
                .any(|(_, line)| requests_a_save(line))
        })
        .collect();
    assert_eq!(
        requesters,
        set(SAVE_REQUESTERS),
        "the save transaction's requesters are exact; a new trigger is a reviewed edit"
    );
    for (file, trigger) in SAVE_TRIGGER_SITES {
        assert!(
            production_code(file)
                .iter()
                .any(|(_, line)| line.contains(trigger)),
            "{file} requests `{trigger}`"
        );
    }
}

/// D6 retirement (#1975): the clear and rewind commands are requested from
/// exactly one interface site through the injected handles — the idle
/// dispatch handlers — with the protocol page size handed to the rewind
/// controller; no other interface production line requests either
/// transaction, and no interface file implements the rewind edit or
/// target resolution itself (the forbidden-needle check above).
#[test]
fn clear_and_rewind_are_requested_only_by_the_dispatch_handlers() {
    let handlers = "src/interface/cli/uds_dispatch_session.rs";
    for needle in [
        ".rewrite.clear.clone()",
        ".rewrite.rewind.clone()",
        ".into_request(HISTORY_PAGE_SIZE)",
    ] {
        let sites: BTreeSet<String> = production_files_calling(needle)
            .into_iter()
            .filter(|path| path.starts_with("src/interface/"))
            .collect();
        assert_eq!(sites, set(&[handlers]), "{needle} is the handlers' alone");
    }
    // The accounting adapter is built by the handlers and, for the same
    // reset, by the session-switch adapter (D7 #1976).
    let sites: BTreeSet<String> = production_files_calling("LoopTurnAccounting::new(")
        .into_iter()
        .filter(|path| path.starts_with("src/interface/"))
        .collect();
    assert_eq!(
        sites,
        set(&[handlers, "src/interface/cli/uds_session_switch_runtime.rs"]),
        "LoopTurnAccounting::new( is the handlers' and the switch adapter's"
    );
    let adapters = production_files_calling("impl TurnAccountingReset for");
    assert_eq!(
        adapters,
        set(&[
            "src/interface/cli/uds_turn_accounting.rs",
            "src/interface/cli/uds_session_switch_runtime.rs",
        ]),
        "the production adapters of the accounting-reset port are exact"
    );
}

/// D7/D8 retirement (#1976, #1977): the fresh-session and resume
/// transactions are requested from exactly one interface site — the idle
/// `new_session`/`resume_session` handlers — through the injected handles;
/// the startup open is requested by exactly the two startup paths; the
/// departing-children collaborator is no interface handle at all; the
/// switch runtime and the transition ports have exactly one production
/// adapter each; the raw session key is generated in one infrastructure
/// adapter and the dispatch context carries no raw copy and no store; the
/// retired helpers are gone without a facade.
#[test]
fn fresh_session_is_requested_only_by_the_dispatch_handler() {
    let handlers = "src/interface/cli/uds_dispatch_session.rs";
    let adapter = "src/interface/cli/uds_session_switch_runtime.rs";
    for needle in [
        ".switch.fresh.clone()",
        ".switch.resume.clone()",
        "LoopSessionSwitchRuntime::new(",
        "fleet_settlement_of(",
    ] {
        let sites: BTreeSet<String> = production_files_calling(needle)
            .into_iter()
            .filter(|path| path.starts_with("src/interface/"))
            .collect();
        assert_eq!(sites, set(&[handlers]), "{needle} is the handler's alone");
    }
    // The startup open (#1863): the UDS loop and the one-shot run, and no
    // other interface line claims or loads a session.
    let openers: BTreeSet<String> = production_files_calling(".resume.open_at_startup()")
        .into_iter()
        .filter(|path| path.starts_with("src/interface/"))
        .collect();
    assert_eq!(
        openers,
        set(&[
            "src/interface/cli/agent/run_session.rs",
            "src/interface/cli/uds_lifecycle.rs",
        ]),
        "the startup open is requested by exactly the two startup paths"
    );
    for needle in [".claim(&", ".release(&", "store.load(&"] {
        let sites: BTreeSet<String> = production_files_calling(needle)
            .into_iter()
            .filter(|path| path.starts_with("src/interface/cli/"))
            .collect();
        assert!(
            sites.is_empty(),
            "{needle} survives in the interface's cli tree: {sites:?}"
        );
    }
    assert!(
        production_files_calling(".reset_roster(")
            .into_iter()
            .all(|path| path.starts_with("src/application/")),
        "the roster is replaced by the application's transactions only"
    );
    for (needle, owner) in [
        ("impl SessionSwitchRuntime for", adapter),
        ("impl SessionKeyPropagation for", adapter),
        (
            "impl FreshSessionIdentityGenerator for",
            "src/infrastructure/persistence/fresh_session_identity.rs",
        ),
        (
            "impl DelegatedChildrenRoster for",
            "src/infrastructure/tools/delegated_roster.rs",
        ),
        (
            "impl FleetSettlement for",
            "src/composition/fleet_settlement.rs",
        ),
        (
            "SessionIdentity::fresh_chat(",
            "src/infrastructure/persistence/fresh_session_identity.rs",
        ),
    ] {
        assert_eq!(
            production_files_calling(needle),
            set(&[owner]),
            "{needle} has exactly one production owner"
        );
    }
    // The loop's raw-key holders adopt an identity only through the
    // propagation adapter (and the startup identity of the loop).
    let propagators: BTreeSet<String> = production_files_calling(".set_session_key(")
        .into_iter()
        .filter(|path| path.starts_with("src/interface/"))
        .collect();
    assert_eq!(
        propagators,
        set(&[adapter, "src/interface/cli/agent.rs"]),
        "the raw session key is propagated by the switch adapter and the startup path only"
    );
    // The typed identity is the agent loop's only session-key input.
    let loop_session =
        std::fs::read_to_string("src/application/agent_loop/agent_loop_session.rs").unwrap();
    assert!(loop_session.contains("pub fn set_session_key(&mut self, identity: SessionIdentity)"));
    // Retired without a facade.
    assert!(!Path::new("src/interface/cli/agent_session_identity.rs").exists());
    assert!(!Path::new("src/interface/cli/uds/uds_session_load.rs").exists());
    for retired in [
        "generate_chat_key",
        "generate_chat_identity",
        "resolve_uds_session_key",
        "reset_to_with_spill_store",
        "settle_departing_children",
        "reset_subagent_roster",
        "note_persisted_roster_is_history",
        "set_workflow_run",
        "sync_message_count",
        "load_session(",
    ] {
        assert!(
            production_files_calling(retired).is_empty(),
            "{retired} survives in production code"
        );
    }
    // The identity switch is the active session's own step, private to it.
    let state = std::fs::read_to_string("src/application/sessions/active_session.rs").unwrap();
    assert!(state.contains("pub(in crate::application::sessions) fn switch_to("));
    assert!(!state.contains("pub fn switch_to("));
    assert!(!state.contains("pub fn switch_identity("));
}

#[test]
fn interface_never_lists_the_store_directly() {
    let direct: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .flat_map(|path| {
            let code = production_code(&path);
            let names_store = code
                .iter()
                .any(|(_, line)| line.contains("SessionStore") || line.contains("session_store"));
            code.into_iter()
                .filter(move |(_, line)| lists_a_session_store(line, names_store))
                .map(move |(n, line)| format!("{path}:{n}: {}", line.trim()))
        })
        .collect();
    assert!(
        direct.is_empty(),
        "interface lists the store directly in {direct:#?}"
    );
    let query = std::fs::read_to_string("src/interface/cli/uds_dispatch_query.rs").unwrap();
    assert!(
        query.contains("handle_list_sessions(ctx, id, tn, *scope)")
            && std::fs::read_to_string("src/interface/cli/uds_dispatch_session.rs")
                .unwrap()
                .contains("ctx.discovery.list(requested).await"),
        "the list_sessions command is answered through the composed controller"
    );
}

/// R7a/R9: the interface declares no conversation read model of its own and
/// re-implements none of the history/recovery policy; no raw-key
/// conversion survives in the interface (D8 #1977).
#[test]
fn interface_owns_no_conversation_state_or_history_policy() {
    let interface: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/interface/"))
        .collect();
    for needle in INTERFACE_FORBIDDEN_NEEDLES {
        let owners: BTreeSet<String> = interface
            .iter()
            .filter(|path| {
                production_code(path)
                    .iter()
                    .any(|(_, line)| line.contains(needle))
            })
            .cloned()
            .collect();
        let allowed = if *needle == "SessionIdentity::from_persisted_key(" {
            set(RAW_KEY_CONVERSION_SITES)
        } else {
            BTreeSet::new()
        };
        assert_eq!(
            owners, allowed,
            "{needle} declared or spelled in the interface"
        );
    }
    let application_ledger = production_files_calling("struct ConversationLedger");
    assert_eq!(
        application_ledger,
        set(&["src/application/sessions/conversation_ledger.rs"]),
        "one conversation ledger, owned by the application"
    );
    let state_owner = production_files_calling("struct ActiveSessionState");
    assert_eq!(
        state_owner,
        set(&["src/application/sessions/active_session.rs"]),
        "one active-session state, owned by the application"
    );
}

/// D3 (#1973): both transports answer `sync` through the composed
/// controller and one presenter. The idle dispatch and the reader fast
/// path name the sync wire module; that module invokes the controller;
/// and the sync frame's vocabulary (`resync`, `caughtUp`, `nextRev`) is
/// spelled by exactly one interface production file.
#[test]
fn sync_is_answered_through_the_composed_controller_on_both_transports() {
    let idle = std::fs::read_to_string("src/interface/cli/uds_dispatch_query.rs").unwrap();
    assert!(
        idle.contains("uds_sync::sync_data("),
        "the idle loop presents sync through the shared presenter"
    );
    let reader = std::fs::read_to_string("src/interface/cli/uds_reader_dispatch.rs").unwrap();
    assert!(
        reader.contains("uds_sync::intercept("),
        "the reader fast path is the sync wire module's"
    );
    let wire = std::fs::read_to_string("src/interface/cli/uds_sync.rs").unwrap();
    assert!(
        wire.contains(".sync(epoch, since_rev, HISTORY_PAGE_SIZE, carry)"),
        "the presenter invokes the composed controller with the protocol page size"
    );
    for field in ["\"resync\"", "\"caughtUp\"", "\"nextRev\""] {
        let presenters: BTreeSet<String> = production_files_calling(field)
            .into_iter()
            .filter(|path| path.starts_with("src/interface/"))
            .collect();
        assert_eq!(
            presenters,
            set(&["src/interface/cli/uds_sync.rs"]),
            "{field} is presented by the sync wire module alone"
        );
    }
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

/// The production files that hold the retention port (D9 #1978): the
/// sessions capability that declares and consumes it, the composition
/// that builds the store and the graph, the persistence adapter that
/// implements it, and the interface handle structs the loop is composed
/// over. Exact: no pruning-policy file, no tool, no dispatch handler.
const RETENTION_PORT_HOLDERS: &[&str] = &[
    "src/application/sessions/active_session.rs",
    "src/application/sessions/conversation_ledger.rs",
    "src/application/sessions/ports.rs",
    "src/application/sessions/use_cases/clear_conversation.rs",
    "src/application/sessions/use_cases/export_session_report.rs",
    "src/application/sessions/use_cases/recall_context.rs",
    "src/application/sessions/use_cases/retain_context.rs",
    "src/composition/retention.rs",
    "src/composition/sessions.rs",
    "src/infrastructure/persistence/context_spill.rs",
    "src/interface/cli/retention_handles.rs",
    "src/interface/cli/uds_session_handles.rs",
];

/// The pruning-policy files (D9 #1978): they decide when and what to
/// retain and consume the narrow writer/reader handles. Exact and
/// non-empty; they never name the port or call a store method.
const PRUNING_POLICY_OWNERS: &[&str] = &[
    "src/application/agent_loop.rs",
    "src/application/context.rs",
    "src/application/context_pruning.rs",
    "src/application/context_pruning_messages.rs",
];

/// The spill-id allocators (D9 #1978): the tool-result grammar
/// `turn{n}:{tool}:{idx}` and the conversation grammar `turn{n}:msg:{role}`,
/// each spelled by exactly one policy file; sessions allocates no id.
const SPILL_ID_ALLOCATORS: &[(&str, &str)] = &[
    (
        "src/application/agent_loop_tool_exec.rs",
        "format!(\"turn{}:{}:{}\", current_turn, tc.name, idx)",
    ),
    (
        "src/application/context_pruning_messages.rs",
        "format!(\"turn{}:msg:{role}\", msg.turn.unwrap_or(0))",
    ),
];

/// A retention store method reached directly (append, recall, index,
/// presence, clear-by-identity, scrub), whatever the binding is called.
fn reaches_a_store_method(line: &str) -> bool {
    [
        ".append(",
        ".list_entries(",
        ".has_entries(",
        ".scrub_sync(",
        "store.recall(",
        "store.clear(",
    ]
    .iter()
    .any(|needle| line.contains(needle))
}

/// D9 (#1978): exactly one owner per retained-context role — the sessions
/// capability for the port, the identity-keyed recall/list/clear selection
/// and the id append/deduplication; the pruning policy for when and what
/// to retain, over the narrow handles; persistence for the file and the
/// ephemeral scrub; the recall tool for schema, formatting and diagnostics
/// only; composition for construction. Each inventory is asserted exact
/// and non-empty.
#[test]
fn retained_context_has_exactly_one_owner_per_role() {
    // The port is declared once, in the capability's ports file, and held
    // by exactly the listed files.
    assert_eq!(
        production_files_calling("pub trait ContextSpillStore"),
        set(&["src/application/sessions/ports.rs"])
    );
    assert!(!RETENTION_PORT_HOLDERS.is_empty());
    assert_eq!(
        production_files_calling("ContextSpillStore"),
        set(RETENTION_PORT_HOLDERS),
        "the retention port's holders are exact"
    );
    // The pruning policy consumes the narrow handles and nothing else.
    assert!(!PRUNING_POLICY_OWNERS.is_empty());
    let handle_consumers: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|p| {
            p.starts_with("src/application/") && !p.starts_with("src/application/sessions/")
        })
        .filter(|p| {
            production_code(p).iter().any(|(_, line)| {
                line.contains("RetainContext")
                    || line.contains("ListRetainedContext")
                    || line.contains("ContextRetention")
            })
        })
        .collect();
    assert_eq!(
        handle_consumers,
        set(PRUNING_POLICY_OWNERS),
        "the pruning policy's owners are exact"
    );
    for owner in PRUNING_POLICY_OWNERS {
        let direct: Vec<String> = production_code(owner)
            .into_iter()
            .filter(|(_, line)| reaches_a_store_method(line) || line.contains("ContextSpillStore"))
            .map(|(n, line)| format!("{owner}:{n}: {}", line.trim()))
            .collect();
        assert!(
            direct.is_empty(),
            "the pruning policy reaches the retention store directly: {direct:#?}"
        );
    }
    // The pruning decisions stay outside sessions: no threshold, ladder,
    // exemption or manifest policy joins the capability.
    let sessions_files: Vec<String> = production_files()
        .into_iter()
        .filter(|p| p.starts_with("src/application/sessions/"))
        .collect();
    for needle in [
        "fn collapse_tool_results_over_limit(",
        "fn collapse_conversation_messages_over_limit(",
        "fn enforce_context_ceiling_ladder(",
        "fn exempt_flags(",
        "fn update_spill_manifest(",
        "fn estimate_tokens(",
        "pin_recent_turns",
        "context_collapse_after",
        "turn{",
    ] {
        let owners: BTreeSet<String> = sessions_files
            .iter()
            .filter(|p| {
                production_code(p)
                    .iter()
                    .any(|(_, line)| line.contains(needle))
            })
            .cloned()
            .collect();
        assert!(
            owners.is_empty(),
            "pruning policy `{needle}` moved into sessions: {owners:?}"
        );
    }
    // The ids are allocated by the policy, each grammar in one file.
    for (file, grammar) in SPILL_ID_ALLOCATORS {
        assert_eq!(
            production_files_calling(grammar),
            set(&[file]),
            "spill-id grammar `{grammar}` has exactly one allocator"
        );
    }
    // Sessions appends and deduplicates: the suffix rule is the writer's.
    assert_eq!(
        production_files_calling("fn highest_suffix_taken("),
        set(&["src/application/sessions/use_cases/retain_context.rs"])
    );
    // Persistence implements the port once; the recall tool adapts the use
    // case, holds no store and selects nothing (the reserved index id and
    // the empty-id refusal are the DTO's).
    assert_eq!(
        production_files_calling("impl ContextSpillStore for"),
        set(&["src/infrastructure/persistence/context_spill.rs"])
    );
    let recall_tool = "src/infrastructure/tools/recall.rs";
    assert_eq!(
        production_files_calling("impl Tool for RecallTool"),
        set(&[recall_tool])
    );
    let tool_code = production_code(recall_tool);
    assert!(
        tool_code
            .iter()
            .any(|(_, line)| line.contains("RecallQuery::parse(")),
        "the recall tool asks the DTO what an id selects"
    );
    for needle in [
        "== \"list\"",
        "ContextSpillStore",
        "list_entries",
        "SpillId::new(",
    ] {
        assert!(
            !tool_code.iter().any(|(_, line)| line.contains(needle)),
            "the recall tool selects nothing itself: `{needle}` found"
        );
    }
    assert_eq!(
        production_files_calling("pub const INDEX_QUERY"),
        set(&["src/application/sessions/dto/retained_context.rs"])
    );
    // The ephemeral scrub is requested by exactly the two run exits, over
    // the composed handle; the interface's own scrub helper is gone.
    assert_eq!(
        production_files_calling(".scrub_ephemeral("),
        set(&[
            "src/interface/cli/agent.rs",
            "src/interface/cli/agent/run_session.rs",
        ])
    );
    assert!(
        !production_code("src/interface/shared.rs")
            .iter()
            .any(|(_, line)| line.contains("spill")),
        "interface/shared.rs no longer scrubs or names the spill file"
    );
    // Composition alone builds the graph and hands the builder to `main`.
    let main = std::fs::read_to_string("src/main.rs").unwrap();
    assert!(main.contains("retention: quecto::composition::sessions::build_retention_handles"));
    assert_eq!(
        production_files_calling("fn build_retention_handles("),
        set(&["src/composition/sessions.rs"])
    );
}
