//! Contract of the resume decisions (#2011) over the REAL adapters: the file
//! session store and its home authority, real Git/filesystem discovery, real
//! cross-process-style claims — composed by the production composition
//! function. Every refusal leaves the active session, the source files and
//! the target's claim as they were.
use quecto::application::durable_prefix::DurablePrefixLatch;
use quecto::application::sessions::dto::{
    ActionAvailability, ResumeDecision, ResumeIntent, ResumeOutcome, ResumeRequest,
    ResumeSavedSessionError,
};
use quecto::application::sessions::ports::session_home::{
    SessionHomeCatalogue, WorkspaceDiscovery,
};
use quecto::application::sessions::ports::session_runtime::TurnAccountingReset;
use quecto::application::sessions::ports::{
    SessionKeyPropagation, SessionStore, SessionSwitchRuntime,
};
use quecto::composition::session_home::session_home_in;
use quecto::domain::message::Message;
use quecto::domain::resume_decision::{HomeVersion, ResumeAction, ResumeDecisionKind};
use quecto::domain::session::Session;
use quecto::domain::session_home::SessionHomeScope;
use quecto::domain::session_identity::SessionIdentity;
use quecto::domain::workflow::WorkflowRunPersisted;
use quecto::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
use quecto::interface::cli::uds_session_handles::{SessionHandles, SessionLoopInputs};
use quecto::interface::uds::sessions::controller::ListSessionsController;
use std::os::unix::fs::PermissionsExt;
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
            Arc::new(ListSessionsController::new(Arc::new(list))),
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

fn offered(decision: &ResumeDecision) -> Vec<ResumeAction> {
    decision.offers.iter().map(|offer| offer.action).collect()
}

fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[tokio::test]
async fn the_same_execution_directory_restores_and_moves_the_single_ownership() {
    let world = World::new().await;
    world.saved_in("cli:mine", &world.here).await;
    let (result, messages) = world.request(&ResumeRequest::restore("cli:mine")).await;
    let Ok(ResumeOutcome::Resumed(resumed)) = result else {
        panic!("restored expected: {result:?}");
    };
    assert_eq!(resumed.identity.runtime_key(), "cli:mine");
    assert!(messages.iter().any(|m| m.content == "history of cli:mine"));
    let competitor = FileSessionStore::new(world.layout.clone());
    assert!(
        competitor.claim(&identity("cli:mine")).is_err(),
        "now owned"
    );
    competitor
        .claim(&identity(ACTIVE))
        .expect("the departing key was released: ownership stays single");
}

#[tokio::test]
async fn an_unrelated_directory_is_a_cross_folder_decision_with_every_executor_unavailable() {
    let world = World::new().await;
    let elsewhere = world
        .saved_in("cli:theirs", &world.base.join("../elsewhere"))
        .await;
    let decision = world.decision("cli:theirs").await;
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    assert_eq!(decision.execution_dir, Some(elsewhere));
    assert_eq!(
        offered(&decision),
        [
            ResumeAction::OpenOriginal,
            ResumeAction::ForkCurrent,
            ResumeAction::Cancel
        ]
    );
    for offer in &decision.offers {
        let cancel = offer.action == ResumeAction::Cancel;
        assert_eq!(offer.availability == ActionAvailability::Available, cancel);
    }
}

#[tokio::test]
async fn a_linked_worktree_of_the_same_repository_is_still_cross_folder() {
    let world = World::new().await;
    let git = |dir: &Path, args: &[&str]| {
        let output = std::process::Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(dir)
            .args(["-c", "user.email=t@e.st", "-c", "user.name=t"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    git(&world.here, &["init", "-q"]);
    git(
        &world.here,
        &["commit", "-q", "--allow-empty", "-m", "root"],
    );
    let linked = world.here.parent().unwrap().join("linked");
    git(
        &world.here,
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    world.saved_in("cli:worktree", &linked).await;
    let decision = world.decision("cli:worktree").await;
    assert_eq!(
        decision.kind,
        ResumeDecisionKind::CrossFolder,
        "grouping is discovery, never permission"
    );
}

#[tokio::test]
async fn a_moved_directory_is_a_home_missing_decision() {
    let world = World::new().await;
    let dir = world
        .saved_in("cli:moved", &world.base.join("../before"))
        .await;
    std::fs::rename(&dir, dir.with_file_name("after")).unwrap();
    let decision = world.decision("cli:moved").await;
    assert_eq!(decision.kind, ResumeDecisionKind::HomeMissing);
    assert_eq!(
        offered(&decision),
        [
            ResumeAction::Locate,
            ResumeAction::ForkCurrent,
            ResumeAction::Cancel
        ]
    );
    assert!(decision.detail.is_some(), "the adapter's reason is carried");
    assert!(!dir.exists(), "nothing was created in its place");
}

#[tokio::test]
async fn a_permission_inaccessible_directory_never_broadens_eligibility() {
    if running_as_root() {
        return; // permissions do not bind root; the moved-directory contract covers it
    }
    let world = World::new().await;
    let parent = world.base.join("../locked");
    let dir = world.saved_in("cli:locked", &parent.join("inner")).await;
    let parent = dir.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (result, _) = world.request(&ResumeRequest::restore("cli:locked")).await;
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Err(ResumeSavedSessionError::Decision(decision)) = result else {
        panic!("decision expected: {result:?}");
    };
    assert_eq!(decision.kind, ResumeDecisionKind::HomeMissing);
}

#[tokio::test]
async fn a_directory_that_became_a_repository_is_a_home_changed_decision() {
    let world = World::new().await;
    world.saved_in("cli:regrouped", &world.here).await;
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args(["init", "-q"])
        .arg(&world.here)
        .output()
        .unwrap();
    assert!(output.status.success());
    let decision = world.decision("cli:regrouped").await;
    assert_eq!(decision.kind, ResumeDecisionKind::HomeChanged);
}

#[tokio::test]
async fn unreadable_and_hostile_metadata_is_a_home_unknown_decision_without_a_path() {
    let world = World::new().await;
    world
        .saved_in("cli:broken", &world.base.join("../broken"))
        .await;
    let hostile = "/tmp/s2011\u{1b}[2Jevil".as_bytes().to_vec();
    let record = serde_json::json!({
        "version": 1, "execution_dir": hostile, "group_kind": "folder",
        "group_path": hostile, "provenance": "saved_here",
    });
    for bytes in [
        b"{broken".to_vec(),
        serde_json::to_vec(&record).unwrap(),
        vec![0xff, 0xfe],
    ] {
        std::fs::write(world.layout.home_file(&identity("cli:broken")), &bytes).unwrap();
        let decision = world.decision("cli:broken").await;
        assert_eq!(decision.kind, ResumeDecisionKind::HomeUnknown);
        assert_eq!(
            decision.execution_dir, None,
            "an unvalidated path is never carried"
        );
    }
}

#[tokio::test]
async fn a_legacy_pretty_printed_transcript_is_a_legacy_decision_and_gains_no_home() {
    let world = World::new().await;
    world.save_transcript("cli:legacy").await;
    let path = world.layout.session_file(&identity("cli:legacy"));
    assert!(!world.layout.home_file(&identity("cli:legacy")).exists());
    let decision = world.decision("cli:legacy").await;
    assert_eq!(decision.kind, ResumeDecisionKind::LegacyUnscoped);
    assert_eq!(
        offered(&decision),
        [ResumeAction::Associate, ResumeAction::Cancel]
    );
    assert!(path.exists());
    assert!(
        !world.layout.home_file(&identity("cli:legacy")).exists(),
        "never associated implicitly"
    );
}

#[tokio::test]
async fn an_exact_miss_never_resolves_a_prefix_neighbour_and_leaks_no_claim() {
    let world = World::new().await;
    world.saved_in("cli:neighbour", &world.here).await;
    let files = world.files();
    for near in ["cli:neighbou", "cli:neighbour2", "neighbou"] {
        let (result, messages) = world.request(&ResumeRequest::restore(near)).await;
        assert!(
            matches!(&result, Err(ResumeSavedSessionError::NotFound(name)) if name == near),
            "{near}: {result:?}"
        );
        world
            .assert_untouched("cli:neighbour", &messages, &files)
            .await;
        assert!(
            !world.layout.session_file(&identity(ACTIVE)).exists(),
            "a miss is known before any effect: not even the departing save ran"
        );
        let competitor = FileSessionStore::new(world.layout.clone());
        let missed = SessionIdentity::named_cli(near.trim_start_matches("cli:")).unwrap();
        competitor
            .claim(&missed)
            .expect("the miss's claim is released");
    }
}

#[tokio::test]
async fn a_corrupt_derived_index_cannot_block_exact_key_resolution() {
    let world = World::new().await;
    world.saved_in("cli:mine", &world.here).await;
    let elsewhere = world.base.join("../elsewhere");
    world.saved_in("cli:theirs", &elsewhere).await;
    let index = world.base.join("sessions/home.catalogue");
    std::fs::write(&index, b"\x00not an index").unwrap();
    assert_eq!(
        world.decision("cli:theirs").await.kind,
        ResumeDecisionKind::CrossFolder
    );
    assert_eq!(
        std::fs::read(&index).unwrap(),
        b"\x00not an index",
        "the exact path never reads or repairs it"
    );
    let (result, _) = world.request(&ResumeRequest::restore("cli:mine")).await;
    assert!(
        matches!(result, Ok(ResumeOutcome::Resumed(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn the_home_version_is_the_authoritys_and_a_stale_one_is_refused() {
    let world = World::new().await;
    world.saved_in("cli:mine", &world.here).await;
    let catalogue = FileSessionHomeCatalogue::with_store(world.store.clone());
    let mine = identity("cli:mine");
    let scope = catalogue.read(&mine).unwrap();
    let listed = HomeVersion::of(&mine, &scope);
    assert_eq!(
        HomeVersion::of(&mine, &catalogue.read(&mine).unwrap()),
        listed
    );
    assert_ne!(
        listed,
        HomeVersion::of(&mine, &SessionHomeScope::LegacyUnscoped)
    );
    // The authority is replaced after the client was shown `listed`.
    let other = world
        .saved_in("cli:other", &world.base.join("../other"))
        .await;
    assert!(other.exists());
    let authority = std::fs::read(world.layout.home_file(&identity("cli:mine"))).unwrap();
    std::fs::copy(
        world.layout.home_file(&identity("cli:other")),
        world.layout.home_file(&identity("cli:mine")),
    )
    .unwrap();
    let files = world.files();
    let request = ResumeRequest {
        target: "cli:mine".into(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(listed.clone()),
    };
    let (result, messages) = world.request(&request).await;
    assert!(
        matches!(result, Err(ResumeSavedSessionError::StaleHomeVersion)),
        "{result:?}"
    );
    world.assert_untouched("cli:mine", &messages, &files).await;
    // Restored authority: the listed version is current again and restores.
    std::fs::write(world.layout.home_file(&identity("cli:mine")), authority).unwrap();
    let (result, _) = world.request(&request).await;
    assert!(
        matches!(result, Ok(ResumeOutcome::Resumed(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_target_owned_by_another_live_process_is_refused_and_not_stolen() {
    let world = World::new().await;
    world.saved_in("cli:busy", &world.here).await;
    let owner = FileSessionStore::new(world.layout.clone());
    owner.claim(&identity("cli:busy")).unwrap();
    let (result, _) = world.request(&ResumeRequest::restore("cli:busy")).await;
    assert!(
        matches!(result, Err(ResumeSavedSessionError::Claim(_))),
        "{result:?}"
    );
    assert!(
        FileSessionStore::new(world.layout.clone())
            .claim(&identity("cli:busy"))
            .is_err(),
        "the foreign owner still holds it"
    );
    assert_eq!(
        world.handles.read_handles().current_session_key().await,
        ACTIVE
    );
}

#[tokio::test]
async fn cancel_and_every_unavailable_action_touch_nothing() {
    let world = World::new().await;
    world
        .saved_in("cli:theirs", &world.base.join("../elsewhere"))
        .await;
    let shown = world.decision("cli:theirs").await.home_version;
    let files = world.files();
    for action in ResumeAction::ALL {
        let request = ResumeRequest {
            target: "cli:theirs".into(),
            intent: ResumeIntent::Act(action),
            expected_home_version: Some(shown.clone()),
        };
        let (result, messages) = world.request(&request).await;
        match (action, result) {
            (ResumeAction::Cancel, Ok(ResumeOutcome::Cancelled { name })) => {
                assert_eq!(name, "cli:theirs");
            }
            (
                _,
                Err(ResumeSavedSessionError::ActionUnavailable {
                    action: refused, ..
                }),
            ) => {
                assert_eq!(refused, action, "never substituted");
            }
            (action, other) => panic!("{action:?}: {other:?}"),
        }
        world
            .assert_untouched("cli:theirs", &messages, &files)
            .await;
    }
}
