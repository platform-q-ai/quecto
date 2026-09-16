//! Steps for `config_discovery.feature` (#1966): a `config.json` in the
//! process working directory is selected ahead of the global configuration.

use super::*;

fn cwd(world: &QuectoWorld) -> PathBuf {
    world
        .cli_context
        .cwd
        .clone()
        .expect("BDD world should pin a hermetic cwd")
}

#[given(expr = "a config file named {string} in the current directory with content:")]
fn given_local_config(world: &mut QuectoWorld, step: &gherkin::Step, name: String) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    std::fs::write(cwd(world).join(name), content).expect("write local config");
}

/// The hermetic cwd's own parent is the base directory (the global config's
/// home), so this step descends into a fresh child directory and writes the
/// file into the directory being left behind.
#[given(expr = "a config file named {string} in the parent of the current directory with content:")]
fn given_parent_config(world: &mut QuectoWorld, step: &gherkin::Step, name: String) {
    let content = step.docstring().expect("step should have a docstring");
    ensure_temp_dir(world);
    let parent = cwd(world);
    let child = parent.join("nested");
    std::fs::create_dir_all(&child).expect("create nested cwd");
    std::fs::write(parent.join(name), content).expect("write parent config");
    world.cli_context.cwd = Some(child);
}

#[given(expr = "a directory named {string} in the current directory")]
fn given_local_directory(world: &mut QuectoWorld, name: String) {
    ensure_temp_dir(world);
    std::fs::create_dir_all(cwd(world).join(name)).expect("create local directory");
}

#[given(expr = "a dangling symlink named {string} in the current directory")]
fn given_dangling_symlink(world: &mut QuectoWorld, name: String) {
    ensure_temp_dir(world);
    let dir = cwd(world);
    std::os::unix::fs::symlink(dir.join("missing-target.json"), dir.join(name))
        .expect("create dangling symlink");
}

#[when(expr = "I run quecto status with --config pointing at the current directory's {string}")]
fn when_run_status_with_explicit_config(world: &mut QuectoWorld, name: String) {
    let explicit = cwd(world).join(name);
    let args = vec![
        "quecto".to_string(),
        "--config".to_string(),
        explicit.to_string_lossy().into_owned(),
        "status".to_string(),
    ];
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

fn reported_config_path(world: &QuectoWorld) -> String {
    let line = world
        .stdout
        .lines()
        .find(|line| line.trim_start().starts_with("Config:"))
        .unwrap_or_else(|| panic!("status output lacks a Config line:\n{}", world.stdout));
    line.trim_start()
        .trim_start_matches("Config:")
        .trim()
        .to_string()
}

#[then(expr = "the reported config path should be the current directory's {string}")]
fn then_reported_local(world: &mut QuectoWorld, name: String) {
    assert_eq!(
        reported_config_path(world),
        cwd(world).join(name).to_string_lossy(),
        "status reported the wrong config path"
    );
}

#[then(expr = "the reported config path should be the global {string}")]
fn then_reported_global(world: &mut QuectoWorld, name: String) {
    assert_eq!(
        reported_config_path(world),
        base_path(world).join(name).to_string_lossy(),
        "status reported the wrong config path"
    );
}
