//! `quecto container doctor [--name <config>]` (#2024 S4b): parse the
//! arguments, invoke the composed diagnosis for the working directory's
//! effective container config (or the `--config` file's), present each
//! check with its remedy. Exit 1 when any check failed or the diagnosis
//! could not be made. The interface resolves no config and runs no
//! script itself.

use super::CliContext;
use crate::application::environments::dto::{
    CheckStatus, ContainerRuntimeDiagnosis, ContainerRuntimeTarget,
};

const USAGE: &str = "usage: quecto container doctor [--name <config>]\n(with --config <file> the file's container_configs are diagnosed instead of the working directory's effective ones)\n";

pub(crate) fn cmd_container(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    match args.first().map(String::as_str) {
        Some("doctor") => cmd_doctor(ctx, &args[1..], stdout, stderr),
        _ => {
            stderr.push_str(USAGE);
            1
        }
    }
}

fn parse_doctor(args: &[String]) -> Result<Option<String>, String> {
    let mut name = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--name" => {
                let value = rest
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("--name requires a container config name\n{USAGE}"))?;
                if name.replace(value.clone()).is_some() {
                    return Err(format!("--name may be given once\n{USAGE}"));
                }
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(name)
}

fn cmd_doctor(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let outcome = parse_doctor(args).and_then(|name| {
        let Some(build) = ctx.container_doctor else {
            return Err("container doctor not composed".to_string());
        };
        // The selection carries an explicit `--config` when given: that
        // file alone, otherwise the working directory's layers.
        let selection = ctx.config_selection()?;
        build(&ctx.base_dir(), &selection)
            .execute(&ContainerRuntimeTarget { name })
            .map_err(|error| error.to_string())
    });
    match outcome {
        Ok(diagnosis) => {
            present(&diagnosis, stdout);
            for line in &diagnosis.diagnostics {
                stderr.push_str(line);
                stderr.push('\n');
            }
            if diagnosis.healthy() { 0 } else { 1 }
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

fn present(diagnosis: &ContainerRuntimeDiagnosis, stdout: &mut String) {
    stdout.push_str(&format!(
        "container config \"{}\" (create: {})\n",
        diagnosis.config,
        diagnosis.create.join(" ")
    ));
    let width = diagnosis
        .checks
        .iter()
        .map(|check| check.name.len())
        .max()
        .unwrap_or(0);
    for check in &diagnosis.checks {
        let mark = match check.status {
            CheckStatus::Passed => "✓",
            CheckStatus::Warned => "!",
            CheckStatus::Failed => "✗",
        };
        stdout.push_str(&format!(
            "  {mark} {:<width$}  {}\n",
            check.name,
            check.detail,
            width = width
        ));
        if check.status != CheckStatus::Passed && !check.remedy.is_empty() {
            stdout.push_str(&format!("    remedy: {}\n", check.remedy));
        }
    }
    let failed = diagnosis.failed();
    let warned = diagnosis
        .checks
        .iter()
        .filter(|check| check.status == CheckStatus::Warned)
        .count();
    stdout.push_str(&format!(
        "{} check{} failed, {warned} warning{}\n",
        failed,
        if failed == 1 { "" } else { "s" },
        if warned == 1 { "" } else { "s" }
    ));
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
