//! Contract of the resume decisions (#2011) over the REAL adapters: the file
//! session store and its home authority, real Git/filesystem discovery, real
//! cross-process-style claims — composed by the production composition
//! function. Every refusal leaves the active session, the source files and
//! the target's claim as they were.
use quecto::application::durable_prefix::DurablePrefixLatch;
use quecto::application::sessions::dto::{
    ResumeDecision, ResumeOutcome, ResumeRequest, ResumeSavedSessionError,
};
use quecto::application::sessions::ports::session_runtime::TurnAccountingReset;
use quecto::application::sessions::ports::{
    SessionKeyPropagation, SessionStore, SessionSwitchRuntime,
};
use quecto::composition::session_home::session_home_in;
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::domain::workflow::WorkflowRunPersisted;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
use quecto::interface::cli::uds_session_handles::{SessionHandles, SessionLoopInputs};
use quecto::interface::uds::sessions::controller::ListSessionsController;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Default)]
struct Runtime {
    switched_to: Vec<String>,
}
impl TurnAccountingReset for Runtime {
    fn history_replaced(&mut self, _: usize) {}
}
impl SessionKeyPropagation for Runtime {
    fn session_key_changed(&mut self, identity: &SessionIdentity) {
        self.switched_to.push(identity.runtime_key().to_string());
    }
}
impl SessionSwitchRuntime for Runtime {
    fn reset_effort_to_default(&mut self) {}
    fn reset_workflow(&mut self) {}
    fn restore_workflow(&mut self, _: WorkflowRunPersisted) {}
}

/// A store with one active loop in `here` and saved sessions elsewhere.
pub(super) struct World {
    _temp: tempfile::TempDir,
    pub(super) base: PathBuf,
    pub(super) here: PathBuf,
    pub(super) layout: FlatSessionLayout,
    pub(super) store: Arc<FileSessionStore>,
    pub(super) handles: SessionHandles,
}

const ACTIVE: &str = "cli:active";

pub(super) fn identity(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

impl World {
    pub(super) async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let here = temp.path().join("here");
        std::fs::create_dir_all(&here).unwrap();
        let here = here.canonicalize().unwrap();
        let layout = FlatSessionLayout::new(&base);
        let store = Arc::new(FileSessionStore::new(layout.clone()));
        let home = session_home_in(store.clone(), Ok(here.clone()));
        let list = quecto::application::sessions::use_cases::ListSessions::new(store.clone())
            .with_home(home.clone());
        // The production graph, over an explicit execution directory.
        let handles = quecto::composition::active_session::assemble_session_handles(
            SessionLoopInputs {
                base_dir: base.clone(),
                identity: identity(ACTIVE),
                ephemeral: false,
                system_prompt: String::new(),
                spill_store: None,
                durable_prefix: DurablePrefixLatch::shared(),
                workflow_state: None,
                subagent_registry: None,
            },
            store.clone(),
            quecto::composition::session_search::discovery_handles(
                Arc::new(ListSessionsController::new(Arc::new(list))),
                home.clone(),
            ),
            None,
            quecto::composition::sessions::build_fresh_session_identity(),
            home,
        );
        handles
            .switch
            .resume
            .open_at_startup()
            .await
            .expect("startup");
        Self {
            _temp: temp,
            base,
            here,
            layout,
            store,
            handles,
        }
    }

    /// Save `key` as a session whose home is `dir` (created), through the
    /// real store and the real discovery, then release it as a finished run.
    pub(super) async fn saved_in(&self, key: &str, dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let home = GitScopeDiscovery::default().discover(&dir).unwrap();
        self.store.record_new_home(&identity(key), &home).unwrap();
        self.save_transcript(key).await;
        dir
    }

    pub(super) async fn save_transcript(&self, key: &str) {
        let mut session = Session::new(identity(key));
        session
            .messages
            .push(Message::user(format!("history of {key}")));
        self.store.save(&session).await.unwrap();
        self.store.release(&identity(key));
    }

    pub(super) async fn request(
        &self,
        request: &ResumeRequest,
    ) -> (Result<ResumeOutcome, ResumeSavedSessionError>, Vec<Message>) {
        let mut messages = vec![Message::user("live conversation")];
        let mut runtime = Runtime::default();
        let result = self
            .handles
            .switch
            .resume
            .execute(request, &mut messages, None, &mut runtime)
            .await;
        if result.is_err() {
            assert!(runtime.switched_to.is_empty(), "a refusal switches nothing");
        }
        (result, messages)
    }

    pub(super) async fn decision(&self, key: &str) -> ResumeDecision {
        let files = self.files();
        let (result, messages) = self.request(&ResumeRequest::restore(key)).await;
        let Err(ResumeSavedSessionError::Decision(decision)) = result else {
            panic!("decision expected: {result:?}");
        };
        self.assert_untouched(key, &messages, &files).await;
        *decision
    }

    pub(super) fn files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(self.base.join("sessions"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|e| e != "lock"))
            .filter(|path| !path.to_string_lossy().contains(".owner"))
            .filter(|path| !path.to_string_lossy().contains("catalogue"))
            // The departing session's own lifecycle save is permitted (a miss
            // is only known under the claim, after that save).
            .filter(|path| !path.to_string_lossy().contains("cli_active"))
            .map(|path| (path.clone(), std::fs::read(&path).unwrap()))
            .collect();
        files.sort();
        files
    }

    /// The refusal invariants: the live conversation and identity stand, no
    /// transcript or home changed, the active claim is held and the target's
    /// is free for another process.
    pub(super) async fn assert_untouched(
        &self,
        target: &str,
        messages: &[Message],
        files: &[(PathBuf, Vec<u8>)],
    ) {
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "live conversation");
        assert_eq!(
            self.handles.read_handles().current_session_key().await,
            ACTIVE
        );
        assert_eq!(
            self.files(),
            files,
            "no transcript or home authority changed"
        );
        let competitor = FileSessionStore::new(self.layout.clone());
        assert!(
            competitor.claim(&identity(ACTIVE)).is_err(),
            "active claim kept"
        );
        competitor
            .claim(&identity(target))
            .expect("the target's claim is not leaked");
        competitor.release(&identity(target));
    }
}
