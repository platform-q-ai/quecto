//! End-to-end acceptance guards for epic #1193 slice 6 (issue #1576): legacy
//! provider/model authorities are removed or demonstrably subordinated to the
//! application-published snapshot, capability heuristics live in canonical
//! metadata, and the contributor documentation maps layer ownership.
//!
//! The source-tree scans are the grep-able forms of the epic's "no duplicate
//! authority" acceptance criteria; each is paired with a behavioral check so
//! a rename alone can neither satisfy nor evade the criterion. The scan and
//! docs logic itself lives in `tests/common/catalogue_conformance.rs`, shared
//! with the BDD convergence steps and the consumer contract tests.

#[path = "common/catalogue_conformance.rs"]
mod conformance;

use conformance::{harness_root, production_sources};

#[test]
fn cli_interface_owns_no_catalogue_composition_bridges() {
    let present = conformance::legacy_bridge_modules_present();
    assert!(
        present.is_empty(),
        "legacy interface catalogue authorities still present: {present:?}"
    );
}

#[test]
fn interface_layer_does_not_read_the_legacy_model_registry() {
    let readers = conformance::model_registry_readers(&production_sources(
        &harness_root().join("src/interface"),
    ));
    assert!(
        readers.is_empty(),
        "interface modules still read the legacy model registry: {readers:?}"
    );
}

#[test]
fn infrastructure_defines_no_canonical_catalogue_types() {
    // "No canonical types in infrastructure": adapters map wire formats into
    // the domain's canonical types, never redefine them.
    let redefinitions = conformance::canonical_type_redefinitions(&production_sources(
        &harness_root().join("src/infrastructure"),
    ));
    assert!(
        redefinitions.is_empty(),
        "infrastructure redefines canonical catalogue types: {redefinitions:?}"
    );
}

#[test]
fn set_model_derives_limits_from_the_published_snapshot_only() {
    // The UDS set_model path must resolve limits from one published snapshot
    // read (`interface::catalogue_runtime::published_model_limits`), not via
    // a bridge that re-resolves the catalogue on its own. The grep is a
    // tripwire; the behavioral counterparts are
    // `contracts::catalogue_consumers::set_model_limits_and_selection_come_from_the_published_snapshot`
    // and the dispatch-level re-clamp test
    // (`uds_dispatch_935_clamp_tests::dispatch_set_model_re_clamps_effective_max_tokens`).
    let dispatch =
        std::fs::read_to_string(harness_root().join("src/interface/cli/uds_dispatch_runtime.rs"))
            .unwrap();
    assert!(
        !dispatch.contains("model_limits_from_base_dir"),
        "uds set_model still derives limits through the legacy bridge re-resolve"
    );
    assert!(
        dispatch.contains("published_model_limits"),
        "uds set_model must derive limits from the published snapshot"
    );
}

#[test]
fn canonical_capabilities_carry_the_effort_vocabulary() {
    // Behavioral, not a source grep: the built-in entries loaded through the
    // production `CatalogueSource` port must carry a non-empty canonical
    // effort vocabulary.
    let missing = conformance::builtin_entries_missing_effort_vocabulary();
    assert!(
        missing.is_empty(),
        "builtin entries lack a canonical effort vocabulary: {missing:?}"
    );
}

#[test]
fn effort_capability_is_not_inferred_from_model_names_outside_the_domain() {
    // The provider/model-name reasoning-effort heuristic may survive only as
    // the domain rule seeding canonical metadata; interface and
    // infrastructure consumers project the canonical vocabulary instead.
    for layer in ["src/interface", "src/infrastructure"] {
        let sites = conformance::effort_name_inference_sites(&production_sources(
            &harness_root().join(layer),
        ));
        assert!(
            sites.is_empty(),
            "effort capability inferred from model names in {layer}: {sites:?}"
        );
    }
}

#[test]
fn contributor_docs_map_layer_ownership_and_forbid_new_authorities() {
    conformance::assert_contributor_docs_structure(&conformance::contributor_docs());
}
