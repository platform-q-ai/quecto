use super::*;

// Slice 5 (#1369): epic acceptance driving the CANONICAL container-runtime
// reference scripts (scripts/container-runtime/{create,exec,inspect,kill}.sh
// at the workspace root) through the production script adapter and strict
// parser, in their host-local mode so the suite runs in CI without Docker.
// ===========================================================================

/// Absolute path to the canonical container-runtime script directory at the
/// workspace root. The scripts are the shipped reference runtime — the suite
/// must execute those exact files, never a test-local copy.
fn canonical_runtime_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("workspace root above the harness crate")
        .join("scripts")
        .join("container-runtime")
}

/// Root the canonical scripts use for their host-local invocation records
/// (`creates.log`, `execs.log`, and per-environment `ref`/`kill.log`/
/// `inspect.log`/`children.jsonl`), passed via `--state-dir`.
fn canonical_state_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("canonical-state")
}

fn count_lines(path: &PathBuf) -> usize {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

/// Resolve the canonical state directory for the environment an agent lives
/// in, by matching the agent's captured `CN` ref against each environment's
/// recorded `ref` file.
fn env_state_dir_for(world: &QuectoWorld, agent_id: &str) -> PathBuf {
    let env_ref = world
        .agent_env_refs
        .get(agent_id)
        .unwrap_or_else(|| {
            panic!(
                "no captured environment ref for {agent_id}: {:?}",
                world.agent_env_refs
            )
        })
        .clone();
    let state = canonical_state_dir(world);
    let entries = std::fs::read_dir(&state)
        .unwrap_or_else(|e| panic!("canonical state dir {} unreadable: {e}", state.display()));
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let recorded = std::fs::read_to_string(dir.join("ref")).unwrap_or_default();
        if recorded.trim() == env_ref {
            return dir;
        }
    }
    panic!(
        "no canonical environment state recorded for ref {env_ref} under {}",
        state.display()
    );
}

fn make_fixture_repo(dir: &PathBuf, marker: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .expect("run git for fixture repo");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    };
    if !dir.join(".git").exists() {
        run(&["init", "--quiet"]);
    }
    std::fs::write(dir.join("marker.txt"), format!("{marker}\n")).unwrap();
    run(&["add", "marker.txt"]);
    run(&[
        "-c",
        "user.email=bdd@example.invalid",
        "-c",
        "user.name=bdd",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "fixture",
    ]);
}

fn fixture_repo_path(world: &QuectoWorld, name: &str) -> PathBuf {
    base_path(world).join("fixtures").join(name)
}

/// `repo-a` → `REPO_A_MARKER`: the marker committed into a fixture repository
/// derives from its name, so the Gherkin states where each marker comes from.
fn fixture_marker(name: &str) -> String {
    format!("{}_MARKER", name.to_uppercase().replace('-', "_"))
}

#[given(expr = "repository fixtures {string} and {string} exist")]
fn given_repository_fixtures(world: &mut QuectoWorld, a: String, b: String) {
    ensure_temp_dir(world);
    for name in [&a, &b] {
        make_fixture_repo(&fixture_repo_path(world, name), &fixture_marker(name));
    }
}

#[given(expr = "the parent session's repository is fixture {string}")]
fn given_parent_repository_fixture(world: &mut QuectoWorld, name: String) {
    // Omitted-repo spawns discover the parent checkout's origin, so pointing
    // origin at the fixture makes the canonical create script clone it.
    let base = base_path(world);
    let repo = fixture_repo_path(world, &name);
    std::process::Command::new("git")
        .arg("-C")
        .arg(&base)
        .args(["remote", "remove", "origin"])
        .status()
        .ok();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&base)
        .args(["remote", "add", "origin", repo.to_str().unwrap()])
        .status()
        .expect("git remote add origin fixture");
    assert!(status.success(), "git remote add origin failed");
}

#[given("the canonical container-runtime script set is configured")]
fn given_canonical_runtime(world: &mut QuectoWorld) {
    // Full production wiring (SpawnTool, environment control, notification and
    // live-event channels) comes from the shared slice-2 given; only the
    // configured script set is replaced with the canonical repository scripts.
    spawn_env_steps::given_shared_script_spawn(world, false);

    let state = canonical_state_dir(world);
    std::fs::create_dir_all(&state).unwrap();

    let runtime_dir = canonical_runtime_dir();
    let script = |name: &str, extra: &[&str]| {
        let path = runtime_dir.join(name);
        let mut argv = vec![
            path.to_string_lossy().to_string(),
            "--state-dir".to_string(),
            state.to_string_lossy().to_string(),
        ];
        argv.extend(extra.iter().map(|s| s.to_string()));
        serde_json::json!(argv)
    };
    let cfg_path = PathBuf::from(world.config_path.clone().unwrap());
    let mut cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    cfg["container_scripts"] = serde_json::json!({
        "default": "container-runtime",
        "scripts": {
            "container-runtime": {
                "create": script("create.sh", &[]),
                "exec": script("exec.sh", &[]),
                "inspect": script("inspect.sh", &[]),
                // One kill.sh serves both operations; --op tags each recorded
                // invocation so assertions can prove WHICH operation ran.
                "kill": script("kill.sh", &["--op", "kill"]),
                "cleanup": script("kill.sh", &["--op", "cleanup"]),
            }
        }
    });
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
}

// --- When / Given (spawns) ---

#[when(expr = "I spawn canonical subagent {string} into a new environment with task {string}")]
fn when_spawn_canonical_new(world: &mut QuectoWorld, agent_id: String, task: String) {
    // Canonical subagents are full implementers (read_only: false); only the
    // explicit "read-only subagent" steps spawn observers, so the language
    // distinction between implementer and reviewer is real.
    spawn_env_steps::execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({
            "agent_id": agent_id,
            "task": task,
            "container": {"mode": "new"},
            "read_only": false,
        }),
    );
}

#[when(
    expr = "I spawn canonical subagent {string} for repository fixture {string} with task {string}"
)]
fn when_spawn_canonical_repo(
    world: &mut QuectoWorld,
    agent_id: String,
    repo: String,
    task: String,
) {
    let repo_path = fixture_repo_path(world, &repo);
    spawn_env_steps::execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({
            "agent_id": agent_id,
            "task": task,
            "container": {"mode": "new", "repo": repo_path.to_string_lossy()},
            "read_only": false,
        }),
    );
}

#[given(expr = "canonical subagent {string} is running in a new environment with task {string}")]
fn given_canonical_running(world: &mut QuectoWorld, agent_id: String, task: String) {
    when_spawn_canonical_new(world, agent_id.clone(), task);
    let result = world.spawn_result.as_ref().expect("spawn result");
    assert!(
        !result.is_error,
        "canonical spawn of {agent_id} failed: {}",
        result.content
    );
}

#[given(
    expr = "canonical subagent {string} is running for repository fixture {string} with task {string}"
)]
fn given_canonical_running_repo(
    world: &mut QuectoWorld,
    agent_id: String,
    repo: String,
    task: String,
) {
    when_spawn_canonical_repo(world, agent_id.clone(), repo, task);
    let result = world.spawn_result.as_ref().expect("spawn result");
    assert!(
        !result.is_error,
        "canonical spawn of {agent_id} failed: {}",
        result.content
    );
}

#[when(expr = "I spawn canonical subagent {string} for a missing repository with task {string}")]
fn when_spawn_canonical_missing_repo(world: &mut QuectoWorld, agent_id: String, task: String) {
    let missing = base_path(world).join("fixtures").join("does-not-exist");
    spawn_env_steps::execute_env_spawn(
        world,
        &agent_id,
        serde_json::json!({
            "agent_id": agent_id,
            "task": task,
            "container": {"mode": "new", "repo": missing.to_string_lossy()},
            "read_only": false,
        }),
    );
}

#[then("the spawn result should be a canonical create failure")]
fn then_canonical_create_failed(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("spawn result");
    assert!(
        result.is_error,
        "expected canonical create failure, got success: {}",
        result.content
    );
    assert!(
        result.content.contains("create failed"),
        "expected a create-phase failure, got: {}",
        result.content
    );
}

#[then("the canonical state root should contain no environment directories")]
fn then_canonical_state_empty(world: &mut QuectoWorld) {
    // The create script's ERR trap must have rolled back the partially
    // created environment: a directory Quecto was never told about could
    // never be reached by the `cleanup` operation.
    let state = canonical_state_dir(world);
    let leftover: Vec<_> = std::fs::read_dir(&state)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftover.is_empty(),
        "failed create leaked environment state under {}: {leftover:?}",
        state.display()
    );
}

#[then(expr = "the canonical exec invocations should target the environment of {string}")]
fn then_canonical_exec_targets(world: &mut QuectoWorld, agent_id: String) {
    // exec.sh logs the QUECTO_CONTAINER_ENVIRONMENT_ID it was invoked with;
    // every recorded join must name the create-reported id of this agent's
    // environment, so a join routed at the wrong environment cannot pass.
    let expected = env_state_dir_for(world, &agent_id)
        .file_name()
        .expect("environment dir name")
        .to_string_lossy()
        .to_string();
    let log = canonical_state_dir(world).join("execs.log");
    let recorded = std::fs::read_to_string(&log)
        .unwrap_or_else(|e| panic!("no canonical exec log {}: {e}", log.display()));
    let lines: Vec<_> = recorded.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "no canonical exec invocations recorded in {}",
        log.display()
    );
    for line in &lines {
        assert_eq!(
            line.trim(),
            expected,
            "canonical exec targeted environment {line} instead of {expected}"
        );
    }
}

#[then(expr = "the child process for {string} should be running inside its environment checkout")]
fn then_child_runs_in_checkout(world: &mut QuectoWorld, agent_id: String) {
    // Runtime evidence, not a test-side disk read: the recorded child pid's
    // /proc cwd must resolve to the environment's checkout, proving the agent
    // process genuinely operates inside its isolated workspace.
    let uuid = world
        .agent_spawn_uuids
        .get(&agent_id)
        .unwrap_or_else(|| {
            panic!(
                "no captured uuid for {agent_id}: {:?}",
                world.agent_spawn_uuids
            )
        })
        .clone();
    let env_dir = env_state_dir_for(world, &agent_id);
    let children = env_dir.join("children.jsonl");
    let text = std::fs::read_to_string(&children)
        .unwrap_or_else(|e| panic!("no canonical children record {}: {e}", children.display()));
    let pid = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| {
            v["socket"]
                .as_str()
                .is_some_and(|socket| socket.contains(uuid.as_str()))
        })
        .and_then(|v| v["pid"].as_i64())
        .unwrap_or_else(|| {
            panic!("no recorded canonical child pid for {agent_id} ({uuid}): {text}")
        });
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
        .unwrap_or_else(|e| panic!("cannot read cwd of canonical child pid {pid}: {e}"));
    let checkout = env_dir
        .join("workspace")
        .join("repo")
        .canonicalize()
        .expect("canonical checkout path");
    assert_eq!(
        cwd.canonicalize().unwrap_or(cwd.clone()),
        checkout,
        "canonical child for {agent_id} runs in {} instead of its checkout {}",
        cwd.display(),
        checkout.display()
    );
}

#[when(expr = "the canonical child {string} is killed behind Quecto's back")]
fn when_canonical_child_killed(world: &mut QuectoWorld, agent_id: String) {
    let uuid = world
        .agent_spawn_uuids
        .get(&agent_id)
        .unwrap_or_else(|| {
            panic!(
                "no captured uuid for {agent_id}: {:?}",
                world.agent_spawn_uuids
            )
        })
        .clone();
    let children = env_state_dir_for(world, &agent_id).join("children.jsonl");
    let text = std::fs::read_to_string(&children)
        .unwrap_or_else(|e| panic!("no canonical children record {}: {e}", children.display()));
    let pid = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| {
            v["socket"]
                .as_str()
                .is_some_and(|socket| socket.contains(uuid.as_str()))
        })
        .and_then(|v| v["pid"].as_i64())
        .unwrap_or_else(|| {
            panic!("no recorded canonical child pid for {agent_id} ({uuid}): {text}")
        });
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("invoke kill");
    assert!(status.success(), "kill -9 {pid} failed");
}

// --- Then ---

#[then(expr = "the canonical runtime should have recorded exactly {int} create invocation(s)")]
fn then_canonical_creates(world: &mut QuectoWorld, expected: usize) {
    let log = canonical_state_dir(world).join("creates.log");
    let actual = count_lines(&log);
    assert_eq!(
        actual,
        expected,
        "expected {expected} canonical create invocation(s), found {actual} in {}",
        log.display()
    );
}

#[then(expr = "the canonical runtime should have recorded exactly {int} exec invocation(s)")]
fn then_canonical_execs(world: &mut QuectoWorld, expected: usize) {
    let log = canonical_state_dir(world).join("execs.log");
    let actual = count_lines(&log);
    assert_eq!(
        actual,
        expected,
        "expected {expected} canonical exec invocation(s), found {actual} in {}",
        log.display()
    );
}

#[then(
    expr = "the canonical runtime should have recorded exactly {int} {string} operation(s) for the environment of {string}"
)]
fn then_canonical_kill_ops(world: &mut QuectoWorld, expected: usize, op: String, agent_id: String) {
    // kill.sh records the OPERATION word (`kill` or `cleanup`) per invocation,
    // so an implementation that swaps kill_container onto the cleanup argv (or
    // vice versa) cannot satisfy a kill assertion with a cleanup run.
    let log = env_state_dir_for(world, &agent_id).join("kill.log");
    let recorded = std::fs::read_to_string(&log).unwrap_or_default();
    let actual = recorded.lines().filter(|l| l.trim() == op).count();
    assert_eq!(
        actual,
        expected,
        "expected {expected} canonical {op} operation(s) for {agent_id}, found {actual} in {} ({recorded:?})",
        log.display()
    );
}

#[then(
    expr = "the canonical runtime should have recorded exactly {int} inspect invocation(s) for the environment of {string}"
)]
fn then_canonical_inspects(world: &mut QuectoWorld, expected: usize, agent_id: String) {
    let log = env_state_dir_for(world, &agent_id).join("inspect.log");
    let actual = count_lines(&log);
    assert_eq!(
        actual,
        expected,
        "expected {expected} canonical inspect invocation(s) for {agent_id}, found {actual} in {}",
        log.display()
    );
}

#[then(expr = "the workspaces of {string} and {string} should be different")]
fn then_workspaces_differ(world: &mut QuectoWorld, a: String, b: String) {
    let wa = world
        .agent_workspaces
        .get(&a)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {a}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    let wb = world
        .agent_workspaces
        .get(&b)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {b}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    assert_ne!(
        wa, wb,
        "expected distinct workspaces for {a} and {b}, both reported {wa}"
    );
}

#[then(expr = "the workspace checkout for {string} should contain repository marker {string}")]
fn then_workspace_checkout_marker(world: &mut QuectoWorld, agent_id: String, marker: String) {
    let workspace = world
        .agent_workspaces
        .get(&agent_id)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {agent_id}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    let checkout = PathBuf::from(&workspace).join("repo").join("marker.txt");
    let content = std::fs::read_to_string(&checkout).unwrap_or_else(|e| {
        panic!(
            "workspace checkout marker {} unreadable for {agent_id}: {e}",
            checkout.display()
        )
    });
    assert!(
        content.contains(&marker),
        "expected repository marker {marker} in {}, found: {content}",
        checkout.display()
    );
}
