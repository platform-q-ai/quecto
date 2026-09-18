//! Diagnosable container failures (#2024 S4b): a failing create script's
//! stderr tail travels in the spawn error; the official Docker/Podman
//! `create.sh` carries a `--preflight-only` mode whose checks name each
//! missing prerequisite with its remedy; `quecto container doctor` runs
//! that preflight for the effective container config of the working
//! directory. The runtime is never real: a controlled PATH holds a fake
//! `podman` whose answer to `image exists` the scenario sets.
use super::*;

use crate::container_mapping_steps::{REAL_AGENT_RUN, config_set_local};

/// The official adapter under test, resolved from the workspace root.
fn official_create_script() -> PathBuf {
    workspace_script("scripts/container-runtime/docker/create.sh")
}

/// The host-local reference create script (the CI-exercised default).
fn host_local_create_script() -> PathBuf {
    workspace_script("scripts/container-runtime/create.sh")
}

fn workspace_script(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join(relative)
}

/// A fixture script, run through `bash` (a freshly written file executed
/// directly by a multi-threaded test process can race a concurrent fork
/// holding it open, ETXTBSY): the argv to configure.
fn write_script(path: &Path, body: &str) -> Vec<String> {
    std::fs::write(path, body).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

/// A fixture program that must be found on PATH (the fake `podman`).
fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(path).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(path, p).unwrap();
    }
}

/// Replace the global default entry's create script with `body`.
fn replace_default_create_script(world: &QuectoWorld, body: &str) {
    let config_path = PathBuf::from(world.config_path.clone().expect("config path"));
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let script = config_path.parent().unwrap().join("failing-create.sh");
    config["container_configs"]["default"]["create"] =
        serde_json::json!(write_script(&script, body));
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
}

#[given(
    expr = "the default container config's create script fails with status {int} after printing {string} on stderr"
)]
fn given_default_create_fails(world: &mut QuectoWorld, status: i32, message: String) {
    replace_default_create_script(
        world,
        &format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' 'container-runtime create: {message}' >&2\nexit {status}\n"
        ),
    );
}

#[given(
    expr = "the default container config's create script fails after printing {int} bytes of filler then an escape-coloured line {string} on stderr"
)]
fn given_default_create_fails_long(world: &mut QuectoWorld, filler: usize, marker: String) {
    replace_default_create_script(
        world,
        &format!(
            "#!/usr/bin/env bash\nhead -c {filler} /dev/zero | tr '\\0' 'x' >&2\nprintf '\\n\\033[31m%s\\033[0m\\r\\n' '{marker}' >&2\nexit 1\n"
        ),
    );
}

#[then("the spawn result should not contain a control character other than newline")]
fn then_spawn_result_has_no_control_chars(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let offending: Vec<char> = result
        .content
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(
        offending.is_empty(),
        "control characters {offending:?} in: {:?}",
        result.content
    );
}

#[then(expr = "the spawn result should be shorter than {int} bytes")]
fn then_spawn_result_shorter_than(world: &mut QuectoWorld, bound: usize) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.content.len() < bound,
        "{} bytes: {:?}",
        result.content.len(),
        result.content
    );
}

// ─── Controlled PATH ─────────────────────────────────────────────────────────

/// A directory holding only what the official create script needs besides
/// the container runtime, so the runtime's presence is the scenario's
/// decision and never the host's.
struct Toolbox {
    dir: PathBuf,
}

const TOOLBOX_PROGRAMS: &[&str] = &[
    "bash", "sh", "env", "jq", "git", "dirname", "basename", "mktemp", "realpath", "readlink",
    "id", "mkdir", "rm", "cat", "head", "tr", "timeout", "stat", "touch", "printf", "sleep",
    "true", "false",
];

impl Toolbox {
    /// The world's toolbox, built earlier in the scenario.
    fn existing(world: &QuectoWorld) -> Self {
        let dir = base_path(world).join("toolbox");
        assert!(
            dir.is_dir(),
            "no controlled PATH was built for this scenario"
        );
        Self { dir }
    }

    fn build(world: &QuectoWorld) -> Self {
        let dir = base_path(world).join("toolbox");
        std::fs::create_dir_all(&dir).unwrap();
        let host_path = std::env::var_os("PATH").expect("PATH");
        for program in TOOLBOX_PROGRAMS {
            let Some(found) = std::env::split_paths(&host_path)
                .map(|p| p.join(program))
                .find(|candidate| candidate.is_file())
            else {
                continue;
            };
            let link = dir.join(program);
            if !link.exists() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&found, &link).unwrap();
            }
        }
        Self { dir }
    }

    fn marker(&self) -> PathBuf {
        self.dir.join("image-present")
    }

    fn podman_log(&self) -> PathBuf {
        self.dir.join("podman.log")
    }

    /// A fake `podman` that records every invocation and answers `image
    /// exists` from the marker file; anything else (a pull, a run) is
    /// recorded and refused so a test can prove it never happened.
    fn install_fake_podman(&self, image_present: bool) {
        let body = format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = image ] && [ \"$2\" = exists ]; then [ -e '{}' ] && exit 0 || exit 1; fi\nexit 125\n",
            self.podman_log().display(),
            self.marker().display()
        );
        write_executable(&self.dir.join("podman"), &body);
        self.set_image_present(image_present);
    }

    fn set_image_present(&self, present: bool) {
        if present {
            std::fs::write(self.marker(), "").unwrap();
        } else {
            let _ = std::fs::remove_file(self.marker());
        }
    }
}

/// A local repository with one commit, reachable through `git ls-remote`.
fn reachable_repository(world: &QuectoWorld) -> PathBuf {
    let repo = base_path(world).join("origin-repo");
    if repo.join(".git").exists() {
        return repo;
    }
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("README"), "origin\n").unwrap();
    git(&["add", "README"]);
    git(&[
        "-c",
        "user.name=bdd",
        "-c",
        "user.email=bdd@example.test",
        "commit",
        "-q",
        "-m",
        "origin",
    ]);
    repo
}

const UNREACHABLE_REPOSITORY: &str = "/nonexistent/quecto-bdd/unreachable-repo.git";

fn toolbox_with_fake_podman(world: &QuectoWorld, image_present: bool) -> Toolbox {
    let toolbox = Toolbox::build(world);
    toolbox.install_fake_podman(image_present);
    toolbox
}

#[given(expr = "a controlled PATH whose fake podman reports every image as {word}")]
fn given_controlled_path_fake_podman(world: &mut QuectoWorld, state: String) {
    toolbox_with_fake_podman(world, image_state(&state));
}

fn image_state(word: &str) -> bool {
    match word {
        "present" => true,
        "missing" => false,
        other => panic!("image state {other:?} is not present|missing"),
    }
}

/// The preflight's exit code and streams land in the world's CLI slots.
fn run_preflight(world: &mut QuectoWorld, script: &Path, image: Option<&str>, repo: &Path) {
    let toolbox = Toolbox::existing(world).dir;
    let state_dir = preflight_state_dir(world);
    let mut cmd = std::process::Command::new(script);
    cmd.arg("--state-dir")
        .arg(&state_dir)
        .arg("--repo")
        .arg(repo);
    if let Some(image) = image {
        cmd.args(["--image", image]);
    }
    let output = cmd
        .arg("--preflight-only")
        .env_clear()
        .env("PATH", &toolbox)
        .env("HOME", base_path(world))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run create.sh --preflight-only");
    world.exit_code = output.status.code().unwrap_or(-1);
    world.stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.stderr = String::from_utf8_lossy(&output.stderr).to_string();
}

#[when(
    expr = "I run the official docker create script with --preflight-only for image {string} and a reachable local repository"
)]
fn when_preflight_reachable(world: &mut QuectoWorld, image: String) {
    let repo = reachable_repository(world);
    run_preflight(world, &official_create_script(), Some(&image), &repo);
}

/// A state dir the preflight is asked about but must never create.
fn preflight_state_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("doctor-state")
}

#[then("the preflight should have created no state directory")]
fn then_preflight_created_nothing(world: &mut QuectoWorld) {
    let state_dir = preflight_state_dir(world);
    assert!(
        !state_dir.exists(),
        "--preflight-only created {}",
        state_dir.display()
    );
}

#[when(
    "I run the host-local reference create script with --preflight-only and an unreachable repository"
)]
fn when_host_local_preflight_unreachable(world: &mut QuectoWorld) {
    run_preflight(
        world,
        &host_local_create_script(),
        None,
        Path::new(UNREACHABLE_REPOSITORY),
    );
}

#[when(
    expr = "I run the official docker create script with --preflight-only for image {string} and an unreachable repository"
)]
fn when_preflight_unreachable(world: &mut QuectoWorld, image: String) {
    run_preflight(
        world,
        &official_create_script(),
        Some(&image),
        Path::new(UNREACHABLE_REPOSITORY),
    );
}

/// One `status<TAB>check<TAB>detail<TAB>remedy` line of the preflight
/// report.
fn preflight_line(world: &QuectoWorld, check: &str) -> (String, String, String) {
    world
        .stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() == 4 && fields[1] == check).then(|| {
                (
                    fields[0].to_string(),
                    fields[2].to_string(),
                    fields[3].to_string(),
                )
            })
        })
        .next()
        .unwrap_or_else(|| {
            panic!(
                "no preflight line for check {check:?}\nstdout: {}\nstderr: {}",
                world.stdout, world.stderr
            )
        })
}

#[then("the preflight should exit with a non-zero status")]
fn then_preflight_failed(world: &mut QuectoWorld) {
    assert!(
        world.exit_code > 0,
        "status {}\nstdout: {}\nstderr: {}",
        world.exit_code,
        world.stdout,
        world.stderr
    );
}

#[then(expr = "the preflight should report check {string} as failed naming {string}")]
fn then_preflight_check_failed_naming(world: &mut QuectoWorld, check: String, needle: String) {
    let (status, detail, _) = preflight_line(world, &check);
    assert_eq!(status, "fail", "{check}: {detail}");
    assert!(detail.contains(&needle), "{check}: {detail}");
}

#[then(
    expr = "the preflight should report check {string} as failed naming the unreachable repository"
)]
fn then_preflight_repo_failed(world: &mut QuectoWorld, check: String) {
    let (status, detail, _) = preflight_line(world, &check);
    assert_eq!(status, "fail", "{check}: {detail}");
    assert!(detail.contains(UNREACHABLE_REPOSITORY), "{check}: {detail}");
}

#[then(expr = "the preflight should report a remedy for check {string} mentioning {string}")]
fn then_preflight_remedy(world: &mut QuectoWorld, check: String, needle: String) {
    let (_, _, remedy) = preflight_line(world, &check);
    assert!(remedy.contains(&needle), "{check} remedy: {remedy}");
}

#[then(expr = "the preflight should report check {string} as passed")]
fn then_preflight_check_passed(world: &mut QuectoWorld, check: String) {
    let (status, detail, _) = preflight_line(world, &check);
    assert_eq!(status, "ok", "{check}: {detail}");
}

#[then("the fake podman should never have been asked to pull")]
fn then_podman_never_pulled(world: &mut QuectoWorld) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    assert!(
        log.lines().any(|line| line.starts_with("image exists ")),
        "the image check never reached podman: {log:?}"
    );
    assert!(
        !log.lines()
            .any(|line| line.starts_with("pull ") || line.starts_with("run ")),
        "podman was asked to pull or run: {log:?}"
    );
}

// ─── quecto container doctor ─────────────────────────────────────────────────

/// Bind the checkout to config `official`: the official create script
/// behind a wrapper that pins PATH to the toolbox, with a state dir, the
/// reachable repository and the image under test.
fn bind_official_config(world: &mut QuectoWorld, toolbox: &Toolbox) {
    let repo = reachable_repository(world);
    // The wrapper pins PATH and clears every knob the host's environment
    // could leak into an in-process doctor run, so the scenario alone
    // decides what the preflight finds.
    let wrapper = base_path(world).join("official-create.sh");
    let mut create = write_script(
        &wrapper,
        &format!(
            "export PATH='{}'\nunset QUECTO_CONTAINER_CLI QUECTO_DOCKER_IMAGE QUECTO_REPO_CHECK_TIMEOUT\nexport GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null\nexec '{}' \"$@\"\n",
            toolbox.dir.display(),
            official_create_script().display()
        ),
    );
    create.extend([
        "--state-dir".to_string(),
        base_path(world)
            .join("doctor-state")
            .to_string_lossy()
            .into_owned(),
        "--repo".to_string(),
        repo.to_string_lossy().into_owned(),
        "--image".to_string(),
        "quecto-box:local".to_string(),
    ]);
    let cleanup = write_script(&base_path(world).join("official-cleanup.sh"), "exit 0\n");
    let entry = serde_json::json!({
        "default": true,
        "create": create,
        "cleanup": cleanup,
    });
    config_set_local(world, "official", &entry);
}

#[given(
    "the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH without any container runtime"
)]
fn given_bound_without_runtime(world: &mut QuectoWorld) {
    let toolbox = Toolbox::build(world);
    bind_official_config(world, &toolbox);
}

#[given(
    expr = "the checkout binds itself to the official docker create script with a reachable local repository under a controlled PATH whose fake podman reports every image as {word}"
)]
fn given_bound_with_fake_podman(world: &mut QuectoWorld, state: String) {
    let toolbox = toolbox_with_fake_podman(world, image_state(&state));
    bind_official_config(world, &toolbox);
}

#[when(expr = "the fake podman is fixed to report every image as {word}")]
fn when_fake_podman_fixed(world: &mut QuectoWorld, state: String) {
    Toolbox::existing(world).set_image_present(image_state(&state));
}

#[given("the checkout binds itself to a create script that rejects --preflight-only")]
fn given_bound_without_preflight_support(world: &mut QuectoWorld) {
    let create = write_script(
        &base_path(world).join("legacy-create.sh"),
        "echo 'legacy create: unknown argument --preflight-only' >&2\nexit 1\n",
    );
    let cleanup = write_script(&base_path(world).join("legacy-cleanup.sh"), "exit 0\n");
    let entry = serde_json::json!({
        "default": true,
        "create": create,
        "cleanup": cleanup,
    });
    config_set_local(world, "legacy", &entry);
}

/// The doctor's line for `check`: `✓`/`✗`/`!` then the check name.
fn doctor_line(world: &QuectoWorld, check: &str) -> String {
    world
        .stdout
        .lines()
        .find(|line| {
            let mut words = line.split_whitespace();
            matches!(words.next(), Some("✓" | "✗" | "!")) && words.next() == Some(check)
        })
        .map(str::to_string)
        .unwrap_or_else(|| {
            panic!(
                "no doctor line for check {check:?}\nstdout: {}\nstderr: {}",
                world.stdout, world.stderr
            )
        })
}

#[then(
    expr = "the doctor output should show check {string} as failed with a remedy mentioning {string}"
)]
fn then_doctor_check_failed(world: &mut QuectoWorld, check: String, needle: String) {
    let line = doctor_line(world, &check);
    assert!(line.starts_with("  ✗"), "{line}");
    // The remedy is the indented line that follows the check.
    let remedy = world
        .stdout
        .lines()
        .skip_while(|l| *l != line)
        .nth(1)
        .unwrap_or_default();
    assert!(
        remedy.trim_start().starts_with("remedy: ") && remedy.contains(&needle),
        "expected a remedy mentioning {needle:?} after {line:?}, got {remedy:?}\nstdout: {}",
        world.stdout
    );
}

#[then(expr = "the doctor output should name container config {string}")]
fn then_doctor_names_config(world: &mut QuectoWorld, name: String) {
    let expected = format!("container config \"{name}\"");
    assert!(
        world.stdout.contains(&expected),
        "expected {expected:?} in stdout: {}",
        world.stdout
    );
}

#[then(expr = "the doctor output should show check {string} as passed")]
fn then_doctor_check_passed(world: &mut QuectoWorld, check: String) {
    let line = doctor_line(world, &check);
    assert!(line.starts_with("  ✓"), "{line}");
}

/// Warnings (no `gh` on the controlled PATH) are allowed; failures are not.
#[then("the doctor output should show no failed check")]
fn then_doctor_no_failure(world: &mut QuectoWorld) {
    let checks: Vec<&str> = world
        .stdout
        .lines()
        .filter(|line| {
            let first = line.split_whitespace().next();
            matches!(first, Some("✓" | "✗" | "!"))
        })
        .collect();
    assert!(!checks.is_empty(), "no checks in stdout: {}", world.stdout);
    for line in checks {
        assert!(!line.starts_with("  ✗"), "{line}\nstdout: {}", world.stdout);
    }
    assert!(
        world.stdout.contains("0 checks failed"),
        "stdout: {}",
        world.stdout
    );
}

#[then(expr = "the tool result the fake provider received should be a spawn error naming {string}")]
fn then_provider_saw_spawn_error_naming(_world: &mut QuectoWorld, needle: String) {
    let run = REAL_AGENT_RUN.lock().unwrap();
    let run = run.as_ref().expect("the real agent ran");
    let results = run.tool_results.lock().unwrap();
    assert!(
        results.iter().any(
            |content| content.contains("script-managed create failed with status")
                && content.contains(&needle)
        ),
        "expected a spawn error naming {needle:?} among the tool results the model saw: {results:?}\nstderr: {}",
        run.stderr
    );
}
