use super::uds::uds_dispatch_session;
use super::uds::{DispatchCtx, run_command_loop};
use super::uds_cancel::{CancelSlot, TurnControl};
use super::uds_multi::MultiClientArgs;
#[path = "uds/uds_session_load.rs"]
mod uds_session_load;
use super::uds_session::AgentSession;
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use uds_session_load::load_session;

#[cfg(test)]
#[path = "uds_lifecycle_cov2_tests.rs"]
mod cov2_tests;
#[cfg(test)]
#[path = "uds_lifecycle_cov_tests.rs"]
mod cov_tests;

type ExtRegistry = std::sync::Arc<
    std::sync::Mutex<crate::infrastructure::extensions::registry::ExtensionRegistry>,
>;

pub struct UdsLoopArgs<'a> {
    pub agent: AgentLoopImpl,
    pub base_dir: &'a std::path::Path,
    pub workspace: &'a std::path::Path,
    pub session_key: String,
    pub model: String,
    pub agent_display_name: Option<String>,
    pub ephemeral: bool,
    pub system_prompt: String,
    pub socket_path: std::path::PathBuf,
    /// `None` = multi-client mode. `Some` = single-client mode (tests).
    pub socket_override: Option<std::os::unix::net::UnixStream>,
    pub session_store_override: Option<Box<dyn SessionStore + 'static>>,
    pub ext_registry: Option<ExtRegistry>,
    pub persist: bool,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,      // #562
    /// Pre-created broadcast channel for workflow event emission (#598).
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    pub provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
}
pub fn run_uds_loop(args: UdsLoopArgs<'_>) -> i32 {
    let rt = match crate::interface::cli::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to create runtime: {e}");
            return 1;
        }
    };
    rt.block_on(uds_loop_async(args))
}
use super::uds_socket::{SocketGuard, bind_secure_socket};

pub(crate) fn capture_execution_metadata(
    base_dir: &std::path::Path,
    agent_name: Option<&str>,
) -> crate::domain::execution_metadata::ExecutionMetadata {
    capture_execution_metadata_for_backend(
        base_dir,
        agent_name,
        crate::domain::execution_metadata::ExecutionBackend::from_marker(
            std::env::var_os(crate::domain::execution_metadata::NON_NATIVE_EXECUTION_ENV)
                .as_deref(),
        ),
    )
}

pub(crate) fn capture_execution_metadata_for_backend(
    base_dir: &std::path::Path,
    agent_name: Option<&str>,
    backend: crate::domain::execution_metadata::ExecutionBackend,
) -> crate::domain::execution_metadata::ExecutionMetadata {
    use crate::domain::execution_metadata::{
        AgentDisplayName, ExecutionBackend, ExecutionMetadata, FolderDisplayLabel, FolderIdentity,
        GitBranchDisplay,
    };
    if backend != ExecutionBackend::Native {
        return ExecutionMetadata::new(
            None,
            None,
            agent_name.and_then(AgentDisplayName::new),
            None,
        );
    }
    let canonical = std::fs::canonicalize(base_dir).ok();
    #[cfg(unix)]
    let folder_identity = canonical.as_ref().and_then(|path| {
        use std::os::unix::ffi::OsStrExt;
        FolderIdentity::from_unix_bytes(path.as_os_str().as_bytes().to_vec())
    });
    #[cfg(windows)]
    let folder_identity = canonical.as_ref().and_then(|path| {
        use std::os::windows::ffi::OsStrExt;
        FolderIdentity::from_windows_units(path.as_os_str().encode_wide().collect())
    });
    #[cfg(not(any(unix, windows)))]
    let folder_identity = None;
    let folder_label = canonical
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .and_then(FolderDisplayLabel::new);
    let git_branch = std::process::Command::new("git")
        .args([
            "-C",
            &base_dir.to_string_lossy(),
            "symbolic-ref",
            "--short",
            "-q",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|branch| GitBranchDisplay::new(branch.trim()));
    ExecutionMetadata::new(
        folder_identity,
        folder_label,
        agent_name.and_then(AgentDisplayName::new),
        git_branch,
    )
}

async fn uds_loop_async(args: UdsLoopArgs<'_>) -> i32 {
    let UdsLoopArgs {
        agent,
        base_dir,
        workspace,
        session_key,
        model,
        agent_display_name,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override,
        session_store_override,
        ext_registry,
        persist,
        notification_rx,
        subagent_registry,
        workflow_state,
        workflow_config,
        broadcast_tx,
        provider_reload,
        provider_reload_inputs,
    } = args;
    let file_store;
    let session_store: &dyn SessionStore = match session_store_override {
        Some(ref s) => s.as_ref(),
        None => {
            file_store = FileSessionStore::new(base_dir);
            &file_store
        }
    };
    // Refuse at open, not at first save (#1460): a key owned by another
    // live process must fail before any turn runs against it.
    if !ephemeral
        && !session_key.is_empty()
        && let Err(err) = session_store.claim(&session_key)
    {
        eprintln!("{err}");
        return 1;
    }
    let loaded_session = match load_session(session_store, &session_key, ephemeral).await {
        Ok(m) => m,
        Err(err) => {
            eprintln!("failed to load session: {err}");
            return 1;
        }
    };
    let loaded_message_count = loaded_session.messages.len();
    let messages = loaded_session.messages;
    if !ephemeral && !session_key.is_empty() {
        let metadata = capture_execution_metadata(base_dir, agent_display_name.as_deref());
        let metadata_write = if loaded_session.origin_execution_metadata.is_none() {
            crate::domain::execution_metadata::ExecutionMetadataWrite::Initialize(metadata)
        } else {
            crate::domain::execution_metadata::ExecutionMetadataWrite::UpdateLatest(metadata)
        };
        if let Err(err) = session_store
            .save_execution_metadata(&session_key, metadata_write)
            .await
        {
            eprintln!("failed to persist execution metadata: {err}");
            return 1;
        }
    }
    if let (Some(ws), Some(persisted)) = (&workflow_state, loaded_session.workflow_run) {
        if let Ok(mut engine) = ws.lock() {
            engine.restore_run(persisted);
        }
    }

    if let Some(std_stream) = socket_override {
        // Single-client path: backward-compatible with existing tests.
        single_client_loop(
            SingleClientArgs {
                agent,
                base_dir,
                workspace,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                subagent_registry,
                workflow_state,
                provider_reload,
                provider_reload_inputs,
                last_persisted_message_index: loaded_message_count,
            },
            std_stream,
            session_store,
        )
        .await
    } else {
        // Multi-client path: bind, accept loop, broadcast events.
        let listener = match bind_secure_socket(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("failed to bind socket {}: {e}", socket_path.display());
                return 1;
            }
        };
        eprint!("{}", super::uds_wire::socket_announcement(&socket_path));
        let _guard = SocketGuard(socket_path);
        super::uds_multi::multi_client_loop(
            MultiClientArgs {
                agent,
                base_dir,
                workspace,
                messages,
                model,
                session_key,
                ephemeral,
                system_prompt,
                ext_registry,
                persist,
                notification_rx,
                subagent_registry,
                workflow_state,
                workflow_config,
                broadcast_tx,
                provider_reload,
                provider_reload_inputs,
                last_persisted_message_index: loaded_message_count,
            },
            listener,
            session_store,
        )
        .await
    }
}

struct SingleClientArgs<'a> {
    agent: AgentLoopImpl,
    base_dir: &'a std::path::Path,
    workspace: &'a std::path::Path,
    messages: Vec<Message>,
    model: String,
    session_key: String,
    ephemeral: bool,
    system_prompt: String,
    ext_registry: Option<ExtRegistry>,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    provider_reload: Option<&'a mut super::provider_reload::ProviderReload>,
    provider_reload_inputs: Option<&'a super::provider_reload::ProviderReloadInputs>,
    last_persisted_message_index: usize,
}

async fn single_client_loop(
    args: SingleClientArgs<'_>,
    std_stream: std::os::unix::net::UnixStream,
    session_store: &dyn SessionStore,
) -> i32 {
    let SingleClientArgs {
        mut agent,
        base_dir,
        workspace,
        mut messages,
        model,
        mut session_key,
        ephemeral,
        system_prompt,
        ext_registry,
        subagent_registry,
        workflow_state,
        provider_reload,
        provider_reload_inputs,
        last_persisted_message_index,
    } = args;
    std_stream
        .set_nonblocking(true)
        .expect("set_nonblocking failed for test socket");
    let tokio_stream = tokio::net::UnixStream::from_std(std_stream).expect("std→tokio UnixStream");
    let (r, w) = tokio::io::split(tokio_stream);
    let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = Box::new(r);
    let mut writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin> = Box::new(w);

    let wire_mode = super::uds_wire::ConnectionWireMode::legacy();
    let workspace_event = super::protocol::AgentEvent::Workspace {
        path: workspace.display().to_string(),
    };
    let line = workspace_event.to_json_line() + "\n";
    let _ = super::uds_wire::write_event_line(&mut writer, &line, &wire_mode).await;

    inject_system_prompt(&mut messages, &system_prompt);

    let mut agent_session = AgentSession::new(model, session_key.clone());
    let max_context_tokens = agent.max_context_tokens();
    let initial_effort = agent.effort().map(|l| l.as_str().to_string());
    let initial_stats = super::uds_session::compute_session_stats(&session_key, &messages);
    let mut initial_conversation_snapshot =
        super::uds_snapshots::ConversationSnapshotData::from_messages(messages.clone());
    initial_conversation_snapshot
        .set_spill_store(agent.spill_store().cloned(), session_key.clone());

    run_command_loop(
        reader,
        &mut DispatchCtx {
            wire_mode,
            base_dir,
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                initial_conversation_snapshot,
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                agent_session.state_snapshot(0, None, max_context_tokens, initial_effort),
            )),
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut agent_session,
            stdout: Some(&mut *writer),
            session_key: &mut session_key,
            session_store,
            ephemeral,
            system_prompt: &system_prompt,
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::<TurnControl>::default(),
            broadcast_tx: None,
            _ext_registry: ext_registry,
            client_tool_registry: super::uds_ext_protocol::new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: subagent_registry.clone(),
            notification_rx: None,
            workflow_state: workflow_state.clone(),
            workflow_config: None,
            provider_reload,
            provider_reload_inputs,
            last_persisted_message_index,
            durable_prefix_dirty: false,
        },
    )
    .await;

    if !ephemeral && !session_key.is_empty() {
        remove_injected_system_prompt(&mut messages, &system_prompt);
        let session = Session::from_parts(
            session_key,
            std::mem::take(&mut messages),
            workflow_state
                .as_ref()
                .and_then(|ws| ws.lock().ok().and_then(|engine| engine.persisted_run())),
            uds_dispatch_session::snapshot_subagent_roster_with_restore_reason(
                &subagent_registry,
                if agent_session.killing_exit {
                    crate::domain::session::SubagentRestoreReason::OrdinaryTuiExitStopped
                } else {
                    crate::domain::session::SubagentRestoreReason::LegacyUnspecified
                },
            ),
        );
        let _ = session_store.save(&session).await;
    }
    0
}

pub(crate) fn inject_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    let has_real_system = messages
        .first()
        .is_some_and(|m| m.role == Role::System && !m.is_manifest);
    if !has_real_system {
        messages.insert(0, Message::system(prompt.to_string()));
    }
}

pub(crate) fn remove_injected_system_prompt(messages: &mut Vec<Message>, prompt: &str) {
    if prompt.is_empty() {
        return;
    }
    let is_injected_prompt = messages.first().is_some_and(|m| {
        m.role == Role::System
            && !m.is_manifest
            && (m.content == prompt || m.content.starts_with(prompt))
    });
    if is_injected_prompt {
        messages.remove(0);
    }
}

#[cfg(test)]
mod issue_1612_execution_context_tests {
    use super::capture_execution_metadata_for_backend;
    use crate::domain::execution_metadata::ExecutionBackend;

    #[test]
    fn non_native_execution_does_not_claim_a_host_folder_or_git_branch() {
        let metadata = capture_execution_metadata_for_backend(
            std::path::Path::new("."),
            Some("worker"),
            ExecutionBackend::NonNative,
        );
        assert!(metadata.folder_identity().is_none());
        assert!(metadata.folder_label().is_none());
        assert!(metadata.git_branch().is_none());
        assert_eq!(metadata.agent_name().map(|v| v.as_str()), Some("worker"));
    }

    #[test]
    fn native_execution_captures_the_server_working_folder() {
        let metadata = capture_execution_metadata_for_backend(
            std::path::Path::new("."),
            None,
            ExecutionBackend::Native,
        );
        assert!(metadata.folder_identity().is_some());
        assert!(metadata.folder_label().is_some());
    }
}
