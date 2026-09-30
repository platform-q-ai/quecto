//! Execute the documented opt-in recipe against a local configuration fixture.
#![cfg(unix)]
use std::{fs, process::Command};

#[test]
fn documented_cache_wiring_preserves_init_arguments() {
    let docs = include_str!("../../../docs/container-runtimes.md");
    let expression = docs
        .split("create=$(jq -ce --arg adapter \"$adapter\" '")
        .nth(1)
        .expect("documented jq recipe")
        .split("' .quecto/config.json)")
        .next()
        .unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let config = serde_json::json!({"container_configs":{"standard":{
        "default":true,
        "create":["/repo/.quecto/containers/standard/scripts/create.sh","--state-dir","/private/state","--repo","https://example.invalid/repo","--image","pinned:v4"],
        "exec":["/repo/.quecto/containers/standard/scripts/exec.sh","--state-dir","/private/state"],
        "inspect":["/repo/.quecto/containers/standard/scripts/inspect.sh"],
        "kill":["/repo/.quecto/containers/standard/scripts/kill.sh"],
        "cleanup":["/repo/.quecto/containers/standard/scripts/kill.sh","--op","cleanup"]
    }}});
    let path = fixture.path().join("config.json");
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let adapter = "/repo/.quecto/containers/standard/create.sh";
    let output = Command::new("timeout")
        .args([
            "--signal=KILL",
            "5",
            "jq",
            "-ce",
            "--arg",
            "adapter",
            adapter,
            expression,
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "recipe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let create: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let original = config["container_configs"]["standard"]["create"]
        .as_array()
        .unwrap();
    let changed = create.as_array().unwrap();
    assert_eq!(changed[0], adapter);
    assert_eq!(&changed[1..], &original[1..]);
    // The recipe only reads the trusted local document; config set updates the
    // create key, leaving every other init field and bundled script unchanged.
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
        config
    );
    let mut selected = config.clone();
    selected["container_configs"]["standard"]["create"] = create;
    for key in ["default", "exec", "inspect", "kill", "cleanup"] {
        assert_eq!(
            selected["container_configs"]["standard"][key],
            config["container_configs"]["standard"][key]
        );
    }
}
