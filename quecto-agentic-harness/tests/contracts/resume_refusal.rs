//! Contract of a refused resume (#2011, #2045) over the REAL adapters: the
//! file session store and its home authority, real Git/filesystem discovery,
//! real cross-process-style claims — composed by the production composition
//! function. A session that belongs elsewhere is refused with its kind and
//! folder; every refusal leaves the active session, the source files and the
//! target's claim as they were; only the same execution directory restores.
use super::resume_fixture::{World, identity};
use quecto::application::sessions::dto::{ResumeOutcome, ResumeRequest, ResumeSavedSessionError};
use quecto::application::sessions::ports::SessionStore;
use quecto::application::sessions::ports::session_home::SessionHomeCatalogue;
use quecto::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use quecto::domain::session_home::SessionHomeScope;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_home_catalogue::FileSessionHomeCatalogue;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const ACTIVE: &str = "cli:active";

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
async fn an_unrelated_directory_is_refused_as_belonging_elsewhere_and_names_the_folder() {
    let world = World::new().await;
    let elsewhere = world
        .saved_in("cli:theirs", &world.base.join("../elsewhere"))
        .await;
    let decision = world.decision("cli:theirs").await;
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    assert_eq!(decision.kind.refusal_code(), "belongs_elsewhere");
    assert_eq!(decision.execution_dir, Some(elsewhere));
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
    assert_eq!(decision.kind.refusal_code(), "home_missing");
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
    assert_eq!(decision.kind.refusal_code(), "no_home_recorded");
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

/// #2045 acceptance: the command is RUN, not read. Whatever a folder is
/// called, `cd` lands in exactly that folder and nothing in its name executes.
#[test]
fn the_command_changes_into_exactly_that_folder_whatever_it_is_called() {
    use quecto::domain::session_open_command::{cd_there_command, open_there_command};
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    for name in [
        "plain",
        "a b  c",
        "it's",
        "$(touch PWNED)",
        "`touch PWNED`; touch PWNED",
        "&& touch PWNED #",
        "-rf",
        "q\"uo\"te",
        "caf\u{e9} \u{65e5}\u{672c}",
    ] {
        let dir = root_path.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let cd = cd_there_command(&dir).expect(name);
        assert_eq!(
            open_there_command(&dir),
            Some(format!("{cd} && quecto-tui")),
            "{name}"
        );
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{cd} && pwd -P"))
            .current_dir(&root_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}: {output:?}");
        let landed = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            landed.trim_end_matches('\n'),
            dir.to_str().unwrap(),
            "{name}"
        );
    }
    let planted: Vec<_> = walk(&root_path)
        .into_iter()
        .filter(|path| path.ends_with("PWNED"))
        .collect();
    assert!(planted.is_empty(), "a folder name executed: {planted:?}");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        }
        found.push(path);
    }
    found
}
