//! `quecto config get|set|trust` (#2024): parse the arguments, map them to
//! the configuration use cases the context was composed with, present the
//! outcome. The interface never reads or writes a config file itself.

use std::path::PathBuf;

use super::CliContext;
use super::config_loading::layer_diagnostics;
use crate::application::configuration::dto::{
    ConfigLayer, ConfigPatch, ConfigReadRequest, ConfigReadScope, ConfigSelection, ConfigTarget,
    OverlayTrustRequest,
};

const USAGE: &str = "usage: quecto config get [<dotted.path>] [--effective|--global|--local]\n       quecto config set <dotted.path> <json-value> [--global|--local]\n       quecto config trust [--path <file>]\n(`--` ends the options; a negative number is always a value)\n";

pub(crate) fn cmd_config(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let outcome = match args.first().map(String::as_str) {
        Some("get") => cmd_get(ctx, &args[1..], stdout),
        Some("set") => cmd_set(ctx, &args[1..], stdout),
        Some("trust") => cmd_trust(ctx, &args[1..], stdout),
        _ => Err(USAGE.to_string()),
    };
    match outcome {
        Ok(diagnostics) => {
            for line in diagnostics {
                stderr.push_str(&line);
                stderr.push('\n');
            }
            0
        }
        Err(error) => {
            stderr.push_str(&error);
            if !error.ends_with('\n') {
                stderr.push('\n');
            }
            1
        }
    }
}

/// Flags and positionals of one subcommand; every flag is affirmatively
/// known, anything else is a usage error.
struct Parsed {
    positionals: Vec<String>,
    scope_flag: Option<&'static str>,
    path_flag: Option<PathBuf>,
}

fn parse(args: &[String], scope_flags: &[&'static str], path_flag: bool) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        positionals: Vec::new(),
        scope_flag: None,
        path_flag: None,
    };
    let mut rest = args.iter();
    let mut options_ended = false;
    while let Some(arg) = rest.next() {
        if options_ended {
            parsed.positionals.push(arg.clone());
        } else if arg == "--" {
            options_ended = true;
        } else if let Some(flag) = scope_flags.iter().find(|flag| **flag == arg.as_str()) {
            if parsed.scope_flag.is_some() {
                return Err(format!(
                    "only one of {} may be given\n{USAGE}",
                    scope_flags.join(", ")
                ));
            }
            parsed.scope_flag = Some(flag);
        } else if path_flag && arg == "--path" {
            let value = rest
                .next()
                .ok_or_else(|| format!("--path requires a file\n{USAGE}"))?;
            parsed.path_flag = Some(PathBuf::from(value));
        } else if arg.starts_with('-') && serde_json::from_str::<serde_json::Value>(arg).is_err() {
            // A negative number is a value, not an option.
            return Err(format!("unknown option {arg}\n{USAGE}"));
        } else {
            parsed.positionals.push(arg.clone());
        }
    }
    Ok(parsed)
}

/// The overlay file of this invocation, or why there is none.
fn overlay_target(selection: &ConfigSelection) -> Result<PathBuf, String> {
    match selection {
        ConfigSelection::Explicit(_) => Err(
            "--local does not apply with --config (an explicit file replaces both layers); use --global or drop --config"
                .to_string(),
        ),
        ConfigSelection::Layered(layers) => layers.overlay.clone().ok_or_else(|| {
            "the working directory is unknown, so there is no repo-local overlay to write".to_string()
        }),
    }
}

fn cmd_get(ctx: &CliContext, args: &[String], stdout: &mut String) -> Result<Vec<String>, String> {
    let parsed = parse(args, &["--effective", "--global", "--local"], false)?;
    if parsed.positionals.len() > 1 {
        return Err(format!("get takes at most one path\n{USAGE}"));
    }
    let scope = match parsed.scope_flag {
        Some("--global") => ConfigReadScope::Global,
        Some("--local") => ConfigReadScope::Overlay,
        _ => ConfigReadScope::Effective,
    };
    let handles = ctx.configuration_handles(false)?;
    let readout = handles
        .read
        .execute(ConfigReadRequest {
            selection: ctx.config_selection()?,
            scope,
            key_path: parsed.positionals.first().cloned(),
        })
        .map_err(|error| error.to_string())?;
    stdout.push_str(&serde_json::to_string_pretty(&readout.value).expect("a JSON value renders"));
    stdout.push('\n');
    Ok(readout
        .sources
        .as_ref()
        .map(layer_diagnostics)
        .unwrap_or_default())
}

fn cmd_set(ctx: &CliContext, args: &[String], stdout: &mut String) -> Result<Vec<String>, String> {
    let parsed = parse(args, &["--global", "--local"], false)?;
    let [key_path, raw_value] = parsed.positionals.as_slice() else {
        return Err(format!("set takes a path and a value\n{USAGE}"));
    };
    // Values are JSON; a bare word that is not valid JSON is taken as a
    // string, so `set agents.defaults.model gpt-5.5` reads naturally.
    let value = serde_json::from_str(raw_value)
        .unwrap_or_else(|_| serde_json::Value::String(raw_value.clone()));
    let selection = ctx.config_selection()?;
    let target = match parsed.scope_flag {
        Some("--global") => ConfigTarget {
            layer: ConfigLayer::Global,
            path: selection.path().to_path_buf(),
        },
        _ => ConfigTarget {
            layer: ConfigLayer::Overlay,
            path: overlay_target(&selection)?,
        },
    };
    let layer = target.layer;
    let receipt = ctx
        .configuration_handles(false)?
        .patch
        .execute(ConfigPatch {
            target,
            key_path: key_path.clone(),
            value,
        })
        .map_err(|error| error.to_string())?;
    stdout.push_str(&format!(
        "set {key_path} in {}{}{}\n",
        receipt.path.display(),
        if receipt.created { " (created)" } else { "" },
        if layer == ConfigLayer::Overlay {
            " (trusted)"
        } else {
            ""
        }
    ));
    Ok(Vec::new())
}

fn cmd_trust(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
) -> Result<Vec<String>, String> {
    let parsed = parse(args, &[], true)?;
    if !parsed.positionals.is_empty() {
        return Err(format!("trust takes no positional arguments\n{USAGE}"));
    }
    let path = match parsed.path_flag {
        Some(path) => path,
        None => overlay_target(&ctx.config_selection()?)?,
    };
    let approval = ctx
        .configuration_handles(false)?
        .trust
        .execute(OverlayTrustRequest { path })
        .map_err(|error| error.to_string())?;
    stdout.push_str(&format!(
        "trusted {} (sha256 {})\n",
        approval.path.display(),
        approval.fingerprint
    ));
    Ok(Vec::new())
}

#[cfg(test)]
#[path = "config_cmd_tests.rs"]
mod tests;
