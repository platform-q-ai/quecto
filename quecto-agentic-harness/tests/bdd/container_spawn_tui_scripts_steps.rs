use cucumber::{given, then, when};
use std::path::Path;
use std::process::Command;

use crate::QuectoWorld;

const OPS: [&str; 4] = ["create", "exec", "inspect", "kill"];

fn read_runtime_docs() -> String {
    for path in [
        "docs/container-runtimes.md",
        "../docs/container-runtimes.md",
    ] {
        if let Ok(docs) = std::fs::read_to_string(path) {
            return docs;
        }
    }
    String::new()
}

#[given("a hybrid container panel check")]
fn container_backed_agent_panel_contract(world: &mut QuectoWorld) {
    world.stdout.clear();
    world.stderr.clear();
    world.exit_code = 0;
}

#[when("the panel contains one solo environment and one shared environment")]
fn panel_contains_solo_and_shared_environments(world: &mut QuectoWorld) {
    let output = Command::new("cargo")
        .args([
            "test",
            "-p",
            "quecto-tui",
            "--features",
            "test-harness",
            "--lib",
            "container_panel_probe_drives_real_roster_render_navigation_and_details",
            "--",
            "--exact",
            "--nocapture",
        ])
        .output()
        .expect("run quecto-tui container panel probe");
    world.exit_code = output.status.code().unwrap_or(1);
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when("the shared environment row is selected")]
fn shared_environment_row_is_selected(world: &mut QuectoWorld) {
    if world.stdout.is_empty() && world.stderr.is_empty() {
        panel_contains_solo_and_shared_environments(world);
    }
}

#[then("the solo agent row exposes its environment ref inline")]
fn solo_agent_row_exposes_environment_ref_inline(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[then(
    "the shared environment is exposed as a selectable group row with its member agents beneath it"
)]
fn shared_environment_exposed_as_selectable_group(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[then("the main pane renders the selected environment repository")]
fn main_pane_renders_selected_environment_repository(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[then("the main pane renders the selected environment runtime")]
fn main_pane_renders_selected_environment_runtime(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[then("the main pane renders the selected environment workspace")]
fn main_pane_renders_selected_environment_workspace(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[then("the main pane renders the selected environment health")]
fn main_pane_renders_selected_environment_health(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "TUI probe failed:\nstdout={}\nstderr={}",
        world.stdout, world.stderr
    );
}

#[given("a container runtime contract check")]
fn in_repository_container_runtime_contract(world: &mut QuectoWorld) {
    world.stdout.clear();
}

#[when("I check supported runtime operations")]
fn repository_checked_for_supported_runtime_operations(world: &mut QuectoWorld) {
    let mut report = String::new();
    for op in OPS {
        let path = format!("scripts/container-runtime/{op}.sh");
        report.push_str(&path);
        if Path::new(&path).exists() {
            report.push_str(" present");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&path)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    report.push_str(" executable");
                }
            }
        } else {
            report.push_str(" missing");
        }
        report.push('\n');
    }
    report.push_str("docs/container-runtimes.md");
    if Path::new("docs/container-runtimes.md").exists() {
        report.push_str(" present");
    } else {
        report.push_str(" missing");
    }
    world.stdout = report;
}

#[then(expr = "the {word} container runtime script is present and executable")]
fn runtime_script_present_and_executable(world: &mut QuectoWorld, op: String) {
    let expected = format!("scripts/container-runtime/{op}.sh present executable");
    assert!(
        world.stdout.contains(&expected),
        "expected {expected}; contract check was:\n{}",
        world.stdout
    );
}

#[then("docs/container-runtimes.md documents the create exec inspect kill contract")]
fn docs_container_runtimes_documents_operations(world: &mut QuectoWorld) {
    assert!(
        world.stdout.contains("docs/container-runtimes.md present"),
        "docs/container-runtimes.md must exist; contract check was:\n{}",
        world.stdout
    );
    let docs = read_runtime_docs();
    for phrase in [
        "new",
        "create",
        "existing",
        "exec",
        "endpoint",
        "cleanup_failed",
        "inspect_failed",
        "runtime-agnostic",
    ] {
        assert!(
            docs.contains(phrase),
            "docs/container-runtimes.md must document {phrase}"
        );
    }
}

#[then("each container runtime script documents a JSON result contract")]
fn each_script_documents_json_result_contract(_world: &mut QuectoWorld) {
    for op in OPS {
        let path = format!("scripts/container-runtime/{op}.sh");
        let script = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            script.contains("JSON") && script.contains("result"),
            "{path} must document/emits a machine-readable JSON result contract"
        );
    }
}

#[then("docs/container-runtimes.md documents required JSON fields for each operation")]
fn docs_document_required_json_fields(_world: &mut QuectoWorld) {
    let docs = read_runtime_docs();
    for required in [
        "container_ref",
        "environment_id",
        "workspace_path",
        "status",
        "metadata",
    ] {
        assert!(
            docs.contains(required),
            "docs/container-runtimes.md must document required JSON field {required}"
        );
    }
}
