//! Runs the exact packaged Python API against real local SQLite, including
//! contention from independent clients rather than a mocked SQL adapter.
#[test]
fn packaged_swarm_behavior_contract() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/swarm_helpers_test.py"
        ))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("python3 required by swarm");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pure_coordination_policy_contract() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/swarm_policy_test.py"
        ))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("python3 required by swarm");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
