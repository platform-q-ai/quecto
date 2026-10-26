pub mod board_server;
pub mod controllers;
/// Drive an in-process harness through the termination-signal path (#1938)
/// without signalling the test process.
#[cfg(any(test, feature = "test-support"))]
pub use crate::interface::uds::framing::uds_shutdown::test_support::deliver_termination_signal;
pub use controllers::container::{
    ContainerDoctorBuilder, ContainerInitBuilder, ContainerInventoryBuilder,
    ContainerStatusBuilder, EnvironmentRegistryBuilder,
};
use controllers::{
    admission_broker, admission_handles, auth, board, catalogue_handles, claude_member,
    configuration_handles, container_config_handles, retention_handles, run_end_fleet,
    swarm_board_handles,
};

pub(in crate::interface) mod uds_lifecycle;
pub mod uds_session;
#[cfg(test)]
mod uds_session_admission_tests;
pub mod uds_session_switch_runtime;
pub(in crate::interface) mod uds_snapshots;
pub(in crate::interface) mod uds_state_projection;
#[cfg(test)]
mod uds_state_projection_tests;
#[cfg(test)]
mod uds_thinking_1231_tests;

use crate::application::configuration::dto::{ConfigSelection, ConfigSelectionRequest};
use controllers::config_flag::{extract_config_flag, strip_global_config_flag};
use std::path::PathBuf;

/// Result of a CLI invocation, capturing stdout, stderr, and exit code.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Runtime context for CLI commands, allowing override of paths for testing.
pub type WebFetchToolFactory = fn(
    crate::infrastructure::http::web_fetch::WebFetchClientRecipe,
    u32,
) -> std::sync::Arc<dyn crate::application::tools::ports::Tool>;

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
    pub owner: crate::domain::identity::ids::AgentUuid,
    pub registry: crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub notify_tx: Option<crate::infrastructure::tools::agents::subagent_registry::NotificationTx>,
    /// The harness lifecycle cell (#1938): a frozen harness refuses new
    /// control commands; shared with the spawn tool and the teardown graph.
    pub harness_lifecycle:
        crate::infrastructure::tools::agents::harness_lifecycle::SharedHarnessLifecycle,
    /// The session-scoped environment registry the spawn tool commits
    /// members to; the environment control is composed over it.
    pub environment_registry:
        crate::domain::environments::entities::environment_registry::EnvironmentRegistry,
    /// Where the built agent-control tools read their composed use cases.
    pub slots: crate::infrastructure::tools::agents::environment_member_shutdown::TerminationSlots,
    /// The base directory this harness's children share (#2192): where an
    /// ended child's crash record and transcript are read.
    pub base_dir: std::path::PathBuf,
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
    fn(
        crate::interface::uds::controllers::uds_teardown_handles::TeardownLoopInputs,
    ) -> crate::interface::uds::controllers::uds_teardown_handles::TeardownHandles;

/// Composition's builder of the handles one loop holds on the sessions
/// capability (#1970): the session store and the saved-session queries,
/// composed over the loop's base directory. Injected through the CLI
/// context; the interface never constructs a store or a use case.
pub type SessionHandlesBuilder =
    fn(
        crate::interface::uds::controllers::uds_session_handles::SessionLoopInputs,
    ) -> crate::interface::uds::controllers::uds_session_handles::SessionHandles;

/// Composition's builder of the handles one run holds on retained context
/// (D9 #1978): the retention store, the recall use case and the pruning
/// policy's writer/reader, composed over the run's base directory.
/// Injected through the CLI context; the interface never constructs the
/// store or the recall graph.
pub type RetentionHandlesBuilder = fn(&std::path::Path) -> retention_handles::RetentionHandles;

/// Composition's builder of a UDS loop's session tracker for a model
/// (#1909, #1855): it holds the agent-turn use cases composition builds,
/// resume-after-a-provider-failure and inspect-session-usage.
pub type TurnSessionBuilder = fn(String) -> uds_session::AgentSession;

/// Composition's builder of a trusted root parent's turn-outcome runtime
/// (#2708); the interface never constructs the use case.
pub type ParentOutcomeBuilder = fn() -> std::sync::Arc<
    crate::application::agent_turn::use_cases::parent_outcome::ParentOutcomeRuntime,
>;

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
    crate::infrastructure::config::runtime_configuration::ProviderRuntimeBuilder;

/// Composition's builder of the tool-policy persistence hook (#1849): the
/// durable `set_tool_policy … persist` writer over the run's config file.
/// Injected through the CLI context; the agent build installs it on the
/// loop, the interface never constructs the writer.
pub type ToolPolicyPersistenceBuilder =
    fn(
        &std::path::Path,
        &crate::application::configuration::dto::ConfigSources,
    ) -> crate::application::agent_turn::agent_loop::ToolPolicyPersistence;

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
    /// Composition's turn-session builder (#1909, #1855); a UDS loop
    /// refuses to start without it.
    pub turn_session: Option<TurnSessionBuilder>,
    /// Composition's turn-outcome runtime builder (#2708); without it no
    /// root is guarded.
    pub parent_outcome: Option<ParentOutcomeBuilder>,
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
    /// Composition's auth handles builder (#1839); `auth login` needs it.
    pub auth: Option<auth::AuthHandlesBuilder>,
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
    /// Composition's run-end fleet builder (#2206): a one-shot run settles
    /// the children it launched when it ends in an orderly way. Without it
    /// the run returns and its children run their parent-loss shutdown.
    pub run_end_fleet: Option<run_end_fleet::RunEndFleetBuilder>,
    /// Composition's claude-code member builder (#2287); an agent run with
    /// `--backend claude-code` refuses to start without it.
    pub claude_member: Option<claude_member::ClaudeMemberHandlesBuilder>,
    /// Composition's coordination-board handles builder (#2270, #2278):
    /// an agent run gives every `SwarmContext` one board built through
    /// it. A process launched as a swarm member refuses to start without
    /// it, since the interface never constructs the board.
    pub swarm_board: Option<swarm_board_handles::SwarmBoardHandlesBuilder>,
    /// Composition's `board_wire` (#2279): how the board's structured ops
    /// read a member's text and write their answers. The board is built
    /// only with it.
    pub swarm_board_wire: Option<swarm_board_handles::BoardWire>,
    /// Composition's `board_op_log` (#2303): the session's event log the
    /// board records its `swarm_op`s in, while the event log is on.
    pub swarm_board_log: Option<swarm_board_handles::SwarmBoardOpLogBuilder>,
    /// Composition's project board builder (#2477); `quecto board`
    /// refuses to run without it.
    pub project_board: Option<board::ProjectBoardHandlesBuilder>,
    /// Composition's builder of the agent `board` tool's recording (epic
    /// #2481 T3), bound to the session's event log; without it the tool
    /// records nothing.
    pub project_board_recording: Option<board::BoardToolRecordingBuilder>,
    /// Composition's builder of `quecto board`'s recording (epic #2481
    /// T3), into the day's `board-cli` log; without it the CLI records
    /// nothing.
    pub board_cli_recording: Option<board::CliRecordingBuilder>,
    /// Composition's board server builder (epic #2705 B4).
    pub board_server: Option<board_server::BoardServerBuilder>,
    /// Offline export graph, independent of the listener/token graph.
    pub board_history: Option<board_server::history::BoardHistoryBuilder>,
    pub board_history_import: Option<board_server::history::BoardHistoryImportBuilder>,
    pub board_backup: Option<board_server::backup::BoardBackupBuilder>,
    pub board_fence: Option<board_server::fence::BoardFenceBuilder>,
    /// Composition's builder of `quecto board watch` (epic #2705 B10).
    pub board_watch: Option<board::watch::BoardWatchBuilder>,
    /// Output reaches the real terminal (`run`, not `run_with_output`): a
    /// one-shot run writes its answer out before it settles its children.
    pub live_output: bool,
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
    pub turn_session: TurnSessionBuilder,
    pub parent_outcome: ParentOutcomeBuilder,
    pub retention: RetentionHandlesBuilder,
    pub fresh_session_identity: FreshSessionIdentityBuilder,
    pub configuration: ConfigurationHandlesBuilder,
    pub admission: AdmissionHandlesBuilder,
    pub auth: auth::AuthHandlesBuilder,
    pub catalogue: CatalogueHandlesBuilder,
    pub provider_runtime: ProviderRuntimeBuilder,
    pub tool_policy_persistence: ToolPolicyPersistenceBuilder,
    pub container_configs: ContainerConfigHandlesBuilder,
    pub container_doctor: ContainerDoctorBuilder,
    pub environment_registry: EnvironmentRegistryBuilder,
    pub container_inventory: ContainerInventoryBuilder,
    pub container_init: ContainerInitBuilder,
    pub container_status: ContainerStatusBuilder,
    pub run_end_fleet: run_end_fleet::RunEndFleetBuilder,
    /// Composition's coordination-board handles builder (#2270), which
    /// `run` carries onto `CliContext` for `SwarmContext` (#2278).
    pub swarm_board: swarm_board_handles::SwarmBoardHandlesBuilder,
    /// Composition's `board_wire` (#2279).
    pub swarm_board_wire: swarm_board_handles::BoardWire,
    /// Composition's `board_op_log` (#2303, #2278).
    pub swarm_board_log: swarm_board_handles::SwarmBoardOpLogBuilder,
    pub claude_member: claude_member::ClaudeMemberHandlesBuilder,
    /// Composition's project board builder (#2477).
    pub project_board: board::ProjectBoardHandlesBuilder,
    /// Composition's `session_board_recording_decided` (epic #2481 T3).
    pub project_board_recording: board::BoardToolRecordingBuilder,
    /// Composition's `cli_board_recording` (epic #2481 T3).
    pub board_cli_recording: board::CliRecordingBuilder,
    /// Composition's `build_board_server` (epic #2705 B4).
    pub board_server: board_server::BoardServerBuilder,
    pub board_history: board_server::history::BoardHistoryBuilder,
    pub board_history_import: board_server::history::BoardHistoryImportBuilder,
    pub board_backup: board_server::backup::BoardBackupBuilder,
    pub board_fence: board_server::fence::BoardFenceBuilder,
    /// Composition's `build_board_watch` (epic #2705 B10).
    pub board_watch: board::watch::BoardWatchBuilder,
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
    let ctx = composed_context(composition, config_path, stdin_is_tty);

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

/// The context a real run's commands see: every builder `main` composed,
/// the process's working directory and stdin, and live output.
fn composed_context(
    composition: CliComposition,
    config_path: Option<PathBuf>,
    stdin_is_tty: bool,
) -> CliContext {
    CliContext {
        config_path,
        cwd: std::env::current_dir().ok(),
        stdin_is_tty: Some(stdin_is_tty),
        web_fetch_tool_factory: Some(composition.web_fetch_tool_factory),
        teardown_graph: Some(composition.teardown_graph),
        kill_tool: Some(composition.kill_tool),
        sessions: Some(composition.sessions),
        turn_session: Some(composition.turn_session),
        parent_outcome: Some(composition.parent_outcome),
        retention: Some(composition.retention),
        fresh_session_identity: Some(composition.fresh_session_identity),
        configuration: Some(composition.configuration),
        admission: Some(composition.admission),
        auth: Some(composition.auth),
        catalogue: Some(composition.catalogue),
        provider_runtime: Some(composition.provider_runtime),
        tool_policy_persistence: Some(composition.tool_policy_persistence),
        container_configs: Some(composition.container_configs),
        container_doctor: Some(composition.container_doctor),
        environment_registry: Some(composition.environment_registry),
        container_inventory: Some(composition.container_inventory),
        container_init: Some(composition.container_init),
        container_status: Some(composition.container_status),
        run_end_fleet: Some(composition.run_end_fleet),
        claude_member: Some(composition.claude_member),
        project_board: Some(composition.project_board),
        project_board_recording: Some(composition.project_board_recording),
        board_cli_recording: Some(composition.board_cli_recording),
        board_server: Some(composition.board_server),
        board_history: Some(composition.board_history),
        board_history_import: Some(composition.board_history_import),
        board_backup: Some(composition.board_backup),
        board_fence: Some(composition.board_fence),
        board_watch: Some(composition.board_watch),
        swarm_board: Some(composition.swarm_board),
        swarm_board_wire: Some(composition.swarm_board_wire),
        swarm_board_log: Some(composition.swarm_board_log),
        live_output: true,
        ..Default::default()
    }
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
            "agent" => controllers::agent::cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "status" => controllers::commands::cmd_status(ctx, &mut stdout, &mut stderr),
            "config" => {
                controllers::config_cmd::cmd_config(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "models" => controllers::models::cmd_models(ctx, &args[2..], &mut stdout, &mut stderr),
            "admission-broker" => {
                admission_broker::cmd_admission_broker(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "container" => {
                controllers::container::cmd_container(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "board" => board::cmd_board(ctx, &args[2..], &mut stdout, &mut stderr),
            "board-server" => {
                board_server::cmd_board_server(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "help" | "--help" | "-h" => {
                controllers::help::help_text(&mut stdout);
                0
            }
            "version" | "--version" | "-v" => {
                controllers::help::version_text(&mut stdout);
                0
            }
            other => {
                stderr.push_str(&format!("Unknown command: {other}\n"));
                controllers::help::help_text(&mut stdout);
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
        Some("status") => controllers::commands::cmd_status(ctx, &mut stdout, &mut stderr),
        Some("models") => {
            controllers::models::cmd_models(ctx, &args[1..], &mut stdout, &mut stderr)
        }
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
    crate::domain::sessions::entities::session_identity::SessionIdentity::is_valid_cli_name(name)
}

#[cfg(test)]
#[path = "leading_config_dispatch_tests.rs"]
mod leading_config_dispatch_tests;

#[cfg(test)]
mod mod_tests;

/// A swarm member's execution ports on its agent loop, for the tests that
/// drive the real loop through the CLI's own wiring (#2339 review L1).
#[cfg(any(test, feature = "test-support"))]
pub use controllers::swarm_composition::wire_agent as wire_swarm_agent;
