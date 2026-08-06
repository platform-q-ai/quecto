//! Slice 5 (#1369): canonical container-runtime documentation and script-set
//! shape. One canonical user doc (`docs/container-runtimes.md`) and one
//! canonical reference-script path (`scripts/container-runtime/`) exist at the
//! workspace root, linked from the README, with the superseded slice-1
//! artifacts folded in rather than left as parallel copies. Behavioral claims
//! are proven by tests/features/script_managed_runtime_slice5.feature; these
//! tests pin the repository shape and the doc/script cross-links.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above the harness crate")
        .join(relative)
}

fn read_workspace_file(relative: &str) -> String {
    fs::read_to_string(workspace_file(relative))
        .unwrap_or_else(|e| panic!("failed to read {relative}: {e}"))
}

const CANONICAL_DOC: &str = "docs/container-runtimes.md";
const CANONICAL_SCRIPTS: [&str; 4] = [
    "scripts/container-runtime/create.sh",
    "scripts/container-runtime/exec.sh",
    "scripts/container-runtime/inspect.sh",
    "scripts/container-runtime/kill.sh",
];

/// The official Docker adapter lives INSIDE the canonical reference-script
/// path (one canonical path, per the #1369 delivery rule), next to the
/// CI-exercised host-local set.
const DOCKER_SCRIPTS: [&str; 4] = [
    "scripts/container-runtime/docker/create.sh",
    "scripts/container-runtime/docker/exec.sh",
    "scripts/container-runtime/docker/inspect.sh",
    "scripts/container-runtime/docker/kill.sh",
];

#[test]
fn canonical_container_runtimes_doc_exists_and_names_the_script_set() {
    let doc = read_workspace_file(CANONICAL_DOC);
    for script in CANONICAL_SCRIPTS.iter().chain(DOCKER_SCRIPTS.iter()) {
        assert!(
            doc.contains(script),
            "{CANONICAL_DOC} should reference the canonical script {script}"
        );
    }
}

#[test]
fn canonical_doc_covers_the_documented_contract_surface() {
    let doc = read_workspace_file(CANONICAL_DOC);
    // Distinctive needles only (backticked tokens, JSON field spellings,
    // literal env-var names) so incidental prose cannot satisfy them.
    for required in [
        // config/selection
        "`container_scripts`",
        "container_scripts.default",
        // operations and identity split
        "QUECTO_CONTAINER_ENVIRONMENT_REF",
        "QUECTO_CONTAINER_ENVIRONMENT_ID",
        "QUECTO_CONTAINER_REPO",
        // required JSON contract (field spellings as they appear in JSON)
        "\"environment_id\"",
        "\"workspace_path\"",
        "\"metadata\"",
        // endpoint modes. The canonical reference scripts are deliberately
        // direct-only (host-local, CI-safe); `socket_proxy` is documented as
        // an authoring option whose production behavior is exercised by the
        // proxy scenarios in script_managed_liveness_slice3.feature, and the
        // doc cross-references those tests explicitly.
        "\"socket_path\"",
        "\"socket_proxy\"",
        "script_managed_liveness_slice3.feature",
        // lifecycle semantics
        "`inspect`",
        "`kill_container`",
        "`get_containers`",
        "`cleanup`",
    ] {
        assert!(
            doc.contains(required),
            "{CANONICAL_DOC} should document `{required}`"
        );
    }
    assert!(
        doc.to_lowercase().contains("author"),
        "{CANONICAL_DOC} should explain how to author another runtime adapter"
    );
}

#[test]
fn readme_links_the_canonical_runtime_doc_and_scripts() {
    let readme = read_workspace_file("README.md");
    assert!(
        readme.contains("docs/container-runtimes.md"),
        "workspace README should link the canonical container-runtimes doc"
    );
    assert!(
        readme.contains("scripts/container-runtime/"),
        "workspace README should point at the canonical container-runtime script set"
    );
}

#[test]
fn docker_adapter_keeps_its_load_bearing_properties() {
    // Contract needles for the Docker adapter: strict jq-encoded JSON,
    // identity mounts, rollback, id containment, op logging, and the
    // QUECTO_BASE_DIR warning (overriding it broke OAuth providers).
    let create = read_workspace_file(DOCKER_SCRIPTS[0]);
    for needle in [
        "QUECTO_DOCKER_IMAGE",
        "--image",
        "docker rm -f",
        "trap",
        "jq -cn",
        "HOME=$HOME",
        "QUECTO_CONTAINER_ENVIRONMENT_REF",
        "QUECTO_BASE_DIR",
        "OAuth",
        // Identity bind-mounts (same path inside and outside): workspace rw,
        // socket dir rw, child binary ro, $HOME/.quecto rw with HOME kept.
        "$workspace_path:$workspace_path:rw",
        "$socket_dir:$socket_dir:rw",
        "$child_binary:$child_binary:ro",
        "$HOME/.quecto:$HOME/.quecto:rw",
    ] {
        assert!(
            create.contains(needle),
            "{} should contain {needle}",
            DOCKER_SCRIPTS[0]
        );
    }
    assert!(
        !create.contains("QUECTO_BASE_DIR="),
        "docker create must never override QUECTO_BASE_DIR (credentials/config home)"
    );
    let kill = read_workspace_file(DOCKER_SCRIPTS[3]);
    for needle in [
        "--op",
        "kill.log",
        "docker rm -f",
        "QUECTO_CONTAINER_ENVIRONMENT_ID",
    ] {
        assert!(
            kill.contains(needle),
            "{} should contain {needle}",
            DOCKER_SCRIPTS[3]
        );
    }
    for script in [DOCKER_SCRIPTS[1], DOCKER_SCRIPTS[2], DOCKER_SCRIPTS[3]] {
        let content = read_workspace_file(script);
        assert!(
            content.contains("*/* | *..*)"),
            "{script} should contain the environment-id containment check"
        );
    }
}

#[test]
fn canonical_scripts_exist_and_are_executable() {
    for script in CANONICAL_SCRIPTS
        .iter()
        .chain(DOCKER_SCRIPTS.iter())
        .copied()
    {
        let path = workspace_file(script);
        assert!(path.is_file(), "{script} should exist");
        let content = fs::read_to_string(&path).unwrap();
        assert!(
            content.starts_with("#!"),
            "{script} should start with a shebang"
        );
        assert!(
            content.contains("set -euo pipefail"),
            "{script} should fail fast (set -euo pipefail)"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "{script} should be executable (mode {mode:o})"
            );
        }
    }
}

#[test]
fn superseded_slice1_doc_and_script_copies_are_folded_into_the_canonical_path() {
    // Delivery rule (#1369): ONE canonical user documentation path and ONE
    // canonical reference-script path. The slice-1 artifacts must be folded
    // into the slice-5 canonical ones, not left as overlapping second copies.
    assert!(
        !workspace_file("docs/container-scripts.md").exists(),
        "docs/container-scripts.md should be folded into docs/container-runtimes.md"
    );
    assert!(
        !workspace_file("scripts/container-script-reference.sh").exists(),
        "scripts/container-script-reference.sh should be folded into scripts/container-runtime/"
    );
    let readme = read_workspace_file("README.md");
    assert!(
        !readme.contains("container-scripts.md")
            && !readme.contains("container-script-reference.sh"),
        "workspace README should reference only the canonical runtime doc/scripts"
    );
    // No second container doc alongside the canonical one.
    let docs_dir = workspace_file("docs");
    let container_docs: Vec<String> = fs::read_dir(&docs_dir)
        .expect("read workspace docs dir")
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.contains("container"))
        .collect();
    assert_eq!(
        container_docs,
        vec!["container-runtimes.md".to_string()],
        "exactly one canonical container doc should exist in docs/: {container_docs:?}"
    );
}

#[test]
fn canonical_scripts_keep_runtime_knowledge_out_of_rust_core() {
    // Runtime-agnostic core: Docker/Podman/devcontainer knowledge may live in
    // the adapted scripts only, never in Quecto Rust sources — across EVERY
    // workspace member crate, not just the harness.
    let workspace_root = workspace_file("");
    let mut scanned_crates = 0usize;
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&workspace_root)
        .expect("read workspace root")
        .flatten()
    {
        let crate_dir = entry.path();
        let src = crate_dir.join("src");
        if crate_dir.join("Cargo.toml").is_file() && src.is_dir() {
            scanned_crates += 1;
            scan_rust_sources(&src, &mut offenders);
        }
    }
    assert!(
        scanned_crates >= 5,
        "workspace member discovery looks broken: only {scanned_crates} crates scanned"
    );
    assert!(
        offenders.is_empty(),
        "Rust core must not special-case container runtimes: {offenders:?}"
    );
}

fn scan_rust_sources(dir: &Path, offenders: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read src dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_rust_sources(&path, offenders);
        } else if path.extension().is_some_and(|e| e == "rs")
            // In-tree test modules may use runtime names as inert fixture
            // data (e.g. exec-allowlist matching examples); the boundary
            // guards production code.
            && !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"))
        {
            let content = fs::read_to_string(&path).unwrap_or_default();
            for needle in ["docker", "podman", "devcontainer"] {
                if content.to_lowercase().contains(needle) {
                    offenders.push(format!("{}: {needle}", path.display()));
                }
            }
        }
    }
}
