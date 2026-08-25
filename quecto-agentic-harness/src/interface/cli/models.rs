use std::time::Duration;

use crate::application::catalogue_refresh::CatalogueRefreshStatus;
use crate::infrastructure::catalogue_discovery::ModelsJsonCatalogueRefreshAdapter;

use super::CliContext;

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

    let refresh_port = ModelsJsonCatalogueRefreshAdapter::new(ctx.base_dir());
    loop {
        match refresh_port.refresh_source(&provider).status {
            CatalogueRefreshStatus::Refreshed { models } => stdout.push_str(&format!(
                "Discovered {models} model(s) for provider {provider}\n"
            )),
            CatalogueRefreshStatus::Skipped { reason }
            | CatalogueRefreshStatus::Failed { error: reason } => {
                stderr.push_str(&format!("models discover failed: {reason}\n"));
                return 1;
            }
        }
        if !watch {
            return 0;
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
