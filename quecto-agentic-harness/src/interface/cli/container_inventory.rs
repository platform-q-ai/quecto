//! `quecto container ls [--all]`, `quecto container kill <ref|name>` and
//! `quecto container gc [--dry-run] [--state-dir <dir>]...` (#2024 S4d):
//! parse the arguments, invoke one composed use case over the base
//! directory's restored environment registry, present the outcome. The
//! interface restores nothing and runs no script itself.

use std::time::{SystemTime, UNIX_EPOCH};

use super::CliContext;
use super::container::USAGE;
use super::container_handles::ContainerInventoryHandles;
use crate::application::environments::dto::{GcCandidate, GcRemoval, GcReport, GcRequest};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, EnvironmentTarget, ref_number,
};

fn inventory(ctx: &CliContext) -> Result<ContainerInventoryHandles, String> {
    let Some(build) = ctx.container_inventory else {
        return Err("container inventory not composed".to_string());
    };
    Ok(build(&ctx.base_dir()))
}

fn fail(stderr: &mut String, error: &str) -> i32 {
    stderr.push_str(error);
    if !error.ends_with('\n') {
        stderr.push('\n');
    }
    1
}

// ─── ls ──────────────────────────────────────────────────────────────────────

fn parse_ls(args: &[String]) -> Result<bool, String> {
    let mut all = false;
    for arg in args {
        match arg.as_str() {
            "--all" | "-a" => all = true,
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(all)
}

pub(crate) fn cmd_ls(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_ls(args).and_then(|all| inventory(ctx).map(|handles| (all, handles)));
    match outcome {
        Ok((all, handles)) => {
            let mut records = handles.list.execute();
            records.sort_by_key(|record| ref_number(&record.environment_ref).unwrap_or(u64::MAX));
            let shown: Vec<&EnvironmentRecord> = records
                .iter()
                .filter(|record| all || record.status != EnvironmentStatus::Stopped)
                .collect();
            present_table(&shown, all, records.len(), stdout);
            for (environment_ref, reason) in &handles.restore.unverified {
                stderr.push_str(&format!(
                    "note: {environment_ref} could not be verified against the runtime: {reason}\n"
                ));
            }
            0
        }
        Err(error) => fail(stderr, &error),
    }
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
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
                    crate::domain::redaction::redact_url_userinfo(&record.repository)
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

fn parse_kill(args: &[String]) -> Result<EnvironmentTarget, String> {
    match args {
        [target] if !target.is_empty() => Ok(if ref_number(target).is_some() {
            EnvironmentTarget::Ref(target.clone())
        } else {
            EnvironmentTarget::Name(target.clone())
        }),
        _ => Err(format!("kill takes exactly one <ref|name>\n{USAGE}")),
    }
}

pub(crate) fn cmd_kill(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_kill(args).and_then(|target| {
        let handles = inventory(ctx)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("runtime: {error}"))?;
        runtime
            .block_on(handles.kill.kill_container(&target))
            .map_err(|error| error.to_string())
    });
    match outcome {
        Ok(killed) => {
            stdout.push_str(&format!(
                "killed {}{}\n",
                killed.record.environment_ref,
                killed
                    .record
                    .name
                    .as_deref()
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default()
            ));
            0
        }
        Err(error) => fail(stderr, &error),
    }
}

// ─── gc ──────────────────────────────────────────────────────────────────────

fn parse_gc(args: &[String]) -> Result<GcRequest, String> {
    let mut request = GcRequest::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--dry-run" | "-n" => request.dry_run = true,
            "--state-dir" => {
                let value = rest
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("--state-dir requires a directory\n{USAGE}"))?;
                request.state_roots.push(std::path::PathBuf::from(value));
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(request)
}

pub(crate) fn cmd_gc(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = parse_gc(args).and_then(|request| {
        let handles = inventory(ctx)?;
        Ok(handles.gc.execute(&request))
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
        GcRemoval::Direct => "directly".to_string(),
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
    if report.state_roots.is_empty() {
        stdout
            .push_str("scanned no state roots (no environment recorded; pass --state-dir <dir>)\n");
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
#[path = "container_inventory_tests.rs"]
mod tests;
