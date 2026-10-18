//! #2206 round 2: a box counts as a plain container only when its checkout
//! resolves inside its workspace and every place a board may be answers
//! "no such file". Every other answer — a renamed checkout, an unreadable
//! `.quecto`, a symlink loop, a checkout outside the workspace — is an
//! unreadable store, so the owner's end keeps the box.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmRunInspection, PortFuture,
};
use crate::application::environments::use_cases::FinalizeEnvironmentMember;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environments::services::environment_retention::{
    MemberFinalizeMode, SwarmRunObservation,
};

use super::HostedStoreObservation;

fn record(workspace: &Path, checkout: Option<&Path>) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "u1".into(),
        name: None,
        workspace_path: workspace.to_path_buf(),
        repository: String::new(),
        script_name: "standard".into(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into()],
        retained_cleanup_argv: vec!["cleanup.sh".into()],
        retained_inspect_argv: vec![],
        members: vec!["child".into()],
        status: EnvironmentStatus::Running,
        metadata: match checkout {
            Some(checkout) => serde_json::json!({"checkout": checkout.display().to_string()}),
            None => serde_json::json!({}),
        },
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

struct Scripts(Mutex<Vec<&'static str>>);

impl EnvironmentProcessCommands for Scripts {
    fn run_retained_inspect<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }
    fn run_retained_kill<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        self.0.lock().unwrap().push("kill");
        Box::pin(async { Ok(()) })
    }
    fn run_retained_cleanup<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        self.0.lock().unwrap().push("cleanup");
        Box::pin(async { Ok(()) })
    }
    fn observe_liveness<'a>(
        &'a self,
        _: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness> {
        Box::pin(async { panic!("never asked") })
    }
}

/// The owner ends the only member: what ran, and where the record stands.
fn owner_end(record: EnvironmentRecord) -> (Vec<&'static str>, Option<EnvironmentStatus>) {
    end(record, MemberFinalizeMode::OwnerEnd)
}

/// The only member ends in `mode`: what ran, and where the record stands.
fn end(
    record: EnvironmentRecord,
    mode: MemberFinalizeMode,
) -> (Vec<&'static str>, Option<EnvironmentStatus>) {
    let registry = EnvironmentRegistry::new();
    registry.commit(record);
    let scripts = Arc::new(Scripts(Mutex::new(Vec::new())));
    let finalize = FinalizeEnvironmentMember::new(
        registry.clone(),
        scripts.clone(),
        Arc::new(HostedStoreObservation::new(
            crate::composition::swarm::swarm_board(),
        )),
    );
    futures::executor::block_on(finalize.finalize_member("C1", "child", None, mode));
    let ran = scripts.0.lock().unwrap().clone();
    (ran, registry.get("C1").map(|record| record.status))
}

fn assert_kept(record: EnvironmentRecord, case: &str) {
    let observed = HostedStoreObservation::new(crate::composition::swarm::swarm_board())
        .inspect_hosted_run(&record);
    assert!(
        matches!(observed, SwarmRunObservation::Unreadable(_)),
        "{case}: {observed:?}"
    );
    let (ran, status) = owner_end(record);
    assert!(ran.is_empty(), "{case}: nothing ran ({ran:?})");
    assert_eq!(status, Some(EnvironmentStatus::Retained), "{case}: kept");
}

fn sandbox() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    (dir, workspace)
}

#[test]
fn a_resolving_checkout_with_no_board_anywhere_is_plain_and_ends_for_good() {
    let (_dir, workspace) = sandbox();
    for checkout in [Some(workspace.as_path()), None] {
        let record = record(&workspace, checkout);
        assert_eq!(
            HostedStoreObservation::new(crate::composition::swarm::swarm_board())
                .inspect_hosted_run(&record),
            SwarmRunObservation::NoStore
        );
        let (ran, status) = owner_end(record);
        assert_eq!(ran, ["cleanup"]);
        assert_eq!(status, None, "forgotten");
    }
}

#[test]
fn a_renamed_checkout_keeps_the_box() {
    let (dir, workspace) = sandbox();
    let checkout = workspace.join("repo");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::rename(&checkout, workspace.join("repo-moved")).unwrap();
    assert_kept(record(&workspace, Some(&checkout)), "renamed checkout");
    // The advertised checkout's workspace renamed away with it.
    std::fs::rename(&workspace, dir.path().join("workspace-moved")).unwrap();
    assert_kept(record(&workspace, Some(&checkout)), "renamed workspace");
}

/// #2206 round 3: without an advertised checkout (older or third-party
/// script sets), a workspace absent from the host — one that lives only in
/// the container — proves nothing: it may host a live swarm. It is never
/// plain: the owner's end keeps it (for `container kill`), and every other
/// end runs the retained kill exactly as before #2206.
#[test]
fn an_unadvertised_workspace_absent_from_the_host_is_unverified_never_plain() {
    let dir = tempfile::tempdir().unwrap();
    let record = record(&dir.path().join("in-container-only"), None);
    assert_eq!(
        HostedStoreObservation::new(crate::composition::swarm::swarm_board())
            .inspect_hosted_run(&record),
        SwarmRunObservation::NoStoreUnverified
    );
    let (ran, status) = end(record.clone(), MemberFinalizeMode::OwnerEnd);
    assert!(ran.is_empty(), "the owner's end runs nothing: {ran:?}");
    assert_eq!(status, Some(EnvironmentStatus::Retained));
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        let (ran, status) = end(record.clone(), mode);
        assert_eq!(ran, ["kill"], "{mode:?}: the retained kill, as on master");
        assert_eq!(status, Some(EnvironmentStatus::Stopped), "{mode:?}");
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_quecto_directory_keeps_the_box() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, workspace) = sandbox();
    let hidden = workspace.join(".quecto");
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads through any mode: the case cannot be staged there.
    let staged = std::fs::symlink_metadata(hidden.join("swarm.sqlite"))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied);
    if staged {
        assert_kept(record(&workspace, Some(&workspace)), "unreadable .quecto");
        assert_kept(record(&workspace, None), "unreadable .quecto, no checkout");
    }
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_keeps_the_box() {
    let (_dir, workspace) = sandbox();
    let checkout = workspace.join("loop");
    std::os::unix::fs::symlink(&checkout, &checkout).unwrap();
    assert_kept(record(&workspace, Some(&checkout)), "checkout loop");
    let (_dir, workspace) = sandbox();
    let hidden = workspace.join(".quecto");
    std::os::unix::fs::symlink(&hidden, &hidden).unwrap();
    assert_kept(record(&workspace, Some(&workspace)), ".quecto loop");
}

#[test]
fn a_checkout_outside_the_workspace_keeps_the_box() {
    let (dir, workspace) = sandbox();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    assert_kept(record(&workspace, Some(&elsewhere)), "outside");
    assert_kept(
        record(&workspace, Some(Path::new("relative/checkout"))),
        "relative",
    );
}

/// #2206 round 3: a `.git` pointer whose git directory does not resolve on
/// the host hides where a board may be: never "no board". And "not a
/// directory" counts as empty only below a `.git` pointer that resolves.
#[test]
fn an_unresolved_git_pointer_or_a_quecto_file_keeps_the_box() {
    let (_dir, workspace) = sandbox();
    std::fs::write(workspace.join(".git"), "gitdir: /nonexistent/worktree\n").unwrap();
    assert_kept(
        record(&workspace, Some(&workspace)),
        "unresolved .git pointer",
    );
    let (_dir, workspace) = sandbox();
    std::fs::write(workspace.join(".quecto"), "not a directory\n").unwrap();
    assert_kept(record(&workspace, Some(&workspace)), ".quecto is a file");
}

/// A linked worktree whose `.git` pointer resolves on the host: its board
/// lives in the git directory it names, and with none there the checkout
/// is plain and ends for good.
#[test]
fn a_resolving_worktree_pointer_with_no_board_is_plain() {
    let (dir, workspace) = sandbox();
    let git_dir = dir.path().join("main.git/worktrees/w");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::write(
        workspace.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .unwrap();
    let record = record(&workspace, Some(&workspace));
    assert_eq!(
        HostedStoreObservation::new(crate::composition::swarm::swarm_board())
            .inspect_hosted_run(&record),
        SwarmRunObservation::NoStore
    );
    let (ran, status) = owner_end(record);
    assert_eq!(ran, ["cleanup"]);
    assert_eq!(status, None);
}
