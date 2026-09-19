mod admission_broker;
pub mod admission_handles;
mod agent;
mod auth;
pub mod catalogue_handles;
mod commands;
mod config_cmd;
mod config_flag;
mod config_loading;
pub mod configuration_handles;
mod container;
pub mod container_config_handles;
pub mod container_handles;
mod container_inventory;
mod container_setup;
pub use container::{
    ContainerDoctorBuilder, ContainerInitBuilder, ContainerInventoryBuilder,
    ContainerStatusBuilder, EnvironmentRegistryBuilder,
};
mod help;
mod models;
pub mod protocol;
pub mod uds;
mod uds_admission_projection;
#[cfg(test)]
#[path = "uds_admission_projection_tests.rs"]
mod uds_admission_projection_tests;
mod uds_busy_get_message;
mod uds_busy_subagents;
#[cfg(test)]
mod uds_busy_subagents_tests;
pub mod uds_cancel;
mod uds_cancel_history;
mod uds_control_forward;
mod uds_delete_all_subagents;
mod uds_dispatch_reload;
pub mod uds_execution_state;
mod uds_progress_forward;

#[cfg(any(test, feature = "test-support"))]
mod uds_test_support;
#[cfg(any(test, feature = "test-support"))]
pub use uds_test_support::{
    AdmissionStateProbe, admission_broadcast_hook, busy_reader_dispatch,
    completed_live_execution_state, ledger_hint_lines_for_turn_events,
    live_execution_state_for_events,
};
#[cfg(any(test, feature = "test-support"))]
mod uds_busy_test_support;
#[cfg(any(test, feature = "test-support"))]
pub use uds_busy_test_support::{busy_reader_intercept, busy_reader_intercept_with_fleet};
/// Drive an in-process harness through the termination-signal path (#1938)
/// without signalling the test process.
#[cfg(any(test, feature = "test-support"))]
pub use uds_shutdown::test_support::deliver_termination_signal;

pub mod retention_handles;
pub mod uds_discovery_handles;
#[cfg(test)]
mod uds_execution_state_tests;
mod uds_ext_protocol;
mod uds_extensions;
mod uds_latest_report;
mod uds_lifecycle;
pub mod uds_models;
pub(crate) mod uds_multi;
mod uds_multi_accept;
pub mod uds_parent_control;
mod uds_query;
mod uds_reader;
mod uds_reader_dispatch;
mod uds_search_numbers;
pub mod uds_session;
pub mod uds_session_handles;
pub mod uds_session_switch_runtime;
mod uds_shutdown;
mod uds_single_client;
mod uds_snapshots;
mod uds_socket;
mod uds_state_projection;
#[cfg(test)]
mod uds_state_projection_tests;
mod uds_swarm_control;
mod uds_sync;
#[cfg(test)]
mod uds_sync_tests;
pub(crate) mod uds_teardown_adapters;
pub mod uds_teardown_handles;
#[cfg(test)]
mod uds_thinking_1231_tests;
mod uds_tool_intercept;
pub mod uds_turn_accounting;
pub mod uds_wire;
mod uds_workflow_nudge;

use crate::application::configuration::dto::{ConfigSelection, ConfigSelectionRequest};
use config_flag::{extract_config_flag, strip_global_config_flag};
use std::path::PathBuf;

// Re-export public types for external consumers.

/// Re-export for test access to OpenAI import params struct.
pub use auth::auth_import::OpenAiImportParams;

/// Test-friendly OpenAI import with optional OAuth base URL override.
///
/// Wraps the internal `auth_import::import_openai` for BDD testing,
/// allowing injection of a mock OAuth server URL.
pub fn auth_import_openai(
    auth_json: &serde_json::Value,
    params: &OpenAiImportParams<'_>,
    stdout: &mut String,
    stderr: &mut String,
) -> Option<u32> {
    let mut out = auth::Output { stdout, stderr };
    auth::auth_import::import_openai(auth_json, params, &mut out)
}

// Re-export shared functions for backward compatibility.
pub use super::shared::merge_prompts;

/// Result of a CLI invocation, capturing stdout, stderr, and exit code.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Runtime context for CLI commands, allowing override of paths for testing.
pub type WebFetchToolFactory =
    fn(reqwest::Client, u32) -> std::sync::Arc<dyn crate::application::tools::ports::Tool>;

/// What the agent-control use cases are built over (#1936, #1939): the
/// launcher registry the spawn tool populates, the event stream their
/// compensation broadcasts on, the notification channel it posts passive
/// notes to, the lineage owner, the harness lifecycle cell a frozen
/// harness refuses new control commands from, the session's environment
/// registry, and the slots the built agent-control tools read their use
/// cases from. The interface hands it to composition's installer once those
/// tools exist; composition builds the graph and fills the slots.
#[derive(Clone)]
pub struct KillToolWiring {
    pub owner: crate::domain::ids::AgentUuid,
    pub registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub notify_tx: Option<crate::infrastructure::tools::subagent_registry::NotificationTx>,
    /// The harness lifecycle cell (#1938): a frozen harness refuses new
    /// control commands; shared with the spawn tool and the teardown graph.
    pub harness_lifecycle: crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle,
    /// The session-scoped environment registry the spawn tool commits
    /// members to; the environment control is composed over it.
    pub environment_registry: crate::domain::environment_registry::EnvironmentRegistry,
    /// Where the built agent-control tools read their composed use cases.
    pub slots: crate::infrastructure::tools::environment_member_shutdown::TerminationSlots,
}

/// Composition's installer of the agent-control use cases — the `agent_cmd
/// kill` tool, the environment member shutdown and the swarm member
/// termination over one shared graph, the spawn tool's launch lifecycle
/// and the environment control — injected through the CLI context like
/// the web-fetch factory and the teardown handles builder. `true` when
/// every slot was empty and took its owner: one set per harness.
pub type KillToolBuilder = fn(KillToolWiring) -> bool;

/// Composition's builder of the handles one dispatch loop holds on the
/// subagent teardown capability (#1935, #1938), injected through the CLI
/// context; the loop hands over its runtime inputs and holds only the
/// use-case and controller handles back, never the graph between them.
pub type TeardownHandlesBuilder =
    fn(uds_teardown_handles::TeardownLoopInputs) -> uds_teardown_handles::TeardownHandles;

/// Composition's builder of the handles one loop holds on the sessions
/// capability (#1970): the session store and the saved-session queries,
/// composed over the loop's base directory. Injected through the CLI
/// context; the interface never constructs a store or a use case.
pub type SessionHandlesBuilder =
    fn(uds_session_handles::SessionLoopInputs) -> uds_session_handles::SessionHandles;

/// Composition's builder of the handles one run holds on retained context
/// (D9 #1978): the retention store, the recall use case and the pruning
/// policy's writer/reader, composed over the run's base directory.
/// Injected through the CLI context; the interface never constructs the
/// store or the recall graph.
pub type RetentionHandlesBuilder = fn(&std::path::Path) -> retention_handles::RetentionHandles;

/// Composition's builder of the fresh user-chat identity generator (D7
/// #1976): the one source of a fresh key, for the startup identity of an
/// unnamed chat run. The fresh-session transaction holds its own injected
/// handle on the same generator; the interface never generates a key.
pub type FreshSessionIdentityBuilder =
    fn() -> std::sync::Arc<dyn crate::application::sessions::ports::FreshSessionIdentityGenerator>;

/// Composition's builder of the catalogue handles (#1845): the controllers
/// a dispatch loop answers the catalogue commands through, over the loop's
/// base directory and, for an agent run, its reloadable configuration
/// (#1849). Injected through the CLI context; the interface never
/// constructs a catalogue use case.
pub type CatalogueHandlesBuilder = fn(
    &std::path::Path,
    Option<&catalogue_handles::RuntimeConfigurationInputs>,
) -> catalogue_handles::CatalogueHandles;

/// Composition's provider-runtime builder (#1849), the one type startup
/// and reload share: injected through the CLI context; startup calls it and
/// hands it on to the reload inputs, the interface never composes a
/// provider.
pub type ProviderRuntimeBuilder =
    crate::infrastructure::runtime_configuration::ProviderRuntimeBuilder;

/// Composition's builder of the tool-policy persistence hook (#1849): the
/// durable `set_tool_policy … persist` writer over the run's config file.
/// Injected through the CLI context; the agent build installs it on the
/// loop, the interface never constructs the writer.
pub type ToolPolicyPersistenceBuilder = fn(
    &std::path::Path,
    &crate::application::configuration::dto::ConfigSources,
)
    -> crate::application::agent_loop::ToolPolicyPersistence;

/// Composition's builder of the container-config handles (#2024 S4a,
/// S4c): launch policy over the run's own configuration selection, so
/// `container: true` resolves against the working directory's trusted
/// overlay, and the discovery query `get_container_configs` and the spawn
/// description's roster read the same layers. Injected through the CLI
/// context; the interface never composes them or reads a container config
/// itself.
pub type ContainerConfigHandlesBuilder =
    fn(&std::path::Path, &ConfigSelection) -> container_config_handles::ContainerConfigHandles;

/// Composition's builder of the configuration handles (#1966, #2024):
/// which files a run loads, the effective merge, and the one safe write
/// path. Injected through the CLI context; the interface never probes or
/// writes a config file itself.
pub type ConfigurationHandlesBuilder = fn(
    &configuration_handles::ConfigurationEnvironment,
) -> configuration_handles::ConfigurationHandles;

/// Composition's builder of the admission-operation handles (#2024 S3):
/// inspect/reset the authority, install/uninstall its service, and the
/// startup negotiation decision. Injected through the CLI context; the
/// interface never opens the admin socket or runs `systemctl` itself.
pub type AdmissionHandlesBuilder = fn() -> admission_handles::AdmissionHandles;

#[derive(Debug, Clone, Default)]
pub struct CliContext {
    /// Override for the base directory (default: ~/.quecto).
    pub base_dir: Option<PathBuf>,
    /// Explicit `--config` override; otherwise `./config.json` in the working
    /// directory, then `<base_dir>/config.json` (#1966).
    pub config_path: Option<PathBuf>,
    /// Pre-loaded stdin data for testing interactive commands.
    pub stdin_data: Option<String>,
    /// Whether active stdin is interactive, when known.
    pub stdin_is_tty: Option<bool>,
    /// Whether commands are being delegated by the setup/configuration REPL.
    pub is_repl: bool,
    /// Override OAuth base URL for testing (e.g. wiremock URI).
    pub oauth_base_url: Option<String>,
    /// Override process current working directory for hermetic tests.
    pub cwd: Option<PathBuf>,
    /// Opaque outer-layer constructor for the optional web-fetch graph.
    pub web_fetch_tool_factory: Option<WebFetchToolFactory>,
    /// Composition's subagent teardown handles builder (#1935). Supplied by
    /// the binary's `main` through [`run`]'s [`CliComposition`]; a launched
    /// child (one started with `--parent-control`) refuses to start without
    /// it.
    pub teardown_graph: Option<TeardownHandlesBuilder>,
    /// Composition's builder of the `agent_cmd kill` owner (#1936). Without
    /// it `kill` is unavailable: the interface never composes a lifecycle.
    pub kill_tool: Option<crate::interface::cli::KillToolBuilder>,
    /// Composition's sessions handles builder (#1970). Supplied by the
    /// binary's `main` through [`run`]'s [`CliComposition`]; an agent run
    /// refuses to start without it, since the interface never constructs
    /// a session store.
    pub sessions: Option<SessionHandlesBuilder>,
    /// Composition's retained-context handles builder (#1978). Supplied by
    /// the binary's `main` through [`run`]'s [`CliComposition`]; an agent
    /// run refuses to start without it, since the interface never
    /// constructs the retention store or the recall graph.
    pub retention: Option<RetentionHandlesBuilder>,
    /// Composition's fresh-identity generator builder (#1976). Supplied by
    /// the binary's `main` through [`run`]'s [`CliComposition`]; an unnamed
    /// chat run refuses to start without it.
    pub fresh_session_identity: Option<FreshSessionIdentityBuilder>,
    /// Composition's configuration handles builder (#1966, #2024).
    /// Supplied by the binary's `main` through [`run`]'s
    /// [`CliComposition`]; any command that loads or writes configuration
    /// refuses to run without it.
    pub configuration: Option<ConfigurationHandlesBuilder>,
    /// Composition's admission-operation handles builder (#2024 S3).
    /// Supplied by the binary's `main`; the `admission-broker` command
    /// refuses to run without it, since the interface never opens the admin
    /// socket or runs `systemctl` itself.
    pub admission: Option<AdmissionHandlesBuilder>,
    /// Composition's catalogue handles builder (#1845). Supplied by the
    /// binary's `main` through [`run`]'s [`CliComposition`]; an agent run
    /// refuses to start without it.
    pub catalogue: Option<CatalogueHandlesBuilder>,
    /// Composition's provider-runtime builder (#1849). Supplied by the
    /// binary's `main` through [`run`]'s [`CliComposition`]; an agent run
    /// refuses to start without it.
    pub provider_runtime: Option<ProviderRuntimeBuilder>,
    /// Composition's tool-policy persistence builder (#1849). Supplied by
    /// the binary's `main` through [`run`]'s [`CliComposition`]; an agent
    /// run refuses to start without it.
    pub tool_policy_persistence: Option<ToolPolicyPersistenceBuilder>,
    /// Composition's container-config handles builder (#2024 S4a, S4c),
    /// from the binary's `main` through [`run`]'s [`CliComposition`]; an
    /// agent run's spawn tool selects container configs through it and its
    /// agent_cmd lists them.
    pub container_configs: Option<ContainerConfigHandlesBuilder>,
    /// Composition's container-doctor builder (#2024 S4b); `quecto
    /// container doctor` refuses to run without it.
    pub container_doctor: Option<ContainerDoctorBuilder>,
    /// Composition's durable environment registry builder (#2024 S4d):
    /// the registry an agent run's spawn tool commits to, restored from
    /// and journalled through the base directory. `None` (unit rigs)
    /// leaves the run with an in-memory registry.
    pub environment_registry: Option<EnvironmentRegistryBuilder>,
    /// Composition's container inventory builder (#2024 S4d); `quecto
    /// container ls|kill|gc` refuse to run without it.
    pub container_inventory: Option<ContainerInventoryBuilder>,
    /// Composition's standard-container builders (#2024 S4e); `quecto
    /// container init|status` refuse to run without them.
    pub container_init: Option<ContainerInitBuilder>,
    pub container_status: Option<ContainerStatusBuilder>,
}

impl CliContext {
    /// The composed configuration handles (#2024). `prompt_for_trust`
    /// lets an unrecorded overlay be offered to an interactive user; only
    /// an agent run started from a terminal asks for it.
    /// The composed admission-operation handles (#2024 S3).
    pub(crate) fn admission_handles(&self) -> Result<admission_handles::AdmissionHandles, String> {
        let Some(build) = self.admission else {
            return Err("admission capability not composed".to_string());
        };
        Ok(build())
    }

    pub(crate) fn configuration_handles(
        &self,
        prompt_for_trust: bool,
    ) -> Result<configuration_handles::ConfigurationHandles, String> {
        let Some(build) = self.configuration else {
            return Err("configuration capability not composed".to_string());
        };
        Ok(build(&configuration_handles::ConfigurationEnvironment {
            base_dir: self.base_dir(),
            prompt_for_trust,
        }))
    }

    /// Select the config layers (#1966, #2024): an explicit override
    /// replaces everything; otherwise `<base_dir>/config.json` with the
    /// working directory's `.quecto/config.json` as the overlay candidate.
    /// The working directory is the one handed in (`run` supplies the
    /// process's; rigs supply a hermetic one); without one no overlay is
    /// discovered.
    pub(crate) fn config_selection(&self) -> Result<ConfigSelection, String> {
        // Both directories are compared by identity by the use case; the
        // filesystem's canonical form (symlinked homes, relative base dirs)
        // is what makes that comparison honest.
        let canonical = |path: PathBuf| std::fs::canonicalize(&path).unwrap_or(path);
        Ok(self
            .configuration_handles(false)?
            .select
            .execute(ConfigSelectionRequest {
                explicit: self.config_path.clone(),
                working_directory: self.cwd.clone().map(canonical),
                global: canonical(self.base_dir()).join("config.json"),
            }))
    }

    /// Resolve the base directory: explicit override > QUECTO_BASE_DIR env var > default.
    pub(crate) fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .or_else(|| std::env::var("QUECTO_BASE_DIR").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".quecto")))
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    }
}

/// The outer-owned graph builders a binary's `main` hands to the CLI: the
/// composition layer constructs them, the interface only threads them into
/// its [`CliContext`].
#[derive(Debug, Clone, Copy)]
pub struct CliComposition {
    pub web_fetch_tool_factory: WebFetchToolFactory,
    pub teardown_graph: TeardownHandlesBuilder,
    pub kill_tool: crate::interface::cli::KillToolBuilder,
    pub sessions: SessionHandlesBuilder,
    pub retention: RetentionHandlesBuilder,
    pub fresh_session_identity: FreshSessionIdentityBuilder,
    pub configuration: ConfigurationHandlesBuilder,
    pub admission: AdmissionHandlesBuilder,
    pub catalogue: CatalogueHandlesBuilder,
    pub provider_runtime: ProviderRuntimeBuilder,
    pub tool_policy_persistence: ToolPolicyPersistenceBuilder,
    pub container_configs: ContainerConfigHandlesBuilder,
    pub container_doctor: ContainerDoctorBuilder,
    pub environment_registry: EnvironmentRegistryBuilder,
    pub container_inventory: ContainerInventoryBuilder,
    pub container_init: ContainerInitBuilder,
    pub container_status: ContainerStatusBuilder,
}

/// Run the CLI with the given args and the required outer-owned builders,
/// printing to real stdout/stderr. Returns the exit code.
pub fn run(args: Vec<String>, composition: CliComposition) -> i32 {
    let config_path = match extract_config_flag(&args) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let stdin_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let ctx = CliContext {
        config_path,
        cwd: std::env::current_dir().ok(),
        stdin_is_tty: Some(stdin_is_tty),
        web_fetch_tool_factory: Some(composition.web_fetch_tool_factory),
        teardown_graph: Some(composition.teardown_graph),
        kill_tool: Some(composition.kill_tool),
        sessions: Some(composition.sessions),
        retention: Some(composition.retention),
        fresh_session_identity: Some(composition.fresh_session_identity),
        configuration: Some(composition.configuration),
        admission: Some(composition.admission),
        catalogue: Some(composition.catalogue),
        provider_runtime: Some(composition.provider_runtime),
        tool_policy_persistence: Some(composition.tool_policy_persistence),
        container_configs: Some(composition.container_configs),
        container_doctor: Some(composition.container_doctor),
        environment_registry: Some(composition.environment_registry),
        container_inventory: Some(composition.container_inventory),
        container_init: Some(composition.container_init),
        container_status: Some(composition.container_status),
        ..Default::default()
    };

    // Enter the live REPL when no command remains after global options.
    // `run_with_output` intentionally uses captured input, so this decision must
    // happen here for config-only production invocations.
    if strip_global_config_flag(&args).len() < 2 {
        if let Some(error) = ctx
            .config_path
            .as_deref()
            .and_then(|path| selected_config_missing(path, true))
        {
            eprintln!("{error}");
            return 1;
        }
        return super::repl::run_repl(
            std::io::stdin().lock(),
            std::io::stdout(),
            stdin_is_tty,
            |args, reader| run_repl_command(&ctx, args, reader),
        );
    }

    let output = run_with_output(args, &ctx);
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    output.exit_code
}

/// Run the CLI with the given args and context, capturing all output for testing.
pub fn run_with_output(args: Vec<String>, ctx: &CliContext) -> CliOutput {
    // Merge --config from args into context if not already set.
    let merged_ctx;
    let ctx = if ctx.config_path.is_none() {
        match extract_config_flag(&args) {
            Ok(Some(path)) => {
                merged_ctx = CliContext {
                    config_path: Some(path),
                    ..ctx.clone()
                };
                &merged_ctx
            }
            Ok(None) => ctx,
            Err(error) => {
                return CliOutput {
                    stdout: String::new(),
                    stderr: format!("{error}\n"),
                    exit_code: 1,
                };
            }
        }
    } else {
        ctx
    };
    let args = strip_global_config_flag(&args);
    let mut stdout = String::new();
    let mut stderr = String::new();

    if args.len() < 2 {
        // No args → REPL mode. Delegate to run_repl_with_output with empty input
        // so the REPL exits immediately on EOF (consistent with piped empty input).
        return run_repl_with_output(ctx, &[], &[], false);
    }

    let exit_code = {
        match args[1].as_str() {
            "agent" => agent::cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "status" => commands::cmd_status(ctx, &mut stdout, &mut stderr),
            "config" => config_cmd::cmd_config(ctx, &args[2..], &mut stdout, &mut stderr),
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "models" => models::cmd_models(ctx, &args[2..], &mut stdout, &mut stderr),
            "admission-broker" => {
                admission_broker::cmd_admission_broker(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "container" => container::cmd_container(ctx, &args[2..], &mut stdout, &mut stderr),
            "help" | "--help" | "-h" => {
                help::help_text(&mut stdout);
                0
            }
            "version" | "--version" | "-v" => {
                help::version_text(&mut stdout);
                0
            }
            other => {
                stderr.push_str(&format!("Unknown command: {other}\n"));
                help::help_text(&mut stdout);
                1
            }
        }
    };

    CliOutput {
        stdout,
        stderr,
        exit_code,
    }
}

/// Run the setup/configuration REPL with captured output.
pub fn run_repl_with_output(
    ctx: &CliContext,
    args: &[String],
    input: &[u8],
    is_tty: bool,
) -> CliOutput {
    let mut output = Vec::new();
    if !args.is_empty() {
        return CliOutput {
            stdout: String::new(),
            stderr:
                "REPL agent flags are no longer supported; use `quecto agent` for agent operation\n"
                    .into(),
            exit_code: 1,
        };
    }
    let ctx = CliContext {
        stdin_is_tty: Some(is_tty),
        ..ctx.clone()
    };
    let exit_code = super::repl::run_repl(
        std::io::BufReader::new(input),
        &mut output,
        is_tty,
        |args, reader| run_repl_command(&ctx, args, reader),
    );
    CliOutput {
        stdout: String::from_utf8_lossy(&output).to_string(),
        stderr: String::new(),
        exit_code,
    }
}

fn run_repl_command(
    ctx: &CliContext,
    args: Vec<String>,
    reader: &mut dyn std::io::BufRead,
) -> (String, String, i32) {
    let ctx = CliContext {
        is_repl: true,
        ..ctx.clone()
    };
    let ctx = &ctx;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = match args.first().map(String::as_str) {
        Some("auth") => {
            auth::cmd_auth_with_reader(ctx, &args[1..], &mut stdout, &mut stderr, reader)
        }
        Some("status") => commands::cmd_status(ctx, &mut stdout, &mut stderr),
        Some("models") => models::cmd_models(ctx, &args[1..], &mut stdout, &mut stderr),
        _ => 1,
    };
    (stdout, stderr, code)
}

/// A selected config that must exist (an explicit `--config`, or the
/// working-directory file that was present at selection) but is missing
/// now: an error naming the path, never a fall-through to defaults.
pub(crate) fn selected_config_missing(
    config_path: &std::path::Path,
    must_exist: bool,
) -> Option<String> {
    (must_exist && !config_path.exists())
        .then(|| format!("config not found: {}", config_path.display()))
}

/// Wire syntax of a session name: the domain's named-session allowlist
/// (`-`, the ephemeral marker, is itself an admitted name).
pub(crate) fn is_valid_session_name(name: &str) -> bool {
    crate::domain::session_identity::SessionIdentity::is_valid_cli_name(name)
}

/// The harness runs a current-thread runtime whose blocking pool keeps a
/// resident thread on purpose: Tokio's `spawn_blocking` panics (and with
/// `panic = "abort"` kills the process) when it must create a thread and the
/// OS answers EAGAIN, but merely queues that task when at least one blocking
/// thread already exists. Under a full pid cgroup (the case that was killing
/// swarm containers) that is the difference between a stalled call and a
/// dead container. Other spawn failures (ENOMEM) still panic.
///
/// The keep-alive is pool-wide: every blocking thread this process ever
/// creates is retained (bounded by Tokio's default cap of 512), each holding
/// one pid for the process lifetime. That is the accepted price.
///
/// A thread is probed with `std` first so that a process starting inside an
/// already exhausted cgroup gets an ordinary error, not an abort.
pub(crate) fn build_tokio_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    std::thread::Builder::new()
        .name("quecto-thread-probe".into())
        .spawn(|| {})?
        .join()
        .map_err(|_| std::io::Error::other("thread probe panicked"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_keep_alive(std::time::Duration::from_secs(60 * 60 * 24 * 365))
        .build()?;
    runtime.block_on(async {
        let _ = tokio::task::spawn_blocking(|| {}).await;
    });
    Ok(runtime)
}

#[cfg(test)]
#[path = "leading_config_dispatch_tests.rs"]
mod leading_config_dispatch_tests;

#[cfg(test)]
mod mod_tests;

mod swarm_composition;
