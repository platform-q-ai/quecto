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

use quote::ToTokens;

use super::dependency_scan;
use syn::visit::Visit;

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
        "src/application/swarm/use_cases/join_run.rs:JoinRun::execute",
        "coordination run missing",
        "RunMissing",
    ),
    (
        "src/application/swarm/use_cases/join_run.rs:JoinRun::execute",
        "invoking member is unknown or death confirmed",
        "NotMember",
    ),
    (
        "src/application/swarm/use_cases/join_run.rs:JoinRun::execute",
        "launch reservation does not match invoking process",
        "LaunchConflict",
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
        "src/application/swarm/use_cases/release_unlaunched_member.rs:ReleaseUnlaunchedMember::execute",
        "only an unlaunched reservation may be released",
        "WrongState",
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
        "completion revision required",
        "Invalid",
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
        "src/infrastructure/persistence/swarm_board/repository_control.rs:lost_members",
        "the loss scan read an event it did not select",
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
        "src/infrastructure/tools/swarm_board_dispatch.rs:bind",
        "{name}: arguments must be a JSON array or object",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:bind",
        "{name}: missing required argument {}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:bind",
        "{name}: takes {} arguments, {} given",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:bind",
        "{name}: unexpected argument {key}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:call",
        "swarm board has no method {method}",
        "Calling",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:float",
        "the board holds a non-finite number: {value}",
        "Store",
    ),
    (
        "src/infrastructure/tools/swarm_board_dispatch.rs:take",
        "swarm board bound the wrong number of arguments",
        "Internal",
    ),
];

/// The files a refusal table entry may come from: production sources only
/// (a test builds refusals of any kind it likes). An allowlist (#2303
/// round-3 review L5): a file is production only when the crate's module
/// tree mounts it outside `#[cfg(test)]`, whatever its name.
fn production(path: &str) -> bool {
    dependency_scan::production_file(path)
}

/// One `BoardError::new(kind, text)` site: the function it is in
/// (`Type::method` inside an impl), its text and its kind.
type Site = (String, String, String);

/// Every `BoardError::new(kind, text)` call, with the function it is in.
#[derive(Default)]
struct Built {
    sites: Vec<Site>,
    /// The impl's self type and the functions entered, innermost last.
    impls: Vec<String>,
    functions: Vec<String>,
}

impl Built {
    fn function(&self) -> String {
        let function = self.functions.last().map_or("<item>", String::as_str);
        match self.impls.last() {
            Some(owner) if self.functions.len() == 1 => format!("{owner}::{function}"),
            _ => function.to_owned(),
        }
    }
}

fn names_constructor(path: &syn::Path) -> bool {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segments.ends_with(&["BoardError".to_owned(), "new".to_owned()])
}

fn tokens(expr: &syn::Expr) -> String {
    tokens_of(expr)
}

fn kind_of(expr: &syn::Expr) -> String {
    match expr {
        syn::Expr::Path(path)
            if path.path.segments.len() == 2 && path.path.segments[0].ident == "RefusalKind" =>
        {
            path.path.segments[1].ident.to_string()
        }
        other => format!("expr: {}", tokens(other)),
    }
}

fn text_of(expr: &syn::Expr) -> String {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => text.value(),
        syn::Expr::Macro(call) if call.mac.path.is_ident("format") => {
            let template = call
                .mac
                .parse_body_with(
                    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
                )
                .ok()
                .and_then(|arguments| arguments.first().cloned());
            match template {
                Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                })) => text.value(),
                _ => format!("expr: {}", tokens(expr)),
            }
        }
        other => format!("expr: {}", tokens(other)),
    }
}

impl<'ast> Visit<'ast> for Built {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let owner = match &*item.self_ty {
            syn::Type::Path(path) => path
                .path
                .segments
                .last()
                .map_or_else(|| tokens_of(&item.self_ty), |s| s.ident.to_string()),
            other => tokens_of(other),
        };
        self.impls.push(owner);
        syn::visit::visit_item_impl(self, item);
        self.impls.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.functions.push(item.sig.ident.to_string());
        let impls = std::mem::take(&mut self.impls);
        syn::visit::visit_item_fn(self, item);
        self.impls = impls;
        self.functions.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.functions.push(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.functions.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = &*call.func {
            if names_constructor(&function.path) {
                let arguments: Vec<&syn::Expr> = call.args.iter().collect();
                assert_eq!(arguments.len(), 2, "BoardError::new(kind, text)");
                let site = (
                    self.function(),
                    text_of(arguments[1]),
                    kind_of(arguments[0]),
                );
                self.sites.push(site);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn tokens_of(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn built(source: &str) -> Vec<Site> {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = Built::default();
    found.visit_file(&file);
    found.sites
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
    let mut table: Vec<(String, String, String)> = REFUSALS
        .iter()
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

/// The scan's own tests, beside the table (#2273: the table's file stays
/// within 750 lines).
#[path = "swarm_board_refusal_kinds_scan.rs"]
mod scan;
