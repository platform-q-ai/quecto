//! Steps for `config_discovery.feature` (#1966, #2024): the global
//! configuration, the trust-gated repo-local overlay `<cwd>/.quecto/config.json`
//! merged over it, `quecto config get|set|trust`, and the retired
//! `<cwd>/config.json` selection.

use super::*;

const OVERLAY_RELATIVE: &str = ".quecto/config.json";

fn cwd(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

fn global_config(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("config.json")
}

fn overlay_path(world: &QuectoWorld) -> PathBuf {
    cwd(world).join(OVERLAY_RELATIVE)
}

/// Remember the bytes of both configuration files before a run so a
/// scenario can assert what the run did (or did not) change.
pub(crate) fn snapshot_config_files(world: &mut QuectoWorld) {
    world.config_snapshots.clear();
    if world.cli_context.base_dir.is_none() || world.cli_context.cwd.is_none() {
        return;
    }
    for path in [global_config(world), overlay_path(world)] {
        if let Ok(bytes) = std::fs::read(&path) {
            world.config_snapshots.insert(path, bytes);
        }
    }
}

fn previous_content<'a>(world: &'a QuectoWorld, path: &Path) -> &'a [u8] {
    world
        .config_snapshots
        .get(path)
        .unwrap_or_else(|| panic!("no snapshot of {} before the run", path.display()))
}

fn run_cli(world: &mut QuectoWorld, args: Vec<String>) {
    snapshot_config_files(world);
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[given(expr = "a config file named {string} in the current directory with content:")]
fn given_local_config(world: &mut QuectoWorld, step: &gherkin::Step, name: String) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    std::fs::write(cwd(world).join(name), content).expect("write local config");
}

#[given("a repo-local overlay in the current directory with content:")]
fn given_overlay(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    let path = overlay_path(world);
    std::fs::create_dir_all(path.parent().expect("overlay has a parent")).expect("create .quecto");
    std::fs::write(path, content).expect("write overlay");
}

/// The hermetic cwd's own parent is the base directory (the global config's
/// home), so this step descends into a fresh child directory and writes the
/// overlay into the directory being left behind.
#[given("a repo-local overlay in the parent of the current directory with content:")]
fn given_parent_overlay(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    let parent = cwd(world);
    let child = parent.join("nested");
    std::fs::create_dir_all(&child).expect("create nested cwd");
    let overlay = parent.join(OVERLAY_RELATIVE);
    std::fs::create_dir_all(overlay.parent().unwrap()).expect("create parent .quecto");
    std::fs::write(overlay, content).expect("write parent overlay");
    world.cli_context.cwd = Some(child);
}

/// Approval through the real entry point, so a scenario's trust is exactly
/// what `quecto config trust` grants.
#[given("the repo-local overlay is trusted")]
fn given_overlay_trusted(world: &mut QuectoWorld) {
    run_cli(
        world,
        vec![
            "quecto".to_string(),
            "config".to_string(),
            "trust".to_string(),
        ],
    );
    assert_eq!(
        world.exit_code, 0,
        "quecto config trust failed:\nstdout: {}\nstderr: {}",
        world.stdout, world.stderr
    );
}

/// The trust record is written directly with the overlay's content hash,
/// as an interactive `[y/N]` approval would leave it — without the checks
/// `quecto config trust` applies — so a scenario can show what a *trusted*
/// overlay that is broken, or carries a global-only section, does at load.
#[given("the repo-local overlay is trusted regardless of its content")]
fn given_overlay_trusted_unchecked(world: &mut QuectoWorld) {
    use sha2::Digest;
    let path = overlay_path(world);
    let canonical = path.canonicalize().expect("overlay exists");
    let content = std::fs::read(&path).expect("read overlay");
    let hash = format!("{:x}", sha2::Sha256::digest(&content));
    let store = serde_json::json!({
        "approved": { canonical.to_string_lossy(): [hash] }
    });
    std::fs::write(
        base_path(world).join("config-overlay-trust.json"),
        serde_json::to_vec_pretty(&store).unwrap(),
    )
    .expect("write trust store");
}

#[when(expr = "I run quecto status with --config pointing at the current directory's {string}")]
fn when_run_status_with_explicit_config(world: &mut QuectoWorld, name: String) {
    let explicit = cwd(world).join(name);
    run_cli(
        world,
        vec![
            "quecto".to_string(),
            "--config".to_string(),
            explicit.to_string_lossy().into_owned(),
            "status".to_string(),
        ],
    );
}

/// Like "I run quecto with arguments", taking the argument line from a
/// docstring so JSON values can be quoted without Gherkin escaping.
#[when("I run quecto with the arguments:")]
fn when_run_with_docstring_args(world: &mut QuectoWorld, step: &gherkin::Step) {
    let line = step.docstring().expect("step should have a docstring");
    let mut args = vec!["quecto".to_string()];
    args.extend(shell_split(line.trim()));
    run_cli(world, args);
}

fn status_line(world: &QuectoWorld, label: &str) -> String {
    let line = world
        .stdout
        .lines()
        .find(|line| line.trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("status output lacks a {label} line:\n{}", world.stdout));
    line.trim_start()
        .trim_start_matches(label)
        .trim()
        .to_string()
}

#[then(expr = "the stderr should name the current directory's {string}")]
fn then_stderr_names_local(world: &mut QuectoWorld, name: String) {
    let expected = cwd(world).join(name).to_string_lossy().into_owned();
    assert!(
        world.stderr.contains(&expected),
        "expected stderr to name {expected}, got: {}",
        world.stderr
    );
}

#[then(expr = "the stdout should name the current directory's {string}")]
fn then_stdout_names_local(world: &mut QuectoWorld, name: String) {
    let expected = cwd(world).join(name).to_string_lossy().into_owned();
    assert!(
        world.stdout.contains(&expected),
        "expected stdout to name {expected}, got: {}",
        world.stdout
    );
}

#[then(expr = "the reported config path should be the global {string}")]
fn then_reported_global(world: &mut QuectoWorld, name: String) {
    assert_eq!(
        status_line(world, "Config:"),
        base_path(world).join(name).to_string_lossy(),
        "status reported the wrong config path"
    );
}

#[then(
    expr = "the reported overlay path should be the current directory's {string} marked {string}"
)]
fn then_reported_overlay(world: &mut QuectoWorld, name: String, mark: String) {
    let expected = format!("{} ({mark})", cwd(world).join(name).to_string_lossy());
    assert_eq!(
        status_line(world, "Overlay:"),
        expected,
        "status reported the wrong overlay line"
    );
}

#[then("the global config file should be byte-identical to its previous content")]
fn then_global_unchanged(world: &mut QuectoWorld) {
    let path = global_config(world);
    let now = std::fs::read(&path).expect("read global config");
    assert_eq!(now, previous_content(world, &path), "global config changed");
}

#[then(expr = "the current directory's {string} should be byte-identical to its previous content")]
fn then_local_unchanged(world: &mut QuectoWorld, name: String) {
    let path = cwd(world).join(&name);
    let now = std::fs::read(&path).expect("read local file");
    assert_eq!(now, previous_content(world, &path), "{name} changed");
}

#[then(
    expr = "the global config file should differ from its previous content only on the line containing {string}"
)]
fn then_global_differs_on_one_line(world: &mut QuectoWorld, needle: String) {
    let path = global_config(world);
    let before = String::from_utf8(previous_content(world, &path).to_vec()).unwrap();
    let after = std::fs::read_to_string(&path).expect("read global config");
    // A docstring carries surrounding newlines; the writer normalises the
    // file to start at `{` and end with exactly one newline, so the
    // comparison is line-wise over the document itself.
    let before: Vec<&str> = before.trim().lines().collect();
    assert!(
        after.ends_with("}\n"),
        "the written file ends with a newline"
    );
    assert!(
        after.starts_with('{'),
        "the written file starts at the document"
    );
    let after: Vec<&str> = after.trim().lines().collect();
    assert_eq!(
        before.len(),
        after.len(),
        "line count changed:\nbefore: {before:?}\nafter: {after:?}"
    );
    let changed: Vec<(usize, &str, &str)> = before
        .iter()
        .zip(after.iter())
        .enumerate()
        .filter(|(_, (b, a))| b != a)
        .map(|(i, (b, a))| (i + 1, *b, *a))
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "expected exactly one changed line: {changed:?}"
    );
    assert!(
        changed[0].1.contains(&needle) && changed[0].2.contains(&needle),
        "the changed line does not contain {needle}: {changed:?}"
    );
}

fn json_at<'a>(document: &'a serde_json::Value, key_path: &str) -> &'a serde_json::Value {
    key_path
        .split('.')
        .try_fold(document, |current, segment| {
            segment
                .parse::<usize>()
                .ok()
                .and_then(|index| current.get(index))
                .or_else(|| current.get(segment))
        })
        .unwrap_or_else(|| panic!("no `{key_path}` in {document}"))
}

fn file_json(path: &Path) -> serde_json::Value {
    let content =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

#[then(expr = "the global config file should set {string} to {string}")]
fn then_global_sets(world: &mut QuectoWorld, key_path: String, expected: String) {
    let document = file_json(&global_config(world));
    assert_eq!(
        json_at(&document, &key_path),
        &serde_json::Value::String(expected)
    );
}

#[then(expr = "the current directory's {string} should set {string} to {string}")]
fn then_local_sets(world: &mut QuectoWorld, name: String, key_path: String, expected: String) {
    let document = file_json(&cwd(world).join(&name));
    assert_eq!(
        json_at(&document, &key_path),
        &serde_json::Value::String(expected)
    );
}

fn printed_json(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&world.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{}", world.stdout))
}

#[then(expr = "the printed JSON should have {string} equal to {string}")]
fn then_printed_json_has(world: &mut QuectoWorld, key_path: String, expected: String) {
    let document = printed_json(world);
    assert_eq!(
        json_at(&document, &key_path),
        &serde_json::Value::String(expected)
    );
}

#[then(expr = "the printed JSON should have {string} set to {word}")]
fn then_printed_json_has_literal(world: &mut QuectoWorld, key_path: String, expected: String) {
    let document = printed_json(world);
    let expected: serde_json::Value = serde_json::from_str(&expected).expect("a JSON literal");
    assert_eq!(json_at(&document, &key_path), &expected);
}

/// The `$HOME` case: the global file is `<home>/.quecto/config.json` and
/// the run starts in `<home>`, where `<cwd>/.quecto/config.json` *is* the
/// global file. Built under the scenario's temp dir, never under `/tmp`.
#[given("the current directory is the parent of the base directory")]
fn given_cwd_is_base_parent(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let home = base_path(world).join("home");
    let quecto_dir = home.join(".quecto");
    std::fs::create_dir_all(&quecto_dir).expect("create .quecto");
    std::fs::copy(global_config(world), quecto_dir.join("config.json")).expect("copy global");
    world.cli_context.base_dir = Some(quecto_dir);
    world.cli_context.cwd = Some(home);
}

#[then(expr = "the printed JSON should be {string}")]
fn then_printed_json_is(world: &mut QuectoWorld, expected: String) {
    assert_eq!(printed_json(world), serde_json::Value::String(expected));
}

#[then(expr = "the current directory's {string} should not exist")]
fn then_local_absent(world: &mut QuectoWorld, name: String) {
    let path = cwd(world).join(&name);
    assert!(!path.exists(), "{} exists", path.display());
}

#[then("no temporary config files should remain beside the global config file")]
fn then_no_temp_files(world: &mut QuectoWorld) {
    let leftovers: Vec<String> = std::fs::read_dir(base_path(world))
        .expect("read base dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
}
