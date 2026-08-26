//! CLI adapter for the catalogue refresh use case (epic #1193, slice 4).
//!
//! `quecto models discover` is an adapter over the one application refresh
//! operation: it performs no HTTP, parses no registry data, and persists
//! nothing itself — discovery lives in the infrastructure refresh sources and
//! results publish through the normal catalogue path.

use std::time::Duration;

use super::CliContext;
use super::catalogue_refresh_bridge::refresh_catalogue;
use crate::application::catalogue_refresh::{RefreshBounds, RefreshSelection, SourceRefreshStatus};

pub fn cmd_models(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    match args.first().map(String::as_str) {
        Some("discover") => cmd_discover(ctx, &args[1..], stdout, stderr),
        _ => {
            stderr.push_str(
                "Usage: quecto models discover <provider-key> [--watch] [--interval <seconds>]\n",
            );
            1
        }
    }
}

fn cmd_discover(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    if args.is_empty() {
        stderr.push_str(
            "Usage: quecto models discover <provider-key> [--watch] [--interval <seconds>]\n",
        );
        return 1;
    }
    let provider = args[0].clone();
    let mut watch = false;
    let mut interval = 300_u64;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--watch" => {
                watch = true;
                i += 1;
            }
            "--interval" if i + 1 < args.len() => {
                interval = match args[i + 1].parse() {
                    Ok(0) => {
                        stderr.push_str("--interval must be at least 1 second\n");
                        return 1;
                    }
                    Ok(v) => v,
                    Err(_) => {
                        stderr.push_str("--interval must be an integer number of seconds\n");
                        return 1;
                    }
                };
                i += 2;
            }
            other => {
                stderr.push_str(&format!("Unknown models discover option: {other}\n"));
                return 1;
            }
        }
    }

    loop {
        match discover_once(ctx, &provider) {
            Ok(count) => stdout.push_str(&format!(
                "Discovered {count} model(s) for provider {provider}\n"
            )),
            Err(error) => {
                stderr.push_str(&format!("models discover failed: {error}\n"));
                return 1;
            }
        }
        if !watch {
            return 0;
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

/// Refresh one provider through the application refresh use case. The user is
/// waiting on this directly, so the (generous) default bounds apply.
fn discover_once(ctx: &CliContext, provider_key: &str) -> Result<usize, String> {
    let report = refresh_catalogue(
        &ctx.base_dir(),
        &RefreshSelection::Only(vec![provider_key.to_string()]),
        RefreshBounds::default(),
    );
    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.source == provider_key)
        .ok_or_else(|| format!("refresh reported no outcome for '{provider_key}'"))?;
    match &outcome.status {
        SourceRefreshStatus::Updated { models } => Ok(*models),
        SourceRefreshStatus::Unchanged => Ok(0),
        SourceRefreshStatus::Unsupported { reason } | SourceRefreshStatus::Failed { reason } => {
            Err(reason.clone())
        }
        SourceRefreshStatus::Cancelled => Err("refresh was cancelled".to_string()),
    }
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
