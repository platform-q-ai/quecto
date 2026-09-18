//! `quecto admission-broker run|status|reset|install-service|uninstall-service`:
//! the one host-wide inference-admission authority (#1679 P3, #2024 S3).
//!
//! `run` owns the singleton lock and sockets until SIGTERM/SIGINT. Every other
//! action is owner-only administration invoked through one `application/admission`
//! use case (`status`→inspect, `reset`, `install-service`, `uninstall-service`).
//! `status`/`reset`/`install-service`/`uninstall-service` (and `run`) address
//! the **global** config file (or an explicit `--config`/`--directory`), never
//! the cwd overlay — S1 made `admission` global-only — so the working directory
//! no longer changes which broker you address. Every output names the directory
//! it addressed.
use std::path::PathBuf;

use super::CliContext;
use crate::application::admission::dto::{
    AuthorityAdminError, InstallServiceRequest, ServiceAction, ServiceReport,
};
use crate::infrastructure::admission::{AuthorityDirectory, AuthorityServer, ServerError};
use crate::infrastructure::config::Config;

/// Parsed options shared by the broker actions.
struct BrokerArgs {
    config: Option<PathBuf>,
    directory: Option<PathBuf>,
    dry_run: bool,
    accept_missing_ledger: bool,
}

fn parse_args(args: &[String], stderr: &mut String) -> Option<BrokerArgs> {
    let mut parsed = BrokerArgs {
        config: None,
        directory: None,
        dry_run: false,
        accept_missing_ledger: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                let Some(value) = args.get(i + 1) else {
                    stderr.push_str("admission-broker: --config needs a path\n");
                    return None;
                };
                parsed.config = Some(PathBuf::from(value));
                i += 2;
            }
            "--directory" => {
                let Some(value) = args.get(i + 1) else {
                    stderr.push_str("admission-broker: --directory needs a path\n");
                    return None;
                };
                parsed.directory = Some(PathBuf::from(value));
                i += 2;
            }
            "--dry-run" => {
                parsed.dry_run = true;
                i += 1;
            }
            "--accept-missing-ledger" => {
                parsed.accept_missing_ledger = true;
                i += 1;
            }
            other => {
                stderr.push_str(&format!("admission-broker: unknown option '{other}'\n"));
                return None;
            }
        }
    }
    Some(parsed)
}

/// The global config path a broker command addresses: an explicit `--config`,
/// else `<base_dir>/config.json` (never the cwd overlay).
fn global_config_path(ctx: &CliContext, opts: &BrokerArgs) -> PathBuf {
    // An explicit `--config` is captured globally into `ctx.config_path` before
    // the subcommand's args are seen, so it wins; a `--config` in the
    // subcommand's own args is a fallback. Absent both, the global file — never
    // the cwd overlay (S1 made `admission` global-only).
    opts.config
        .clone()
        .or_else(|| ctx.config_path.clone())
        .unwrap_or_else(|| ctx.base_dir().join("config.json"))
}

/// The authority directory a command addresses: an explicit `--directory`
/// wins; otherwise the global config's `admission.directory` (or the default
/// `<base_dir>/admission` when no section is configured).
fn resolve_directory(ctx: &CliContext, opts: &BrokerArgs) -> Result<PathBuf, String> {
    if let Some(directory) = &opts.directory {
        validate_explicit_directory(&ctx.base_dir(), directory)?;
        return Ok(directory.clone());
    }
    // Without an explicit --directory, the global config (never the cwd
    // overlay) names the broker to address. No configured `admission` section
    // means there is no broker to address: error rather than guessing the
    // default directory, so `status`/`reset` are unambiguous.
    configured_directory(ctx, opts).map(|(directory, _)| directory)
}

/// An explicit `--directory` gets the checks a configured directory gets:
/// absolute; never the base directory or one of its ancestors (the base
/// directory is mounted into containers, which would expose the journal,
/// admin socket and owner token); and, when it exists, owner-only.
fn validate_explicit_directory(
    base_dir: &std::path::Path,
    directory: &std::path::Path,
) -> Result<(), String> {
    if !directory.is_absolute() {
        return Err(format!(
            "admission-broker: --directory {} must be absolute",
            directory.display()
        ));
    }
    if base_dir.starts_with(directory) {
        return Err(format!(
            "admission-broker: --directory {} is the base directory or one of its ancestors; use a subdirectory such as {}",
            directory.display(),
            crate::infrastructure::config_admission::default_admission_directory(base_dir)
                .display()
        ));
    }
    if directory.exists() {
        AuthorityDirectory::check_private_path(directory)
            .map_err(|e| format!("admission-broker: --directory {e}"))?;
    }
    Ok(())
}

/// The directory and proposal the addressed global config configures.
fn configured_directory(
    ctx: &CliContext,
    opts: &BrokerArgs,
) -> Result<
    (
        PathBuf,
        crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal,
    ),
    String,
> {
    let base_dir = ctx.base_dir();
    let config_path = global_config_path(ctx, opts);
    if !config_path.exists() {
        return Err(format!(
            "admission-broker: config {} not found; no `admission` section to address",
            config_path.display()
        ));
    }
    let config = Config::load(config_path.to_str().unwrap_or(""))
        .map_err(|e| {
            format!(
                "admission-broker: failed to load config {}: {e}",
                config_path.display()
            )
        })?
        .with_admission_base_dir(&base_dir);
    match config.admission_proposal() {
        Ok(Some(configured)) => Ok(configured),
        Ok(None) => Err(
            "admission-broker: no `admission` section is configured; nothing to address (or pass --directory)"
                .to_string(),
        ),
        Err(error) => Err(format!("admission-broker: {error}")),
    }
}

/// What `run` serves: the addressed config's proposal at its configured
/// directory. `run` cannot serve a policy from `--directory` alone, so an
/// explicit `--directory` is honoured only when it names that directory and
/// refused loudly otherwise (never silently ignored).
fn run_plan(
    ctx: &CliContext,
    opts: &BrokerArgs,
) -> Result<
    (
        PathBuf,
        crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal,
    ),
    String,
> {
    let (directory, proposal) = configured_directory(ctx, opts)?;
    if let Some(explicit) = &opts.directory {
        validate_explicit_directory(&ctx.base_dir(), explicit)?;
        if explicit != &directory {
            return Err(format!(
                "admission-broker: run serves the config's admission.directory {}; --directory {} does not match it (edit the config or omit --directory)",
                directory.display(),
                explicit.display()
            ));
        }
    }
    Ok((directory, proposal))
}

pub(crate) fn cmd_admission_broker(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    crate::infrastructure::logging::install_redacting_subscriber();
    let Some(action) = args.first().map(String::as_str) else {
        stderr.push_str(
            "admission-broker: expected one of run, status, reset, install-service, uninstall-service\n",
        );
        return 2;
    };
    let Some(opts) = parse_args(&args[1..], stderr) else {
        return 2;
    };
    match action {
        "run" => cmd_run(ctx, &opts, stdout, stderr),
        "status" => cmd_status(ctx, &opts, stdout, stderr),
        "reset" => cmd_reset(ctx, &opts, stdout, stderr),
        "install-service" => cmd_install(ctx, &opts, stdout, stderr),
        "uninstall-service" => cmd_uninstall(ctx, &opts, stdout, stderr),
        other => {
            stderr.push_str(&format!(
                "admission-broker: unknown action '{other}' (expected run, status, reset, install-service, uninstall-service)\n"
            ));
            2
        }
    }
}

fn cmd_status(
    ctx: &CliContext,
    opts: &BrokerArgs,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let directory = match resolve_directory(ctx, opts) {
        Ok(directory) => directory,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let handles = match ctx.admission_handles() {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    match handles.inspect.execute(&directory) {
        Ok(report) => {
            stdout.push_str(&status_json(&report).to_string());
            stdout.push('\n');
            0
        }
        Err(AuthorityAdminError::NotRunning { directory, reason }) => {
            stderr.push_str(&format!(
                "admission-broker: not running for directory {} ({reason})\n",
                directory.display()
            ));
            1
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            1
        }
    }
}

fn cmd_reset(ctx: &CliContext, opts: &BrokerArgs, stdout: &mut String, stderr: &mut String) -> i32 {
    let directory = match resolve_directory(ctx, opts) {
        Ok(directory) => directory,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let handles = match ctx.admission_handles() {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    match handles.reset.execute(&directory) {
        Ok(report) => {
            stdout.push_str(
                &serde_json::json!({"directory": report.directory, "epoch": report.epoch})
                    .to_string(),
            );
            stdout.push('\n');
            stderr.push_str(&format!(
                "admission-broker: reset acknowledged for {}: old-epoch remote work is no longer claimed bounded; roots re-register on their next attempt, children fail closed and must be respawned\n",
                report.directory.display()
            ));
            0
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            1
        }
    }
}

fn cmd_install(
    ctx: &CliContext,
    opts: &BrokerArgs,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let directory = match resolve_directory(ctx, opts) {
        Ok(directory) => directory,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let config = match install_config_path(ctx, opts) {
        Ok(path) => path,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let binary = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            stderr.push_str(&format!(
                "admission-broker: cannot resolve the quecto binary path: {error}\n"
            ));
            return 1;
        }
    };
    let handles = match ctx.admission_handles() {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    match handles.install.execute(InstallServiceRequest {
        binary,
        config,
        directory,
        dry_run: opts.dry_run,
    }) {
        Ok(report) => {
            print_service_report(&report, stdout);
            0
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: install-service: {error}\n"));
            1
        }
    }
}

fn cmd_uninstall(
    ctx: &CliContext,
    opts: &BrokerArgs,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let directory = match resolve_directory(ctx, opts) {
        Ok(directory) => directory,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let handles = match ctx.admission_handles() {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    match handles.uninstall.execute(directory, opts.dry_run) {
        Ok(report) => {
            print_service_report(&report, stdout);
            0
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: uninstall-service: {error}\n"));
            1
        }
    }
}

/// The config path baked into the installed unit: an explicit `--config`, else
/// the global `<base_dir>/config.json`, canonicalized to an absolute path so
/// the unit never depends on a working directory.
fn install_config_path(ctx: &CliContext, opts: &BrokerArgs) -> Result<PathBuf, String> {
    let path = global_config_path(ctx, opts);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .map_err(|e| format!("admission-broker: cannot resolve cwd for --config: {e}"))?
    };
    Ok(absolute)
}

fn print_service_report(report: &ServiceReport, stdout: &mut String) {
    stdout.push_str(&format!(
        "admission-broker: {} {} (directory {})\n",
        if report.dry_run {
            "would apply"
        } else {
            "applied"
        },
        report.unit,
        report.directory.display()
    ));
    for action in &report.actions {
        stdout.push_str("  - ");
        stdout.push_str(&describe_action(action));
        stdout.push('\n');
    }
}

fn describe_action(action: &ServiceAction) -> String {
    match action {
        ServiceAction::WroteUnit { path } => format!("wrote unit {}", path.display()),
        ServiceAction::UnitUnchanged { path } => {
            format!("unit {} already up to date", path.display())
        }
        ServiceAction::RemovedUnit { path } => format!("removed unit {}", path.display()),
        ServiceAction::NoUnitToRemove { path } => {
            format!("no unit to remove at {}", path.display())
        }
        ServiceAction::DaemonReloaded => "reloaded the user daemon".to_string(),
        ServiceAction::EnabledAndStarted { unit } => format!("enabled and started {unit}"),
        ServiceAction::Restarted { unit } => format!("restarted {unit} on the rewritten unit"),
        ServiceAction::DisabledAndStopped { unit } => format!("disabled and stopped {unit}"),
        ServiceAction::NothingToDisable { unit } => format!("{unit} was not enabled"),
        ServiceAction::Planned { description } => format!("(dry run) {description}"),
    }
}

fn status_json(report: &crate::application::admission::dto::AuthorityReport) -> serde_json::Value {
    let groups: serde_json::Map<String, serde_json::Value> = report
        .groups
        .iter()
        .map(|(id, s)| {
            (
                id.clone(),
                serde_json::json!({
                    "active": s.active,
                    "queued": s.queued,
                    "uncertain": s.uncertain,
                    "cooldown_until_ms": s.cooldown_until_ms,
                    "unavailable": s.unavailable,
                }),
            )
        })
        .collect();
    serde_json::json!({
        "directory": report.directory,
        "epoch": report.epoch,
        "journal_healthy": report.journal_healthy,
        "live_scopes": report.live_scopes,
        "groups": groups,
    })
}

fn cmd_run(ctx: &CliContext, opts: &BrokerArgs, stdout: &mut String, stderr: &mut String) -> i32 {
    let (directory, proposal) = match run_plan(ctx, opts) {
        Ok(plan) => plan,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    stderr.push_str(&format!(
        "admission-broker: serving authority directory {}\n",
        directory.display()
    ));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: runtime: {error}\n"));
            return 1;
        }
    };
    let dir = match AuthorityDirectory::open(&directory) {
        Ok(dir) => dir,
        Err(error) => {
            stderr.push_str(&format!(
                "admission-broker: directory {}: {error}\n",
                directory.display()
            ));
            return 1;
        }
    };
    runtime.block_on(run(
        dir,
        proposal,
        opts.accept_missing_ledger,
        stdout,
        stderr,
    ))
}

async fn run(
    dir: AuthorityDirectory,
    proposal: crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal,
    accept_missing_ledger: bool,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: signal handler: {error}\n"));
            return 1;
        }
    };
    let stop = async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    };
    run_until(dir, proposal, accept_missing_ledger, stop, stdout, stderr).await
}

/// Serve until `stop` resolves. Separated from the signal wiring so the
/// lifecycle is testable in-process.
pub(crate) async fn run_until(
    dir: AuthorityDirectory,
    proposal: crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal,
    accept_missing_ledger: bool,
    stop: impl std::future::Future<Output = ()>,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let started = if accept_missing_ledger {
        AuthorityServer::start_accepting_missing_ledger(dir, proposal).await
    } else {
        AuthorityServer::start(dir, proposal).await
    };
    let server = match started {
        Ok(server) => server,
        Err(ServerError::Busy(message)) => {
            stderr.push_str(&format!("admission-broker: {message}\n"));
            return 3;
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    // Announce readiness on stderr immediately (stdout is buffered by the CLI
    // shell until exit): launch scripts wait for this line or the socket.
    eprintln!(
        "admission authority ready: {}",
        server.directory().client_socket().display()
    );
    stop.await;
    server.shutdown().await;
    stdout.push_str("admission authority stopped; outstanding work stays journaled\n");
    0
}

#[cfg(test)]
#[path = "admission_broker_tests.rs"]
mod tests;
