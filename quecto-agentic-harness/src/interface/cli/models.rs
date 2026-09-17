//! CLI adapter for discovering a provider's models (epic #1193 slice 4,
//! #1844): `quecto models discover` is an adapter over the composed
//! refresh use case selecting one provider. It performs no HTTP, parses no
//! registry data, and persists nothing itself — discovery lives in the
//! infrastructure refresh sources and results publish through the normal
//! catalogue path.

use std::time::Duration;

use super::CliContext;
use crate::application::catalogue::dto::REGISTRY_FILE_SOURCE;
use crate::application::catalogue::dto::{RefreshBounds, RefreshSelection, SourceRefreshStatus};
use crate::application::catalogue::use_cases::RefreshCatalogueSources;

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

    // The interface never constructs the refresh use case (#1844): the
    // command runs over the handles composition builds for this base dir.
    let Some(build_catalogue) = ctx.catalogue else {
        stderr.push_str("models discover: catalogue capability not composed\n");
        return 1;
    };
    let refresh = build_catalogue(&ctx.base_dir(), None).refresh;
    loop {
        match discover_once(&refresh, &provider) {
            Ok(DiscoverOutcome::Updated { models }) => stdout.push_str(&format!(
                "Discovered {models} model(s) for provider {provider}\n"
            )),
            // An unchanged listing is not zero models discovered: report the
            // cached total so users/scripts keying off the count never
            // conclude a stable provider lost its models (slice-4 review).
            Ok(DiscoverOutcome::Unchanged { models }) => stdout.push_str(&format!(
                "Model listing unchanged for provider {provider} ({models} model(s) cached)\n"
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

/// What one discover run did, for rendering.
#[derive(Debug)]
enum DiscoverOutcome {
    Updated { models: usize },
    Unchanged { models: usize },
}

/// Refresh one provider through the composed refresh use case. The user is
/// waiting on this directly, so the (generous) default bounds apply.
fn discover_once(
    refresh: &RefreshCatalogueSources,
    provider_key: &str,
) -> Result<DiscoverOutcome, String> {
    let report = refresh.execute(
        &RefreshSelection::Only(vec![provider_key.to_string()]),
        RefreshBounds::default(),
    );
    let outcome = report
        .outcomes
        .iter()
        .find(|o| o.source == provider_key)
        // A registry file the typed parser refuses is reported as one
        // file-level failed outcome; surface its reason instead of a
        // confusing "no outcome" message.
        .or_else(|| {
            report
                .outcomes
                .iter()
                .find(|o| o.source == REGISTRY_FILE_SOURCE)
        })
        .ok_or_else(|| format!("refresh reported no outcome for '{provider_key}'"))?;
    match &outcome.status {
        SourceRefreshStatus::Updated { models } => Ok(DiscoverOutcome::Updated { models: *models }),
        SourceRefreshStatus::Unchanged { models } => {
            Ok(DiscoverOutcome::Unchanged { models: *models })
        }
        SourceRefreshStatus::Unsupported { reason } | SourceRefreshStatus::Failed { reason } => {
            Err(reason.clone())
        }
        SourceRefreshStatus::Cancelled => Err("refresh was cancelled".to_string()),
    }
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
