//! Every board refusal text maps to exactly one kind (#2303, review L1):
//! a table of each `BoardError::new(kind, text)` site the production
//! sources hold, keyed by file, function, text and kind (round-2 review
//! L5), names the one [`RefusalKind`] telemetry records for it. A refusal
//! whose kind changes (to a wrong one, or to a catch-all such as
//! `Internal`), any new construction site (even one repeating a text and
//! kind already in the table), a text raised under two kinds, and a table
//! row no source builds any more all fail here, so each kind is chosen and
//! reviewed against its text, where it is raised, once.
//!
//! A text is the message's string literal, or the template of its
//! `format!`; a message built some other way (a store error's own text, a
//! codec's) is its expression, prefixed `expr:`. A kind is the
//! `RefusalKind` variant named, or, for one chosen at run time, the
//! expression that chooses it, prefixed `expr:`; those choosers are tested
//! where they are defined (`policy.rs`'s budget kind,
//! `repository.rs`'s store kinds).
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use self::scan::built;
use super::dependency_scan;

/// `(file:function, text, kind)` for every construction site in the
/// production sources, one row per site.
pub(super) const REFUSALS: &[(&str, &str, &str)] = &[
    (
        "src/application/swarm/board_control.rs:current",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/board_control.rs:edited",
        "the board's {record} is not as the board writes it",
        "Store",
    ),
    (
        "src/application/swarm/board_control.rs:pause_started",
        "paused run has no pause record",
        "Internal",
    ),
    (
        "src/application/swarm/board_operation.rs:atomic",
        "coordination store committed without running its work",
        "Internal",
    ),
    (
        "src/application/swarm/board_operation.rs:atomic",
        "coordination store ran a transaction's work twice",
        "Internal",
    ),
    (
        "src/application/swarm/board_operation.rs:operation",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/board_recovery.rs:python_str",
        "a task id that finds a task is text or a number",
        "Internal",
    ),
    (
        "src/application/swarm/board_tasks.rs:owned",
        "stale or unowned claim",
        "StaleToken",
    ),
    (
        "src/application/swarm/board_tasks.rs:read_task",
        "unknown task",
        "NotFound",
    ),
    (
        "src/application/swarm/use_cases/activate_member.rs:activate",
        "member already active in a different process",
        "LaunchConflict",
    ),
    (
        "src/application/swarm/use_cases/activate_member.rs:activate",
        "run stopped before activation",
        "NotRunning",
    ),
    (
        "src/application/swarm/use_cases/activate_member.rs:activate",
        "unknown or stale launch reservation",
        "StaleToken",
    ),
    (
        "src/application/swarm/use_cases/admit_member.rs:admit",
        "coordination store lost the member it admitted",
        "Internal",
    ),
    (
        "src/application/swarm/use_cases/amend_run_contract.rs:AmendRunContract::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/claim_task.rs:ClaimTask::execute",
        "task is not ready to claim",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/claim_task.rs:ClaimTask::execute",
        "unmet dependencies",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/close_run.rs:CloseRun::execute",
        "run is {} without a proposed outcome; resume it or cancel the run",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/complete_run.rs:CompleteRun::execute",
        "completion accepted a revision that is not text",
        "Internal",
    ),
    (
        "src/application/swarm/use_cases/configure_usage_budget.rs:ConfigureUsageBudget::execute",
        "expr: BUDGET_ARGUMENTS",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/create_run.rs:CreateRun::deadline",
        "deadline must be in the next seven days",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/create_run.rs:CreateRun::validated",
        "constraints must be a list of strings",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/create_run.rs:CreateRun::validated",
        "member limit must be 1 through 25 including coordinator",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/create_run.rs:take_over_setup",
        "existing live/reserved members exceed requested limit; terminate and reconcile first",
        "MemberLimit",
    ),
    (
        "src/application/swarm/use_cases/create_run.rs:take_over_setup",
        "only the setup coordinator can create this run; existing runs cannot be reset",
        "RunExists",
    ),
    (
        "src/application/swarm/use_cases/create_task.rs:CreateTask::create",
        "task board full ({TASK_BOARD_CAPACITY}); settle existing work",
        "CapacityFull",
    ),
    (
        "src/application/swarm/use_cases/create_task.rs:acceptance",
        "task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/extend_run_deadline.rs:ExtendRunDeadline::execute",
        "deadline may be at most seven days ahead, as at creation",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/extend_run_deadline.rs:ExtendRunDeadline::execute",
        "run is {}; nothing to extend",
        "NotRunning",
    ),
    (
        "src/application/swarm/use_cases/join_run.rs:join",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/join_run.rs:join",
        "invoking member is unknown or death confirmed",
        "NotMember",
    ),
    (
        "src/application/swarm/use_cases/join_run.rs:join",
        "launch reservation does not match invoking process",
        "LaunchConflict",
    ),
    (
        "src/application/swarm/use_cases/list_file_owners.rs:ListFileOwners::execute",
        "file page requires nonnegative offset and limit 1 through 100",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "evidence must match a configured criterion and kind",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_evidence.rs:RecordEvidence::execute",
        "evidence must match a configured criterion and kind",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_member_launch.rs:RecordMemberLaunch::execute",
        "conflicting launch identity",
        "LaunchConflict",
    ),
    (
        "src/application/swarm/use_cases/record_member_launch.rs:RecordMemberLaunch::execute",
        "stale launch reservation",
        "StaleToken",
    ),
    (
        "src/application/swarm/use_cases/record_request_usage.rs:RecordRequestUsage::execute",
        "invalid request observation",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_request_usage.rs:RecordRequestUsage::execute",
        "request diagnostic exceeds {MAX_REQUEST_PAYLOAD_BYTES} bytes",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/record_request_usage.rs:RecordRequestUsage::execute",
        "request diagnostic ledger full; export before starting another run",
        "CapacityFull",
    ),
    (
        "src/application/swarm/use_cases/record_request_usage.rs:RecordRequestUsage::execute",
        "request observation ID reused with different data",
        "RequestIdReused",
    ),
    (
        "src/application/swarm/use_cases/recover_task.rs:RecoverTask::execute",
        "only abandoned active work can be recovered",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/recover_task.rs:RecoverTask::execute",
        "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/recover_task.rs:RecoverTask::execute",
        "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/release_unlaunched_member.rs:ReleaseUnlaunchedMember::execute",
        "only an unlaunched reservation may be released",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/reserve_files.rs:ReserveFiles::execute",
        "file already reserved: {path}; acquire the entire set or release and retry",
        "ReservedByOther",
    ),
    (
        "src/application/swarm/use_cases/reserve_files.rs:ReserveFiles::execute",
        "file reservation board full (1000); release settled work",
        "CapacityFull",
    ),
    (
        "src/application/swarm/use_cases/reserve_files.rs:ReserveFiles::normalized",
        "reserve 1 through 100 paths together",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/resume_run.rs:ResumeRun::execute",
        "a paused run is resumed only by the supervisor outside the swarm (agent_cmd swarm_control resume); members cannot resume it",
        "SupervisorOnly",
    ),
    (
        "src/application/swarm/use_cases/resume_run_externally.rs:ResumeRunExternally::execute",
        "only a paused run may resume",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/resume_run_externally.rs:ResumeRunExternally::execute",
        "resume would pause again at once: {}",
        "BudgetExhausted",
    ),
    (
        "src/application/swarm/use_cases/revalidate_task.rs:RevalidateTask::execute",
        "the board's task row has no evidence",
        "Store",
    ),
    (
        "src/application/swarm/use_cases/revalidate_task.rs:RevalidateTask::execute",
        "unknown task",
        "NotFound",
    ),
    (
        "src/application/swarm/use_cases/revoke_task.rs:RevokeTask::execute",
        "only claimed, blocked or submitted work can be revoked",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/set_task_dependencies.rs:SetTaskDependencies::execute",
        "dependencies may change only before claiming",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/stop_run.rs:StopRun::cancel",
        "run not created yet; nothing to cancel. To start one: swarm op=create",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/stop_run.rs:StopRun::execute",
        "invalid non-success outcome",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/submit_task.rs:evidence",
        "artifact and revision evidence required",
        "Invalid",
    ),
    (
        "src/application/swarm/use_cases/unblock_task.rs:UnblockTask::execute",
        "only blocked or claimed work may resume",
        "WrongState",
    ),
    (
        "src/application/swarm/use_cases/verify_task.rs:VerifyTask::execute",
        "stale claim or work not submitted",
        "StaleToken",
    ),
    (
        "src/application/swarm/use_cases/verify_task.rs:current_revision",
        "stale evidence revision",
        "StaleRevision",
    ),
    (
        "src/application/swarm/use_cases/verify_task.rs:current_revision",
        "stored evidence is not a list of revisioned entries",
        "Store",
    ),
    (
        "src/domain/swarm/dependencies.rs:acyclic",
        "cyclic dependencies",
        "DependencyCycle",
    ),
    (
        "src/domain/swarm/dependencies.rs:dependency_list",
        "dependencies must be a bounded list",
        "Invalid",
    ),
    (
        "src/domain/swarm/dependencies.rs:validate_dependencies",
        "invalid, missing or self dependencies",
        "Invalid",
    ),
    (
        "src/domain/swarm/notification.rs:notification_targets",
        "expr: unorderable",
        "Store",
    ),
    (
        "src/domain/swarm/notification.rs:unhashable",
        "cannot use '{kind}' as {role} (unhashable type: '{kind}')",
        "Store",
    ),
    (
        "src/domain/swarm/policy.rs:admission",
        "member identity already used; choose a stable new identity",
        "IdentityTaken",
    ),
    (
        "src/domain/swarm/policy.rs:admission",
        "run is {}; no new admission",
        "expr: not_running (budget_spent (run) || expired (run , now))",
    ),
    (
        "src/domain/swarm/policy.rs:admission",
        "swarm limit {}, current usage {usage}; reuse the existing pool",
        "MemberLimit",
    ),
    (
        "src/domain/swarm/policy.rs:authorize",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/domain/swarm/policy.rs:authorize",
        "invoking member is unknown or death confirmed",
        "NotMember",
    ),
    (
        "src/domain/swarm/policy.rs:authorize",
        "only the designated coordinator may do this",
        "NotCoordinator",
    ),
    (
        "src/domain/swarm/policy.rs:authorize",
        "run is {}; no new work permitted",
        "expr: not_running (budget_spent (run))",
    ),
    (
        "src/domain/swarm/policy.rs:completion",
        "completion requires accepted evidence at the current revision for every criterion",
        "CompletionUnmet",
    ),
    (
        "src/domain/swarm/policy.rs:completion",
        "settle outstanding work and file reservations before success",
        "CompletionUnmet",
    ),
    (
        "src/domain/swarm/policy.rs:completion",
        "task evidence refers to stale revision",
        "StaleRevision",
    ),
    (
        "src/domain/swarm/policy.rs:require_budget",
        "run is paused (budget-exhausted: deadline); no new work permitted",
        "BudgetExhausted",
    ),
    (
        "src/domain/swarm/policy.rs:require_unsubmitted",
        "submitted evidence is immutable; release and reclaim before revising",
        "Immutable",
    ),
    (
        "src/domain/swarm/policy.rs:require_unsubmitted",
        "task status '{unknown}' is not a known status; no revision permitted",
        "WrongState",
    ),
    (
        "src/domain/swarm/policy.rs:revalidation",
        "new artifact and revision evidence required",
        "Invalid",
    ),
    (
        "src/domain/swarm/policy.rs:revalidation",
        "new artifact evidence must match the revalidated revision",
        "Invalid",
    ),
    (
        "src/domain/swarm/policy.rs:revalidation",
        "only completed tasks may be revalidated",
        "WrongState",
    ),
    (
        "src/domain/swarm/policy.rs:run_already",
        "run already {}",
        "expr: not_running (budget_spent (run))",
    ),
    (
        "src/domain/swarm/policy.rs:run_already_held",
        "run already {}; only the supervisor can resume or close it, and op=cancel_run cancels it",
        "expr: not_running (budget_spent (run))",
    ),
    (
        "src/domain/swarm/policy.rs:validate_extension",
        "deadline extension must be 1..604800 seconds",
        "Invalid",
    ),
    (
        "src/domain/swarm/usage.rs:request_measurement",
        "invalid request observation",
        "Invalid",
    ),
    (
        "src/domain/swarm/usage.rs:request_measurement",
        "invalid request usage {field}",
        "Invalid",
    ),
    (
        "src/domain/swarm/validation.rs:completion_revision",
        "completion revision required",
        "Invalid",
    ),
    (
        "src/domain/swarm/validation.rs:criteria",
        "duplicate criterion id",
        "Invalid",
    ),
    (
        "src/domain/swarm/validation.rs:criteria",
        "explicit evidence criteria required",
        "Invalid",
    ),
    (
        "src/domain/swarm/validation.rs:criterion",
        "criteria distinguish command checks from parent-reviewed requirements",
        "Invalid",
    ),
    (
        "src/domain/swarm/validation.rs:edited_criteria",
        "the board's run criteria is not as the board writes it",
        "Store",
    ),
    (
        "src/domain/swarm/validation.rs:too_long",
        "{label} must be nonempty and at most {maximum} bytes",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/encoding.rs:PyJsonEncoding::encode",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:Stored::loads",
        "expr: undecodable . to_string ()",
        "Store",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:Stored::loads",
        "the JSON object must be str, bytes or bytearray, not {kind}",
        "Store",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:Stored::read",
        "{CONTENDED}: {}",
        "Store",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:bounded_request",
        "request id must be nonempty and at most {REQUEST_ID_MAX_BYTES} bytes",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:encoded",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:replayed",
        "request id reused with different payload",
        "RequestIdReused",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:retry",
        "coordination request ledger full ({REQUEST_LEDGER_CAPACITY})",
        "CapacityFull",
    ),
    (
        "src/infrastructure/persistence/swarm_board/ledger.rs:utf8_json",
        "the stored result is not UTF-8 JSON",
        "Store",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository.rs:SqliteBoard::event",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository.rs:atomic_on",
        "expr: refusal . 0",
        "expr: store_kind (failure)",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository.rs:encoded",
        "expr: error",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository.rs:failed",
        "expr: contended (& error) . 0",
        "expr: kind",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository.rs:loose",
        "{CONTENDED}: Error binding parameter {position}: {error}",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_control.rs:lost_among",
        "the loss scan read an event it did not select",
        "Internal",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_evidence.rs:SqliteBoard::completion_state",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_evidence.rs:SqliteBoard::replace_task_evidence",
        "expr: error",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_reservations.rs:SqliteBoard::bound_count",
        "a count answered no row",
        "Internal",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_tasks.rs:SqliteBoard::retry",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_tasks.rs:SqliteBoard::retry",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_tasks.rs:SqliteBoard::retry",
        "expr: stored . to_string ()",
        "Store",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_usage.rs:SqliteBoard::configure_usage_budget",
        "expr: error . to_string ()",
        "Invalid",
    ),
    (
        "src/infrastructure/persistence/swarm_board/repository_usage.rs:SqliteBoard::insert_request_usage",
        "count beyond i64: {value}",
        "Invalid",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:call_as",
        "swarm board has no method {method}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_binding.rs:bind",
        "{name}: arguments must be a JSON array or object",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_binding.rs:bind",
        "{name}: missing required argument {}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_binding.rs:bind",
        "{name}: takes {} arguments, {} given",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_binding.rs:bind",
        "{name}: unexpected argument {key}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_usage.rs:request_admission",
        "_request_admission: gate must be one of model, retry, tool",
        "Invalid",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch_render.rs:float",
        "the board holds a non-finite number: {value}",
        "Store",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:take",
        "swarm board bound the wrong number of arguments",
        "Internal",
    ),
    (
        "src/infrastructure/workspace/checkout_paths.rs:ResolvedCheckout::normalize",
        "file must resolve inside the shared checkout",
        "Invalid",
    ),
];

/// The files a refusal table entry may come from: production sources only
/// (a test builds refusals of any kind it likes). An allowlist (#2303
/// round-3 review L5): a file is production only when the crate's module
/// tree mounts it outside `#[cfg(test)]`, whatever its name.
fn production(path: &str) -> bool {
    dependency_scan::production_file(path)
}

fn sources(dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = std::fs::read_to_string(&path).expect("read file");
            files.push((path.display().to_string(), content));
        }
    }
}

/// Every construction site in the production sources, as
/// `(file:function, text, kind)`, sorted.
fn sites_in_sources() -> Vec<(String, String, String)> {
    let mut files = Vec::new();
    sources(Path::new("src"), &mut files);
    let mut sites = Vec::new();
    for (path, content) in files.iter().filter(|(path, _)| production(path)) {
        for (function, text, kind) in built(content) {
            assert!(!kind.is_empty(), "{path}: a refusal without a kind");
            sites.push((format!("{path}:{function}"), text, kind));
        }
    }
    sites.sort();
    sites
}

#[test]
fn every_board_refusal_site_is_in_the_table_with_one_kind() {
    let found = sites_in_sources();
    assert!(found.len() > 30, "the scan reads the board's refusals");
    let mut kinds: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (_, text, kind) in &found {
        kinds.entry(text).or_default().insert(kind);
    }
    let two_kinds: Vec<_> = kinds.iter().filter(|(_, kinds)| kinds.len() > 1).collect();
    assert!(
        two_kinds.is_empty(),
        "a refusal text raised under two kinds: {two_kinds:?}"
    );
    let mut table: Vec<(String, String, String)> = refusals()
        .map(|(site, text, kind)| ((*site).to_owned(), (*text).to_owned(), (*kind).to_owned()))
        .collect();
    table.sort();
    let rows: Vec<String> = found
        .iter()
        .map(|(site, text, kind)| format!("    ({site:?}, {text:?}, {kind:?}),"))
        .collect();
    assert_eq!(
        found,
        table,
        "the refusal table is out of date (one row per construction site); the sources build:\n{}",
        rows.join("\n")
    );
}

/// Every row of the site table: [`REFUSALS`] and the rows kept beside it
/// ([`messages::MESSAGE_REFUSALS`], #2276).
pub(super) fn refusals() -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)>
{
    REFUSALS.iter().chain(messages::MESSAGE_REFUSALS)
}

/// The scan of a source's construction sites and its own tests, beside
/// the table (#2273: the table's file stays within 750 lines).
#[path = "swarm_board_refusal_kinds_scan.rs"]
mod scan;

/// The durable messages' rows of the table (#2276), beside it.
#[path = "swarm_board_refusal_kinds_messages.rs"]
mod messages;
