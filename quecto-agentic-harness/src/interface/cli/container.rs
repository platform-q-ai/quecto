//! `quecto container doctor [--name <config>]` (#2024 S4b): parse the
//! arguments, invoke the composed diagnosis for the working directory's
//! effective container config (or the `--config` file's), present each
//! check with its remedy. Exit 1 when any check failed or the diagnosis
//! could not be made. `quecto container init|status` (#2024 S4e) parse
//! and present the standard bundle's initialisation and standing the
//! same way (`container_setup.rs`); `quecto container ls|kill|gc` (#2024
//! S4d) live in `container_inventory.rs` over the composed inventory
//! handles. The interface resolves no config and runs no script itself.

use super::CliContext;
use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::dto::{
    CheckStatus, ContainerRuntimeDiagnosis, ContainerRuntimeTarget,
};
use crate::application::environments::use_cases::{
    ContainerStatus, DiagnoseContainerRuntime, InitialiseStandardContainer,
};
use crate::domain::redaction::redact_url_userinfo;

/// Composition's builder of the container-runtime doctor: the create
/// preflight of the effective container config, over the run's own
/// configuration selection. Injected through the CLI context; the
/// interface never resolves a container config or runs a script itself.
pub type ContainerDoctorBuilder =
    fn(&std::path::Path, &ConfigSelection) -> std::sync::Arc<DiagnoseContainerRuntime>;

/// Composition's builder of an agent run's durable environment registry
/// (#2024 S4d): `(base_dir, session key, seed)` — seeded with the base
/// directory's records for a top-level session, journalling only for a
/// spawned child.
pub type EnvironmentRegistryBuilder =
    fn(&std::path::Path, &str, bool) -> crate::domain::environment_registry::EnvironmentRegistry;

/// Composition's builder of the `container ls|kill|gc` handles (#2024
/// S4d) over a registry restored from the base directory.
/// The inventory handles over a registry restored from the base directory
/// in the given mode (round 3 H1, #2033): correcting for `ls|kill|gc`,
/// observing for `gc --dry-run`, which must leave the document untouched.
pub type ContainerInventoryBuilder = fn(
    &std::path::Path,
    &ConfigSelection,
    crate::application::environments::dto::RestoreMode,
) -> super::container_handles::ContainerInventoryHandles;

const USAGE: &str = "usage: quecto container doctor [--name <config>]\n(with --config <file> the file's container_configs are diagnosed instead of the working directory's effective ones)\n";
pub(super) const TOP_USAGE: &str = "usage: quecto container init [--project <abs dir>] [--repo <url>] [--image <tag>] [--refresh] [--dry-run]\nusage: quecto container status [--project <abs dir>]\nusage: quecto container doctor [--name <config>]\nusage: quecto container ls [--all]\nusage: quecto container kill <ref|name>\nusage: quecto container gc [--dry-run] [--name <config>]\n(with --config <file> the file's container_configs are diagnosed instead of the working directory's effective ones)\n";

/// Composition's builder of `quecto container init` (#2024 S4e): the
/// standard bundle materialised below the project and the overlay entry
/// written through the configuration capability's one write path.
pub type ContainerInitBuilder =
    fn(&std::path::Path, &ConfigSelection) -> std::sync::Arc<InitialiseStandardContainer>;

/// Composition's builder of `quecto container status` (#2024 S4e).
pub type ContainerStatusBuilder =
    fn(&std::path::Path, &ConfigSelection) -> std::sync::Arc<ContainerStatus>;

pub(crate) fn cmd_container(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    match args.first().map(String::as_str) {
        Some("doctor") => cmd_doctor(ctx, &args[1..], stdout, stderr),
        Some("init") => super::container_setup::cmd_init(ctx, &args[1..], stdout, stderr),
        Some("status") => super::container_setup::cmd_status(ctx, &args[1..], stdout, stderr),
        Some("ls") => super::container_inventory::cmd_ls(ctx, &args[1..], stdout, stderr),
        Some("kill") => super::container_inventory::cmd_kill(ctx, &args[1..], stdout, stderr),
        Some("gc") => super::container_inventory::cmd_gc(ctx, &args[1..], stdout, stderr),
        _ => {
            stderr.push_str(TOP_USAGE);
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
    // The create argv names the config's `--repo`; a token embedded in
    // it belongs to the config file, not the terminal.
    stdout.push_str(&format!(
        "container config \"{}\" (create: {})\n",
        diagnosis.config,
        redact_url_userinfo(&diagnosis.create.join(" "))
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
