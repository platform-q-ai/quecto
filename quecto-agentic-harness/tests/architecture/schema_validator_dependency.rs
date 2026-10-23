//! The schema validator is genuine test infrastructure, never a runtime dependency.
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

fn dependencies() -> &'static [Value] {
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
fn schema_validator_dependency_is_direct_nonoptional_registry_dev_dependency() {
    assert!(
        dependencies().iter().any(|dep| {
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
        "harness must directly declare genuine registry jsonschema as a nonoptional dev dependency"
    );
}

#[test]
fn schema_validator_dependency_is_absent_from_runtime_dependency_kinds() {
    assert!(
        dependencies()
            .iter()
            .filter(|dep| dep["name"] == "jsonschema")
            .all(|dep| dep["kind"] == "dev"),
        "jsonschema dependency entries may only have kind dev (not normal or build)"
    );
}
