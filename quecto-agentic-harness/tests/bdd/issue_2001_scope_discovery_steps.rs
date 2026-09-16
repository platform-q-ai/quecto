//! Executable RED specification for issue #2001 workspace-scope discovery.
//!
//! These scenarios deliberately use only existing public CLI and UDS boundaries.
//! Git repositories and linked worktrees are created with the locally installed
//! `git` executable; every fixture command and every session-producing CLI launch
//! must succeed before the final, intentionally RED discovery assertion runs.

use super::*;
use std::process::{Command, Output};

const QUERY_KEY: &str = "scope-discovery:query";
const EXPECTED_KEY: &str = "scope-discovery:expected";
const UNAVAILABLE_KEY: &str = "scope-discovery:git-unavailable";
const SAVE_PREFIX: &str = "scope-discovery:save:";

fn scope_base(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .base_dir
        .as_ref()
        .expect("scope discovery requires the temp base-directory fixture")
        .join("scope-discovery")
}

fn path_text(path: &Path) -> String {
    path.to_str()
        .expect("temporary BDD fixture paths must be UTF-8")
        .to_owned()
}

fn run_git(cwd: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Quecto BDD")
        .env("GIT_AUTHOR_EMAIL", "quecto-bdd@example.invalid")
        .env("GIT_COMMITTER_NAME", "Quecto BDD")
        .env("GIT_COMMITTER_EMAIL", "quecto-bdd@example.invalid")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "scope-discovery fixture could not execute git {:?} in {}: {error}",
                args,
                cwd.display()
            )
        });
    assert!(
        output.status.success(),
        "scope-discovery git setup {:?} in {} must succeed before the behavioral assertion; status={:?}, stdout={}, stderr={}",
        args,
        cwd.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn init_repo(path: &Path) {
    std::fs::create_dir_all(path).expect("create Git fixture directory");
    run_git(path, &["init", "--quiet"]);
    std::fs::write(path.join("tracked.txt"), "scope discovery fixture\n")
        .expect("write tracked Git fixture file");
    run_git(path, &["add", "tracked.txt"]);
    run_git(path, &["commit", "--quiet", "-m", "BDD fixture"]);
}

fn record_query(world: &mut QuectoWorld, path: &Path) {
    assert!(
        path.is_dir(),
        "query directory must exist: {}",
        path.display()
    );
    world
        .session_keys
        .insert(QUERY_KEY.to_owned(), path_text(path));
}

fn record_save(world: &mut QuectoWorld, session: &str, path: &Path) {
    assert!(
        path.is_dir(),
        "session directory must exist: {}",
        path.display()
    );
    assert!(
        session
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-'),
        "scope fixture session names use the allowlisted lowercase/hyphen alphabet"
    );
    let previous = world
        .session_keys
        .insert(format!("{SAVE_PREFIX}{session}"), path_text(path));
    assert!(
        previous.is_none(),
        "scope fixture session names must be unique"
    );
}

fn record_expected(world: &mut QuectoWorld, sessions: &[&str]) {
    assert!(
        sessions.iter().all(|session| session.starts_with("cli:")),
        "expected discovery keys use the public opaque CLI-key form"
    );
    world
        .session_keys
        .insert(EXPECTED_KEY.to_owned(), sessions.join(","));
}

fn add_unrelated_session(world: &mut QuectoWorld, base: &Path) {
    let unrelated = base.join("unrelated");
    std::fs::create_dir_all(&unrelated).expect("create unrelated folder fixture");
    record_save(world, "unrelated", &unrelated);
}

fn assert_git_metadata_unavailable(workspace: &Path) {
    let probe = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("execute unavailable-metadata verification");
    assert_eq!(
        probe.status.code(),
        Some(128),
        "fixture must expose unavailable Git metadata; stdout={}, stderr={}",
        String::from_utf8_lossy(&probe.stdout),
        String::from_utf8_lossy(&probe.stderr)
    );
}

fn record_git_unavailable(world: &mut QuectoWorld) {
    world
        .session_keys
        .insert(UNAVAILABLE_KEY.to_owned(), "true".to_owned());
}

fn configure_workspace(world: &mut QuectoWorld, workspace: &Path) {
    assert!(
        workspace.is_dir(),
        "configured workspace must exist before launch: {}",
        workspace.display()
    );
    let base = world
        .cli_context
        .base_dir
        .as_ref()
        .expect("scope discovery requires a configured base directory");
    let config_path = base.join("config.json");
    let raw = std::fs::read_to_string(&config_path).expect("read mock-provider config");
    let mut config: serde_json::Value =
        serde_json::from_str(&raw).expect("mock-provider config must remain valid JSON");
    config["agents"]["defaults"]["workspace"] = serde_json::Value::String(path_text(workspace));
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).expect("serialize configured workspace"),
    )
    .expect("write configured workspace");
    world.cli_context.cwd = Some(workspace.to_path_buf());
}

#[given(expr = "a real scope-discovery fixture for {string}")]
fn given_scope_discovery_fixture(world: &mut QuectoWorld, layout: String) {
    let base = scope_base(world);
    std::fs::create_dir_all(&base).expect("create scope-discovery fixture root");

    match layout.as_str() {
        "repository root" => {
            let repo = base.join("repo");
            init_repo(&repo);
            record_save(world, "repo-root", &repo);
            add_unrelated_session(world, &base);
            record_query(world, &repo);
            record_expected(world, &["cli:repo-root"]);
        }
        "repository subdirectory" => {
            let repo = base.join("repo");
            init_repo(&repo);
            let subdir = repo.join("src/deep");
            std::fs::create_dir_all(&subdir).expect("create repository subdirectory");
            record_save(world, "repo-root", &repo);
            record_save(world, "repo-subdir", &subdir);
            add_unrelated_session(world, &base);
            record_query(world, &subdir);
            record_expected(world, &["cli:repo-root", "cli:repo-subdir"]);
        }
        "nested repository" => {
            let parent = base.join("parent");
            init_repo(&parent);
            let nested = parent.join("nested");
            init_repo(&nested);
            record_save(world, "parent-repo", &parent);
            record_save(world, "nested-repo", &nested);
            record_query(world, &nested);
            record_expected(world, &["cli:nested-repo"]);
        }
        "linked worktree" => {
            let repo = base.join("repo");
            init_repo(&repo);
            let linked = base.join("linked");
            let linked_text = path_text(&linked);
            run_git(
                &repo,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    "bdd-linked",
                    &linked_text,
                ],
            );
            record_save(world, "repo-root", &repo);
            record_save(world, "linked-worktree", &linked);
            add_unrelated_session(world, &base);
            record_query(world, &linked);
            record_expected(world, &["cli:repo-root", "cli:linked-worktree"]);
        }
        "non-Git exact folder" => {
            let exact = base.join("plain/exact");
            let sibling = base.join("plain/sibling");
            std::fs::create_dir_all(&exact).expect("create exact non-Git folder");
            std::fs::create_dir_all(&sibling).expect("create sibling non-Git folder");
            record_save(world, "exact-folder", &exact);
            record_save(world, "plain-sibling", &sibling);
            record_query(world, &exact);
            record_expected(world, &["cli:exact-folder"]);
        }
        "symlink canonical identity" => {
            let canonical = base.join("canonical");
            std::fs::create_dir_all(&canonical).expect("create canonical folder");
            let symlink = base.join("canonical-link");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&canonical, &symlink).expect("create directory symlink");
            #[cfg(not(unix))]
            panic!("scope-discovery symlink scenarios require the repository's Unix test target");
            record_save(world, "canonical", &canonical);
            record_save(world, "symlink", &symlink);
            add_unrelated_session(world, &base);
            record_query(world, &symlink);
            record_expected(world, &["cli:canonical", "cli:symlink"]);
        }
        "detached Git worktree" => {
            let repo = base.join("repo");
            init_repo(&repo);
            let subdir = repo.join("detached/subdir");
            std::fs::create_dir_all(&subdir).expect("create detached repository subdirectory");
            run_git(&repo, &["checkout", "--quiet", "--detach", "HEAD"]);
            let symbolic_ref = Command::new("git")
                .args(["symbolic-ref", "-q", "HEAD"])
                .current_dir(&repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("execute detached-HEAD verification");
            assert_eq!(
                symbolic_ref.status.code(),
                Some(1),
                "fixture must be detached before discovery; stdout={}, stderr={}",
                String::from_utf8_lossy(&symbolic_ref.stdout),
                String::from_utf8_lossy(&symbolic_ref.stderr)
            );
            record_save(world, "repo-root", &repo);
            record_save(world, "repo-subdir", &subdir);
            add_unrelated_session(world, &base);
            record_query(world, &subdir);
            record_expected(world, &["cli:repo-root", "cli:repo-subdir"]);
        }
        "unavailable Git metadata" => {
            let broken = base.join("broken-worktree");
            std::fs::create_dir_all(&broken).expect("create broken Git worktree fixture");
            std::fs::write(broken.join(".git"), "gitdir: ../missing-git-dir\n")
                .expect("write deliberately unavailable Git metadata pointer");
            assert_git_metadata_unavailable(&broken);
            record_save(world, "broken-worktree", &broken);
            record_query(world, &broken);
            record_git_unavailable(world);
        }
        "permission-denied Git metadata" => {
            let broken = base.join("permission-denied-worktree");
            std::fs::create_dir_all(&broken).expect("create permission Git worktree fixture");
            std::fs::create_dir(broken.join(".git")).expect("create Git metadata directory");
            let mut permissions = std::fs::metadata(broken.join(".git"))
                .expect("read Git metadata permissions")
                .permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                permissions.set_mode(0o000);
            }
            #[cfg(not(unix))]
            panic!("permission discovery scenarios require the repository's Unix test target");
            std::fs::set_permissions(broken.join(".git"), permissions)
                .expect("make Git metadata inaccessible");
            assert_git_metadata_unavailable(&broken);
            record_save(world, "permission-denied", &broken);
            record_query(world, &broken);
            record_git_unavailable(world);
        }
        other => panic!(
            "scope discovery layout {other:?} is outside the fixture's affirmative allowlist"
        ),
    }
}

#[given("real CLI sessions are saved from the scope-discovery execution directories")]
fn given_real_cli_sessions_are_saved(world: &mut QuectoWorld) {
    let mut launches: Vec<(String, PathBuf)> = world
        .session_keys
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(SAVE_PREFIX)
                .map(|session| (session.to_owned(), PathBuf::from(value)))
        })
        .collect();
    launches.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        !launches.is_empty(),
        "scope-discovery fixture must specify at least one real CLI launch"
    );

    for (session, workspace) in launches {
        configure_workspace(world, &workspace);
        let output = cli::run_with_output(
            vec![
                "quecto".to_owned(),
                "agent".to_owned(),
                "-s".to_owned(),
                session.clone(),
                "-m".to_owned(),
                format!("save scope fixture from {}", workspace.display()),
            ],
            &world.cli_context,
        );
        assert_eq!(
            output.exit_code,
            0,
            "real CLI launch for session {session:?} in {} must succeed before discovery; stdout={}, stderr={}",
            workspace.display(),
            output.stdout,
            output.stderr
        );
    }
}

#[when("I launch UDS session discovery from the fixture query directory")]
fn when_launch_uds_discovery_from_query(world: &mut QuectoWorld) {
    let query = PathBuf::from(
        world
            .session_keys
            .get(QUERY_KEY)
            .expect("scope-discovery fixture must record a query directory"),
    );
    configure_workspace(world, &query);
    world.session_name = Some("scope-discovery-current".to_owned());
    world.no_session = false;
}

#[then(expr = "the local discovery candidates should contain exactly {string}")]
fn then_local_discovery_contains_exactly(world: &mut QuectoWorld, declared: String) {
    let configured = world
        .session_keys
        .get(EXPECTED_KEY)
        .expect("scope-discovery fixture must record its expected opaque keys");
    assert_eq!(
        declared, *configured,
        "feature example and executable fixture expectation must agree"
    );

    let response = uds_steps::find_agent_response_by_id(world, "scope-discovery-list")
        .expect("real UDS launch must return a list_sessions response before scope is asserted");
    assert_eq!(
        response["success"], true,
        "real UDS list_sessions setup must succeed before the behavioral result assertion: {response:#?}"
    );
    let mut actual: Vec<String> = response["data"]["sessions"]
        .as_array()
        .expect("successful list_sessions response must carry data.sessions")
        .iter()
        .map(|session| {
            session["key"]
                .as_str()
                .expect("each public session summary must carry an opaque string key")
                .to_owned()
        })
        .collect();
    let mut expected: Vec<String> = declared
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(
        actual, expected,
        "candidate discovery must use the canonical nearest repository/worktree group or exact non-Git folder; membership does not imply immediate resume eligibility"
    );
}

#[then("session discovery should report that Git workspace metadata is unavailable")]
fn then_discovery_reports_unavailable_git(world: &mut QuectoWorld) {
    assert_eq!(
        world.session_keys.get(UNAVAILABLE_KEY).map(String::as_str),
        Some("true"),
        "the unavailable-metadata assertion is valid only for its allowlisted fixture"
    );
    let response = uds_steps::find_agent_response_by_id(world, "scope-discovery-list")
        .expect("real UDS launch must produce a final discovery response");
    let success = response["success"]
        .as_bool()
        .expect("list_sessions response success must be boolean");
    let error = response["error"].as_str().unwrap_or_default();
    assert_eq!(
        success, false,
        "inaccessible Git metadata must be an explicit behavioral discovery outcome: {response:#?}"
    );
    assert!(
        !error.trim().is_empty(),
        "an inaccessible-metadata outcome must carry a public diagnostic without prescribing its prose: {response:#?}"
    );
}
