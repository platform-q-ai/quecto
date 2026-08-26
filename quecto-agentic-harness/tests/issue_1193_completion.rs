//! End-to-end acceptance guards for epic #1193 slice 6 (issue #1576): legacy
//! provider/model authorities are removed or demonstrably subordinated to the
//! application-published snapshot, capability heuristics live in canonical
//! metadata, and the contributor documentation maps layer ownership.
//!
//! These are the grep-able forms of the epic's "no duplicate authority"
//! acceptance criteria: no catalogue parsing/merging or composition bridges in
//! CLI-specific interface modules, and no capability inference outside the
//! domain's canonical metadata.

use std::{fs, path::Path, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The production portion (before a trailing `#[cfg(test)]` module) of every
/// non-`_tests.rs` file under `dir`, as `(path, content)` pairs.
fn production_sources(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("readable dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            out.extend(production_sources(&path));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("readable file");
        out.push((
            path.display().to_string(),
            raw.split("#[cfg(test)]").next().unwrap().to_string(),
        ));
    }
    out
}

#[test]
fn cli_interface_owns_no_catalogue_composition_bridges() {
    // Slices 2/4 parked interface-level composition in bridge modules; slice 6
    // removes them. Composition lives in the shared interface composition root
    // (`interface/catalogue_runtime.rs`) invoked by process entry points.
    for legacy in [
        "src/interface/cli/catalogue_bridge.rs",
        "src/interface/cli/catalogue_refresh_bridge.rs",
    ] {
        assert!(
            !root().join(legacy).exists(),
            "legacy interface catalogue authority still present: {legacy}"
        );
    }
}

#[test]
fn interface_layer_does_not_read_the_legacy_model_registry() {
    // "No catalogue parsing/merging in interface": only infrastructure source
    // adapters may see the models.json wire format and the legacy registry
    // parser that reads it.
    for (path, content) in production_sources(&root().join("src/interface")) {
        assert!(
            !content.contains("infrastructure::model_registry"),
            "interface module still reads the legacy model registry: {path}"
        );
    }
}

#[test]
fn set_model_derives_limits_from_the_published_snapshot_only() {
    // The UDS set_model path must resolve limits and the selection verdict
    // from one published snapshot read, not via a bridge that re-resolves the
    // catalogue on its own.
    let dispatch = read("src/interface/cli/uds_dispatch_runtime.rs");
    assert!(
        !dispatch.contains("model_limits_from_base_dir"),
        "uds set_model still derives limits through the legacy bridge re-resolve"
    );
}

#[test]
fn canonical_capabilities_carry_the_effort_vocabulary() {
    let domain = read("src/domain/catalogue.rs");
    assert!(
        domain.contains("effort_levels"),
        "domain ModelCapabilities does not represent the effort vocabulary"
    );
}

#[test]
fn effort_capability_is_not_inferred_from_model_names_outside_the_domain() {
    // The provider/model-name reasoning-effort heuristic may survive only as
    // the domain rule seeding canonical built-in metadata; interface and
    // infrastructure consumers read the snapshot instead.
    for layer in ["src/interface", "src/infrastructure"] {
        for (path, content) in production_sources(&root().join(layer)) {
            assert!(
                !content.contains("levels_for_model"),
                "effort capability inferred from model names outside canonical metadata: {path}"
            );
        }
    }
}

#[test]
fn contributor_docs_map_layer_ownership_and_forbid_new_authorities() {
    let docs = read("../docs/runtime-models-providers.md").to_ascii_lowercase();
    for phrase in [
        "layer ownership",
        "domain metadata",
        "another authority",
        "existing transport",
        "new transport",
        "source precedence",
    ] {
        assert!(docs.contains(phrase), "missing documentation: {phrase}");
    }
}
