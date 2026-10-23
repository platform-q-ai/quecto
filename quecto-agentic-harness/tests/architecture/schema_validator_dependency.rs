//! Check the schema validator's direct dependency declarations and Unicode behavior.
//! `cargo metadata --no-deps` describes declarations, not resolved package provenance:
//! a crates.io declaration can still resolve through a local patch. Review the actual
//! resolved package, source, and lock/archive checksums when dependencies or patches
//! change, outside nested Cargo tests/build locks. Cargo deny and lock checks remain
//! gates, but do not prohibit every local patch.
use serde_json::Value;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct ReapedChild(Child);
impl Drop for ReapedChild {
    fn drop(&mut self) {
        // Child::kill is SIGKILL on Unix and TerminateProcess on Windows;
        // wait reaps even on timeout, polling errors, or assertion unwinding.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn declared_dependencies() -> &'static [Value] {
    static METADATA: OnceLock<Value> = OnceLock::new();
    let metadata = METADATA.get_or_init(|| {
        let output = tempfile::tempdir().expect("metadata output directory");
        let stdout = output.path().join("stdout.json");
        let stderr = output.path().join("stderr.log");
        // File-backed streams cannot deadlock on full pipes; no resolution/build.
        let mut child = ReapedChild(
            Command::new(env!("CARGO"))
                .arg("metadata")
                .args(["--no-deps", "--offline", "--format-version", "1"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .stdin(Stdio::null())
                .stdout(std::fs::File::create(&stdout).expect("metadata stdout"))
                .stderr(std::fs::File::create(&stderr).expect("metadata stderr"))
                .spawn()
                .expect("spawn cargo metadata"),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.0.try_wait().expect("poll cargo metadata") {
                break status;
            }
            assert!(Instant::now() < deadline, "metadata exceeded 60s");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            status.success(),
            "cargo metadata {status}: {}",
            std::fs::read_to_string(&stderr).expect("read metadata stderr")
        );
        serde_json::from_slice(&std::fs::read(stdout).expect("read metadata stdout"))
            .expect("parse cargo metadata")
    });
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .find(|package| package["name"] == "quecto-agentic-harness")
        .expect("harness package")["dependencies"]
        .as_array()
        .expect("harness dependencies")
}

#[test]
fn schema_validator_dependency_declares_direct_nonoptional_registry_dev_dependency() {
    assert!(
        declared_dependencies().iter().any(|dep| {
            dep["name"] == "jsonschema"
                && dep["kind"] == "dev"
                && dep["optional"] == false
                && dep["uses_default_features"] == false
                && [
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                    Some("registry+sparse+https://index.crates.io/"),
                ]
                .contains(&dep["source"].as_str())
        }),
        "harness must directly declare crates.io jsonschema as a nonoptional dev dependency with default features disabled (declaration only, not resolved provenance)"
    );
}

#[test]
fn schema_validator_dependency_declarations_are_absent_from_runtime_dependency_kinds() {
    assert!(
        declared_dependencies()
            .iter()
            .filter(|dep| dep["name"] == "jsonschema")
            .all(|dep| dep["kind"] == "dev"),
        "jsonschema dependency entries may only have kind dev (not normal or build)"
    );
}

#[test]
fn schema_validator_dependency_counts_unicode_scalars_for_max_length() {
    let schema = serde_json::json!({ "type": "string", "maxLength": 512 });
    let validator = jsonschema::validator_for(&schema).expect("valid maxLength schema");
    let accepted = "🦀".repeat(512);
    let rejected = format!("{accepted}🦀");
    assert_eq!(accepted.chars().count(), 512);
    assert_eq!(rejected.chars().count(), 513);
    assert!(
        accepted.len() > 512,
        "fixture must distinguish bytes from scalars"
    );
    assert!(validator.validate(&serde_json::json!(accepted)).is_ok());
    assert!(validator.validate(&serde_json::json!(rejected)).is_err());
}
