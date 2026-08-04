use cucumber::{given, then, when};
use std::path::Path;

use crate::QuectoWorld;

const OPS: [&str; 4] = ["create", "exec", "inspect", "kill"];

#[given("a hybrid container panel check")]
fn container_backed_agent_panel_contract(world: &mut QuectoWorld) {
    world.stdout.clear();
}

#[when("the panel contains one solo environment and one shared environment")]
fn panel_contains_solo_and_shared_environments(world: &mut QuectoWorld) {
    world.stdout = "agent alpha\nagent beta\nagent gamma".into();
}

#[then("the solo agent row exposes its environment ref inline")]
fn solo_agent_row_exposes_environment_ref_inline(world: &mut QuectoWorld) {
    assert!(
        world.stdout.contains("alpha") && world.stdout.contains("C1"),
        "solo rows must include environment ref inline; panel was:\n{}",
        world.stdout
    );
}

#[then(
    "the shared environment is exposed as a selectable group row with its member agents beneath it"
)]
fn shared_environment_exposed_as_selectable_group(world: &mut QuectoWorld) {
    assert!(
        world.stdout.contains("C2")
            && world.stdout.contains("selectable")
            && world.stdout.contains("beta")
            && world.stdout.contains("gamma"),
        "shared environments must render as selectable groups; panel was:\n{}",
        world.stdout
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
    let docs = std::fs::read_to_string("docs/container-runtimes.md").unwrap_or_default();
    for op in OPS {
        assert!(
            docs.contains(op),
            "docs/container-runtimes.md must document {op} operation"
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
    let docs = std::fs::read_to_string("docs/container-runtimes.md").unwrap_or_default();
    for required in [
        "container_ref",
        "environment_id",
        "workspace_path",
        "status",
    ] {
        assert!(
            docs.contains(required),
            "docs/container-runtimes.md must document required JSON field {required}"
        );
    }
}
