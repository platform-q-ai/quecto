//! The shared dispatch fixture (#1972): owns everything a `DispatchCtx`
//! borrows, composes one sessions graph over its own store so the context's
//! read handles, `list_sessions` and save transaction share one
//! active-session state, and offers the save shims the dispatch tests call.
use super::cov_tests::make_agent;
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::message::Message;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::uds::DispatchCtx;
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};
use crate::interface::cli::uds_ext_protocol::{ClientToolRegistry, new_client_tool_registry};
use crate::interface::cli::uds_session::AgentSession;

/// Owns everything a `DispatchCtx` borrows so individual tests stay short.
/// The sessions graph (#1970, #1972) is composed once over the fixture's own
/// store, so the context's read handles, its `list_sessions` and its save
/// transaction share one active-session state and one file store.
/// A built-in model with the OpenAI effort scale, so effort rigs exercise a
/// real catalogue vocabulary.
pub(crate) const RIG_MODEL: &str = "openai-api/gpt-5.6-sol";

pub(super) struct Fixture {
    pub(super) agent: AgentLoopImpl,
    pub(super) messages: Vec<Message>,
    pub(super) session: AgentSession,
    pub(super) session_key: String,
    pub(super) store: std::sync::Arc<FileSessionStore>,
    pub(super) sessions: crate::interface::cli::uds_session_handles::SessionHandles,
    /// The dirty latch shared between the agent and the save transaction;
    /// a swapped agent adopts it (`set_agent`).
    latch: std::sync::Arc<crate::application::durable_prefix::DurablePrefixLatch>,
    pub(super) _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
    pub(super) cancel: CancelHandle,
    registry: ClientToolRegistry,
    ephemeral: bool,
    /// Injected system prompt mirrored into `DispatchCtx::system_prompt`
    /// and the active session's marker. Default `""` keeps existing callers
    /// free of live/durable skew.
    pub(super) system_prompt: String,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub(super) provider_reload_inputs:
        Option<crate::interface::cli::provider_reload::ProviderReloadInputs>,
    pub(super) provider_reload: Option<crate::interface::cli::provider_reload::ProviderReload>,
    /// The retention store paired with the active session (D9 #1978): the
    /// loop hands it to the sessions graph itself, never via the agent.
    spill_store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        // Publish the rig directory's catalogue generation (#1848): the
        // effort vocabulary every `get_state`/`set_effort` reads comes from
        // the published snapshot, as the composed loop has at startup.
        let _ = crate::composition::catalogue::list_models_wire_for(tmp.path());
        let store = std::sync::Arc::new(FileSessionStore::new(FlatSessionLayout::new(tmp.path())));
        let agent = make_agent();
        let latch = agent.durable_prefix_latch();
        let mut fixture = Self {
            agent,
            messages: Vec::new(),
            session: AgentSession::new(RIG_MODEL.into()),
            session_key: "cli:test".to_string(),
            store: store.clone(),
            sessions: list_handle_placeholder(&store),
            latch,
            _tmp: tmp,
            writer: tokio::io::sink(),
            cancel: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            registry: new_client_tool_registry(),
            ephemeral: false,
            system_prompt: String::new(),
            subagent_registry: None,
            provider_reload_inputs: None,
            provider_reload: None,
            spill_store: None,
        };
        fixture.compose_sessions();
        fixture
    }

    /// Recompose the sessions graph from the fixture's current inputs,
    /// carrying the persisted watermark over.
    fn compose_sessions(&mut self) {
        let watermark = self
            .sessions
            .active_session
            .try_read()
            .map(|state| state.persisted_watermark())
            .unwrap_or(0);
        let mut inputs = crate::interface::cli::uds::dispatch_session_roster_tests::loop_inputs(
            self._tmp.path(),
            &self.session_key,
        );
        inputs.ephemeral = self.ephemeral;
        inputs.system_prompt = self.system_prompt.clone();
        inputs.spill_store = self.spill_store.clone();
        inputs.durable_prefix = self.latch.clone();
        inputs.subagent_registry = self.subagent_registry.clone();
        self.sessions =
            crate::interface::cli::uds::dispatch_session_roster_tests::composed_sessions_over(
                self.store.clone(),
                inputs,
            );
        self.sessions
            .active_session
            .try_write()
            .expect("fresh session is uncontended")
            .set_persisted_watermark(watermark);
    }

    pub(super) fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self.compose_sessions();
        self
    }

    /// Replace the agent; it adopts the fixture's shared dirty latch.
    pub(super) fn set_agent(&mut self, mut agent: AgentLoopImpl) {
        agent.adopt_durable_prefix_latch(self.latch.clone());
        self.agent = agent;
        self.compose_sessions();
    }

    /// Pair a retention store with the active session, as the loop pairs
    /// the run's one store at startup (D9 #1978).
    pub(super) fn set_retention(
        &mut self,
        store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
    ) {
        self.spill_store = store;
        self.compose_sessions();
    }

    pub(super) fn set_ephemeral(&mut self, ephemeral: bool) {
        self.ephemeral = ephemeral;
        self.compose_sessions();
    }

    pub(super) fn set_session_key(&mut self, session_key: impl Into<String>) {
        self.session_key = session_key.into();
        self.compose_sessions();
    }

    /// The key the loop currently stands for, read from the active session
    /// (`session_key` is the key the fixture was opened on).
    pub(super) fn current_session_key(&self) -> String {
        self.sessions
            .active_session
            .try_read()
            .expect("the fixture holds no other lock")
            .identity()
            .runtime_key()
            .to_string()
    }

    /// Track a sub-agent roster: the context sees it and the save
    /// transaction records it.
    pub(super) fn set_subagent_registry(
        &mut self,
        registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    ) {
        self.subagent_registry = Some(registry);
        self.compose_sessions();
    }

    /// How many messages the store already holds (the durable prefix).
    pub(super) fn set_watermark(&self, persisted: usize) {
        self.sessions
            .active_session
            .try_write()
            .expect("uncontended")
            .set_persisted_watermark(persisted);
    }

    pub(super) fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            sessions: self.sessions.read_handles(),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot("cli:test", 0, None, 0, None),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            system_prompt: self.system_prompt.as_str(),
            cancel_handle: self.cancel.clone(),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: self.registry.clone(),
            current_client_id: 0,
            subagent_registry: self.subagent_registry.clone(),
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: self.provider_reload.as_mut(),
            provider_reload_inputs: self.provider_reload_inputs.as_ref(),
            fleet_teardown: None,
            list_sessions: self.sessions.list_sessions.clone(),
            save_session: self.sessions.save_session.clone(),
            rewrite: self.sessions.rewrite.clone(),
            switch: self.sessions.switch.clone(),
            catalogue: crate::interface::cli::uds::dispatch_session_roster_tests::catalogue_handles(
                self._tmp.path(),
            ),
        }
    }
}

/// A throwaway graph over `store` so the fixture can be constructed before
/// its real graph is composed from the complete inputs.
fn list_handle_placeholder(
    store: &std::sync::Arc<FileSessionStore>,
) -> crate::interface::cli::uds_session_handles::SessionHandles {
    crate::interface::cli::uds::dispatch_session_roster_tests::handles_over(
        store.clone(),
        "cli:test",
        None,
        &[],
    )
}

/// The routine save every idle-boundary trigger requests (#1860): the shape
/// the pre-D5 `persist_current_session` had, kept for the dispatch tests.
pub(crate) async fn persist_current_session(
    ctx: &mut DispatchCtx<'_>,
) -> Result<(), crate::application::sessions::dto::SaveSessionError> {
    ctx.save_session
        .save(
            ctx.messages,
            crate::application::sessions::dto::SaveTrigger::Routine,
        )
        .await
        .map(|_| ())
}

/// The explicit `persist_session` save with a client restore reason.
pub(crate) async fn persist_current_session_with_restore_reason(
    ctx: &mut DispatchCtx<'_>,
    restore_reason: crate::domain::session::SubagentRestoreReason,
) -> Result<(), crate::application::sessions::dto::SaveSessionError> {
    ctx.save_session
        .save(
            ctx.messages,
            crate::application::sessions::dto::SaveTrigger::Explicit { restore_reason },
        )
        .await
        .map(|_| ())
}

/// The persisted watermark of the context's active session (the rig's
/// state is uncontended between dispatches).
pub(crate) fn persisted_watermark(ctx: &DispatchCtx<'_>) -> usize {
    ctx.sessions
        .active_session
        .try_read()
        .expect("uncontended")
        .persisted_watermark()
}

/// Whether the context's active session armed a killing exit.
pub(crate) fn killing_exit(ctx: &DispatchCtx<'_>) -> bool {
    ctx.sessions
        .active_session
        .try_read()
        .expect("uncontended")
        .killing_exit()
}

/// Latch the session's sticky durable-prefix flag by hand.
pub(crate) fn latch_durable_prefix_dirty(ctx: &DispatchCtx<'_>) {
    ctx.sessions
        .active_session
        .try_write()
        .expect("uncontended")
        .latch_durable_prefix_dirty();
}

#[tokio::test]
async fn the_fixture_composes_one_graph_and_moves_its_persistence_state() {
    let mut fx = Fixture::new();
    fx.set_watermark(3);
    fx.set_ephemeral(true);
    let ctx = fx.ctx();
    assert_eq!(
        persisted_watermark(&ctx),
        3,
        "recomposition keeps the watermark"
    );
    assert!(!killing_exit(&ctx));
    latch_durable_prefix_dirty(&ctx);
    assert!(
        ctx.sessions
            .active_session
            .try_read()
            .unwrap()
            .durable_prefix_dirty()
    );
    let mut ctx = fx.ctx();
    assert!(persist_current_session(&mut ctx).await.is_ok());
    assert!(
        !fx._tmp.path().join("sessions").exists(),
        "an ephemeral fixture saves nothing"
    );
}
