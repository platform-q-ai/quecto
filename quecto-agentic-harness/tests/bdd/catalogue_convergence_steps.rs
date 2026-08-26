//! Steps for catalogue_convergence.feature (issue #1576, epic #1193 slice 6).
//!
//! Slice 6 removes or subordinates every legacy provider/model authority left
//! by slices 1-5. The scans, canonical-capability checks, and docs-structure
//! checks live in `tests/common/catalogue_conformance.rs`, shared with the
//! `issue_1193_completion` acceptance guard and the consumer contract tests;
//! these steps drive them over world state.

use super::*;
use common::catalogue_conformance as conformance;
use quecto::interface::cli::uds_models::list_models_data;

#[derive(Debug, Default)]
pub struct CatalogueConvergenceState {
    source_root: Option<PathBuf>,
    /// Production sources per scanned layer, keyed by layer directory name.
    scanned_layers: Vec<(String, Vec<(String, String)>)>,
    base_dir: Option<tempfile::TempDir>,
    listing: Option<serde_json::Value>,
    contributor_docs: Option<String>,
}

impl CatalogueConvergenceState {
    fn layer(&self, name: &str) -> &[(String, String)] {
        &self
            .scanned_layers
            .iter()
            .find(|(layer, _)| layer == name)
            .expect("source tree scanned")
            .1
    }
    fn docs(&self) -> &str {
        self.contributor_docs.as_deref().expect("docs read")
    }
}

#[given("the harness source tree")]
fn given_harness_source_tree(world: &mut QuectoWorld) {
    world.catalogue_convergence.source_root = Some(conformance::harness_root());
}

#[when("the source tree is scanned for legacy authorities")]
fn when_source_tree_scanned(world: &mut QuectoWorld) {
    let root = world
        .catalogue_convergence
        .source_root
        .clone()
        .expect("source tree given");
    world.catalogue_convergence.scanned_layers = ["src/interface", "src/infrastructure"]
        .into_iter()
        .map(|layer| {
            (
                layer.to_string(),
                conformance::production_sources(&root.join(layer)),
            )
        })
        .collect();
}

#[then("the CLI interface declares no catalogue bridge modules")]
fn then_no_bridge_modules(_world: &mut QuectoWorld) {
    let present = conformance::legacy_bridge_modules_present();
    assert!(
        present.is_empty(),
        "legacy interface catalogue authorities still present: {present:?}"
    );
}

#[then("no interface module reads the legacy model registry")]
fn then_interface_reads_no_registry(world: &mut QuectoWorld) {
    let readers =
        conformance::model_registry_readers(world.catalogue_convergence.layer("src/interface"));
    assert!(
        readers.is_empty(),
        "interface modules still read the legacy model registry: {readers:?}"
    );
}

#[then("no infrastructure module defines canonical catalogue types")]
fn then_infrastructure_defines_no_canonical_types(world: &mut QuectoWorld) {
    let redefinitions = conformance::canonical_type_redefinitions(
        world.catalogue_convergence.layer("src/infrastructure"),
    );
    assert!(
        redefinitions.is_empty(),
        "infrastructure redefines canonical catalogue types: {redefinitions:?}"
    );
}

#[then("canonical model capabilities declare an effort vocabulary")]
fn then_capabilities_declare_effort(_world: &mut QuectoWorld) {
    let missing = conformance::builtin_entries_missing_effort_vocabulary();
    assert!(
        missing.is_empty(),
        "builtin entries lack a canonical effort vocabulary: {missing:?}"
    );
}

#[then("no interface or infrastructure module infers effort levels from model names")]
fn then_no_effort_name_inference(world: &mut QuectoWorld) {
    for layer in ["src/interface", "src/infrastructure"] {
        let sites =
            conformance::effort_name_inference_sites(world.catalogue_convergence.layer(layer));
        assert!(
            sites.is_empty(),
            "effort capability inferred from model names in {layer}: {sites:?}"
        );
    }
}

#[given("a base directory with only built-in catalogue data")]
fn given_builtin_only_base_dir(world: &mut QuectoWorld) {
    world.catalogue_convergence.base_dir = Some(tempfile::tempdir().expect("tempdir"));
}

#[when("the model listing is requested")]
fn when_model_listing_requested(world: &mut QuectoWorld) {
    let base = world
        .catalogue_convergence
        .base_dir
        .as_ref()
        .expect("base dir prepared")
        .path()
        .to_path_buf();
    world.catalogue_convergence.listing = Some(list_models_data(&base));
}

fn listing_models(world: &QuectoWorld) -> Vec<serde_json::Value> {
    world
        .catalogue_convergence
        .listing
        .as_ref()
        .expect("listing requested")["models"]
        .as_array()
        .expect("models array")
        .clone()
}

#[then("every listed model carries an effort vocabulary from the snapshot")]
fn then_all_models_carry_effort(world: &mut QuectoWorld) {
    let models = listing_models(world);
    assert!(!models.is_empty(), "builtin listing is empty");
    for model in &models {
        let levels = model["effortLevels"].as_array();
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
    let got = model["effortLevels"]
        .as_array()
        .unwrap_or_else(|| panic!("model {qualified} carries no effort vocabulary"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(got, expected, "effort vocabulary mismatch for {qualified}");
}

#[given("the contributor documentation exists")]
fn given_contributor_docs(world: &mut QuectoWorld) {
    // Context only: remember where the documentation lives; reading and
    // verifying happen in the When/Then steps.
    world.catalogue_convergence.source_root = Some(conformance::harness_root());
}

#[when("a contributor reads it")]
fn when_contributor_reads_docs(world: &mut QuectoWorld) {
    world.catalogue_convergence.contributor_docs = Some(conformance::contributor_docs());
}

#[then("it maps layer ownership across domain, application, infrastructure, and interface")]
fn then_docs_map_layer_ownership(world: &mut QuectoWorld) {
    let missing =
        conformance::layers_missing_from_ownership_map(world.catalogue_convergence.docs());
    assert!(
        missing.is_empty(),
        "layer ownership map missing layers: {missing:?}"
    );
}

#[then("it explains how to add domain metadata")]
fn then_docs_explain_domain_metadata(world: &mut QuectoWorld) {
    assert!(
        conformance::has_doc_section(
            world.catalogue_convergence.docs(),
            "## Add or change domain metadata",
        ),
        "missing domain-metadata guidance"
    );
}

#[then("it explains how to add a catalogue source")]
fn then_docs_explain_catalogue_source(world: &mut QuectoWorld) {
    assert!(
        conformance::has_doc_section(
            world.catalogue_convergence.docs(),
            "## Add a catalogue source",
        ),
        "missing catalogue-source guidance"
    );
}

#[then("it explains user overrides")]
fn then_docs_explain_user_overrides(world: &mut QuectoWorld) {
    assert!(
        conformance::has_doc_section(world.catalogue_convergence.docs(), "### User overrides"),
        "missing user-override guidance"
    );
}

#[then("it warns against creating another authority")]
fn then_docs_warn_single_authority(world: &mut QuectoWorld) {
    assert!(
        conformance::ownership_map_warns_against_another_authority(
            world.catalogue_convergence.docs()
        ),
        "layer ownership map must warn against creating another authority"
    );
}
