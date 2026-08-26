//! Steps for catalogue_convergence.feature (issue #1576, epic #1193 slice 6).
//!
//! Slice 6 removes or subordinates every legacy provider/model authority left
//! by slices 1-5. These steps check the convergence directly: source-tree
//! scans for the grep-able "no duplicate authority" criteria, and the real
//! UDS listing surface for canonical-capability projection.

use super::*;
use quecto::interface::cli::uds_models::list_models_data;

#[derive(Debug, Default)]
pub struct CatalogueConvergenceState {
    base_dir: Option<tempfile::TempDir>,
    uds_listing: Option<serde_json::Value>,
}

fn harness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collect the production portion (everything before a trailing `#[cfg(test)]`
/// module marker) of every non-test `.rs` file under `dir`, as
/// `(path, content)` pairs. `_tests.rs` companions are skipped entirely.
fn production_sources(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir).expect("readable source dir");
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(production_sources(&path));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("readable source file");
        let production = raw.split("#[cfg(test)]").next().unwrap().to_string();
        out.push((path.display().to_string(), production));
    }
    out
}

#[given("the harness source tree")]
fn given_harness_source_tree(_world: &mut QuectoWorld) {}

#[then("the CLI interface declares no catalogue bridge modules")]
fn then_no_bridge_modules(_world: &mut QuectoWorld) {
    for legacy in [
        "src/interface/cli/catalogue_bridge.rs",
        "src/interface/cli/catalogue_refresh_bridge.rs",
    ] {
        assert!(
            !harness_root().join(legacy).exists(),
            "legacy interface catalogue authority still present: {legacy}"
        );
    }
}

#[then("no interface module reads the legacy model registry")]
fn then_interface_reads_no_registry(_world: &mut QuectoWorld) {
    for (path, content) in production_sources(&harness_root().join("src/interface")) {
        assert!(
            !content.contains("infrastructure::model_registry"),
            "interface module still reads the legacy model registry: {path}"
        );
    }
}

#[then("canonical model capabilities declare an effort vocabulary")]
fn then_capabilities_declare_effort(_world: &mut QuectoWorld) {
    let domain = fs::read_to_string(harness_root().join("src/domain/catalogue.rs")).unwrap();
    assert!(
        domain.contains("effort_levels"),
        "domain ModelCapabilities does not carry an effort vocabulary"
    );
}

#[then("no interface or infrastructure module infers effort levels from model names")]
fn then_no_effort_name_inference(_world: &mut QuectoWorld) {
    for layer in ["src/interface", "src/infrastructure"] {
        for (path, content) in production_sources(&harness_root().join(layer)) {
            assert!(
                !content.contains("levels_for_model"),
                "effort capability still inferred from model names outside canonical metadata: {path}"
            );
        }
    }
}

#[given("a base directory with only built-in catalogue data")]
fn given_builtin_only_base_dir(world: &mut QuectoWorld) {
    world.catalogue_convergence.base_dir = Some(tempfile::tempdir().expect("tempdir"));
}

#[when("the UDS model listing is rendered")]
fn when_uds_listing_rendered(world: &mut QuectoWorld) {
    let base = world
        .catalogue_convergence
        .base_dir
        .as_ref()
        .expect("base dir prepared")
        .path()
        .to_path_buf();
    world.catalogue_convergence.uds_listing = Some(list_models_data(&base));
}

fn listing_models(world: &QuectoWorld) -> Vec<serde_json::Value> {
    world
        .catalogue_convergence
        .uds_listing
        .as_ref()
        .expect("listing rendered")["models"]
        .as_array()
        .expect("models array")
        .clone()
}

#[then("every listed model carries an effort vocabulary from the snapshot")]
fn then_all_models_carry_effort(world: &mut QuectoWorld) {
    let models = listing_models(world);
    assert!(!models.is_empty(), "builtin listing is empty");
    for model in &models {
        let levels = model["effort_levels"].as_array();
        assert!(
            levels.is_some_and(|l| !l.is_empty()),
            "listed model lacks a snapshot effort vocabulary: {model}"
        );
    }
}

#[then(expr = "the listed model {string} has effort vocabulary {string}")]
fn then_model_effort_vocabulary(world: &mut QuectoWorld, qualified: String, expected: String) {
    let models = listing_models(world);
    let model = models
        .iter()
        .find(|m| m["model"].as_str() == Some(qualified.as_str()))
        .unwrap_or_else(|| panic!("model {qualified} not listed"));
    let got = model["effort_levels"]
        .as_array()
        .unwrap_or_else(|| panic!("model {qualified} carries no effort vocabulary"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(got, expected, "effort vocabulary mismatch for {qualified}");
}

#[given("the contributor documentation")]
fn given_contributor_docs(_world: &mut QuectoWorld) {
    assert!(
        harness_root()
            .join("../docs/runtime-models-providers.md")
            .exists(),
        "contributor documentation missing"
    );
}

fn contributor_docs() -> String {
    fs::read_to_string(harness_root().join("../docs/runtime-models-providers.md"))
        .unwrap()
        .to_ascii_lowercase()
}

#[then("it maps layer ownership across domain, application, infrastructure, and interface")]
fn then_docs_map_layer_ownership(_world: &mut QuectoWorld) {
    let docs = contributor_docs();
    assert!(
        docs.contains("layer ownership"),
        "missing layer ownership map"
    );
    for layer in ["domain", "application", "infrastructure", "interface"] {
        assert!(
            docs.contains(layer),
            "layer ownership map missing layer: {layer}"
        );
    }
}

#[then("it explains how to add domain metadata")]
fn then_docs_explain_domain_metadata(_world: &mut QuectoWorld) {
    assert!(
        contributor_docs().contains("domain metadata"),
        "missing guidance on adding domain metadata"
    );
}

#[then("it warns against creating another authority")]
fn then_docs_warn_single_authority(_world: &mut QuectoWorld) {
    assert!(
        contributor_docs().contains("another authority"),
        "missing warning against creating another catalogue authority"
    );
}
