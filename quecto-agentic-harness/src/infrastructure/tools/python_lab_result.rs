use std::path::Path;

use serde_json::json;

use super::{PythonLabConfig, artifact_rel, read_preview, truncation_marker_exists};
use crate::domain::error::DomainError;

/// True when an artifact's size no longer matches what was captured when the
/// job completed, which means something rewrote it after the fact. Reports
/// false while the job is still running and has no captured sizes yet.
pub(crate) async fn artifacts_diverged(
    result: Option<&serde_json::Value>,
    stdout_path: &Path,
    stderr_path: &Path,
) -> bool {
    let Some(usage) = result.and_then(|r| r.get("resource_usage")) else {
        return false;
    };
    for (key, path) in [
        ("stdout_bytes_retained", stdout_path),
        ("stderr_bytes_retained", stderr_path),
    ] {
        let Some(captured) = usage.get(key).and_then(|v| v.as_u64()) else {
            // Nothing was captured for this stream, so there is nothing to
            // compare — check the other one rather than giving up on both.
            continue;
        };
        if file_len(path).await != Some(captured) {
            return true;
        }
    }
    false
}

pub(crate) struct ResultContext<'a> {
    pub(crate) status: &'a str,
    pub(crate) exit_code: Option<i32>,
    pub(crate) exec_id: &'a str,
    pub(crate) _session_key: &'a str,
    pub(crate) _invocation_type: &'a str,
    pub(crate) background: bool,
    pub(crate) start: u128,
    pub(crate) end: u128,
    pub(crate) timeout: u64,
    pub(crate) stdout_path: &'a Path,
    pub(crate) stderr_path: &'a Path,
    pub(crate) max_out: usize,
    pub(crate) changed: Vec<String>,
    pub(crate) cfg: &'a PythonLabConfig,
}

pub(crate) async fn build_result(ctx: ResultContext<'_>) -> Result<serde_json::Value, DomainError> {
    let (stdout, stdout_full) = read_preview(ctx.stdout_path, ctx.max_out).await?;
    let (stderr, stderr_full) = read_preview(ctx.stderr_path, ctx.max_out).await?;
    let stdout_marker = truncation_marker_exists(ctx.stdout_path).await?;
    let stderr_marker = truncation_marker_exists(ctx.stderr_path).await?;
    let st = stdout_full || stdout_marker;
    let et = stderr_full || stderr_marker;
    let artifact_paths = [(st, ctx.stdout_path), (et, ctx.stderr_path)]
        .into_iter()
        .filter(|(t, _)| *t)
        .map(|(_, p)| artifact_rel(p))
        .collect::<Vec<_>>();
    // Per-process CPU and peak-RSS accounting would need wait4(2) rusage, which
    // tokio::process reaps internally and does not expose. Those fields are
    // reported as null rather than omitted so consumers can tell "not measured"
    // apart from "measured as zero".
    // Byte counts are what was RETAINED after the cap clipped the streams, not
    // what the program produced — named accordingly so they are not read as
    // output volume. A missing artifact reports null rather than 0, so "no file"
    // stays distinguishable from "wrote nothing". Wall-clock time is already
    // reported as `duration_ms` on the enclosing object and is not repeated.
    let resource_usage = json!({
        "stdout_bytes_retained": file_len(ctx.stdout_path).await,
        "stderr_bytes_retained": file_len(ctx.stderr_path).await,
        "cpu_time_ms": serde_json::Value::Null,
        "max_rss_bytes": serde_json::Value::Null,
    });
    let mut result = json!({"status":ctx.status,"exit_code":ctx.exit_code,"execution_id":ctx.exec_id,"stdout":stdout,"stderr":stderr,"duration_ms":ctx.end.saturating_sub(ctx.start)});
    let obj = result.as_object_mut().expect("result object");
    if st || et {
        obj.insert("output_truncated".into(), json!(true));
        if st {
            obj.insert("stdout_truncated".into(), json!(true));
        }
        if et {
            obj.insert("stderr_truncated".into(), json!(true));
        }
        obj.insert("artifact_paths".into(), json!(artifact_paths));
        obj.insert("resource_usage".into(), resource_usage.clone());
    }
    if !ctx.changed.is_empty() {
        obj.insert("files_created_or_modified".into(), json!(ctx.changed));
    }
    if ctx.background {
        obj.insert("resource_usage".into(), resource_usage.clone());
    }
    if ctx.status == "timed_out" || ctx.status == "cancelled" {
        obj.insert(
            "timeout_or_cancel_reason".into(),
            json!(if ctx.status == "timed_out" {
                "timeout"
            } else {
                "cancelled"
            }),
        );
        obj.insert("timeout_seconds".into(), json!(ctx.timeout));
    }
    if ctx.cfg.max_memory_bytes.is_some()
        || ctx.cfg.max_cpu_seconds.is_some()
        || ctx.cfg.max_processes != Some(1)
        || ctx.status == "timed_out"
    {
        obj.insert(
            "resource_limits".into(),
            json!({"memory_bytes":ctx.cfg.max_memory_bytes,"cpu_seconds":ctx.cfg.max_cpu_seconds,"processes":ctx.cfg.max_processes}),
        );
        obj.insert("resource_usage".into(), resource_usage);
    }
    Ok(result)
}

/// `None` when the artifact is absent or unreadable, so a missing file is not
/// reported as a zero-byte one.
pub(crate) async fn file_len(path: &Path) -> Option<u64> {
    tokio::fs::metadata(path).await.ok().map(|m| m.len())
}
