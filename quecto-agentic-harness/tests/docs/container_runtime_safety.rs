use std::fs;
use std::path::PathBuf;

fn workspace_file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above harness crate")
        .join(path)
}

fn script(path: &str) -> String {
    fs::read_to_string(workspace_file(path)).expect("runtime script exists")
}

#[test]
fn destructive_adapters_allowlist_operations_and_ids() {
    for path in [
        "scripts/container-runtime/kill.sh",
        "scripts/container-runtime/podman/kill.sh",
    ] {
        let source = script(path);
        assert!(
            source.contains("kill|cleanup") || source.contains("kill | cleanup"),
            "{path} must allowlist destructive operations"
        );
    }
    for path in ["scripts/container-runtime/podman/kill.sh"] {
        assert!(
            script(path).contains("*[!A-Za-z0-9_.-]*"),
            "{path} must allowlist environment IDs"
        );
    }
    let host = script("scripts/container-runtime/kill.sh");
    assert!(
        host.contains("case \"$pid\" in") && host.contains("kill -9 -- \"$pid\""),
        "host cleanup must validate PID values before signalling"
    );
}
