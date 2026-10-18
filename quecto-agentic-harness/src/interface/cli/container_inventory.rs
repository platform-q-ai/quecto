//! `quecto container ls [--all] [--json]`, `quecto container kill
//! <ref|name> [--created-by <key>]` and `quecto container gc [--dry-run]
//! [--name <config>]` (#2024 S4d):
//! parse the arguments, invoke one composed use case over the base
//! directory's restored environment registry, present the outcome. The
//! interface restores nothing and runs no script itself.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::CliContext;
use super::container::TOP_USAGE as USAGE;
use super::container_handles::ContainerInventoryHandles;
use crate::application::environments::dto::{
    AbandonedRuns, GcCandidate, GcRemoval, GcReport, GcRequest, RestoreMode,
};
use crate::application::environments::use_cases::{KillEnvironmentError, KilledEnvironment};
use crate::domain::environments::entities::environment_registry::{
    ClaimCondition, CreatorKey, EnvironmentLookupError, EnvironmentRecord, EnvironmentStatus,
    EnvironmentTarget, ref_number,
};

/// `quecto container kill <ref> --created-by <key>` found the record
/// created by another key: nothing was killed.
pub(super) const EXIT_NOT_CREATED_BY: i32 = 3;
/// `quecto container kill` found nothing to kill: an unknown ref (or
/// name), a record this command's own restore forgot, or a `stopped`
/// record whose script set retained no cleanup. With `--created-by` the
/// creator is judged first (exit 3 when it differs). Every other refusal
/// or failure exits 1.
pub(super) const EXIT_NOTHING_TO_KILL: i32 = 4;

fn inventory(ctx: &CliContext, mode: RestoreMode) -> Result<ContainerInventoryHandles, String> {
    let Some(build) = ctx.container_inventory else {
        return Err("container inventory not composed".to_string());
    };
    // The selection carries an explicit `--config` when given: that file
    // alone, otherwise the working directory's layers (the collector's
    // scripts come from it, as the doctor's do).
    let selection = ctx.config_selection()?;
    let handles = build(&ctx.base_dir(), &selection, mode);
    // An unreadable registry is not an empty one (review F3, #2033): a
    // listing would show nothing, a kill find nothing and the collector
    // would read every exited state dir as unrecorded — refuse all three
    // in the store's own words.
    if let Some(read_error) = &handles.restore.read_error {
        return Err(format!(
            "{read_error}
nothing is listed, killed or collected until it is repaired (or moved aside)"
        ));
    }
    Ok(handles)
}

/// The restore's notes belong on stderr, after the command's own output.
fn present_restore_notes(handles: &ContainerInventoryHandles, stderr: &mut String) {
    for line in &handles.restore.diagnostics {
        stderr.push_str(line);
        stderr.push('\n');
    }
    for environment_ref in handles.restore.forgotten_refs() {
        stderr.push_str(&format!(
            "{environment_ref} forgotten (stopped; nothing left on disk or in the runtime)\n"
        ));
    }
}

fn fail(stderr: &mut String, error: &str) -> i32 {
    stderr.push_str(error);
    if !error.ends_with('\n') {
        stderr.push('\n');
    }
    1
}

// ─── ls ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LsRequest {
    /// Include `stopped` environments.
    all: bool,
    /// One line of JSON instead of the table.
    json: bool,
}

fn parse_ls(args: &[String]) -> Result<LsRequest, String> {
    let mut request = LsRequest::default();
    for arg in args {
        match arg.as_str() {
            "--all" | "-a" => request.all = true,
            "--json" => request.json = true,
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(request)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

pub(crate) fn cmd_ls(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_ls(args)
        .and_then(|request| inventory(ctx, RestoreMode::Correct).map(|handles| (request, handles)));
    match outcome {
        Ok((LsRequest { all, json }, handles)) => {
            let mut records = handles.list.execute();
            records.sort_by_key(|record| ref_number(&record.environment_ref).unwrap_or(u64::MAX));
            let shown: Vec<&EnvironmentRecord> = records
                .iter()
                .filter(|record| all || record.status != EnvironmentStatus::Stopped)
                .collect();
            if json {
                match present_json(&shown, records.len() - shown.len(), &handles) {
                    Ok(line) => stdout.push_str(&line),
                    Err(error) => return fail(stderr, &error),
                }
            } else {
                present_table(&shown, all, records.len(), stdout);
            }
            present_restore_notes(&handles, stderr);
            for (environment_ref, reason) in &handles.restore.retained {
                stderr.push_str(&format!(
                    "note: {environment_ref} retained at restore: {reason}\n"
                ));
            }
            for (environment_ref, reason) in &handles.restore.unverified {
                stderr.push_str(&format!(
                    "note: {environment_ref} could not be verified against the runtime: {reason}\n"
                ));
            }
            // Stopped records are the listing's only with --all (#2134).
            if all {
                for (environment_ref, reason) in &handles.restore.kept_stopped {
                    stderr.push_str(&format!("note: stopped {environment_ref} kept: {reason}\n"));
                }
            }
            0
        }
        Err(error) => fail(stderr, &error),
    }
}

// ─── ls --json ───────────────────────────────────────────────────────────────
//
// The listing as one line of JSON for machines (the Pocket relay matches
// CREATED-BY on it). The table cannot carry a name or repository holding
// whitespace; serde's escaping can. Key order and nulls are part of the
// contract (docs/container-runtimes.md).

/// The JSON listing: each shown record with its restore's "could not be
/// verified" reason, and how many stopped ones `--all` would add.
fn present_json(
    shown: &[&EnvironmentRecord],
    hidden_stopped: usize,
    handles: &ContainerInventoryHandles,
) -> Result<String, String> {
    let now = now_secs();
    let rows: Vec<LsJsonEnvironment> = shown
        .iter()
        .map(|record| {
            let reasons: Vec<&str> = handles
                .restore
                .unverified
                .iter()
                .filter(|(environment_ref, _)| *environment_ref == record.environment_ref)
                .map(|(_, reason)| reason.as_str())
                .collect();
            let unverified = Some(reasons.join("; ")).filter(|joined| !joined.is_empty());
            LsJsonEnvironment::from_record(record, unverified, now)
        })
        .collect();
    ls_json(&rows, hidden_stopped)
}

/// One environment of the listing. Absent values are `null`, never `"-"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct LsJsonEnvironment {
    #[serde(rename = "ref")]
    pub environment_ref: String,
    pub name: Option<String>,
    /// The container config that created it.
    pub config: String,
    /// The registry status (`running` even with no members recorded).
    pub status: String,
    /// Member agents this listing's registry records (members live in
    /// their session's memory, not on file).
    pub members: usize,
    /// Userinfo redacted, as in the table.
    pub repository: Option<String>,
    pub created_by: Option<String>,
    /// Unix seconds.
    pub created_at: Option<u64>,
    pub age_s: Option<u64>,
    /// Why the restore could not verify it against the runtime, when it
    /// could not (the stderr note's reason).
    pub unverified: Option<String>,
}

impl LsJsonEnvironment {
    pub(super) fn from_record(
        record: &EnvironmentRecord,
        unverified: Option<String>,
        now: u64,
    ) -> Self {
        let present = |value: &str| Some(value.to_string()).filter(|value| !value.is_empty());
        Self {
            environment_ref: record.environment_ref.clone(),
            name: record.name.as_deref().and_then(present),
            config: record.script_name.clone(),
            status: record.status.name().to_string(),
            members: record.members.len(),
            repository: present(&record.repository)
                .map(|url| crate::domain::shared::redaction::redact_url_userinfo(&url)),
            created_by: present(&record.created_by),
            created_at: record.created_at,
            age_s: record.created_at.map(|at| now.saturating_sub(at)),
            unverified,
        }
    }
}

#[derive(Serialize)]
struct LsJson<'a> {
    environments: &'a [LsJsonEnvironment],
    /// Stopped environments left out because `--all` was not given.
    hidden_stopped: usize,
}

/// The listing as one line of JSON, newline-terminated. Strings, integers
/// and nulls cannot fail to serialise; were one to, the listing is refused
/// rather than replaced by an empty one.
pub(super) fn ls_json(
    environments: &[LsJsonEnvironment],
    hidden_stopped: usize,
) -> Result<String, String> {
    let listing = LsJson {
        environments,
        hidden_stopped,
    };
    let line = serde_json::to_string(&listing)
        .map_err(|error| format!("the listing could not be written as JSON: {error}"))?;
    // serde escapes control characters but not these three, which some
    // line readers (JavaScript's, Python's `str.splitlines`) split on; they
    // occur only inside strings, where the escape means the same.
    let mut line = line
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .replace('\u{0085}', "\\u0085");
    debug_assert!(!line.contains('\n'), "the listing is one line");
    line.push('\n');
    Ok(line)
}

const COLUMNS: [&str; 7] = [
    "REF",
    "NAME",
    "CONFIG",
    "STATUS",
    "REPOSITORY",
    "CREATED-BY",
    "AGE",
];

fn present_table(records: &[&EnvironmentRecord], all: bool, total: usize, stdout: &mut String) {
    let now = now_secs();
    let rows: Vec<[String; 7]> = records
        .iter()
        .map(|record| {
            [
                record.environment_ref.clone(),
                record.name.clone().unwrap_or_else(|| "-".to_string()),
                record.script_name.clone(),
                record.status_label().to_string(),
                if record.repository.is_empty() {
                    "-".to_string()
                } else {
                    crate::domain::shared::redaction::redact_url_userinfo(&record.repository)
                },
                if record.created_by.is_empty() {
                    "-".to_string()
                } else {
                    record.created_by.clone()
                },
                record
                    .created_at
                    .map(|at| age(now.saturating_sub(at)))
                    .unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    let mut widths: Vec<usize> = COLUMNS.iter().map(|column| column.len()).collect();
    for row in &rows {
        for (column, cell) in row.iter().enumerate() {
            widths[column] = widths[column].max(cell.chars().count());
        }
    }
    let line = |cells: &[&str]| {
        let mut out = String::new();
        for (column, cell) in cells.iter().enumerate() {
            if column > 0 {
                out.push_str("  ");
            }
            if column + 1 == cells.len() {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{cell:<width$}", width = widths[column]));
            }
        }
        out.trim_end().to_string()
    };
    stdout.push_str(&line(&COLUMNS));
    stdout.push('\n');
    for row in &rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        stdout.push_str(&line(&cells));
        stdout.push('\n');
    }
    let hidden = total - records.len();
    if records.is_empty() {
        stdout.push_str(if all {
            "no environments recorded\n"
        } else {
            "no live environments\n"
        });
    }
    if !all && hidden > 0 {
        stdout.push_str(&format!(
            "({hidden} stopped environment{} hidden; --all shows {})\n",
            if hidden == 1 { "" } else { "s" },
            if hidden == 1 { "it" } else { "them" }
        ));
    }
}

/// A compact age: `42s`, `7m`, `3h`, `12d`.
fn age(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

// ─── kill ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct KillRequest {
    target: EnvironmentTarget,
    /// `--created-by <key>`: only a record created by exactly `key` is
    /// killed, judged with the kill's own claim.
    condition: ClaimCondition,
}

fn parse_kill(args: &[String]) -> Result<KillRequest, String> {
    let mut targets = Vec::new();
    let mut created_by = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--created-by" => {
                let key = rest
                    .next()
                    .ok_or_else(|| format!("--created-by requires a key\n{USAGE}"))?;
                let key = CreatorKey::try_from(key.clone()).map_err(|invalid| {
                    format!("--created-by requires a key: {invalid}\n{USAGE}")
                })?;
                if created_by.replace(key).is_some() {
                    return Err(format!("--created-by may be given once\n{USAGE}"));
                }
            }
            target if target.starts_with('-') => {
                return Err(format!("unknown argument {target}\n{USAGE}"));
            }
            _ => targets.push(arg),
        }
    }
    let target = match targets.as_slice() {
        [target] if !target.is_empty() => {
            if ref_number(target).is_some() {
                EnvironmentTarget::Ref((*target).clone())
            } else {
                EnvironmentTarget::Name((*target).clone())
            }
        }
        _ => return Err(format!("kill takes exactly one <ref|name>\n{USAGE}")),
    };
    // A name can be reused the moment its environment stops; a ref is the
    // one address a creator-checked kill accepts.
    let condition = match (created_by, &target) {
        (Some(key), EnvironmentTarget::Ref(_)) => ClaimCondition::CreatedBy(key),
        (Some(_), EnvironmentTarget::Name(name)) => {
            return Err(format!(
                "--created-by takes a ref (C followed by digits), not the name '{name}'\n{USAGE}"
            ));
        }
        (None, _) => ClaimCondition::Any,
    };
    Ok(KillRequest { target, condition })
}

/// Parse, restore and kill: the restore's notes go to `stderr` first.
fn end_environment(
    ctx: &CliContext,
    args: &[String],
    stderr: &mut String,
) -> Result<KilledEnvironment, KillFailure> {
    let KillRequest { target, condition } = parse_kill(args)?;
    let handles = inventory(ctx, RestoreMode::Correct)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime: {error}"))?;
    present_restore_notes(&handles, stderr);
    let forgotten = match &target {
        EnvironmentTarget::Ref(environment_ref) => handles
            .restore
            .forgotten
            .iter()
            .find(|forgotten| forgotten.environment_ref == *environment_ref),
        EnvironmentTarget::Name(_) => None,
    };
    match forgotten {
        // The restore this command ran just forgot it (#2134): it was
        // stopped and nothing of it is left. Its creator is judged first,
        // as for any record.
        Some(forgotten) => {
            condition
                .judge_creator(&forgotten.environment_ref, &forgotten.created_by)
                .map_err(KillEnvironmentError::NotCreatedBy)?;
            Err(KillFailure::NothingToKill(format!(
                "{}; nothing of it was left, so it was forgotten",
                EnvironmentLookupError::Stopped(forgotten.environment_ref.clone())
            )))
        }
        None => Ok(runtime.block_on(handles.kill.kill_container_if(&target, &condition))?),
    }
}

/// Why a kill did not happen, and the exit code that says so.
enum KillFailure {
    /// The record under the target was created by another key.
    NotCreatedBy(String),
    /// Nothing is there to kill.
    NothingToKill(String),
    Other(String),
}

impl From<String> for KillFailure {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

impl From<KillEnvironmentError> for KillFailure {
    fn from(error: KillEnvironmentError) -> Self {
        match error {
            KillEnvironmentError::NotCreatedBy(_) => Self::NotCreatedBy(error.to_string()),
            KillEnvironmentError::NothingToKill(_) => Self::NothingToKill(error.to_string()),
            KillEnvironmentError::Refused(_)
            | KillEnvironmentError::MembersUnsettled { .. }
            | KillEnvironmentError::KillFailed { .. } => Self::Other(error.to_string()),
        }
    }
}

pub(crate) fn cmd_kill(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = end_environment(ctx, args, stderr);
    match outcome {
        Ok(killed) => {
            let name = killed
                .record
                .name
                .as_deref()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default();
            let line = if killed.removed_stopped {
                // #2206: a stopped environment's leftovers went.
                format!(
                    "removed stopped {}{name}: its container and state directory are gone and its record is forgotten\n",
                    killed.record.environment_ref
                )
            } else {
                format!("killed {}{name}\n", killed.record.environment_ref)
            };
            stdout.push_str(&line);
            0
        }
        Err(KillFailure::NotCreatedBy(error)) => {
            fail(stderr, &error);
            EXIT_NOT_CREATED_BY
        }
        Err(KillFailure::NothingToKill(error)) => {
            fail(stderr, &error);
            EXIT_NOTHING_TO_KILL
        }
        Err(KillFailure::Other(error)) => fail(stderr, &error),
    }
}

// ─── gc ──────────────────────────────────────────────────────────────────────

fn parse_gc(args: &[String]) -> Result<GcRequest, String> {
    let mut request = GcRequest::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--dry-run" | "-n" => request.dry_run = true,
            "--abandoned" => {
                if request.abandoned != AbandonedRuns::Keep {
                    return Err(format!(
                        "--abandoned and --abandoned-after may be given once\n{USAGE}"
                    ));
                }
                request.abandoned = AbandonedRuns::Collect;
            }
            "--abandoned-after" => {
                if request.abandoned != AbandonedRuns::Keep {
                    return Err(format!(
                        "--abandoned and --abandoned-after may be given once\n{USAGE}"
                    ));
                }
                let value = rest
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        format!("--abandoned-after requires a duration such as 2h or 3d\n{USAGE}")
                    })?;
                let secs = parse_duration_secs(value)
                    .filter(|secs| *secs > 0)
                    .ok_or_else(|| {
                        format!(
                            "--abandoned-after: '{value}' is not a duration above zero (a whole number of s, m, h or d, e.g. 30m, 12h, 3d; for any age pass --abandoned)\n{USAGE}"
                        )
                    })?;
                request.abandoned = AbandonedRuns::OlderThan {
                    secs,
                    spelled: value.clone(),
                };
            }
            "--name" => {
                let value = rest
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("--name requires a container config name\n{USAGE}"))?;
                if request.config.replace(value.clone()).is_some() {
                    return Err(format!("--name may be given once\n{USAGE}"));
                }
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(request)
}

/// `<digits><unit>` with a unit of `s`, `m`, `h` or `d`, as seconds; nothing
/// else (no bare number, no fractions, no compound spellings). Admitted by
/// shape: one or more ASCII digits, then one of the four units.
fn parse_duration_secs(value: &str) -> Option<u64> {
    let (digits, per_unit) = [("s", 1u64), ("m", 60), ("h", 3_600), ("d", 86_400)]
        .into_iter()
        .find_map(|(unit, per_unit)| value.strip_suffix(unit).map(|digits| (digits, per_unit)))?;
    let admitted = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if admitted {
        digits.parse::<u64>().ok()?.checked_mul(per_unit)
    } else {
        None
    }
}

pub(crate) fn cmd_gc(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_gc(args).and_then(|request| {
        // A dry run has no effect at all (round 3 H1, #2033): what the
        // restore would correct is previewed, never written.
        let mode = if request.dry_run {
            RestoreMode::Observe
        } else {
            RestoreMode::Correct
        };
        let handles = inventory(ctx, mode)?;
        present_restore_notes(&handles, stderr);
        handles
            .gc
            .execute(&request)
            .map_err(|refused| refused.to_string())
    });
    match outcome {
        Ok(report) => {
            present_gc(&report, stdout);
            for error in &report.errors {
                stderr.push_str(&format!("error: {error}\n"));
            }
            if report.errors.is_empty() { 0 } else { 1 }
        }
        Err(error) => fail(stderr, &error),
    }
}

fn candidate_line(candidate: &GcCandidate) -> String {
    let via = match &candidate.removal {
        GcRemoval::RetainedCleanup { environment_ref } => {
            format!("via retained cleanup of {environment_ref}")
        }
        GcRemoval::ConfiguredCleanup { config } => format!("via cleanup of config '{config}'"),
        GcRemoval::ForgetRecord { environment_ref } => {
            format!("by forgetting record {environment_ref}")
        }
    };
    let what = match (&candidate.state_dir, &candidate.container) {
        (Some(dir), Some(container)) => format!("{} + container {container}", dir.display()),
        (Some(dir), None) => dir.display().to_string(),
        (None, Some(container)) => format!("container {container} (no state dir)"),
        (None, None) => "nothing on disk".to_string(),
    };
    format!(
        "  {}  {what}  [{}; {via}]",
        candidate.environment_id, candidate.reason
    )
}

fn present_gc(report: &GcReport, stdout: &mut String) {
    stdout.push_str(&format!("container config \"{}\"\n", report.config));
    if report.state_roots.is_empty() {
        stdout.push_str("scanned no state roots (the config's create argv names no --state-dir and none of its records implies one)\n");
    } else {
        stdout.push_str("scanned state roots:\n");
        for root in &report.state_roots {
            stdout.push_str(&format!("  {}\n", root.display()));
        }
    }
    if report.dry_run {
        stdout.push_str(&format!(
            "would remove {} orphaned environment{}:\n",
            report.removable.len(),
            if report.removable.len() == 1 { "" } else { "s" }
        ));
        for candidate in &report.removable {
            stdout.push_str(&candidate_line(candidate));
            stdout.push('\n');
        }
    } else {
        stdout.push_str(&format!(
            "removed {} of {} orphaned environment{}:\n",
            report.removed.len(),
            report.removable.len(),
            if report.removable.len() == 1 { "" } else { "s" }
        ));
        for candidate in &report.removed {
            stdout.push_str(&candidate_line(candidate));
            stdout.push('\n');
        }
    }
    if !report.kept.is_empty() {
        stdout.push_str(&format!(
            "kept {} environment{}:\n",
            report.kept.len(),
            if report.kept.len() == 1 { "" } else { "s" }
        ));
        for kept in &report.kept {
            stdout.push_str(&format!("  {}  {}\n", kept.environment_id, kept.reason));
        }
    }
}

#[cfg(test)]
#[path = "container_inventory_hosted_tests.rs"]
mod hosted_tests;
#[cfg(test)]
#[path = "container_inventory_json_tests.rs"]
mod json_tests;
#[cfg(test)]
#[path = "container_inventory_kill_stopped_tests.rs"]
mod kill_stopped_tests;

#[cfg(test)]
#[path = "container_inventory_overtaken_tests.rs"]
mod overtaken_tests;
#[cfg(test)]
#[path = "container_inventory_tests.rs"]
mod tests;
