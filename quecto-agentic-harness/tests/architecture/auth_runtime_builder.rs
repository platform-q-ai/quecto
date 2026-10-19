//! The auth adapters run their blocking calls on the harness's one runtime
//! builder (#1839 review, #1718): `build_tokio_runtime`'s thread probe and
//! resident blocking thread turn thread exhaustion into an error, not an
//! abort. No auth adapter builds a Tokio runtime of its own.
use super::teardown_authority::{production_code, production_files};

/// The adapters that block on a runtime, each of which must use the
/// shared builder.
const BLOCKING_ADAPTERS: &[&str] = &[
    "src/infrastructure/auth/browser_login.rs",
    "src/infrastructure/auth/device_code_flow.rs",
];

#[test]
fn no_auth_adapter_builds_its_own_runtime() {
    let own_builders: Vec<String> = production_files()
        .into_iter()
        .filter(|file| file.starts_with("src/infrastructure/auth/"))
        .flat_map(|file| {
            production_code(&file)
                .into_iter()
                .filter(|(_, line)| line.contains("runtime::Builder"))
                .map(move |(number, line)| format!("{file}:{number}: {}", line.trim()))
        })
        .collect();
    assert!(
        own_builders.is_empty(),
        "build runtimes with infrastructure::processes::tokio_runtime::build_tokio_runtime: {own_builders:#?}"
    );
}

#[test]
fn every_blocking_auth_adapter_uses_the_shared_builder() {
    for adapter in BLOCKING_ADAPTERS {
        assert!(
            production_code(adapter)
                .iter()
                .any(|(_, line)| line.contains("build_tokio_runtime")),
            "{adapter} must run on build_tokio_runtime"
        );
    }
}
