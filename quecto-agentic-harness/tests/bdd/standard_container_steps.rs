//! The standard container landed (#2024 S4e): `quecto container init`
//! materialises the embedded bundle (the official adapter scripts plus
//! the Containerfile) under `<project>/.quecto/containers/standard`,
//! binds the project through the trusted overlay with argv naming
//! exactly those files, derives `--repo` from the checkout's origin;
//! `quecto container status` reports the standing; the materialised
//! create script honours the S4b preflight and S4c swarm contracts. The
//! runtime is never real: a controlled PATH holds a fake `podman`.
use super::*;

use crate::container_doctor_steps::{
    Toolbox, preflight_state_dir, reachable_repository, write_executable,
};
use std::process::Command;

fn checkout(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

fn assets_dir(world: &QuectoWorld) -> PathBuf {
    checkout(world).join(".quecto/containers/standard")
}

fn overlay_path(world: &QuectoWorld) -> PathBuf {
    checkout(world).join(".quecto/config.json")
}

fn overlay_entry(world: &QuectoWorld, name: &str) -> serde_json::Value {
    let overlay: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(overlay_path(world)).unwrap()).unwrap();
    overlay["container_configs"][name].clone()
}

fn argv(entry: &serde_json::Value, key: &str) -> Vec<String> {
    entry[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} argv in {entry}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

const ASSETS: [&str; 5] = [
    "Containerfile",
    "scripts/create.sh",
    "scripts/exec.sh",
    "scripts/inspect.sh",
    "scripts/kill.sh",
];

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

#[given(
    "the current directory is a git checkout whose origin remote is a reachable local repository"
)]
fn given_checkout_with_origin(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let origin = reachable_repository(world);
    let checkout = checkout(world);
    git(&checkout, &["init", "-q"]);
    git(
        &checkout,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
}

#[given(expr = "the current directory is a git checkout whose origin remote is {string}")]
fn given_checkout_with_named_origin(world: &mut QuectoWorld, url: String) {
    ensure_temp_dir(world);
    let checkout = checkout(world);
    git(&checkout, &["init", "-q"]);
    git(&checkout, &["remote", "add", "origin", &url]);
}

#[given(expr = "the checkout's {string} is a symbolic link to a directory outside the checkout")]
fn given_symlinked_dir(world: &mut QuectoWorld, relative: String) {
    let outside = base_path(world).join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let link = checkout(world).join(&relative);
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
}

#[then("the directory outside the checkout should still be empty")]
fn then_outside_empty(world: &mut QuectoWorld) {
    let outside = base_path(world).join("outside");
    let entries: Vec<_> = std::fs::read_dir(&outside)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(entries.is_empty(), "{entries:?}");
}

#[given("the current directory is a git checkout without an origin remote")]
fn given_checkout_without_origin(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    git(&checkout(world), &["init", "-q"]);
}

#[given(expr = "the global configuration already labels container config {string} as the default")]
fn given_global_default(world: &mut QuectoWorld, name: String) {
    let config = base_path(world).join("config.json");
    std::fs::write(
        &config,
        serde_json::json!({"container_configs": {name: {
            "default": true, "create": ["/bin/true", "--repo", "https://example.test/other"], "cleanup": ["/bin/true"]
        }}})
        .to_string(),
    )
    .unwrap();
}

/// An overlay entry carrying the default label, written the agent's way
/// (`quecto config set --local`, trusted), over a global file with no
/// container config: the label init must take over (#2035).
#[given(
    expr = "the checkout binds itself to default container config {string} with repository {string} through quecto config set --local"
)]
fn given_checkout_binds_default_entry(world: &mut QuectoWorld, name: String, repo: String) {
    let entry = serde_json::json!({
        "default": true,
        "create": ["/bin/true", "--repo", repo],
        "cleanup": ["/bin/true"],
    });
    crate::container_mapping_steps::config_set_local(world, &name, &entry);
}

#[given(expr = "the checkout carries an untrusted overlay declaring container config {string}")]
fn given_untrusted_overlay(world: &mut QuectoWorld, name: String) {
    let overlay = overlay_path(world);
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(
        &overlay,
        serde_json::json!({"container_configs": {name: {
            "default": true, "create": ["/bin/true"], "cleanup": ["/bin/true"]
        }}})
        .to_string(),
    )
    .unwrap();
}

#[then(
    expr = "the standard container assets should be materialised under {string} byte-identical to the official adapter"
)]
fn then_assets_materialised(world: &mut QuectoWorld, relative: String) {
    let dir = checkout(world).join(&relative);
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    for (asset, source) in ASSETS.iter().zip([
        "quecto-agentic-harness/assets/standard-container/Containerfile",
        "scripts/container-runtime/docker/create.sh",
        "scripts/container-runtime/docker/exec.sh",
        "scripts/container-runtime/docker/inspect.sh",
        "scripts/container-runtime/docker/kill.sh",
    ]) {
        let path = dir.join(asset);
        let expected = std::fs::read(workspace_root.join(source)).unwrap();
        let actual = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{} was not materialised: {e}", path.display()));
        assert_eq!(actual, expected, "{} differs from {source}", path.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o111;
            assert_eq!(
                mode != 0,
                asset.starts_with("scripts/"),
                "{} executable bits: {mode:o}",
                path.display()
            );
        }
    }
}

#[then("the output should list every materialised standard container asset")]
fn then_output_lists_assets(world: &mut QuectoWorld) {
    for asset in ASSETS {
        let path = assets_dir(world).join(asset);
        let line = format!("wrote  {}", path.display());
        assert!(
            world.stdout.contains(&line),
            "expected {line:?} in stdout:\n{}",
            world.stdout
        );
    }
}

#[then(expr = "no standard container asset should exist under {string}")]
fn then_no_assets(world: &mut QuectoWorld, relative: String) {
    let dir = checkout(world).join(&relative);
    assert!(!dir.exists(), "{} exists", dir.display());
}

#[then("the checkout should carry no overlay")]
fn then_no_overlay(world: &mut QuectoWorld) {
    assert!(!overlay_path(world).exists());
}

#[then(expr = "{string} should appear in no file under the checkout's {string}")]
fn then_secret_in_no_file(world: &mut QuectoWorld, secret: String, relative: String) {
    fn walk(dir: &Path, secret: &str) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, secret);
            } else if let Ok(bytes) = std::fs::read(&path) {
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(secret),
                    "{secret} appears in {}",
                    path.display()
                );
            }
        }
    }
    walk(&checkout(world).join(&relative), &secret);
}

#[then("the checkout's overlay should be trusted")]
fn then_overlay_trusted(world: &mut QuectoWorld) {
    let output = cli::run_with_output(vec!["quecto".into(), "status".into()], &world.cli_context);
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    assert!(
        output.stdout.contains("trusted") && !output.stdout.contains("untrusted"),
        "quecto status:\n{}",
        output.stdout
    );
}

#[then(
    expr = "the overlay entry {string} should be the default and its create argv should be the materialised create script with {string} under the base directory, {string} the origin remote and {string} {string}"
)]
fn then_entry_default_create(
    world: &mut QuectoWorld,
    name: String,
    state_flag: String,
    repo_flag: String,
    image_flag: String,
    image: String,
) {
    let entry = overlay_entry(world, &name);
    assert_eq!(entry["default"], serde_json::json!(true), "{entry}");
    let origin = reachable_repository(world).to_string_lossy().into_owned();
    assert_eq!(
        argv(&entry, "create"),
        [
            assets_dir(world)
                .join("scripts/create.sh")
                .to_string_lossy()
                .into_owned(),
            state_flag,
            base_path(world)
                .join("container-environments")
                .to_string_lossy()
                .into_owned(),
            repo_flag,
            origin,
            image_flag,
            image
        ]
    );
}

#[then(
    expr = "the overlay entry {string} should carry exec, inspect, kill and cleanup argv naming the materialised scripts"
)]
fn then_entry_other_argv(world: &mut QuectoWorld, name: String) {
    let entry = overlay_entry(world, &name);
    let scripts = assets_dir(world).join("scripts");
    let state = base_path(world)
        .join("container-environments")
        .to_string_lossy()
        .into_owned();
    let script = |n: &str| scripts.join(n).to_string_lossy().into_owned();
    assert_eq!(
        argv(&entry, "exec"),
        [script("exec.sh"), "--state-dir".into(), state.clone()]
    );
    assert_eq!(
        argv(&entry, "inspect"),
        [script("inspect.sh"), "--state-dir".into(), state.clone()]
    );
    assert_eq!(
        argv(&entry, "kill"),
        [
            script("kill.sh"),
            "--state-dir".into(),
            state.clone(),
            "--op".into(),
            "kill".into()
        ]
    );
    assert_eq!(
        argv(&entry, "cleanup"),
        [
            script("kill.sh"),
            "--state-dir".into(),
            state,
            "--op".into(),
            "cleanup".into()
        ]
    );
}

#[then(expr = "no argv of the overlay entry {string} should end with {string}")]
fn then_no_trailing(world: &mut QuectoWorld, name: String, tail: String) {
    let entry = overlay_entry(world, &name);
    for key in ["create", "exec", "inspect", "kill", "cleanup"] {
        assert_ne!(argv(&entry, key).last().unwrap(), &tail, "{key}");
    }
}

#[then(expr = "the overlay entry {string} should not be the default")]
fn then_entry_not_default(world: &mut QuectoWorld, name: String) {
    let entry = overlay_entry(world, &name);
    assert!(entry.is_object(), "no entry {name}");
    assert!(entry.get("default").is_none_or(|d| d == false), "{entry}");
}

#[then(expr = "the effective default container config should still be {string}")]
fn then_effective_default(world: &mut QuectoWorld, name: String) {
    let output = cli::run_with_output(
        vec![
            "quecto".into(),
            "config".into(),
            "get".into(),
            "--effective".into(),
            "container_configs".into(),
        ],
        &world.cli_context,
    );
    assert_eq!(output.exit_code, 0, "{}", output.stderr);
    let configs: serde_json::Value = serde_json::from_str(output.stdout.trim()).unwrap();
    let defaults: Vec<&String> = configs
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, entry)| entry["default"] == true)
        .map(|(name, _)| name)
        .collect();
    assert_eq!(defaults, [&name], "{configs}");
    assert!(configs["standard"].is_object(), "{configs}");
}

#[then(expr = "the overlay entry {string} create argv should carry no {string}")]
fn then_create_without(world: &mut QuectoWorld, name: String, flag: String) {
    let create = argv(&overlay_entry(world, &name), "create");
    assert!(!create.contains(&flag), "{create:?}");
}

#[then(expr = "the overlay entry {string} create argv should carry {string} {string}")]
fn then_create_with(world: &mut QuectoWorld, name: String, flag: String, value: String) {
    let create = argv(&overlay_entry(world, &name), "create");
    let at = create
        .iter()
        .position(|a| *a == flag)
        .unwrap_or_else(|| panic!("{flag} not in {create:?}"));
    assert_eq!(create.get(at + 1), Some(&value), "{create:?}");
}

#[then("I remember the checkout's overlay bytes")]
fn then_remember_overlay(world: &mut QuectoWorld) {
    let bytes = std::fs::read(overlay_path(world)).unwrap();
    std::fs::write(base_path(world).join("overlay-before"), bytes).unwrap();
}

#[then("the checkout's overlay bytes should be unchanged")]
fn then_overlay_unchanged(world: &mut QuectoWorld) {
    let before = std::fs::read(base_path(world).join("overlay-before")).unwrap();
    let after = std::fs::read(overlay_path(world)).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&before),
        String::from_utf8_lossy(&after)
    );
}

// ─── The real binary under a controlled PATH ─────────────────────────────────

#[when(expr = "I run the real quecto binary under the controlled PATH with arguments {string}")]
fn when_real_binary(world: &mut QuectoWorld, args_str: String) {
    let toolbox = Toolbox::existing(world).dir;
    let output = Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(shell_split(&args_str))
        .current_dir(checkout(world))
        .env_clear()
        .env("PATH", &toolbox)
        .env("HOME", base_path(world))
        .env("QUECTO_BASE_DIR", base_path(world))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run the real quecto binary");
    world.exit_code = output.status.code().unwrap_or(-1);
    world.stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.stderr = String::from_utf8_lossy(&output.stderr).to_string();
}

#[when(
    "I run the materialised standard create script with --preflight-only under the controlled PATH"
)]
fn when_materialised_preflight(world: &mut QuectoWorld) {
    let toolbox = Toolbox::existing(world).dir;
    let entry = overlay_entry(world, "standard");
    let create = argv(&entry, "create");
    // The entry's own argv, with its state dir swapped for the one the
    // scenario asserts is never created.
    let mut args: Vec<String> = create[1..].to_vec();
    let at = args.iter().position(|a| a == "--state-dir").unwrap();
    args[at + 1] = preflight_state_dir(world).to_string_lossy().into_owned();
    let output = Command::new(&create[0])
        .args(&args)
        .arg("--preflight-only")
        .env_clear()
        .env("PATH", &toolbox)
        .env("HOME", base_path(world))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run the materialised create.sh --preflight-only");
    world.exit_code = output.status.code().unwrap_or(-1);
    world.stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.stderr = String::from_utf8_lossy(&output.stderr).to_string();
}

/// A fake `podman` that answers `image exists` from the marker, records
/// every invocation, and accepts `run` (printing a container id) so a
/// create completes without a runtime.
#[given("a controlled PATH whose fake podman reports every image as present and accepts every run")]
fn given_fake_podman_accepting_run(world: &mut QuectoWorld) {
    let toolbox = Toolbox::build(world);
    let body = format!(
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = image ] && [ \"$2\" = exists ]; then exit 0; fi\nif [ \"$1\" = run ] && [ \"${{2:-}}\" = --rm ]; then exit 0; fi\nif [ \"$1\" = run ]; then echo deadbeef; exit 0; fi\nif [ \"$1\" = rm ]; then exit 0; fi\nexit 125\n",
        toolbox.podman_log().display()
    );
    write_executable(&toolbox.dir.join("podman"), &body);
}

#[when("I run the materialised standard create script for a fake child under the controlled PATH")]
fn when_materialised_create(world: &mut QuectoWorld) {
    let toolbox = Toolbox::existing(world).dir;
    let entry = overlay_entry(world, "standard");
    let create = argv(&entry, "create");
    let socket_dir = base_path(world).join("sockets");
    std::fs::create_dir_all(&socket_dir).unwrap();
    let child = base_path(world).join("fake-child");
    write_executable(&child, "#!/usr/bin/env bash\nexit 0\n");
    let output = Command::new(&create[0])
        .args(&create[1..])
        .arg("--")
        .arg(&child)
        .arg("agent")
        .arg("--socket")
        .arg(socket_dir.join("child.sock"))
        .env_clear()
        .env("PATH", &toolbox)
        .env("HOME", base_path(world))
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "C1")
        .env("QUECTO_CONTAINER_CONFIG", "standard")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run the materialised create.sh");
    world.exit_code = output.status.code().unwrap_or(-1);
    world.stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.stderr = String::from_utf8_lossy(&output.stderr).to_string();
}

#[then("the create should have succeeded reporting the clone as its checkout")]
fn then_create_succeeded(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "stdout: {}\nstderr: {}",
        world.stdout, world.stderr
    );
    let result: serde_json::Value = serde_json::from_str(world.stdout.trim())
        .unwrap_or_else(|e| panic!("create result is not JSON ({e}): {}", world.stdout));
    let checkout = result["metadata"]["checkout"].as_str().unwrap();
    assert!(checkout.ends_with("/workspace/repo"), "{result}");
    assert!(
        Path::new(checkout).join("README").exists(),
        "the clone at {checkout} has no README"
    );
    assert_eq!(result["metadata"]["source"], "repo");
    assert_eq!(result["metadata"]["runtime"], "podman");
}

#[then(expr = "the fake podman run argv should carry {string}")]
fn then_run_argv_carries(world: &mut QuectoWorld, needle: String) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    let run = log
        .lines()
        .find(|line| line.starts_with("run ") && !line.starts_with("run --rm "))
        .unwrap_or_else(|| panic!("podman was never asked to run a child: {log:?}"));
    assert!(run.contains(&needle), "{needle:?} not in: {run}");
}

#[then("the fake podman should never have been asked to pull an image")]
fn then_never_pulled(world: &mut QuectoWorld) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    assert!(
        !log.lines().any(|line| line.starts_with("pull ")
            || line.contains("--pull=always")
            || line.contains("--pull=missing")
            || line.contains("--pull=newer")),
        "podman was asked to pull: {log:?}"
    );
}

// ─── The project must be the checkout's root (#2024 S4e, review F4) ─────────

#[given(expr = "the checkout has a subdirectory {string}")]
fn given_subdirectory(world: &mut QuectoWorld, relative: String) {
    std::fs::create_dir_all(checkout(world).join(relative)).unwrap();
}

#[given(expr = "{string} below the checkout is its own git checkout without an origin remote")]
fn given_nested_checkout(world: &mut QuectoWorld, relative: String) {
    git(&checkout(world).join(relative), &["init", "-q"]);
}

#[then("the output should contain the build command with the bundle directory single-quoted")]
fn then_build_command_quoted(world: &mut QuectoWorld) {
    let dir = checkout(world)
        .canonicalize()
        .unwrap()
        .join("my projects/repo one/.quecto/containers/standard");
    // The bundle's template is `-f {dir}/Containerfile {dir}`: each
    // `{dir}` is one quoted word, so the shell reads
    // `'…/standard'/Containerfile` as one path.
    let expected = format!(
        "podman build -t quecto-dev:local -f '{}'/Containerfile '{}'",
        dir.display(),
        dir.display()
    );
    assert!(
        world.stdout.contains(&expected),
        "expected {expected:?} in: {}",
        world.stdout
    );
}

/// `<checkout>` in the arguments is the hermetic checkout's absolute path.
#[when(expr = "I run quecto with arguments {string} where <checkout> is the checkout")]
fn when_run_with_checkout(world: &mut QuectoWorld, args_str: String) {
    let checkout = checkout(world).canonicalize().unwrap();
    let args = args_str.replace("<checkout>", &checkout.to_string_lossy());
    let mut argv = vec!["quecto".to_string()];
    argv.extend(shell_split(&args));
    let output = cli::run_with_output(argv, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[then("the stderr should name the checkout as the repository root")]
fn then_stderr_names_root(world: &mut QuectoWorld) {
    let checkout = checkout(world).canonicalize().unwrap();
    let expected = format!("pass --project {}", checkout.display());
    assert!(
        world.stderr.contains(&expected),
        "expected {expected:?} in stderr: {}",
        world.stderr
    );
}

// ─── The trust boundary at launch (#2024 S4e, review F2) ─────────────────────

/// The launcher composed for the checkout, over a global file that
/// declares no container config, so the standard entry init writes is the
/// one `container: true` selects.
#[given(
    "script-managed subagent spawning is available from the checkout with no global container config"
)]
fn given_spawn_from_checkout_no_global_configs(world: &mut QuectoWorld) {
    crate::container_mapping_steps::given_script_spawn_from_checkout(world, "default".into());
    let config = PathBuf::from(world.config_path.clone().expect("config path"));
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    document
        .as_object_mut()
        .unwrap()
        .remove("container_configs");
    std::fs::write(&config, serde_json::to_string_pretty(&document).unwrap()).unwrap();
}

fn invocation_marker(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("create-invoked")
}

/// The edit a pull or a hand could make: the script records each run in
/// a marker file and stops, so "never invoked" is a fact about the host
/// and a regression of the check can never reach the real runtime.
#[when("the materialised standard create script is edited to record every invocation")]
fn when_create_script_edited(world: &mut QuectoWorld) {
    let create = assets_dir(world).join("scripts/create.sh");
    let original = std::fs::read_to_string(&create).unwrap();
    let (shebang, rest) = original.split_once('\n').unwrap();
    std::fs::write(
        &create,
        format!(
            "{shebang}\ntouch '{}'\nexit 99\n{rest}",
            invocation_marker(world).display()
        ),
    )
    .unwrap();
}

#[then(
    expr = "the spawn result should name the materialised {string} as differing from the standard bundle"
)]
fn then_spawn_names_differing_script(world: &mut QuectoWorld, relative: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    let expected = format!(
        "{} differs from the standard bundle this quecto embeds",
        assets_dir(world).join(&relative).display()
    );
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then("the materialised standard create script should never have been invoked")]
fn then_create_never_invoked(world: &mut QuectoWorld) {
    let marker = invocation_marker(world);
    assert!(
        !marker.exists(),
        "the edited create script ran on the host ({} exists)",
        marker.display()
    );
}

// ─── The retained argv is judged too (#2024 S4e, review round 2) ─────────────

/// Where the fake podman keeps the pid of each "container" it ran.
fn fake_containers_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("toolbox").join("containers")
}

/// A fake `podman` whose `run` starts the command after the image on the
/// host (tracked like every fixture child, so teardown can end it),
/// whose `inspect` answers from that process and whose `rm -f` ends it —
/// enough runtime for a create to commit an environment and a kill to
/// tear it down, with every invocation recorded.
#[given(
    "a controlled PATH whose fake podman reports every image as present and runs each container's command on the host"
)]
fn given_fake_podman_running_on_host(world: &mut QuectoWorld) {
    let toolbox = Toolbox::build(world);
    let containers = fake_containers_dir(world);
    std::fs::create_dir_all(&containers).unwrap();
    let cfg_dir = PathBuf::from(world.config_path.clone().expect("config path"))
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::write(
        cfg_dir.join("fixture-processes.py"),
        include_str!("fixture_processes.py"),
    )
    .unwrap();
    let pid_dir = cfg_dir.join("env-pids");
    std::fs::create_dir_all(&pid_dir).unwrap();
    let body = format!(
        r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> '{log}'
case "$1" in
  image) [ "$2" = exists ] && exit 0; exit 1 ;;
  run)
    shift
    # The standard preflight's short-lived allowlisted development-tool
    # probe succeeds in this runtime fixture; lifecycle runs continue below.
    if [ "${{1:-}}" = --rm ]; then exit 0; fi
    name=""
    while [ "$#" -gt 0 ] && [ "$1" != quecto-dev:local ]; do
      [ "$1" = --name ] && name="$2"
      shift
    done
    [ "$1" = quecto-dev:local ] || exit 125
    shift
    setsid "$@" >/dev/null 2>&1 </dev/null &
    pid=$!
    printf '%s\n' "$pid" > '{containers}'/"$name"
    python3 '{pid_dir}/../fixture-processes.py' track '{pid_dir}' "$name" "$pid"
    echo "$name"
    exit 0 ;;
  inspect)
    name="${{@: -1}}"
    pid="$(cat '{containers}'/"$name" 2>/dev/null || true)"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then echo "true 0 false"; else echo "false 0 false"; fi
    exit 0 ;;
  rm)
    name="${{@: -1}}"
    pid="$(cat '{containers}'/"$name" 2>/dev/null || true)"
    [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null || true
    exit 0 ;;
esac
exit 125
"#,
        log = toolbox.podman_log().display(),
        containers = containers.display(),
        pid_dir = pid_dir.display(),
    );
    write_executable(&toolbox.dir.join("podman"), &body);
}

/// The launcher composed in the checkout, with `kill_container` wired to
/// the same environment registry. The standard scripts run with this
/// process's environment, so the controlled PATH is put in front of it
/// for the scenario (restored at teardown) and `podman` resolves to the
/// fake.
#[given(
    "script-managed subagent spawning through the fake podman is available from the checkout with no global container config"
)]
fn given_spawn_through_fake_podman(world: &mut QuectoWorld) {
    given_spawn_from_checkout_no_global_configs(world);
    crate::spawn_env_steps::wire_environment_control(world, None);
    let mut path = vec![Toolbox::existing(world).dir];
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH"),
    ));
    let path = std::env::join_paths(path).unwrap();
    world.delegated_subtree.override_env("PATH", &path);
}

/// The launcher over the fake podman, keeping the S4a global set (a
/// labelled `default` plus `alternate`): the rig for #2035, where the
/// repo's `standard` entry must win over that global default.
#[given(
    expr = "script-managed subagent spawning through the fake podman is available from the checkout with global default script {string}"
)]
fn given_spawn_through_fake_podman_with_global(world: &mut QuectoWorld, script: String) {
    crate::container_mapping_steps::given_script_spawn_from_checkout(world, script);
    crate::spawn_env_steps::wire_environment_control(world, None);
    let mut path = vec![Toolbox::existing(world).dir];
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH"),
    ));
    let path = std::env::join_paths(path).unwrap();
    world.delegated_subtree.override_env("PATH", &path);
}

/// The global file rewritten by hand: a new entry carries the one
/// `"default": true` label, every other global entry loses it (a valid
/// global file labels exactly one). The overlay is untouched.
#[when(
    expr = "the global configuration is hand-edited to label container config {string} as its default"
)]
fn when_global_default_moved(world: &mut QuectoWorld, name: String) {
    let config_path = world.config_path.clone().expect("config path");
    let mut global: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let configs = global["container_configs"]
        .as_object_mut()
        .expect("global container_configs");
    let template = configs
        .values()
        .next()
        .cloned()
        .expect("a global entry to copy the scripts from");
    for entry in configs.values_mut() {
        entry.as_object_mut().unwrap().remove("default");
    }
    let mut entry = template;
    entry["default"] = serde_json::json!(true);
    configs.insert(name, entry);
    std::fs::write(&config_path, serde_json::to_string_pretty(&global).unwrap()).unwrap();
}

/// `quecto config unset --local container_configs.<name>.default`: the
/// label removed through the trusted write path, the way an agent or an
/// operator would remove it, so the overlay stays trusted.
#[when(expr = "the overlay entry {string} is un-defaulted through quecto config unset --local")]
fn when_overlay_entry_undefaulted(world: &mut QuectoWorld, name: String) {
    let output = cli::run_with_output(
        vec![
            "quecto".into(),
            "config".into(),
            "unset".into(),
            "--local".into(),
            format!("container_configs.{name}.default"),
        ],
        &world.cli_context,
    );
    assert_eq!(
        output.exit_code, 0,
        "quecto config unset --local failed:\nstdout: {}\nstderr: {}",
        output.stdout, output.stderr
    );
}

#[then(expr = "the overlay entry {string} should be the default")]
fn then_entry_is_default(world: &mut QuectoWorld, name: String) {
    let entry = overlay_entry(world, &name);
    assert!(entry.is_object(), "no entry {name}");
    assert_eq!(entry["default"], serde_json::json!(true), "{entry}");
}

#[then(expr = "the fake podman should have been asked to run the child {int} times")]
fn then_fake_podman_ran_times(world: &mut QuectoWorld, expected: usize) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    let runs = log
        .lines()
        .filter(|line| line.starts_with("run ") && !line.starts_with("run --rm "))
        .count();
    assert_eq!(runs, expected, "podman log: {log}");
}

#[then("the fake podman should have been asked to run the child once")]
fn then_fake_podman_ran_once(world: &mut QuectoWorld) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    let runs = log
        .lines()
        .filter(|line| line.starts_with("run ") && !line.starts_with("run --rm "))
        .count();
    assert_eq!(runs, 1, "podman log: {log}");
}

#[then("the fake podman should have been asked to remove the container")]
fn then_fake_podman_removed(world: &mut QuectoWorld) {
    let log = std::fs::read_to_string(Toolbox::existing(world).podman_log()).unwrap_or_default();
    assert!(
        log.lines().any(|line| line.starts_with("rm -f ")),
        "podman log: {log}"
    );
}

fn kill_invocation_marker(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("kill-invoked")
}

/// The same edit as the create script's: a marker, then `exit 99` before
/// any runtime call.
#[when("the materialised standard kill script is edited to record every invocation")]
fn when_kill_script_edited(world: &mut QuectoWorld) {
    let kill = assets_dir(world).join("scripts/kill.sh");
    let original = std::fs::read_to_string(&kill).unwrap();
    let (shebang, rest) = original.split_once('\n').unwrap();
    std::fs::write(
        &kill,
        format!(
            "{shebang}\ntouch '{}'\nexit 99\n{rest}",
            kill_invocation_marker(world).display()
        ),
    )
    .unwrap();
}

#[then(
    expr = "the container command result should name the materialised {string} as differing from the standard bundle"
)]
fn then_container_cmd_names_differing_script(world: &mut QuectoWorld, relative: String) {
    let result = world
        .container_cmd_result
        .as_ref()
        .expect("no container command result");
    let expected = format!(
        "{} differs from the standard bundle this quecto embeds",
        assets_dir(world).join(&relative).display()
    );
    assert!(
        result.is_error && result.content.contains(&expected),
        "expected {expected:?} in: {}",
        result.content
    );
}

#[then("the materialised standard kill script should never have been invoked")]
fn then_kill_never_invoked(world: &mut QuectoWorld) {
    let marker = kill_invocation_marker(world);
    assert!(
        !marker.exists(),
        "the edited kill script ran on the host ({} exists)",
        marker.display()
    );
}
