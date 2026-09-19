//! Host-local runtime stop must fail closed before writing its preservation receipt.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run_stop(state: &Path, id: &str) -> Output {
    Command::new(repo_root().join("scripts/container-runtime/kill.sh"))
        .args(["--state-dir"])
        .arg(state)
        .args(["--op", "stop"])
        .env("QUECTO_CONTAINER_ENVIRONMENT_ID", id)
        .output()
        .expect("run host kill adapter")
}

fn environment(state: &Path, id: &str) -> PathBuf {
    let path = state.join(id);
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn malformed_or_unreadable_child_inventory_refuses_the_stop_receipt() {
    for (id, inventory_is_directory) in [("env-malformed", false), ("env-unreadable", true)] {
        let temp = tempfile::tempdir().unwrap();
        let env = environment(temp.path(), id);
        let inventory = env.join("children.jsonl");
        if inventory_is_directory {
            fs::create_dir(&inventory).unwrap();
        } else {
            fs::write(&inventory, "not json\n").unwrap();
        }

        let output = run_stop(temp.path(), id);
        assert!(
            !output.status.success(),
            "invalid inventory must fail closed"
        );
        assert!(
            !env.join("runtime-stopped").exists(),
            "no stop receipt may be written"
        );
    }
}

#[test]
fn stop_verifies_recorded_process_retirement_before_writing_the_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let env = environment(temp.path(), "env-live");
    let mut child: Child = Command::new("sleep").arg("60").spawn().unwrap();
    fs::write(
        env.join("children.jsonl"),
        format!("{{\"pid\":{}}}\n", child.id()),
    )
    .unwrap();

    let output = run_stop(temp.path(), "env-live");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "the recorded process must have been killed"
    );
    assert_eq!(
        fs::read_to_string(env.join("runtime-stopped")).unwrap(),
        "stopped\n"
    );
}
