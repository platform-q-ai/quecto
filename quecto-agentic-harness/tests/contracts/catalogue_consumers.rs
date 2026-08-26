//! Consumer-surface contract for the published catalogue snapshot (issue
//! #1572, epic #1193 slice 2).
//!
//! Every read surface must render one snapshot generation: the CLI/UDS model
//! listing (`uds_models::list_models_data`, the response the CLI serves and
//! the TUI consumes), the TUI model-list projection
//! (`quecto_tui::protocol::model_payloads::parse_model_list` over that same
//! response), and the shared application projection over the snapshot store.
//! None of them may parse or merge catalogue files on the read path — the only
//! parse happens inside the infrastructure source adapters feeding the resolve
//! use case.

use quecto::application::catalogue::project_model_listing;
use quecto::infrastructure::catalogue_registry::snapshot_store_for;
use quecto::interface::cli::uds_models::list_models_data;

fn write_models_json(dir: &std::path::Path, body: &str) {
    std::fs::write(dir.join("models.json"), body).unwrap();
}

#[test]
fn cli_uds_and_tui_surfaces_render_the_same_snapshot_generation() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(
        tmp.path(),
        r#"{"providers":{"contractish":{"api":"openai-completions","apiKey":"sk-contract",
            "models":[{"id":"contract-model","name":"Contract Model"}]}}}"#,
    );

    // The real UDS/CLI listing response.
    let response = list_models_data(tmp.path());
    let models = response["models"].as_array().unwrap();

    // The shared projection over the published snapshot store for the same
    // base directory: same generation, same content, same order.
    let snapshot = snapshot_store_for(tmp.path()).current();
    let listing = project_model_listing(&snapshot);
    assert_eq!(response["generation"].as_u64().unwrap(), listing.generation);
    assert_eq!(models.len(), listing.rows.len());
    for (rendered, row) in models.iter().zip(&listing.rows) {
        assert_eq!(rendered["model"].as_str().unwrap(), row.qualified_id);
        assert_eq!(
            rendered["name"].as_str().map(str::to_string),
            row.display_name
        );
        assert_eq!(rendered["configured"].as_bool().unwrap(), row.runnable);
    }
    let contract_row = listing
        .rows
        .iter()
        .find(|row| row.qualified_id == "contractish/contract-model")
        .expect("user-defined model missing from the shared projection");
    assert!(
        contract_row.runnable,
        "apiKey-configured model must be runnable"
    );

    // The real TUI model-list projection consumes this same response, so the
    // TUI list is a projection of the identical snapshot generation.
    let tui = quecto_tui::protocol::model_payloads::parse_model_list(&response, &|s| s.to_string());
    assert_eq!(tui.len(), listing.rows.len());
    for (entry, row) in tui.iter().zip(&listing.rows) {
        assert_eq!(entry.id, row.qualified_id);
    }
}

#[test]
fn listing_surfaces_share_one_generation_across_repeated_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let first = list_models_data(tmp.path())["generation"].as_u64().unwrap();
    let second = list_models_data(tmp.path())["generation"].as_u64().unwrap();
    assert_eq!(
        second,
        first + 1,
        "each resolve publishes the next generation"
    );
    assert_eq!(
        snapshot_store_for(tmp.path()).current().generation(),
        second,
        "every surface reads the same published store"
    );
}

#[test]
fn credential_material_from_models_json_never_reaches_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    // The literal key is parsed by the real infrastructure adapter and flows
    // as far as the credential-status port; the published snapshot (which the
    // listing surfaces render) must never carry it.
    write_models_json(
        tmp.path(),
        r#"{"providers":{"secretish":{"api":"openai-completions","apiKey":"sk-super-secret-42",
            "models":[{"id":"m","name":"M"}]}}}"#,
    );
    let response = list_models_data(tmp.path());
    let snapshot = snapshot_store_for(tmp.path()).current();
    assert!(
        snapshot
            .find(&quecto::domain::catalogue::ModelRef::parse_qualified("secretish/m").unwrap())
            .is_some()
    );
    assert!(!format!("{snapshot:?}").contains("sk-super-secret-42"));
    assert!(!response.to_string().contains("sk-super-secret-42"));
}

#[test]
fn a_malformed_models_json_reports_an_error_and_keeps_a_coherent_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let valid = list_models_data(tmp.path());
    assert!(valid.get("error").is_none());
    let valid_count = snapshot_store_for(tmp.path()).current().entries().len();
    assert!(valid_count > 0);

    write_models_json(tmp.path(), "not json");
    let broken = list_models_data(tmp.path());
    // Legacy wire parity: a malformed file yields no models plus an error.
    assert_eq!(broken["models"].as_array().unwrap().len(), 0);
    assert!(
        broken["error"]
            .as_str()
            .unwrap()
            .contains("failed to parse")
    );
    // Malformed-source isolation: the failed layer is skipped, the remaining
    // layers still publish, and the catalogue never collapses to empty.
    assert_eq!(
        snapshot_store_for(tmp.path()).current().entries().len(),
        valid_count
    );
}
