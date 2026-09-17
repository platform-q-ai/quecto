mod admission_broker;
mod agent;
mod auth;
pub mod catalogue_handles;
mod commands;
mod config_flag;
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
pub mod uds_execution_state;
mod uds_progress_forward;

#[cfg(any(test, feature = "test-support"))]
pub fn live_execution_state_for_events(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}

/// Test-support (#1679 P4): the `get_state` projection of a process whose
/// admission activity comes from `source`, polled the way a supervisor does.
#[cfg(any(test, feature = "test-support"))]
pub struct AdmissionStateProbe {
    execution: uds_execution_state::ExecutionState,
    session: protocol::SessionState,
}

#[cfg(any(test, feature = "test-support"))]
impl AdmissionStateProbe {
    pub fn new(
        source: std::sync::Arc<dyn crate::application::ports::AdmissionObservation>,
    ) -> Self {
        let mut execution = uds_execution_state::ExecutionState::default();
        execution.set_admission_source(source);
        Self {
            execution,
            session: protocol::SessionState {
                model: "probe".into(),
                generation: 0,
                is_streaming: false,
                session_key: "probe".into(),
                message_count: 0,
                pending_message_count: 0,
                max_context_tokens: 0,
                effort: None,
                effort_levels: vec![],
                workflow: None,
                execution: None,
                sync: 0,
                control_receipts: vec![],
                automatic_turns_suspended: false,
                repeated_failure_notifications: Default::default(),
            },
        }
    }
    pub fn start_run(&mut self) {
        self.execution.start_run();
    }
    pub fn finish_run(&mut self) {
        self.execution.finish_run();
    }
    /// The slim `get_state` response data for an optional `since` cursor.
    pub fn poll(&mut self, since: Option<u64>) -> serde_json::Value {
        self.session.generation = self.execution.observe_visible_revisions(0, 0);
        self.session.execution = Some(self.execution.snapshot());
        uds_state_projection::slim_state_response_data(&self.session, since)
    }
}

/// Test-support (#1679 P4): the production `admission_state_changed` hook.
#[cfg(any(test, feature = "test-support"))]
pub fn admission_broadcast_hook(
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
) -> crate::infrastructure::admission::ActivityHook {
    uds_admission_projection::admission_event_hook(broadcast_tx)
}

#[cfg(any(test, feature = "test-support"))]
pub fn completed_live_execution_state(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    state.finish_run();
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}
/// Test-support: run a sequence of progress events through the mid-turn
/// publish path (`publish_turn_progress`) against a fresh conversation
/// snapshot, returning every event line emitted to the sink. BDD scenarios use
/// this to pin that mid-turn `TurnCompleted` events emit `ledger_advanced`
/// hints (the child-progress-freeze fix, 2026-07-29).
#[cfg(any(test, feature = "test-support"))]
pub async fn ledger_hint_lines_for_turn_events(
    events: &[crate::domain::agent::AgentProgressEvent],
    session: &crate::application::sessions::active_session::ActiveSessionHandle,
) -> Vec<serde_json::Value> {
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = uds_cancel::EventSink::writer(&mut buf);
    for event in events {
        uds_cancel::publish_turn_progress(event, Some(session), &mut sink).await;
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Test-support: run one raw command line through the FULL per-connection
/// reader dispatch (`uds_reader_dispatch::dispatch`) against a snapshot with
/// one committed message. Returns `(served_inline, response)`: `served_inline`
/// is true when the command was answered on the reader task and never queued
/// behind the dispatch loop. Covers the TUI's DIRECT child-feed path — a
/// plain `sync` with no `agent_id` on the child's own socket — which is
/// served by the child-local `uds_sync` fast path even while the child's
/// dispatch loop is occupied (PR #1307 review).
#[cfg(any(test, feature = "test-support"))]
pub async fn busy_reader_dispatch(
    line: &str,
    session: &uds_session_handles::SessionReadHandles,
) -> (bool, Option<serde_json::Value>) {
    let _ = session
        .active_session
        .write()
        .await
        .publish(&[crate::domain::message::Message::user("committed")]);
    let clients = uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    uds_ext_protocol::register_client_writer(&clients, 1, tx);
    // The dispatch-loop channel: anything landing here would have queued
    // behind an in-flight parent/child turn.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(8);
    uds_reader_dispatch::dispatch(uds_reader_dispatch::ReaderDispatchCtx {
        line: line.to_string(),
        session,
        registry: &clients,
        subagent_registry: &None,
        fleet: None,
        client_id: 1,
        cmd_tx: &cmd_tx,
        cancel_handle: &std::sync::Arc::new(std::sync::Mutex::new(uds_cancel::CancelSlot::Idle)),
        turn_control: &uds_cancel::TurnControl::default(),
    })
    .await;
    let served_inline = cmd_rx.try_recv().is_err();
    if !served_inline {
        return (false, None);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}

#[cfg(any(test, feature = "test-support"))]
mod uds_busy_test_support;
#[cfg(any(test, feature = "test-support"))]
pub use uds_busy_test_support::{busy_reader_intercept, busy_reader_intercept_with_fleet};
/// Drive an in-process harness through the termination-signal path (#1938)
/// without signalling the test process.
#[cfg(any(test, feature = "test-support"))]
pub use uds_shutdown::test_support::deliver_termination_signal;

pub mod retention_handles;
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

/// Composition's provider-runtime builder (#1849): composes and publishes
/// the provider runtime for a base directory and returns its routing
/// provider. Injected through the CLI context; startup calls it (reload
/// goes through the catalogue handles), the interface never composes a
/// provider.
pub type ProviderRuntimeBuilder =
    fn(
        &crate::infrastructure::config::Config,
        &std::path::Path,
        &reqwest::Client,
    )
        -> Result<std::sync::Arc<dyn crate::application::providers::ports::LlmProvider>, String>;

/// Composition's builder of the configuration-selection use case (#1966):
/// which one file a run loads its configuration from. Injected through the
/// CLI context; the interface never probes the filesystem for a config.
pub type ConfigSelectionBuilder =
    fn() -> std::sync::Arc<crate::application::configuration::use_cases::SelectConfig>;

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
    /// Composition's configuration-selection builder (#1966). Supplied by
    /// the binary's `main` through [`run`]'s [`CliComposition`]; any command
    /// that loads configuration refuses to run without it.
    pub config_selection: Option<ConfigSelectionBuilder>,
    /// Composition's catalogue handles builder (#1845). Supplied by the
    /// binary's `main` through [`run`]'s [`CliComposition`]; an agent run
    /// refuses to start without it.
    pub catalogue: Option<CatalogueHandlesBuilder>,
    /// Composition's provider-runtime builder (#1849). Supplied by the
    /// binary's `main` through [`run`]'s [`CliComposition`]; an agent run
    /// refuses to start without it.
    pub provider_runtime: Option<ProviderRuntimeBuilder>,
}

impl CliContext {
    /// Select the config file (#1966): explicit override > `./config.json` in
    /// the working directory > `<base_dir>/config.json`. A local file that is
    /// present but unusable is an error, never a fallback. The working
    /// directory is the one handed in (`run` supplies the process's; rigs
    /// supply a hermetic one); without one nothing local is discovered.
    pub(crate) fn config_selection(&self) -> Result<ConfigSelection, String> {
        let Some(select_config) = self.config_selection else {
            return Err("configuration selection capability not composed".to_string());
        };
        select_config()
            .execute(ConfigSelectionRequest {
                explicit: self.config_path.clone(),
                working_directory: self.cwd.clone(),
                global: self.base_dir().join("config.json"),
            })
            .map_err(|error| error.to_string())
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
    pub config_selection: ConfigSelectionBuilder,
    pub catalogue: CatalogueHandlesBuilder,
    pub provider_runtime: ProviderRuntimeBuilder,
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
        config_selection: Some(composition.config_selection),
        catalogue: Some(composition.catalogue),
        provider_runtime: Some(composition.provider_runtime),
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
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "models" => models::cmd_models(ctx, &args[2..], &mut stdout, &mut stderr),
            "admission-broker" => {
                admission_broker::cmd_admission_broker(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "help" | "--help" | "-h" => {
                help_text(&mut stdout);
                0
            }
            "version" | "--version" | "-v" => {
                version_text(&mut stdout);
                0
            }
            other => {
                stderr.push_str(&format!("Unknown command: {other}\n"));
                help_text(&mut stdout);
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

fn version_text(out: &mut String) {
    out.push_str(&format!("quecto {}\n", env!("CARGO_PKG_VERSION")));
}

fn help_text(out: &mut String) {
    out.push_str(&format!(
        "quecto - Personal AI Assistant v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("\nUsage: quecto [command]\n");
    out.push_str("\nWhen run with no arguments, quecto enters the setup and configuration REPL.\n");
    out.push_str("  Use `quecto agent` or `quecto-tui` for agent operation.\n");
    out.push_str("\nGlobal options:\n");
    out.push_str("  --config <path>  Override config file path (default: ./config.json in the\n");
    out.push_str("                   working directory, else <base_dir>/config.json)\n");
    out.push_str("\nCommands:\n");
    out.push_str("  admission-broker run|status|reset\n");
    out.push_str("              Shared inference admission authority (requires an `admission` config section)\n");
    out.push_str("  agent       Run a one-shot agent session (-m required)\n");
    out.push_str("              Options: -s <name>  Named session (default: \"default\")\n");
    out.push_str("                       --no-session  Ephemeral mode — nothing saved or loaded\n");
    out.push_str("                       --model <m>   Override model\n");
    out.push_str("                       --system <p>  System prompt\n");
    out.push_str("                       --max-iterations <n>  Max tool iterations\n");
    out.push_str("                       --max-time <s>  Wall-clock timeout in seconds\n");
    out.push_str(
        "                       --mode uds    framed JSON agent mode via Unix domain socket\n",
    );
    out.push_str(
        "                       --socket <path>  Socket path for --mode uds (default: auto in tmpdir)\n",
    );
    out.push_str(
        "                       --persist     Keep a top-level UDS agent alive after its last client disconnects; SIGTERM/SIGINT or a protocol shutdown then tears its subagents down over the protocol before it exits (harness-spawned subagents are lifetime-bound to their launcher instead)\n",
    );
    out.push_str(
        "                       --effort <level>  Effort level for 4.6 models (low/medium/high/max)\n",
    );
    out.push_str(
        "                       --disable-tool <name>  Disable/hide a tool and deny re-registration (repeatable)\n",
    );
    out.push_str("  auth        Manage authentication (login, logout, status)\n");
    out.push_str("  models      Manage runtime model registry (discover)\n");
    out.push_str("  status      Show status\n");
    out.push_str("  help        Show this help\n");
    out.push_str("  version     Show version information\n");
}

#[cfg(test)]
#[path = "leading_config_dispatch_tests.rs"]
mod leading_config_dispatch_tests;

#[cfg(test)]
mod mod_tests;

mod swarm_composition;
